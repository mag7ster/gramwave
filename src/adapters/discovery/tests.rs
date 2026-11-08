use super::*;
#[cfg(test)]
mod regression {
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
