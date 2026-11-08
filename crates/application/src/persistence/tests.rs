use super::*;
use crate::{
    commands::AppCommand,
    workflow::{AppEvent, Effect},
    AppCore, AppError,
};
use std::time::Instant;

fn track(message: i64) -> Track {
    Track {
        chat: 42,
        message,
        ..Default::default()
    }
}
fn persistence(effects: Vec<Effect>) -> ChangeSet {
    let mut changes = ChangeSet::default();
    for effect in effects {
        if let Effect::Persist(delta) = effect {
            changes.merge(*delta);
        }
    }
    changes
}

#[test]
fn rejected_completion_is_retained_with_later_offline_changes_without_retry_recursion() {
    let mut core = AppCore::new(State::default());
    core.command(AppCommand::SaveTrack(track(1)));
    let request = core
        .pump(Instant::now(), true)
        .into_iter()
        .find_map(|effect| match effect {
            Effect::DownloadOffline {
                operation,
                track: pending,
            } if pending.id() == track(1).id() => Some(operation.key()),
            _ => None,
        })
        .unwrap();
    let rejected = persistence(core.event(AppEvent::OfflineDownloaded {
        request,
        id: track(1).id(),
        result: Ok("ready-file".into()),
    }));
    assert!(rejected.offline.as_ref().unwrap().pending[&track(1).id()].is_none());
    let effects = core.event(AppEvent::PersistenceRejected(
        Box::new(rejected),
        AppError::Storage("closed repository".into()),
    ));
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::Error(_))));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Persist(_))),
        "a rejection must await a later retry, not resubmit recursively"
    );

    let retry = persistence(core.command(AppCommand::SaveTrack(track(2))));
    let delta = retry.offline.unwrap();
    assert!(delta.pending[&track(1).id()].is_none());
    assert_eq!(
        delta.copies[&track(1).id()].as_ref().unwrap().path,
        "ready-file"
    );
    assert_eq!(
        delta.pending[&track(2).id()].as_ref().unwrap().id(),
        track(2).id()
    );
    assert_eq!(delta.pending.len(), 2);
    assert_eq!(delta.copies.len(), 1);
}

#[test]
fn newer_cancellation_and_settings_supersede_rejected_values() {
    let mut core = AppCore::new(State::default());
    let rejected = persistence(core.command(AppCommand::SaveTrack(track(1))));
    core.event(AppEvent::PersistenceRejected(
        Box::new(rejected),
        AppError::Storage("closed".into()),
    ));
    let retry = persistence(core.command(AppCommand::CancelOffline));
    assert!(retry.offline.unwrap().pending[&track(1).id()].is_none());

    let rejected = persistence(core.command(AppCommand::Volume(12.)));
    core.event(AppEvent::PersistenceRejected(
        Box::new(rejected),
        AppError::Storage("closed".into()),
    ));
    let retry = persistence(core.command(AppCommand::Volume(34.)));
    assert_eq!(retry.settings.unwrap().volume, 34.);
    assert!(core.take_changes().is_empty());
}
