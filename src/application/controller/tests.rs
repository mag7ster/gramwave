use super::*;
#[test]
fn cancelled_pin_result_cannot_remove_requeued_track() {
    let mut app = App::new_at(std::env::temp_dir().join("tg-player-regression"), true);
    assert!(app.db.is_none() && app.player.is_none() && app.media.is_none() && app.tray.is_none());
    let track = app.state.playlists[0].tracks[0].clone();
    app.pin(track.clone());
    app.cancel_offline_downloads();
    app.pin(track.clone());
    app.tx
        .send(Event::PinFailed(
            0,
            track.chat,
            track.message,
            "old error".into(),
        ))
        .unwrap();
    app.tx
        .send(Event::Pinned(
            0,
            track.chat,
            track.message,
            "old path".into(),
        ))
        .unwrap();
    app.process(&egui::Context::default());
    assert_eq!(app.state.pending_pins.len(), 1);
    assert!(app.state.playlists[0].tracks[0].pinned.is_none());
    assert!(app.error.is_empty());
    app.tx
        .send(Event::Pinned(
            1,
            track.chat,
            track.message,
            "new path".into(),
        ))
        .unwrap();
    app.process(&egui::Context::default());
    assert!(app.state.pending_pins.is_empty());
    assert_eq!(
        app.state.playlists[0].tracks[0].pinned.as_deref(),
        Some("new path")
    );
    app.shutdown_core().unwrap();
}

#[test]
fn ui_commands_use_injected_player_without_system_services() {
    use crate::application::{
        commands::AppCommand,
        ports::{PlaybackEvent, PlayerBackend},
    };
    use std::{cell::RefCell, rc::Rc};
    struct Fake(Rc<RefCell<Vec<Vec<String>>>>);
    impl PlayerBackend for Fake {
        fn cmd(&self, args: &[&str]) {
            self.0
                .borrow_mut()
                .push(args.iter().map(|s| s.to_string()).collect());
        }
        fn drain(&self) -> Vec<PlaybackEvent> {
            vec![]
        }
        fn number(&self, _: &str) -> f64 {
            0.
        }
    }
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut app = App::new_at(PathBuf::new(), true);
    app.player = Some(Box::new(Fake(calls.clone())));
    app.execute(AppCommand::Volume(42.));
    assert_eq!(app.state.volume, 42.);
    assert_eq!(calls.borrow()[0], vec!["set", "volume", "42"]);
    app.shutdown_core().unwrap();
    app.execute(AppCommand::Volume(10.));
    assert_eq!(app.state.volume, 42.);
}
