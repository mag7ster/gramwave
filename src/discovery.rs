use crate::{
    model::{track, Playlist},
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
        let c = response(
            reply
                .recv_timeout(Duration::from_secs(60))
                .map_err(|e| e.to_string())?,
        )?;
        if c["type"]["@type"] == "chatTypeSecret" {
            continue;
        }
        chats.push(Playlist {
            id: c["id"].as_i64().unwrap_or(0),
            title: c["title"].as_str().unwrap_or("").into(),
            filters,
            image_file: c["photo"]["small"]["id"].as_i64().unwrap_or(0),
            ..Default::default()
        });
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
