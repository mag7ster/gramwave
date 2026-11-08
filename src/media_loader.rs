use crate::{
    model::{track, Track},
    telegram::Telegram,
};
use serde_json::json;

// TDLib's numeric file IDs belong to a running client. Resolve persisted
// message identities before using files in a new client instance.
pub fn resolve(tg: &Telegram, old: &Track) -> Result<Track, String> {
    let message = tg.call(json!({
        "@type": "getMessage", "chat_id": old.chat, "message_id": old.message
    }))?;
    let mut resolved = track(old.chat, &message)
        .filter(|t| t.message == old.message && t.file > 0)
        .ok_or_else(|| crate::tr("This message no longer contains playable audio"))?;
    resolved.pinned = old.pinned.clone();
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Playlist, State};

    #[test]
    fn restart_discards_file_ids_and_resolves_audio_from_message() {
        // A previous client assigned 7 to audio; the new client may assign it
        // to a JPEG. Neither that ID nor its now incorrect cached path is safe.
        let old: Track = serde_json::from_value(json!({
            "chat": 42, "message": 100, "file": 7, "image_file": 8,
            "cached": "/cache/photo.jpg", "image_path": "/cache/old-cover.jpg",
            "pinned": "/offline/42-100", "title": "Song"
        }))
        .unwrap();
        assert_eq!((old.file, old.image_file), (0, 0));
        assert!(old.cached.is_none());
        assert!(old.image_path.is_none());
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
        let resolved = resolve(&tg, &old).unwrap();
        assert_eq!((resolved.file, resolved.image_file), (90, 7));
        assert_eq!(resolved.cached.as_deref(), Some("/cache/song.mp3"));
        assert_eq!(resolved.image_path.as_deref(), Some("/cache/photo.jpg"));
        assert_eq!(resolved.duration, 240);
        assert_eq!(resolved.pinned, old.pinned);

        let state = State {
            playlists: vec![Playlist {
                id: 42,
                image_file: 5,
                image_path: Some("/cache/avatar.jpg".into()),
                tracks: vec![resolved.clone()],
                ..Default::default()
            }],
            chat_pins: vec![42],
            queue: vec![resolved.clone()],
            pending_pins: vec![resolved],
            ..Default::default()
        };
        let restored: State =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(restored.chat_pins, vec![42]);
        assert_eq!(restored.playlists[0].image_file, 0);
        assert!(restored.playlists[0].image_path.is_none());
        for t in [
            &restored.playlists[0].tracks[0],
            &restored.queue[0],
            &restored.pending_pins[0],
        ] {
            assert_eq!((t.chat, t.message), (42, 100));
            assert_eq!((t.file, t.image_file), (0, 0));
            assert!(t.cached.is_none());
            assert!(t.pinned.is_some());
        }
    }

    #[test]
    fn unavailable_message_is_a_playback_error() {
        for response in [
            json!({"@type":"error","message":"Message not found"}),
            json!({"id":100,"content":{"@type":"messageText"}}),
        ] {
            let tg = Telegram::mock_rpc(move |_| response.clone());
            assert!(resolve(
                &tg,
                &Track {
                    chat: 42,
                    message: 100,
                    ..Default::default()
                }
            )
            .is_err());
        }
    }
}
