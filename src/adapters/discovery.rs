use crate::{adapters::telegram_mapping::track, model::Playlist, telegram::Telegram};
use gramwave_application::{
    library::ChatCatalog, media::MediaPlaylist, telegram::is_channel_direct_messages, AppError,
};
use serde_json::{json, Value};
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
fn response(v: Value) -> Result<Value, AppError> {
    if v["@type"] == "error" {
        Err(AppError::Telegram {
            code: v["code"].as_i64().unwrap_or(500) as i32,
            message: v["message"].as_str().unwrap_or("Telegram error").into(),
        })
    } else {
        Ok(v)
    }
}

pub(crate) const MEDIA_FILTERS: [&str; 3] = [
    "searchMessagesFilterAudio",
    "searchMessagesFilterDocument",
    "searchMessagesFilterVoiceNote",
];

pub(crate) async fn warm_playlist_head(
    tg: &Telegram,
    chat: i64,
    kind: usize,
    reserved: &super::telegram::PlaylistHead,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Vec<gramwave_application::media::MediaTrack>, AppError> {
    let batch = tg.request_reserved_head(
        json!({"@type":"searchChatMessages","chat_id":chat,"topic_id":null,"query":"","sender_id":null,"from_message_id":0,"offset":0,"limit":100,"filter":{"@type":MEDIA_FILTERS[kind]}}),
        reserved,
        cancel,
    ).await?;
    let tracks = head_tracks(chat, &batch)?.collect();
    Ok(tracks)
}

fn head_tracks(
    chat: i64,
    batch: &Value,
) -> Result<impl Iterator<Item = gramwave_application::media::MediaTrack> + '_, AppError> {
    let (messages, _) = search_page(batch, 0)?;
    Ok(messages
        .iter()
        .filter_map(move |message| track(chat, message)))
}

pub(crate) fn ready_playlist_tracks(
    tg: &Telegram,
    playlist: &Playlist,
) -> Vec<gramwave_application::media::MediaTrack> {
    let mut tracks = Vec::new();
    for (kind, filter) in MEDIA_FILTERS.into_iter().enumerate() {
        if playlist.filters[kind] {
            if let Some(batch) = tg.ready_playlist_head(playlist.id, filter) {
                if let Ok(head) = head_tracks(playlist.id, &batch) {
                    tracks.extend(head);
                }
            }
        }
    }
    tracks
}

fn search_page(batch: &Value, cursor: i64) -> Result<(&[Value], i64), AppError> {
    let messages = batch["messages"]
        .as_array()
        .ok_or_else(|| AppError::Protocol("Invalid search response".into()))?;
    let next = batch["next_from_message_id"]
        .as_i64()
        .ok_or_else(|| AppError::Protocol("Invalid search cursor".into()))?;
    if next != 0 && next == cursor {
        return Err(AppError::Protocol("Search cursor did not advance".into()));
    }
    Ok((messages, next))
}
fn chat_ids(batch: &Value) -> Result<Vec<i64>, AppError> {
    batch["chat_ids"]
        .as_array()
        .ok_or_else(|| AppError::Protocol("Invalid chat list response".into()))?
        .iter()
        .map(|id| {
            id.as_i64()
                .ok_or_else(|| AppError::Protocol("Invalid chat identity".into()))
        })
        .collect()
}
pub(super) fn chat_response(
    chat: &Value,
    filters: [bool; 3],
) -> Result<Option<MediaPlaylist>, AppError> {
    if chat["type"]["@type"] == "chatTypeSecret"
        || chat["id"].as_i64().is_some_and(is_channel_direct_messages)
    {
        return Ok(None);
    }
    chat_metadata(chat, filters)
        .map(Some)
        .ok_or_else(|| AppError::Protocol("Invalid chat metadata".into()))
}

pub use gramwave_application::media::MediaPatch as Changes;

async fn refresh_async<F: std::future::Future<Output = Result<Value, AppError>>>(
    p: Playlist,
    current: impl Fn() -> bool,
    mut call: impl FnMut(Value) -> F,
    mut emit: impl FnMut(Changes),
) -> Result<(), AppError> {
    for (kind, filter) in [
        "searchMessagesFilterAudio",
        "searchMessagesFilterDocument",
        "searchMessagesFilterVoiceNote",
    ]
    .into_iter()
    .enumerate()
    {
        if !p.filters[kind] {
            continue;
        }
        let newest = p
            .tracks
            .iter()
            .filter(|t| usize::from(t.kind) == kind)
            .map(|t| t.message)
            .max();
        let mut cursor = 0;
        loop {
            if !current() {
                return Err(AppError::Cancelled);
            }
            let batch = response(call(
                json!({"@type":"searchChatMessages", "chat_id":p.id,"topic_id":null,"query":"","sender_id":null,"from_message_id":cursor,"offset":0,"limit":100,"filter":{"@type":filter}}),
            ).await?)?;
            if !current() {
                return Err(AppError::Cancelled);
            }
            let (messages, next) = search_page(&batch, cursor)?;
            let mut changes = Changes {
                chat: p.id,
                tracks: messages.iter().filter_map(|m| track(p.id, m)).collect(),
                ..Default::default()
            };
            // Refreshing the head never advances an existing history cursor.
            if cursor == 0 && p.cursors[kind] == 0 {
                changes.pagination[kind] = Some((next, next == 0));
            }
            emit(changes);
            if next == 0
                || newest.is_none_or(|id| {
                    messages
                        .iter()
                        .any(|m| m["id"].as_i64().is_some_and(|m| m <= id))
                })
            {
                break;
            }
            cursor = next;
        }
    }
    // Check only already indexed messages. Absence from one search page says
    // nothing about older tracks; a failed RPC must never erase them.
    let ids: Vec<_> = p.tracks.iter().map(|t| t.message).collect();
    for chunk in ids.chunks(100) {
        if !current() {
            return Err(AppError::Cancelled);
        }
        let batch = response(
            call(json!({"@type":"getMessages", "chat_id":p.id,"message_ids":chunk})).await?,
        )?;
        if !current() {
            return Err(AppError::Cancelled);
        }
        let messages = batch["messages"]
            .as_array()
            .filter(|m| m.len() == chunk.len())
            .ok_or_else(|| AppError::Protocol("Invalid messages response".into()))?;
        let mut changes = Changes {
            chat: p.id,
            ..Default::default()
        };
        for (id, m) in chunk.iter().zip(messages) {
            if !m.is_null() && m["id"].as_i64() != Some(*id) {
                return Err(AppError::Protocol("Unexpected message identity".into()));
            }
            if let Some(t) = track(p.id, m) {
                changes.tracks.push(t);
            } else {
                changes.removed.push(*id);
            }
        }
        emit(changes);
    }
    Ok(())
}
fn chat_metadata(c: &Value, filters: [bool; 3]) -> Option<MediaPlaylist> {
    Some(MediaPlaylist {
        id: c["id"].as_i64()?,
        title: c["title"].as_str()?.into(),
        emoji_status: super::telegram_mapping::emoji_status(&c["emoji_status"]),
        filters,
        image_file: c["photo"]["small"]["id"].as_i64().unwrap_or(0),
        image_path: super::telegram_mapping::local_path(&c["photo"]["small"]),
        ..Default::default()
    })
}
pub async fn search_chats(
    tg: &Telegram,
    query: &str,
    filters: [bool; 3],
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Vec<MediaPlaylist>, AppError> {
    let mut ids = std::collections::HashSet::new();
    let mut chats = Vec::new();
    let mut error = None;
    for method in ["searchChats", "searchChatsOnServer"] {
        match tg
            .call_async(json!({"@type":method,"query":query,"limit":100}), cancel)
            .await
        {
            Ok(v) => {
                for id in chat_ids(&v)? {
                    if !is_channel_direct_messages(id) && ids.insert(id) {
                        match tg
                            .call_async(json!({"@type":"getChat","chat_id":id}), cancel)
                            .await
                        {
                            Ok(c) => {
                                if let Some(p) = chat_response(&c, filters)? {
                                    chats.push(p);
                                }
                            }
                            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                            Err(e) => error = Some(e),
                        }
                    }
                }
            }
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(e) => error = Some(e),
        }
    }
    if chats.is_empty() {
        if let Some(e) = error {
            return Err(e);
        }
    }
    Ok(chats)
}
pub async fn catalog(
    tg: &Telegram,
    filters: [bool; 3],
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<ChatCatalog, AppError> {
    let mut ids = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut catalog = ChatCatalog::default();
    for (index, name) in ["chatListMain", "chatListArchive"].into_iter().enumerate() {
        let list = json!({"@type":name});
        loop {
            match tg
                .call_async(
                    json!({"@type":"loadChats","chat_list":list,"limit":100}),
                    cancel,
                )
                .await
            {
                Ok(_) => {}
                Err(AppError::Telegram { code: 404, .. }) => break,
                Err(e) => return Err(e),
            }
        }
        let chats = tg
            .call_async(
                json!({"@type":"getChats","chat_list":list,"limit":10000}),
                cancel,
            )
            .await?;
        for id in chat_ids(&chats)? {
            if is_channel_direct_messages(id) {
                continue;
            }
            catalog.lists[index].insert(id);
            if seen.insert(id) {
                ids.push(id);
            }
        }
    }
    for id in ids {
        let c = tg
            .call_async(json!({"@type":"getChat","chat_id":id}), cancel)
            .await?;
        if let Some(p) = chat_response(&c, filters)? {
            if let Some(lists) = super::telegram_mapping::personal_chat_lists(&c) {
                for (list, member) in catalog.lists.iter_mut().zip(lists) {
                    if member {
                        list.insert(id);
                    } else {
                        list.remove(&id);
                    }
                }
            }
            if catalog.lists.iter().any(|list| list.contains(&id)) {
                catalog.chats.push(p);
            }
        } else {
            for list in &mut catalog.lists {
                list.remove(&id);
            }
        }
    }
    let me = tg
        .call_async(json!({"@type":"getOption","name":"my_id"}), cancel)
        .await?;
    catalog.saved_messages =
        super::telegram_mapping::telegram_int64(&me["value"]).filter(|&id| id > 0);
    if let Some(id) = catalog
        .saved_messages
        .filter(|&id| !catalog.chats.iter().any(|p| p.id == id))
    {
        match tg
            .call_async(json!({"@type":"getChat","chat_id":id}), cancel)
            .await
        {
            Ok(chat) => {
                if let Some(playlist) = chat_response(&chat, filters)? {
                    catalog.chats.push(playlist);
                }
            }
            Err(AppError::Telegram {
                code: 400 | 404, ..
            }) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(catalog)
}
async fn index_async<F: std::future::Future<Output = Result<Value, AppError>>>(
    p: Playlist,
    current: impl Fn() -> bool,
    mut call: impl FnMut(Value) -> F,
    mut emit: impl FnMut(MediaPlaylist),
) -> Result<(), AppError> {
    let mut p = MediaPlaylist::from(p);
    for (kind, filter) in [
        "searchMessagesFilterAudio",
        "searchMessagesFilterDocument",
        "searchMessagesFilterVoiceNote",
    ]
    .iter()
    .enumerate()
    {
        if p.done[kind] || !p.filters[kind] {
            continue;
        }
        let cursor = p.cursors[kind];
        {
            if !current() {
                return Err(AppError::Cancelled);
            }
            let batch = response(call(
                json!({"@type":"searchChatMessages","chat_id":p.id,"topic_id":null,"query":"","sender_id":null,"from_message_id":cursor,"offset":0,"limit":100,"filter":{"@type":filter}}),
            ).await?)?;
            if !current() {
                return Err(AppError::Cancelled);
            }
            let (messages, next) = search_page(&batch, cursor)?;
            for m in messages {
                if let Some(t) = track(p.id, m) {
                    if !p.tracks.iter().any(|x| x.message == t.message) {
                        p.tracks.push(t)
                    }
                }
            }
            p.tracks.sort_by_key(|t| std::cmp::Reverse(t.message));
            p.cursors[kind] = next;
            p.done[kind] = next == 0;
            emit(p.clone());
        }
    }
    Ok(())
}
#[cfg(test)]
#[path = "discovery/tests.rs"]
mod tests;

/// An offline search page has an independent cursor from the displayed library.
pub(crate) async fn offline_page(
    tg: &Telegram,
    chat: i64,
    kind: usize,
    cursor: i64,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<gramwave_application::telegram::MediaPage, AppError> {
    use gramwave_application::telegram::TelegramGateway;
    let kind = match kind {
        0 => crate::domain::MediaKind::Audio,
        1 => crate::domain::MediaKind::Document,
        2 => crate::domain::MediaKind::Voice,
        _ => return Err(AppError::Protocol("Invalid media filter".into())),
    };
    tg.search_media(chat, kind, cursor, cancel).await
}

#[cfg(test)]
fn refresh(
    p: Playlist,
    epoch: &AtomicU64,
    generation: u64,
    mut call: impl FnMut(Value) -> Result<Value, AppError>,
    emit: impl FnMut(Changes),
) -> Result<(), AppError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(refresh_async(
            p,
            || epoch.load(Ordering::Relaxed) == generation,
            |v| std::future::ready(call(v)),
            emit,
        ))
}
#[cfg(test)]
fn index(
    p: Playlist,
    epoch: &AtomicU64,
    generation: u64,
    mut call: impl FnMut(Value) -> Result<Value, AppError>,
    emit: impl FnMut(MediaPlaylist),
) -> Result<(), AppError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(index_async(
            p,
            || epoch.load(Ordering::Relaxed) == generation,
            |v| std::future::ready(call(v)),
            emit,
        ))
}

pub(crate) async fn refresh_library(
    tg: &Telegram,
    p: Playlist,
    cancel: &tokio_util::sync::CancellationToken,
    emit: impl FnMut(Changes),
) -> Result<(), AppError> {
    refresh_async(
        p,
        || !cancel.is_cancelled(),
        |value| async move {
            if value["@type"] == "searchChatMessages" && value["from_message_id"] == 0 {
                tg.playlist_head(value, cancel).await
            } else {
                tg.call_async(value, cancel).await
            }
        },
        emit,
    )
    .await
}
pub(crate) async fn index_library(
    tg: &Telegram,
    p: Playlist,
    cancel: &tokio_util::sync::CancellationToken,
    emit: impl FnMut(MediaPlaylist),
) -> Result<(), AppError> {
    index_async(
        p,
        || !cancel.is_cancelled(),
        |value| tg.call_async(value, cancel),
        emit,
    )
    .await
}
