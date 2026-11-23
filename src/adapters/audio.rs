use libloading::Library;
use std::ffi::{c_char, c_void, CStr, CString};
#[allow(non_camel_case_types, non_upper_case_globals, dead_code)]
#[path = "mpv_abi.rs"]
mod abi;
use abi::{mpv_event as MpvEvent, mpv_event_end_file as MpvEndFile};
type Command = unsafe extern "C" fn(*mut c_void, *const *const c_char) -> i32;
type Wakeup = unsafe extern "C" fn(*mut c_void);
pub use crate::application::ports::PlaybackEvent;
struct NativePlayer {
    _lib: Library,
    handle: *mut c_void,
    command: Command,
    active_entry: std::cell::Cell<Option<i64>>,
    waiting_for_entry: std::cell::Cell<bool>,
    started_entry: std::cell::Cell<Option<i64>>,
    ready_entry: std::cell::Cell<Option<i64>>,
    command_errors: std::cell::RefCell<Vec<String>>,
    destroy: unsafe extern "C" fn(*mut c_void),
    wait: unsafe extern "C" fn(*mut c_void, f64) -> *const MpvEvent,
    set_wakeup: unsafe extern "C" fn(*mut c_void, Option<Wakeup>, *mut c_void),
    error_string: unsafe extern "C" fn(i32) -> *const c_char,
    get: unsafe extern "C" fn(*mut c_void, *const c_char, i32, *mut c_void) -> i32,
}
impl NativePlayer {
    fn with_audio_output(output: Option<&str>) -> Result<Self, String> {
        unsafe {
            let lib = Library::new("libmpv.so.2").map_err(|e| e.to_string())?;
            let create: unsafe extern "C" fn() -> *mut c_void =
                *lib.get(b"mpv_create\0").map_err(|e| e.to_string())?;
            let init: unsafe extern "C" fn(*mut c_void) -> i32 =
                *lib.get(b"mpv_initialize\0").map_err(|e| e.to_string())?;
            let set: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i32 = *lib
                .get(b"mpv_set_option_string\0")
                .map_err(|e| e.to_string())?;
            let mut player = Self {
                handle: std::ptr::null_mut(),
                active_entry: Default::default(),
                waiting_for_entry: Default::default(),
                started_entry: Default::default(),
                ready_entry: Default::default(),
                command_errors: Default::default(),
                command: *lib.get(b"mpv_command\0").map_err(|e| e.to_string())?,
                wait: *lib.get(b"mpv_wait_event\0").map_err(|e| e.to_string())?,
                set_wakeup: *lib
                    .get(b"mpv_set_wakeup_callback\0")
                    .map_err(|e| e.to_string())?,
                error_string: *lib.get(b"mpv_error_string\0").map_err(|e| e.to_string())?,
                destroy: *lib
                    .get(b"mpv_terminate_destroy\0")
                    .map_err(|e| e.to_string())?,
                get: *lib.get(b"mpv_get_property\0").map_err(|e| e.to_string())?,
                _lib: lib,
            };
            player.handle = create();
            let handle = player.handle;
            if handle.is_null() {
                return Err("mpv_create failed".into());
            }
            for (k, v) in [
                ("video", "no"),
                ("terminal", "no"),
                ("audio-display", "no"),
                ("idle", "yes"),
            ] {
                let code = set(
                    handle,
                    CString::new(k).unwrap().as_ptr(),
                    CString::new(v).unwrap().as_ptr(),
                );
                if code < 0 {
                    return Err(format!("set {k}: {}", player.error(code)));
                }
            }
            if let Some(output) = output {
                let code = set(
                    handle,
                    c"ao".as_ptr(),
                    CString::new(output).map_err(|e| e.to_string())?.as_ptr(),
                );
                if code < 0 {
                    return Err(format!("set ao: {}", player.error(code)));
                }
            }
            let code = init(handle);
            if code < 0 {
                return Err(format!("mpv_initialize: {}", player.error(code)));
            }
            Ok(player)
        }
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
        let strings: Vec<_> = args
            .iter()
            .map(|s| CString::new(*s))
            .collect::<Result<_, _>>()
            .map_err(|e| format!("mpv command: {e}"))?;
        let mut ptrs: Vec<_> = strings.iter().map(|s| s.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let code = unsafe { (self.command)(self.handle, ptrs.as_ptr()) };
        if code < 0 {
            return Err(self.error(code));
        }
        if args.first() == Some(&"loadfile") {
            let mut id = 0i64;
            let code = unsafe {
                (self.get)(
                    self.handle,
                    c"playlist/0/id".as_ptr(),
                    4,
                    (&mut id as *mut i64).cast(),
                )
            };
            self.active_entry.set((code >= 0).then_some(id));
            self.waiting_for_entry.set(code < 0);
        }
        Ok(())
    }
    pub fn drain(&self) -> Vec<PlaybackEvent> {
        let mut events: Vec<_> = self
            .command_errors
            .borrow_mut()
            .drain(..)
            .map(|e| PlaybackEvent::Failed(tg_player_application::AppError::Player(e)))
            .collect();
        loop {
            let event = unsafe { (self.wait)(self.handle, 0.) };
            if event.is_null() || unsafe { (*event).event_id } == 0 {
                break;
            }
            let event = unsafe { &*event };
            // MPV_EVENT_END_FILE = 7; EOF = 0, STOP = 2, ERROR = 4.
            if event.event_id == 7 && !event.data.is_null() {
                let end = unsafe { &*event.data.cast::<MpvEndFile>() };
                if Some(end.playlist_entry_id) != self.active_entry.get() && end.reason != 2 {
                    continue;
                }
                if Some(end.playlist_entry_id) == self.active_entry.get() {
                    self.active_entry.set(None);
                }
                match end.reason {
                    0 => events.push(PlaybackEvent::Finished),
                    2 => events.push(PlaybackEvent::Stopped),
                    4 => events.push(PlaybackEvent::Failed(
                        tg_player_application::AppError::Player(self.error(end.error)),
                    )),
                    _ => {}
                }
            } else if event.event_id == 6 && !event.data.is_null() {
                let entry =
                    unsafe { (*event.data.cast::<abi::mpv_event_start_file>()).playlist_entry_id };
                if self.waiting_for_entry.replace(false) {
                    self.active_entry.set(Some(entry));
                }
                self.started_entry.set(Some(entry));
            } else if event.event_id == 8 && self.started_entry.get() == self.active_entry.get() {
                events.push(PlaybackEvent::Loaded);
            } else if event.event_id == abi::mpv_event_id_MPV_EVENT_PLAYBACK_RESTART
                && self.started_entry.get() == self.active_entry.get()
            {
                self.ready_entry.set(self.active_entry.get());
            } else if event.event_id == 5 && event.error < 0 {
                events.push(PlaybackEvent::Failed(
                    tg_player_application::AppError::Player(self.error(event.error)),
                ));
            }
        }
        events
    }
    fn error(&self, code: i32) -> String {
        unsafe { CStr::from_ptr((self.error_string)(code)) }
            .to_string_lossy()
            .into_owned()
    }
    fn flag(&self, name: &str) -> bool {
        let mut value = 0i32;
        unsafe {
            (self.get)(
                self.handle,
                CString::new(name).unwrap().as_ptr(),
                3,
                (&mut value as *mut i32).cast(),
            );
        }
        value != 0
    }
    pub fn number(&self, name: &str) -> f64 {
        self.number_if_available(name).unwrap_or_default()
    }
    fn number_if_available(&self, name: &str) -> Option<f64> {
        let mut n = 0f64;
        let code = unsafe {
            (self.get)(
                self.handle,
                CString::new(name).unwrap().as_ptr(),
                5,
                &mut n as *mut _ as *mut c_void,
            )
        };
        (code >= 0 && n.is_finite()).then_some(n)
    }
}
impl Drop for NativePlayer {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.destroy)(self.handle) }
        }
    }
}

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, Thread},
    time::Duration,
};
unsafe extern "C" fn wake_owner(data: *mut c_void) {
    // The owner retains the Thread through destruction of the mpv handle.
    unsafe { &*data.cast::<Thread>() }.unpark();
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
                let mut owner = Box::new(thread::current());
                let native = match NativePlayer::with_audio_output(output.as_deref()) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = ready.send(Err(e));
                        return;
                    }
                };
                unsafe {
                    (native.set_wakeup)(
                        native.handle,
                        Some(wake_owner),
                        (&mut *owner as *mut Thread).cast(),
                    )
                };
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
                unsafe { (native.set_wakeup)(native.handle, None, std::ptr::null_mut()) };
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
    ) -> Result<(), tg_player_application::AppError> {
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
        .map_err(tg_player_application::AppError::Player)
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
    fn shutdown(&mut self, timeout: Duration) -> Result<(), tg_player_application::AppError> {
        self.shutdown(timeout)
            .map_err(tg_player_application::AppError::Player)
    }
}
#[cfg(test)]
#[path = "audio/tests.rs"]
mod tests;
