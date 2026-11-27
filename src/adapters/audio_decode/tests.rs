use super::*;
use crate::adapters::{audio_output::Player, streaming, telegram::Telegram};
use gramwave_application::ports::{PlaybackEvent, PlayerBackend, PlayerCommand};
use gramwave_application::tasks::Scheduler;
use rodio::mixer::MixerSource;
use serde_json::json;
use std::{
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

#[test]
fn mp3_seeks_through_loopback_before_the_range_arrives() {
    let fixture = Partial::new(include_bytes!("fixtures/seek-cbr.mp3"));
    let (mixer, mut output) =
        rodio::mixer::mixer(1.try_into().unwrap(), 44_100.try_into().unwrap());
    let mut player = Player::with_test_mixer(
        fixture.scheduler.handle(),
        mixer,
        (1.try_into().unwrap(), 44_100.try_into().unwrap()),
    )
    .unwrap();
    player
        .command(PlayerCommand::Load {
            path: fixture.url.clone(),
            start_ms: 0,
        })
        .unwrap();
    assert_eq!(
        fixture
            .requests
            .recv_timeout(Duration::from_secs(4))
            .unwrap(),
        0
    );
    let mut events = Vec::new();
    let initial = capture(&player, &mut output, &mut events).unwrap();
    assert!(signal_power(&initial, 44100, 440.) > 5.);
    assert_eq!(events, vec![PlaybackEvent::Loaded]);

    player.command(PlayerCommand::Seek(6.)).unwrap();
    let offset = fixture.requested_seek();
    assert!(capture_for(
        &player,
        &mut output,
        &mut events,
        Duration::from_millis(150)
    )
    .is_none());
    assert!(player.snapshot().seeking);
    assert!(player.snapshot().checkpoint_position.is_none());
    assert_eq!(events, vec![PlaybackEvent::Loaded]);

    *fixture.ready_range.lock().unwrap() = Some(offset);
    let resumed = capture(&player, &mut output, &mut events).unwrap();
    assert!(signal_power(&resumed, 44100, 880.) > 5.);
    assert!(player
        .snapshot()
        .checkpoint_position
        .is_some_and(|pos| (pos - 6.).abs() < 0.3));

    player.command(PlayerCommand::Seek(3.)).unwrap();
    fixture.requested_seek();
    player.command(PlayerCommand::Stop).unwrap();
    player
        .command(PlayerCommand::Load {
            path: fixture.file.display().to_string(),
            start_ms: 0,
        })
        .unwrap();
    events.clear();
    assert!(capture(&player, &mut output, &mut events).is_some());
    assert_eq!(events, vec![PlaybackEvent::Loaded]);
    player.shutdown(Duration::from_secs(1)).unwrap();
}

#[test]
fn variable_bitrate_mp3_seeks_to_the_requested_audio() {
    seek_partial(include_bytes!("fixtures/seek-vbr.mp3"), 3.5, 440.);
}

#[test]
fn streamed_vbr_checkpoint_waits_for_data_and_can_be_replaced() {
    let fixture = Partial::new(include_bytes!("fixtures/seek-vbr.mp3"));
    let format = (1.try_into().unwrap(), 44_100.try_into().unwrap());
    let (mixer, mut output) = rodio::mixer::mixer(format.0, format.1);
    let mut player = Player::with_test_mixer(fixture.scheduler.handle(), mixer, format).unwrap();
    let load = |start_ms| PlayerCommand::Load {
        path: fixture.url.clone(),
        start_ms,
    };
    let mut events = Vec::new();
    player.command(load(3500)).unwrap();
    assert_eq!(
        fixture
            .requests
            .recv_timeout(Duration::from_secs(4))
            .unwrap(),
        0
    );
    assert!(capture_for(
        &player,
        &mut output,
        &mut events,
        Duration::from_millis(150)
    )
    .is_none());
    assert!(player.snapshot().seeking && player.snapshot().checkpoint_position.is_none());

    player.command(PlayerCommand::Stop).unwrap();
    player.command(load(0)).unwrap();
    assert!(capture(&player, &mut output, &mut events).is_some());
    assert_eq!(events, vec![PlaybackEvent::Loaded]);
    events.clear();

    player.command(load(3500)).unwrap();
    assert!(capture_for(
        &player,
        &mut output,
        &mut events,
        Duration::from_millis(150)
    )
    .is_none());
    assert!(events.is_empty());
    *fixture.ready_range.lock().unwrap() = Some(PREFIX);
    let pcm = capture(&player, &mut output, &mut events).unwrap();
    assert!(signal_power(&pcm, 44100, 440.) > 5.);
    assert_eq!(events, vec![PlaybackEvent::Loaded]);
    assert!(player
        .snapshot()
        .checkpoint_position
        .is_some_and(|p| (p - 3.5).abs() < 0.3));
    player.shutdown(Duration::from_secs(1)).unwrap();
}

const PREFIX: u64 = 8192;

struct Partial {
    _dir: tempfile::TempDir,
    file: std::path::PathBuf,
    scheduler: Scheduler,
    server: streaming::Server,
    url: String,
    requests: mpsc::Receiver<u64>,
    ready_range: Arc<Mutex<Option<u64>>>,
}

impl Partial {
    fn new(raw: &[u8]) -> Self {
        Self::with_prefix(raw, PREFIX)
    }

    fn with_prefix(raw: &[u8], prefix: u64) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.mp3");
        std::fs::write(&path, raw).unwrap();
        let size = raw.len() as u64;
        let ready_range = Arc::new(Mutex::new(None::<u64>));
        let published = ready_range.clone();
        let (requests, observed) = mpsc::channel();
        let file = path.clone();
        let telegram = Telegram::mock_rpc(move |request| {
            let offset = request["offset"].as_u64().unwrap_or_default();
            match request["@type"].as_str() {
                Some("getFileDownloadedPrefixSize") => {
                    let range = published.lock().unwrap();
                    json!({"size": if range.is_some_and(|start| offset >= start) {
                    size.saturating_sub(offset)
                } else { prefix.saturating_sub(offset) }})
                }
                Some("cancelDownloadFile") => json!({}),
                kind => {
                    if kind == Some("downloadFile") {
                        requests.send(offset).ok();
                    }
                    json!({"@type":"file","id":1,"size":size,"local":{
                        "path":file,"downloaded_prefix_size":prefix
                    }})
                }
            }
        });
        let scheduler = Scheduler::new();
        let runtime = scheduler.handle();
        let (base, server) = streaming::serve(telegram, runtime.clone()).unwrap();
        Self {
            _dir: dir,
            file: path,
            scheduler,
            server,
            url: format!("{base}/1"),
            requests: observed,
            ready_range,
        }
    }

    fn requested_seek(&self) -> u64 {
        (0..6)
            .find_map(|_| {
                self.requests
                    .recv_timeout(Duration::from_secs(4))
                    .ok()
                    .filter(|&offset| offset > PREFIX)
            })
            .expect("a range beyond the available prefix")
    }
}

impl Drop for Partial {
    fn drop(&mut self) {
        self.server.shutdown(Duration::from_secs(4)).unwrap();
    }
}

fn capture(
    player: &Player,
    output: &mut MixerSource,
    events: &mut Vec<PlaybackEvent>,
) -> Option<Vec<f32>> {
    capture_for(player, output, events, Duration::from_secs(4))
}

fn capture_for(
    player: &Player,
    output: &mut MixerSource,
    events: &mut Vec<PlaybackEvent>,
    timeout: Duration,
) -> Option<Vec<f32>> {
    let deadline = Instant::now() + timeout;
    let mut result = Vec::new();
    while Instant::now() < deadline {
        events.extend(player.drain());
        let chunk = output.by_ref().take(256).collect::<Vec<_>>();
        if result.is_empty() && chunk.iter().all(|&v| v == 0.) {
            thread::sleep(Duration::from_millis(4));
            continue;
        }
        result.extend(chunk);
        if result.len() >= 4410 {
            return Some(result);
        }
        thread::sleep(Duration::from_millis(4));
    }
    None
}

fn seek_partial(raw: &[u8], target: f64, frequency: f64) {
    let fixture = Partial::new(raw);
    let runtime = fixture.scheduler.handle();
    let url = fixture.url.clone();
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let (commands, receive) = mpsc::channel();
    let (decoded, results) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = || -> Result<()> {
            let mut decoder = Decode::open(&url, None, runtime, worker_cancel, None)?;
            let mut samples = Vec::new();
            let first = decoder.next(0., &mut samples)?.unwrap();
            decoded
                .send((first.position, first.rate, samples.clone()))
                .unwrap();
            if receive.recv().is_ok() {
                decoder.seek(target)?;
                let mut first = decoder.next(target, &mut samples)?.unwrap();
                let mut signal = samples.clone();
                while signal.len() < 4410 {
                    let next = decoder.next(target, &mut samples)?.unwrap();
                    first.rate = next.rate;
                    signal.extend_from_slice(&samples);
                }
                decoded.send((first.position, first.rate, signal)).unwrap();
            }
            Ok(())
        };
        result()
    });

    let test = std::panic::catch_unwind(|| {
        assert_eq!(
            fixture
                .requests
                .recv_timeout(Duration::from_secs(4))
                .unwrap(),
            0
        );
        let (first, _, _) = results.recv_timeout(Duration::from_secs(4)).unwrap();
        assert!(first < 0.1);
        commands.send(()).unwrap();
        assert!(matches!(
            results.recv_timeout(Duration::from_millis(150)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        *fixture.ready_range.lock().unwrap() = Some(PREFIX);
        let (resumed, rate, samples) = results.recv_timeout(Duration::from_secs(4)).unwrap();
        assert!(
            (resumed - target).abs() < 0.25,
            "asked for {target}s, got {resumed}s"
        );
        let actual = signal_power(&samples, rate, frequency);
        let other = signal_power(&samples, rate, if frequency == 440. { 880. } else { 440. });
        assert!(actual > 4. * other && actual > 5., "seek {target}s reported {resumed}s, but spectral amplitudes were {actual:.1} at {frequency}Hz and {other:.1} at the other tone");
        assert!(fixture.requests.try_iter().all(|offset| offset <= PREFIX));
    });
    cancel.cancel();
    drop(commands);
    let outcome = worker.join().unwrap();
    if let Err(failure) = test {
        eprintln!("decoder outcome: {outcome:?}");
        std::panic::resume_unwind(failure);
    }
    outcome.unwrap();
}

fn signal_power(samples: &[f32], rate: u32, hz: f64) -> f64 {
    let (real, imaginary) =
        samples
            .iter()
            .enumerate()
            .fold((0., 0.), |(real, imaginary), (i, sample)| {
                let phase = std::f64::consts::TAU * hz * i as f64 / f64::from(rate);
                (
                    real + f64::from(*sample) * phase.cos(),
                    imaginary + f64::from(*sample) * phase.sin(),
                )
            });
    real.hypot(imaginary)
}

#[test]
fn supported_formats_decode_from_files_and_loopback() {
    let dir = tempfile::tempdir().unwrap();
    let tasks = Scheduler::new();
    let runtime = tasks.handle();
    for (name, raw) in [
        ("seek.mp3", &include_bytes!("fixtures/seek-cbr.mp3")[..]),
        ("tone.flac", &include_bytes!("fixtures/tone.flac")[..]),
        ("tone.aac", &include_bytes!("fixtures/tone.aac")[..]),
        ("tone.m4a", &include_bytes!("fixtures/tone.m4a")[..]),
        ("voice.ogg", &include_bytes!("fixtures/voice.ogg")[..]),
        ("tone.wav", &include_bytes!("fixtures/tone.wav")[..]),
        ("tone.aiff", &include_bytes!("fixtures/tone.aiff")[..]),
        ("lossless.m4a", &include_bytes!("fixtures/lossless.m4a")[..]),
        ("vorbis.ogg", &include_bytes!("fixtures/vorbis.ogg")[..]),
    ] {
        let file = dir.path().join(name);
        std::fs::write(&file, raw).unwrap();
        let server_file = file.clone();
        let (base, mut server) =
            streaming::serve(Telegram::mock_file(server_file), runtime.clone()).unwrap();
        let url = format!("{base}/1");
        for path in [file.to_str().unwrap(), &url] {
            let mut decoder = Decode::open(path, None, runtime.clone(), Default::default(), None)
                .unwrap_or_else(|error| panic!("{name} at {path}: {error}"));
            let mut samples = Vec::new();
            let mut frames = 0;
            let mut peak = 0f32;
            while let Some(pcm) = decoder
                .next(0., &mut samples)
                .unwrap_or_else(|error| panic!("{name} at {path}: {error}"))
            {
                frames += samples.len() / usize::from(pcm.channels);
                peak = samples
                    .iter()
                    .fold(peak, |peak, sample| peak.max(sample.abs()));
            }
            if name == "voice.ogg" {
                assert_eq!(frames, 16_800, "Opus delay and padding at {path}");
            } else if name == "seek.mp3" {
                assert_eq!(frames, 352_800, "MP3 delay and padding at {path}");
            }
            assert!(
                peak > 0.05 && frames > 2000,
                "{name} at {path}: {frames} frames, peak {peak}"
            );
        }
        server.shutdown(Duration::from_secs(3)).unwrap();
    }
}

#[test]
fn streamed_next_prepares_before_eof_and_survives_late_data() {
    use gramwave_application::ports::QueuedPlayback;
    use gramwave_domain::QueueEntryId;

    for (prefix, manual) in [(0, false), (PREFIX, false), (0, true)] {
        let fixture = Partial::with_prefix(include_bytes!("fixtures/seek-cbr.mp3"), prefix);
        let voice = fixture.file.with_file_name("current.ogg");
        std::fs::write(&voice, include_bytes!("fixtures/voice.ogg")).unwrap();
        let format = (1.try_into().unwrap(), 44_100.try_into().unwrap());
        let (mixer, mut output) = rodio::mixer::mixer(format.0, format.1);
        let mut player =
            Player::with_test_mixer(fixture.scheduler.handle(), mixer, format).unwrap();
        player.command(PlayerCommand::Pause(true)).unwrap();
        player
            .command(PlayerCommand::Load {
                path: voice.display().to_string(),
                start_ms: 0,
            })
            .unwrap();
        let next = QueuedPlayback {
            entry_id: QueueEntryId(2),
            revision: 7,
            path: fixture.url.clone(),
        };
        player
            .command(PlayerCommand::QueueNext(Some(next.clone())))
            .unwrap();
        assert_eq!(
            fixture
                .requests
                .recv_timeout(Duration::from_secs(4))
                .unwrap(),
            0
        );
        let expected = if manual {
            let request = gramwave_application::tasks::Operation::new(1).key();
            player
                .command(PlayerCommand::PromoteNext {
                    request,
                    entry_id: next.entry_id,
                    revision: next.revision,
                })
                .unwrap();
            PlaybackEvent::NextPromoted(request, next)
        } else {
            player.command(PlayerCommand::Pause(false)).unwrap();
            PlaybackEvent::Advanced(next)
        };
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut playback_events = Vec::new();
        while Instant::now() < deadline && !playback_events.contains(&expected) {
            output.by_ref().take(256).for_each(drop);
            playback_events.extend(player.drain());
            assert!(!playback_events
                .iter()
                .any(|event| matches!(event, PlaybackEvent::Finished | PlaybackEvent::Failed(_))));
            if prefix == 0 && fixture.requests.try_recv().is_ok() {
                // Promotion changes priority while the first bytes are missing.
                assert!(!playback_events.contains(&expected));
                *fixture.ready_range.lock().unwrap() = Some(0);
            }
            thread::sleep(Duration::from_millis(4));
        }
        if let PlaybackEvent::NextPromoted(request, _) = &expected {
            let index = playback_events
                .iter()
                .position(|event| *event == PlaybackEvent::NextBuffering(*request))
                .expect("missing initial PCM is reported before the promotion");
            assert!(index < playback_events.len() - 1);
            playback_events.remove(index);
        }
        assert_eq!(playback_events, vec![PlaybackEvent::Loaded, expected]);
        assert!(player.snapshot().position < 0.2);
        player.shutdown(Duration::from_secs(1)).unwrap();
    }
}

#[test]
fn ogg_prefetch_decodes_from_head_with_the_tail_unavailable() {
    let fixture = Partial::new(include_bytes!("fixtures/streamed-voice.ogg"));
    let runtime = fixture.scheduler.handle();
    let cancel = CancellationToken::new();
    let deadline = cancel.clone();
    let timeout = runtime.spawn(async move {
        tokio::time::sleep(Duration::from_secs(4)).await;
        deadline.cancel();
    });
    let mut decode = Decode::open(
        &fixture.url,
        None,
        runtime,
        cancel,
        Some(CancellationToken::new()),
    )
    .unwrap();
    let mut samples = Vec::new();
    assert!(decode.next(0., &mut samples).unwrap().is_some());
    let mut frames = samples.len();
    assert_eq!(
        fixture
            .requests
            .recv_timeout(Duration::from_secs(1))
            .unwrap(),
        0
    );
    assert!(fixture.requests.try_recv().is_err());
    *fixture.ready_range.lock().unwrap() = Some(0);
    while decode.next(0., &mut samples).unwrap().is_some() {
        frames += samples.len();
    }
    timeout.abort();
    assert_eq!(
        frames, 192_000,
        "Ogg streaming must trim encoder delay and padding"
    );
}

#[test]
fn m4a_prefetch_reads_the_index_from_the_tail() {
    let fixture = Partial::new(include_bytes!("fixtures/tone.m4a"));
    let runtime = fixture.scheduler.handle();
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    let timeout = runtime.spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        stop.cancel();
    });
    let stop = cancel.clone();
    let url = fixture.url.clone();
    let worker = thread::spawn(move || {
        let mut decode = Decode::open(&url, None, runtime, stop, Some(CancellationToken::new()))?;
        decode.next(0., &mut Vec::new())
    });
    let result = std::panic::catch_unwind(|| {
        assert_eq!(
            fixture
                .requests
                .recv_timeout(Duration::from_secs(4))
                .unwrap(),
            0
        );
        let index = fixture.requested_seek();
        *fixture.ready_range.lock().unwrap() = Some(index);
    });
    if let Err(panic) = result {
        cancel.cancel();
        let _ = worker.join();
        std::panic::resume_unwind(panic);
    }
    assert!(worker.join().unwrap().unwrap().is_some());
    timeout.abort();
}
