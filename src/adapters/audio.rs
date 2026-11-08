use libloading::Library;
use std::ffi::{c_char, c_void, CStr, CString};
type Command = unsafe extern "C" fn(*mut c_void, u64, *const *const c_char) -> i32;
#[repr(C)]
struct MpvEvent {
    id: i32,
    error: i32,
    userdata: u64,
    data: *mut c_void,
}
// The ABI-stable prefix of mpv_event_end_file.
#[repr(C)]
struct MpvEndFile {
    reason: i32,
    error: i32,
}
pub use crate::application::ports::PlaybackEvent;
pub struct Player {
    _lib: Library,
    handle: *mut c_void,
    command: Command,
    command_errors: std::cell::RefCell<Vec<String>>,
    destroy: unsafe extern "C" fn(*mut c_void),
    wait: unsafe extern "C" fn(*mut c_void, f64) -> *const MpvEvent,
    error_string: unsafe extern "C" fn(i32) -> *const c_char,
    get: unsafe extern "C" fn(*mut c_void, *const c_char, i32, *mut c_void) -> i32,
}
impl Player {
    pub fn new() -> Result<Self, String> {
        Self::with_audio_output(None)
    }
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
                command_errors: Default::default(),
                command: *lib.get(b"mpv_command_async\0").map_err(|e| e.to_string())?,
                wait: *lib.get(b"mpv_wait_event\0").map_err(|e| e.to_string())?,
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
    pub fn cmd(&self, args: &[&str]) {
        if let Err(error) = self.try_cmd(args) {
            self.command_errors.borrow_mut().push(error);
        }
    }
    pub fn try_cmd(&self, args: &[&str]) -> Result<(), String> {
        // Completed events for the previous file cannot advance a newly chosen
        // track. Manual replacement generates END_FILE with reason STOP.
        if args.first() == Some(&"loadfile") {
            let _ = self.drain();
        }
        let strings: Vec<_> = args
            .iter()
            .map(|s| CString::new(*s))
            .collect::<Result<_, _>>()
            .map_err(|e| format!("mpv command: {e}"))?;
        let mut ptrs: Vec<_> = strings.iter().map(|s| s.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let code = unsafe { (self.command)(self.handle, 0, ptrs.as_ptr()) };
        if code < 0 {
            return Err(self.error(code));
        }
        Ok(())
    }
    pub fn drain(&self) -> Vec<PlaybackEvent> {
        let mut events: Vec<_> = self
            .command_errors
            .borrow_mut()
            .drain(..)
            .map(PlaybackEvent::Failed)
            .collect();
        for _ in 0..64 {
            let event = unsafe { (self.wait)(self.handle, 0.) };
            if event.is_null() || unsafe { (*event).id } == 0 {
                break;
            }
            let event = unsafe { &*event };
            // MPV_EVENT_END_FILE = 7; EOF = 0, STOP = 2, ERROR = 4.
            if event.id == 7 && !event.data.is_null() {
                let end = unsafe { &*event.data.cast::<MpvEndFile>() };
                match end.reason {
                    0 => events.push(PlaybackEvent::Finished),
                    2 => events.push(PlaybackEvent::Stopped),
                    4 => events.push(PlaybackEvent::Failed(self.error(end.error))),
                    _ => {}
                }
            } else if event.id == 8 {
                events.push(PlaybackEvent::Loaded);
            } else if event.id == 5 && event.error < 0 {
                events.push(PlaybackEvent::Failed(self.error(event.error)));
            }
        }
        events
    }
    fn error(&self, code: i32) -> String {
        unsafe { CStr::from_ptr((self.error_string)(code)) }
            .to_string_lossy()
            .into_owned()
    }
    pub fn number(&self, name: &str) -> f64 {
        let mut n = 0f64;
        unsafe {
            (self.get)(
                self.handle,
                CString::new(name).unwrap().as_ptr(),
                5,
                &mut n as *mut _ as *mut c_void,
            );
        };
        n
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.destroy)(self.handle) }
        }
    }
}
#[cfg(test)]
#[path = "audio/tests.rs"]
mod tests;

use crate::application::ports::PlayerBackend;
impl PlayerBackend for Player {
    fn cmd(&self, args: &[&str]) {
        self.cmd(args);
    }
    fn drain(&self) -> Vec<PlaybackEvent> {
        self.drain()
    }
    fn number(&self, name: &str) -> f64 {
        self.number(name)
    }
}
