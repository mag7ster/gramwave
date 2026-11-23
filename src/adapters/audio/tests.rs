#[cfg(feature = "native-tests")]
use super::*;
#[cfg(all(test, feature = "native-tests"))]
mod playback_tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn idle_native_worker_wakes_to_shutdown() {
        let mut player = Player::with_audio_output(Some("null"), Default::default()).unwrap();
        player.shutdown(Duration::from_secs(1)).unwrap();
        assert!(player.worker.is_none());
    }

    fn write_wav(path: &std::path::Path, samples: u32) {
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
        std::fs::write(path, wav).unwrap();
    }

    #[test]
    fn native_load_resumes_at_checkpoint_and_normal_load_starts_at_zero() {
        use crate::application::ports::PlayerCommand;
        use std::time::Instant;
        let player = Player::with_audio_output(Some("null"), Default::default()).unwrap();
        let temp = tempfile::tempdir().unwrap();
        for (name, start_ms) in [("resumed.wav", 3_250), ("next.wav", 0)] {
            let path = temp.path().join(name);
            write_wav(&path, 80_000);
            player.command(PlayerCommand::Stop).unwrap();
            player
                .command(PlayerCommand::Load {
                    path: path.to_str().unwrap().into(),
                    start_ms,
                })
                .unwrap();
            let expected = start_ms as f64 / 1000.;
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut loaded = false;
            loop {
                for event in player.drain() {
                    loaded |= event == PlaybackEvent::Loaded;
                    assert!(!matches!(event, PlaybackEvent::Failed(_)), "{event:?}");
                }
                let position = player.snapshot().checkpoint_position;
                if loaded && position.is_some_and(|value| (value - expected).abs() < 0.25) {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "{name}: expected {expected}, got {position:?}"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    #[test]
    fn native_seek_works_after_a_cached_file_is_unlinked() {
        use std::time::Instant;
        let player = Player::with_audio_output(Some("null"), Default::default()).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cache.wav");
        write_wav(&path, 960_000);
        player
            .try_cmd(&["loadfile", path.to_str().unwrap(), "replace"])
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !player.drain().contains(&PlaybackEvent::Loaded) {
            assert!(Instant::now() < deadline, "mpv did not open the file");
            std::thread::sleep(Duration::from_millis(5));
        }
        std::fs::remove_file(path).unwrap();
        player.try_cmd(&["seek", "100", "absolute+exact"]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            assert!(player
                .drain()
                .iter()
                .all(|event| !matches!(event, PlaybackEvent::Failed(_))));
            if player
                .snapshot()
                .checkpoint_position
                .is_some_and(|position| (100.05..101.).contains(&position))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "mpv did not play from the new position"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn native_eof_advances_two_tracks_and_manual_stop_is_distinct() {
        let player = Player::with_audio_output(Some("null"), Default::default())
            .expect("libmpv with null audio output");
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sample.wav");
        write_wav(&path, 800); // 0.1 second at 8 kHz, mono PCM16.
        let filename = path.to_str().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut completed = 0;
        assert!(player.is_idle());
        player.try_cmd(&["loadfile", filename, "replace"]).unwrap();
        assert!(!player.is_idle());
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
        write_wav(&path, 8000 * 4);
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
        assert!(!player.is_idle());
        assert!(player.number("seeking") < 0.5, "mpv seek did not settle");
        // The null audio clock includes a small buffered offset while paused.
        let paused_at = player.number("time-pos");
        let audio_paused_at = player.snapshot().audio_position.expect("mpv audio clock");
        assert!((audio_paused_at - 2.).abs() < 0.25);
        std::thread::sleep(Duration::from_millis(40));
        assert!(
            (player.number("time-pos") - paused_at).abs() < 0.01,
            "mpv pause failed: before={paused_at}, after={}, paused={}, events={:?}",
            player.number("time-pos"),
            player.number("pause"),
            player.drain()
        );
        assert!(player
            .snapshot()
            .audio_position
            .is_some_and(|position| (position - audio_paused_at).abs() < 0.01));
        assert!((player.number("volume") - 32.).abs() < 0.1);
        player.try_cmd(&["stop"]).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut stopped = false;
        while (!stopped || !player.is_idle()) && std::time::Instant::now() < deadline {
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
        assert!(player.is_idle());
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
        active_load: Default::default(),
        events,
        snapshot: Arc::new(Mutex::new(Default::default())),
        worker: None,
    };
    let backend: &dyn PlayerBackend = &player;
    assert_eq!(
        backend.command(PlayerCommand::Load {
            path: "invalid\0path".into(),
            start_ms: 0
        }),
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
