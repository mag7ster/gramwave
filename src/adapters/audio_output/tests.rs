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
    write_wav(
        path,
        rate,
        channels,
        std::iter::repeat_n(sample, frames as usize * usize::from(channels)),
    );
}

fn write_wav(
    path: &std::path::Path,
    rate: u32,
    channels: u16,
    samples: impl ExactSizeIterator<Item = i16>,
) {
    let bytes = u32::try_from(samples.len()).unwrap() * 2;
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
    for sample in samples {
        wav.extend(sample.to_le_bytes());
    }
    std::fs::write(path, wav).unwrap();
}

fn local_pcm(path: &std::path::Path, rate: u32, start_ms: u64, samples: usize) -> Vec<f32> {
    let tasks = Scheduler::new();
    let format = (1.try_into().unwrap(), rate.try_into().unwrap());
    let (mixer, output) = rodio::mixer::mixer(format.0, format.1);
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
    owner.apply(
        Message::Command(
            0,
            0,
            PlayerCommand::Load {
                path: path.display().to_string(),
                start_ms,
            },
        ),
        &mut events,
    );
    owner.apply(
        receive.recv_timeout(Duration::from_secs(2)).unwrap(),
        &mut events,
    );
    assert_eq!(events, [PlaybackEvent::Loaded]);
    let progress = &owner
        .current
        .as_ref()
        .unwrap()
        .prepared
        .as_ref()
        .unwrap()
        .progress;
    let deadline = Instant::now() + Duration::from_secs(2);
    // These fixtures all fit in the PCM ring before the mixer starts reading it.
    while !progress.eof.load(std::sync::atomic::Ordering::Acquire) {
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
    owner.update(&mut events);
    output.take(samples).collect()
}

fn tone_amplitude(samples: &[f32], rate: u32, frequency: f64) -> f64 {
    let mut sin = 0.;
    let mut cos = 0.;
    for (i, sample) in samples.iter().enumerate() {
        let (s, c) = (std::f64::consts::TAU * frequency * i as f64 / f64::from(rate)).sin_cos();
        sin += f64::from(*sample) * s;
        cos += f64::from(*sample) * c;
    }
    2. * sin.hypot(cos) / samples.len() as f64
}

#[test]
fn resampling_suppresses_aliases_and_preserves_high_frequencies() {
    let dir = tempfile::tempdir().unwrap();
    for (rate, output_rate, tone, alias) in [
        (44_100, 48_000, 15_000., 18_900.),
        (48_000, 44_100, 23_000., 21_100.),
        (96_000, 48_000, 30_000., 18_000.),
    ] {
        let path = dir.path().join("tone.wav");
        let input = (0..rate * 3 / 20).map(|i| {
            (16_384. * (std::f64::consts::TAU * tone * f64::from(i) / f64::from(rate)).sin())
                .round() as i16
        });
        write_wav(&path, rate, 1, input);
        let frames = output_rate as usize * 3 / 20;
        let output = local_pcm(&path, output_rate, 0, frames + 1);
        let steady = &output[output_rate as usize / 20..output_rate as usize / 10];
        let alias_db = 20. * (tone_amplitude(steady, output_rate, alias) / 0.5).log10();
        assert!(alias_db < -60., "{rate} Hz input: alias was {alias_db} dB");
        if tone < f64::from(output_rate) / 2. {
            let tone_db = 20. * (tone_amplitude(steady, output_rate, tone) / 0.5).log10();
            assert!(tone_db.abs() < 0.5, "passband gain was {tone_db} dB");
        }
        assert_eq!(output[frames], 0.);
    }
}

#[test]
fn matching_rates_preserve_pcm_and_stereo_to_mono_keeps_the_right_channel() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("track.wav");
    let samples = [-32_768, -8192, 0, 8192, 32_767];
    write_wav(&path, 48_000, 1, samples.into_iter());
    assert_eq!(
        local_pcm(&path, 48_000, 0, 5),
        samples.map(|sample| f32::from(sample) / 32_768.)
    );
    write_wav(&path, 48_000, 2, [0, 16_384, -8192, 0].into_iter());
    assert_eq!(local_pcm(&path, 48_000, 0, 3), [0.25, -0.125, 0.]);
}

#[test]
fn resampling_after_seek_outputs_the_remaining_short_track() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("track.wav");
    let input = (0..2400).map(|i| if i < 2160 { -8192 } else { 16_384 });
    write_wav(&path, 24_000, 1, input);
    let output = local_pcm(&path, 48_000, 90, 481);
    assert!(output[..479].iter().all(|&sample| sample > 0.2));
    assert!(output[479].abs() > 0.001);
    assert_eq!(output[480], 0.);
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
        let mut successor = output.by_ref().take(64);
        if next_rate == 48_000 {
            assert!(successor.all(|value| value == 0.5));
        } else {
            assert!(successor.all(|value| value > 0.3));
        }
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
