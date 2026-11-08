mod common;
use common::TempDir;
use tg_player::adapters::storage::Store;
use tg_player_application::model::{Playlist, State, Track};
// Snapshot input only constructs migration fixtures; the production port accepts ChangeSet.
trait FixtureSave {
    fn save(&self, state: State) -> Result<(), String>;
    fn save_volume(&self, volume: f32, unmuted: f32) -> Result<(), String>;
}
impl FixtureSave for Store {
    fn save_volume(&self, volume: f32, unmuted: f32) -> Result<(), String> {
        let settings = tg_player_application::persistence::Settings {
            volume,
            unmuted_volume: unmuted,
            ..Default::default()
        };
        self.apply(tg_player_application::persistence::ChangeSet {
            settings: Some(settings),
            ..Default::default()
        })
    }
    fn save(&self, mut state: State) -> Result<(), String> {
        for p in &mut state.playlists {
            p.ensure_entry_ids();
        }
        self.apply(tg_player_application::persistence::ChangeSet::between(
            None, &state,
        ))
    }
}
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
    state.session.jump_to(2);
    store.save(state).unwrap();
    store.flush().unwrap();
    store.shutdown().unwrap();
    let (mut store, mut state) = Store::open(&path).unwrap();
    assert_eq!(state.session.current().as_ref().unwrap().message, 5);
    assert_eq!(state.session.previous().unwrap().message, 4);
    assert_eq!(state.session.previous().unwrap().message, 3);
    for message in [4, 5, 6] {
        assert_eq!(
            state
                .session
                .advance(RepeatMode::Off, false, false)
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
        ..Default::default()
    };
    let mut state = tg_player_application::legacy::LegacyImport {
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
        assert_eq!(restored.session, state.session);
        assert_eq!(store.offline().pending.len(), 1);
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
    db.execute_batch("CREATE TABLE writes(message INTEGER); CREATE TRIGGER audit_tracks AFTER UPDATE ON track_records BEGIN INSERT INTO writes VALUES(new.message); END;").unwrap();
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

#[test]
fn backup_includes_wal_and_legacy_offline_paths_and_is_not_overwritten() {
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL;PRAGMA wal_autocheckpoint=0;CREATE TABLE state(id INTEGER PRIMARY KEY,value TEXT NOT NULL)").unwrap();
    let original=serde_json::json!({"queue_version":2,"session":{"snapshot":[{"chat":42,"message":7,"entry_id":9,"pinned":"/offline/song"}],"current":{"chat":42,"message":7,"entry_id":9},"serial":9},"volume":32}).to_string();
    db.execute("INSERT INTO state VALUES(1,?1)", [&original])
        .unwrap();
    let (mut store, state) = Store::open(&path).unwrap();
    assert_eq!(state.volume, 32.);
    assert_eq!(state.session.current().unwrap().entry_id.0, 9);
    assert_eq!(
        store.offline().path(tg_player::domain::TrackId {
            chat: 42,
            message: 7
        }),
        Some("/offline/song")
    );
    store.shutdown().unwrap();
    let backup = dir.path().join("db.pre-v3-0.sqlite");
    let saved = std::fs::read(&backup).unwrap();
    let copy = rusqlite::Connection::open(&backup).unwrap();
    assert_eq!(
        copy.query_row("SELECT value FROM state", [], |r| r.get::<_, String>(0))
            .unwrap(),
        original
    );
    Store::open(&path).unwrap().0.shutdown().unwrap();
    assert_eq!(std::fs::read(backup).unwrap(), saved);
}
#[test]
fn corrupt_and_newer_databases_never_become_empty_libraries() {
    let dir = TempDir::new();
    let corrupt = dir.path().join("bad");
    std::fs::write(&corrupt, b"not SQLite").unwrap();
    assert!(Store::open(&corrupt).is_err());
    let newer = dir.path().join("new");
    let db = rusqlite::Connection::open(&newer).unwrap();
    db.execute_batch("PRAGMA user_version=5").unwrap();
    assert!(Store::open(&newer).err().unwrap().contains("unsupported"));
    assert_eq!(
        db.query_row("SELECT count(*) FROM sqlite_master", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn session_deltas_do_not_touch_library_or_offline_jobs() {
    use tg_player_application::{commands::AppCommand, workflow::Effect, AppCore};
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let (store, state) = Store::open(&path).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_tracks BEFORE INSERT ON track_records BEGIN SELECT RAISE(FAIL,'tracks written');END;CREATE TRIGGER reject_offline BEFORE INSERT ON offline_jobs BEGIN SELECT RAISE(FAIL,'jobs written');END;").unwrap();
    let mut core = AppCore::new(state);
    for effect in core.command(AppCommand::Volume(12.)) {
        if let Effect::Persist(changes) = effect {
            store.apply(*changes).unwrap();
        }
    }
    store.flush().unwrap();
    let effects = core.command(AppCommand::StartSession(
        vec![Track {
            chat: 1,
            message: 1,
            ..Default::default()
        }],
        0,
    ));
    for effect in effects {
        if let Effect::Persist(changes) = effect {
            store.apply(*changes).unwrap();
        }
    }
    store.flush().unwrap();
}
#[test]
fn library_occurrence_ids_and_order_survive_reorder_of_duplicates() {
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let (mut store, mut state) = Store::open(&path).unwrap();
    state.playlists.push(Playlist {
        id: 42,
        tracks: vec![
            Track {
                chat: 42,
                message: 7,
                ..Default::default()
            };
            3
        ],
        ..Default::default()
    });
    store.save(state).unwrap();
    store.shutdown().unwrap();
    let (mut store, mut state) = Store::open(&path).unwrap();
    let ids: Vec<_> = state.playlists[0]
        .tracks
        .iter()
        .map(|t| t.library_id)
        .collect();
    assert_ne!(ids[0], ids[1]);
    state.playlists[0].tracks.swap(0, 2);
    store.save(state).unwrap();
    store.shutdown().unwrap();
    let (mut store, state) = Store::open(&path).unwrap();
    assert_eq!(
        state.playlists[0]
            .tracks
            .iter()
            .map(|t| t.library_id)
            .collect::<Vec<_>>(),
        vec![ids[2], ids[1], ids[0]]
    );
    store.shutdown().unwrap();
}

#[test]
fn deletion_jobs_survive_restart_without_removing_a_new_copy() {
    use tg_player_application::{offline::OfflineCore, persistence::ChangeSet};
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let (mut store, _) = Store::open(&path).unwrap();
    let track = Track {
        chat: 42,
        message: 7,
        ..Default::default()
    };
    let mut core = OfflineCore::new(Default::default());
    core.publish(track.clone(), "old-file".into());
    core.remove(track.id());
    store
        .apply(ChangeSet {
            offline: core.take_changes(),
            ..Default::default()
        })
        .unwrap();
    store.shutdown().unwrap();
    let (mut store, _) = Store::open(&path).unwrap();
    let mut core = OfflineCore::new(store.offline());
    assert!(core.copies.is_empty());
    assert_eq!(core.removing["old-file"].path, "old-file");
    core.enqueue(track.clone(), true);
    core.publish(track.clone(), "new-file".into());
    store
        .apply(ChangeSet {
            offline: core.take_changes(),
            ..Default::default()
        })
        .unwrap();
    core.finish_removal("old-file");
    store
        .apply(ChangeSet {
            offline: core.take_changes(),
            ..Default::default()
        })
        .unwrap();
    store.shutdown().unwrap();
    let (mut store, _) = Store::open(&path).unwrap();
    assert_eq!(store.offline().path(track.id()), Some("new-file"));
    assert!(store.offline().removing.is_empty());
    store.shutdown().unwrap();
}

#[test]
fn offline_page_jobs_and_cursor_roll_back_and_retry_together() {
    use tg_player_application::{offline::OfflineCore, persistence::ChangeSet};
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let (mut store, _) = Store::open(&path).unwrap();
    let mut core = OfflineCore::new(Default::default());
    core.save_playlist(42, [true, false, false]);
    let track = |message| Track {
        chat: 42,
        message,
        ..Default::default()
    };
    core.page(42, 0, 10, vec![track(1)]);
    store
        .apply(ChangeSet {
            offline: core.take_changes(),
            ..Default::default()
        })
        .unwrap();
    store.flush().unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_jobs BEFORE INSERT ON offline_jobs BEGIN SELECT RAISE(FAIL,'disk failure');END;").unwrap();
    core.page(42, 0, 20, vec![track(2)]);
    store
        .apply(ChangeSet {
            offline: core.take_changes(),
            ..Default::default()
        })
        .unwrap();
    assert!(store.flush().unwrap_err().contains("disk failure"));
    assert!(store.shutdown().is_err());
    db.execute_batch("DROP TRIGGER reject_jobs").unwrap();
    let (mut store, _) = Store::open(&path).unwrap();
    let mut core = OfflineCore::new(store.offline());
    assert_eq!(core.pending.len(), 1);
    assert_eq!(core.policies[&42].cursors[0], 10);
    core.page(42, 0, 20, vec![track(2)]);
    store
        .apply(ChangeSet {
            offline: core.take_changes(),
            ..Default::default()
        })
        .unwrap();
    store.shutdown().unwrap();
    let (mut store, _) = Store::open(&path).unwrap();
    assert_eq!(store.offline().pending.len(), 2);
    assert_eq!(store.offline().policies[0].cursors[0], 20);
    store.shutdown().unwrap();
}

#[test]
fn one_completion_touches_only_two_records_in_large_backlog() {
    use tg_player_application::{
        offline::{OfflineCore, OfflineData},
        persistence::ChangeSet,
    };
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let (mut store, _) = Store::open(&path).unwrap();
    let data = OfflineData {
        pending: (1..=10_000)
            .map(|message| Track {
                chat: 42,
                message,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    store
        .apply(ChangeSet {
            offline: Some(data.clone().into()),
            ..Default::default()
        })
        .unwrap();
    store.flush().unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER no_reinsertion BEFORE INSERT ON offline_jobs BEGIN SELECT RAISE(FAIL,'rewrote backlog'); END; CREATE TRIGGER only_finished_job BEFORE DELETE ON offline_jobs WHEN OLD.id!='42:5000' BEGIN SELECT RAISE(FAIL,'deleted unrelated job'); END;").unwrap();
    let mut core = OfflineCore::new(data);
    core.publish(
        Track {
            chat: 42,
            message: 5000,
            ..Default::default()
        },
        "ready".into(),
    );
    store
        .apply(ChangeSet {
            offline: core.take_changes(),
            ..Default::default()
        })
        .unwrap();
    store.shutdown().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM offline_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        9999
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM offline_copies", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
}

#[test]
fn v3_migration_backs_up_wal_and_retries_without_losing_offline_state() {
    use tg_player_application::offline::OfflineCore;
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let (mut store, mut expected) = Store::open(&path).unwrap();
    expected.api_id = "12345".into();
    expected.volume = 32.;
    let repeated = Track {
        chat: 42,
        message: 9,
        title: "kept".into(),
        ..Default::default()
    };
    expected.playlists.push(Playlist {
        id: 42,
        tracks: vec![repeated.clone(); 3],
        ..Default::default()
    });
    expected.playlists[0].ensure_entry_ids();
    expected
        .session
        .start(&[repeated.clone(), repeated.clone(), repeated], 1, false);
    store.save(expected.clone()).unwrap();
    store.shutdown().unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; PRAGMA user_version=3")
        .unwrap();
    let mut core = OfflineCore::new(Default::default());
    core.save_playlist(42, [true, false, true]);
    core.page(
        42,
        0,
        123,
        vec![Track {
            chat: 42,
            message: 200,
            ..Default::default()
        }],
    );
    core.exclude(tg_player::domain::TrackId {
        chat: 42,
        message: 17,
    });
    core.publish(
        Track {
            chat: 42,
            message: 201,
            ..Default::default()
        },
        "ready-file".into(),
    );
    core.publish(
        Track {
            chat: 42,
            message: 202,
            ..Default::default()
        },
        "removing-file".into(),
    );
    core.remove(tg_player::domain::TrackId {
        chat: 42,
        message: 202,
    });
    let data = core.data();
    for (table, key, copy) in [
        ("offline_copies", "42:201", &data.copies[0]),
        ("offline_removal_jobs", "removing-file", &data.removing[0]),
    ] {
        db.execute(
            &format!("INSERT INTO {table}(id,value) VALUES(?1,?2)"),
            rusqlite::params![key, serde_json::to_string(copy).unwrap()],
        )
        .unwrap();
    }
    let mut legacy = serde_json::to_value(&data.policies).unwrap();
    legacy[0].as_object_mut().unwrap().remove("head");
    for (key, value) in [
        ("offline_policies", legacy),
        (
            "offline_exclusions",
            serde_json::to_value(&data.excluded).unwrap(),
        ),
    ] {
        db.execute(
            "INSERT INTO documents(key,value) VALUES(?1,?2)",
            rusqlite::params![key, value.to_string()],
        )
        .unwrap();
    }
    db.execute(
        "INSERT INTO offline_jobs VALUES('42:200',?1)",
        [serde_json::to_string(&data.pending[0]).unwrap()],
    )
    .unwrap();
    db.execute_batch("CREATE TRIGGER fail_v4 BEFORE INSERT ON offline_policy_records BEGIN SELECT RAISE(FAIL,'migration failure');END;").unwrap();
    assert!(matches!(Store::open(&path), Err(e) if e.contains("migration failure")));
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM offline_policy_records", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let backup = rusqlite::Connection::open(path.with_file_name("db.pre-v4-0.sqlite")).unwrap();
    assert_eq!(
        backup
            .query_row("SELECT count(*) FROM offline_jobs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.execute_batch("DROP TRIGGER fail_v4").unwrap();
    for _ in 0..2 {
        let (mut store, restored) = Store::open(&path).unwrap();
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        let offline = store.offline();
        assert_eq!(offline.policies[0].cursors[0], 123);
        assert_eq!(offline.pending.len(), 1);
        assert_eq!(offline.copies, data.copies);
        assert_eq!(offline.removing, data.removing);
        assert_eq!(
            offline
                .excluded
                .iter()
                .collect::<std::collections::HashSet<_>>(),
            data.excluded
                .iter()
                .collect::<std::collections::HashSet<_>>()
        );
        store.shutdown().unwrap();
    }
    assert!(path.with_file_name("db.pre-v4-1.sqlite").exists());
    assert!(!path.with_file_name("db.pre-v4-2.sqlite").exists());
}

#[test]
fn offline_enqueue_order_survives_restart_and_coalesced_requeue() {
    use tg_player_application::{
        offline::{OfflineCore, OfflineData},
        persistence::ChangeSet,
    };
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let track = |message| Track {
        chat: 42,
        message,
        ..Default::default()
    };
    let (mut store, _) = Store::open(&path).unwrap();
    let mut core = OfflineCore::new(OfflineData::default());
    for message in [20, 3, 100, 2] {
        core.enqueue(track(message), true);
    }
    store
        .apply(ChangeSet {
            offline: core.take_changes(),
            ..Default::default()
        })
        .unwrap();
    store.shutdown().unwrap();

    let (mut store, _) = Store::open(&path).unwrap();
    let data = store.offline();
    assert_eq!(
        data.pending
            .iter()
            .map(|track| track.message)
            .collect::<Vec<_>>(),
        [20, 3, 100, 2]
    );
    let mut core = OfflineCore::new(data);
    core.source_changed(track(3).id());
    core.enqueue(track(5), true);
    let mut changes = ChangeSet {
        offline: core.take_changes(),
        ..Default::default()
    };
    let mut edited = track(3);
    edited.title = "edited and requeued".into();
    core.enqueue(edited, true);
    changes.merge(ChangeSet {
        offline: core.take_changes(),
        ..Default::default()
    });
    store.apply(changes).unwrap();
    store.shutdown().unwrap();

    let (mut store, _) = Store::open(&path).unwrap();
    let data = store.offline();
    assert_eq!(
        data.pending
            .iter()
            .map(|track| track.message)
            .collect::<Vec<_>>(),
        [20, 100, 2, 5, 3]
    );
    assert_eq!(data.pending.last().unwrap().title, "edited and requeued");
    store.shutdown().unwrap();
}

#[test]
fn migration_schema_and_data_roll_back_together_and_retry() {
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let (mut store, _) = Store::open(&path).unwrap();
    store.shutdown().unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP TABLE offline_policy_records; DROP TABLE offline_exclusion_records; PRAGMA user_version=3;
        INSERT INTO documents(key,value) VALUES('offline_policies','[]'),('offline_exclusions','[]');
        INSERT INTO offline_jobs(id,value) VALUES('42:7','{\"chat\":42,\"message\":7}');
        CREATE TRIGGER fail_upgrade BEFORE INSERT ON offline_jobs BEGIN SELECT RAISE(FAIL,'schema rollback'); END;")
        .unwrap();
    assert!(matches!(Store::open(&path), Err(error) if error.contains("schema rollback")));
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(db.query_row("SELECT count(*) FROM sqlite_master WHERE name IN ('offline_policy_records','offline_exclusion_records')", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
    assert_eq!(
        db.query_row("SELECT count(*) FROM offline_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.execute_batch("DROP TRIGGER fail_upgrade").unwrap();
    let (mut store, _) = Store::open(&path).unwrap();
    assert_eq!(store.offline().pending[0].message, 7);
    store.shutdown().unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
}

#[test]
fn missing_v4_table_is_reported_instead_of_recreated() {
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let (mut store, _) = Store::open(&path).unwrap();
    store.shutdown().unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP TABLE offline_jobs").unwrap();
    assert!(matches!(Store::open(&path), Err(error) if error.contains("offline_jobs")));
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='offline_jobs'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn split_legacy_track_and_session_paths_survive_pure_metadata_migration() {
    let dir = TempDir::new();
    let path = dir.path().join("db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA user_version=2;
        CREATE TABLE documents(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        CREATE TABLE library(id INTEGER PRIMARY KEY,value TEXT NOT NULL);
        CREATE TABLE tracks(playlist INTEGER NOT NULL,position INTEGER NOT NULL,chat INTEGER NOT NULL,message INTEGER NOT NULL,value TEXT NOT NULL,PRIMARY KEY(playlist,position));")
        .unwrap();
    let settings = tg_player_application::persistence::Settings {
        volume: 32.,
        api_id: "12345".into(),
        ..Default::default()
    };
    let session = serde_json::json!({"snapshot":[{"chat":42,"message":8,"entry_id":9,"pinned":"/offline/session"}],"current":{"chat":42,"message":8,"entry_id":9},"serial":9}).to_string();
    let offline = serde_json::json!([
        {"chat":42,"message":10,"pinned":"/offline/document"},
        {"chat":42,"message":11}
    ])
    .to_string();
    for (key, value) in [
        ("settings", serde_json::to_string(&settings).unwrap()),
        ("session_v2", session.clone()),
        ("library_order", "[42]".into()),
        (
            "legacy_queue",
            r#"[{"chat":42,"message":9,"pinned":"/offline/queue"}]"#.into(),
        ),
        ("offline", offline),
    ] {
        db.execute(
            "INSERT INTO documents(key,value) VALUES(?1,?2)",
            rusqlite::params![key, value],
        )
        .unwrap();
    }
    let header = serde_json::json!({"id":42,"title":"Legacy","filters":[true,true,false],"cursors":[321,654,0],"done":[false,false,true],"image_file":99}).to_string();
    db.execute("INSERT INTO library(id,value) VALUES(42,?1)", [header])
        .unwrap();
    let original = serde_json::json!({"chat":42,"message":7,"title":"Saved library track","pinned":"/offline/library","file":77,"cached":"/retired/cache","image_file":99}).to_string();
    for position in 0..2 {
        db.execute(
            "INSERT INTO tracks(playlist,position,chat,message,value) VALUES(42,?1,42,7,?2)",
            rusqlite::params![position, original],
        )
        .unwrap();
    }
    for _ in 0..2 {
        let (mut store, state) = Store::open(&path).unwrap();
        assert_eq!(state.api_id, "12345");
        assert_eq!(state.volume, 32.);
        assert_eq!(state.session.current().unwrap().message, 8);
        let playlist = &state.playlists[0];
        assert_eq!(playlist.cursors, [321, 654, 0]);
        assert_eq!(playlist.done, [false, false, true]);
        assert_eq!(playlist.tracks.len(), 2);
        assert_ne!(playlist.tracks[0].library_id, playlist.tracks[1].library_id);
        for field in ["file", "cached", "pinned", "image_file", "image_path"] {
            assert!(serde_json::to_value(&playlist.tracks[0])
                .unwrap()
                .get(field)
                .is_none());
        }
        let data = store.offline();
        for (message, path) in [
            (7, "/offline/library"),
            (8, "/offline/session"),
            (9, "/offline/queue"),
            (10, "/offline/document"),
        ] {
            assert_eq!(
                data.path(tg_player::domain::TrackId { chat: 42, message }),
                Some(path)
            );
        }
        assert_eq!(
            data.pending
                .iter()
                .map(|track| track.message)
                .collect::<Vec<_>>(),
            [11]
        );
        store.shutdown().unwrap();
    }
    assert_eq!(
        db.query_row(
            "SELECT value FROM documents WHERE key='session_v2'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        session
    );
    assert_eq!(
        db.query_row("SELECT value FROM tracks WHERE position=0", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        original
    );
}
