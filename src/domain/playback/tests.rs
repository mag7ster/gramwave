use super::*;
#[cfg(test)]
mod regression {
    use super::*;
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
        s.remaining.iter().map(|t| t.message).collect()
    }
    #[test]
    fn previous_from_middle_survives_restart_and_returns_forward() {
        let mut s = Session::default();
        let mut q = vec![];
        s.start(&tracks(), 3, false);
        let mut s: Session = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s.previous(&mut q).unwrap().message, 2);
        assert_eq!(s.previous(&mut q).unwrap().message, 1);
        assert_eq!(
            s.advance(&mut q, RepeatMode::Off, false, false)
                .unwrap()
                .message,
            2
        );
        assert_eq!(
            s.advance(&mut q, RepeatMode::Off, false, false)
                .unwrap()
                .message,
            3
        );
        assert_eq!(order(&s), [4]);
        s.clear();
        assert!(s.previous(&mut q).is_none());
        s.start(&tracks(), 2, true);
        assert!(s.previous(&mut q).is_none());
    }
    #[test]
    fn edits_duplicates_history_and_repeat() {
        let t = tracks();
        let mut s = Session::default();
        let mut q = vec![];
        s.start(&t, 0, false);
        s.insert(t[2].clone(), true);
        s.insert(t[2].clone(), false);
        assert_eq!(order(&s), [2, 1, 2, 3, 4, 2]);
        assert_ne!(s.remaining[0].entry_id, s.remaining[2].entry_id);
        s.move_before(5, 1);
        s.remove(0);
        assert_eq!(order(&s), [2, 1, 2, 3, 4]);
        assert_eq!(s.jump_to(&mut q, 2).unwrap().message, 2);
        assert_eq!(s.previous(&mut q).unwrap().message, 1);
        assert_eq!(
            s.advance(&mut q, RepeatMode::Off, false, false)
                .unwrap()
                .message,
            2
        );
        assert_eq!(s.history.len(), 3);
        s.clear();
        assert!(s.remaining.is_empty());
        assert_eq!(s.snapshot.len(), 1);
        assert!(s.advance(&mut q, RepeatMode::Off, false, true).is_none());
        assert_eq!(
            s.advance(&mut q, RepeatMode::One, false, true)
                .unwrap()
                .message,
            2
        );
    }
    #[test]
    fn shuffle_edits_restore_and_persist() {
        let mut s = Session::default();
        s.start(&tracks(), 0, false);
        s.reorder(true);
        let anchor = s.remaining[0].entry_id;
        s.insert(tracks()[0].clone(), true);
        let id = s.remaining[0].entry_id;
        s.reorder(false);
        let at = s.remaining.iter().position(|t| t.entry_id == id).unwrap();
        assert_eq!(s.remaining[at + 1].entry_id, anchor);
        let removed = s.remaining[1].entry_id;
        s.remove(1);
        let expected = order(&s);
        let mut restored: Session =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(order(&restored), expected);
        assert_eq!(restored.current, s.current);
        let mut q = vec![];
        for message in expected {
            assert_eq!(
                restored
                    .advance(&mut q, RepeatMode::Off, false, true)
                    .unwrap()
                    .message,
                message
            );
        }
        restored.advance(&mut q, RepeatMode::All, false, true);
        assert!(!restored.snapshot.iter().any(|t| t.entry_id == removed));
    }
    #[test]
    fn empty_single_repeats_and_replacement() {
        let mut s = Session::default();
        let mut q = vec![];
        assert!(s.start(&[], 0, false).is_none());
        assert!(s.advance(&mut q, RepeatMode::All, true, true).is_none());
        s.insert(tracks()[0].clone(), false);
        assert!(s.current.is_none());
        assert_eq!(
            s.advance(&mut q, RepeatMode::Off, false, false)
                .unwrap()
                .message,
            0
        );
        assert_eq!(
            s.advance(&mut q, RepeatMode::All, true, true)
                .unwrap()
                .message,
            0
        );
        s.start(&tracks(), 3, false);
        assert!(s.history.is_empty());
        assert_eq!(order(&s), [4]);
        assert_eq!(
            s.advance(&mut q, RepeatMode::One, false, true)
                .unwrap()
                .message,
            3
        );
        assert_eq!(
            s.advance(&mut q, RepeatMode::One, false, false)
                .unwrap()
                .message,
            4
        );
        assert!(s.advance(&mut q, RepeatMode::Off, false, true).is_none());
        for _ in 0..20 {
            let last = s.current.clone().unwrap();
            let next = s.advance(&mut q, RepeatMode::All, true, true).unwrap();
            if s.remaining.len() == s.snapshot.len() - 1 {
                assert!(!same(&last, &next));
            }
        }
    }
    #[test]
    fn legacy_import_preserves_visible_duplicates() {
        let mut s = Session {
            remaining: tracks()[..2].to_vec(),
            next_cycle: tracks()[..1].to_vec(),
            ..Default::default()
        };
        s.import(tracks()[..1].to_vec());
        assert_eq!(order(&s), [0, 0, 1, 0]);
        let mut ids: Vec<_> = s.remaining.iter().map(|t| t.entry_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 4);
        assert!(s.history.is_empty());
    }
    #[test]
    fn queue_jump_preserves_skipped_positions_and_reordered_duplicates() {
        for shuffle in [false, true] {
            let mut session = Session::default();
            let mut manual = Vec::new();
            session.start(&tracks(), 1, shuffle);
            session.insert(tracks()[2].clone(), true);
            session.insert(tracks()[2].clone(), false);
            let last = session.remaining.len() - 1;
            session.move_before(last, 1);
            session.remove(2);
            let original: Vec<_> = session
                .current
                .iter()
                .chain(&session.remaining)
                .map(|t| t.entry_id)
                .collect();
            session.jump_to(&mut manual, session.remaining.len() - 1);
            let mut session: Session =
                serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
            for expected in original[..original.len() - 1].iter().rev() {
                assert_eq!(&session.previous(&mut manual).unwrap().entry_id, expected);
            }
            for expected in &original[1..] {
                assert_eq!(
                    &session
                        .advance(&mut manual, RepeatMode::Off, shuffle, false)
                        .unwrap()
                        .entry_id,
                    expected
                );
            }
            session.clear();
            assert!(session.previous(&mut manual).is_none());
        }
    }
    #[test]
    fn shuffle_start_has_no_predecessor_and_visits_every_other_entry_once() {
        let mut session = Session::default();
        let mut manual = Vec::new();
        session.start(&tracks(), 3, true);
        assert!(session.previous(&mut manual).is_none());
        let order = session.remaining.clone();
        let mut messages: Vec<_> = order.iter().map(|t| t.message).collect();
        messages.sort();
        assert_eq!(messages, [0, 1, 2, 4]);
        session.advance(&mut manual, RepeatMode::Off, true, false);
        assert_eq!(session.previous(&mut manual).unwrap().message, 3);
        assert_eq!(session.remaining, order);
    }
}
