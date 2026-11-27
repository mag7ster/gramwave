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
        library_id,
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
    assert!(core.next_local_playback().is_none());
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
fn reverse_completes_the_chat_before_swapping_both_sides_of_the_current_track() {
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
    core.command(AppCommand::ReverseQueue);
    let request = page_request(&mut core, (42, 0, 75));
    assert!(core.queue_reversing());
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
fn finishing_during_a_reverse_waits_then_plays_the_reversed_history() {
    let mut core = core();
    let middle = core.view().playlists[0].tracks[1].library_id;
    start(&mut core, Some(middle));
    core.command(AppCommand::ReverseQueue);
    let request = page_request(&mut core, (42, 0, 75));
    core.event(AppEvent::Player(PlaybackEvent::Finished));
    assert!(core.queue_waiting());
    assert_eq!(current(&core), 90);
    page(&mut core, request, 0, &[70, 60]);
    assert_eq!(current(&core), 100);
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
        library_id: None,
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
