use super::*;
use gramwave_application::tasks::Scheduler;
use gramwave_domain::QueueEntryId;
use rodio::mixer::MixerSource;
use std::time::Instant;

fn player(scheduler: &Scheduler) -> (Player, MixerSource) {
    let (mixer, output) = rodio::mixer::mixer(
        NonZeroU16::new(1).unwrap(),
        NonZeroU32::new(48_000).unwrap(),
    );
    let player = Player::with_test_mixer(
        scheduler.handle(),
        mixer,
        (1.try_into().unwrap(), 48_000.try_into().unwrap()),
    )
    .unwrap();
    (player, output)
}

fn events_until(
    player: &Player,
    mixer: &mut MixerSource,
    required: &PlaybackEvent,
) -> Vec<PlaybackEvent> {
    let deadline = Instant::now() + Duration::from_secs(4);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        mixer.by_ref().take(512).for_each(drop);
        seen.extend(player.drain());
        if seen.contains(required) {
            return seen;
        }
        thread::sleep(Duration::from_millis(4));
    }
    panic!(
        "expected {required:?}, got {seen:?}, snapshot {:?}",
        player.snapshot()
    );
}

#[test]
fn opus_eof_uses_the_latest_preloaded_occurrence() {
    let dir = tempfile::tempdir().unwrap();
    let opus = dir.path().join("voice.ogg");
    let mp3 = dir.path().join("next.mp3");
    std::fs::write(&opus, include_bytes!("../audio_decode/fixtures/voice.ogg")).unwrap();
    std::fs::write(
        &mp3,
        include_bytes!("../audio_decode/fixtures/seek-cbr.mp3"),
    )
    .unwrap();
    let tasks = Scheduler::new();
    let (mut player, mut output) = player(&tasks);
    player.command(PlayerCommand::Pause(true)).unwrap();
    player
        .command(PlayerCommand::Load {
            path: opus.display().to_string(),
            start_ms: 0,
        })
        .unwrap();
    events_until(&player, &mut output, &PlaybackEvent::Loaded);
    let next = QueuedPlayback {
        path: mp3.display().to_string(),
        entry_id: QueueEntryId(3),
        revision: 5,
    };
    let older = QueuedPlayback {
        entry_id: QueueEntryId(2),
        revision: 4,
        ..next.clone()
    };
    player
        .command(PlayerCommand::QueueNext(Some(older)))
        .unwrap();
    player
        .command(PlayerCommand::QueueNext(Some(next.clone())))
        .unwrap();
    player.command(PlayerCommand::Pause(false)).unwrap();
    let events = events_until(&player, &mut output, &PlaybackEvent::Advanced(next));
    assert_eq!(events.len(), 1);
    assert!(player
        .snapshot()
        .checkpoint_position
        .is_some_and(|position| position < 0.2));
    player.shutdown(Duration::from_secs(1)).unwrap();
}

#[test]
fn stop_and_replacement_discard_an_already_emitted_eof() {
    let dir = tempfile::tempdir().unwrap();
    let opus = dir.path().join("voice.ogg");
    let mp3 = dir.path().join("next.mp3");
    std::fs::write(&opus, include_bytes!("../audio_decode/fixtures/voice.ogg")).unwrap();
    std::fs::write(
        &mp3,
        include_bytes!("../audio_decode/fixtures/seek-cbr.mp3"),
    )
    .unwrap();
    let tasks = Scheduler::new();
    let (player, mut output) = player(&tasks);
    player
        .command(PlayerCommand::Load {
            path: opus.display().to_string(),
            start_ms: 0,
        })
        .unwrap();
    events_until(&player, &mut output, &PlaybackEvent::Loaded);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !player.is_idle() {
        assert!(Instant::now() < deadline);
        output.by_ref().take(512).for_each(drop);
        thread::sleep(Duration::from_millis(4));
    }
    player.command(PlayerCommand::Stop).unwrap();
    player
        .command(PlayerCommand::Load {
            path: mp3.display().to_string(),
            start_ms: 2200,
        })
        .unwrap();
    let events = events_until(&player, &mut output, &PlaybackEvent::Loaded);
    assert_eq!(events, vec![PlaybackEvent::Loaded]);
    let deadline = Instant::now() + Duration::from_secs(3);
    while player.snapshot().checkpoint_position.is_none() {
        assert!(Instant::now() < deadline);
        output.by_ref().take(512).for_each(drop);
        thread::sleep(Duration::from_millis(4));
    }
    assert!(player
        .snapshot()
        .checkpoint_position
        .is_some_and(|position| (position - 2.2).abs() < 0.2));
    std::fs::remove_file(mp3).unwrap();
    player.command(PlayerCommand::Seek(5.)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(Instant::now() < deadline);
        assert!(player.drain().is_empty());
        if player
            .snapshot()
            .checkpoint_position
            .is_some_and(|position| (position - 5.).abs() < 0.2)
        {
            break;
        }
        output.by_ref().take(512).for_each(drop);
        thread::sleep(Duration::from_millis(4));
    }
}

#[test]
#[ignore = "requires a CPAL output device"]
fn cpal_plays_a_local_opus_track() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("voice.ogg");
    std::fs::write(&path, include_bytes!("../audio_decode/fixtures/voice.ogg")).unwrap();
    let tasks = Scheduler::new();
    let mut player = Player::new(Waker::default(), tasks.handle()).unwrap();
    player
        .command(PlayerCommand::Load {
            path: path.to_string_lossy().into_owned(),
            start_ms: 0,
        })
        .unwrap();
    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(4);
    while !events.contains(&PlaybackEvent::Finished) {
        assert!(
            Instant::now() < deadline,
            "{events:?}, {:?}",
            player.snapshot()
        );
        events.extend(player.drain());
        thread::sleep(Duration::from_millis(4));
    }
    assert_eq!(events, vec![PlaybackEvent::Loaded, PlaybackEvent::Finished]);
    player.shutdown(Duration::from_secs(1)).unwrap();
}

fn constant_wav(path: &std::path::Path, rate: u32, channels: u16, frames: u32, sample: i16) {
    let bytes = frames * u32::from(channels) * 2;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((bytes + 36).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(channels.to_le_bytes());
    wav.extend(rate.to_le_bytes());
    wav.extend((rate * u32::from(channels) * 2).to_le_bytes());
    wav.extend((channels * 2).to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(bytes.to_le_bytes());
    for _ in 0..frames * u32::from(channels) {
        wav.extend(sample.to_le_bytes());
    }
    std::fs::write(path, wav).unwrap();
}

#[test]
fn ready_tracks_join_without_waiting_for_the_owner() {
    for (next_rate, channels, manual) in [(48_000, 1, false), (24_000, 2, false), (24_000, 2, true)]
    {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.wav");
        let next = dir.path().join("next.wav");
        constant_wav(&first, 48_000, 1, 480, 8192);
        constant_wav(&next, next_rate, channels, next_rate / 100, 16384);
        let tasks = Scheduler::new();
        let format = (2.try_into().unwrap(), 48_000.try_into().unwrap());
        let (mixer, mut output) = rodio::mixer::mixer(format.0, format.1);
        let (send, receive) = mpsc::channel();
        let mut owner = Owner::new(
            mixer,
            format,
            tasks.handle(),
            send,
            Arc::default(),
            Arc::default(),
        );
        let mut events = Vec::new();
        let next = QueuedPlayback {
            entry_id: QueueEntryId(2),
            revision: 1,
            path: next.display().to_string(),
        };
        owner.apply(
            Message::Command(
                0,
                0,
                PlayerCommand::Load {
                    path: first.display().to_string(),
                    start_ms: 0,
                },
            ),
            &mut events,
        );
        owner.apply(
            Message::Command(0, 0, PlayerCommand::QueueNext(Some(next.clone()))),
            &mut events,
        );
        for _ in 0..2 {
            owner.apply(
                receive.recv_timeout(Duration::from_secs(2)).unwrap(),
                &mut events,
            );
        }
        owner.update(&mut events);
        let deadline = Instant::now() + Duration::from_secs(2);
        let progress = &owner
            .current
            .as_ref()
            .unwrap()
            .prepared
            .as_ref()
            .unwrap()
            .progress;
        while !progress.eof.load(Relaxed) {
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
        let boundary = if manual { 64 } else { 960 };
        assert!(output.by_ref().take(boundary).all(|value| value == 0.25));
        let request = gramwave_application::tasks::Operation::new(1).key();
        if manual {
            owner.apply(
                Message::Command(
                    0,
                    0,
                    PlayerCommand::PromoteNext {
                        request,
                        entry_id: next.entry_id,
                        revision: next.revision,
                    },
                ),
                &mut events,
            );
            owner.update(&mut events);
        }
        assert!(output.by_ref().take(64).all(|value| value == 0.5));
        assert_eq!(events, vec![PlaybackEvent::Loaded]);
        owner.update(&mut events);
        assert_eq!(
            events,
            vec![
                PlaybackEvent::Loaded,
                if manual {
                    PlaybackEvent::NextPromoted(request, next)
                } else {
                    PlaybackEvent::Advanced(next)
                }
            ]
        );
    }
}

#[test]
fn pending_successor_can_resolve_after_current_pcm_ends() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("track.wav");
    constant_wav(&path, 48_000, 1, 480, 8192);
    let path = path.display().to_string();
    let tasks = Scheduler::new();
    let (player, mut output) = player(&tasks);
    player
        .command(PlayerCommand::Load {
            path: path.clone(),
            start_ms: 0,
        })
        .unwrap();
    player.command(PlayerCommand::QueueNextPending).unwrap();
    events_until(&player, &mut output, &PlaybackEvent::Loaded);
    for _ in 0..8 {
        output.by_ref().take(512).for_each(drop);
        thread::sleep(Duration::from_millis(4));
    }
    assert!(player.drain().is_empty());
    assert!(!player.is_idle());
    let next = QueuedPlayback {
        path,
        entry_id: QueueEntryId(2),
        revision: 4,
    };
    player
        .command(PlayerCommand::QueueNext(Some(next.clone())))
        .unwrap();
    let advanced = PlaybackEvent::Advanced(next);
    assert_eq!(
        events_until(&player, &mut output, &advanced).first(),
        Some(&advanced)
    );
}
