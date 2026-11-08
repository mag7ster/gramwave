use crate::model::Track;
pub fn audio_document(name: &str, mime: &str) -> bool {
    mime.starts_with("audio/")
        || [
            "mp3", "flac", "m4a", "aac", "ogg", "opus", "wav", "aiff", "alac",
        ]
        .iter()
        .any(|e| name.to_lowercase().ends_with(&format!(".{e}")))
}
pub fn track(chat: i64, v: &serde_json::Value) -> Option<Track> {
    let c = &v["content"];
    let ty = c["@type"].as_str()?;
    let (kind, a): (usize, _) = match ty {
        "messageAudio" => (0, &c["audio"]),
        "messageDocument" => {
            let d = &c["document"];
            if !audio_document(
                d["file_name"].as_str().unwrap_or(""),
                d["mime_type"].as_str().unwrap_or(""),
            ) {
                return None;
            }
            (1, d)
        }
        "messageVoiceNote" => (2, &c["voice_note"]),
        _ => return None,
    };
    let f = match kind {
        0 => &a["audio"],
        1 => &a["document"],
        _ => &a["voice"],
    };
    let title = a["title"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| a["file_name"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| crate::i18n::tr("Voice message"));
    Some(Track {
        entry_id: Default::default(),
        chat,
        message: v["id"].as_i64()?,
        file: f["id"].as_i64()?,
        title,
        artist: a["performer"].as_str().unwrap_or("").into(),
        duration: a["duration"].as_u64().unwrap_or(0),
        kind: kind.try_into().ok()?,
        pinned: None,
        cached: local_path(f),
        image_file: a["album_cover_thumbnail"]["file"]["id"]
            .as_i64()
            .or_else(|| a["thumbnail"]["file"]["id"].as_i64())
            .unwrap_or(0),
        image_path: local_path(&a["album_cover_thumbnail"]["file"])
            .or_else(|| local_path(&a["thumbnail"]["file"])),
    })
}
fn local_path(file: &serde_json::Value) -> Option<String> {
    (file["local"]["is_downloading_completed"] == true)
        .then(|| {
            file["local"]["path"]
                .as_str()
                .filter(|p| !p.is_empty())
                .map(str::to_owned)
        })
        .flatten()
}

pub fn apply_message_deletions(state: &mut crate::model::State, update: &serde_json::Value) {
    // TDLib also sends updateDeleteMessages when evicting its own cache.
    // The source messages still exist and our playlist metadata stays valid.
    if update["from_cache"] == true {
        return;
    }
    if let Some(p) = state
        .playlists
        .iter_mut()
        .find(|p| Some(p.id) == update["chat_id"].as_i64())
    {
        p.tracks.retain(|t| {
            !update["message_ids"]
                .as_array()
                .is_some_and(|ids| ids.contains(&serde_json::json!(t.message)))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{default_filters, Playlist, State};
    #[test]
    fn cache_eviction_keeps_second_pinned_playlist() {
        let playlist = |id| Playlist {
            id,
            tracks: vec![Track {
                chat: id,
                message: 10,
                ..Default::default()
            }],
            filters: default_filters(),
            cursors: [10, 0, 0],
            done: [true; 3],
            ..Default::default()
        };
        let mut state = State {
            playlists: vec![playlist(1), playlist(2)],
            chat_pins: vec![1, 2],
            ..Default::default()
        };
        apply_message_deletions(
            &mut state,
            &serde_json::json!({
                "@type":"updateDeleteMessages", "chat_id":2, "message_ids":[10],
                "is_permanent":false, "from_cache":true
            }),
        );
        assert_eq!(state.playlists[1].tracks.len(), 1);
        assert_eq!(state.playlists[1].cursors, [10, 0, 0]);
        assert_eq!(state.sidebar_chats(false, ""), vec![0, 1]);
        // Actual source deletion affects only the corresponding chat, even
        // when both playlists contain the same message ID.
        apply_message_deletions(
            &mut state,
            &serde_json::json!({
                "@type":"updateDeleteMessages", "chat_id":2, "message_ids":[10],
                "is_permanent":true, "from_cache":false
            }),
        );
        assert!(state.playlists[1].tracks.is_empty());
        assert_eq!(state.playlists[0].tracks.len(), 1);
    }

    #[test]
    fn document_filter() {
        assert!(audio_document("Song.FLAC", "application/octet-stream"));
        assert!(!audio_document("movie.mp4", "video/mp4"));
    }
    #[test]
    fn message_identity() {
        let v = serde_json::json!({"id":123,"content":{"@type":"messageAudio","audio":{"title":"Song","performer":"Artist","audio":{"id":8}}}});
        let t = track(7, &v).unwrap();
        assert_eq!((t.chat, t.message, t.file), (7, 123, 8));
    }
}
