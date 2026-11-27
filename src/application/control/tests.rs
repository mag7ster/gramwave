use super::*;

fn app() -> App {
    let mut app = App::new_at(std::path::PathBuf::new(), true);
    app.opened_chat = Some(0);
    app
}
fn apply(app: &mut App, request: Request) -> Value {
    let reply = app.handle_control(request);
    assert!(reply.ok, "{:?}", reply.error);
    reply.data.unwrap()
}

#[test]
fn queue_ranges_resolve_in_the_visible_order_before_editing() {
    let mut app = app();
    let span = |first, last| RowRange { first, last };
    apply(&mut app, Request::Select { row: 2 });
    let before = app.state.view().session.clone();
    for request in [
        Request::QueueAdd {
            rows: vec![span(1, 2), span(3, 100)],
            next: true,
            reverse: false,
        },
        Request::QueueRemove {
            rows: vec![span(1, 2), span(4, usize::MAX)],
        },
        Request::QueueReverse { rows: span(4, 3) },
    ] {
        assert!(!app.handle_control(request).ok);
        assert_eq!(app.state.view().session, before);
    }

    apply(
        &mut app,
        Request::QueueAdd {
            rows: vec![span(1, 2), span(4, 4)],
            next: true,
            reverse: true,
        },
    );
    apply(
        &mut app,
        Request::QueueAdd {
            rows: vec![span(2, 3)],
            next: false,
            reverse: false,
        },
    );
    let queue = apply(&mut app, Request::Queue { offset: 0 });
    let upcoming = queue["items"].as_array().unwrap();
    assert_eq!(queue["current"]["row"], 2);
    assert_eq!(upcoming[0]["row"], 3);
    assert_eq!(upcoming[0]["message_id"], 3);
    assert_eq!(upcoming[1]["message_id"], 1);
    assert_eq!(upcoming[2]["message_id"], 0);
    assert_eq!(upcoming[upcoming.len() - 2]["message_id"], 1);
    assert_eq!(upcoming[upcoming.len() - 1]["message_id"], 2);
    assert_ne!(upcoming[1]["entry_id"], queue["current"]["entry_id"]);

    apply(&mut app, Request::Pause);
    let before = app.state.view().session.clone();
    let result = apply(
        &mut app,
        Request::QueueRemove {
            rows: vec![span(5, 5), span(1, 3), span(2, 2)],
        },
    );
    assert_eq!(result["current"]["entry_id"], upcoming[1]["entry_id"]);
    assert_eq!(result["paused"], true);
    let now = Instant::now();
    let notices: Vec<_> = app.state.playback().queue_removals(now).collect();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].count, 4);
    app.execute(AppCommand::UndoQueueRemoval {
        id: notices[0].id,
        now,
    });
    assert_eq!(app.state.view().session, before);
    let len = app.state.view().session.timeline().len();
    let done = apply(
        &mut app,
        Request::QueueRemove {
            rows: vec![span(1, len)],
        },
    );
    assert!(done["current"].is_null());
    assert_eq!(done["queue_length"], 0);
    app.shutdown_core().unwrap();
}

#[test]
fn queue_history_follows_the_timeline_and_can_be_reversed_and_played_by_row() {
    let mut app = app();
    apply(&mut app, Request::Select { row: 4 });
    apply(&mut app, Request::Next);
    let history = apply(&mut app, Request::Queue { offset: -20 });
    let messages = |items: &Value| {
        items
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["message_id"].as_i64().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(messages(&history["items"]), [0, 1, 2, 3]);
    assert_eq!(history["current"]["row"], 5);
    assert_eq!(
        messages(&apply(&mut app, Request::Queue { offset: -2 })["items"]),
        [2, 3]
    );
    apply(
        &mut app,
        Request::QueueReverse {
            rows: RowRange { first: 3, last: 8 },
        },
    );
    let history = apply(&mut app, Request::Queue { offset: -20 });
    assert_eq!(messages(&history["items"]), [0, 1, 7, 6, 5]);
    assert_eq!(history["current"]["row"], 6);
    assert_eq!(
        history["current"]["entry_id"],
        app.state.view().session.current().unwrap().entry_id.0
    );
    assert_eq!(history["current"]["message_id"], 4);
    assert_eq!(
        apply(&mut app, Request::QueuePlayRow { row: 6 })["current"]["message_id"],
        4
    );
    assert_eq!(
        apply(&mut app, Request::QueuePlayRow { row: 4 })["current"]["message_id"],
        6
    );
    app.shutdown_core().unwrap();
}

#[test]
fn search_opens_matching_chat_and_pin_commands_persist_desired_state() {
    use crate::adapters::storage::Store;
    use gramwave_application::persistence::ChangeSet;

    let mut app = app();
    let directory = tempfile::tempdir().unwrap();
    let db_path = directory.path().join("library.sqlite");
    let (store, _) = Store::open(&db_path).unwrap();
    store
        .apply(ChangeSet::between(None, app.state.view()))
        .unwrap();
    app.db = Some(Box::new(store));
    let found = apply(
        &mut app,
        Request::Search {
            query: " jAZz ".into(),
            offset: 0,
        },
    );
    assert_eq!(found["items"][0]["chat_id"], 2);
    assert_eq!(found["loading"], false);
    apply(&mut app, Request::Open { chat_id: 2 });
    assert_eq!(app.state.view().recent_chats.first(), Some(&2));
    for pinned in [false, false, true, true] {
        let data = apply(&mut app, Request::Pin { chat_id: 2, pinned });
        assert_eq!(data["pinned"], pinned);
    }
    let pinned = apply(&mut app, Request::Pinned { offset: 2 });
    assert_eq!(pinned["items"][0]["chat_id"], 2);
    app.shutdown_core().unwrap();
    let (_, restored) = Store::open(&db_path).unwrap();
    assert_eq!(restored.chat_pins, [0, 4, 2]);
    assert_eq!(restored.last_opened_chat, Some(2));
}

#[test]
fn playlist_selection_rebuilds_queue_and_previous_returns_to_preceding_row() {
    let mut app = app();
    apply(&mut app, Request::Select { row: 2 });
    let tracks = app.playlist_tracks(0);
    app.state
        .command(gramwave_application::commands::AppCommand::Insert(
            tracks[8].metadata(),
            true,
        ));
    let old_entry = app.state.view().session.remaining()[0].entry_id;
    apply(&mut app, Request::Select { row: 5 });
    assert_eq!(app.state.view().session.remaining().len(), 35);
    assert!(!app
        .state
        .view()
        .session
        .snapshot()
        .iter()
        .any(|t| t.entry_id == old_entry));
    for message in [3, 2] {
        let status = apply(&mut app, Request::Previous);
        assert_eq!(status["current"]["message_id"], message);
    }
    for message in [3, 4, 5] {
        let status = apply(&mut app, Request::Next);
        assert_eq!(status["current"]["message_id"], message);
    }
    app.shutdown_core().unwrap();
}

#[test]
fn queue_selection_preserves_occurrences_and_backwards_positions() {
    let mut app = app();
    apply(&mut app, Request::Select { row: 2 });
    let queue = apply(&mut app, Request::Queue { offset: 0 });
    let entry_id = serde_json::from_value(queue["items"][2]["entry_id"].clone()).unwrap();
    let status = apply(&mut app, Request::QueuePlay { entry_id });
    assert_eq!(status["current"]["message_id"], 4);
    assert_eq!(
        apply(&mut app, Request::Previous)["current"]["message_id"],
        3
    );
    assert_eq!(apply(&mut app, Request::Next)["current"]["message_id"], 4);
    app.shutdown_core().unwrap();
}

#[test]
fn list_and_select_share_filters_and_search_and_current_row_toggles_pause() {
    let mut app = app();
    let full = app.playlist_tracks(0);
    app.search = "Pacific".into();
    let list = apply(&mut app, Request::Playlist { offset: 0 });
    let expected = list["items"][1]["message_id"].clone();
    let status = apply(&mut app, Request::Select { row: 2 });
    assert_eq!(status["current"]["message_id"], expected);
    let index = full
        .iter()
        .position(|track| track.message == expected.as_i64().unwrap())
        .unwrap();
    let session = &app.state.view().session;
    assert_eq!(
        session.preceding()[index - 1].message,
        full[index - 1].message
    );
    assert_eq!(session.remaining()[0].message, full[index + 1].message);
    let id = app
        .state
        .view()
        .session
        .current()
        .as_ref()
        .unwrap()
        .entry_id;
    assert_eq!(apply(&mut app, Request::Select { row: 2 })["paused"], true);
    assert_eq!(
        app.state
            .view()
            .session
            .current()
            .as_ref()
            .unwrap()
            .entry_id,
        id
    );
    app.state
        .command(gramwave_application::commands::AppCommand::Filters(
            0, [false; 3],
        ));
    assert_eq!(apply(&mut app, Request::Playlist { offset: 0 })["total"], 0);
    assert!(!app.handle_control(Request::Select { row: 1 }).ok);
    app.shutdown_core().unwrap();
}

#[test]
fn pagination_handles_large_unicode_and_escaped_metadata() {
    let mut app = app();
    let mut fixture = app.state.view().clone();
    for track in &mut fixture.playlists[0].tracks {
        track.title = "\n\"\u{0001}🎵Café Étude".repeat(1000);
        track.artist = track.title.clone();
    }
    app.state = gramwave_application::AppCore::new(fixture);
    let mut offset = 0;
    let mut rows = Vec::new();
    loop {
        let reply = app.handle_control(Request::Playlist { offset });
        let encoded = serde_json::to_string(&reply).unwrap();
        assert!(encoded.len() < REPLY_LIMIT);
        let page = reply.data.unwrap();
        rows.extend(
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["row"].as_u64().unwrap()),
        );
        match page["next_offset"].as_u64() {
            Some(next) => {
                assert!(next as usize > offset);
                offset = next as usize;
            }
            None => break,
        }
    }
    assert_eq!(rows, (1..=40).collect::<Vec<_>>());
    assert!(!app.handle_control(Request::Playlist { offset: 41 }).ok);
    app.shutdown_core().unwrap();
}

#[test]
fn queue_pagination_advances_with_large_current_and_escaped_rows() {
    let mut app = app();
    let mut fixture = app.state.view().clone();
    for track in &mut fixture.playlists[0].tracks {
        track.title = "\u{0001}".repeat(512);
        track.artist = track.title.clone();
    }
    fixture
        .session
        .start(&fixture.playlists[0].tracks, 3, false);
    app.state = gramwave_application::AppCore::new(fixture);
    for (mut offset, expected) in [(-3, 1..4), (0, 5..41)] {
        let mut rows = Vec::new();
        loop {
            let reply = app.handle_control(Request::Queue { offset });
            assert!(serde_json::to_vec(&reply).unwrap().len() <= REPLY_LIMIT);
            let data = reply.data.unwrap();
            assert_eq!(data["current"]["row"], 4);
            rows.extend(
                data["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|row| row["row"].as_u64().unwrap()),
            );
            let Some(next) = data["next_offset"].as_i64() else {
                break;
            };
            assert!(next > offset && (offset < 0) == (next < 0));
            offset = next;
        }
        assert_eq!(rows, expected.collect::<Vec<_>>());
    }
    app.shutdown_core().unwrap();
}

#[test]
fn invalid_and_expired_commands_do_not_change_playback() {
    let mut app = app();
    for request in [
        Request::Select { row: 0 },
        Request::Select { row: 99 },
        Request::QueuePlay {
            entry_id: QueueEntryId(0),
        },
        Request::Open { chat_id: 99 },
        Request::Seek { seconds: f64::NAN },
    ] {
        assert!(!app.handle_control(request).ok);
    }
    let (tx, rx) = mpsc::sync_channel(8);
    app.control = Some(rx);
    let (reply, response) = mpsc::sync_channel(1);
    tx.send(Pending {
        request: Request::Select { row: 2 },
        deadline: Instant::now() - std::time::Duration::from_secs(1),
        reply,
    })
    .unwrap();
    app.process_control();
    assert!(!response.recv().unwrap().ok);
    assert!(app.state.view().session.current().is_none());
    app.shutdown_core().unwrap();
    assert!(!app.handle_control(Request::Next).ok);
}

#[test]
fn previous_restarts_after_three_seconds_then_switches() {
    use crate::application::ports::{PlaybackEvent, PlayerBackend};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    struct Fake {
        position: Rc<Cell<f64>>,
        seeks: Rc<RefCell<Vec<String>>>,
    }
    impl PlayerBackend for Fake {
        fn command(
            &self,
            command: crate::application::ports::PlayerCommand,
        ) -> Result<(), gramwave_application::AppError> {
            if let crate::application::ports::PlayerCommand::Seek(position) = command {
                self.seeks.borrow_mut().push(position.to_string());
                self.position.set(position);
            }
            Ok(())
        }
        fn snapshot(&self) -> crate::application::ports::PlayerSnapshot {
            crate::application::ports::PlayerSnapshot {
                paused: false,
                seeking: false,
                position: self.position.get(),
                audio_position: None,
                checkpoint_position: None,
                duration: 180.,
                volume: 75.,
            }
        }
        fn drain(&self) -> Vec<PlaybackEvent> {
            vec![]
        }
    }
    let mut app = app();
    let position = Rc::new(Cell::new(4.));
    let seeks = Rc::new(RefCell::new(vec![]));
    app.player = Some(Box::new(Fake {
        position: position.clone(),
        seeks: seeks.clone(),
    }));
    apply(&mut app, Request::Select { row: 5 });
    assert_eq!(
        apply(&mut app, Request::Previous)["current"]["message_id"],
        4
    );
    assert_eq!(*seeks.borrow(), ["0"]);
    assert_eq!(
        apply(&mut app, Request::Previous)["current"]["message_id"],
        3
    );
    app.shutdown_core().unwrap();
}
