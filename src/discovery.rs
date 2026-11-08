use crate::{
    model::{track, Playlist, Track},
    telegram::Telegram,
};
use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
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
            .filter(|t| t.kind == kind)
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
    // Batch metadata requests; no message history or track-count request is made here.
    let replies: Vec<_> = ids
        .into_iter()
        .map(|id| tg.request(json!({"@type":"getChat","chat_id":id})))
        .collect();
    let mut chats = Vec::new();
    for reply in replies {
        if let Ok(Ok(c)) = reply.recv_timeout(Duration::from_secs(60)).map(response) {
            if let Some(p) = chat_metadata(&c, filters) {
                chats.push(p);
            }
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
mod tests {
    use super::*;
    #[test]
    fn only_opened_chat_is_indexed() {
        let epoch = AtomicU64::new(1);
        let mut queries = Vec::new();
        let p = Playlist {
            id: 42,
            filters: [true; 3],
            ..Default::default()
        };
        index(
            p,
            &epoch,
            1,
            |v| {
                queries.push(v);
                Ok(json!({"messages":[]}))
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(queries.len(), 3);
        assert!(queries
            .iter()
            .all(|q| q["chat_id"] == 42 && q["@type"] == "searchChatMessages"));
    }
    #[test]
    fn switching_stops_pagination() {
        let epoch = AtomicU64::new(1);
        let mut calls = 0;
        index(
            Playlist {
                id: 7,
                filters: [true; 3],
                ..Default::default()
            },
            &epoch,
            1,
            |_| {
                calls += 1;
                epoch.store(2, Ordering::Relaxed);
                Ok(json!({"messages":[{"id":5}]}))
            },
            |_| panic!("stale page"),
        )
        .unwrap();
        assert_eq!(calls, 1);
    }
}

#[cfg(test)]
mod lazy_tests {
    use super::*;
    #[test]
    fn opening_emptied_index_reloads_from_first_page() {
        let mut p: Playlist = serde_json::from_value(json!({
            "id":2,"tracks":[],"filters":[true,false,false],
            "cursors":[1570766848,737148928,0],"done":[true,true,false]
        }))
        .unwrap();
        assert!(!p.has_more());
        p.recover_empty_index();
        assert!(p.has_more());
        let mut page = None;
        let mut calls = 0;
        index(p, &AtomicU64::new(1), 1, |v| {
            calls += 1;
            assert_eq!(v["chat_id"], 2);
            assert_eq!(v["from_message_id"], 0);
            Ok(json!({"messages":[{"id":100,"content":{"@type":"messageAudio","audio":{"title":"Song","audio":{"id":90}}}}]}))
        }, |p| page = Some(p)).unwrap();
        assert_eq!(calls, 1);
        let page = page.unwrap();
        assert_eq!(page.tracks.len(), 1);
        assert_eq!(page.tracks[0].title, "Song");
        assert_eq!(page.cursors[0], 100);
    }
    #[test]
    fn catalog_never_reads_history_or_counts() {
        let tg = Telegram::mock_rpc(|v| match v["@type"].as_str().unwrap() {
            "loadChats" => json!({"@type":"error","message":"Not Found"}),
            "getChats" => json!({"chat_ids":[1,2]}),
            "getChat" => {
                json!({"id":v["chat_id"],"title":"Chat","type":{"@type":"chatTypePrivate"}})
            }
            other => panic!("Unexpected startup request: {other}"),
        });
        let chats = catalog(&tg, [true, true, false]).unwrap();
        assert_eq!(chats.len(), 2);
        assert!(chats.iter().all(|p| p.tracks.is_empty()));
    }
    #[test]
    fn one_page_per_enabled_type() {
        let epoch = AtomicU64::new(1);
        let mut calls = 0;
        let mut page = None;
        index(
            Playlist {
                id: 5,
                filters: [true, false, false],
                ..Default::default()
            },
            &epoch,
            1,
            |_| {
                calls += 1;
                Ok(json!({"messages":[{"id":100}]}))
            },
            |p| page = Some(p),
        )
        .unwrap();
        assert_eq!(calls, 1);
        let p = page.unwrap();
        assert_eq!(p.cursors[0], 100);
        assert!(p.has_more());
        assert!(!p.done[0]);
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    use std::collections::HashSet;
    fn message(id: i64, title: &str) -> Value {
        json!({"id":id,"content":{"@type":"messageAudio","audio":{"title":title,"audio":{"id":id+1}}}})
    }
    fn library() -> Playlist {
        Playlist {
            id: 2,
            tracks: vec![
                track(2, &message(200, "Old title")).unwrap(),
                track(2, &message(100, "Older song")).unwrap(),
            ],
            filters: [true, false, false],
            cursors: [100, 0, 0],
            done: [true; 3],
            ..Default::default()
        }
    }
    #[test]
    fn completed_playlist_refreshes_additions_edits_and_deletions() {
        let mut displayed = library();
        displayed.tracks[0].pinned = Some("/offline/song".into());
        refresh(displayed.clone(), &AtomicU64::new(1), 1, |v| {
            assert_eq!(v["chat_id"],2);
            match v["@type"].as_str().unwrap() {
                "searchChatMessages" => { assert_eq!(v["from_message_id"],0); Ok(json!({"messages":[message(300,"New song"),message(200,"Edited title")]})) },
                "getMessages" => { assert_eq!(v["message_ids"],json!([200,100])); Ok(json!({"messages":[message(200,"Edited title"),null]})) },
                other => panic!("Unexpected RPC {other}")
            }
        }, |changes| changes.apply(&mut displayed,&HashSet::new())).unwrap();
        assert_eq!(
            displayed
                .tracks
                .iter()
                .map(|t| t.message)
                .collect::<Vec<_>>(),
            vec![300, 200]
        );
        assert_eq!(displayed.tracks[1].title, "Edited title");
        assert_eq!(displayed.tracks[1].pinned.as_deref(), Some("/offline/song"));
        assert_eq!(displayed.cursors, [100, 0, 0]);
        assert_eq!(displayed.done, [true; 3]);
    }
    #[test]
    fn partial_head_and_failed_verification_do_not_erase_history() {
        for fail in [false, true] {
            let mut displayed = library();
            let result = refresh(
                displayed.clone(),
                &AtomicU64::new(1),
                1,
                |v| {
                    if v["@type"] == "searchChatMessages" {
                        Ok(json!({"messages":[message(300,"New"),message(200,"Old title")]}))
                    } else if fail {
                        Err("Network unavailable".into())
                    } else {
                        Ok(json!({"messages":[message(200,"Old title"),message(100,"Older song")]}))
                    }
                },
                |changes| changes.apply(&mut displayed, &HashSet::new()),
            );
            assert_eq!(result.is_err(), fail);
            assert_eq!(displayed.tracks.len(), 3);
            assert!(displayed.tracks.iter().any(|t| t.message == 100));
        }
    }
    #[test]
    fn refresh_catches_up_multiple_new_pages_without_moving_history_cursor() {
        let mut displayed = library();
        let mut searches = Vec::new();
        refresh(
            displayed.clone(),
            &AtomicU64::new(1),
            1,
            |v| {
                if v["@type"] == "getMessages" {
                    return Ok(
                        json!({"messages":[message(200,"Old title"),message(100,"Older song")]}),
                    );
                }
                let cursor = v["from_message_id"].as_i64().unwrap();
                searches.push(cursor);
                Ok(if cursor == 0 {
                    json!({"messages":[message(500,"New"),message(400,"New")]})
                } else {
                    json!({"messages":[message(400,"New"),message(200,"Old title")]})
                })
            },
            |changes| changes.apply(&mut displayed, &HashSet::new()),
        )
        .unwrap();
        assert_eq!(searches, vec![0, 400]);
        assert_eq!(displayed.tracks.len(), 4);
        assert_eq!(displayed.cursors, [100, 0, 0]);
    }
    #[test]
    fn switching_chat_cancels_refresh_before_applying_response() {
        let epoch = AtomicU64::new(1);
        refresh(
            library(),
            &epoch,
            1,
            |_| {
                epoch.store(2, Ordering::Relaxed);
                Ok(json!({"messages":[]}))
            },
            |_| panic!("Stale response"),
        )
        .unwrap();
    }
    #[test]
    fn malformed_verification_cannot_delete_tracks() {
        let mut displayed = library();
        assert!(refresh(
            displayed.clone(),
            &AtomicU64::new(1),
            1,
            |v| {
                Ok(if v["@type"] == "getMessages" {
                    json!({"messages":[null]})
                } else {
                    json!({"messages":[]})
                })
            },
            |changes| changes.apply(&mut displayed, &HashSet::new())
        )
        .is_err());
        assert_eq!(displayed.tracks.len(), 2);
    }
    #[test]
    fn live_updates_win_over_an_older_background_snapshot() {
        let mut displayed = library();
        displayed.tracks.retain(|t| t.message != 200);
        displayed.tracks[0].title = "Live edit".into();
        let changes = Changes {
            chat: 2,
            tracks: vec![track(2, &message(200, "Deleted already")).unwrap()],
            removed: vec![100],
            ..Default::default()
        };
        changes.apply(&mut displayed, &HashSet::from([100, 200]));
        assert_eq!(displayed.tracks.len(), 1);
        assert_eq!(displayed.tracks[0].title, "Live edit");
    }
}

#[cfg(test)]
mod chat_search_tests {
    use super::*;
    #[test]
    fn server_finds_uncached_cyrillic_channel_and_deduplicates() {
        let tg = Telegram::mock_rpc(|v| match v["@type"].as_str().unwrap() {
            "searchChats" => {
                assert_eq!(v["query"], "РАДИО");
                json!({"chat_ids":[1]})
            }
            "searchChatsOnServer" => json!({"chat_ids":[1,2,3]}),
            "getChat" if v["chat_id"] == 3 => json!({"@type":"error","message":"unavailable"}),
            "getChat" => {
                json!({"id":v["chat_id"],"title":"Радио канал","type":{"@type":"chatTypeSupergroup"}})
            }
            _ => panic!("unexpected request {v}"),
        });
        let chats = search_chats(&tg, "РАДИО", [true; 3]).unwrap();
        assert_eq!(chats.iter().map(|p| p.id).collect::<Vec<_>>(), [1, 2]);
        assert!(chats.iter().all(|p| p.tracks.is_empty()));
    }
}
