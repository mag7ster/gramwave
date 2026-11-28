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
            .filter(|t| {
                usize::from(t.kind) == kind
                    && (p.done[kind] || t.message > p.oldest_cursors[kind] || p.cursors[kind] != 0)
            })
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

pub(crate) async fn media_page(
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
    tg.search_media(chat, kind, cursor, 0, cancel).await
}

pub(crate) async fn oldest_media_page(
    tg: &Telegram,
    chat: i64,
    kind: usize,
    cursor: i64,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<gramwave_application::telegram::MediaPage, AppError> {
    use gramwave_application::telegram::TelegramGateway;
    let kind = [
        crate::domain::MediaKind::Audio,
        crate::domain::MediaKind::Document,
        crate::domain::MediaKind::Voice,
    ]
    .get(kind)
    .copied()
    .ok_or_else(|| AppError::Protocol("Invalid media filter".into()))?;
    ascending_page(cursor, |from, offset| {
        tg.search_media(chat, kind, from, offset, cancel)
    })
    .await
}

async fn ascending_page<
    F: std::future::Future<Output = Result<gramwave_application::telegram::MediaPage, AppError>>,
>(
    after: i64,
    mut search: impl FnMut(i64, i32) -> F,
) -> Result<gramwave_application::telegram::MediaPage, AppError> {
    use gramwave_application::telegram::MediaPage;

    // TDLib encodes server message IDs in units of 2²⁰. Zero searches from the newest.
    const FIRST_SERVER_MESSAGE: i64 = 1 << 20;
    let window = match search(after.max(FIRST_SERVER_MESSAGE), -99).await {
        Ok(page) => Some(page),
        Err(AppError::Telegram { code: 400, .. } | AppError::Protocol(_)) => None,
        Err(error) => return Err(error),
    };
    if let Some(window) = window.filter(|page| page.newest > after) {
        let high = window.newest;
        let mut tracks = window.tracks;
        let mut cursor = high;
        // A short ascending window can omit the oldest rows in that window.
        for _ in 0..3 {
            let page = search(cursor, 0).await?;
            tracks.extend(page.tracks);
            if page.next == 0 || page.next <= after {
                tracks.retain(|track| (after < track.message) && (track.message <= high));
                tracks.sort_by_key(|track| track.message);
                tracks.dedup_by_key(|track| track.message);
                return Ok(MediaPage {
                    newest: high,
                    tracks,
                    next: high,
                });
            }
            if page.next >= cursor {
                break;
            }
            cursor = page.next;
        }
    }

    let mut cursor = 0;
    let mut newest = 0;
    let mut tracks = Vec::new();
    loop {
        let page = search(cursor, 0).await?;
        newest = newest.max(page.newest);
        tracks.extend(
            page.tracks
                .into_iter()
                .filter(|track| track.message > after),
        );
        if page.next == 0 || page.next <= after || (page.newest != 0 && page.newest <= after) {
            tracks.sort_by_key(|track| track.message);
            tracks.dedup_by_key(|track| track.message);
            return Ok(MediaPage {
                newest,
                tracks,
                next: 0,
            });
        }
        cursor = page.next;
    }
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

pub(crate) async fn shuffle_sample(
    tg: &Telegram,
    chat: i64,
    media: [bool; 3],
    ranks: Option<gramwave_application::telegram::ShuffleRanks>,
    range: Option<(gramwave_domain::PlaylistOrder, i64)>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<gramwave_application::telegram::ShuffleSample, AppError> {
    use gramwave_application::telegram::ShuffleSample;
    let ranks = if let Some(ranks) = ranks {
        ranks
    } else {
        let replies = futures_util::future::join_all(MEDIA_FILTERS.into_iter().zip(media).map(
            |(filter, enabled)| async move {
                if !enabled {
                    return (0, Vec::new());
                }
                let result = tg
                    .call_async(
                        json!({
                            "@type": "getChatSparseMessagePositions", "chat_id": chat,
                            "filter": {"@type": filter}, "from_message_id": 0, "limit": 50,
                            "saved_messages_topic_id": 0,
                        }),
                        cancel,
                    )
                    .await;
                let Ok(value) = result else {
                    return (0, Vec::new());
                };
                let total = value["total_count"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or_default();
                let positions = value["positions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|row| {
                        let rank = u32::try_from(row["position"].as_u64()?).ok()?;
                        let id = row["message_id"].as_i64()?;
                        (rank < total && id > 0).then_some((rank, id))
                    })
                    .collect();
                (total, positions)
            },
        ))
        .await;
        let mut replies = replies.into_iter();
        std::array::from_fn(|_| replies.next().unwrap_or_default())
    };
    let ids = shuffle_sample_ids(&ranks, range, || getrandom::u64().unwrap_or_default());
    let tracks = if ids.is_empty() {
        Vec::new()
    } else {
        let result = tg
            .call_async(
                json!({"@type":"getMessages", "chat_id":chat,"message_ids":ids}),
                cancel,
            )
            .await?;
        let mut by_id: std::collections::HashMap<_, _> = result["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|message| track(chat, message))
            .filter(|track| media[usize::from(track.kind)])
            .map(|track| (track.message, track))
            .collect();
        ids.into_iter().filter_map(|id| by_id.remove(&id)).collect()
    };
    Ok(ShuffleSample { ranks, tracks })
}

fn shuffle_sample_ids(
    ranks: &gramwave_application::telegram::ShuffleRanks,
    range: Option<(gramwave_domain::PlaylistOrder, i64)>,
    mut random: impl FnMut() -> u64,
) -> Vec<i64> {
    let mut bands: [Vec<_>; 4] = Default::default();
    let mut seen = std::collections::HashSet::new();
    for (total, positions) in ranks {
        let eligible: Vec<_> = positions
            .iter()
            .copied()
            .filter(|&(_, id)| {
                range.is_none_or(|(order, ceiling)| order.key(id) >= order.key(ceiling))
            })
            .collect();
        let (first, end) = if range.is_some() && !eligible.is_empty() {
            (
                eligible.iter().map(|(rank, _)| *rank).min().unwrap(),
                eligible.iter().map(|(rank, _)| *rank).max().unwrap() + 1,
            )
        } else {
            (0, *total)
        };
        for (rank, id) in eligible {
            if rank >= first && rank < end && seen.insert(id) {
                let band = (u64::from(rank - first) * 4 / u64::from(end - first)) as usize;
                bands[band].push((random(), id));
            }
        }
    }
    for band in &mut bands {
        band.sort_unstable();
    }
    let mut ids = Vec::new();
    for _ in 0..4 {
        let mut round: Vec<_> = bands
            .iter_mut()
            .filter_map(|band| band.pop().map(|(_, id)| (random(), id)))
            .collect();
        round.sort_unstable();
        ids.extend(round.into_iter().map(|(_, id)| id));
    }
    ids
}
