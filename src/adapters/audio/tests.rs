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
        let path = std::env::temp_dir().join(format!("tg-player-eof-{}.wav", std::process::id()));
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
        player.cmd(&["loadfile", filename, "replace"]);
        while completed < 2 && std::time::Instant::now() < deadline {
            for event in player.drain() {
                match event {
                    PlaybackEvent::Finished => {
                        completed += 1;
                        if completed == 1 {
                            player.cmd(&["loadfile", filename, "replace"]);
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
        player.cmd(&["loadfile", filename, "replace"]);
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
        player.cmd(&["set", "pause", "yes"]);
        player.cmd(&["seek", "2", "absolute+exact"]);
        player.cmd(&["set", "volume", "32"]);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while (player.number("time-pos") - 2.).abs() > 0.25 && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            (player.number("time-pos") - 2.).abs() < 0.25,
            "mpv seek failed: position {}, events {:?}",
            player.number("time-pos"),
            player.drain()
        );
        // The null audio clock includes a small buffered offset while paused.
        let paused_at = player.number("time-pos");
        std::thread::sleep(Duration::from_millis(40));
        assert!(
            (player.number("time-pos") - paused_at).abs() < 0.01,
            "mpv pause failed"
        );
        assert!((player.number("volume") - 32.).abs() < 0.1);
        player.cmd(&["stop"]);
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
