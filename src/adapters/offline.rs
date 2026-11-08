use crate::{
    adapters::{media_loader, telegram::Telegram},
    model::Track,
};
use std::{path::Path, sync::Mutex, time::Duration};
use tg_player_application::{
    ports::{OfflinePublication, OfflineStorage},
    telegram::{GatewayFuture, TelegramFile, TelegramGateway},
    AppError,
};
use tokio_util::sync::CancellationToken;
pub(crate) type Progress = Mutex<Option<(u64, String)>>;
/// Publish a verified file under a unique name; an older attempt cannot overwrite it.
pub(crate) async fn download(
    downloader: &Telegram,
    track: &Track,
    dir: &Path,
    generation: u64,
    progress: &Progress,
    cancel: &CancellationToken,
) -> Result<String, AppError> {
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
    FileStorage::new(dir.to_path_buf())
        .publish(
            OfflinePublication {
                track: t.id(),
                attempt: generation,
                source,
                size: file.size,
            },
            cancel,
        )
        .await
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
            *progress.lock().unwrap() = Some((
                generation,
                format!(
                    "Offline: {} · {} / {} bytes",
                    track.title, file.downloaded, file.size,
                ),
            ));
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
            let copied = tokio::select! {_=cancel.cancelled()=>return Err(AppError::Cancelled),copied=tokio::fs::copy(&request.source,temp.path())=>copied.map_err(storage)?};
            if copied != request.size {
                return Err(AppError::Storage(format!(
                    "Incomplete offline file: {copied}/{}",
                    request.size
                )));
            }
            tokio::fs::OpenOptions::new()
                .write(true)
                .open(temp.path())
                .await
                .map_err(storage)?
                .sync_all()
                .await
                .map_err(storage)?;
            if cancel.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            let path = temp.path().with_extension("media");
            temp.persist_noclobber(&path)
                .map_err(|e| AppError::Storage(e.to_string()))?;
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
                "success" => assert_eq!(std::fs::read(result.unwrap()).unwrap(), b"music"),
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
