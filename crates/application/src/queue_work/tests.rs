use crate::{
    commands::AppCommand,
    model::{Playlist, State, Track},
    ports::{PlaybackEvent, PlayerCommand, PlayerSnapshot},
    tasks::Operation,
    telegram::MediaPage,
    workflow::{AppEvent, Effect},
    AppCore, AppError,
};
use gramwave_domain::{MediaKind, RepeatMode};
use std::time::Instant;

fn track(message: i64) -> Track {
    Track {
        chat: 42,
        message,
        ..Default::default()
    }
}

fn core() -> AppCore {
    AppCore::new(State {
        playlists: vec![
            Playlist {
                id: 42,
                tracks: vec![track(100), track(90), track(80)],
                filters: [true, false, false],
                cursors: [75, 0, 0],
                done: [false, true, true],
                ..Default::default()
            },
            Playlist {
                id: 77,
                ..Default::default()
            },
        ],
        ..Default::default()
    })
}

fn start(core: &mut AppCore, library_id: Option<u64>) {
    core.command(AppCommand::StartPlaylist {
        chat: 42,
        start: library_id.map_or(
            crate::commands::PlaylistStart::Ordered,
            crate::commands::PlaylistStart::Track,
        ),
        order: Default::default(),
    });
    core.event(AppEvent::Player(PlaybackEvent::Loaded));
}

fn page_request(core: &mut AppCore, expected: (i64, usize, i64)) -> Operation {
    let mut pages = core
        .pump(Instant::now(), true)
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::QueuePage {
                operation,
                chat,
                kind,
                cursor,
                ..
            } => {
                assert_eq!((chat, kind, cursor), expected);
                Some(operation)
            }
            _ => None,
        });
    let page = pages.next().expect("request the next queue page");
    assert!(pages.next().is_none());
    assert!(!core
        .pump(Instant::now(), true)
        .into_iter()
        .any(|effect| matches!(effect, Effect::QueuePage { .. })));
    page
}

fn page(core: &mut AppCore, operation: Operation, next: i64, tracks: &[i64]) -> Vec<Effect> {
    core.event(AppEvent::QueuePage {
        browsing: false,
        request: operation.key(),
        result: Ok(MediaPage {
            newest: tracks.iter().copied().max().unwrap_or_default(),
            tracks: tracks.iter().copied().map(|id| track(id).into()).collect(),
            next,
        }),
    })
}

fn current(core: &AppCore) -> i64 {
    core.view().session.current().unwrap().message
}

fn upcoming(core: &AppCore) -> Vec<i64> {
    core.view()
        .session
        .remaining()
        .iter()
        .map(|t| t.message)
        .collect()
}

#[test]
fn playback_from_a_row_waits_at_an_empty_page_and_manual_tracks_follow_the_chat() {
    let mut core = core();
    core.enable_streamed_next(true);
    let last = core.view().playlists[0].tracks[2].library_id;
    start(&mut core, Some(last));
    assert_eq!(current(&core), 80);
    core.command(AppCommand::Repeat(RepeatMode::All));
    core.command(AppCommand::Insert(track(500), false));
    core.command(AppCommand::Insert(track(501), true));
    assert_eq!(upcoming(&core), [501, 500]);
    assert_eq!(core.view().session.available_len(), 1);
    core.command(AppCommand::OpenPlaylist(77));
    core.command(AppCommand::Filters(42, [false, false, true]));
    core.event(AppEvent::Player(PlaybackEvent::Finished));
    assert_eq!(current(&core), 501);
    core.event(AppEvent::Player(PlaybackEvent::Loaded));
    assert!(core.next_streamed_playback().is_none());
    let request = page_request(&mut core, (42, 0, 75));
    core.event(AppEvent::Player(PlaybackEvent::Finished));
    assert!(core.queue_waiting());
    page(&mut core, request, 60, &[]);
    assert_eq!(current(&core), 501);
    assert!(core.queue_waiting());
    let request = page_request(&mut core, (42, 0, 60));
    page(&mut core, request, 0, &[50, 40]);
    assert_eq!(current(&core), 50);
    assert_eq!(upcoming(&core), [40, 500]);
    assert!(!core.queue_waiting());
}

#[test]
fn reverse_swaps_immediately_and_an_in_flight_page_fills_the_new_top() {
    let mut core = core();
    let middle = core.view().playlists[0].tracks[1].library_id;
    start(&mut core, Some(middle));
    core.command(AppCommand::Insert(track(500), false));
    core.save_playback_position(PlayerSnapshot {
        duration: 100.,
        checkpoint_position: Some(42.),
        ..Default::default()
    });
    let id = core.view().session.current().unwrap().entry_id;
    let request = page_request(&mut core, (42, 0, 75));
    core.command(AppCommand::ReverseQueue);
    assert_eq!(upcoming(&core), [100]);
    assert_eq!(core.view().session.timeline().current_index(), Some(2));
    assert!(!request.cancel.is_cancelled());
    let effects = page(&mut core, request, 0, &[70, 60]);
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::PreparePlayback { .. } | Effect::Player(PlayerCommand::Stop)
    )));
    let session = &core.view().session;
    assert_eq!(session.current().unwrap().entry_id, id);
    assert_eq!(session.position_ms(), 42_000);
    let timeline = session.timeline();
    assert_eq!(
        (0..timeline.len())
            .map(|i| timeline.get(i).unwrap().message)
            .collect::<Vec<_>>(),
        [500, 60, 70, 80, 90, 100]
    );
    assert_eq!(timeline.current_index(), Some(4));
    assert_eq!(upcoming(&core), [100]);
    core.command(AppCommand::Previous);
    assert_eq!(current(&core), 80);
}

#[test]
fn reversing_a_new_shuffle_moves_its_unloaded_boundary_above_the_current_track() {
    let mut core = core();
    core.command(AppCommand::StartPlaylist {
        chat: 42,
        start: crate::commands::PlaylistStart::Shuffled,
        order: Default::default(),
    });
    core.event(AppEvent::Player(PlaybackEvent::Loaded));
    core.command(AppCommand::Insert(track(500), false));
    let first = current(&core);
    let future = upcoming(&core);
    core.command(AppCommand::ReverseQueue);
    assert!(upcoming(&core).is_empty());
    assert!(core.pump(Instant::now(), true).is_empty());
    core.command(AppCommand::Next);
    assert_eq!(current(&core), first);
    assert!(!core.queue_waiting());

    let restored = serde_json::from_value::<gramwave_domain::Session>(
        serde_json::to_value(&core.view().session).unwrap(),
    )
    .unwrap();
    assert_eq!(restored.page_request(true), Some((42, 0, 75)));
    assert_eq!(restored.page_request(false), None);
    core.command(AppCommand::ReverseQueue);
    assert_eq!(upcoming(&core), future);
    core.command(AppCommand::ReverseQueue);

    for message in future.into_iter().filter(|&message| message != 500) {
        core.command(AppCommand::Previous);
        assert_eq!(current(&core), message);
    }
    let request = page_request(&mut core, (42, 0, 75));
    core.command(AppCommand::Previous);
    assert!(core.queue_waiting());
    page(&mut core, request, 65, &[70]);
    assert_eq!(current(&core), 70);
    assert!(!core.queue_waiting());
    assert_eq!(core.view().session.timeline().get(0).unwrap().message, 500);
}

#[test]
fn repeat_all_can_wrap_at_a_reversed_edge_without_draining_pages_from_above() {
    let mut core = core();
    start(&mut core, None);
    core.command(AppCommand::Repeat(RepeatMode::All));
    core.command(AppCommand::ReverseQueue);
    assert!(upcoming(&core).is_empty());
    core.command(AppCommand::Next);
    assert_eq!(current(&core), 80);
    assert_eq!(upcoming(&core), [90, 100]);
    assert!(core.pump(Instant::now(), true).is_empty());
}

#[test]
fn retries_keep_the_same_cursor_and_a_late_page_cannot_repopulate_a_cleared_queue() {
    let mut core = core();
    start(&mut core, None);
    let request = page_request(&mut core, (42, 0, 75));
    core.event(AppEvent::QueuePage {
        browsing: false,
        request: request.key(),
        result: Err(AppError::Unavailable("Telegram")),
    });
    assert!(core.queue_page_error().is_some());
    assert_eq!(upcoming(&core), [90, 80]);
    core.command(AppCommand::RetryQueue);
    let request = page_request(&mut core, (42, 0, 75));
    core.command(AppCommand::Clear);
    assert!(request.cancel.is_cancelled());
    page(&mut core, request, 0, &[60]);
    assert!(upcoming(&core).is_empty());
    assert!(core.view().session.playlist_page_request().is_none());
}

#[test]
fn continuation_restores_buffered_pages_and_deferred_tracks() {
    let mut core = core();
    core.state.playlists[0].filters = [true, true, false];
    core.state.playlists[0].cursors[1] = 65;
    core.state.playlists[0].done[1] = false;
    start(&mut core, None);
    core.command(AppCommand::Insert(track(500), false));
    let request = page_request(&mut core, (42, 0, 75));
    page(&mut core, request, 40, &[70, 60, 50]);
    assert_eq!(upcoming(&core), [90, 80, 70, 500]);
    let mut state = core.view().clone();
    state.session = serde_json::from_value(serde_json::to_value(&state.session).unwrap()).unwrap();
    let mut core = AppCore::new(state);
    let request = page_request(&mut core, (42, 1, 65));
    core.event(AppEvent::QueuePage {
        browsing: false,
        request: request.key(),
        result: Ok(MediaPage {
            newest: 64,
            tracks: vec![Track {
                kind: MediaKind::Document,
                ..track(64)
            }
            .into()],
            next: 0,
        }),
    });
    assert_eq!(upcoming(&core), [90, 80, 70, 64, 60, 50, 500]);
    assert_eq!(core.view().session.available_len(), 6);
}

#[test]
fn removing_and_restoring_rows_keeps_the_history_boundary() {
    let mut core = core();
    start(&mut core, None);
    core.command(AppCommand::Insert(track(500), false));
    let timeline = core.view().session.timeline();
    let expected = [2, 3].map(|i| (i, timeline.get(i).unwrap().entry_id));
    let now = Instant::now();
    core.command(AppCommand::RemoveQueueRows {
        expected: expected.into(),
        now,
    });
    let undo = core.playback().queue_removals(now).next().unwrap().id;
    let request = page_request(&mut core, (42, 0, 75));
    page(&mut core, request, 55, &[70, 60]);
    assert_eq!(upcoming(&core), [90, 70, 60]);
    core.command(AppCommand::UndoQueueRemoval { id: undo, now });
    assert_eq!(upcoming(&core), [90, 80, 70, 60, 500]);
    assert_eq!(core.view().session.available_len(), 4);
}

#[test]
fn finishing_after_reverse_uses_the_reversed_history_without_waiting_for_the_other_side() {
    let mut core = core();
    let middle = core.view().playlists[0].tracks[1].library_id;
    start(&mut core, Some(middle));
    core.command(AppCommand::ReverseQueue);
    assert!(core.pump(Instant::now(), true).is_empty());
    core.event(AppEvent::Player(PlaybackEvent::Finished));
    assert_eq!(current(&core), 100);
    assert!(!core.queue_waiting());
}

#[test]
fn reverse_resumes_playback_waiting_for_a_forward_page() {
    for manual_next in [false, true] {
        let mut core = core();
        let last = core.view().playlists[0].tracks[2].library_id;
        start(&mut core, Some(last));
        if manual_next {
            core.command(AppCommand::Repeat(RepeatMode::One));
            core.command(AppCommand::Next);
        }
        let request = page_request(&mut core, (42, 0, 75));
        core.event(AppEvent::Player(PlaybackEvent::Finished));
        assert!(core.queue_waiting());
        assert_eq!(current(&core), 80);

        let effects = core.command(AppCommand::ReverseQueue);
        let prepared: Vec<_> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::PreparePlayback { track, .. } => Some(track.message),
                _ => None,
            })
            .collect();
        assert_eq!(prepared, [90]);
        assert_eq!(current(&core), 90);
        assert!(!core.queue_waiting());

        assert!(!request.cancel.is_cancelled());
        let effects = page(&mut core, request, 0, &[70]);
        assert!(!effects
            .iter()
            .any(|effect| matches!(effect, Effect::PreparePlayback { .. })));
        assert_eq!(current(&core), 90);
        assert_eq!(core.view().session.timeline().get(0).unwrap().message, 70);
    }
}

#[test]
fn reverse_stops_at_an_exhausted_forward_edge_after_finishing() {
    let mut core = core();
    core.state.playlists[0].tracks.truncate(1);
    start(&mut core, None);
    core.event(AppEvent::Player(PlaybackEvent::Finished));
    assert!(core.queue_waiting());

    let effects = core.command(AppCommand::ReverseQueue);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::Player(PlayerCommand::Stop))));
    assert_eq!(
        core.playback().state(),
        crate::playback::PlaybackState::Stopped
    );
    assert!(!core.queue_waiting());
}

#[test]
fn finishing_while_next_waits_does_not_turn_it_into_repeat_one() {
    let mut core = core();
    let last = core.view().playlists[0].tracks[2].library_id;
    start(&mut core, Some(last));
    core.command(AppCommand::Repeat(RepeatMode::One));
    core.command(AppCommand::Next);
    assert!(core.queue_waiting());
    let request = page_request(&mut core, (42, 0, 75));
    core.event(AppEvent::Player(PlaybackEvent::Finished));
    page(&mut core, request, 0, &[70]);
    assert_eq!(current(&core), 70);
}

#[test]
fn oldest_start_waits_for_the_verified_prefix_then_queues_newer_pages() {
    use crate::library::LibraryCommand;
    use gramwave_domain::PlaylistOrder;

    let mut core = core();
    start(&mut core, None);
    core.library_command(LibraryCommand::Connected(1));
    let effects = core.command(AppCommand::StartPlaylist {
        chat: 42,
        start: crate::commands::PlaylistStart::Ordered,
        order: PlaylistOrder::OldestFirst,
    });
    assert_eq!(current(&core), 100);
    let request = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::QueuePage {
                operation,
                browsing: true,
                cursor: 0,
                ..
            } => Some(operation.key()),
            _ => None,
        })
        .unwrap();
    core.event(AppEvent::QueuePage {
        browsing: true,
        request,
        result: Ok(MediaPage {
            newest: 30,
            tracks: vec![track(30).into(), track(20).into()],
            next: 30,
        }),
    });
    assert_eq!(current(&core), 20);
    assert_eq!(upcoming(&core), [30]);
    assert_eq!(
        core.view().session.playlist_order(),
        PlaylistOrder::OldestFirst
    );
    let request = page_request(&mut core, (42, 0, 30));
    page(&mut core, request, 0, &[45, 80, 90, 100]);
    assert_eq!(upcoming(&core), [30, 45, 80, 90, 100]);
    let restored: gramwave_domain::Session =
        serde_json::from_value(serde_json::to_value(&core.view().session).unwrap()).unwrap();
    assert_eq!(restored.playlist_order(), PlaylistOrder::OldestFirst);
    core.command(AppCommand::ReverseQueue);
    assert_eq!(
        core.view().session.playlist_order(),
        PlaylistOrder::NewestFirst
    );
    core.command(AppCommand::ReverseQueue);
    assert_eq!(
        core.view().session.playlist_order(),
        PlaylistOrder::OldestFirst
    );
    start(&mut core, None);
    assert_eq!(
        core.view().session.playlist_order(),
        PlaylistOrder::NewestFirst
    );
}

fn shuffle_request(effects: Vec<Effect>) -> crate::tasks::RequestId {
    effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ShuffleSample { operation, .. } => Some(operation.key()),
            _ => None,
        })
        .unwrap()
}

fn shuffle_start() -> AppCommand {
    AppCommand::StartPlaylist {
        chat: 42,
        start: crate::commands::PlaylistStart::Shuffled,
        order: Default::default(),
    }
}

fn deep_shuffle_sample() -> crate::telegram::ShuffleSample {
    crate::telegram::ShuffleSample {
        ranks: [
            (300, vec![(280, 20), (250, 50)]),
            Default::default(),
            Default::default(),
        ],
        tracks: [20, 50, 90].map(|message| track(message).into()).to_vec(),
    }
}

fn prepares(effects: &[Effect], message: i64) -> bool {
    effects.iter().any(|effect| matches!(effect, Effect::PreparePlayback { track, .. } if track.message == message))
}

#[test]
fn warmed_and_pending_shuffle_both_play_the_deep_sample_first_and_replenish() {
    use crate::library::LibraryCommand;
    use gramwave_domain::PlaylistOrder;
    for order in [PlaylistOrder::NewestFirst, PlaylistOrder::OldestFirst] {
        for warm in [false, true] {
            let mut core = core();
            let playlist = &mut core.state.playlists[0];
            playlist.tracks = (80..180).rev().map(track).collect();
            playlist.ensure_entry_ids();
            start(&mut core, None);
            core.library_command(LibraryCommand::Connected(1));
            let request = shuffle_request(core.command(AppCommand::OpenPlaylist(42)));
            let start = || AppCommand::StartPlaylist {
                chat: 42,
                start: crate::commands::PlaylistStart::Shuffled,
                order,
            };
            let reply = || AppEvent::ShuffleSample {
                request,
                result: Ok(deep_shuffle_sample()),
            };
            let effects = if warm {
                assert!(core.event(reply()).is_empty());
                core.command(start())
            } else {
                for _ in 0..3 {
                    assert!(core.command(start()).is_empty());
                    assert!(core.shuffle_start_loading(42));
                    assert_eq!(current(&core), 179);
                }
                core.event(reply())
            };
            assert!(!core.shuffle_start_loading(42));
            assert!(prepares(&effects, 20));
            assert_eq!(current(&core), 20);
            assert_eq!(upcoming(&core)[..2], [50, 90]);
            assert!(effects.iter().any(|effect| matches!(
                effect,
                Effect::ShuffleSample {
                    ranks: Some(_),
                    range: None,
                    ..
                }
            )));
            let cursor = if order == PlaylistOrder::NewestFirst {
                75
            } else {
                0
            };
            while core.view().session.available_len() > crate::page_browse::QUEUE_PREFETCH_ROWS {
                core.command(AppCommand::Next);
            }
            let next_page = page_request(&mut core, (42, 0, cursor));
            page(&mut core, next_page, 0, &[50, 20, 10]);
            let session = &core.view().session;
            for message in [20, 50] {
                assert_eq!(
                    session
                        .snapshot()
                        .iter()
                        .filter(|row| row.message == message)
                        .count(),
                    1
                );
            }
        }
    }
}

#[test]
fn shuffle_start_falls_back_on_error_empty_response_or_deadline() {
    use crate::library::LibraryCommand;
    for outcome in [
        Some(Err(AppError::Unavailable("Telegram"))),
        Some(Ok(crate::telegram::ShuffleSample {
            ranks: Default::default(),
            tracks: Vec::new(),
        })),
        None,
    ] {
        let mut core = core();
        core.library_command(LibraryCommand::Connected(1));
        let request = shuffle_request(core.command(shuffle_start()));
        assert!(core.view().session.current().is_none());
        let effects = if let Some(result) = outcome {
            core.event(AppEvent::ShuffleSample { request, result })
        } else {
            let deadline = core.shuffle_start_deadline().unwrap();
            let effects = core.pump(deadline, true);
            assert!(core
                .event(AppEvent::ShuffleSample {
                    request,
                    result: Ok(deep_shuffle_sample()),
                })
                .is_empty());
            effects
        };
        assert!([100, 90, 80].contains(&current(&core)));
        assert!(prepares(&effects, current(&core)));
        assert!(core.shuffle_start_deadline().is_none());
    }
}

#[test]
fn late_shuffle_reply_cannot_replace_playback_after_cancellation() {
    use crate::library::LibraryCommand;
    for command in [
        AppCommand::Stop,
        AppCommand::OpenPlaylist(77),
        AppCommand::Filters(42, [false; 3]),
        AppCommand::CancelPlaylistStart(42),
        AppCommand::Play(track(100)),
    ] {
        let mut core = core();
        core.library_command(LibraryCommand::Connected(1));
        let request = shuffle_request(core.command(AppCommand::OpenPlaylist(42)));
        assert!(core.command(shuffle_start()).is_empty());
        core.command(command);
        assert!(!core.shuffle_start_loading(42));
        let effects = core.event(AppEvent::ShuffleSample {
            request,
            result: Ok(deep_shuffle_sample()),
        });
        assert!(!prepares(&effects, 20));
        assert!(core
            .view()
            .session
            .current()
            .is_none_or(|track| track.message != 20));
    }
}

#[test]
fn advancing_prefetches_without_a_queue_view_and_retries_until_the_buffer_recovers() {
    let buffer = crate::page_browse::QUEUE_PREFETCH_ROWS;
    for (advance, shuffle) in [
        (AppCommand::Next, false),
        (AppCommand::Advance(true), false),
        (AppCommand::Next, true),
    ] {
        let mut core = core();
        core.state.playlists[0].tracks = (100..102 + buffer as i64).rev().map(track).collect();
        core.state.playlists[0].ensure_entry_ids();
        start(&mut core, None);
        if shuffle {
            core.command(AppCommand::Shuffle(true));
        }
        core.command(AppCommand::Insert(track(500), false));
        let now = Instant::now();
        assert_eq!(core.view().session.available_len(), buffer + 1);
        assert!(core.pump(now, true).is_empty());

        core.command(advance);
        assert_eq!(core.view().session.available_len(), buffer);
        let request = page_request(&mut core, (42, 0, 75));
        core.event(AppEvent::QueuePage {
            browsing: false,
            request: request.key(),
            result: Err(AppError::Unavailable("Telegram")),
        });
        let retry_at = core.page_retry_deadline(now, true).unwrap();
        assert!(core.pump(now, true).is_empty());
        let retry = core
            .pump(retry_at, true)
            .into_iter()
            .find_map(|effect| match effect {
                Effect::QueuePage {
                    operation,
                    cursor: 75,
                    ..
                } => Some(operation),
                _ => None,
            })
            .unwrap();
        page(&mut core, retry, 65, &[70, 69]);
        assert_eq!(core.view().session.available_len(), buffer + 2);
        assert_eq!(upcoming(&core).last(), Some(&500));
        assert!(core.pump(retry_at, true).is_empty());
    }
}

#[test]
fn scrolling_loads_one_page_and_retries_only_while_requested() {
    use crate::page_browse::Browse;

    let mut core = core();
    core.state.playlists[0].tracks = (80..110).rev().map(track).collect();
    core.state.playlists[0].ensure_entry_ids();
    start(&mut core, None);
    core.command(AppCommand::Pause(true));
    core.command(AppCommand::Shuffle(true));
    core.command(AppCommand::Insert(track(500), false));
    let current_id = core.view().session.current().unwrap().entry_id;
    let queued = upcoming(&core);
    let now = Instant::now();
    assert!(core.pump(now, true).is_empty());
    assert!(core.browse_queue(Browse::Down));
    core.finish_page_browse_frame();
    let request = page_request(&mut core, (42, 0, 75));
    core.event(AppEvent::QueuePage {
        browsing: false,
        request: request.key(),
        result: Err(AppError::Unavailable("Telegram")),
    });
    let deadline = core.page_retry_deadline(now, true).unwrap();
    assert!(core.pump(now, true).is_empty());
    core.browse_queue(Browse::Away);
    core.finish_page_browse_frame();
    assert!(core.pump(deadline, true).is_empty());
    assert!(core.browse_queue(Browse::Down));
    core.finish_page_browse_frame();
    let retry = core
        .pump(deadline, true)
        .into_iter()
        .find_map(|effect| match effect {
            Effect::QueuePage {
                operation,
                cursor: 75,
                browsing: false,
                ..
            } => Some(operation),
            _ => None,
        })
        .unwrap();
    page(&mut core, retry, 65, &[70, 69]);
    assert_eq!(core.view().session.current().unwrap().entry_id, current_id);
    assert!(core.playback().state().paused());
    let after = upcoming(&core);
    assert_eq!(&after[..queued.len() - 1], &queued[..queued.len() - 1]);
    assert!(after.contains(&70) && after.contains(&69));
    assert_eq!(after.last(), Some(&500));
    core.browse_queue(Browse::Nearby);
    core.finish_page_browse_frame();
    assert!(core.pump(deadline, true).is_empty());
    core.browse_queue(Browse::Down);
    core.finish_page_browse_frame();
    let more = page_request(&mut core, (42, 0, 65));
    page(&mut core, more, 60, &[]);
    assert!(core.pump(deadline, true).is_empty());
}

#[test]
fn playlist_browse_cancels_automatic_retry_when_the_pane_closes() {
    use crate::{
        library::{LibraryCommand, LibraryEffect, LibraryEvent},
        page_browse::{Browse, PageStatus},
    };
    use gramwave_domain::PlaylistOrder::NewestFirst;

    let mut core = core();
    core.library_command(LibraryCommand::Connected(1));
    core.browse_playlist(42, NewestFirst, [true, false, false], Browse::Down);
    core.finish_page_browse_frame();
    let now = Instant::now();
    let request = core
        .pump(now, true)
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Library(LibraryEffect::Index {
                operation,
                refresh: false,
                ..
            }) => Some(operation.key()),
            _ => None,
        })
        .unwrap();
    assert!(core
        .event(AppEvent::Library(LibraryEvent::Indexed {
            request,
            result: Err(AppError::Timeout("Telegram")),
        }))
        .is_empty());
    assert_eq!(
        core.playlist_page_status(42, NewestFirst),
        PageStatus::Loading
    );
    let deadline = core.page_retry_deadline(now, true).unwrap();
    core.finish_page_browse_frame();
    assert!(core.pump(deadline, true).is_empty());
    assert_eq!(core.playlist_page_status(42, NewestFirst), PageStatus::Idle);
}

#[test]
fn unshuffle_requests_neighbors_at_the_current_sample_and_rejects_stale_pages() {
    use gramwave_domain::PlaylistOrder;
    for order in [PlaylistOrder::NewestFirst, PlaylistOrder::OldestFirst] {
        let mut core = core();
        let (loaded, cursor, anchor, prior, next, far_before, far_after) = match order {
            PlaylistOrder::NewestFirst => ([100, 95], 90, 40, 45, 35, 60, 20),
            PlaylistOrder::OldestFirst => ([10, 15], 20, 60, 55, 65, 40, 80),
        };
        let mut session = gramwave_domain::Session::default();
        session.start_shuffled_playlist(
            loaded.map(|id| track(id).into()).to_vec(),
            [Some(cursor), None, None],
            order,
        );
        session.add_shuffle_sample([track(anchor).into()], true);
        session.set_position_ms(20_000);
        core.state.session = session;
        core.state.shuffle = true;
        let effects = core.command(AppCommand::Shuffle(false));
        assert!(!effects.iter().any(|effect| matches!(
            effect,
            Effect::PreparePlayback { .. } | Effect::Player(PlayerCommand::Stop)
        )));
        core.state.session =
            serde_json::from_value(serde_json::to_value(&core.view().session).unwrap()).unwrap();
        assert_eq!(current(&core), anchor);
        assert_eq!(core.view().session.position_ms(), 20_000);
        assert!(core.view().session.remaining().is_empty());
        core.command(AppCommand::Insert(track(next), false));

        let request = |core: &mut AppCore, wanted_order, wanted_cursor| {
            core.pump(Instant::now(), true)
                .into_iter()
                .find_map(|effect| match effect {
                    Effect::QueuePage {
                        operation,
                        cursor,
                        order,
                        browsing: false,
                        ..
                    } if order == wanted_order && cursor == wanted_cursor => Some(operation),
                    _ => None,
                })
                .expect("request the adjacent section")
        };
        core.command(AppCommand::Previous);
        assert_eq!(current(&core), anchor);
        let backward = request(&mut core, order.reversed(), anchor);
        page(&mut core, backward, far_before, &[prior]);
        assert_eq!(current(&core), prior);
        core.command(AppCommand::Next);
        assert_eq!(current(&core), anchor);
        core.command(AppCommand::Next);
        assert!(core.queue_waiting());
        let forward = request(&mut core, order, anchor);
        page(&mut core, forward, far_after, &[next]);
        assert_eq!(current(&core), next);
        assert_eq!(
            core.view()
                .session
                .snapshot()
                .iter()
                .filter(|entry| entry.message == next)
                .count(),
            2
        );

        let on_again = request(&mut core, order, far_after);
        core.command(AppCommand::Shuffle(true));
        assert!(on_again.cancel.is_cancelled());
        let before = upcoming(&core);
        page(&mut core, on_again, 0, &[far_after]);
        assert_eq!(upcoming(&core), before);
        assert!(core.command(AppCommand::Shuffle(true)).is_empty());
        assert_eq!(upcoming(&core), before);
        assert!(before.contains(&anchor));
        assert_eq!(core.view().session.timeline().current_index(), Some(0));

        core.browse_queue(crate::page_browse::Browse::Down);
        let full = request(&mut core, order, cursor);
        page(&mut core, full, 0, &[prior, anchor, next, far_after]);
        let snapshot = core.view().session.snapshot();
        assert_eq!(
            snapshot
                .iter()
                .filter(|entry| entry.message == anchor)
                .count(),
            1
        );
        assert_eq!(
            snapshot
                .iter()
                .filter(|entry| entry.message == prior)
                .count(),
            1
        );
        assert_eq!(
            snapshot
                .iter()
                .filter(|entry| entry.message == next)
                .count(),
            2
        );
    }
}
