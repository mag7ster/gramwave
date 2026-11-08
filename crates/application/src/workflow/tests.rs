use super::*;
use crate::{model::State, offline::OfflineData};
use std::time::Instant;
fn track(message: i64) -> Track {
    Track {
        chat: 42,
        message,
        ..Default::default()
    }
}
fn download(core: &mut AppCore, id: tg_player_domain::TrackId) -> RequestId {
    core.pump(Instant::now(), true)
        .into_iter()
        .find_map(|effect| match effect {
            Effect::DownloadOffline { operation, track } if track.id() == id => {
                Some(operation.key())
            }
            _ => None,
        })
        .unwrap()
}
#[test]
fn cancelled_download_cannot_publish_over_a_new_attempt() {
    let mut core = AppCore::new(State::default());
    let track = track(7);
    let id = track.id();
    core.command(AppCommand::SaveTrack(track.clone()));
    let old = download(&mut core, id);
    core.command(AppCommand::CancelOffline);
    core.command(AppCommand::SaveTrack(track));
    let current = download(&mut core, id);
    let effects = core.event(AppEvent::OfflineDownloaded {
        request: old,
        id,
        result: Ok("old".into()),
    });
    assert!(core.offline().removing.contains_key("old"));
    assert!(effects.iter().any(|effect| matches!(effect,
        Effect::Persist(changes) if changes.offline.as_ref().is_some_and(|delta|
            delta.removing.get("old").is_some_and(Option::is_some)))));
    assert!(core.offline().path(id).is_none());
    assert!(core.offline().pending.contains_key(&id));
    core.event(AppEvent::OfflineDownloaded {
        request: current,
        id,
        result: Ok("new".into()),
    });
    assert_eq!(core.offline().path(id), Some("new"));
    assert!(core.offline().pending.is_empty());
}
#[test]
fn failed_download_keeps_its_job_without_stopping_playback_or_other_jobs() {
    let mut core = AppCore::new(State::default());
    core.command(AppCommand::SaveTrack(track(7)));
    core.command(AppCommand::SaveTrack(track(8)));
    let id = track(7).id();
    let request = download(&mut core, id);
    core.command(AppCommand::Play(track(9)));
    core.event(AppEvent::OfflineDownloaded {
        request,
        id,
        result: Err(AppError::Timeout("download")),
    });
    assert!(core.offline().retrying(id));
    assert_eq!(core.offline().pending.len(), 2);
    assert!(core.playback().state().loading());
}
#[test]
fn removing_old_file_cannot_remove_a_new_copy_of_the_same_message() {
    let mut data = crate::offline::OfflineCore::new(OfflineData::default());
    data.publish(track(7), "old".into());
    let mut core = AppCore::with_offline(State::default(), data.data());
    let id = track(7).id();
    core.command(AppCommand::RemoveOffline(id, "old".into()));
    let request = core
        .pump(Instant::now(), false)
        .into_iter()
        .find_map(|e| match e {
            Effect::RemoveOffline { operation, .. } => Some(operation.key()),
            _ => None,
        })
        .unwrap();
    core.command(AppCommand::SaveTrack(track(7)));
    let next = download(&mut core, id);
    core.event(AppEvent::OfflineDownloaded {
        request: next,
        id,
        result: Ok("new".into()),
    });
    core.event(AppEvent::OfflineRemoved {
        request,
        path: "old".into(),
        result: Ok(()),
    });
    assert_eq!(core.offline().path(id), Some("new"));
    assert!(core.offline().removing.is_empty());
}
#[test]
fn repeated_ready_is_idempotent_and_old_auth_result_is_ignored() {
    let mut core = AppCore::new(State::default());
    core.command(AppCommand::SavePlaylist(42, [true, false, false]));
    let old = core
        .command(AppCommand::Authenticate(crate::telegram::AuthRequest::Qr))
        .into_iter()
        .find_map(|e| match e {
            Effect::Authenticate { operation, .. } => Some(operation.key()),
            _ => None,
        })
        .unwrap();
    core.event(AppEvent::Authorization(
        crate::telegram::AuthorizationState::Ready,
    ));
    assert!(core
        .event(AppEvent::Authorization(
            crate::telegram::AuthorizationState::Ready
        ))
        .is_empty());
    assert!(core
        .event(AppEvent::Authenticated {
            request: old,
            result: Err(AppError::Cancelled)
        })
        .is_empty());
    assert!(!core.auth().running());
    assert!(core.offline().policies[&42].head[0].active);
}
#[test]
fn latest_playback_result_wins_and_queue_occurrences_are_preserved() {
    let mut core = AppCore::new(State::default());
    let first = core.command(AppCommand::StartSession(
        vec![track(7), track(7), track(8)],
        0,
    ));
    let old = first
        .into_iter()
        .find_map(|e| match e {
            Effect::PreparePlayback { operation, .. } => Some(operation.key()),
            _ => None,
        })
        .unwrap();
    let effects = core.command(AppCommand::Next);
    let request = effects
        .into_iter()
        .find_map(|e| match e {
            Effect::PreparePlayback { operation, .. } => Some(operation.key()),
            _ => None,
        })
        .unwrap();
    assert!(core
        .event(AppEvent::PlaybackPrepared {
            request: old,
            revision: 0,
            result: Err(AppError::Timeout("old"))
        })
        .is_empty());
    let current: Track = core.view().session.current().unwrap().into();
    core.event(AppEvent::PlaybackPrepared {
        request,
        revision: 0,
        result: Ok(PreparedPlayback {
            track: current.into(),
            path: Some("stream".into()),
        }),
    });
    assert_eq!(core.view().session.snapshot().len(), 3);
    assert_eq!(core.view().session.remaining().len(), 1);
}

#[test]
fn stale_publication_cleanup_survives_restart_and_removal_failure_then_retries() {
    let mut core = AppCore::new(State::default());
    let id = track(7).id();
    core.command(AppCommand::SaveTrack(track(7)));
    let old = download(&mut core, id);
    core.command(AppCommand::CancelOffline);
    let effects = core.event(AppEvent::OfflineDownloaded {
        request: old,
        id,
        result: Ok("orphan-attempt".into()),
    });
    assert!(effects.iter().any(|effect| matches!(effect,
        Effect::Persist(changes) if changes.offline.as_ref().is_some_and(|delta|
            delta.removing.get("orphan-attempt").is_some_and(Option::is_some)))));
    assert!(
        core.event(AppEvent::OfflineDownloaded {
            request: old,
            id,
            result: Ok("orphan-attempt".into())
        })
        .is_empty(),
        "duplicate stale events must not duplicate removal jobs"
    );
    let mut core = AppCore::with_offline(State::default(), core.offline().data());
    let now = Instant::now();
    let request = core
        .pump(now, false)
        .into_iter()
        .find_map(|effect| match effect {
            Effect::RemoveOffline { operation, path } if path == "orphan-attempt" => {
                Some(operation.key())
            }
            _ => None,
        })
        .unwrap();
    core.event(AppEvent::OfflineRemoved {
        request,
        path: "orphan-attempt".into(),
        result: Err(AppError::Storage("disk unavailable".into())),
    });
    assert!(core.offline().removing.contains_key("orphan-attempt"));
    assert!(core.pump(now, false).is_empty());
    let retry = core
        .pump(Instant::now() + std::time::Duration::from_secs(31), false)
        .into_iter()
        .find_map(|effect| match effect {
            Effect::RemoveOffline { operation, path } if path == "orphan-attempt" => {
                Some(operation.key())
            }
            _ => None,
        })
        .unwrap();
    assert_ne!(request, retry);
    let effects = core.event(AppEvent::OfflineRemoved {
        request: retry,
        path: "orphan-attempt".into(),
        result: Ok(()),
    });
    assert!(core.offline().removing.is_empty());
    assert!(effects.iter().any(|effect| matches!(effect,
        Effect::Persist(changes) if changes.offline.as_ref().is_some_and(|delta|
            delta.removing.get("orphan-attempt").is_some_and(Option::is_none)))));
}

#[test]
fn stale_publication_cleanup_never_removes_a_registered_copy_with_the_same_path() {
    let mut data = crate::offline::OfflineCore::new(OfflineData::default());
    data.publish(track(8), "registered-file".into());
    let mut core = AppCore::with_offline(State::default(), data.data());
    let effects = core.event(AppEvent::OfflineDownloaded {
        request: crate::tasks::Operation::new(0).key(),
        id: track(7).id(),
        result: Ok("registered-file".into()),
    });
    assert!(effects.is_empty());
    assert!(core.offline().removing.is_empty());
    assert_eq!(core.offline().path(track(8).id()), Some("registered-file"));
}
