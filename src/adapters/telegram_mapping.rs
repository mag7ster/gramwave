use gramwave_application::media::MediaTrack as Track;
use gramwave_application::model::{Caption, CaptionEmoji, CaptionLink, EmojiStatus};
use serde::Deserialize;

pub(super) fn telegram_int64(value: &serde_json::Value) -> Option<i64> {
    value
        .as_str()
        .and_then(|id| id.parse().ok())
        .or_else(|| value.as_i64())
}

pub(super) fn emoji_status(value: &serde_json::Value) -> Option<EmojiStatus> {
    let kind = &value["type"];
    let id = match kind["@type"].as_str() {
        Some("emojiStatusTypeCustomEmoji") => telegram_int64(&kind["custom_emoji_id"]),
        Some("emojiStatusTypeUpgradedGift") => telegram_int64(&kind["model_custom_emoji_id"]),
        _ => telegram_int64(&value["custom_emoji_id"]),
    }?;
    (id != 0).then_some(EmojiStatus {
        id,
        expires_at: value["expiration_date"].as_u64().unwrap_or(0),
    })
}

fn caption(value: &serde_json::Value) -> Option<Caption> {
    let text = value["text"]
        .as_str()
        .filter(|text| !text.trim().is_empty())?;
    let mut offsets = vec![Some(0)];
    for (byte, character) in text.char_indices() {
        if character.len_utf16() == 2 {
            offsets.push(None);
        }
        offsets.push(Some(byte + character.len_utf8()));
    }
    let mut emoji = Vec::new();
    let mut links = Vec::new();
    let range = |entity: &serde_json::Value| {
        let start = usize::try_from(entity["offset"].as_u64()?).ok()?;
        let length = usize::try_from(entity["length"].as_u64()?).ok()?;
        let from = offsets.get(start).copied().flatten()?;
        let to = offsets.get(start.checked_add(length)?).copied().flatten()?;
        (from < to).then_some(from..to)
    };
    for entity in value["entities"].as_array().into_iter().flatten() {
        let Some(bytes) = range(entity) else { continue };
        let kind = &entity["type"];
        match kind["@type"].as_str() {
            Some("textEntityTypeCustomEmoji") => {
                if let Some(id) = telegram_int64(&kind["custom_emoji_id"]).filter(|id| *id != 0) {
                    emoji.push(CaptionEmoji { bytes, id });
                }
            }
            Some("textEntityTypeUrl") => links.push(CaptionLink {
                url: text[bytes.clone()].into(),
                bytes,
            }),
            Some("textEntityTypeTextUrl") => {
                if let Some(url) = kind["url"].as_str() {
                    links.push(CaptionLink {
                        bytes,
                        url: url.into(),
                    });
                }
            }
            _ => {}
        }
    }
    emoji.sort_by_key(|entity| entity.bytes.start);
    let mut end = 0;
    emoji.retain(|entity| {
        if entity.bytes.start < end {
            return false;
        }
        end = entity.bytes.end;
        true
    });
    Some(Caption {
        text: text.to_owned(),
        emoji,
        links,
    })
}

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
        caption: caption(&c["caption"]),
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

use gramwave_application::telegram::TelegramFile;
#[cfg(any(feature = "telegram", test))]
use gramwave_application::telegram::{AuthorizationState as AuthState, TelegramUpdate};
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
#[derive(serde::Deserialize)]
pub(super) struct FileReply {
    pub id: i64,
    pub size: Option<u64>,
    pub local: Option<LocalFile>,
    remote: Option<RemoteFile>,
}

#[cfg_attr(not(any(feature = "telegram", test)), allow(dead_code))]
#[derive(Default, serde::Deserialize)]
#[serde(default)]
pub(super) struct LocalFile {
    pub path: String,
    pub is_downloading_completed: bool,
    pub downloaded_size: u64,
    pub download_offset: u64,
    pub downloaded_prefix_size: u64,
}

#[derive(serde::Deserialize)]
struct RemoteFile {
    id: Option<String>,
}

impl FileReply {
    pub(super) fn decode(v: &serde_json::Value) -> Result<Self, gramwave_application::AppError> {
        let error = || gramwave_application::AppError::Protocol("Invalid Telegram file".into());
        let file = Self::deserialize(v).map_err(|_| error())?;
        if file.id <= 0 {
            return Err(error());
        }
        Ok(file)
    }

    pub(super) fn into_file(self) -> TelegramFile {
        let local = self.local.unwrap_or_default();
        TelegramFile {
            id: self.id,
            size: self.size.unwrap_or_default(),
            downloaded: local.downloaded_size,
            path: Some(local.path).filter(|path| !path.is_empty()),
            complete: local.is_downloading_completed,
        }
    }

    pub(super) fn into_remote_id(self) -> Option<String> {
        self.remote?.id.filter(|id| !id.is_empty())
    }
}
fn personal_chat_list(
    list: &serde_json::Value,
) -> Option<gramwave_application::telegram::PersonalChatList> {
    use gramwave_application::telegram::PersonalChatList;
    match list["@type"].as_str()? {
        "chatListMain" => Some(PersonalChatList::Main),
        "chatListArchive" => Some(PersonalChatList::Archive),
        _ => None,
    }
}
pub(super) fn personal_chat_lists(chat: &serde_json::Value) -> Option<[bool; 2]> {
    let mut lists = [false; 2];
    for list in chat["chat_lists"]
        .as_array()?
        .iter()
        .filter_map(personal_chat_list)
    {
        lists[list as usize] = true;
    }
    Some(lists)
}
#[cfg(any(feature = "telegram", test))]
pub(crate) fn update(v: serde_json::Value) -> Option<TelegramUpdate> {
    Some(match v["@type"].as_str()? {
        "updateAuthorizationState" => {
            TelegramUpdate::Authorization(auth_state(&v["authorization_state"]))
        }
        "updateNewChat" => {
            let chat = &v["chat"];
            TelegramUpdate::Chat(
                crate::adapters::discovery::chat_response(chat, [true, true, false]).ok()??,
                personal_chat_lists(chat),
            )
        }
        "updateChatAddedToList" | "updateChatRemovedFromList" => {
            TelegramUpdate::ChatListMembership(
                v["chat_id"].as_i64()?,
                personal_chat_list(&v["chat_list"])?,
                v["@type"] == "updateChatAddedToList",
            )
        }
        "updateChatTitle" => {
            TelegramUpdate::ChatTitle(v["chat_id"].as_i64()?, v["title"].as_str()?.into())
        }
        "updateChatEmojiStatus" => TelegramUpdate::ChatEmojiStatus(
            v["chat_id"].as_i64()?,
            emoji_status(&v["emoji_status"]),
        ),
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
        app.state = gramwave_application::AppCore::new(std::mem::take(state));
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

    #[test]
    fn custom_emoji_caption_offsets_are_utf16_code_units() {
        let caption = super::caption(&serde_json::json!({
            "text":"😀a💿!",
            "entities":[
                {"offset":4,"length":1,"type":{"@type":"textEntityTypeCustomEmoji","custom_emoji_id":"2"}},
                {"offset":3,"length":2,"type":{"@type":"textEntityTypeCustomEmoji","custom_emoji_id":"9007199254740999"}},
                {"offset":3,"length":2,"type":{"@type":"textEntityTypeCustomEmoji","custom_emoji_id":"5"}}
            ]
        })).unwrap();
        assert_eq!(caption.emoji.len(), 1);
        assert_eq!(caption.emoji[0].id, 9007199254740999);
        assert_eq!(&caption.text[caption.emoji[0].bytes.clone()], "💿");
    }

    #[test]
    fn caption_links_share_utf16_offsets_with_emoji() {
        let caption = super::caption(&serde_json::json!({
            "text":"😀 go t.me/x",
            "entities":[
                {"offset":1,"length":1,"type":{"@type":"textEntityTypeUrl"}},
                {"offset":3,"length":2,"type":{"@type":"textEntityTypeTextUrl","url":"https://example.com"}},
                {"offset":6,"length":6,"type":{"@type":"textEntityTypeUrl"}}
            ]
        })).unwrap();
        assert_eq!(caption.links.len(), 2);
        assert_eq!(&caption.text[caption.links[0].bytes.clone()], "go");
        assert_eq!(caption.links[0].url, "https://example.com");
        assert_eq!(caption.links[1].url, "t.me/x");
    }

    #[test]
    fn chat_emoji_status_updates_can_remove_the_badge() {
        let status = super::update(serde_json::json!({
            "@type":"updateChatEmojiStatus","chat_id":42,"emoji_status":{
                "expiration_date":123,"type":{"@type":"emojiStatusTypeCustomEmoji","custom_emoji_id":"456"}
            }
        }));
        assert!(matches!(
            status,
            Some(TelegramUpdate::ChatEmojiStatus(
                42,
                Some(EmojiStatus {
                    id: 456,
                    expires_at: 123
                })
            ))
        ));
        assert!(matches!(
            super::update(serde_json::json!({
                "@type":"updateChatEmojiStatus","chat_id":42,"emoji_status":null
            })),
            Some(TelegramUpdate::ChatEmojiStatus(42, None))
        ));
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
