use super::*;
use crate::{model::State, offline::OfflineData, ports::QueuedPlayback};
use std::time::Instant;
fn track(message: i64) -> Track {
    Track {
        chat: 42,
        message,
        ..Default::default()
    }
}

#[test]
fn native_advance_preserves_queue_occurrences_and_rechecks_queue_edits() {
    let repeated = track(1);
    let third = track(3);
    let mut state = State::default();
    state.session.start(
        &[repeated.clone(), repeated.clone(), third.clone()],
        0,
        false,
    );
    let first = state.session.current().unwrap().entry_id;
    let second = state.session.remaining()[0].entry_id;
    let mut core = AppCore::new(state);
    core.enable_streamed_next(true);
    core.playback.state = crate::playback::PlaybackState::Playing;
    for (track, path) in [(repeated, "repeated.ogg"), (third, "third.ogg")] {
        let mut media = crate::media::MediaTrack::from(track);
        media.cached = Some(path.into());
        core.library.cache_track(&media);
    }
    let (next, revision, _) = core.next_streamed_playback().unwrap();
    assert_eq!(next.entry_id, second);
    let effects = core.event(AppEvent::Player(PlaybackEvent::Advanced(QueuedPlayback {
        entry_id: next.entry_id,
        revision,
        path: next.cached.unwrap(),
    })));
    assert_eq!(core.view().session.current().unwrap().entry_id, second);
    assert_eq!(core.view().session.history()[0].entry_id, first);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::PlaybackAdvanced(_))));
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::Player(_) | Effect::PreparePlayback { .. })));

    let (stale, revision, _) = core.next_streamed_playback().unwrap();
    core.command(AppCommand::Insert(track(4), true));
    let effects = core.event(AppEvent::Player(PlaybackEvent::Advanced(QueuedPlayback {
        entry_id: stale.entry_id,
        revision,
        path: stale.cached.unwrap(),
    })));
    assert_eq!(core.view().session.current().unwrap().message, 4);
    assert!(effects.iter().any(
        |effect| matches!(effect, Effect::PreparePlayback { track, .. } if track.message == 4)
    ));
}

#[test]
fn startup_uses_the_loaded_track_chat_even_after_stopping() {
    use crate::{
        model::{Playlist, StartupPlaylist},
        persistence::Settings,
    };

    let mut first = track(1);
    first.chat = 1;
    let mut mixed = track(2);
    mixed.chat = 2;
    let legacy = r#"{"last_opened_chat":3}"#;
    let settings: Settings = serde_json::from_str(legacy).unwrap();
    assert_eq!(settings.startup_playlist, StartupPlaylist::LastOpened);
    let mut state = State {
        playlists: (1..=3)
            .map(|id| Playlist {
                id,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    settings.apply(&mut state);
    state.session.start(&[first, mixed.clone()], 0, false);
    assert_eq!(state.startup_chat(), Some(3));
    state.startup_playlist = StartupPlaylist::LastPlayed;
    assert_eq!(state.startup_chat(), Some(1));

    let mut core = AppCore::new(state);
    let effects = core.command(AppCommand::Next);
    let (request, revision) = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::PreparePlayback {
                operation,
                revision,
                ..
            } => Some((operation.key(), revision)),
            _ => None,
        })
        .unwrap();
    core.event(AppEvent::PlaybackPrepared {
        request,
        revision,
        result: Ok(PreparedPlayback {
            track: mixed.into(),
            path: Some("test.ogg".into()),
        }),
    });
    let effects = core.event(AppEvent::Player(PlaybackEvent::Loaded));
    let saved = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Persist(changes) => changes.settings,
            _ => None,
        })
        .unwrap();
    core.command(AppCommand::Stop);

    let mut reopened = core.view().clone();
    serde_json::from_str::<Settings>(&serde_json::to_string(&saved).unwrap())
        .unwrap()
        .apply(&mut reopened);
    assert!(reopened.session.current().is_none());
    assert_eq!(reopened.startup_chat(), Some(2));
    reopened.playlists.retain(|playlist| playlist.id != 2);
    assert_eq!(reopened.startup_chat(), Some(3));
}

fn download(core: &mut AppCore, id: gramwave_domain::TrackId) -> RequestId {
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
    let deadline = core.offline().retry_deadline(now, false).unwrap();
    assert!(deadline >= now + std::time::Duration::from_secs(30));
    let retry = core
        .pump(deadline, false)
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

#[test]
fn removing_playlists_cancels_only_selected_work_and_survives_restart() {
    let mut data = crate::offline::OfflineCore::new(OfflineData::default());
    let saved = track(2);
    let selected = track(3);
    let mut other = track(4);
    other.chat = 90;
    data.save_playlist(saved.chat, [true, false, false]);
    data.save_playlist(other.chat, [true, false, false]);
    data.publish_file(
        saved.clone(),
        crate::offline::PublishedFile {
            path: "saved".into(),
            size: 100,
            remote_id: None,
        },
        crate::offline::unix_seconds(std::time::SystemTime::now()),
    );
    let mut core = AppCore::with_offline(State::default(), data.data());
    core.command(AppCommand::SaveTrack(selected.clone()));
    core.command(AppCommand::SaveTrack(other.clone()));
    let attempts: std::collections::HashMap<_, _> = core
        .pump(Instant::now(), true)
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::DownloadOffline { track, operation } => Some((track.id(), operation)),
            _ => None,
        })
        .collect();
    core.command(AppCommand::RemoveOfflinePlaylists(vec![saved.chat]));
    assert!(attempts[&selected.id()].cancel.is_cancelled());
    assert!(!attempts[&other.id()].cancel.is_cancelled());
    core.event(AppEvent::OfflineDownloaded {
        request: attempts[&selected.id()].key(),
        id: selected.id(),
        result: Ok("late".into()),
    });
    let mut reopened = AppCore::with_offline(State::default(), core.offline().data());
    assert!(reopened.offline().removing.contains_key("late"));
    assert!(reopened.offline().pending.contains_key(&other.id()));
    assert!(!reopened.offline().policies.contains_key(&saved.chat));
    assert!(reopened.offline().policies.contains_key(&other.chat));
    assert_eq!(reopened.offline().path(saved.id()), None);
    assert!(reopened.offline().copies[&saved.id()].cached);
    reopened.command(AppCommand::SaveTrack(saved.clone()));
    assert_eq!(reopened.offline().path(saved.id()), Some("saved"));
    assert!(!reopened.offline().pending.contains_key(&saved.id()));
}

#[test]
fn offline_plays_keep_the_same_file_fresh_and_clear_only_cached_audio() {
    use crate::offline::{OfflineCore, PublishedFile};
    let mut offline = OfflineCore::new(OfflineData::default());
    let [old, played, saved] = [track(1), track(2), track(3)];
    let now = 9 * 86_400;
    for (track, path) in [(&old, "old"), (&played, "played"), (&saved, "saved")] {
        offline.publish_file(
            track.clone(),
            PublishedFile {
                path: path.into(),
                size: 100,
                remote_id: None,
            },
            86_400,
        );
    }
    offline.mark_used(played.id(), "played", now - 60);
    offline.unpin(old.id(), now, 7);
    offline.unpin(played.id(), now, 7);
    assert!(offline.removing.contains_key("old"));
    assert_eq!(offline.local_path(played.id(), now, 7), Some("played"));
    assert_eq!(offline.copies[&played.id()].last_used, now - 60);
    offline.clear_cache(Some(900));
    assert_eq!(offline.local_path(played.id(), now, 7), Some("played"));
    offline.clear_cache(Some(42));
    assert!(offline.removing.contains_key("played"));
    assert_eq!(offline.path(saved.id()), Some("saved"));
}

#[test]
fn playlist_completion_follows_enabled_filters_and_saved_copies() {
    let first = track(1);
    let mut voice = track(2);
    voice.kind = gramwave_domain::MediaKind::Voice;
    let mut playlist = crate::model::Playlist {
        id: first.chat,
        filters: [true, false, false],
        done: [true; 3],
        tracks: vec![first.clone(), voice.clone()],
        ..Default::default()
    };
    let mut offline = crate::offline::OfflineCore::new(OfflineData::default());
    offline.publish(first, "music".into());
    assert!(offline.playlist_complete(&playlist));
    playlist.filters[2] = true;
    assert!(!offline.playlist_complete(&playlist));
    offline.publish(voice, "voice".into());
    assert!(offline.playlist_complete(&playlist));
}

#[test]
fn group_queue_edits_persist_without_player_effects() {
    use gramwave_domain::playback::QueueRangeEdit;

    let mut state = State::default();
    state
        .session
        .start(&[track(1), track(2), track(3), track(4)], 1, false);
    state.session.set_position_ms(27_000);
    let current = state.session.current().unwrap().entry_id;
    let expected = (0..4)
        .map(|i| state.session.timeline().get(i).unwrap().entry_id)
        .collect();
    let mut core = AppCore::new(state);
    core.playback.state = crate::playback::PlaybackState::Playing;
    let effects = core.command(AppCommand::EditQueueRange {
        start: 0,
        expected,
        edit: QueueRangeEdit::Reverse,
    });
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::Player(_) | Effect::PreparePlayback { .. })));
    let session = &core.view().session;
    assert_eq!(session.current().unwrap().entry_id, current);
    assert_eq!(session.position_ms(), 27_000);
    assert_eq!(session.timeline().current_index(), Some(2));
    assert!(effects.iter().any(|effect| matches!(effect, Effect::Persist(changes) if changes.session.as_ref() == Some(session))));
}

#[test]
fn undo_current_loads_its_checkpoint_with_the_original_pause_state() {
    use crate::playback::PlaybackState;
    let mut state = State::default();
    state.session.start(&[track(1), track(2)], 0, false);
    let old = state.session.current().unwrap().entry_id;
    let now = Instant::now();
    let mut core = AppCore::new(state);
    core.playback.state = PlaybackState::Paused;
    core.playback.target = core.view().session.current().map(Into::into);
    core.event(AppEvent::Snapshot(PlayerSnapshot {
        checkpoint_position: Some(23.5),
        duration: 100.,
        ..Default::default()
    }));

    core.command(AppCommand::RemoveQueueRows {
        expected: vec![(0, old)],
        now,
    });
    assert_eq!(core.view().session.current().unwrap().message, 2);
    assert!(core.playback.state().paused());
    let id = core.playback().queue_removals(now).next().unwrap().id;
    let effects = core.command(AppCommand::UndoQueueRemoval { id, now });
    assert_eq!(core.view().session.current().unwrap().entry_id, old);
    let (request, revision, restored) = effects
        .into_iter()
        .find_map(|effect| {
            if let Effect::PreparePlayback {
                operation,
                revision,
                track,
                ..
            } = effect
            {
                Some((operation.key(), revision, track))
            } else {
                None
            }
        })
        .unwrap();
    let effects = core.event(AppEvent::PlaybackPrepared {
        request,
        revision,
        result: Ok(PreparedPlayback {
            track: restored.into(),
            path: Some("restored.ogg".into()),
        }),
    });
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Player(PlayerCommand::Load {
            start_ms: 23_500,
            ..
        })
    )));
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::Player(PlayerCommand::Pause(true)))));
}

#[test]
fn streamed_advance_uses_the_reserved_shuffle_cycle() {
    let mut state = State {
        repeat: gramwave_domain::RepeatMode::All,
        shuffle: true,
        ..State::default()
    };
    state
        .session
        .start(&[track(1), track(2), track(3)], 2, false);
    let mut core = AppCore::new(state);
    core.enable_streamed_next(true);
    core.playback.state = crate::playback::PlaybackState::Playing;
    let (next, revision, _) = core.next_streamed_playback().unwrap();
    assert_ne!(next.message, 3);
    assert_eq!(
        core.next_streamed_playback().unwrap().0.entry_id,
        next.entry_id
    );
    let effects = core.event(AppEvent::Player(PlaybackEvent::Advanced(QueuedPlayback {
        entry_id: next.entry_id,
        revision,
        path: "http://127.0.0.1:1/token/1".into(),
    })));
    assert_eq!(
        core.view().session.current().unwrap().entry_id,
        next.entry_id
    );
    assert_eq!(core.view().session.remaining().len(), 2);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::PlaybackAdvanced(_))));
}

#[test]
fn streamed_repeat_one_plans_the_current_occurrence() {
    let mut state = State {
        repeat: gramwave_domain::RepeatMode::One,
        ..State::default()
    };
    state
        .session
        .start(&[track(1), track(1), track(2)], 0, false);
    let mut core = AppCore::new(state);
    core.enable_streamed_next(true);
    core.playback.state = crate::playback::PlaybackState::Playing;
    let (next, revision, _) = core.next_streamed_playback().unwrap();
    let current = core.view().session.current().unwrap().entry_id;
    assert_eq!(next.entry_id, current);
    let effects = core.event(AppEvent::Player(PlaybackEvent::Advanced(QueuedPlayback {
        entry_id: current,
        revision,
        path: "http://127.0.0.1:1/token/1".into(),
    })));
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::PlaybackAdvanced(_))));
    assert_eq!(core.view().session.current().unwrap().entry_id, current);
    let remaining = core.view().session.remaining()[0].entry_id;
    core.command(AppCommand::Next);
    assert_eq!(core.view().session.current().unwrap().entry_id, remaining);
}

#[test]
fn manual_next_promotes_once_and_disregards_superseded_completion() {
    let mut state = State::default();
    state
        .session
        .start(&[track(1), track(2), track(3)], 0, false);
    let mut core = AppCore::new(state);
    core.enable_streamed_next(true);
    core.playback.state = crate::playback::PlaybackState::Playing;
    let effects = core.command(AppCommand::Next);
    let (request, next) = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Player(PlayerCommand::PromoteNext {
                request,
                entry_id,
                revision,
            }) => Some((
                *request,
                QueuedPlayback {
                    entry_id: *entry_id,
                    revision: *revision,
                    path: "next.ogg".into(),
                },
            )),
            _ => None,
        })
        .unwrap();
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::Player(PlayerCommand::Stop))));
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::Notice(crate::notice::Notice::LoadingTrack))));
    assert!(!core.playback.shows_loading());
    core.event(AppEvent::Player(PlaybackEvent::Finished));
    core.event(AppEvent::Player(PlaybackEvent::NextPromoted(
        request,
        next.clone(),
    )));
    assert_eq!(core.view().session.current().unwrap().message, 2);
    assert!(!core.playback.promoting_next());
    core.command(AppCommand::Next);
    assert!(!core.playback.shows_loading());
    core.event(AppEvent::Player(PlaybackEvent::NextBuffering(request)));
    assert!(!core.playback.shows_loading());
    let pending = core.playback.active.as_ref().unwrap().key();
    core.event(AppEvent::Player(PlaybackEvent::NextBuffering(pending)));
    assert!(core.playback.shows_loading());
    assert!(core
        .event(AppEvent::Player(PlaybackEvent::NextPromoted(request, next)))
        .is_empty());
    assert_eq!(core.view().session.current().unwrap().message, 3);
    core.command(AppCommand::Stop);
    assert!(!core.playback.shows_loading());
    assert!(core
        .event(AppEvent::Player(PlaybackEvent::NextUnavailable(request)))
        .is_empty());
}

#[test]
fn unavailable_promotion_prepares_the_selected_occurrence_once() {
    let mut state = State::default();
    state
        .session
        .start(&[track(1), track(2), track(3)], 0, false);
    let mut core = AppCore::new(state);
    core.enable_streamed_next(true);
    core.playback.state = crate::playback::PlaybackState::Playing;
    let request = core
        .command(AppCommand::Next)
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Player(PlayerCommand::PromoteNext { request, .. }) => Some(request),
            _ => None,
        })
        .unwrap();
    let selected = core.view().session.current().unwrap().entry_id;
    let effects = core.event(AppEvent::Player(PlaybackEvent::NextUnavailable(request)));
    let prepared: Vec<_> = effects
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::PreparePlayback { track, .. } => Some(track.entry_id),
            _ => None,
        })
        .collect();
    assert_eq!(prepared, [selected]);
    assert_eq!(core.view().session.current().unwrap().entry_id, selected);
    assert_eq!(core.view().session.remaining()[0].message, 3);
    assert!(core
        .event(AppEvent::Player(PlaybackEvent::NextUnavailable(request)))
        .is_empty());
}
