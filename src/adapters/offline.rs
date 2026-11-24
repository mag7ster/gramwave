use crate::{
    adapters::{media_loader, telegram::Telegram},
    model::Track,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use tg_player_application::{
    ports::{OfflinePublication, OfflineStorage},
    telegram::{GatewayFuture, TelegramFile, TelegramGateway},
    AppError,
};
use tokio_util::sync::CancellationToken;
#[derive(Default)]
pub(crate) struct Progress {
    latest: Mutex<Option<(u64, String)>>,
    wake: fastframe_shell::Waker,
}

impl Progress {
    pub(crate) fn new(wake: fastframe_shell::Waker) -> Self {
        Self {
            wake,
            ..Self::default()
        }
    }

    pub(crate) fn take(&self) -> Option<(u64, String)> {
        self.latest.lock().unwrap().take()
    }

    fn publish(&self, generation: u64, status: String) {
        let first = self
            .latest
            .lock()
            .unwrap()
            .replace((generation, status))
            .is_none();
        if first {
            self.wake.wake();
        }
    }
}
/// Publish a verified file under a unique name; an older attempt cannot overwrite it.
pub(crate) async fn download(
    downloader: &Telegram,
    track: &Track,
    dir: &Path,
    generation: u64,
    progress: &Progress,
    cancel: &CancellationToken,
) -> Result<tg_player_application::offline::PublishedFile, AppError> {
    let t = media_loader::resolve_async(downloader, track, cancel).await?;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| AppError::Storage(e.to_string()))?;
    let _lease = downloader.begin_download(t.file, 0, true, cancel).await?;
    let file = wait_for_download(
        downloader,
        &t,
        generation,
        progress,
        cancel,
        Duration::from_secs(600),
    )
    .await?;
    let source = file
        .path
        .ok_or(AppError::Unavailable("Downloaded media file"))?;
    if file.size == 0 {
        return Err(AppError::Protocol(
            "Completed media file has no size".into(),
        ));
    }
    let remote_id = downloader
        .call_async(
            serde_json::json!({"@type":"getFile","file_id":t.file}),
            cancel,
        )
        .await
        .ok()
        .and_then(|value| {
            value["remote"]["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
        });
    let path = FileStorage::new(dir.to_path_buf())
        .publish(
            OfflinePublication {
                track: t.id(),
                attempt: generation,
                source,
                size: file.size,
            },
            cancel,
        )
        .await?;
    Ok(tg_player_application::offline::PublishedFile {
        path,
        size: file.size,
        remote_id,
    })
}

pub(crate) fn recover_metadata(
    root: &Path,
    data: &mut tg_player_application::offline::OfflineData,
) {
    let Ok(root) = root.canonicalize() else {
        return;
    };
    for copy in &mut data.copies {
        if copy.size != 0 && copy.last_used != 0 {
            continue;
        }
        let Ok(path) = Path::new(&copy.path).canonicalize() else {
            continue;
        };
        if !path.starts_with(&root) {
            continue;
        }
        let Ok(metadata) = path.metadata() else {
            continue;
        };
        if metadata.is_file() {
            copy.size = metadata.len();
            if copy.last_used == 0 {
                copy.last_used = [metadata.accessed(), metadata.modified()]
                    .into_iter()
                    .filter_map(Result::ok)
                    .map(tg_player_application::offline::unix_seconds)
                    .max()
                    .unwrap_or_default();
            }
        }
    }
}

pub(crate) async fn release_temporary_copy(
    tg: &Telegram,
    copy: &tg_player_application::offline::OfflineCopy,
    root: &Path,
) -> Result<(), AppError> {
    use serde_json::json;
    let cancel = CancellationToken::new();
    let id = copy.track.id();
    let file = if let Some(remote_id) = &copy.remote_id {
        match tg
            .call_async(
                json!({"@type":"getRemoteFile","remote_file_id":remote_id}),
                &cancel,
            )
            .await
        {
            Ok(file) => Some(super::telegram_mapping::file_state(&file)),
            Err(AppError::Telegram {
                code: 400 | 404, ..
            }) => None,
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    let file =
        match file {
            Some(file) => file,
            None => {
                let value = match tg.call_async(
                json!({"@type":"getMessageLocally","chat_id":id.chat,"message_id":id.message}),
                &cancel,
            ).await {
                Ok(value) => value,
                Err(AppError::Telegram { code: 400 | 404, .. }) => return Ok(()),
                Err(error) => return Err(error),
            };
                let Some(track) = super::telegram_mapping::track(id.chat, &value)
                    .filter(|track| track.id() == id && track.file > 0)
                else {
                    return Ok(());
                };
                tg.file(track.file, &cancel).await?
            }
        };
    let Some(source) = file.path.filter(|_| file.complete && file.id > 0) else {
        return Ok(());
    };
    let local = copy.path.clone();
    let root = root.to_owned();
    let candidate = source.clone();
    let same = tokio::task::spawn_blocking(move || same_audio(&root, &local, &candidate))
        .await
        .map_err(|e| AppError::Storage(e.to_string()))?
        .map_err(|e| AppError::Storage(e.to_string()))?;
    if same {
        let current = tg.file(file.id, &cancel).await?;
        if current.complete && current.path.as_deref() == Some(&source) && current.size == file.size
        {
            let reply = tg
                .call_async(json!({"@type":"deleteFile","file_id":file.id}), &cancel)
                .await?;
            if reply["@type"] != "ok" {
                return Err(AppError::Protocol(
                    "Invalid Telegram file cleanup reply".into(),
                ));
            }
        }
    }
    Ok(())
}

fn same_audio(root: &Path, local: &str, source: &str) -> std::io::Result<bool> {
    use std::io::{BufRead, BufReader};
    let (Ok(offline), Ok(cache), Ok(local), Ok(source)) = (
        root.join("offline").canonicalize(),
        root.join("cache").canonicalize(),
        Path::new(local).canonicalize(),
        Path::new(source).canonicalize(),
    ) else {
        return Ok(false);
    };
    if !local.starts_with(offline) || !source.starts_with(cache) || local == source {
        return Ok(false);
    }
    let mut a = BufReader::with_capacity(128 * 1024, std::fs::File::open(local)?);
    let mut b = BufReader::with_capacity(128 * 1024, std::fs::File::open(source)?);
    let a_meta = a.get_ref().metadata()?;
    let b_meta = b.get_ref().metadata()?;
    if !a_meta.is_file() || !b_meta.is_file() || a_meta.len() != b_meta.len() {
        return Ok(false);
    }
    // Compare prefixes independently of short reads.
    loop {
        let aa = a.fill_buf()?;
        let bb = b.fill_buf()?;
        let len = aa.len().min(bb.len());
        if len == 0 {
            return Ok(aa.is_empty() && bb.is_empty());
        }
        if aa[..len] != bb[..len] {
            return Ok(false);
        }
        a.consume(len);
        b.consume(len);
    }
}

async fn wait_for_download(
    downloader: &Telegram,
    track: &tg_player_application::media::MediaTrack,
    generation: u64,
    progress: &Progress,
    cancel: &CancellationToken,
    timeout: Duration,
) -> Result<TelegramFile, AppError> {
    let wait = async {
        loop {
            let file = downloader.file(track.file, cancel).await?;
            if file.id != track.file {
                return Err(AppError::Protocol(
                    "Unexpected downloaded file identity".into(),
                ));
            }
            if file.complete {
                return Ok(file);
            }
            progress.publish(
                generation,
                format!(
                    "Offline: {} · {} / {} bytes",
                    track.title, file.downloaded, file.size,
                ),
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(AppError::Cancelled),
        result = tokio::time::timeout(timeout,wait) => result.map_err(|_|AppError::Timeout("Offline download"))?,
    }
}

pub(crate) struct FileStorage {
    root: std::path::PathBuf,
}
pub(crate) async fn saved_sizes(root: PathBuf, paths: Vec<String>) -> HashMap<String, u64> {
    tokio::task::spawn_blocking(move || {
        if paths.is_empty() {
            return HashMap::new();
        }
        let Ok(root) = root.canonicalize() else {
            return HashMap::new();
        };
        paths
            .into_iter()
            .filter_map(|path| {
                let file = std::fs::canonicalize(&path).ok()?;
                if !file.starts_with(&root) {
                    return None;
                }
                let metadata = file.metadata().ok()?;
                metadata.is_file().then_some((path, metadata.len()))
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

impl FileStorage {
    pub(crate) fn new(root: std::path::PathBuf) -> Self {
        Self { root }
    }
}
impl OfflineStorage for FileStorage {
    fn publish<'a>(
        &'a self,
        request: OfflinePublication,
        cancel: &'a CancellationToken,
    ) -> GatewayFuture<'a, String> {
        Box::pin(async move {
            let storage = |e: std::io::Error| AppError::Storage(e.to_string());
            tokio::fs::create_dir_all(&self.root)
                .await
                .map_err(storage)?;
            let temp = tempfile::Builder::new()
                .prefix(&format!(
                    "{}-{}-{}-",
                    request.track.chat, request.track.message, request.attempt
                ))
                .suffix(".part")
                .tempfile_in(&self.root)
                .map_err(storage)?;
            let mut source = tokio::fs::File::open(&request.source)
                .await
                .map_err(storage)?;
            let mut destination = tokio::fs::File::from_std(temp.reopen().map_err(storage)?);
            let copied = tokio::select! {
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                result = tokio::io::copy(&mut source, &mut destination) => result.map_err(storage)?,
            };
            if copied != request.size {
                return Err(AppError::Storage(format!(
                    "Incomplete offline file: {copied}/{}",
                    request.size
                )));
            }
            destination.sync_all().await.map_err(storage)?;
            let directory = std::fs::File::open(&self.root).map_err(storage)?;
            if cancel.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            let path = temp.path().with_extension("media");
            temp.persist_noclobber(&path)
                .map_err(|e| AppError::Storage(e.to_string()))?;
            if let Err(error) = directory.sync_all() {
                let _ = std::fs::remove_file(&path);
                return Err(storage(error));
            }
            Ok(path.to_string_lossy().into())
        })
    }
    fn verify<'a>(&'a self, path: &'a str, size: u64) -> GatewayFuture<'a, bool> {
        Box::pin(async move {
            match tokio::fs::metadata(path).await {
                Ok(meta) => Ok(meta.is_file() && meta.len() == size),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(AppError::Storage(e.to_string())),
            }
        })
    }
    fn remove<'a>(&'a self, path: &'a str) -> GatewayFuture<'a, ()> {
        Box::pin(async move {
            let file = match tokio::fs::canonicalize(path).await {
                Ok(path) => path,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(AppError::Storage(e.to_string())),
            };
            let root = tokio::fs::canonicalize(&self.root)
                .await
                .map_err(|e| AppError::Storage(e.to_string()))?;
            if !file.starts_with(root) {
                return Err(AppError::Storage(
                    "Offline file is outside the offline directory".into(),
                ));
            }
            tokio::fs::remove_file(path)
                .await
                .map_err(|e| AppError::Storage(e.to_string()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cache_handoff_only_deletes_matching_temporary_media() {
        use serde_json::json;
        let root = tempfile::tempdir().unwrap();
        for subdirectory in ["offline", "cache"] {
            std::fs::create_dir(root.path().join(subdirectory)).unwrap();
        }
        let local = root.path().join("offline/published");
        let source = root.path().join("cache/source");
        std::fs::write(&local, b"music").unwrap();
        std::fs::write(&source, b"other").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let candidate = source.clone();
        let tg = Telegram::mock_rpc(move |request| match request["@type"].as_str().unwrap() {
            "getRemoteFile" | "getFile" => json!({"@type":"file","id":7,"size":5,
                "local":{"path":candidate,"is_downloading_completed":true}}),
            "deleteFile" => {
                tx.send(request["file_id"].as_i64()).unwrap();
                std::fs::remove_file(&candidate).unwrap();
                json!({"@type":"ok"})
            }
            _ => panic!("Unexpected request {request}"),
        });
        let copy = tg_player_application::offline::OfflineCopy {
            track: Track {
                chat: 1,
                message: 2,
                ..Default::default()
            },
            path: local.to_string_lossy().into(),
            cached: false,
            size: 5,
            last_used: 1,
            handoff_pending: true,
            remote_id: Some("test-id".into()),
        };
        release_temporary_copy(&tg, &copy, root.path())
            .await
            .unwrap();
        assert!(source.is_file());
        assert!(rx.try_recv().is_err());
        std::fs::write(&source, b"music").unwrap();
        release_temporary_copy(&tg, &copy, root.path())
            .await
            .unwrap();
        assert_eq!(rx.try_recv().unwrap(), Some(7));
        assert!(!source.exists());
        assert_eq!(std::fs::read(local).unwrap(), b"music");
    }

    #[test]
    fn publication_verifies_size_cleans_failures_and_never_clobbers_an_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::write(&source, b"music").unwrap();
        let root = temp.path().join("offline");
        let storage = FileStorage::new(root.clone());
        let request = OfflinePublication {
            track: crate::domain::TrackId {
                chat: 42,
                message: 7,
            },
            attempt: 1,
            source: source.to_string_lossy().into(),
            size: 5,
        };
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let mut invalid = request.clone();
            invalid.size = 6;
            assert!(storage.publish(invalid, &Default::default()).await.is_err());
            assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
            let cancel = CancellationToken::new();
            cancel.cancel();
            assert!(storage.publish(request.clone(), &cancel).await.is_err());
            let first = storage
                .publish(request.clone(), &Default::default())
                .await
                .unwrap();
            let second = storage.publish(request, &Default::default()).await.unwrap();
            assert_ne!(first, second);
            assert!(storage.verify(&first, 5).await.unwrap());
            assert!(storage.remove(source.to_str().unwrap()).await.is_err());
            let sizes = saved_sizes(
                root.clone(),
                vec![
                    first.clone(),
                    second.clone(),
                    source.to_string_lossy().into(),
                ],
            )
            .await;
            assert_eq!(
                sizes,
                HashMap::from([(first.clone(), 5), (second.clone(), 5)])
            );
            storage.remove(&first).await.unwrap();
            assert!(!storage.verify(&first, 5).await.unwrap());
            assert!(storage.verify(&second, 5).await.unwrap());
        });
    }
    #[test]
    fn download_keeps_gateway_protocol_and_storage_error_categories() {
        use serde_json::json;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::write(&source, b"music").unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for case in ["telegram", "protocol", "unavailable", "storage", "success"] {
            let mut completed = json!({"id":90,"size":5,"local":{
                "path":source.to_str().unwrap(),"is_downloading_completed":true,"downloaded_size":5,
            }});
            match case {
                "telegram" => {
                    completed = json!({"@type":"error","code":429,"message":"FLOOD_WAIT_3"})
                }
                "protocol" => completed["id"] = json!(91),
                "unavailable" => completed["local"]["path"] = json!(""),
                "storage" => {
                    completed["local"]["path"] =
                        json!(temp.path().join("missing").to_str().unwrap())
                }
                _ => {}
            }
            let tg = Telegram::mock_rpc(move |request| match request["@type"].as_str() {
                Some("getMessage") => {
                    json!({"id":7,"content":{"@type":"messageAudio","audio":{"title":"Song","audio":{"id":90}}}})
                }
                Some("getFile") => completed.clone(),
                _ => json!({"@type":"ok"}),
            });
            let result = runtime.block_on(download(
                &tg,
                &Track {
                    chat: 42,
                    message: 7,
                    ..Default::default()
                },
                &temp.path().join("offline"),
                1,
                &Default::default(),
                &Default::default(),
            ));
            match case {
                "telegram" => assert_eq!(
                    result,
                    Err(AppError::Telegram {
                        code: 429,
                        message: "FLOOD_WAIT_3".into()
                    })
                ),
                "protocol" => assert!(matches!(result, Err(AppError::Protocol(_)))),
                "unavailable" => {
                    assert_eq!(result, Err(AppError::Unavailable("Downloaded media file")))
                }
                "storage" => assert!(matches!(result, Err(AppError::Storage(_)))),
                "success" => assert_eq!(std::fs::read(result.unwrap().path).unwrap(), b"music"),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn incomplete_download_wait_has_typed_timeout_and_cancellation() {
        use serde_json::json;
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let tg = Telegram::mock_rpc(|_| json!({"id":90,"size":5,"local":{"downloaded_size":1}}));
        let track = tg_player_application::media::MediaTrack {
            file: 90,
            ..Default::default()
        };
        assert_eq!(
            runtime
                .block_on(wait_for_download(
                    &tg,
                    &track,
                    1,
                    &Default::default(),
                    &Default::default(),
                    Duration::from_millis(5)
                ))
                .unwrap_err(),
            AppError::Timeout("Offline download")
        );
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            runtime
                .block_on(wait_for_download(
                    &tg,
                    &track,
                    1,
                    &Default::default(),
                    &cancel,
                    Duration::from_secs(1)
                ))
                .unwrap_err(),
            AppError::Cancelled
        );
    }
}
