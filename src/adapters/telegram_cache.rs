use super::{json, Telegram, Value};
use std::sync::Arc;
use tg_player_application::{persistence::CachePolicy, AppError};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug)]
pub(crate) enum CacheCategory {
    Audio,
    Images,
    Emoji,
    Temporary,
    Other,
}

impl CacheCategory {
    pub(crate) const ALL: [Self; 5] = [
        Self::Audio,
        Self::Images,
        Self::Emoji,
        Self::Temporary,
        Self::Other,
    ];

    fn file_types(self) -> &'static [&'static str] {
        match self {
            Self::Audio => &["fileTypeAudio", "fileTypeDocument", "fileTypeVoiceNote"],
            Self::Images => &[
                "fileTypePhoto",
                "fileTypeProfilePhoto",
                "fileTypeThumbnail",
                "fileTypePhotoStory",
                "fileTypeSelfDestructingPhoto",
            ],
            Self::Emoji => &["fileTypeSticker"],
            Self::Temporary => &["fileTypeUnknown"],
            Self::Other => &[
                "fileTypeAnimation",
                "fileTypeNotificationSound",
                "fileTypeSecure",
                "fileTypeSecret",
                "fileTypeSelfDestructingVideo",
                "fileTypeSelfDestructingVideoNote",
                "fileTypeSelfDestructingVoiceNote",
                "fileTypeVideo",
                "fileTypeVideoNote",
                "fileTypeVideoStory",
                "fileTypeWallpaper",
            ],
        }
    }

    fn accesses(self) -> (bool, bool) {
        match self {
            Self::Audio => (true, false),
            Self::Images | Self::Emoji => (false, true),
            Self::Temporary | Self::Other => (true, true),
        }
    }
}

pub(crate) type CacheSelection = [bool; CacheCategory::ALL.len()];

#[derive(Clone, Copy, Debug)]
pub(crate) struct CacheFilter {
    pub(crate) chat: Option<i64>,
    pub(crate) categories: CacheSelection,
}

pub(crate) struct CacheWrite {
    pub(crate) filter: CacheFilter,
    _audio: Option<OwnedRwLockWriteGuard<()>>,
    _artwork: Option<OwnedRwLockWriteGuard<()>>,
}

#[derive(Debug)]
pub(crate) struct CacheUsage {
    pub(crate) bytes: u64,
    pub(crate) count: u64,
    pub(crate) by_category: [u64; CacheCategory::ALL.len()],
}

#[derive(Debug)]
pub(crate) struct CacheChat {
    pub(crate) id: i64,
    pub(crate) usage: CacheUsage,
}

#[derive(Debug)]
pub(crate) struct CacheStats {
    pub(crate) total: CacheUsage,
    pub(crate) chats: Vec<CacheChat>,
}

pub(crate) struct CacheReport {
    pub(crate) stats: CacheStats,
    pub(crate) freed: Option<u64>,
    pub(crate) error: Option<AppError>,
}

impl CacheStats {
    fn selected_bytes(&self, filter: CacheFilter) -> u64 {
        let usage = match filter.chat {
            Some(id) => self
                .chats
                .iter()
                .find(|chat| chat.id == id)
                .map(|chat| &chat.usage),
            None => Some(&self.total),
        };
        usage.map_or(0, |usage| {
            usage
                .by_category
                .into_iter()
                .zip(filter.categories)
                .filter_map(|(bytes, selected)| selected.then_some(bytes))
                .sum()
        })
    }

    fn parse(value: &Value) -> Result<Self, AppError> {
        let malformed = || AppError::Protocol("Invalid Telegram storage statistics".into());
        if value["@type"] != "storageStatistics" {
            return Err(malformed());
        }
        let mut total = CacheUsage {
            bytes: value["size"].as_u64().ok_or_else(malformed)?,
            count: value["count"].as_u64().ok_or_else(malformed)?,
            by_category: Default::default(),
        };
        let mut chats = Vec::new();
        for chat in value["by_chat"].as_array().ok_or_else(malformed)? {
            let id = chat["chat_id"].as_i64().ok_or_else(malformed)?;
            let mut usage = CacheUsage {
                bytes: chat["size"].as_u64().ok_or_else(malformed)?,
                count: chat["count"].as_u64().ok_or_else(malformed)?,
                by_category: Default::default(),
            };
            for file_type in chat["by_file_type"].as_array().ok_or_else(malformed)? {
                let ty = file_type["file_type"]["@type"]
                    .as_str()
                    .ok_or_else(malformed)?;
                let bytes = file_type["size"].as_u64().ok_or_else(malformed)?;
                if let Some(index) = CacheCategory::ALL
                    .iter()
                    .position(|category| category.file_types().contains(&ty))
                {
                    usage.by_category[index] = usage.by_category[index].saturating_add(bytes);
                    total.by_category[index] = total.by_category[index].saturating_add(bytes);
                }
            }
            chats.push(CacheChat { id, usage });
        }
        Ok(Self { total, chats })
    }

    pub(crate) fn demo() -> Self {
        const MIB: u64 = 1024 * 1024;
        Self {
            total: CacheUsage {
                bytes: 172 * MIB,
                count: 39,
                by_category: [128 * MIB, 16 * MIB, 4 * MIB, 24 * MIB, 0],
            },
            chats: Vec::new(),
        }
    }
}

fn optimize_request(filter: &CacheFilter, policy: Option<CachePolicy>) -> Result<Value, AppError> {
    let types: Vec<_> = CacheCategory::ALL
        .into_iter()
        .zip(filter.categories)
        .filter(|(_, selected)| *selected)
        .flat_map(|(category, _)| category.file_types())
        .map(|ty| json!({"@type":ty}))
        .collect();
    if types.is_empty() || filter.chat == Some(0) {
        return Err(AppError::Protocol("Invalid cache selection".into()));
    }
    let policy = policy.map(CachePolicy::normalized);
    Ok(json!({
        "@type": "optimizeStorage",
        "size": policy.map_or(0, |policy| u64::from(policy.size_mib) * 1024 * 1024),
        "ttl": policy.map_or(0, |policy| policy.days * 24 * 60 * 60),
        "count": if policy.is_some() { i32::MAX } else { 0 },
        "immunity_delay": 0,
        "file_types": types,
        "chat_ids": filter.chat.into_iter().collect::<Vec<_>>(),
        "exclude_chat_ids": [],
        "return_deleted_file_statistics": false,
        "chat_limit": 0
    }))
}

impl Telegram {
    pub(crate) async fn cache_read(
        &self,
        cancel: &CancellationToken,
    ) -> Result<OwnedRwLockReadGuard<()>, AppError> {
        read(&self.cache_access, cancel).await
    }

    pub(super) async fn artwork_cache_read(
        &self,
        cancel: &CancellationToken,
    ) -> Result<OwnedRwLockReadGuard<()>, AppError> {
        read(&self.artwork_cache, cancel).await
    }

    pub(crate) fn try_cache_write(
        &self,
        mut filter: CacheFilter,
        audio_allowed: bool,
    ) -> Option<CacheWrite> {
        let requirements = |categories: CacheSelection| {
            CacheCategory::ALL
                .into_iter()
                .zip(categories)
                .filter(|(_, selected)| *selected)
                .fold((false, false), |(audio, artwork), (category, _)| {
                    let (needs_audio, needs_artwork) = category.accesses();
                    (audio || needs_audio, artwork || needs_artwork)
                })
        };
        let (audio_needed, artwork_needed) = requirements(filter.categories);
        let mut audio = (audio_needed && audio_allowed)
            .then(|| self.cache_access.clone().try_write_owned().ok())
            .flatten();
        let mut artwork = artwork_needed
            .then(|| self.artwork_cache.clone().try_write_owned().ok())
            .flatten();
        for (category, selected) in CacheCategory::ALL.into_iter().zip(&mut filter.categories) {
            let (needs_audio, needs_artwork) = category.accesses();
            *selected &= (!needs_audio || audio.is_some()) && (!needs_artwork || artwork.is_some());
        }
        let (audio_used, artwork_used) = requirements(filter.categories);
        if !audio_used {
            audio = None;
        }
        if !artwork_used {
            artwork = None;
        }
        (audio_used || artwork_used).then_some(CacheWrite {
            filter,
            _audio: audio,
            _artwork: artwork,
        })
    }

    pub(crate) async fn cache_stats(&self) -> Result<CacheStats, AppError> {
        let value = self
            .call_async(
                json!({"@type":"getStorageStatistics","chat_limit":i32::MAX}),
                &Default::default(),
            )
            .await?;
        CacheStats::parse(&value)
    }

    pub(crate) async fn optimize_cache(
        &self,
        cache: CacheWrite,
        policy: Option<CachePolicy>,
    ) -> Result<CacheReport, AppError> {
        let request = optimize_request(&cache.filter, policy)?;
        let before = if policy.is_none() {
            self.cache_stats()
                .await
                .ok()
                .map(|stats| stats.selected_bytes(cache.filter))
        } else {
            None
        };
        let result = self
            .call_async(request, &Default::default())
            .await
            .and_then(|value| {
                if value["@type"] == "storageStatistics" {
                    Ok(())
                } else {
                    Err(AppError::Protocol(
                        "Invalid Telegram cache cleanup reply".into(),
                    ))
                }
            });
        // A fresh scan verifies remaining sizes and cancels GC if the request timed out.
        let stats = self.cache_stats().await;
        self.resolved.lock().unwrap().clear();
        match (stats, result) {
            (Ok(stats), result) => Ok(CacheReport {
                freed: before.map(|size| size.saturating_sub(stats.selected_bytes(cache.filter))),
                stats,
                error: result.err(),
            }),
            (Err(error), result) => Err(result.err().unwrap_or(error)),
        }
    }
}

async fn read(
    lock: &Arc<RwLock<()>>,
    cancel: &CancellationToken,
) -> Result<OwnedRwLockReadGuard<()>, AppError> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(AppError::Cancelled),
        guard = lock.clone().read_owned() => Ok(guard),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_statistics_break_down_usage_by_category_and_chat() {
        let value = json!({"@type":"storageStatistics","size":210,"count":5,"by_chat":[
            {"chat_id":-42,"size":110,"count":2,"by_file_type":[
                {"file_type":{"@type":"fileTypeAudio"},"size":85},
                {"file_type":{"@type":"fileTypeVoiceNote"},"size":25}
            ]},
            {"chat_id":0,"size":100,"count":3,"by_file_type":[
                {"file_type":{"@type":"fileTypeProfilePhoto"},"size":30},
                {"file_type":{"@type":"fileTypeSticker"},"size":10},
                {"file_type":{"@type":"fileTypeUnknown"},"size":60}
            ]}
        ]});
        let stats = CacheStats::parse(&value).unwrap();
        assert_eq!(stats.total.bytes, 210);
        assert_eq!(stats.total.count, 5);
        assert_eq!(stats.total.by_category, [110, 30, 10, 60, 0]);
        assert_eq!(stats.chats[0].id, -42);
        assert_eq!(stats.chats[0].usage.count, 2);
        assert_eq!(stats.chats[0].usage.by_category, [110, 0, 0, 0, 0]);
        assert_eq!(stats.chats[1].id, 0);
        assert!(CacheStats::parse(&json!({"@type":"error"})).is_err());
    }

    #[test]
    fn cache_selection_and_policy_use_explicit_tdlib_units_and_filters() {
        let mut filter = CacheFilter {
            chat: Some(-42),
            categories: [false, false, true, true, false],
        };
        let req = optimize_request(&filter, None).unwrap();
        assert_eq!(req["size"], 0);
        assert_eq!(req["ttl"], 0);
        assert_eq!(req["chat_ids"], json!([-42]));
        assert_eq!(
            req["file_types"],
            json!([{"@type":"fileTypeSticker"}, {"@type":"fileTypeUnknown"}])
        );
        filter.chat = None;
        let req = optimize_request(&filter, Some(CachePolicy::default())).unwrap();
        assert_eq!(req["chat_ids"], json!([]));
        assert_eq!(req["size"], 1024 * 1024 * 1024);
        assert_eq!(req["ttl"], 7 * 24 * 60 * 60);
        filter.chat = Some(0);
        assert!(optimize_request(&filter, None).is_err());
        filter.chat = None;
        filter.categories.fill(false);
        assert!(optimize_request(&filter, None).is_err());
    }

    #[tokio::test]
    async fn cleanup_waits_for_cache_readers_and_inspects_after_a_partial_failure() {
        let (sent, received) = std::sync::mpsc::channel();
        let tg = Telegram::mock_rpc(move |request| {
            let method = request["@type"].as_str().unwrap();
            sent.send(method.to_owned()).unwrap();
            match method {
                "optimizeStorage" => json!({"@type":"error","code":500,"message":"Interrupted"}),
                "getStorageStatistics" => {
                    json!({"@type":"storageStatistics","size":0,"count":0,"by_chat":[]})
                }
                _ => panic!("unexpected RPC {method}"),
            }
        });
        let cancel = CancellationToken::new();
        let all = CacheFilter {
            chat: None,
            categories: [true; 5],
        };
        let reader = tg.cache_read(&cancel).await.unwrap();
        let artwork = tg.try_cache_write(all, true).unwrap();
        assert_eq!(artwork.filter.categories, [false, true, true, false, false]);
        assert!(tg.cache_read(&cancel).await.is_ok());
        drop(artwork);
        drop(reader);
        let writer = tg.try_cache_write(all, true).unwrap();
        assert_eq!(writer.filter.categories, all.categories);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), tg.cache_read(&cancel))
                .await
                .is_err()
        );
        let report = tg.optimize_cache(writer, None).await.unwrap();
        assert_eq!(report.stats.total.bytes, 0);
        assert_eq!(report.freed, Some(0));
        assert!(matches!(
            report.error,
            Some(AppError::Telegram { code: 500, .. })
        ));
        assert_eq!(
            received.try_iter().collect::<Vec<_>>(),
            [
                "getStorageStatistics",
                "optimizeStorage",
                "getStorageStatistics"
            ]
        );
        assert!(tg.try_cache_write(all, true).is_some());
    }

    #[tokio::test]
    async fn offline_download_and_artwork_hold_independent_cache_access() {
        let tg = Telegram::mock_rpc(|request| match request["@type"].as_str() {
            Some("downloadFile") => json!({"@type":"file","id":request["file_id"]}),
            Some("cancelDownloadFile") => json!({"@type":"ok"}),
            method => panic!("unexpected RPC {method:?}"),
        });
        let cancel = CancellationToken::new();
        let all = CacheFilter {
            chat: Some(42),
            categories: [true; 5],
        };
        let offline = tg.begin_download(7, 0, true, &cancel).await.unwrap();
        let artwork = tg.artwork_cache_read(&cancel).await.unwrap();
        assert!(tg.try_cache_write(all, true).is_none());
        drop(artwork);
        let available = tg.try_cache_write(all, true).unwrap();
        assert_eq!(
            available.filter.categories,
            [false, true, true, false, false]
        );
        drop(available);
        drop(offline);
        let artwork = tg.artwork_cache_read(&cancel).await.unwrap();
        let available = tg.try_cache_write(all, true).unwrap();
        assert_eq!(
            available.filter.categories,
            [true, false, false, false, false]
        );
        drop(available);
        drop(artwork);
    }

    #[cfg(all(feature = "native-tests", feature = "telegram"))]
    #[tokio::test]
    async fn native_tdlib_inspects_synthetic_files_without_an_account() {
        use tg_player_application::telegram::{AuthRequest, TelegramGateway, TelegramParameters};

        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("telegram");
        let files = root.path().join("cache");
        for (dir, name) in [
            (database.join("profile_photos"), "avatar.jpg"),
            (files.join("music"), "song.mp3"),
            (files.join("temp"), "part"),
        ] {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(name), [0_u8; 4096]).unwrap();
        }
        let (tg, _updates) = Telegram::start(Default::default()).unwrap();
        tg.authenticate(
            AuthRequest::Parameters(TelegramParameters {
                api_id: 1,
                api_hash: "0".repeat(32),
                database: database.to_string_lossy().into(),
                files: files.to_string_lossy().into(),
            }),
            &Default::default(),
        )
        .await
        .unwrap();
        let stats = tg.cache_stats().await.unwrap();
        tg.shutdown(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(stats.total.count, 3);
        assert_eq!(
            stats
                .total
                .by_category
                .iter()
                .filter(|&&size| size > 0)
                .count(),
            3,
        );
    }
}
