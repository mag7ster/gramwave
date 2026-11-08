use crate::{
    adapters::telegram_mapping::track,
    model::{Playlist, Track},
    telegram::Telegram,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
fn response(v: Value) -> Result<Value, String> {
    if v["@type"] == "error" {
        Err(v["message"].as_str().unwrap_or("Telegram error").into())
    } else {
        Ok(v)
    }
}

#[derive(Default)]
pub struct Changes {
    pub chat: i64,
    pub tracks: Vec<Track>,
    pub removed: Vec<i64>,
    pub url: Option<String>,
    pub pagination: [Option<(i64, bool)>; 3],
}
impl Changes {
    pub fn apply(self, p: &mut Playlist, protected: &std::collections::HashSet<i64>) {
        p.tracks
            .retain(|t| protected.contains(&t.message) || !self.removed.contains(&t.message));
        p.merge_tracks(self.tracks, protected);
        if let Some(url) = self.url {
            p.url = url;
        }
        for (kind, progress) in self.pagination.into_iter().enumerate() {
            if let Some((cursor, done)) = progress {
                p.cursors[kind] = cursor;
                p.done[kind] = done;
            }
        }
    }
}

pub fn refresh(
    p: Playlist,
    epoch: &AtomicU64,
    generation: u64,
    mut call: impl FnMut(Value) -> Result<Value, String>,
    mut emit: impl FnMut(Changes),
) -> Result<(), String> {
    let current = || epoch.load(Ordering::Relaxed) == generation;
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
                return Ok(());
            }
            let batch = response(call(
                json!({"@type":"searchChatMessages", "chat_id":p.id,"topic_id":null,"query":"","sender_id":null,"from_message_id":cursor,"offset":0,"limit":100,"filter":{"@type":filter}}),
            )?)?;
            if !current() {
                return Ok(());
            }
            let messages = batch["messages"]
                .as_array()
                .ok_or("Invalid search response")?;
            let next = messages.last().and_then(|m| m["id"].as_i64()).unwrap_or(0);
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
            if next == 0 || next == cursor || newest.is_none_or(|id| next <= id) {
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
            return Ok(());
        }
        let batch = response(call(
            json!({"@type":"getMessages", "chat_id":p.id,"message_ids":chunk}),
        )?)?;
        if !current() {
            return Ok(());
        }
        let messages = batch["messages"]
            .as_array()
            .filter(|m| m.len() == chunk.len())
            .ok_or("Invalid messages response")?;
        let mut changes = Changes {
            chat: p.id,
            ..Default::default()
        };
        for (id, m) in chunk.iter().zip(messages) {
            if !m.is_null() && m["id"].as_i64() != Some(*id) {
                return Err("Unexpected message identity".into());
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
pub fn chat_metadata(c: &Value, filters: [bool; 3]) -> Option<Playlist> {
    if c["type"]["@type"] == "chatTypeSecret" {
        return None;
    }
    Some(Playlist {
        id: c["id"].as_i64()?,
        title: c["title"].as_str()?.into(),
        filters,
        image_file: c["photo"]["small"]["id"].as_i64().unwrap_or(0),
        ..Default::default()
    })
}
pub fn search_chats(
    tg: &Telegram,
    query: &str,
    filters: [bool; 3],
) -> Result<Vec<Playlist>, String> {
    let mut ids = std::collections::HashSet::new();
    let mut chats = Vec::new();
    let mut error = None;
    for method in ["searchChats", "searchChatsOnServer"] {
        match tg.call(json!({"@type":method,"query":query,"limit":100})) {
            Ok(v) => {
                for id in v["chat_ids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_i64)
                {
                    if ids.insert(id) {
                        match tg.call(json!({"@type":"getChat","chat_id":id})) {
                            Ok(c) => {
                                if let Some(p) = chat_metadata(&c, filters) {
                                    chats.push(p);
                                }
                            }
                            Err(e) => error = Some(e),
                        }
                    }
                }
            }
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
pub fn catalog(tg: &Telegram, filters: [bool; 3]) -> Result<Vec<Playlist>, String> {
    let mut ids = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for archive in [false, true] {
        let list = json!({"@type":if archive{"chatListArchive"}else{"chatListMain"}});
        loop {
            match tg.call(json!({"@type":"loadChats","chat_list":list,"limit":100})) {
                Ok(_) => {}
                Err(e) if e.contains("404") || e.contains("Not Found") => break,
                Err(e) => return Err(e),
            }
        }
        let chats = tg.call(json!({"@type":"getChats","chat_list":list,"limit":10000}))?;
        for id in chats["chat_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_i64)
        {
            if seen.insert(id) {
                ids.push(id);
            }
        }
    }
    let mut chats = Vec::new();
    for id in ids {
        let c = tg.call(json!({"@type":"getChat","chat_id":id}))?;
        if let Some(p) = chat_metadata(&c, filters) {
            chats.push(p);
        }
    }
    Ok(chats)
}
pub fn index(
    mut p: Playlist,
    epoch: &AtomicU64,
    generation: u64,
    mut call: impl FnMut(Value) -> Result<Value, String>,
    mut emit: impl FnMut(Playlist),
) -> Result<(), String> {
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
            if epoch.load(Ordering::Relaxed) != generation {
                return Ok(());
            }
            let batch = call(
                json!({"@type":"searchChatMessages","chat_id":p.id,"topic_id":null,"query":"","sender_id":null,"from_message_id":cursor,"offset":0,"limit":100,"filter":{"@type":filter}}),
            )?;
            if epoch.load(Ordering::Relaxed) != generation {
                return Ok(());
            }
            let messages = batch["messages"].as_array().cloned().unwrap_or_default();
            let next = messages.last().and_then(|m| m["id"].as_i64()).unwrap_or(0);
            for m in &messages {
                if let Some(t) = track(p.id, m) {
                    if !p.tracks.iter().any(|x| x.message == t.message) {
                        p.tracks.push(t)
                    }
                }
            }
            p.tracks.sort_by_key(|t| std::cmp::Reverse(t.message));
            if next == 0 || next == cursor {
                p.done[kind] = true;
            } else {
                p.cursors[kind] = next;
            }
            emit(p.clone());
        }
    }
    Ok(())
}
#[cfg(test)]
#[path = "discovery/tests.rs"]
mod tests;
