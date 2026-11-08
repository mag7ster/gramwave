#[cfg(feature = "native-tests")]
use super::*;
#[cfg(all(test, feature = "native-tests"))]
mod playback_tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn native_eof_advances_two_tracks_and_manual_stop_is_distinct() {
        let player =
            Player::with_audio_output(Some("null")).expect("libmpv with null audio output");
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sample.wav");
        let samples = 800u32; // 0.1 second at 8 kHz, mono PCM16.
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend((36 + samples * 2).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(8000u32.to_le_bytes());
        wav.extend(16000u32.to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend((samples * 2).to_le_bytes());
        wav.resize(44 + samples as usize * 2, 0);
        std::fs::write(&path, wav).unwrap();
        let filename = path.to_str().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut completed = 0;
        player.try_cmd(&["loadfile", filename, "replace"]).unwrap();
        while completed < 2 && std::time::Instant::now() < deadline {
            for event in player.drain() {
                match event {
                    PlaybackEvent::Finished => {
                        completed += 1;
                        if completed == 1 {
                            player.try_cmd(&["loadfile", filename, "replace"]).unwrap();
                        }
                    }
                    PlaybackEvent::Failed(e) => panic!("mpv: {e}"),
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(completed, 2, "EOF was lost after mpv returned to idle");
        let mut long = std::fs::read(&path).unwrap();
        let bytes = 8000u32 * 4 * 2;
        long[4..8].copy_from_slice(&(36 + bytes).to_le_bytes());
        long[40..44].copy_from_slice(&bytes.to_le_bytes());
        long.resize(44 + bytes as usize, 0);
        std::fs::write(&path, long).unwrap();
        player.try_cmd(&["loadfile", filename, "replace"]).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut loaded = false;
        while (!loaded || player.number("duration") < 2.) && std::time::Instant::now() < deadline {
            for event in player.drain() {
                loaded |= event == PlaybackEvent::Loaded;
                assert!(!matches!(
                    event,
                    PlaybackEvent::Finished | PlaybackEvent::Failed(_)
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(loaded && player.number("duration") > 2.);
        player.try_cmd(&["set", "pause", "yes"]).unwrap();
        player.try_cmd(&["seek", "2", "absolute+exact"]).unwrap();
        player.try_cmd(&["set", "volume", "32"]).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while (player.number("pause") < 0.5
            || player.number("seeking") > 0.5
            || (player.number("time-pos") - 2.).abs() > 0.25)
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            (player.number("time-pos") - 2.).abs() < 0.25,
            "mpv seek failed: position {}, events {:?}",
            player.number("time-pos"),
            player.drain()
        );
        assert!(
            player.number("pause") > 0.5,
            "mpv did not acknowledge pause"
        );
        assert!(player.number("seeking") < 0.5, "mpv seek did not settle");
        // The null audio clock includes a small buffered offset while paused.
        let paused_at = player.number("time-pos");
        std::thread::sleep(Duration::from_millis(40));
        assert!(
            (player.number("time-pos") - paused_at).abs() < 0.01,
            "mpv pause failed: before={paused_at}, after={}, paused={}, events={:?}",
            player.number("time-pos"),
            player.number("pause"),
            player.drain()
        );
        assert!((player.number("volume") - 32.).abs() < 0.1);
        player.try_cmd(&["stop"]).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut stopped = false;
        while !stopped && std::time::Instant::now() < deadline {
            for event in player.drain() {
                assert_ne!(
                    event,
                    PlaybackEvent::Finished,
                    "Manual stop must not advance the queue"
                );
                stopped |= event == PlaybackEvent::Stopped;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(stopped);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn slow_property_source_does_not_hold_snapshot_mutex() {
    use crate::application::ports::PlayerSnapshot;
    use std::{
        sync::{mpsc, Arc, Mutex},
        time::Duration,
    };
    let shared = Arc::new(Mutex::new(PlayerSnapshot {
        position: 7.,
        ..Default::default()
    }));
    let writer = shared.clone();
    let (started, ready) = mpsc::channel();
    let (resume, delayed) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        super::publish_snapshot(&writer, || {
            started.send(()).unwrap();
            delayed.recv_timeout(Duration::from_secs(2)).unwrap();
            PlayerSnapshot {
                position: 9.,
                duration: 42.,
                ..Default::default()
            }
        })
    });
    ready.recv_timeout(Duration::from_secs(1)).unwrap();
    let old = *shared.try_lock().expect("native read holds snapshot lock");
    assert_eq!(old.position, 7.);
    resume.send(()).unwrap();
    worker.join().unwrap();
    let new = *shared.lock().unwrap();
    assert_eq!((new.position, new.duration), (9., 42.));
}

#[test]
fn player_port_reports_invalid_commands_and_a_closed_worker() {
    use super::Player;
    use crate::application::ports::{PlayerBackend, PlayerCommand};
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    };
    use tg_player_application::AppError;
    let (sender, commands) = mpsc::channel();
    let (_, events) = mpsc::channel();
    let mut player = Player {
        sender: Some(sender),
        generation: AtomicU64::new(0),
        events,
        snapshot: Arc::new(Mutex::new(Default::default())),
        worker: None,
    };
    let backend: &dyn PlayerBackend = &player;
    assert_eq!(
        backend.command(PlayerCommand::Load("invalid\0path".into())),
        Err(AppError::Player("mpv command contains NUL".into()))
    );
    assert_eq!(player.generation.load(Ordering::Acquire), 0);
    assert!(commands.try_recv().is_err());
    drop(commands);
    assert!(matches!(
        backend.command(PlayerCommand::Pause(true)),
        Err(AppError::Player(_))
    ));
    PlayerBackend::shutdown(&mut player, std::time::Duration::ZERO).unwrap();
    assert_eq!(
        player.command(PlayerCommand::Stop),
        Err(AppError::Player("mpv is closed".into()))
    );
}
