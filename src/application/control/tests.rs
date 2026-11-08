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
fn playlist_selection_rebuilds_queue_and_previous_returns_to_preceding_row() {
    let mut app = app();
    apply(&mut app, Request::Select { row: 2 });
    let tracks = app.playlist_tracks(0);
    app.state
        .command(tg_player_application::commands::AppCommand::Insert(
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
fn list_and_select_share_filters_search_and_sort_and_current_row_toggles_pause() {
    let mut app = app();
    app.search = "Pacific".into();
    app.sort = 2;
    let list = apply(&mut app, Request::Playlist { offset: 0 });
    let expected = list["items"][1]["message_id"].clone();
    let status = apply(&mut app, Request::Select { row: 2 });
    assert_eq!(status["current"]["message_id"], expected);
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
        .command(tg_player_application::commands::AppCommand::Filters(
            0, [false; 3],
        ));
    assert_eq!(apply(&mut app, Request::Playlist { offset: 0 })["total"], 0);
    assert!(!app.handle_control(Request::Select { row: 1 }).ok);
    app.shutdown_core().unwrap();
}

#[test]
fn pagination_handles_large_unicode_and_escaped_metadata_without_leaking_state() {
    let mut app = app();
    let mut fixture = app.state.view().clone();
    for track in &mut fixture.playlists[0].tracks {
        track.title = "\n\"\u{0001}🎵Музыка".repeat(1000);
        track.artist = track.title.clone();
    }
    fixture.api_hash = "private-api-hash".into();
    app.state = tg_player_application::AppCore::new(fixture);
    let mut offset = 0;
    let mut rows = Vec::new();
    loop {
        let reply = app.handle_control(Request::Playlist { offset });
        let encoded = serde_json::to_string(&reply).unwrap();
        assert!(encoded.len() < REPLY_LIMIT);
        assert!(!encoded.contains("private-"));
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
        ) -> Result<(), tg_player_application::AppError> {
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
