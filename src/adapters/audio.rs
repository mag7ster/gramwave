pub use crate::application::ports::PlaybackEvent;
use gramwave_application::AppError;
use rsmpv::{EndFileReason, Event, Mpv};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

struct NativePlayer {
    mpv: Mpv,
    active_entry: std::cell::Cell<Option<i64>>,
    waiting_for_entry: std::cell::Cell<bool>,
    started_entry: std::cell::Cell<Option<i64>>,
    ready_entry: std::cell::Cell<Option<i64>>,
    command_errors: std::cell::RefCell<Vec<String>>,
}
impl NativePlayer {
    fn with_audio_output(output: Option<&str>) -> Result<Self, String> {
        let mut builder = Mpv::builder().map_err(|error| error.to_string())?;
        for (name, value) in [
            ("video", "no"),
            ("terminal", "no"),
            ("audio-display", "no"),
            ("idle", "yes"),
        ] {
            builder = builder
                .set_property(name, value)
                .map_err(|error| format!("set {name}: {error}"))?;
        }
        if let Some(output) = output {
            builder = builder
                .set_property("ao", output)
                .map_err(|error| format!("set ao: {error}"))?;
        }
        Ok(Self {
            mpv: builder
                .build()
                .map_err(|error| format!("mpv_initialize: {error}"))?,
            active_entry: Default::default(),
            waiting_for_entry: Default::default(),
            started_entry: Default::default(),
            ready_entry: Default::default(),
            command_errors: Default::default(),
        })
    }
    fn cmd(&self, args: &[&str], start_ms: Option<u64>) {
        let result = start_ms.map_or(Ok(()), |ms| {
            let start = if ms == 0 {
                "none".into()
            } else {
                (ms as f64 / 1000.).to_string()
            };
            self.try_cmd(&["set", "options/start", &start])
        });
        if let Err(error) = result.and_then(|()| self.try_cmd(args)) {
            self.command_errors.borrow_mut().push(error);
        }
    }
    pub fn try_cmd(&self, args: &[&str]) -> Result<(), String> {
        // Completed events for the previous file cannot advance a newly chosen
        // track. Manual replacement generates END_FILE with reason STOP.
        if args.first() == Some(&"loadfile") {
            let _ = self.drain();
            self.ready_entry.set(None);
        }
        if args.first() == Some(&"stop") {
            self.active_entry.set(None);
            self.waiting_for_entry.set(false);
        }
        self.mpv.command(args).map_err(|error| error.to_string())?;
        if args.first() == Some(&"loadfile") {
            let entry = self.mpv.get_property::<i64>("playlist/0/id").ok();
            self.active_entry.set(entry);
            self.waiting_for_entry.set(entry.is_none());
        }
        Ok(())
    }
    pub fn drain(&self) -> Vec<PlaybackEvent> {
        let mut events: Vec<_> = self
            .command_errors
            .borrow_mut()
            .drain(..)
            .map(|e| PlaybackEvent::Failed(AppError::Player(e)))
            .collect();
        while let Some(event) = self.mpv.poll_event() {
            match event {
                Event::EndFile {
                    reason,
                    error,
                    playlist_entry_id,
                    ..
                } => {
                    let current = self.active_entry.get() == Some(playlist_entry_id);
                    if !current && reason != EndFileReason::Stop {
                        continue;
                    }
                    if current {
                        self.active_entry.set(None);
                    }
                    match reason {
                        EndFileReason::Eof => events.push(PlaybackEvent::Finished),
                        EndFileReason::Stop => events.push(PlaybackEvent::Stopped),
                        EndFileReason::Error => {
                            let message = error
                                .map(|error| error.to_string())
                                .unwrap_or_else(|| "mpv playback failed".into());
                            events.push(PlaybackEvent::Failed(AppError::Player(message)));
                        }
                        _ => {}
                    }
                }
                Event::StartFile { playlist_entry_id } => {
                    if self.waiting_for_entry.replace(false) {
                        self.active_entry.set(Some(playlist_entry_id));
                    }
                    self.started_entry.set(Some(playlist_entry_id));
                }
                Event::FileLoaded if self.started_entry.get() == self.active_entry.get() => {
                    events.push(PlaybackEvent::Loaded);
                }
                Event::PlaybackRestart if self.started_entry.get() == self.active_entry.get() => {
                    self.ready_entry.set(self.active_entry.get());
                }
                Event::CommandReply {
                    result: Err(error), ..
                } => {
                    events.push(PlaybackEvent::Failed(AppError::Player(error.to_string())));
                }
                _ => {}
            }
        }
        events
    }
    fn flag(&self, name: &str) -> bool {
        self.mpv.get_property::<bool>(name).unwrap_or_default()
    }
    fn number(&self, name: &str) -> f64 {
        self.number_if_available(name).unwrap_or_default()
    }
    fn number_if_available(&self, name: &str) -> Option<f64> {
        self.mpv
            .get_property::<f64>(name)
            .ok()
            .filter(|n| n.is_finite())
    }
}

fn publish_snapshot(
    shared: &Mutex<crate::application::ports::PlayerSnapshot>,
    read: impl FnOnce() -> crate::application::ports::PlayerSnapshot,
) -> crate::application::ports::PlayerSnapshot {
    let snapshot = read();
    *shared.lock().unwrap() = snapshot;
    snapshot
}
struct Request {
    generation: u64,
    args: Vec<String>,
    start_ms: Option<u64>,
}

pub struct Player {
    sender: Option<mpsc::Sender<Request>>,
    generation: AtomicU64,
    active_load: Arc<AtomicU64>,
    events: mpsc::Receiver<(u64, PlaybackEvent)>,
    snapshot: Arc<Mutex<crate::application::ports::PlayerSnapshot>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Player {
    pub fn new(wake: fastframe_shell::Waker) -> Result<Self, String> {
        Self::with_audio_output(None, wake)
    }
    fn with_audio_output(
        output: Option<&str>,
        wake: fastframe_shell::Waker,
    ) -> Result<Self, String> {
        let output = output.map(str::to_owned);
        let (sender, commands) = mpsc::channel::<Request>();
        let (tx, events) = mpsc::channel();
        let (ready, started) = mpsc::sync_channel(1);
        let snapshot = Arc::new(Mutex::new(Default::default()));
        let values = snapshot.clone();
        let active_load = Arc::new(AtomicU64::new(0));
        let owner_load = active_load.clone();
        let worker = thread::Builder::new()
            .name("tg-mpv".into())
            .spawn(move || {
                let native = match NativePlayer::with_audio_output(output.as_deref()) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = ready.send(Err(e));
                        return;
                    }
                };
                let owner = thread::current();
                native.mpv.set_wakeup_callback(move || owner.unpark());
                let _ = ready.send(Ok(()));
                let mut generation = 0;
                let mut snapshot = crate::application::ports::PlayerSnapshot::default();
                loop {
                    let commanded = match commands.try_recv() {
                        Ok(request) => {
                            generation = request.generation;
                            let strings: Vec<_> = request.args.iter().map(String::as_str).collect();
                            native.cmd(&strings, request.start_ms);
                            true
                        }
                        Err(mpsc::TryRecvError::Disconnected) => break,
                        Err(mpsc::TryRecvError::Empty) => false,
                    };
                    let events = native.drain();
                    if native.active_entry.get().is_none()
                        && !native.waiting_for_entry.get()
                        && native.flag("idle-active")
                    {
                        let pending = owner_load.load(Ordering::Acquire);
                        if pending <= generation {
                            let _ = owner_load.compare_exchange(
                                pending,
                                0,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            );
                        }
                    }
                    let previous = snapshot;
                    snapshot = publish_snapshot(&values, || {
                        let seeking = native.flag("seeking");
                        let position = native.number_if_available("time-pos");
                        let ready = native
                            .active_entry
                            .get()
                            .is_some_and(|entry| native.ready_entry.get() == Some(entry));
                        crate::application::ports::PlayerSnapshot {
                            paused: native.flag("pause"),
                            seeking,
                            position: position.unwrap_or_default(),
                            checkpoint_position: position.filter(|_| ready && !seeking),
                            audio_position: native.number_if_available("audio-pts"),
                            duration: native.number("duration"),
                            volume: native.number("volume"),
                        }
                    });
                    let notify = commanded
                        || !events.is_empty()
                        || previous.paused != snapshot.paused
                        || previous.seeking != snapshot.seeking
                        || previous.duration != snapshot.duration
                        || (snapshot.paused && previous.position != snapshot.position);
                    for event in events {
                        let _ = tx.send((generation, event));
                    }
                    if notify {
                        wake.wake();
                    }
                    if !commanded {
                        if snapshot.seeking
                            || (native.active_entry.get().is_some() && !snapshot.paused)
                        {
                            thread::park_timeout(Duration::from_millis(10));
                        } else {
                            thread::park();
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        started
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())??;
        Ok(Self {
            sender: Some(sender),
            generation: AtomicU64::new(0),
            active_load,
            events,
            snapshot,
            worker: Some(worker),
        })
    }
    pub fn try_cmd(&self, args: &[&str]) -> Result<(), String> {
        self.submit(args, None)
    }
    fn submit(&self, args: &[&str], start_ms: Option<u64>) -> Result<(), String> {
        if args.iter().any(|s| s.contains('\0')) {
            return Err("mpv command contains NUL".into());
        }
        let generation = if matches!(args.first(), Some(&"loadfile" | &"stop")) {
            self.generation.fetch_add(1, Ordering::AcqRel) + 1
        } else {
            self.generation.load(Ordering::Acquire)
        };
        let is_load = args.first() == Some(&"loadfile");
        if is_load {
            self.active_load.store(generation, Ordering::Release);
        }
        let result = self
            .sender
            .as_ref()
            .ok_or("mpv is closed".to_owned())
            .and_then(|sender| {
                sender
                    .send(Request {
                        generation,
                        args: args.iter().map(|s| (*s).into()).collect(),
                        start_ms,
                    })
                    .map_err(|e| e.to_string())
            });
        if result.is_err() && is_load {
            let _ = self.active_load.compare_exchange(
                generation,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
        result?;
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
        Ok(())
    }
    pub fn drain(&self) -> Vec<PlaybackEvent> {
        let generation = self.generation.load(Ordering::Acquire);
        self.events
            .try_iter()
            .filter(|(id, _)| *id == generation)
            .map(|(_, e)| e)
            .collect()
    }
    pub fn number(&self, name: &str) -> f64 {
        let value = *self.snapshot.lock().unwrap();
        match name {
            "time-pos" => value.position,
            "duration" => value.duration,
            "volume" => value.volume,
            "pause" => f64::from(value.paused),
            "seeking" => f64::from(value.seeking),
            _ => 0.,
        }
    }

    pub fn shutdown(&mut self, timeout: Duration) -> Result<(), String> {
        self.sender.take();
        let deadline = std::time::Instant::now() + timeout;
        if let Some(worker) = self.worker.as_ref() {
            worker.thread().unpark();
            while !worker.is_finished() {
                if std::time::Instant::now() >= deadline {
                    self.worker.take();
                    return Err("mpv shutdown timed out".into());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "mpv worker panicked".to_owned())?;
        }
        Ok(())
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
    }
}
use crate::application::ports::PlayerBackend;
impl PlayerBackend for Player {
    fn command(
        &self,
        command: crate::application::ports::PlayerCommand,
    ) -> Result<(), gramwave_application::AppError> {
        use crate::application::ports::PlayerCommand;
        match command {
            PlayerCommand::Load { path, start_ms } => {
                self.submit(&["loadfile", &path, "replace"], Some(start_ms))
            }
            PlayerCommand::Pause(paused) => {
                self.try_cmd(&["set", "pause", if paused { "yes" } else { "no" }])
            }
            PlayerCommand::Stop => self.try_cmd(&["stop"]),
            PlayerCommand::Seek(position) => {
                self.try_cmd(&["seek", &position.to_string(), "absolute+exact"])
            }
            PlayerCommand::Volume(volume) => self.try_cmd(&["set", "volume", &volume.to_string()]),
        }
        .map_err(gramwave_application::AppError::Player)
    }
    fn drain(&self) -> Vec<PlaybackEvent> {
        self.drain()
    }
    fn snapshot(&self) -> crate::application::ports::PlayerSnapshot {
        *self.snapshot.lock().unwrap()
    }
    fn is_idle(&self) -> bool {
        self.active_load.load(Ordering::Acquire) == 0
    }
    fn shutdown(&mut self, timeout: Duration) -> Result<(), gramwave_application::AppError> {
        self.shutdown(timeout)
            .map_err(gramwave_application::AppError::Player)
    }
}
#[cfg(test)]
#[path = "audio/tests.rs"]
mod tests;
