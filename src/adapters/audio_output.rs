use super::audio_decode::{Decode, Pcm};
use fastframe_shell::Waker;
use gramwave_application::{
    ports::{PlaybackEvent, PlayerBackend, PlayerCommand, PlayerSnapshot, QueuedPlayback},
    tasks::RequestId,
    AppError,
};
use rodio::DeviceSinkBuilder;
use rodio::{mixer::Mixer, Source};
use std::{
    fs::File,
    num::{NonZeroU16, NonZeroU32},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tokio::{runtime::Handle, sync::Semaphore};
use tokio_util::sync::CancellationToken;

type OutputFormat = (NonZeroU16, NonZeroU32);

const POLL: Duration = Duration::from_millis(10);

pub(crate) struct Player {
    messages: mpsc::Sender<Message>,
    worker: Option<thread::JoinHandle<()>>,
    generation: Arc<AtomicU64>,
    next_epoch: Arc<AtomicU64>,
    events: mpsc::Receiver<(u64, PlaybackEvent)>,
    snapshot: Arc<Mutex<(u64, PlayerSnapshot, bool)>>,
}

enum Message {
    Command(u64, u64, PlayerCommand),
    Ready(u64, Prepared),
    Failed(u64, String),
    OutputError(String),
    Shutdown,
}

impl Player {
    pub(crate) fn new(wake: Waker, runtime: Handle) -> Result<Self, String> {
        Self::start(wake, runtime, |messages| {
            let mut output = DeviceSinkBuilder::from_default_device()
                .map_err(|e| e.to_string())?
                .with_error_callback(move |error| {
                    let _ = messages.send(Message::OutputError(error.to_string()));
                })
                .open_sink_or_fallback()
                .map_err(|e| e.to_string())?;
            output.log_on_drop(false);
            let format = (
                output.config().channel_count(),
                output.config().sample_rate(),
            );
            Ok((output.mixer().clone(), format, output))
        })
    }

    #[cfg(test)]
    pub(super) fn with_test_mixer(
        runtime: Handle,
        mixer: Mixer,
        format: OutputFormat,
    ) -> Result<Self, String> {
        Self::start(Waker::default(), runtime, move |_| Ok((mixer, format, ())))
    }

    fn start<T: 'static>(
        wake: Waker,
        runtime: Handle,
        output: impl FnOnce(mpsc::Sender<Message>) -> Result<(Mixer, OutputFormat, T), String>
            + Send
            + 'static,
    ) -> Result<Self, String> {
        let (messages, input) = mpsc::channel();
        let (notify, events) = mpsc::channel();
        let (ready, opened) = mpsc::sync_channel(1);
        let worker_messages = messages.clone();
        let snapshot = Arc::new(Mutex::new((0, PlayerSnapshot::default(), true)));
        let values = snapshot.clone();
        let generation = Arc::new(AtomicU64::new(0));
        let next_epoch = Arc::new(AtomicU64::new(0));
        let playback_generation = generation.clone();
        let playback_next_epoch = next_epoch.clone();
        let worker = thread::Builder::new()
            .name("gramwave-audio".into())
            .spawn(move || {
                let (mixer, format, _output) = match output(worker_messages.clone()) {
                    Ok(output) => output,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                if ready.send(Ok(())).is_ok() {
                    Owner::new(
                        mixer,
                        format,
                        runtime,
                        worker_messages,
                        playback_generation,
                        playback_next_epoch,
                    )
                    .run(input, notify, values, wake);
                }
            })
            .map_err(|e| e.to_string())?;
        opened
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())??;
        Ok(Self {
            messages,
            worker: Some(worker),
            generation,
            next_epoch,
            events,
            snapshot,
        })
    }
}

impl PlayerBackend for Player {
    fn prepares_streamed_next(&self) -> bool {
        true
    }

    fn command(&self, command: PlayerCommand) -> Result<(), AppError> {
        match &command {
            PlayerCommand::Load { path, .. } if path.contains('\0') => {
                return Err(AppError::Player("Invalid media path".into()));
            }
            PlayerCommand::Seek(seconds) if !seconds.is_finite() => {
                return Err(AppError::Player("Invalid seek position".into()));
            }
            _ => {}
        }
        let generation = match command {
            PlayerCommand::Load { .. } | PlayerCommand::Seek(_) | PlayerCommand::Stop => {
                self.generation.fetch_add(1, Relaxed) + 1
            }
            _ => self.generation.load(Relaxed),
        };
        let next_epoch = if matches!(
            command,
            PlayerCommand::QueueNext(_) | PlayerCommand::QueueNextPending
        ) {
            self.next_epoch.fetch_add(1, Relaxed) + 1
        } else {
            self.next_epoch.load(Relaxed)
        };
        self.messages
            .send(Message::Command(generation, next_epoch, command))
            .map_err(|_| AppError::Player("Audio output is closed".into()))
    }

    fn drain(&self) -> Vec<PlaybackEvent> {
        let generation = self.generation.load(Relaxed);
        self.events
            .try_iter()
            .filter_map(|(id, event)| (id == generation).then_some(event))
            .collect()
    }

    fn snapshot(&self) -> PlayerSnapshot {
        let (generation, mut snapshot, _) = *self.snapshot.lock().unwrap();
        if generation != self.generation.load(Relaxed) {
            snapshot.checkpoint_position = None;
        }
        snapshot
    }

    fn is_idle(&self) -> bool {
        let (generation, _, idle) = *self.snapshot.lock().unwrap();
        idle && generation == self.generation.load(Relaxed)
    }

    fn shutdown(&mut self, timeout: Duration) -> Result<(), AppError> {
        let _ = self.messages.send(Message::Shutdown);
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        let start = std::time::Instant::now();
        while !worker.is_finished() {
            if start.elapsed() >= timeout {
                return Err(AppError::Timeout("Audio shutdown"));
            }
            thread::sleep(Duration::from_millis(2));
        }
        worker
            .join()
            .map_err(|_| AppError::Player("Audio thread panicked".into()))
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.messages.send(Message::Shutdown);
    }
}

struct Owner {
    output: rtrb::Producer<OutputTrack>,
    retired: rtrb::Consumer<OutputTrack>,
    format: OutputFormat,
    next_epoch: u64,
    runtime: Handle,
    messages: mpsc::Sender<Message>,
    slots: Arc<Semaphore>,
    generation: u64,
    serial: u64,
    controls: Arc<Controls>,
    current: Option<Track>,
    current_queued: Option<QueuedPlayback>,
    next: Option<(QueuedPlayback, Track)>,
    promoting: Option<(RequestId, u64)>,
    resolving_next: bool,
    snapshot: PlayerSnapshot,
}

struct Track {
    id: u64,
    path: String,
    cancel: CancellationToken,
    prepared: Option<Prepared>,
    seeking: bool,
    promote: Option<CancellationToken>,
    file: Option<Arc<File>>,
}

impl Drop for Track {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

struct Prepared {
    source: Option<Buffered>,
    pcm: Pcm,
    duration: f64,
    progress: Arc<Progress>,
    file: Option<Arc<File>>,
}

#[derive(Default)]
struct Progress {
    played: AtomicU64,
    eof: AtomicBool,
    drained: AtomicBool,
}

struct Controls {
    paused: AtomicBool,
    volume: AtomicU32,
    promote: AtomicU64,
}

struct Buffered {
    cancel: CancellationToken,
    progress: Arc<Progress>,
    ring: rtrb::Consumer<f32>,
}

struct OutputTrack {
    id: u64,
    generation: u64,
    next_epoch: Option<u64>,
    audio: Buffered,
}

struct Output {
    incoming: rtrb::Consumer<OutputTrack>,
    retired: rtrb::Producer<OutputTrack>,
    current: Option<OutputTrack>,
    next: Option<OutputTrack>,
    format: OutputFormat,
    generation: Arc<AtomicU64>,
    next_epoch: Arc<AtomicU64>,
    controls: Arc<Controls>,
    remaining: u16,
    play_frame: bool,
}

impl Output {
    fn begin_frame(&mut self) -> bool {
        let generation = self.generation.load(Relaxed);
        let epoch = self.next_epoch.load(Relaxed);
        let promote = self
            .controls
            .promote
            .load(std::sync::atomic::Ordering::Acquire);
        let valid = |track: &OutputTrack| {
            track.generation == generation
                && !track.audio.cancel.is_cancelled()
                && track.next_epoch.is_none_or(|candidate| candidate == epoch)
        };
        if self.current.as_ref().is_some_and(|track| !valid(track)) {
            let _ = self.retired.push(self.current.take().unwrap());
        }
        if self.next.as_ref().is_some_and(|track| !valid(track)) {
            let _ = self.retired.push(self.next.take().unwrap());
        }
        while let Ok(track) = self.incoming.pop() {
            if valid(&track) {
                let slot = if track.next_epoch.is_some() {
                    &mut self.next
                } else {
                    &mut self.current
                };
                if let Some(old) = slot.replace(track) {
                    let _ = self.retired.push(old);
                }
            } else {
                let _ = self.retired.push(track);
            }
        }
        if self.controls.paused.load(Relaxed) {
            return false;
        }
        if promote != 0 {
            if self
                .current
                .as_ref()
                .is_some_and(|track| track.id != promote)
            {
                let _ = self.retired.push(self.current.take().unwrap());
            }
            if self.next.as_ref().is_some_and(|track| track.id == promote) {
                let mut next = self.next.take().unwrap();
                next.next_epoch = None;
                self.current = Some(next);
            }
        } else if let Some(current) = &self.current {
            let audio = &current.audio;
            if audio
                .progress
                .eof
                .load(std::sync::atomic::Ordering::Acquire)
                && audio.ring.is_empty()
            {
                audio.progress.drained.store(true, Relaxed);
                if let Some(mut next) = self.next.take() {
                    next.next_epoch = None;
                    let old = self.current.replace(next).unwrap();
                    let _ = self.retired.push(old);
                }
            }
        }
        self.current
            .as_ref()
            .is_some_and(|track| track.audio.ring.slots() >= usize::from(self.format.0.get()))
    }
}

impl Iterator for Output {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.remaining == 0 {
            self.play_frame = self.begin_frame();
            self.remaining = self.format.0.get();
        }
        self.remaining -= 1;
        if self.play_frame {
            let audio = &mut self.current.as_mut().unwrap().audio;
            let sample = audio.ring.pop().expect("PCM frame was available");
            if self.remaining == 0 {
                audio.progress.played.fetch_add(1, Relaxed);
            }
            Some(sample * f32::from_bits(self.controls.volume.load(Relaxed)))
        } else {
            Some(0.)
        }
    }
}

impl Source for Output {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> NonZeroU16 {
        self.format.0
    }
    fn sample_rate(&self) -> NonZeroU32 {
        self.format.1
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl Owner {
    fn new(
        mixer: Mixer,
        format: OutputFormat,
        runtime: Handle,
        messages: mpsc::Sender<Message>,
        generation: Arc<AtomicU64>,
        next_epoch: Arc<AtomicU64>,
    ) -> Self {
        let (output, incoming) = rtrb::RingBuffer::new(4);
        let (retire, retired) = rtrb::RingBuffer::new(8);
        let controls = Arc::new(Controls {
            paused: AtomicBool::new(false),
            volume: AtomicU32::new(1f32.to_bits()),
            promote: AtomicU64::new(0),
        });
        mixer.add(Output {
            incoming,
            retired: retire,
            current: None,
            next: None,
            format,
            generation,
            next_epoch,
            controls: controls.clone(),
            remaining: 0,
            play_frame: false,
        });
        Self {
            output,
            retired,
            format,
            next_epoch: 0,
            runtime,
            messages,
            slots: Arc::new(Semaphore::new(2)),
            generation: 0,
            serial: 0,
            controls,
            current: None,
            current_queued: None,
            next: None,
            promoting: None,
            resolving_next: false,
            snapshot: PlayerSnapshot {
                volume: 100.,
                ..Default::default()
            },
        }
    }

    fn start(
        &mut self,
        path: String,
        seconds: f64,
        seeking: bool,
        mut file: Option<Arc<File>>,
        speculative: bool,
    ) -> Track {
        self.serial += 1;
        let id = self.serial;
        let cancel = CancellationToken::new();
        let job_cancel = cancel.clone();
        let promote = (speculative && path.starts_with("http://")).then(CancellationToken::new);
        let job_promote = promote.clone();
        let job_path = path.clone();
        let format = self.format;
        let messages = self.messages.clone();
        let runtime = self.runtime.clone();
        let slots = self.slots.clone();
        self.runtime.spawn(async move {
            let slot = tokio::select! {
                biased;
                _ = job_cancel.cancelled() => return,
                slot = slots.acquire_owned() => slot.expect("audio semaphore stays open"),
            };
            let job = job_cancel.clone();
            let error_sender = messages.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                let _slot = slot;
                if job.is_cancelled() {
                    return Ok(());
                }
                if file.is_none() && !job_path.starts_with("http://") {
                    file = Some(Arc::new(File::open(&job_path).map_err(|e| e.to_string())?));
                }
                decode(
                    id,
                    &job_path,
                    seconds,
                    file,
                    runtime,
                    job,
                    job_promote,
                    format,
                    messages,
                )
            })
            .await;
            let error = match outcome {
                Ok(Ok(())) => return,
                Ok(Err(error)) => error,
                Err(error) => error.to_string(),
            };
            if !job_cancel.is_cancelled() {
                let _ = error_sender.send(Message::Failed(id, error));
            }
        });
        Track {
            id,
            path,
            cancel,
            prepared: None,
            seeking,
            promote,
            file: None,
        }
    }

    fn apply(&mut self, message: Message, events: &mut Vec<PlaybackEvent>) -> bool {
        match message {
            Message::Shutdown => return false,
            Message::Command(generation, next_epoch, command) => {
                if generation != self.generation {
                    events.clear();
                    self.generation = generation;
                    self.current_queued = None;
                    self.promoting = None;
                    self.controls.promote.store(0, Relaxed);
                }
                match command {
                    PlayerCommand::Load { path, start_ms } => {
                        self.current.take();
                        self.next.take();
                        self.resolving_next = false;
                        self.reset_position(start_ms as f64 / 1000., 0., start_ms != 0);
                        self.current =
                            Some(self.start(path, self.snapshot.position, false, None, false));
                    }
                    PlayerCommand::Seek(seconds) => {
                        if let Some(current) = self.current.take() {
                            let seconds = if self.snapshot.duration > 0. {
                                seconds.clamp(0., (self.snapshot.duration - 0.001).max(0.))
                            } else {
                                seconds.max(0.)
                            };
                            let queued = self.next.take().map(|(queued, _)| queued);
                            current.cancel.cancel();
                            self.reset_position(seconds, self.snapshot.duration, true);
                            self.current = Some(self.start(
                                current.path.clone(),
                                seconds,
                                true,
                                current.file.clone(),
                                false,
                            ));
                            self.current.as_mut().unwrap().file = current.file.clone();
                            if let Some(queued) = queued {
                                let track = self.start(queued.path.clone(), 0., false, None, true);
                                self.next = Some((queued, track));
                            }
                        }
                    }
                    PlayerCommand::Stop => {
                        self.current.take();
                        self.next.take();
                        self.resolving_next = false;
                        self.reset_position(0., 0., false);
                        events.push(PlaybackEvent::Stopped);
                    }
                    PlayerCommand::QueueNextPending => {
                        self.next_epoch = next_epoch;
                        self.next.take();
                        self.resolving_next = true;
                    }
                    PlayerCommand::QueueNext(next) => {
                        self.resolving_next = false;
                        self.next_epoch = next_epoch;
                        self.next.take();
                        if let Some(next) = next.filter(|_| self.current.is_some()) {
                            let track = self.start(next.path.clone(), 0., false, None, true);
                            self.next = Some((next, track));
                        }
                    }
                    PlayerCommand::PromoteNext {
                        request,
                        entry_id,
                        revision,
                    } => {
                        let matches = |next: &QueuedPlayback| {
                            next.entry_id == entry_id && next.revision == revision
                        };
                        if let Some((_, track)) =
                            self.next.as_ref().filter(|(next, _)| matches(next))
                        {
                            if let Some(promote) = &track.promote {
                                promote.cancel();
                            }
                            self.promoting = Some((request, track.id));
                            self.reset_position(0., 0., false);
                        } else if let Some(next) =
                            self.current_queued.as_ref().filter(|next| matches(next))
                        {
                            events.push(PlaybackEvent::NextPromoted(request, next.clone()));
                        } else {
                            events.push(PlaybackEvent::NextUnavailable(request));
                            return true;
                        }
                        self.controls.paused.store(false, Relaxed);
                        self.snapshot.paused = false;
                    }
                    PlayerCommand::Pause(paused) => {
                        self.controls.paused.store(paused, Relaxed);
                        self.snapshot.paused = paused;
                    }
                    PlayerCommand::Volume(volume) if volume.is_finite() => {
                        let volume = volume.clamp(0., 100.);
                        self.controls
                            .volume
                            .store((volume / 100.).to_bits(), Relaxed);
                        self.snapshot.volume = f64::from(volume);
                    }
                    PlayerCommand::Volume(_) => {}
                }
            }
            Message::Ready(id, prepared) => {
                if let Some(current) = self.current.as_mut().filter(|track| track.id == id) {
                    current.file = prepared.file.clone();
                    current.prepared = Some(prepared);
                    if !current.seeking {
                        events.push(PlaybackEvent::Loaded);
                    }
                } else if let Some((_, next)) = self.next.as_mut().filter(|(_, next)| next.id == id)
                {
                    next.prepared = Some(prepared);
                }
            }
            Message::Failed(id, error) => {
                if self
                    .current
                    .as_ref()
                    .is_some_and(|current| current.id == id && self.promoting.is_none())
                {
                    self.fail(error, events);
                } else if self.next.as_ref().is_some_and(|(_, next)| next.id == id) {
                    self.next.take();
                    if let Some((request, _)) = self.promoting.take() {
                        self.current.take();
                        self.current_queued = None;
                        events.push(PlaybackEvent::NextUnavailable(request));
                    }
                }
            }
            Message::OutputError(error) if self.current.is_some() => self.fail(error, events),
            Message::OutputError(_) => {}
        }
        true
    }

    fn fail(&mut self, error: String, events: &mut Vec<PlaybackEvent>) {
        self.current.take();
        self.current_queued = None;
        self.next.take();
        self.promoting = None;
        self.resolving_next = false;
        self.reset_position(0., 0., false);
        events.push(PlaybackEvent::Failed(AppError::Player(error)));
    }

    fn reset_position(&mut self, position: f64, duration: f64, seeking: bool) {
        self.snapshot.position = position;
        self.snapshot.duration = duration;
        self.snapshot.seeking = seeking;
        self.snapshot.audio_position = None;
        self.snapshot.checkpoint_position = None;
    }

    fn reconcile_output(&mut self, events: &mut Vec<PlaybackEvent>) {
        if self
            .next
            .as_ref()
            .and_then(|(_, track)| track.prepared.as_ref())
            .is_some_and(|ready| ready.progress.played.load(Relaxed) != 0)
        {
            let (next, mut track) = self.next.take().unwrap();
            if let Some(promote) = &track.promote {
                promote.cancel();
            }
            track.file = track.prepared.as_ref().and_then(|ready| ready.file.clone());
            self.current = Some(track);
            self.current_queued = Some(next.clone());
            events.push(match self.promoting.take() {
                Some((request, _)) => PlaybackEvent::NextPromoted(request, next),
                None => PlaybackEvent::Advanced(next),
            });
        }
    }

    fn update(&mut self, events: &mut Vec<PlaybackEvent>) {
        while self.retired.pop().is_ok() {}
        self.reconcile_output(events);
        if self.promoting.is_none()
            && self
                .current
                .as_ref()
                .and_then(|track| track.prepared.as_ref())
                .is_some_and(|ready| ready.progress.drained.load(Relaxed))
        {
            if let Some((_, next)) = &self.next {
                if let Some(promote) = &next.promote {
                    promote.cancel();
                }
            } else if !self.resolving_next {
                self.current.take();
                self.current_queued = None;
                self.reset_position(0., 0., false);
                events.push(PlaybackEvent::Finished);
            }
        }
        let current = self.current.as_mut().filter(|_| self.promoting.is_none());
        let next = self.next.as_mut().map(|(_, track)| track);
        for (track, epoch) in [(current, None), (next, Some(self.next_epoch))] {
            if let Some((id, ready)) =
                track.and_then(|track| Some((track.id, track.prepared.as_mut()?)))
            {
                if let Some(audio) = ready.source.take() {
                    let track = OutputTrack {
                        id,
                        generation: self.generation,
                        next_epoch: epoch,
                        audio,
                    };
                    if let Err(rtrb::PushError::Full(track)) = self.output.push(track) {
                        ready.source = Some(track.audio);
                    }
                }
                if epoch.is_none() {
                    let played = ready.progress.played.load(Relaxed);
                    self.snapshot.duration = ready.duration;
                    if played > 0 || self.snapshot.paused {
                        let position =
                            ready.pcm.position + played as f64 / f64::from(self.format.1.get());
                        self.snapshot.seeking = false;
                        self.snapshot.position = position;
                        self.snapshot.audio_position = Some(position);
                        self.snapshot.checkpoint_position = Some(position);
                    }
                }
            }
        }
        let promote = self.promoting.filter(|_| {
            self.next.as_ref().is_some_and(|(_, track)| {
                track
                    .prepared
                    .as_ref()
                    .is_none_or(|ready| ready.source.is_none())
            })
        });
        if let Some((request, id)) = promote {
            if self.controls.promote.load(Relaxed) != id
                && self
                    .next
                    .as_ref()
                    .is_some_and(|(_, track)| track.prepared.is_none())
            {
                events.push(PlaybackEvent::NextBuffering(request));
            }
        }
        self.controls.promote.store(
            promote.map_or(0, |(_, id)| id),
            std::sync::atomic::Ordering::Release,
        );
    }

    fn run(
        &mut self,
        input: mpsc::Receiver<Message>,
        notify: mpsc::Sender<(u64, PlaybackEvent)>,
        snapshot: Arc<Mutex<(u64, PlayerSnapshot, bool)>>,
        wake: Waker,
    ) {
        loop {
            let first = if self.current.is_some() {
                match input.recv_timeout(POLL) {
                    Ok(message) => Some(message),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            } else {
                let Ok(message) = input.recv() else {
                    break;
                };
                Some(message)
            };
            let mut events = Vec::new();
            let commanded = first.is_some();
            self.reconcile_output(&mut events);
            for message in first.into_iter().chain(input.try_iter()) {
                if !self.apply(message, &mut events) {
                    return;
                }
            }
            self.update(&mut events);
            *snapshot.lock().unwrap() = (self.generation, self.snapshot, self.current.is_none());
            if commanded || !events.is_empty() {
                for event in events {
                    let _ = notify.send((self.generation, event));
                }
                wake.wake();
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn decode(
    id: u64,
    path: &str,
    seconds: f64,
    file: Option<Arc<File>>,
    runtime: Handle,
    cancel: CancellationToken,
    promote: Option<CancellationToken>,
    format: OutputFormat,
    messages: mpsc::Sender<Message>,
) -> Result<(), String> {
    let mut decoder = Decode::open(path, file.clone(), runtime, cancel.clone(), promote)
        .map_err(|e| e.to_string())?;
    if seconds > 0. {
        decoder.seek(seconds).map_err(|e| e.to_string())?;
    }
    let mut samples = Vec::new();
    let pcm = decoder
        .next(seconds, &mut samples)
        .map_err(|e| e.to_string())?
        .ok_or("Empty audio stream")?;
    let channels = NonZeroU16::new(pcm.channels).ok_or("Invalid channel count")?;
    let rate = NonZeroU32::new(pcm.rate).ok_or("Invalid sample rate")?;
    let duration = decoder.duration;
    let mut decoded = Decoded {
        decoder,
        samples,
        at: 0,
        channels: pcm.channels,
        rate: pcm.rate,
        error: None,
    };
    let resampled =
        rodio::conversions::SampleRateConverter::new(&mut decoded, rate, format.1, channels);
    let mut converted =
        rodio::conversions::ChannelCountConverter::new(resampled, channels, format.0);
    let channels = usize::from(format.0.get());
    let (mut producer, ring) =
        rtrb::RingBuffer::new(channels * (format.1.get() as usize / 5).max(1));
    let progress = Arc::new(Progress::default());
    let source = Buffered {
        cancel: cancel.clone(),
        progress: progress.clone(),
        ring,
    };
    let mut chunk = Vec::with_capacity(channels * 256);
    let mut ready = Some(Prepared {
        source: Some(source),
        pcm,
        duration,
        progress: progress.clone(),
        file,
    });
    while !cancel.is_cancelled() {
        let count = producer.slots().min(chunk.capacity()) / channels * channels;
        if count == 0 {
            thread::sleep(POLL);
            continue;
        }
        chunk.clear();
        chunk.extend(converted.by_ref().take(count));
        if chunk.is_empty() {
            break;
        }
        producer
            .push_entire_slice(&chunk)
            .expect("PCM slots were available");
        if let Some(ready) = ready.take() {
            messages
                .send(Message::Ready(id, ready))
                .map_err(|_| "Audio output is closed")?;
        }
    }
    if let Some(error) = decoded.error {
        return Err(error);
    }
    if !cancel.is_cancelled() && ready.is_some() {
        return Err("Empty audio stream".into());
    }
    progress
        .eof
        .store(true, std::sync::atomic::Ordering::Release);
    Ok(())
}

struct Decoded {
    decoder: Decode,
    samples: Vec<f32>,
    at: usize,
    channels: u16,
    rate: u32,
    error: Option<String>,
}

impl Iterator for Decoded {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.at == self.samples.len() {
            if self.error.is_some() {
                return None;
            }
            match self.decoder.next(0., &mut self.samples) {
                Ok(Some(pcm)) if pcm.channels == self.channels && pcm.rate == self.rate => {
                    self.at = 0
                }
                Ok(Some(_)) => self.error = Some("Audio format changed within the track".into()),
                Ok(None) => return None,
                Err(error) => self.error = Some(error.to_string()),
            }
            if self.error.is_some() {
                return None;
            }
        }
        let value = self.samples[self.at];
        self.at += 1;
        Some(value)
    }
}

#[cfg(test)]
mod tests;
