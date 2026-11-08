use crate::{
    commands::AppCommand,
    library::LibraryCommand,
    media::MediaTrack,
    model::{Playlist, State, Track},
    playback::PlaybackState,
    ports::{PlaybackEvent, PlayerCommand, PlayerSnapshot},
    tasks::{Operation, RequestId},
    telegram::{AuthRequest, AuthorizationState, MediaPage, TelegramUpdate},
    workflow::{AppEvent, Effect, PreparedPlayback},
    AppCore, AppError,
};
use std::time::Instant;

fn song() -> Track {
    Track {
        chat: 42,
        message: 7,
        title: "Queue snapshot".into(),
        ..Default::default()
    }
}

fn preparation(effects: &[Effect]) -> (Operation, Track, u64) {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PreparePlayback {
                operation,
                track,
                revision,
                ..
            } => Some((operation.clone(), track.clone(), *revision)),
            _ => None,
        })
        .expect("the selected track must be prepared")
}

#[test]
fn retrying_failed_playback_prepares_again_despite_previous_duration() {
    let mut core = AppCore::new(State::default());
    let (failed, target, revision) =
        preparation(&core.command(AppCommand::StartSession(vec![song()], 0)));
    core.event(AppEvent::PlaybackPrepared {
        request: failed.key(),
        revision,
        result: Err(AppError::Timeout("prepare playback")),
    });
    assert_eq!(core.playback().state(), PlaybackState::Failed);
    // mpv may retain its previous file's properties after a failed replacement.
    core.event(AppEvent::Snapshot(PlayerSnapshot {
        duration: 240.,
        position: 30.,
        ..Default::default()
    }));
    let effects = core.command(AppCommand::Pause(false));
    let (retry, retried_track, _) = preparation(&effects);
    assert_ne!(retry.key(), failed.key());
    assert_eq!(retried_track, target);
    assert_eq!(
        core.playback().state(),
        PlaybackState::Loading { paused: false }
    );
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::Player(PlayerCommand::Pause(false)))),
        "resuming a failed selection must not unpause the previous file"
    );
    assert_eq!(
        core.view().session.current().unwrap().entry_id,
        target.entry_id
    );
}

#[test]
fn repeated_code_refreshes_resend_deadline_but_repeated_ready_keeps_scan_position() {
    let mut core = AppCore::new(State::default());
    let code = AuthorizationState::Code {
        email: false,
        delivery: "SMS".into(),
        next: true,
        timeout: 60,
    };
    core.event(AppEvent::Authorization(code.clone()));
    let resend = core
        .command(AppCommand::Authenticate(AuthRequest::Resend))
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Authenticate {
                operation,
                request: AuthRequest::Resend,
            } => Some(operation.key()),
            _ => None,
        })
        .unwrap();
    core.event(AppEvent::Authenticated {
        request: resend,
        result: Ok(()),
    });
    let effects = core.event(AppEvent::Authorization(code.clone()));
    assert!(
        effects.iter().any(|effect| matches!(effect,
            Effect::AuthorizationChanged(state) if state == &code
        )),
        "a fresh code challenge may repeat the same delivery and timeout values"
    );
    assert!(!core.auth().running());

    core.command(AppCommand::SavePlaylist(42, [true, false, false]));
    core.event(AppEvent::Authorization(AuthorizationState::Ready));
    let request = core
        .pump(Instant::now(), true)
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ScanOffline {
                operation,
                chat: 42,
                kind: 0,
                ..
            } => Some(operation.key()),
            _ => None,
        })
        .unwrap();
    core.event(AppEvent::OfflinePage {
        request,
        chat: 42,
        kind: 0,
        result: Ok(MediaPage {
            newest: 100,
            tracks: vec![],
            next: 50,
        }),
    });
    let before = core.offline().policies[&42].clone();
    assert_eq!(before.head[0].cursor, 50);
    assert!(core
        .event(AppEvent::Authorization(AuthorizationState::Ready))
        .is_empty());
    assert_eq!(core.offline().policies[&42], before);
}

#[test]
fn failed_preparation_from_before_live_edit_retries_and_rejects_other_job_ids() {
    let original = song();
    let mut core = AppCore::new(State {
        playlists: vec![Playlist {
            id: original.chat,
            tracks: vec![original.clone()],
            ..Default::default()
        }],
        ..Default::default()
    });
    let (old, _, old_revision) =
        preparation(&core.command(AppCommand::StartSession(vec![original.clone()], 0)));
    let mut edited: MediaTrack = original.clone().into();
    edited.title = "Live edited title".into();
    edited.file = 90;
    core.library_command(LibraryCommand::Source(TelegramUpdate::Message {
        id: original.id(),
        track: Some(edited),
    }));
    let effects = core.event(AppEvent::PlaybackPrepared {
        request: old.key(),
        revision: old_revision,
        result: Err(AppError::Unavailable("outdated file")),
    });
    let (retry, target, revision) = preparation(&effects);
    assert!(old.cancel.is_cancelled());
    assert_ne!(retry.key(), old.key());
    assert!(revision > old_revision);
    assert!(effects
        .iter()
        .all(|effect| !matches!(effect, Effect::Error(_))));
    assert_eq!(target.title, original.title);
    assert_eq!(
        core.view().playlists[0].tracks[0].title,
        "Live edited title"
    );
    assert_eq!(core.view().session.current().unwrap().title, original.title);

    // Neither the original result nor a different job using the new generation
    // may clear loading or install media for the active selection.
    for request in [
        old.key(),
        RequestId {
            job: old.id,
            generation: retry.generation,
        },
    ] {
        assert!(core
            .event(AppEvent::PlaybackPrepared {
                request,
                revision,
                result: Err(AppError::Timeout("stale preparation")),
            })
            .is_empty());
    }
    assert!(core.playback().state().loading());
    let effects = core.event(AppEvent::PlaybackPrepared {
        request: retry.key(),
        revision,
        result: Ok(PreparedPlayback {
            track: target.into(),
            path: Some("current-media".into()),
        }),
    });
    assert!(effects.iter().any(|effect| matches!(effect,
        Effect::Player(PlayerCommand::Load(path)) if path == "current-media"
    )));
}

#[test]
fn native_stop_submission_failure_cancels_preparation_and_rejects_late_success() {
    let mut core = AppCore::new(State::default());
    let effects = core.command(AppCommand::StartSession(vec![song()], 0));
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::Player(PlayerCommand::Stop))));
    let (operation, track, revision) = preparation(&effects);
    let error = AppError::Player("mpv worker is closed".into());
    let effects = core.event(AppEvent::Player(PlaybackEvent::Failed(error.clone())));
    assert!(effects.iter().any(|effect| matches!(effect,
        Effect::Error(actual) if actual == &error
    )));
    assert!(
        operation.cancel.is_cancelled(),
        "failed Stop must prevent later preparation from loading a file"
    );
    assert_eq!(core.playback().state(), PlaybackState::Failed);
    assert!(core
        .event(AppEvent::PlaybackPrepared {
            request: operation.key(),
            revision,
            result: Ok(PreparedPlayback {
                track: track.into(),
                path: Some("late-success".into()),
            }),
        })
        .is_empty());
    assert_eq!(core.playback().state(), PlaybackState::Failed);
}

#[test]
fn saving_after_a_live_edit_seeds_current_metadata_before_protected_scan_pages() {
    let original = song();
    let mut core = AppCore::new(State {
        playlists: vec![Playlist {
            id: 42,
            tracks: vec![original.clone()],
            ..Default::default()
        }],
        ..Default::default()
    });
    core.library_command(LibraryCommand::Connected(1));
    let mut edited = original.clone();
    edited.title = "Edited before saving".into();
    core.library_command(LibraryCommand::Source(TelegramUpdate::Message {
        id: edited.id(),
        track: Some(edited.clone().into()),
    }));
    assert!(core.offline().pending.is_empty());
    core.command(AppCommand::SavePlaylist(42, [true, false, false]));
    assert_eq!(core.offline().pending[&edited.id()].title, edited.title);
    let request = core
        .pump(Instant::now(), true)
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ScanOffline { operation, .. } => Some(operation.key()),
            _ => None,
        })
        .unwrap();
    core.event(AppEvent::OfflinePage {
        request,
        chat: 42,
        kind: 0,
        result: Ok(MediaPage {
            newest: original.message,
            next: 0,
            tracks: vec![original.into()],
        }),
    });
    assert_eq!(core.offline().pending[&edited.id()].title, edited.title);
    assert_eq!(core.offline().pending.len(), 1);
}
