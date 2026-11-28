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
                Ok(json!({"next_from_message_id":0,"messages":[]}))
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
        let result = index(
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
                Ok(json!({"next_from_message_id":0,"messages":[{"id":5}]}))
            },
            |_| panic!("stale page"),
        );
        assert_eq!(result, Err(AppError::Cancelled));
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
            Ok(json!({"next_from_message_id":100,"messages":[{"id":100,"content":{"@type":"messageAudio","audio":{"title":"Song","audio":{"id":90}}}}]}))
        }, |p| page = Some(p)).unwrap();
        assert_eq!(calls, 1);
        let page = page.unwrap();
        assert_eq!(page.tracks.len(), 1);
        assert_eq!(page.tracks[0].title, "Song");
        assert_eq!(page.cursors[0], 100);
    }
    #[test]
    fn catalog_never_reads_history_or_counts() {
        const DIRECT_MESSAGES: i64 = -2_100_000_000_000;
        let tg = Telegram::mock_rpc(|v| match v["@type"].as_str().unwrap() {
            "loadChats" => json!({"@type":"error","code":404,"message":"Not Found"}),
            "getChats" if v["chat_list"]["@type"] == "chatListMain" => {
                json!({"chat_ids":[1,4,DIRECT_MESSAGES]})
            }
            "getChats" => json!({"chat_ids":[2]}),
            "getOption" if v["name"] == "my_id" => json!({"value":"3"}),
            "getChat" if v["chat_id"] == 4 => {
                json!({"id":4,"title":"Public chat","chat_lists":[],"positions":[{"list":{"@type":"chatListMain"},"order":"100"}]})
            }
            "getChat" if v["chat_id"] != DIRECT_MESSAGES => {
                json!({"id":v["chat_id"],"title":"Chat","type":{"@type":"chatTypePrivate"}})
            }
            other => panic!("Unexpected startup request: {other}"),
        });
        let catalog = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(catalog(&tg, [true, true, false], &Default::default()))
            .unwrap();
        assert_eq!(
            catalog.chats.iter().map(|p| p.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert!(catalog.chats.iter().all(|p| p.tracks.is_empty()));
        assert_eq!(catalog.lists, [[1].into(), [2].into()]);
        assert_eq!(catalog.saved_messages, Some(3));

        let temp = tempfile::tempdir().unwrap();
        let (mut store, state) =
            crate::adapters::storage::Store::open(&temp.path().join("db")).unwrap();
        use gramwave_application::{
            library::{LibraryCommand, LibraryEffect, LibraryEvent},
            workflow::Effect,
            AppCore,
        };
        let mut core = AppCore::new(state);
        core.library_command(LibraryCommand::Connected(1));
        let request = core
            .library_command(LibraryCommand::RefreshCatalog)
            .into_iter()
            .find_map(|effect| match effect {
                Effect::Library(LibraryEffect::Catalog { operation, .. }) => Some(operation.key()),
                _ => None,
            })
            .unwrap();
        for effect in core.library_event(LibraryEvent::Catalog {
            request,
            result: Ok(catalog),
        }) {
            if let Effect::Persist(changes) = effect {
                store.apply(*changes).unwrap();
            }
        }
        store.shutdown().unwrap();
        let (mut store, state) =
            crate::adapters::storage::Store::open(&temp.path().join("db")).unwrap();
        assert_eq!(state.sidebar_chats(true, "chat"), [0, 1, 2]);
        assert_eq!(state.playlists.len(), 3);
        store.shutdown().unwrap();
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
                Ok(json!({"next_from_message_id":100,"messages":[{"id":100}]}))
            },
            |p| page = Some(p),
        )
        .unwrap();
        assert_eq!(calls, 1);
        let p = Playlist::from(page.unwrap());
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
                track(2, &message(200, "Old title")).unwrap().into(),
                track(2, &message(100, "Older song")).unwrap().into(),
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
        refresh(displayed.clone(), &AtomicU64::new(1), 1, |v| {
            assert_eq!(v["chat_id"],2);
            match v["@type"].as_str().unwrap() {
                "searchChatMessages" => { assert_eq!(v["from_message_id"],0); Ok(json!({"next_from_message_id":0,"messages":[message(300,"New song"),message(200,"Edited title")]})) },
                "getMessages" => { assert_eq!(v["message_ids"],json!([200,100])); Ok(json!({"next_from_message_id":0,"messages":[message(200,"Edited title"),null]})) },
                other => panic!("Unexpected RPC {other}")
            }
        }, |changes| gramwave_application::core::LibraryPatch::from(changes).apply(&mut displayed,&HashSet::new())).unwrap();
        assert_eq!(
            displayed
                .tracks
                .iter()
                .map(|t| t.message)
                .collect::<Vec<_>>(),
            vec![300, 200]
        );
        assert_eq!(displayed.tracks[1].title, "Edited title");
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
                        Ok(
                            json!({"next_from_message_id":0,"messages":[message(300,"New"),message(200,"Old title")]}),
                        )
                    } else if fail {
                        Err(AppError::Unavailable("Network"))
                    } else {
                        Ok(
                            json!({"next_from_message_id":0,"messages":[message(200,"Old title"),message(100,"Older song")]}),
                        )
                    }
                },
                |changes| {
                    gramwave_application::core::LibraryPatch::from(changes)
                        .apply(&mut displayed, &HashSet::new())
                },
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
                        json!({"next_from_message_id":0,"messages":[message(200,"Old title"),message(100,"Older song")]}),
                    );
                }
                let cursor = v["from_message_id"].as_i64().unwrap();
                searches.push(cursor);
                Ok(if cursor == 0 {
                    json!({"next_from_message_id":400,"messages":[message(500,"New"),message(400,"New")]})
                } else {
                    json!({"next_from_message_id":0,"messages":[message(400,"New"),message(200,"Old title")]})
                })
            },
            |changes| gramwave_application::core::LibraryPatch::from(changes).apply(&mut displayed, &HashSet::new()),
        )
        .unwrap();
        assert_eq!(searches, vec![0, 400]);
        assert_eq!(displayed.tracks.len(), 4);
        assert_eq!(displayed.cursors, [100, 0, 0]);
    }
    #[test]
    fn switching_chat_cancels_refresh_before_applying_response() {
        let epoch = AtomicU64::new(1);
        let result = refresh(
            library(),
            &epoch,
            1,
            |_| {
                epoch.store(2, Ordering::Relaxed);
                Ok(json!({"next_from_message_id":0,"messages":[]}))
            },
            |_| panic!("Stale response"),
        );
        assert_eq!(result, Err(AppError::Cancelled));
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
                    json!({"next_from_message_id":0,"messages":[null]})
                } else {
                    json!({"next_from_message_id":0,"messages":[]})
                })
            },
            |changes| gramwave_application::core::LibraryPatch::from(changes)
                .apply(&mut displayed, &HashSet::new())
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
        gramwave_application::core::LibraryPatch::from(changes)
            .apply(&mut displayed, &HashSet::from([100, 200]));
        assert_eq!(displayed.tracks.len(), 1);
        assert_eq!(displayed.tracks[0].title, "Live edit");
    }
}

#[cfg(test)]
mod chat_search_tests {
    use super::*;
    #[test]
    fn server_finds_uncached_channel_with_unicode_query_and_deduplicates() {
        let tg = Telegram::mock_rpc(|v| match v["@type"].as_str().unwrap() {
            "searchChats" => {
                assert_eq!(v["query"], "CAFÉ");
                json!({"chat_ids":[1]})
            }
            "searchChatsOnServer" => json!({"chat_ids":[1,2,3]}),
            "getChat" if v["chat_id"] == 3 => json!({"@type":"error","message":"unavailable"}),
            "getChat" => {
                json!({"id":v["chat_id"],"title":"Café Radio","type":{"@type":"chatTypeSupergroup"}})
            }
            _ => panic!("unexpected request {v}"),
        });
        let chats = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(search_chats(&tg, "CAFÉ", [true; 3], &Default::default()))
            .unwrap();
        assert_eq!(chats.iter().map(|p| p.id).collect::<Vec<_>>(), [1, 2]);
        assert!(chats.iter().all(|p| p.tracks.is_empty()));
    }
}

#[test]
fn offline_search_follows_server_cursors_through_short_pages_for_ten_thousand_tracks() {
    let mut page = 0usize;
    let tg = Telegram::mock_rpc(move |request| {
        assert_eq!(request["@type"], "searchChatMessages");
        assert_eq!(request["query"], "");
        assert_eq!(request["filter"]["@type"], "searchMessagesFilterAudio");
        assert_eq!(
            request["from_message_id"].as_i64().unwrap(),
            if page == 0 {
                0
            } else {
                1_000_000 + page as i64
            }
        );
        let start = page * 50;
        page += 1;
        let messages:Vec<_>=(start..start+50).map(|id|json!({"id":id,"content":{"@type":"messageAudio","audio":{"title":"Track","audio":{"id":id+1}}}})).collect();
        json!({"messages":messages,"next_from_message_id":if page==200 {0}else{1_000_000+page as i64}})
    });
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut cursor = 0;
        let mut count = 0;
        loop {
            let page = media_page(&tg, 42, 0, cursor, &Default::default())
                .await
                .unwrap();
            assert_eq!(page.tracks.len(), 50);
            count += page.tracks.len();
            if page.next == 0 {
                break;
            }
            cursor = page.next;
        }
        assert_eq!(count, 10_000);
    });
}
#[test]
fn malformed_or_repeated_server_cursor_is_an_error() {
    for batch in [
        json!({"messages":[]}),
        json!({"messages":[],"next_from_message_id":10}),
    ] {
        assert!(index(
            Playlist {
                id: 1,
                filters: [true, false, false],
                cursors: [10, 0, 0],
                ..Default::default()
            },
            &AtomicU64::new(1),
            1,
            |_| Ok(batch.clone()),
            |_| panic!("invalid page applied")
        )
        .is_err());
    }
}

#[test]
fn filtered_non_music_page_keeps_server_watermark() {
    let tg = Telegram::mock_rpc(|request| {
        assert_eq!(request["filter"]["@type"], "searchMessagesFilterDocument");
        json!({"next_from_message_id":300,"messages":[{"id":500,"content":{"@type":"messageDocument","document":{"mime_type":"application/pdf","file_name":"notes.pdf","document":{"id":99}}}}]})
    });
    let page = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(media_page(&tg, 42, 1, 0, &Default::default()))
        .unwrap();
    assert!(page.tracks.is_empty());
    assert_eq!((page.next, page.newest), (300, 500));
}

#[test]
fn public_library_boundaries_preserve_telegram_codes_and_cancellation() {
    use tokio_util::sync::CancellationToken;
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let tg = Telegram::mock_rpc(|_| json!({"@type":"error","code":429,"message":"FLOOD_WAIT_3"}));
    let playlist = || Playlist {
        id: 42,
        filters: [true, false, false],
        ..Default::default()
    };
    for cancelled in [false, true] {
        let cancel = CancellationToken::new();
        if cancelled {
            cancel.cancel();
        }
        let expected = if cancelled {
            AppError::Cancelled
        } else {
            AppError::Telegram {
                code: 429,
                message: "FLOOD_WAIT_3".into(),
            }
        };
        runtime.block_on(async {
            assert_eq!(
                catalog(&tg, [true, false, false], &cancel)
                    .await
                    .unwrap_err(),
                expected
            );
            assert_eq!(
                search_chats(&tg, "radio", [true, false, false], &cancel)
                    .await
                    .unwrap_err(),
                expected
            );
            assert_eq!(
                index_library(&tg, playlist(), &cancel, |_| panic!("error page emitted"))
                    .await
                    .unwrap_err(),
                expected
            );
            assert_eq!(
                refresh_library(&tg, playlist(), &cancel, |_| panic!("error patch emitted"))
                    .await
                    .unwrap_err(),
                expected
            );
            assert_eq!(
                media_page(&tg, 42, 0, 0, &cancel).await.unwrap_err(),
                expected
            );
        });
    }
    assert_eq!(
        response(json!({"@type":"error","code":420,"message":"test"})),
        Err(AppError::Telegram {
            code: 420,
            message: "test".into()
        })
    );
}

#[test]
fn malformed_chat_lists_and_metadata_are_protocol_errors() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for invalid in [json!({}), json!({"chat_ids":["wrong type"]})] {
        let tg = Telegram::mock_rpc(move |request| {
            if request["@type"] == "loadChats" {
                json!({"@type":"error","code":404,"message":"end"})
            } else {
                invalid.clone()
            }
        });
        assert!(matches!(
            runtime.block_on(catalog(&tg, [true; 3], &Default::default())),
            Err(AppError::Protocol(_))
        ));
        assert!(matches!(
            runtime.block_on(search_chats(&tg, "radio", [true; 3], &Default::default())),
            Err(AppError::Protocol(_))
        ));
    }
    assert!(matches!(
        chat_response(&json!({"id":42}), [true; 3]),
        Err(AppError::Protocol(_))
    ));
    assert!(
        chat_response(&json!({"type":{"@type":"chatTypeSecret"}}), [true; 3])
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn oldest_media_verifies_short_windows_and_keeps_filtered_cursor_progress() {
    use gramwave_application::telegram::MediaPage;
    let mut calls = Vec::new();
    let result = ascending_page(100, |cursor, offset| {
        calls.push((cursor, offset));
        let (newest, next, ids): (_, _, &[_]) = match calls.len() {
            1 => (300, 0, &[300]), // A short window omitted 200.
            2 => (300, 250, &[300]),
            3 => (250, 80, &[200]), // 250 is a document that isn't playable.
            _ => panic!("unexpected request"),
        };
        std::future::ready(Ok(MediaPage {
            newest,
            next,
            tracks: ids
                .iter()
                .map(|&message| gramwave_application::media::MediaTrack {
                    message,
                    ..Default::default()
                })
                .collect(),
        }))
    })
    .await
    .unwrap();
    assert_eq!(calls, [(1 << 20, -99), (300, 0), (250, 0)]);
    assert_eq!(result.next, 300);
    assert_eq!(
        result.tracks.iter().map(|t| t.message).collect::<Vec<_>>(),
        [200, 300]
    );
}

#[tokio::test]
async fn empty_oldest_window_uses_descending_search_instead_of_ending_early() {
    use gramwave_application::telegram::MediaPage;
    let mut calls = Vec::new();
    let result = ascending_page(1 << 21, |cursor, offset| {
        calls.push((cursor, offset));
        let (newest, next, track) = match calls.len() {
            1 => (0, 0, None),
            2 => (6 << 20, 5 << 20, Some(6 << 20)),
            3 => (4 << 20, 1 << 20, Some(4 << 20)),
            _ => panic!("unexpected request"),
        };
        std::future::ready(Ok(MediaPage {
            newest,
            next,
            tracks: track
                .into_iter()
                .map(|message| gramwave_application::media::MediaTrack {
                    message,
                    ..Default::default()
                })
                .collect(),
        }))
    })
    .await
    .unwrap();
    assert_eq!(calls, [(1 << 21, -99), (0, 0), (5 << 20, 0)]);
    assert_eq!(result.next, 0);
    assert_eq!(
        result.tracks.iter().map(|t| t.message).collect::<Vec<_>>(),
        [4 << 20, 6 << 20]
    );
}

#[test]
fn sparse_shuffle_uses_all_four_quarters_before_repeating_any() {
    let positions = [
        (
            100,
            (0..50).map(|i| (i * 2, i64::from(1000 - i * 2))).collect(),
        ),
        (0, vec![]),
        (0, vec![]),
    ];
    let ids = super::shuffle_sample_ids(&positions, None, || 42);
    let mut quarters: Vec<_> = ids[..4].iter().map(|id| (1000 - id) / 25).collect();
    quarters.sort_unstable();
    assert_eq!(quarters, [0, 1, 2, 3]);
    assert_eq!(ids.len(), 16);
    let ids = super::shuffle_sample_ids(
        &positions,
        Some((gramwave_domain::PlaylistOrder::NewestFirst, 940)),
        || 42,
    );
    assert!(ids.iter().all(|id| *id <= 940));
}
