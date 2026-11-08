use super::*;
use serde_json::json;
#[cfg(test)]
mod regression {
    use super::*;
    use crate::model::{Playlist, State};

    #[test]
    fn restart_discards_file_ids_and_resolves_audio_from_message() {
        // A previous client assigned 7 to audio; the new client may assign it
        // to a JPEG. Neither that ID nor its now incorrect cached path is safe.
        let old: Track = serde_json::from_value(json!({
            "chat": 42, "message": 100, "file": 7, "image_file": 8,
            "cached": "/cache/photo.jpg", "image_path": "/cache/old-cover.jpg",
            "pinned": "/offline/42-100", "title": "Queue snapshot", "duration": 200
        }))
        .unwrap();
        let metadata = serde_json::to_value(&old).unwrap();
        for field in ["file", "cached", "pinned", "image_file", "image_path"] {
            assert!(metadata.get(field).is_none());
        }
        let tg = Telegram::mock_rpc(|v| {
            assert_eq!(v["@type"], "getMessage");
            assert_eq!(
                (v["chat_id"].as_i64(), v["message_id"].as_i64()),
                (Some(42), Some(100))
            );
            json!({"id":100,"content":{"@type":"messageAudio","audio":{
                "title":"Song", "duration":240,
                "audio":{"id":90,"local":{"path":"/cache/song.mp3","is_downloading_completed":true}},
                "album_cover_thumbnail":{"file":{"id":7,"local":{"path":"/cache/photo.jpg","is_downloading_completed":true}}}
            }}})
        });
        let resolved = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(resolve_async(&tg, &old, &Default::default()))
            .unwrap();
        assert_eq!((resolved.file, resolved.image_file), (90, 7));
        assert_eq!(resolved.cached.as_deref(), Some("/cache/song.mp3"));
        assert_eq!(resolved.image_path.as_deref(), Some("/cache/photo.jpg"));
        assert_eq!(resolved.duration, 200);
        assert_eq!(resolved.title, "Queue snapshot");

        let state = State {
            playlists: vec![Playlist {
                id: 42,
                tracks: vec![resolved.clone().into()],
                ..Default::default()
            }],
            chat_pins: vec![42],
            ..Default::default()
        };
        let restored: State =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(restored.chat_pins, vec![42]);
        let json = serde_json::to_string(&restored).unwrap();
        assert!(
            !json.contains("/cache/") && !json.contains("image_file") && !json.contains("cached")
        );
        {
            let t = &restored.playlists[0].tracks[0];
            assert_eq!((t.chat, t.message), (42, 100));
            let media =
                tg_player_application::AppCore::new(restored.clone()).media_track(t.clone());
            assert_eq!((media.file, media.image_file), (0, 0));
            assert!(media.cached.is_none());
        }
    }

    #[test]
    fn unavailable_message_is_a_playback_error() {
        for response in [
            json!({"@type":"error","message":"Message not found"}),
            json!({"id":100,"content":{"@type":"messageText"}}),
        ] {
            let tg = Telegram::mock_rpc(move |_| response.clone());
            let error = tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(resolve_async(
                    &tg,
                    &Track {
                        chat: 42,
                        message: 100,
                        ..Default::default()
                    },
                    &Default::default(),
                ))
                .unwrap_err();
            assert!(matches!(
                error,
                tg_player_application::AppError::Telegram { .. }
            ));
        }
    }
}

#[test]
fn async_resolution_preserves_telegram_codes_and_cancellation() {
    use tg_player_application::AppError;
    use tokio_util::sync::CancellationToken;
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let tg = Telegram::mock_rpc(|_| json!({"@type":"error","code":429,"message":"FLOOD_WAIT_3"}));
    let track = Track {
        chat: 42,
        message: 7,
        ..Default::default()
    };
    assert_eq!(
        runtime.block_on(resolve_async(&tg, &track, &Default::default())),
        Err(AppError::Telegram {
            code: 429,
            message: "FLOOD_WAIT_3".into()
        })
    );
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        runtime.block_on(resolve_async(&tg, &track, &cancelled)),
        Err(AppError::Cancelled)
    );
}
