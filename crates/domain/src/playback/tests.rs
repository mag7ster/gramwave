use super::*;
#[cfg(test)]
mod regression {
    use super::*;
    use crate::Track;
    fn tracks() -> Vec<Track> {
        (0..5)
            .map(|message| Track {
                message,
                chat: 7,
                ..Default::default()
            })
            .collect()
    }
    fn order(s: &Session) -> Vec<i64> {
        s.remaining().iter().map(|t| t.message).collect()
    }
    #[test]
    fn timeline_moves_every_role_and_keeps_the_current_track_and_position() {
        for (from, to, current, upcoming) in [
            (0, 3, 1, vec![3, 0, 4]),
            (4, 1, 3, vec![3]),
            (2, 0, 0, vec![0, 1, 3, 4]),
            (2, 4, 4, vec![]),
        ] {
            let mut s = Session::default();
            let playing = s.start(&tracks(), 2, false).unwrap().entry_id;
            s.set_position_ms(42_000);
            let id = s.timeline().get(from).unwrap().entry_id;
            s.move_timeline_row(from, id, to);
            let restored: Session =
                serde_json::from_value(serde_json::to_value(s).unwrap()).unwrap();
            assert_eq!(restored.timeline().current_index(), Some(current));
            assert_eq!(restored.timeline().get(to).unwrap().entry_id, id);
            assert_eq!(restored.current().unwrap().entry_id, playing);
            assert_eq!(restored.position_ms(), 42_000);
            assert_eq!(order(&restored), upcoming);
        }
    }

    #[test]
    fn timeline_moves_a_repeat_appearance_independently() {
        let mut s = Session::default();
        let id = s.start(&tracks(), 0, false).unwrap().entry_id;
        for _ in 0..5 {
            s.advance(RepeatMode::All, false, true);
        }
        s.move_timeline_row(0, id, 6);
        assert_eq!(s.current().unwrap().entry_id, id);
        let moved = &s.remaining()[1];
        assert_eq!(moved.message, 0);
        assert_ne!(moved.entry_id, id);
        s.remove(1);
        assert_eq!(s.current().unwrap().entry_id, id);
        assert!(s.snapshot().iter().any(|entry| entry.entry_id == id));
    }

    #[test]
    fn range_edits_keep_the_current_occurrence_position_and_navigation() {
        let mut session = Session::default();
        let current = session.start(&tracks(), 1, false).unwrap().entry_id;
        session.set_position_ms(42_000);
        let original: Vec<_> = (0..5)
            .map(|i| session.timeline().get(i).unwrap().entry_id)
            .collect();
        assert!(!session.edit_timeline_range(0, &original[1..5], QueueRangeEdit::Reverse));
        assert!(session.edit_timeline_range(0, &original[..4], QueueRangeEdit::Reverse));
        let reversed = [original[3], original[2], current, original[0], original[4]];
        let mut session: Session =
            serde_json::from_slice(&serde_json::to_vec(&session).unwrap()).unwrap();
        assert!((0..5)
            .map(|i| session.timeline().get(i).unwrap().entry_id)
            .eq(reversed));
        assert_eq!(session.timeline().current_index(), Some(2));
        assert_eq!(session.current().unwrap().entry_id, current);
        assert_eq!(session.position_ms(), 42_000);

        let mut moved = session.clone();
        assert!(moved.edit_timeline_range(0, &reversed[..4], QueueRangeEdit::MoveDown));
        assert_eq!(moved.timeline().current_index(), Some(3));
        assert!(moved.edit_timeline_range(1, &reversed[..4], QueueRangeEdit::MoveUp));
        assert!((0..5)
            .map(|i| moved.timeline().get(i).unwrap().entry_id)
            .eq(reversed));
        assert_eq!(session.previous().unwrap().entry_id, original[2]);
        assert_eq!(
            session
                .advance(RepeatMode::Off, false, false)
                .unwrap()
                .entry_id,
            current
        );
        assert_eq!(
            session
                .advance(RepeatMode::Off, false, false)
                .unwrap()
                .entry_id,
            original[0]
        );
    }

    #[test]
    fn reversing_a_repeat_appearance_keeps_the_playing_occurrence_distinct() {
        let mut session = Session::default();
        let current = session.start(&tracks(), 0, false).unwrap().entry_id;
        for _ in 0..5 {
            session.advance(RepeatMode::All, false, true);
        }
        let ids: Vec<_> = (0..6)
            .map(|i| session.timeline().get(i).unwrap().entry_id)
            .collect();
        assert_eq!(ids[0], ids[5]);
        assert!(session.edit_timeline_range(0, &ids, QueueRangeEdit::Reverse));
        assert_eq!(session.timeline().current_index(), Some(0));
        assert_eq!(session.current().unwrap().entry_id, current);
        assert_eq!(session.remaining()[4].message, 0);
        assert_ne!(session.remaining()[4].entry_id, current);
        session.remove(4);
        assert!(session
            .snapshot()
            .iter()
            .any(|entry| entry.entry_id == current));
    }

    #[test]
    fn rewind_selects_an_exact_repeat_appearance_and_matches_previous() {
        let mut s = Session::default();
        s.start(&tracks(), 1, false);
        for _ in 0..13 {
            s.advance(RepeatMode::All, false, true);
        }
        for index in 0..s.timeline().past_len() {
            let mut actual = s.clone();
            let mut expected = s.clone();
            let id = s.timeline().get(index).unwrap().entry_id;
            while expected.timeline().past_len() > index {
                expected.previous().unwrap();
            }
            assert!(actual.play_timeline_row(index, QueueEntryId(0)).is_none());
            assert_eq!(actual.play_timeline_row(index, id).unwrap().entry_id, id);
            assert_eq!(actual, expected);
        }
    }
    #[test]
    fn previous_from_middle_survives_restart_and_returns_forward() {
        let mut s = Session::default();

        s.start(&tracks(), 3, false);
        let mut s: Session = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s.previous().unwrap().message, 2);
        assert_eq!(s.previous().unwrap().message, 1);
        assert_eq!(s.advance(RepeatMode::Off, false, false).unwrap().message, 2);
        assert_eq!(s.advance(RepeatMode::Off, false, false).unwrap().message, 3);
        assert_eq!(order(&s), [4]);
        s.clear();
        assert!(s.previous().is_none());
        s.start(&tracks(), 2, true);
        assert!(s.previous().is_none());
    }
    #[test]
    fn edits_duplicates_history_and_repeat() {
        let t = tracks();
        let mut s = Session::default();

        s.start(&t, 0, false);
        s.insert(t[2].clone(), true);
        s.insert(t[2].clone(), false);
        assert_eq!(order(&s), [2, 1, 2, 3, 4, 2]);
        assert_ne!(s.remaining()[0].entry_id, s.remaining()[2].entry_id);
        s.move_before(5, 1);
        s.remove(0);
        assert_eq!(order(&s), [2, 1, 2, 3, 4]);
        assert_eq!(s.jump_to(2).unwrap().message, 2);
        assert_eq!(s.previous().unwrap().message, 1);
        assert_eq!(s.advance(RepeatMode::Off, false, false).unwrap().message, 2);
        assert_eq!(s.history().len(), 3);
        s.clear();
        assert!(s.remaining().is_empty());
        assert_eq!(s.snapshot().len(), 1);
        assert!(s.advance(RepeatMode::Off, false, true).is_none());
        assert_eq!(s.advance(RepeatMode::One, false, true).unwrap().message, 2);
    }
    #[test]
    fn shuffle_edits_restore_and_persist() {
        let mut s = Session::default();
        s.start(&tracks(), 0, false);
        s.reorder(true);
        s.insert(tracks()[0].clone(), true);
        let id = s.remaining()[0].entry_id;
        s.reorder(false);
        assert_eq!(s.remaining()[0].entry_id, id);
        let removed = s.remaining()[1].entry_id;
        s.remove(1);
        let expected = order(&s);
        let mut restored: Session =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(order(&restored), expected);
        assert_eq!(restored.current(), s.current());

        for message in expected {
            assert_eq!(
                restored
                    .advance(RepeatMode::Off, false, true)
                    .unwrap()
                    .message,
                message
            );
        }
        restored.advance(RepeatMode::All, false, true);
        assert!(!restored.snapshot().iter().any(|t| t.entry_id == removed));
    }
    #[test]
    fn empty_single_repeats_and_replacement() {
        let mut s = Session::default();

        assert!(s.start::<Track>(&[], 0, false).is_none());
        assert!(s.advance(RepeatMode::All, true, true).is_none());
        s.insert(tracks()[0].clone(), false);
        assert!(s.current().is_none());
        assert_eq!(s.advance(RepeatMode::Off, false, false).unwrap().message, 0);
        assert_eq!(s.advance(RepeatMode::All, true, true).unwrap().message, 0);
        s.start(&tracks(), 3, false);
        assert!(s.history().is_empty());
        assert_eq!(order(&s), [4]);
        assert_eq!(s.advance(RepeatMode::One, false, true).unwrap().message, 3);
        assert_eq!(s.advance(RepeatMode::One, false, false).unwrap().message, 4);
        assert!(s.advance(RepeatMode::Off, false, true).is_none());
        for _ in 0..20 {
            let last = s.current().cloned().unwrap();
            let next = s.advance(RepeatMode::All, true, true).unwrap();
            if s.remaining().len() == s.snapshot().len() - 1 {
                assert!(!same(&last, &next));
            }
        }
    }
    #[test]
    fn legacy_import_preserves_visible_duplicates() {
        let mut s: Session = serde_json::from_value(
            serde_json::json!({"remaining":tracks()[..2],"next_cycle":tracks()[..1]}),
        )
        .unwrap();
        s.import(tracks()[..1].to_vec());
        assert_eq!(order(&s), [0, 0, 1, 0]);
        let mut ids: Vec<_> = s.remaining().iter().map(|t| t.entry_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 4);
        assert!(s.history().is_empty());
    }
    #[test]
    fn queue_jump_preserves_skipped_positions_and_reordered_duplicates() {
        for shuffle in [false, true] {
            let mut session = Session::default();

            session.start(&tracks(), 1, shuffle);
            session.insert(tracks()[2].clone(), true);
            session.insert(tracks()[2].clone(), false);
            let last = session.remaining().len() - 1;
            session.move_before(last, 1);
            session.remove(2);
            let original: Vec<_> = session
                .current()
                .into_iter()
                .chain(session.remaining().iter())
                .map(|t| t.entry_id)
                .collect();
            session.jump_to(session.remaining().len() - 1);
            let mut session: Session =
                serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
            for expected in original[..original.len() - 1].iter().rev() {
                assert_eq!(&session.previous().unwrap().entry_id, expected);
            }
            for expected in &original[1..] {
                assert_eq!(
                    &session
                        .advance(RepeatMode::Off, shuffle, false)
                        .unwrap()
                        .entry_id,
                    expected
                );
            }
            session.clear();
            assert!(session.previous().is_none());
        }
    }
    #[test]
    fn shuffle_start_has_no_predecessor_and_visits_every_other_entry_once() {
        let mut session = Session::default();

        session.start(&tracks(), 3, true);
        assert!(session.previous().is_none());
        let order = session.remaining().to_vec();
        let mut messages: Vec<_> = order.iter().map(|t| t.message).collect();
        messages.sort();
        assert_eq!(messages, [0, 1, 2, 4]);
        session.advance(RepeatMode::Off, true, false);
        assert_eq!(session.previous().unwrap().message, 3);
        assert_eq!(session.remaining(), order);
    }
}

#[test]
fn seeded_command_sequences_preserve_occurrence_ids_and_restart_state() {
    use rand::{Rng, SeedableRng};
    let tracks: Vec<_> = (0..20)
        .map(|message| crate::Track {
            chat: 42,
            message: message % 5,
            title: format!("Snapshot {message}"),
            ..Default::default()
        })
        .collect();
    let mut commands = rand::rngs::StdRng::seed_from_u64(23);
    let mut shuffle = rand::rngs::StdRng::seed_from_u64(57);
    let mut session = Session::default();
    session.start(&tracks, 3, false);
    for _ in 0..500 {
        match commands.gen_range(0..7) {
            0 => {
                session.insert(
                    tracks[commands.gen_range(0..tracks.len())].clone(),
                    commands.gen(),
                );
            }
            1 => {
                session.advance(RepeatMode::All, false, false);
            }
            2 => {
                session.previous();
            }
            3 => session.reorder_with(true, &mut shuffle),
            4 => session.reorder(false),
            5 if !session.remaining().is_empty() => {
                session.remove(commands.gen_range(0..session.remaining().len()))
            }
            _ if session.remaining().len() > 1 => session.move_before(0, session.remaining().len()),
            _ => {}
        }
        let restored: Session =
            serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        assert_eq!(restored, session);
        let ids: std::collections::HashSet<_> =
            session.snapshot().iter().map(|e| e.entry_id).collect();
        assert_eq!(ids.len(), session.snapshot().len());
    }
}

#[test]
fn legacy_snapshots_without_occurrence_ids_share_references_and_keep_duplicates() {
    let track = serde_json::json!({"chat":42,"message":7,"title":"Song"});
    let mut session:Session=serde_json::from_value(serde_json::json!({"snapshot":[track,track,track],"preceding":[track],"current":track,"remaining":[track]})).unwrap();
    assert_eq!(session.snapshot().len(), 3);
    assert_eq!(
        session.current().unwrap().entry_id,
        session.snapshot()[1].entry_id
    );
    assert_eq!(
        session.remaining()[0].entry_id,
        session.snapshot()[2].entry_id
    );
    assert_eq!(
        serde_json::to_value(&session).unwrap()["entries"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let first = session.snapshot()[0].entry_id;
    assert_eq!(session.previous().unwrap().entry_id, first);
    assert_eq!(
        session
            .advance(RepeatMode::Off, false, false)
            .unwrap()
            .entry_id,
        session.snapshot()[1].entry_id
    );
}

#[test]
fn shuffle_samples_rejoin_source_order_once_and_keep_manual_insertions() {
    fn track(message: i64) -> TrackMetadata {
        TrackMetadata {
            chat: 42,
            message,
            ..Default::default()
        }
    }
    let mut session = Session::default();
    session.start_shuffled_playlist(
        vec![track(100), track(95)],
        [Some(90), None, None],
        PlaylistOrder::NewestFirst,
    );
    session.add_shuffle_sample([track(50), track(20), track(95), track(80)], true);
    assert_eq!(session.current().unwrap().message, 50);
    session.insert(track(200), true);
    session.insert_many([track(201), track(202)], false);
    let mut session: Session =
        serde_json::from_value(serde_json::to_value(session).unwrap()).unwrap();
    session.reorder(false);
    assert_eq!(session.current().unwrap().message, 50);
    assert_eq!(
        session
            .remaining()
            .iter()
            .map(|t| t.message)
            .collect::<Vec<_>>(),
        [200, 100, 95, 201, 202]
    );
    session.add_playlist_page(0, 0, [85, 80, 70, 50, 30, 20, 10].map(track), false);
    assert_eq!(
        session
            .remaining()
            .iter()
            .map(|t| t.message)
            .collect::<Vec<_>>(),
        [200, 100, 95, 85, 80, 70, 30, 20, 10, 201, 202]
    );
    assert_eq!(
        session
            .snapshot()
            .iter()
            .filter(|t| t.message == 50)
            .count(),
        1
    );
}

#[test]
fn completed_playlist_starts_with_four_different_parts_in_both_orders() {
    for order in [PlaylistOrder::NewestFirst, PlaylistOrder::OldestFirst] {
        let mut tracks: Vec<_> = (1..=40)
            .map(|message| TrackMetadata {
                message,
                ..Default::default()
            })
            .collect();
        tracks.sort_by_key(|track| order.key(track.message));
        let mut session = Session::default();
        session.start_shuffled_playlist(tracks, [None; 3], order);
        let mut bands: Vec<_> = session
            .current()
            .into_iter()
            .chain(session.remaining().iter().take(3))
            .map(|entry| (entry.message - 1) / 10)
            .collect();
        bands.sort_unstable();
        assert_eq!(bands, [0, 1, 2, 3]);
        assert_eq!(session.remaining().len(), 39);
    }
}

#[test]
fn switching_shuffle_rebuilds_whole_queue_around_current_and_consumes_play_next() {
    fn track(message: i64) -> TrackMetadata {
        TrackMetadata {
            message,
            ..Default::default()
        }
    }
    let mut session = Session::default();
    session.start(&[1, 2, 3, 4, 5].map(track), 3, true);
    let current = session.current().unwrap().entry_id;
    session.set_position_ms(18_000);
    session.insert(track(10), true);
    session.insert(track(20), false);
    session.set_shuffle(false);
    assert_eq!(
        session
            .preceding()
            .iter()
            .map(|entry| entry.message)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        session
            .remaining()
            .iter()
            .map(|entry| entry.message)
            .collect::<Vec<_>>(),
        [10, 5, 20]
    );
    assert_eq!(session.current().unwrap().entry_id, current);
    assert_eq!(session.position_ms(), 18_000);
    session.set_shuffle(true);
    assert_eq!(session.timeline().current_index(), Some(0));
    let mut future: Vec<_> = session
        .remaining()
        .iter()
        .map(|entry| entry.message)
        .collect();
    future.sort_unstable();
    assert_eq!(future, [1, 2, 3, 5, 10, 20]);

    let mut restored: Session =
        serde_json::from_value(serde_json::to_value(&session).unwrap()).unwrap();
    restored.set_shuffle(false);
    assert_eq!(restored.remaining()[0].message, 10);
    assert_eq!(restored.previous().unwrap().message, 3);
    assert_eq!(
        restored
            .advance(RepeatMode::Off, false, false)
            .unwrap()
            .message,
        4
    );
    assert_eq!(
        restored
            .advance(RepeatMode::Off, false, false)
            .unwrap()
            .message,
        10
    );
    restored.set_shuffle(true);
    assert!(restored.remaining().iter().any(|entry| entry.message == 1));
    restored.set_shuffle(false);
    assert_eq!(restored.remaining()[0].message, 5);
}

#[test]
fn toggling_shuffle_keeps_legacy_rows_missing_from_the_original_order() {
    let mut session: Session = serde_json::from_value(serde_json::json!({
        "preceding": [{"message": 1}],
        "history": [{"message": 2}],
        "current": {"message": 3},
        "remaining": [{"message": 4}],
    }))
    .unwrap();
    session.set_shuffle(true);
    assert_eq!(session.remaining().len(), 3);
    session.set_shuffle(false);
    assert_eq!(
        session
            .preceding()
            .iter()
            .map(|track| track.message)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(session.current().unwrap().message, 3);
    assert_eq!(session.remaining()[0].message, 4);
}
