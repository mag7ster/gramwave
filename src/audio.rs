use crate::telegram::Telegram;
use libloading::Library;
use serde_json::json;
use std::{
    ffi::{c_char, c_void, CStr, CString},
    io::{Read, Seek, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
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
#[derive(Debug, PartialEq)]
pub enum PlaybackEvent {
    Finished,
    Stopped,
    Failed(String),
}
pub struct Player {
    _lib: Library,
    handle: *mut c_void,
    command: Command,
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
                *lib.get(b"mpv_initialize\0").unwrap();
            let set: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i32 =
                *lib.get(b"mpv_set_option_string\0").unwrap();
            let handle = create();
            if handle.is_null() {
                return Err("mpv_create failed".into());
            }
            for (k, v) in [
                ("video", "no"),
                ("terminal", "no"),
                ("audio-display", "no"),
                ("idle", "yes"),
            ] {
                set(
                    handle,
                    CString::new(k).unwrap().as_ptr(),
                    CString::new(v).unwrap().as_ptr(),
                );
            }
            if let Some(output) = output {
                set(
                    handle,
                    c"ao".as_ptr(),
                    CString::new(output).unwrap().as_ptr(),
                );
            }
            if init(handle) < 0 {
                return Err("mpv_initialize failed".into());
            }
            Ok(Self {
                handle,
                command: *lib.get(b"mpv_command_async\0").unwrap(),
                wait: *lib.get(b"mpv_wait_event\0").unwrap(),
                error_string: *lib.get(b"mpv_error_string\0").unwrap(),
                destroy: *lib.get(b"mpv_terminate_destroy\0").unwrap(),
                get: *lib.get(b"mpv_get_property\0").unwrap(),
                _lib: lib,
            })
        }
    }
    pub fn cmd(&self, args: &[&str]) {
        // Completed events for the previous file cannot advance a newly chosen
        // track. Manual replacement generates END_FILE with reason STOP.
        if args.first() == Some(&"loadfile") {
            let _ = self.drain();
        }
        let strings: Vec<_> = args.iter().filter_map(|s| CString::new(*s).ok()).collect();
        let mut ptrs: Vec<_> = strings.iter().map(|s| s.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        unsafe {
            (self.command)(self.handle, 0, ptrs.as_ptr());
        }
    }
    pub fn drain(&self) -> Vec<PlaybackEvent> {
        let mut events = Vec::new();
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
        unsafe { (self.destroy)(self.handle) }
    }
}
// A loopback-only range endpoint lets mpv own decoding, buffering and seeking.
pub fn serve(tg: Telegram) -> Result<(String, Arc<AtomicBool>), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().unwrap().port();
    let mut entropy = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut entropy))
        .map_err(|e| e.to_string())?;
    let token = entropy
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let base = format!("http://127.0.0.1:{port}/{token}");
    let alive = Arc::new(AtomicBool::new(true));
    let a = alive.clone();
    listener.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        while a.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((s, _)) => {
                    let t = tg.clone();
                    let stop = a.clone();
                    let token = token.clone();
                    std::thread::spawn(move || {
                        let _ = respond(s, t, stop, &token);
                    });
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    });
    Ok((base, alive))
}
fn respond(
    mut socket: TcpStream,
    tg: Telegram,
    alive: Arc<AtomicBool>,
    token: &str,
) -> Result<(), String> {
    socket.set_read_timeout(Some(Duration::from_secs(5))).ok();
    socket.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let mut data = Vec::new();
    let mut byte = [0];
    while data.len() < 8192 {
        if socket.read(&mut byte).map_err(|e| e.to_string())? == 0 {
            return Ok(());
        }
        data.push(byte[0]);
        if data.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let request = String::from_utf8_lossy(&data);
    let first = request.lines().next().unwrap_or("");
    let path = first.split_whitespace().nth(1).unwrap_or("");
    let Some(file) = path.strip_prefix(&format!("/{token}/")) else {
        socket
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .ok();
        return Ok(());
    };
    let id: i64 = file.parse().map_err(|_| "invalid file")?;
    let info = tg.call(json!({"@type":"getFile","file_id":id}))?;
    let size = info["size"].as_u64().unwrap_or(0);
    if size == 0 {
        return Err("Unknown file size".into());
    }
    let range = request.lines().find_map(|l| {
        l.to_ascii_lowercase()
            .strip_prefix("range: bytes=")
            .map(str::to_owned)
    });
    let (start, end) = match parse_range(range.as_deref(), size) {
        Ok(r) => r,
        Err(_) => {
            let response=format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{size}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            socket.write_all(response.as_bytes()).ok();
            return Ok(());
        }
    };
    if start > end {
        socket
            .write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\nConnection: close\r\n\r\n")
            .ok();
        return Ok(());
    }
    let status = if range.is_some() {
        "206 Partial Content"
    } else {
        "200 OK"
    };
    let header=format!("HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{size}\r\nConnection: close\r\n\r\n",end-start+1);

    socket
        .write_all(header.as_bytes())
        .map_err(|e| e.to_string())?;
    if first.starts_with("HEAD ") {
        return Ok(());
    }
    tg.call(json!({"@type":"downloadFile","file_id":id,"priority":32,"offset":start,"limit":0,"synchronous":false}))?;
    let mut pos = start;
    let mut idle = 0;
    while pos <= end && alive.load(Ordering::Relaxed) {
        let prefix = tg
            .call(json!({"@type":"getFileDownloadedPrefixSize","file_id":id,"offset":pos}))?
            ["size"]
            .as_u64()
            .unwrap_or(0);
        if prefix == 0 {
            idle += 1;
            if idle % 20 == 0 {
                tg.call(json!({"@type":"downloadFile","file_id":id,"priority":32,"offset":pos,"limit":0,"synchronous":false}))?;
            }
            if idle > 300 {
                return Err("Download stalled".into());
            }
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        idle = 0;
        let info = tg.call(json!({"@type":"getFile","file_id":id}))?;
        let path = info["local"]["path"].as_str().ok_or("No local file")?;
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        file.seek(std::io::SeekFrom::Start(pos))
            .map_err(|e| e.to_string())?;
        let mut buf = vec![0; prefix.min(65536).min(end - pos + 1) as usize];
        file.read_exact(&mut buf).map_err(|e| e.to_string())?;
        socket.write_all(&buf).map_err(|e| e.to_string())?;
        pos += buf.len() as u64;
    }
    Ok(())
}

fn parse_range(range: Option<&str>, size: u64) -> Result<(u64, u64), &'static str> {
    if size == 0 {
        return Err("Empty resource");
    }
    let Some(range) = range else {
        return Ok((0, size - 1));
    };
    let (a, b) = range.trim().split_once('-').ok_or("Invalid range")?;
    let (start, end) = if a.is_empty() {
        let length = b.parse::<u64>().map_err(|_| "Invalid suffix")?;
        if length == 0 {
            return Err("Empty suffix");
        }
        (size.saturating_sub(length), size - 1)
    } else {
        (
            a.parse::<u64>().map_err(|_| "Invalid start")?,
            if b.is_empty() {
                size - 1
            } else {
                b.parse::<u64>().map_err(|_| "Invalid end")?.min(size - 1)
            },
        )
    };
    if start > end || start >= size {
        return Err("Unsatisfiable range");
    }
    Ok((start, end))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_ranges() {
        assert_eq!(parse_range(None, 100), Ok((0, 99)));
        assert_eq!(parse_range(Some("30-"), 100), Ok((30, 99)));
        assert_eq!(parse_range(Some("-20"), 100), Ok((80, 99)));
        assert_eq!(parse_range(Some("20-300"), 100), Ok((20, 99)));
        for r in ["100-", "9-3", "0-1,4-5", "-0", "a-"] {
            assert!(parse_range(Some(r), 100).is_err());
        }
    }
}

#[cfg(test)]
mod streaming_tests {
    use super::*;
    #[test]
    fn cached_range_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tg-player-range-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sample");
        std::fs::write(&file, b"0123456789abcdef").unwrap();
        let (base, alive) = serve(Telegram::mock_file(file)).unwrap();
        let authority = base.strip_prefix("http://").unwrap();
        let (host, path) = authority.split_once('/').unwrap();
        let mut socket = TcpStream::connect(host).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(
            socket,
            "GET /{path}/1 HTTP/1.1\r\nHost: {host}\r\nRange: bytes=4-8\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 206"));
        assert!(response.ends_with("45678"));
        let mut socket = TcpStream::connect(host).unwrap();
        write!(socket, "GET /wrong/1 HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 404"));
        alive.store(false, Ordering::Relaxed);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod playback_tests {
    use super::*;
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
        while player.number("duration") < 2. && std::time::Instant::now() < deadline {
            for event in player.drain() {
                assert!(!matches!(
                    event,
                    PlaybackEvent::Finished | PlaybackEvent::Failed(_)
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(player.number("duration") > 2.);
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
