use crate::telegram::Telegram;
use serde_json::json;
use std::{
    io::{Read, Seek, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
pub struct Server {
    alive: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<Result<(), String>>>,
}
impl Server {
    pub fn stop(&self) {
        self.alive.store(false, Ordering::Release);
    }
    pub fn shutdown(&mut self, timeout: Duration) -> Result<(), String> {
        self.stop();
        let deadline = std::time::Instant::now() + timeout;
        if let Some(worker) = &self.worker {
            while !worker.is_finished() {
                if std::time::Instant::now() >= deadline {
                    return Err("HTTP shutdown timed out".into());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "HTTP worker panicked".to_owned())??;
        }
        Ok(())
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.shutdown(Duration::from_secs(7));
    }
}
// A loopback-only range endpoint lets mpv own decoding, buffering and seeking.
pub fn serve(tg: Telegram) -> Result<(String, Server), String> {
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
    let mut clients = crate::application::workers::Workers::new(4, 16);
    let worker = std::thread::spawn(move || {
        while a.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((s, _)) => {
                    let t = tg.clone();
                    let stop = a.clone();
                    let token = token.clone();
                    let _ = clients.spawn(move || {
                        let _ = respond(s, t, stop, &token);
                    });
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
        clients.shutdown(Duration::from_secs(6))
    });
    Ok((
        base,
        Server {
            alive,
            worker: Some(worker),
        },
    ))
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
    while data.len() < 8192 && alive.load(Ordering::Acquire) {
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
#[path = "streaming/tests.rs"]
mod tests;
