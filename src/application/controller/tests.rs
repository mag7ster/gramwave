use super::*;
use crate::{App, Playlist, State};
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
