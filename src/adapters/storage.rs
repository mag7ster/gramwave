use crate::model::{Playlist, State};
use std::{
    collections::VecDeque,
    path::Path,
    sync::{mpsc, Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::Duration,
};
type Outcome = Result<(), String>;
enum Request {
    Save(Box<State>),
    Volume(f32, f32),
    Flush(mpsc::Sender<Outcome>),
    Stop(mpsc::Sender<Outcome>),
}
#[derive(Default)]
struct Queue {
    requests: VecDeque<Request>,
    closed: bool,
}
struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    error: Mutex<Option<String>>,
}
pub struct Store {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

fn document(db: &rusqlite::Connection, key: &str, value: &impl serde::Serialize) -> Outcome {
    let value = serde_json::to_string(value).map_err(|e| format!("serialize {key}: {e}"))?;
    db.execute("INSERT INTO documents(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE value != excluded.value", [key, &value]).map_err(|e| format!("write {key}: {e}"))?;
    Ok(())
}
fn write_state(db: &mut rusqlite::Connection, state: &State) -> Outcome {
    let tx = db.transaction().map_err(|e| format!("begin save: {e}"))?;
    // Keep library records out of settings and the versioned session document.
    let mut value = serde_json::to_value(state).map_err(|e| e.to_string())?;
    value.as_object_mut().unwrap().remove("playlists");
    value.as_object_mut().unwrap().remove("session");
    value.as_object_mut().unwrap().remove("pending_pins");
    for key in ["queue", "queue_version", "previous_queue"] {
        value.as_object_mut().unwrap().remove(key);
    }
    document(
        &tx,
        "legacy_queue",
        &serde_json::json!({"queue":state.queue,"queue_version":state.queue_version,"previous_queue":state.previous_queue}),
    )?;
    document(&tx, "settings", &value)?;
    document(&tx, "session_v2", &state.session)?;
    document(&tx, "offline", &state.pending_pins)?;
    document(
        &tx,
        "library_order",
        &state.playlists.iter().map(|p| p.id).collect::<Vec<_>>(),
    )?;
    for p in &state.playlists {
        let header = serde_json::json!({"id":p.id,"title":p.title,"url":p.url,"filters":p.filters,"pinned":p.pinned,"cursors":p.cursors,"done":p.done}).to_string();
        tx.execute("INSERT INTO library(id,value) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET value=excluded.value WHERE value != excluded.value", rusqlite::params![p.id,header]).map_err(|e| format!("write library: {e}"))?;
        for (position, track) in p.tracks.iter().enumerate() {
            let json = serde_json::to_string(track).map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO tracks(playlist,position,chat,message,value) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(playlist,position) DO UPDATE SET chat=excluded.chat,message=excluded.message,value=excluded.value WHERE value != excluded.value",rusqlite::params![p.id,position as i64,track.chat,track.message,json]).map_err(|e|format!("write track: {e}"))?;
        }
        tx.execute(
            "DELETE FROM tracks WHERE playlist=?1 AND position>=?2",
            rusqlite::params![p.id, p.tracks.len() as i64],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.execute("DELETE FROM library WHERE id NOT IN (SELECT value FROM json_each((SELECT value FROM documents WHERE key='library_order')))",[]).map_err(|e|e.to_string())?;
    tx.execute(
        "DELETE FROM tracks WHERE playlist NOT IN (SELECT id FROM library)",
        [],
    )
    .map_err(|e| e.to_string())?;
    tx.execute_batch("PRAGMA user_version=2")
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| format!("commit save: {e}"))
}
fn read_document(db: &rusqlite::Connection, key: &str) -> Result<String, String> {
    db.query_row("SELECT value FROM documents WHERE key=?1", [key], |r| {
        r.get(0)
    })
    .map_err(|e| format!("read {key}: {e}"))
}
impl Store {
    pub fn open(path: &Path) -> Result<(Self, State), String> {
        let mut db = rusqlite::Connection::open(path).map_err(|e| format!("open: {e}"))?;
        let version: u32 = db
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        if version > 2 {
            return Err(format!("open: unsupported database schema {version}"));
        }
        db.busy_timeout(Duration::from_secs(2))
            .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS state(id INTEGER PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS documents(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS library(id INTEGER PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS tracks(playlist INTEGER NOT NULL, position INTEGER NOT NULL, chat INTEGER NOT NULL, message INTEGER NOT NULL, value TEXT NOT NULL, PRIMARY KEY(playlist,position)); CREATE INDEX IF NOT EXISTS track_identity ON tracks(chat,message);").map_err(|e| e.to_string())?;
        let migrated: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM documents WHERE key='settings')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let state = if migrated {
            let mut state: State = serde_json::from_str(&read_document(&db, "settings")?)
                .map_err(|e| e.to_string())?;
            let legacy: State = serde_json::from_str(&read_document(&db, "legacy_queue")?)
                .map_err(|e| e.to_string())?;
            state.queue = legacy.queue;
            state.queue_version = legacy.queue_version;
            state.previous_queue = legacy.previous_queue;
            state.session = serde_json::from_str(&read_document(&db, "session_v2")?)
                .map_err(|e| e.to_string())?;
            state.pending_pins =
                serde_json::from_str(&read_document(&db, "offline")?).map_err(|e| e.to_string())?;
            let order: Vec<i64> = serde_json::from_str(&read_document(&db, "library_order")?)
                .map_err(|e| e.to_string())?;
            for id in order {
                let s: String = db
                    .query_row("SELECT value FROM library WHERE id=?1", [id], |r| r.get(0))
                    .map_err(|e| e.to_string())?;
                let mut playlist: Playlist = serde_json::from_str(&s).map_err(|e| e.to_string())?;
                let mut statement = db
                    .prepare("SELECT value FROM tracks WHERE playlist=?1 ORDER BY position")
                    .map_err(|e| e.to_string())?;
                let rows = statement
                    .query_map([id], |r| r.get::<_, String>(0))
                    .map_err(|e| e.to_string())?;
                for row in rows {
                    playlist.tracks.push(
                        serde_json::from_str(&row.map_err(|e| e.to_string())?)
                            .map_err(|e| e.to_string())?,
                    );
                }
                state.playlists.push(playlist);
            }
            state
        } else {
            let original = match db.query_row("SELECT value FROM state WHERE id=1", [], |r| {
                r.get::<_, String>(0)
            }) {
                Ok(s) => Some(s),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(e.to_string()),
            };
            let mut state: State = match &original {
                Some(s) => {
                    serde_json::from_str(s).map_err(|e| format!("decode legacy library: {e}"))?
                }
                None => State::default(),
            };
            // The source state row is never overwritten or deleted.
            if let Some(s) = &original {
                db.execute_batch("CREATE TABLE IF NOT EXISTS queue_backup(id INTEGER PRIMARY KEY,value TEXT NOT NULL)").map_err(|e| e.to_string())?;
                db.execute(
                    "INSERT OR IGNORE INTO queue_backup(id,value) VALUES(1,?1)",
                    [s],
                )
                .map_err(|e| e.to_string())?;
            }
            state.repair_legacy_queue();
            if state.queue_version < 2 {
                if state.previous_queue.is_empty() {
                    state.previous_queue = state.queue.clone();
                }
                state.session.import(std::mem::take(&mut state.queue));
                state.queue_version = 2;
            }
            write_state(&mut db, &state)?;
            state
        };
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            error: Mutex::new(None),
        });
        let worker_shared = shared.clone();
        let worker = std::thread::spawn(move || {
            let mut failure: Option<String> = None;
            loop {
                let request = {
                    let mut q = worker_shared.queue.lock().unwrap();
                    while q.requests.is_empty() {
                        q = worker_shared.wake.wait(q).unwrap();
                    }
                    q.requests.pop_front().unwrap()
                };
                let result = match request {
                    Request::Save(state) => write_state(&mut db, &state),
                    Request::Volume(volume, unmuted) => (|| {
                        let mut settings: serde_json::Value =
                            serde_json::from_str(&read_document(&db, "settings")?)
                                .map_err(|e| e.to_string())?;
                        settings["volume"] = serde_json::json!(volume);
                        settings["unmuted_volume"] = serde_json::json!(unmuted);
                        document(&db, "settings", &settings)
                    })(),
                    Request::Flush(reply) => {
                        let _ = reply.send(failure.clone().map_or(Ok(()), Err));
                        continue;
                    }
                    Request::Stop(reply) => {
                        let _ = reply.send(failure.map_or(Ok(()), Err));
                        break;
                    }
                };
                if let Err(e) = result {
                    failure = Some(e.clone());
                    *worker_shared.error.lock().unwrap() = Some(e);
                }
            }
        });
        Ok((
            Self {
                shared,
                worker: Some(worker),
            },
            state,
        ))
    }
    fn enqueue(&self, request: Request) -> Outcome {
        let mut q = self
            .shared
            .queue
            .lock()
            .map_err(|e| format!("save queue: {e}"))?;
        if q.closed {
            return Err("save: worker is closed".into());
        }
        // Coalesce only within the tail after the last acknowledgement barrier.
        let boundary = q
            .requests
            .iter()
            .rposition(|r| matches!(r, Request::Flush(_) | Request::Stop(_)))
            .map_or(0, |i| i + 1);
        if matches!(request, Request::Save(_) | Request::Volume(..)) {
            let mut tail = q.requests.split_off(boundary);
            tail.retain(|r| match &request {
                Request::Save(_) => !matches!(r, Request::Save(_) | Request::Volume(..)),
                Request::Volume(..) => !matches!(r, Request::Volume(..)),
                _ => true,
            });
            q.requests.append(&mut tail);
        }
        if matches!(request, Request::Stop(_)) {
            q.closed = true;
        }
        q.requests.push_back(request);
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn save(&self, state: State) -> Outcome {
        self.enqueue(Request::Save(Box::new(state)))
    }
    pub fn save_volume(&self, volume: f32, unmuted: f32) -> Outcome {
        self.enqueue(Request::Volume(volume, unmuted))
    }
    pub fn flush(&self) -> Outcome {
        self.flush_timeout(Duration::from_secs(5))
    }
    pub fn flush_timeout(&self, timeout: Duration) -> Outcome {
        let (tx, rx) = mpsc::channel();
        self.enqueue(Request::Flush(tx))?;
        rx.recv_timeout(timeout)
            .map_err(|e| format!("flush: {e}"))?
    }
    pub fn shutdown(&mut self) -> Outcome {
        if self.worker.is_none() {
            return Ok(());
        }
        let (tx, rx) = mpsc::channel();
        self.enqueue(Request::Stop(tx))?;
        let result = rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| format!("shutdown store: {e}"))?;
        self.worker
            .take()
            .unwrap()
            .join()
            .map_err(|_| "store worker panicked".to_owned())?;
        result
    }
    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().take()
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

use crate::application::ports::Repository;
impl Repository for Store {
    fn save(&self, state: State) -> Result<(), String> {
        self.save(state)
    }
    fn save_volume(&self, volume: f32, unmuted: f32) -> Result<(), String> {
        self.save_volume(volume, unmuted)
    }
    fn flush(&self) -> Result<(), String> {
        self.flush()
    }
    fn shutdown(&mut self) -> Result<(), String> {
        self.shutdown()
    }
    fn error(&self) -> Option<String> {
        self.error()
    }
}
