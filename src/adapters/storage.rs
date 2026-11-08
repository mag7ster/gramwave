use crate::model::{Playlist, State};
use std::{
    collections::VecDeque,
    path::Path,
    sync::{mpsc, Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::Duration,
};
use tg_player_application::persistence::ChangeSet;
type Outcome = Result<(), String>;
enum Request {
    Changes(Box<ChangeSet>),
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
    offline: tg_player_application::offline::OfflineData,
}

fn document(db: &rusqlite::Connection, key: &str, value: &impl serde::Serialize) -> Outcome {
    let value = serde_json::to_string(value).map_err(|e| format!("serialize {key}: {e}"))?;
    db.execute("INSERT INTO documents(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE value != excluded.value", [key, &value]).map_err(|e| format!("write {key}: {e}"))?;
    Ok(())
}
fn write_changes(db: &mut rusqlite::Connection, changes: &ChangeSet) -> Outcome {
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| format!("begin save: {e}"))?;
    write_changes_in(&tx, changes)?;
    tx.commit().map_err(|e| format!("commit save: {e}"))
}
fn write_changes_in(tx: &rusqlite::Transaction<'_>, changes: &ChangeSet) -> Outcome {
    if let Some(value) = &changes.offline {
        write_offline(tx, value)?;
    }
    if let Some(value) = &changes.settings {
        document(tx, "settings", value)?;
    }
    if let Some(value) = &changes.session {
        document(tx, "session_v3", value)?;
    }
    if let Some(value) = &changes.library_order {
        document(tx, "library_order", value)?;
    }
    for header in &changes.headers {
        let value = serde_json::to_string(header).map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO library(id,value) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET value=excluded.value WHERE value != excluded.value", rusqlite::params![header.id,value]).map_err(|e| format!("write library: {e}"))?;
    }
    for track in &changes.tracks {
        let value = serde_json::to_string(&track.track).map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO track_records(playlist,entry_id,chat,message,value) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(playlist,entry_id) DO UPDATE SET chat=excluded.chat,message=excluded.message,value=excluded.value WHERE value != excluded.value",rusqlite::params![track.playlist,track.track.library_id as i64,track.track.chat,track.track.message,value]).map_err(|e|format!("write track: {e}"))?;
    }
    for (id, order) in &changes.track_order {
        for (position, entry_id) in order.iter().enumerate() {
            tx.execute("INSERT INTO track_order(playlist,position,entry_id) VALUES(?1,?2,?3) ON CONFLICT(playlist,position) DO UPDATE SET entry_id=excluded.entry_id WHERE entry_id!=excluded.entry_id",rusqlite::params![id,position as i64,*entry_id as i64]).map_err(|e|e.to_string())?;
        }
        tx.execute(
            "DELETE FROM track_order WHERE playlist=?1 AND position>=?2",
            rusqlite::params![id, order.len() as i64],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM track_records WHERE playlist=?1 AND entry_id NOT IN (SELECT entry_id FROM track_order WHERE playlist=?1)",[id]).map_err(|e|e.to_string())?;
    }
    if changes.library_order.is_some() {
        tx.execute("DELETE FROM library WHERE id NOT IN (SELECT value FROM json_each((SELECT value FROM documents WHERE key='library_order')))",[]).map_err(|e|e.to_string())?;
        tx.execute(
            "DELETE FROM track_order WHERE playlist NOT IN (SELECT id FROM library)",
            [],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM track_records WHERE playlist NOT IN (SELECT id FROM library)",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
/// SQLite's online backup includes committed WAL pages. Publication cannot overwrite a prior backup.
fn backup(db: &rusqlite::Connection, path: &Path, target_version: u32) -> Outcome {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| format!("backup: {e}"))?;
    {
        let mut destination =
            rusqlite::Connection::open(temp.path()).map_err(|e| format!("backup: {e}"))?;
        rusqlite::backup::Backup::new(db, &mut destination)
            .map_err(|e| format!("backup: {e}"))?
            .run_to_completion(128, Duration::from_millis(5), None)
            .map_err(|e| format!("backup: {e}"))?;
        let result: String = destination
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|e| format!("backup verification: {e}"))?;
        if result != "ok" {
            return Err(format!("backup verification: {result}"));
        }
    }
    temp.as_file()
        .sync_all()
        .map_err(|e| format!("backup sync: {e}"))?;
    let name = path
        .file_name()
        .ok_or("backup: missing filename")?
        .to_string_lossy();
    let mut index = 0u64;
    let mut temp = temp;
    loop {
        let target = parent.join(format!("{name}.pre-v{target_version}-{index}.sqlite"));
        match temp.persist_noclobber(target) {
            Ok(_) => return Ok(()),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                temp = error.file;
                index += 1;
            }
            Err(error) => return Err(format!("backup publication: {}", error.error)),
        }
    }
}
fn write_offline(
    db: &rusqlite::Connection,
    delta: &tg_player_application::offline::OfflineDelta,
) -> Outcome {
    fn records<T: serde::Serialize>(
        db: &rusqlite::Connection,
        table: &str,
        values: impl Iterator<Item = (String, Option<T>)>,
    ) -> Outcome {
        for (id, value) in values {
            if let Some(value) = value {
                let value = serde_json::to_string(&value).map_err(|e| e.to_string())?;
                db.execute(&format!("INSERT INTO {table}(id,value) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET value=excluded.value WHERE value!=excluded.value"), [&id, &value]).map_err(|e| e.to_string())?;
            } else {
                db.execute(&format!("DELETE FROM {table} WHERE id=?1"), [&id])
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    let key = |id: &crate::domain::TrackId| format!("{}:{}", id.chat, id.message);
    records(
        db,
        "offline_copies",
        delta
            .copies
            .iter()
            .map(|(id, value)| (key(id), value.as_ref())),
    )?;
    // Pending upserts represent enqueue events. Requeueing a removed job must
    // append it after existing work, even when changes coalesced before a flush.
    for (id, value) in &delta.pending {
        let id = key(id);
        db.execute("DELETE FROM offline_jobs WHERE id=?1", [&id])
            .map_err(|e| e.to_string())?;
        if let Some(track) = value {
            let value = serde_json::to_string(track).map_err(|e| e.to_string())?;
            db.execute(
                "INSERT INTO offline_jobs(id,value) VALUES(?1,?2)",
                [&id, &value],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    records(
        db,
        "offline_removal_jobs",
        delta
            .removing
            .iter()
            .map(|(path, value)| (path.clone(), value.as_ref())),
    )?;
    records(
        db,
        "offline_policy_records",
        delta
            .policies
            .iter()
            .map(|(chat, value)| (chat.to_string(), value.as_ref())),
    )?;
    records(
        db,
        "offline_exclusion_records",
        delta
            .excluded
            .iter()
            .map(|(id, excluded)| (key(id), excluded.then_some(id))),
    )
}
fn read_offline(
    db: &rusqlite::Connection,
    version: u32,
) -> Result<tg_player_application::offline::OfflineData, String> {
    fn rows<T: serde::de::DeserializeOwned>(
        db: &rusqlite::Connection,
        table: &str,
    ) -> Result<Vec<T>, String> {
        let mut query = db
            .prepare(&format!("SELECT value FROM {table} ORDER BY rowid"))
            .map_err(|e| e.to_string())?;
        let rows = query
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|row| {
            serde_json::from_str(&row.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
        })
        .collect()
    }
    Ok(tg_player_application::offline::OfflineData {
        copies: rows(db, "offline_copies")?,
        removing: rows(db, "offline_removal_jobs")?,
        pending: rows(db, "offline_jobs")?,
        policies: if version >= 4 {
            rows(db, "offline_policy_records")?
        } else {
            serde_json::from_str(&read_document(db, "offline_policies")?)
                .map_err(|e| e.to_string())?
        },
        excluded: if version >= 4 {
            rows(db, "offline_exclusion_records")?
        } else {
            serde_json::from_str(&read_document(db, "offline_exclusions")?)
                .map_err(|e| e.to_string())?
        },
    })
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
        if version > 4 {
            return Err(format!("open: unsupported database schema {version}"));
        }
        let check: String = db
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(|e| format!("check database: {e}"))?;
        if check != "ok" {
            return Err(format!("check database: {check}"));
        }
        let existing: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if existing && version < 4 {
            backup(&db, path, if version < 3 { 3 } else { 4 })?;
        }
        db.busy_timeout(Duration::from_secs(2))
            .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA journal_mode=WAL")
            .map_err(|e| e.to_string())?;
        // Hold schema changes and imported records in the same transaction.
        // On any decoding/write failure, dropping this transaction rolls back
        // the entire attempted upgrade before the connection is released.
        let (state, offline) = {
            let migration = if version < 4 {
                Some(
                    db.unchecked_transaction()
                        .map_err(|e| format!("begin migration: {e}"))?,
                )
            } else {
                None
            };
            if migration.is_some() {
                db.execute_batch("CREATE TABLE IF NOT EXISTS state(id INTEGER PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS documents(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS library(id INTEGER PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS tracks(playlist INTEGER NOT NULL, position INTEGER NOT NULL, chat INTEGER NOT NULL, message INTEGER NOT NULL, value TEXT NOT NULL, PRIMARY KEY(playlist,position)); CREATE INDEX IF NOT EXISTS track_identity ON tracks(chat,message); CREATE TABLE IF NOT EXISTS offline_copies(id TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS offline_jobs(id TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS offline_removal_jobs(id TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS offline_policy_records(id TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS offline_exclusion_records(id TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS track_records(playlist INTEGER NOT NULL,entry_id INTEGER NOT NULL,chat INTEGER NOT NULL,message INTEGER NOT NULL,value TEXT NOT NULL,PRIMARY KEY(playlist,entry_id)); CREATE TABLE IF NOT EXISTS track_order(playlist INTEGER NOT NULL,position INTEGER NOT NULL,entry_id INTEGER NOT NULL,PRIMARY KEY(playlist,position));").map_err(|e| e.to_string())?;
            }
            let migrated: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM documents WHERE key='settings')",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if version > 0 && !migrated {
                return Err("open: settings record is missing".into());
            }
            let mut state = if migrated {
                let mut state: State = serde_json::from_str(&read_document(&db, "settings")?)
                    .map_err(|e| e.to_string())?;
                state.session = serde_json::from_str(&read_document(
                    &db,
                    if version < 3 {
                        "session_v2"
                    } else {
                        "session_v3"
                    },
                )?)
                .map_err(|e| e.to_string())?;
                let order: Vec<i64> = serde_json::from_str(&read_document(&db, "library_order")?)
                    .map_err(|e| e.to_string())?;
                for id in order {
                    let s: String = db
                        .query_row("SELECT value FROM library WHERE id=?1", [id], |r| r.get(0))
                        .map_err(|e| e.to_string())?;
                    let mut playlist: Playlist =
                        serde_json::from_str(&s).map_err(|e| e.to_string())?;
                    let mut statement = db
                    .prepare(if version<3 {"SELECT value FROM tracks WHERE playlist=?1 ORDER BY position"}else{"SELECT r.value FROM track_order o JOIN track_records r ON r.playlist=o.playlist AND r.entry_id=o.entry_id WHERE o.playlist=?1 ORDER BY o.position"})
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
                    if version >= 3 {
                        let expected: usize = db
                            .query_row(
                                "SELECT count(*) FROM track_order WHERE playlist=?1",
                                [id],
                                |r| r.get(0),
                            )
                            .map_err(|e| e.to_string())?;
                        if expected != playlist.tracks.len() {
                            return Err("open: missing library records".into());
                        }
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
                let imported: tg_player_application::legacy::LegacyImport = match &original {
                    Some(s) => serde_json::from_str(s)
                        .map_err(|e| format!("decode legacy library: {e}"))?,
                    None => Default::default(),
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
                imported.finish()
            };
            for p in &mut state.playlists {
                p.ensure_entry_ids();
            }
            let offline = if version >= 3 {
                read_offline(&db, version)?
            } else {
                let mut offline =
                    tg_player_application::offline::OfflineCore::new(Default::default());
                if let Ok(value) = read_document(&db, "offline") {
                    let tracks: Vec<crate::model::Track> = serde_json::from_str(&value)
                        .map_err(|e| format!("read legacy downloads: {e}"))?;
                    for track in tracks {
                        offline.enqueue(track, true);
                    }
                }
                fn collect(
                    value: &serde_json::Value,
                    offline: &mut tg_player_application::offline::OfflineCore,
                ) {
                    match value {
                        serde_json::Value::Array(values) => {
                            for value in values {
                                collect(value, offline);
                            }
                        }
                        serde_json::Value::Object(values) => {
                            if let Some(tracks) = value["pending_pins"].as_array() {
                                for track in tracks {
                                    if let Ok(track) = serde_json::from_value(track.clone()) {
                                        offline.enqueue(track, true);
                                    }
                                }
                            }

                            if let Some(path) = value["pinned"].as_str() {
                                if value["chat"].is_i64() && value["message"].is_i64() {
                                    if let Ok(track) =
                                        serde_json::from_value::<crate::model::Track>(value.clone())
                                    {
                                        offline.publish(track, path.into());
                                    }
                                }
                            }
                            for value in values.values() {
                                collect(value, offline);
                            }
                        }
                        _ => {}
                    }
                }
                for key in ["offline", "session_v2", "legacy_queue"] {
                    if let Ok(value) = read_document(&db, key) {
                        let value = serde_json::from_str(&value)
                            .map_err(|e| format!("decode {key}: {e}"))?;
                        collect(&value, &mut offline);
                    }
                }
                // Old split libraries stored offline paths inside raw track JSON.
                // Read them before those records become pure metadata: serde intentionally
                // ignores the retired pinned/file/artwork presentation fields.
                for table in ["library", "tracks"] {
                    let mut query = db
                        .prepare(&format!("SELECT value FROM {table} ORDER BY rowid"))
                        .map_err(|e| format!("read legacy {table}: {e}"))?;
                    let rows = query
                        .query_map([], |row| row.get::<_, String>(0))
                        .map_err(|e| format!("read legacy {table}: {e}"))?;
                    for row in rows {
                        let value = serde_json::from_str(&row.map_err(|e| e.to_string())?)
                            .map_err(|e| format!("decode legacy {table}: {e}"))?;
                        collect(&value, &mut offline);
                    }
                }
                if let Ok(value) = db.query_row("SELECT value FROM state WHERE id=1", [], |r| {
                    r.get::<_, String>(0)
                }) {
                    let value = serde_json::from_str(&value)
                        .map_err(|e| format!("decode legacy state: {e}"))?;
                    collect(&value, &mut offline);
                }

                offline.data()
            };
            if let Some(tx) = migration {
                let mut changes = if version < 3 {
                    ChangeSet::between(None, &state)
                } else {
                    ChangeSet::default()
                };
                changes.offline = Some(offline.clone().into());
                if version < 3 {
                    tx.execute_batch("CREATE TABLE IF NOT EXISTS legacy_documents(key TEXT PRIMARY KEY,value TEXT NOT NULL); INSERT OR IGNORE INTO legacy_documents SELECT key,value FROM documents;").map_err(|e|e.to_string())?;
                }
                write_changes_in(&tx, &changes)?;
                tx.execute_batch("PRAGMA user_version=4")
                    .map_err(|e| e.to_string())?;
                tx.commit().map_err(|e| format!("commit migration: {e}"))?;
            }
            (state, offline)
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
                    Request::Changes(changes) => write_changes(&mut db, &changes),
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
                offline,
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
        if let Request::Changes(newer) = request {
            if let Some(Request::Changes(pending)) = q.requests.back_mut() {
                pending.merge(*newer);
                return Ok(());
            }
            q.requests.push_back(Request::Changes(newer));
            self.shared.wake.notify_one();
            return Ok(());
        }
        if matches!(request, Request::Stop(_)) {
            q.closed = true;
        }
        q.requests.push_back(request);
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn offline(&self) -> tg_player_application::offline::OfflineData {
        self.offline.clone()
    }
    pub fn apply(&self, changes: ChangeSet) -> Outcome {
        self.enqueue(Request::Changes(Box::new(changes)))
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
        self.shutdown_timeout(Duration::from_secs(5))
    }
    pub fn shutdown_timeout(&mut self, timeout: Duration) -> Outcome {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        let deadline = std::time::Instant::now() + timeout;
        let (tx, rx) = mpsc::channel();
        self.enqueue(Request::Stop(tx))?;
        let result = rx
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .map_err(|e| format!("shutdown store: {e}"))?;
        while !worker.is_finished() {
            if std::time::Instant::now() >= deadline {
                return Err("shutdown store timed out".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        worker
            .join()
            .map_err(|_| "store worker panicked".to_owned())?;
        result
    }
    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        if self.worker.take().is_some() {
            let (tx, _) = mpsc::channel();
            let _ = self.enqueue(Request::Stop(tx));
        }
    }
}

use crate::application::ports::Repository;
impl Repository for Store {
    fn apply(&self, changes: ChangeSet) -> Result<(), tg_player_application::AppError> {
        self.apply(changes)
            .map_err(tg_player_application::AppError::Storage)
    }
    fn flush(&self, timeout: Duration) -> Result<(), tg_player_application::AppError> {
        self.flush_timeout(timeout)
            .map_err(tg_player_application::AppError::Storage)
    }
    fn shutdown(&mut self, timeout: Duration) -> Result<(), tg_player_application::AppError> {
        self.shutdown_timeout(timeout)
            .map_err(tg_player_application::AppError::Storage)
    }
    fn error(&self) -> Option<tg_player_application::AppError> {
        self.error().map(tg_player_application::AppError::Storage)
    }
}
