use crate::model::State;
use std::{
    path::Path,
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};
enum Request {
    Save(Box<State>),
    Flush(mpsc::Sender<()>),
}
pub struct Store {
    tx: mpsc::Sender<Request>,
    error: Arc<Mutex<Option<String>>>,
}
impl Store {
    pub fn open(path: &Path) -> Result<(Self, State), String> {
        let db = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS state(id INTEGER PRIMARY KEY,value TEXT NOT NULL);").map_err(|e|e.to_string())?;
        let state = match db.query_row("SELECT value FROM state WHERE id=1", [], |r| {
            r.get::<_, String>(0)
        }) {
            Ok(s) => {
                let state: State = serde_json::from_str(&s)
                    .map_err(|e| format!("Library index cannot be decoded: {e}"))?;
                if state.queue_version < 2 {
                    db.execute_batch("CREATE TABLE IF NOT EXISTS queue_backup(id INTEGER PRIMARY KEY,value TEXT NOT NULL);").map_err(|e| e.to_string())?;
                    db.execute(
                        "INSERT OR IGNORE INTO queue_backup(id,value) VALUES(1,?1)",
                        [&s],
                    )
                    .map_err(|e| e.to_string())?;
                }
                state
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => State::default(),
            Err(e) => return Err(e.to_string()),
        };
        let (tx, rx) = mpsc::channel();
        let error = Arc::new(Mutex::new(None));
        let errors = error.clone();
        std::thread::spawn(move || {
            while let Ok(request) = rx.recv() {
                match request {
                    Request::Save(state) => {
                        let result = serde_json::to_string(&state)
                            .map_err(|e| e.to_string())
                            .and_then(|s| {
                                db.execute(
                                    "INSERT OR REPLACE INTO state(id,value) VALUES(1,?1)",
                                    [s],
                                )
                                .map(|_| ())
                                .map_err(|e| e.to_string())
                            });
                        if let Err(e) = result {
                            *errors.lock().unwrap() = Some(e);
                        }
                    }
                    Request::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
        });
        Ok((Self { tx, error }, state))
    }
    pub fn save(&self, state: State) {
        let _ = self.tx.send(Request::Save(Box::new(state)));
    }
    pub fn flush(&self) {
        let (tx, rx) = mpsc::channel();
        let _ = self.tx.send(Request::Flush(tx));
        let _ = rx.recv_timeout(Duration::from_secs(5));
    }
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().take()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_roundtrip() {
        let path =
            std::env::temp_dir().join(format!("tg-player-store-{}.sqlite", std::process::id()));
        let (store, mut state) = Store::open(&path).unwrap();
        state.api_id = "12345".into();
        state.filters = [false; 3];
        state.chat_pins = vec![42];
        store.save(state);
        store.flush();
        let (_, state) = Store::open(&path).unwrap();
        assert_eq!(state.api_id, "12345");
        assert_eq!(state.filters, [false; 3]);
        assert_eq!(state.chat_pins, vec![42]);
        std::fs::remove_file(path).unwrap();
    }
}
