mod common;
use common::TempDir;
use tg_player::{
    adapters::storage::Store,
    domain::{Playlist, State, Track},
};
#[test]
fn roundtrip_and_closed_channel_are_explicit() {
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let (mut store, mut state) = Store::open(&path).unwrap();
    state.api_id = "12345".into();
    state.filters = [false; 3];
    state.chat_pins = vec![42];
    store.save(state).unwrap();
    store.flush().unwrap();
    store.shutdown().unwrap();
    assert!(store.save(State::default()).unwrap_err().contains("closed"));
    assert!(store.flush().is_err());
    let (_, state) = Store::open(&path).unwrap();
    assert_eq!(state.api_id, "12345");
    assert_eq!(state.filters, [false; 3]);
    assert_eq!(state.chat_pins, vec![42]);
}
#[test]
fn queue_jump_backwards_and_forwards_survive_sqlite_restart() {
    use tg_player::domain::RepeatMode;
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let (mut store, mut state) = Store::open(&path).unwrap();
    let tracks: Vec<_> = (1..=6)
        .map(|message| Track {
            chat: 42,
            message,
            ..Default::default()
        })
        .collect();
    state.session.start(&tracks, 1, false);
    state.session.jump_to(&mut state.queue, 2);
    store.save(state).unwrap();
    store.flush().unwrap();
    store.shutdown().unwrap();
    let (mut store, mut state) = Store::open(&path).unwrap();
    assert_eq!(state.session.current.as_ref().unwrap().message, 5);
    assert_eq!(state.session.previous(&mut state.queue).unwrap().message, 4);
    assert_eq!(state.session.previous(&mut state.queue).unwrap().message, 3);
    for message in [4, 5, 6] {
        assert_eq!(
            state
                .session
                .advance(&mut state.queue, RepeatMode::Off, false, false)
                .unwrap()
                .message,
            message
        );
    }
    store.shutdown().unwrap();
}
#[test]
fn migration_is_repeatable_preserves_duplicates_history_and_source() {
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let track = Track {
        chat: 42,
        message: 100,
        pinned: Some("/offline/song".into()),
        ..Default::default()
    };
    let mut state = State {
        queue_version: 2,
        ..Default::default()
    };
    state
        .session
        .start(&[track.clone(), track.clone(), track.clone()], 1, false);
    state.session.insert(track.clone(), true);
    state.pending_pins.push(track.clone());
    state.previous_queue.push(track);
    let original = serde_json::to_string(&state).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE state(id INTEGER PRIMARY KEY,value TEXT NOT NULL)")
        .unwrap();
    db.execute("INSERT INTO state VALUES(1,?1)", [&original])
        .unwrap();
    for _ in 0..2 {
        let (mut store, restored) = Store::open(&path).unwrap();
        assert_eq!(serde_json::to_string(&restored).unwrap(), original);
        store.shutdown().unwrap();
    }
    let backup: String = db
        .query_row("SELECT value FROM state WHERE id=1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(backup, original);
    let backup: String = db
        .query_row("SELECT value FROM queue_backup WHERE id=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(backup, original);
}
#[test]
fn volume_does_not_write_library_and_flush_reports_write_failure() {
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let (mut store, mut state) = Store::open(&path).unwrap();
    state.playlists = vec![Playlist {
        id: 1,
        tracks: (0..20_000)
            .map(|message| Track {
                chat: 1,
                message,
                title: "large library".repeat(10),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }];
    store.save(state).unwrap();
    store.flush().unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_library BEFORE UPDATE ON library BEGIN SELECT RAISE(FAIL, 'library must not be written'); END;").unwrap();
    for volume in 0..100 {
        store.save_volume(volume as f32, volume as f32).unwrap();
    }
    store.flush().unwrap();
    db.execute_batch("CREATE TRIGGER reject_settings BEFORE UPDATE ON documents BEGIN SELECT RAISE(FAIL, 'injected write failure'); END;").unwrap();
    store.save_volume(12., 12.).unwrap();
    assert!(store
        .flush()
        .unwrap_err()
        .contains("injected write failure"));
    assert!(store.shutdown().is_err());
}
#[test]
fn failed_migration_keeps_legacy_document_and_can_retry() {
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let db = rusqlite::Connection::open(&path).unwrap();
    let original = serde_json::to_string(&State::default()).unwrap();
    db.execute_batch("CREATE TABLE state(id INTEGER PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE documents(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TRIGGER fail_migration BEFORE INSERT ON documents BEGIN SELECT RAISE(FAIL, 'migration failure'); END;").unwrap();
    db.execute("INSERT INTO state VALUES(1,?1)", [&original])
        .unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(
        db.query_row("SELECT value FROM state", [], |r| r.get::<_, String>(0))
            .unwrap(),
        original
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM documents", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER fail_migration").unwrap();
    assert!(Store::open(&path).is_ok());
}
#[test]
fn flush_timeout_is_not_reported_as_success() {
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let (store, _) = Store::open(&path).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    store.save_volume(1., 1.).unwrap();
    assert!(store
        .flush_timeout(std::time::Duration::from_millis(10))
        .unwrap_err()
        .contains("flush"));
    db.execute_batch("ROLLBACK").unwrap();
    store.flush().unwrap();
}

#[test]
fn editing_one_track_only_updates_that_record() {
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let (store, mut state) = Store::open(&path).unwrap();
    state.playlists = vec![Playlist {
        id: 42,
        tracks: vec![
            Track {
                chat: 42,
                message: 1,
                ..Default::default()
            },
            Track {
                chat: 42,
                message: 2,
                ..Default::default()
            },
        ],
        ..Default::default()
    }];
    store.save(state.clone()).unwrap();
    store.flush().unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE writes(message INTEGER); CREATE TRIGGER audit_tracks AFTER UPDATE ON tracks BEGIN INSERT INTO writes VALUES(new.message); END;").unwrap();
    state.playlists[0].tracks[1].title = "edited".into();
    store.save(state).unwrap();
    store.flush().unwrap();
    let writes: Vec<i64> = db
        .prepare("SELECT message FROM writes")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(writes, vec![2]);
}
