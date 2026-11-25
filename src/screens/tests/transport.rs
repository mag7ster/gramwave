use super::*;

use crate::application::ports::{PlaybackEvent, PlayerBackend, PlayerCommand, PlayerSnapshot};
use crate::Track;
use gramwave_application::AppError;

struct Clock(PlayerSnapshot);

impl PlayerBackend for Clock {
    fn command(&self, _: PlayerCommand) -> Result<(), AppError> {
        Ok(())
    }

    fn drain(&self) -> Vec<PlaybackEvent> {
        Vec::new()
    }

    fn snapshot(&self) -> PlayerSnapshot {
        self.0
    }
}

#[test]
fn play_pause_remains_clickable_during_resize() {
    let mut h = queue_harness(3);
    let play = |output: egui::FullOutput| {
        output
            .shapes
            .iter()
            .find_map(|painted| match &painted.shape {
                egui::Shape::Rect(shape)
                    if shape.corner_radius == egui::CornerRadius::same(22)
                        && (shape.rect.width() - 44.).abs() < 1. =>
                {
                    assert!(
                        painted.clip_rect.expand(0.5).contains_rect(shape.rect),
                        "{:?} clipped by {:?}",
                        shape.rect,
                        painted.clip_rect
                    );
                    Some(shape.rect)
                }
                _ => None,
            })
            .expect("visible playback button")
    };
    h.size = egui::vec2(850., 149.);
    h.settle();
    let paused = h.app.state.playback().state().paused();
    h.size.y = 151.;
    h.frame(vec![]);
    let moving = play(h.frame(vec![]));
    h.click(moving.center());
    assert_ne!(h.app.state.playback().state().paused(), paused);
    h.app.shutdown_core().unwrap();
}

#[test]
fn smallest_player_keeps_basic_playback_controls() {
    let mut h = queue_harness(3);
    h.size = egui::Vec2::from(crate::launch::MIN_WINDOW_SIZE);
    h.settle();
    h.app.test_seek = None;
    h.frame(vec![]);
    assert!(h.app.test_seek.is_none());
    let first = h.app.state.view().session.current().unwrap().entry_id;
    let center = h.size.to_pos2() / 2.;
    h.click(center + egui::vec2(48., 0.));
    assert_ne!(
        h.app.state.view().session.current().unwrap().entry_id,
        first
    );
    h.click(center - egui::vec2(48., 0.));
    assert_eq!(
        h.app.state.view().session.current().unwrap().entry_id,
        first
    );
    assert!(h.ctx.read_response(egui::Id::new("track-search")).is_none());
}

#[test]
fn compact_transport_volume_supports_drag_keyboard_and_mute() {
    let volume_id = egui::Id::new("test-volume");
    for (width, height) in [(760., 190.), (820., 320.), (1200., 190.)] {
        let mut h = Harness::new();
        h.size = egui::vec2(width, height);
        h.settle();
        let (slider, id): (egui::Rect, egui::Id) =
            h.ctx.data(|data| data.get_temp(volume_id)).unwrap();
        assert!(slider.right() <= width / 2. + 190.);
        assert!(slider.bottom() <= height);
        h.pointer(slider.center(), true);
        let lower = slider.center() - egui::vec2(22., 0.);
        h.frame(vec![egui::Event::PointerMoved(lower)]);
        h.pointer(lower, false);
        let volume = h.app.state.view().volume;
        assert!((25. ..30.).contains(&volume));
        assert!(h.ctx.memory(|memory| memory.has_focus(id)));
        h.key(egui::Key::ArrowRight);
        assert_eq!(h.app.state.view().volume, volume + 1.);
        let mute = egui::pos2(slider.left() - 26., slider.center().y);
        h.click(mute);
        assert_eq!(h.app.state.view().volume, 0.);
        h.click(mute);
        assert_eq!(h.app.state.view().volume, volume + 1.);
    }

    let mut h = Harness::new();
    for size in [
        egui::vec2(320., 190.),
        crate::launch::MIN_WINDOW_SIZE.into(),
    ] {
        h.size = size;
        h.settle();
        h.ctx
            .data_mut(|data| data.remove::<(egui::Rect, egui::Id)>(volume_id));
        h.frame(vec![]);
        assert!(h
            .ctx
            .data(|data| data.get_temp::<(egui::Rect, egui::Id)>(volume_id))
            .is_none());
    }
}

#[test]
fn focused_volume_slider_remains_keyboard_operable_after_resize() {
    let mut h = Harness::new();
    h.size.x = 815.;
    h.settle();
    let (slider, id): (egui::Rect, egui::Id) = h
        .ctx
        .data(|data| data.get_temp(egui::Id::new("test-volume")))
        .unwrap();
    h.click(slider.center());
    assert!(h.ctx.memory(|memory| memory.has_focus(id)));

    h.size.x = 813.;
    h.settle();
    let before = h.app.state.view().volume;
    h.key(egui::Key::ArrowRight);
    assert_eq!(h.app.state.view().volume, before + 1.);
    h.app.shutdown_core().unwrap();
}

#[test]
fn restored_track_fetches_artwork_without_starting_playback() {
    use crate::adapters::telegram::Telegram;
    use gramwave_application::{
        library::LibraryCommand,
        model::{Playlist, State},
    };
    use serde_json::json;

    let mut h = Harness::new();
    let track = Track {
        chat: 42,
        message: 100,
        duration: 180,
        ..Default::default()
    };
    let id = track.id();
    let mut state = State {
        playlists: vec![Playlist {
            id: 0,
            done: [true; 3],
            ..Default::default()
        }],
        ..Default::default()
    };
    state.session.start(&[track.metadata()], 0, false);
    state.session.set_position_ms(37_000);
    h.app.state = gramwave_application::AppCore::new(state);

    let cover = tempfile::NamedTempFile::new().unwrap();
    let path = cover.path().to_string_lossy().into_owned();
    let downloaded = path.clone();
    let tg = Telegram::mock_rpc(move |request| match request["@type"].as_str() {
        Some("getMessage") => {
            assert_eq!(request["chat_id"], 42);
            assert_eq!(request["message_id"], 100);
            json!({"id":100,"content":{"@type":"messageAudio","audio":{
                "title":"Restored", "duration":180, "audio":{"id":90},
                "album_cover_thumbnail":{"file":{"id":7}}
            }}})
        }
        Some("downloadFile") => {
            assert_eq!(request["file_id"], 7);
            json!({"@type":"file","id":7,"local":{"path":downloaded,"is_downloading_completed":true}})
        }
        _ => panic!("unexpected request {request}"),
    });
    h.app
        .library_command(LibraryCommand::Connected(tg.connection));
    h.app.tg = Some(tg);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while h.app.artwork_path(7).is_none() {
        h.frame(vec![]);
        h.app.process(&h.ctx);
        assert!(std::time::Instant::now() < deadline, "cover did not load");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(h.app.artwork_path(7), Some(path.as_str()));
    assert_eq!(h.app.state.view().session.current().unwrap().id(), id);
    assert!(h.app.state.playback().state().paused());
    assert_eq!(h.app.state.view().session.position_ms(), 37_000);
    h.app.shutdown_core().unwrap();
}

#[test]
fn restored_track_displays_elapsed_time_before_play() {
    let mut h = Harness::new();
    crate::demo::apply(&mut h.app, "resumed");
    let mut output = h.ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, h.size)),
            ..Default::default()
        },
        |ui| h.app.render(ui),
    );
    output.textures_delta.clear();
    assert!(h.app.state.playback().state().paused());
    assert!(output.shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "0:37")
    }));
    h.click(h.app.test_seek.unwrap().0.center());
    assert!(h.app.test_seeks.is_empty());
}

#[test]
fn timeline_repaint_interval_accounts_for_track_duration_and_pixel_density() {
    assert_eq!(seek_repaint_interval(180., 600.), Duration::from_millis(75));
    assert_eq!(
        seek_repaint_interval(180., 1200.),
        Duration::from_micros(37_500)
    );
    assert_eq!(
        seek_repaint_interval(5., 600.),
        Duration::from_secs_f64(1. / 60.)
    );
    assert_eq!(
        seek_repaint_interval(3600., 600.),
        Duration::from_millis(200)
    );
}

#[test]
fn timeline_only_requests_frames_while_visible_and_playing() {
    let mut h = Harness::new();
    crate::demo::apply(&mut h.app, "playing");
    h.app.player = Some(Box::new(Clock(PlayerSnapshot {
        duration: 180.,
        ..Default::default()
    })));
    h.settle();
    let delay = h.frame(vec![]).viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    let rect = h.app.test_seek.unwrap().0;
    assert_eq!(
        delay,
        seek_repaint_interval(180., design::slider_rail(rect).width())
    );

    h.app.execute(AppCommand::Pause(true));
    h.settle();
    assert_eq!(
        h.frame(vec![]).viewport_output[&egui::ViewportId::ROOT].repaint_delay,
        Duration::MAX
    );

    h.app.execute(AppCommand::Pause(false));
    h.size.y = 140.;
    h.settle();
    assert_eq!(
        h.frame(vec![]).viewport_output[&egui::ViewportId::ROOT].repaint_delay,
        Duration::MAX
    );
}

#[test]
fn timeline_uses_audio_clock_until_seek_preview_or_backend_seek() {
    let mut h = Harness::new();
    h.click(h.app.test_rows[&0].0.center());
    assert!(!h.app.state.playback().state().paused());
    h.frame(vec![]);
    let (_, seek_id) = h.app.test_seek.unwrap();

    for (audio_position, seeking, preview, expected) in [
        (Some(30.), false, None, 35.),
        (Some(30.), true, None, 17.),
        (None, false, None, 17.),
        (Some(30.), false, Some(50.), 55.),
        (Some(-1.), false, None, 5.),
    ] {
        h.app.player = Some(Box::new(Clock(PlayerSnapshot {
            position: 12.,
            audio_position,
            duration: 180.,
            seeking,
            ..Default::default()
        })));
        h.app.seek_preview = preview;
        h.ctx.memory_mut(|m| m.request_focus(seek_id));
        let prior_seeks = h.app.test_seeks.len();
        h.key(egui::Key::ArrowRight);
        assert_eq!(h.app.test_seeks.len(), prior_seeks + 1);
        assert_eq!(*h.app.test_seeks.last().unwrap(), expected);
    }
}

#[test]
fn seek_supports_click_drag_and_keyboard() {
    let mut h = Harness::new();
    h.click(h.app.test_rows[&0].0.center());
    h.app.execute(AppCommand::Pause(true));
    h.settle();

    // Click seeks once; a drag previews until release, independent of mpv position.
    let (rect, id) = h.app.test_seek.unwrap();
    h.time += 1.;
    h.click(rect.center());
    assert_eq!(h.app.test_seeks.len(), 1);
    assert!((h.app.test_seeks[0] - 90.).abs() < 1.);
    h.time += 1.;
    h.pointer(rect.center(), true);
    h.frame(vec![egui::Event::PointerMoved(egui::pos2(
        rect.right() - 8.,
        rect.center().y,
    ))]);
    assert_eq!(h.app.test_seeks.len(), 1);
    assert!(h.app.seek_preview.unwrap() > 170.);
    h.frame(vec![]);
    assert!(h.app.seek_preview.unwrap() > 170.);
    h.pointer(egui::pos2(rect.right() - 8., rect.center().y), false);
    assert_eq!(h.app.test_seeks.len(), 2);
    assert!(h.app.seek_preview.is_none());
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.app.seek_preview = Some(90.);
    h.key(egui::Key::Home);
    assert_eq!(*h.app.test_seeks.last().unwrap(), 0.);
    h.key(egui::Key::End);
    assert_eq!(*h.app.test_seeks.last().unwrap(), 180.);
    h.key(egui::Key::ArrowRight);
    assert_eq!(*h.app.test_seeks.last().unwrap(), 5.); // backend position remains zero in demo
    h.time += 1.;
    h.pointer(rect.center(), true);
    h.app.execute(AppCommand::Next);
    assert!(h.app.seek_preview.is_none());
    let before = h.app.test_seeks.len();
    h.pointer(rect.center(), false);
    assert_eq!(
        h.app.test_seeks.len(),
        before,
        "releasing an old drag must not seek the next track"
    );
    // Unknown duration disables seeking, even while holding an old slider.
    h.app.start_session(&[Track::default()], 0);
    let before = h.app.test_seeks.len();
    h.time += 1.;
    h.click(rect.center());
    assert_eq!(h.app.test_seeks.len(), before);
}
