use super::*;
use crate::{App, Playlist, State};

#[test]
fn playlist_search_resolves_the_selected_duplicate_occurrence() {
    let mut app = App::new_at(PathBuf::new(), true);
    let tracks = [
        (1, "Intro"),
        (2, "Match"),
        (3, "Between"),
        (2, "Match"),
        (4, "Outro"),
        (5, "Match document"),
    ]
    .into_iter()
    .map(|(message, title)| tg_player_application::model::Track {
        chat: 42,
        message,
        title: title.into(),
        kind: if message == 5 {
            tg_player_domain::MediaKind::Document
        } else {
            tg_player_domain::MediaKind::Audio
        },
        ..Default::default()
    })
    .collect();
    app.state = tg_player_application::AppCore::new(State {
        playlists: vec![Playlist {
            id: 42,
            tracks,
            filters: [true, false, false],
            ..Default::default()
        }],
        ..Default::default()
    });
    app.opened_chat = Some(42);
    app.search = "match".into();
    for (sort, numbers) in [(1, [3, 4]), (0, [2, 4])] {
        app.sort = sort;
        let visible: Vec<_> = app
            .playlist_tracks(0)
            .iter()
            .map(|track| track.playlist_number)
            .collect();
        assert_eq!(visible, numbers);
    }
    app.select_playlist_row(1);
    let session = &app.state.view().session;
    assert_eq!(session.current().unwrap().message, 2);
    assert_eq!(session.preceding()[2].message, 3);
    assert_eq!(session.remaining()[0].message, 4);
}

#[test]
fn tray_icon_uses_the_embedded_colours_and_transparency() {
    let icon = tray_icon(32);
    assert_eq!(icon.len(), 32 * 32 * 4);
    assert_eq!(icon[3], 0);
    let (pixels, _) = icon.as_chunks::<4>();
    assert!(pixels.contains(&[17, 20, 29, 255]));
    assert!(pixels.contains(&[147, 236, 215, 255]));
}
#[test]
fn headless_idle_has_no_tick_and_drains_more_than_one_batch() {
    use fastframe_shell::{Headless, Resident};
    let mut app = App::new_at(PathBuf::new(), true);
    let ctx = egui::Context::default();
    assert_eq!(app.headless_frame(&ctx), Headless::WaitFor(Duration::MAX));
    for _ in 0..129 {
        app.tx.send(Event::Link("queued".into())).unwrap();
    }
    app.headless_frame(&ctx);
    assert_eq!(app.process(&ctx), 1);
    assert_eq!(app.last_link, "queued");
}
#[test]
fn ui_commands_use_injected_player_without_system_services() {
    use crate::application::{
        commands::AppCommand,
        ports::{PlaybackEvent, PlayerBackend},
    };
    use std::{cell::RefCell, rc::Rc};
    struct Fake(Rc<RefCell<Vec<crate::application::ports::PlayerCommand>>>);
    impl PlayerBackend for Fake {
        fn command(
            &self,
            command: crate::application::ports::PlayerCommand,
        ) -> Result<(), tg_player_application::AppError> {
            self.0.borrow_mut().push(command);
            Ok(())
        }
        fn drain(&self) -> Vec<PlaybackEvent> {
            vec![]
        }
        fn snapshot(&self) -> crate::application::ports::PlayerSnapshot {
            Default::default()
        }
    }
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut app = App::new_at(PathBuf::new(), true);
    app.player = Some(Box::new(Fake(calls.clone())));
    app.execute(AppCommand::Volume(42.));
    assert_eq!(app.state.view().volume, 42.);
    assert_eq!(
        calls.borrow()[0],
        crate::application::ports::PlayerCommand::Volume(42.)
    );
    app.shutdown_core().unwrap();
    app.execute(AppCommand::Volume(10.));
    assert_eq!(app.state.view().volume, 42.);
}

#[test]
fn shutdown_applies_completed_publications_before_its_final_flush() {
    use crate::application::ports::{PlaybackEvent, PlayerBackend, PlayerCommand, PlayerSnapshot};
    struct Finishing {
        tx: mpsc::Sender<Event>,
        id: crate::domain::TrackId,
        request: tg_player_application::tasks::RequestId,
    }
    impl PlayerBackend for Finishing {
        fn command(&self, _: PlayerCommand) -> Result<(), tg_player_application::AppError> {
            Ok(())
        }
        fn drain(&self) -> Vec<PlaybackEvent> {
            vec![]
        }
        fn snapshot(&self) -> PlayerSnapshot {
            Default::default()
        }
        fn shutdown(&mut self, _: Duration) -> Result<(), tg_player_application::AppError> {
            self.tx
                .send(Event::Application(
                    tg_player_application::workflow::AppEvent::OfflineDownloaded {
                        request: self.request,
                        id: self.id,
                        result: Ok("ready-file".into()),
                    },
                ))
                .unwrap();
            Ok(())
        }
    }
    let mut app = App::new_at(PathBuf::new(), true);
    let track = app.state.view().playlists[0].tracks[0].clone();
    let id = track.id();
    app.pin(track);
    let request = app
        .state
        .pump(std::time::Instant::now(), true)
        .into_iter()
        .find_map(|effect| match effect {
            tg_player_application::workflow::Effect::DownloadOffline { operation, .. } => {
                Some(operation.key())
            }
            _ => None,
        })
        .unwrap();
    app.player = Some(Box::new(Finishing {
        tx: app.tx.clone(),
        id,
        request,
    }));
    app.shutdown_core().unwrap();
    assert_eq!(app.state.offline().path(id), Some("ready-file"));
    assert!(app.state.offline().pending.is_empty());
}

#[test]
fn shutdown_roundtrips_queue_and_position_even_without_replaying_after_restart() {
    use crate::adapters::storage::Store;
    use crate::application::{
        control::Request,
        ports::{PlaybackEvent, PlayerBackend, PlayerCommand, PlayerSnapshot},
    };
    struct Sample;
    impl PlayerBackend for Sample {
        fn command(&self, _: PlayerCommand) -> Result<(), tg_player_application::AppError> {
            Ok(())
        }
        fn drain(&self) -> Vec<PlaybackEvent> {
            Vec::new()
        }
        fn snapshot(&self) -> PlayerSnapshot {
            PlayerSnapshot {
                checkpoint_position: Some(37.625),
                duration: 180.,
                ..Default::default()
            }
        }
    }

    for paused in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("library.sqlite");
        let (db, _) = Store::open(&path).unwrap();
        let mut app = App::new_at(PathBuf::new(), true);
        app.db = Some(Box::new(db));
        app.player = Some(Box::new(Sample));
        let track = app.state.view().playlists[0].tracks[0].clone();
        app.start_session(&[track.clone(), track.clone(), track], 1);
        app.set_paused(paused);
        let entry = app.state.view().session.current().unwrap().entry_id;
        let next = app.state.view().session.remaining()[0].entry_id;
        app.shutdown_core().unwrap();

        let (db, state) = Store::open(&path).unwrap();
        let mut app = App::new_at(PathBuf::new(), true);
        app.db = Some(Box::new(db));
        app.state = AppCore::new(state);
        assert_eq!(app.state.view().session.current().unwrap().entry_id, entry);
        assert_eq!(app.state.view().session.remaining()[0].entry_id, next);
        let status = app.handle_control(Request::Status).data.unwrap();
        assert_eq!(status["position"], 37.625);
        assert_eq!(status["paused"], true);
        app.shutdown_core().unwrap();
        let (_, state) = Store::open(&path).unwrap();
        assert_eq!(state.session.position_ms(), 37_625);
    }
}

#[test]
fn remembers_the_opened_playlist_independently_of_the_playing_queue() {
    use crate::{adapters::storage::Store, application::control::Request, bootstrap::Services};
    use tg_player_application::persistence::ChangeSet;

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("library.sqlite");
    let (db, _) = Store::open(&path).unwrap();
    let mut state = crate::demo::state();
    state.session.start(&state.playlists[0].tracks, 0, false);
    db.apply(ChangeSet::between(None, &state)).unwrap();
    let with_storage = |db, state| {
        let mut services = Services::open(PathBuf::new(), true, false, Default::default()).unwrap();
        services.core = AppCore::new(state);
        services.repository = Some(Box::new(db));
        App::from_services(services, Default::default())
    };

    let mut app = with_storage(db, state);
    assert_eq!(app.opened_chat, Some(0)); // Databases without an explicit selection use the queue source.
    app.open_chat(2);
    assert_eq!(app.opened_chat, Some(2));
    assert_eq!(app.state.view().last_opened_chat, Some(2));
    assert!(app.handle_control(Request::Open { chat_id: 4 }).ok);
    app.shutdown_core().unwrap();

    let (db, state) = Store::open(&path).unwrap();
    let mut app = with_storage(db, state);
    assert_eq!(app.state.view().session.source, Some(0));
    assert_eq!(app.opened_chat, Some(4));
    assert!(app.handle_control(Request::Playlist { offset: 0 }).ok);
    app.shutdown_core().unwrap();
}
#[test]
fn live_edits_keep_library_occurrences_and_the_started_queue_snapshot() {
    let mut app = App::new_at(PathBuf::new(), true);
    let old = Track {
        chat: 42,
        message: 7,
        title: "Original".into(),
        ..Default::default()
    };
    app.state = AppCore::new(State {
        playlists: vec![Playlist {
            id: 42,
            filters: [true, false, false],
            tracks: vec![old.clone().into(); 3],
            ..Default::default()
        }],
        ..Default::default()
    });
    let tracks = app.playlist_tracks(0);
    let ids: Vec<_> = tracks.iter().map(|t| t.library_id).collect();
    app.start_session(&tracks, 1);
    let mut edited = old;
    edited.title = "Edited".into();
    edited.file = 99;
    let (tx, rx) = mpsc::channel();
    app.updates = Some(rx);
    tx.send(TelegramUpdate::Message {
        id: edited.id(),
        track: Some(edited.transport()),
    })
    .unwrap();
    app.process(&egui::Context::default());
    assert_eq!(
        app.state.view().playlists[0]
            .tracks
            .iter()
            .map(|t| t.library_id)
            .collect::<Vec<_>>(),
        ids
    );
    assert!(app.state.view().playlists[0]
        .tracks
        .iter()
        .all(|t| t.title == "Edited"));
    assert!(app
        .state
        .view()
        .session
        .snapshot()
        .iter()
        .all(|t| t.title == "Original"));
    app.shutdown_core().unwrap();
}
