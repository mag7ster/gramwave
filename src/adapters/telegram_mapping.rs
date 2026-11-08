use tg_player_application::media::MediaTrack as Track;
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
        library_id: 0,
        entry_id: Default::default(),
        chat,
        message: v["id"].as_i64()?,
        file: f["id"].as_i64()?,
        title,
        artist: a["performer"].as_str().unwrap_or("").into(),
        duration: a["duration"].as_u64().unwrap_or(0),
        kind: kind.try_into().ok()?,
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

use tg_player_application::telegram::TelegramFile;
#[cfg(any(feature = "telegram", test))]
use tg_player_application::telegram::{AuthorizationState as AuthState, TelegramUpdate};
#[cfg(any(feature = "telegram", test))]
pub(crate) fn auth_state(v: &serde_json::Value) -> AuthState {
    match v["@type"].as_str().unwrap_or("") {
        "authorizationStateWaitTdlibParameters" => AuthState::Parameters,
        "authorizationStateWaitPhoneNumber" => AuthState::Phone,
        "authorizationStateWaitOtherDeviceConfirmation" => {
            AuthState::Confirmation(v["link"].as_str().unwrap_or("").into())
        }
        "authorizationStateWaitCode" | "authorizationStateWaitEmailCode" => AuthState::Code {
            email: v["@type"] == "authorizationStateWaitEmailCode",
            delivery: v["code_info"]["type"]["@type"]
                .as_str()
                .unwrap_or("")
                .into(),
            next: v["code_info"]["next_type"].is_object(),
            timeout: v["code_info"]["timeout"].as_u64().unwrap_or(0),
        },
        "authorizationStateWaitPassword" => {
            AuthState::Password(v["password_hint"].as_str().unwrap_or("").into())
        }
        "authorizationStateWaitEmailAddress" => AuthState::Email,
        "authorizationStateReady" => AuthState::Ready,
        "authorizationStateClosed" => AuthState::Closed,
        _ => AuthState::Starting,
    }
}
pub(crate) fn file_state(v: &serde_json::Value) -> TelegramFile {
    TelegramFile {
        id: v["id"].as_i64().unwrap_or(0),
        size: v["size"].as_u64().unwrap_or(0),
        downloaded: v["local"]["downloaded_size"].as_u64().unwrap_or(0),
        path: v["local"]["path"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        complete: v["local"]["is_downloading_completed"] == true,
    }
}
#[cfg(any(feature = "telegram", test))]
pub(crate) fn update(v: serde_json::Value) -> Option<TelegramUpdate> {
    Some(match v["@type"].as_str()? {
        "updateAuthorizationState" => {
            TelegramUpdate::Authorization(auth_state(&v["authorization_state"]))
        }
        "updateFile" => TelegramUpdate::File(file_state(&v["file"])),
        "updateNewChat" => TelegramUpdate::Chat(crate::adapters::discovery::chat_metadata(
            &v["chat"],
            [true, true, false],
        )?),
        "updateChatTitle" => {
            TelegramUpdate::ChatTitle(v["chat_id"].as_i64()?, v["title"].as_str()?.into())
        }
        "updateNewMessage" => {
            let m = &v["message"];
            let chat = m["chat_id"].as_i64()?;
            TelegramUpdate::Message {
                id: crate::domain::TrackId {
                    chat,
                    message: m["id"].as_i64()?,
                },
                track: track(chat, m),
            }
        }
        "updateMessageContent" => {
            let chat = v["chat_id"].as_i64()?;
            let message = v["message_id"].as_i64()?;
            TelegramUpdate::Message {
                id: crate::domain::TrackId { chat, message },
                track: track(
                    chat,
                    &serde_json::json!({"id":message,"content":v["new_content"]}),
                ),
            }
        }
        "updateDeleteMessages" => TelegramUpdate::Deleted {
            chat: v["chat_id"].as_i64()?,
            messages: v["message_ids"]
                .as_array()?
                .iter()
                .filter_map(serde_json::Value::as_i64)
                .collect(),
            from_cache: v["from_cache"] == true,
        },
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn apply_message_deletions(state: &mut crate::model::State, value: &serde_json::Value) {
        let mut app = crate::App::new_at(std::path::PathBuf::new(), true);
        app.state = tg_player_application::AppCore::new(std::mem::take(state));
        let (tx, rx) = std::sync::mpsc::channel();
        app.updates = Some(rx);
        tx.send(super::update(value.clone()).unwrap()).unwrap();
        app.process(&eframe::egui::Context::default());
        *state = app.state.view().clone();
        app.shutdown_core().unwrap();
    }
    use crate::model::{default_filters, Playlist, State, Track};
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

#[cfg(test)]
mod typed_update_tests {
    use super::*;
    #[test]
    fn auth_and_source_deletions_have_typed_boundaries() {
        let update=update(serde_json::json!({"@type":"updateAuthorizationState","authorization_state":{"@type":"authorizationStateWaitPassword","password_hint":"hint"}})).unwrap();
        assert!(
            matches!(update,TelegramUpdate::Authorization(AuthState::Password(hint)) if hint=="hint")
        );
        assert!(matches!(
            super::update(
                serde_json::json!({"@type":"updateDeleteMessages","chat_id":42,"message_ids":[7],"from_cache":true})
            ),
            Some(TelegramUpdate::Deleted {
                from_cache: true,
                ..
            })
        ));
    }
}
