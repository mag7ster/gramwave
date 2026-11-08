use super::*;
#[cfg(test)]
mod regression {
    use super::*;

    #[test]
    fn opening_nonempty_index_preserves_pagination() {
        let mut p = Playlist {
            tracks: vec![Track::default()],
            cursors: [10, 20, 0],
            done: [true, true, false],
            ..Default::default()
        };
        p.recover_empty_index();
        assert_eq!(p.cursors, [10, 20, 0]);
        assert_eq!(p.done, [true, true, false]);
    }
}

#[cfg(test)]
mod pin_tests {
    use super::*;

    fn library() -> State {
        State {
            playlists: vec![
                Playlist {
                    id: 1,
                    title: "Ambient".into(),
                    ..Default::default()
                },
                Playlist {
                    id: 2,
                    title: "Музыка".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }
    #[test]
    fn sidebar_switches_between_pins_and_search() {
        let mut s = library();
        assert!(s.sidebar_chats(false, "").is_empty());
        assert_eq!(s.sidebar_chats(true, ""), vec![0, 1]);
        s.toggle_chat_pin(2);
        assert_eq!(s.sidebar_chats(false, ""), vec![1]);
        assert_eq!(s.sidebar_chats(true, "МУЗ"), vec![1]);
        assert_eq!(s.sidebar_chats(true, "amB"), vec![0]);
        s.toggle_chat_pin(2);
        assert!(s.sidebar_chats(false, "").is_empty());
    }
    #[test]
    fn pinning_does_not_enable_offline_downloads() {
        let mut s = library();
        s.toggle_chat_pin(1);
        assert!(!s.playlists[0].pinned);
        let restored: State = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(restored.chat_pins, vec![1]);
        let old: State = serde_json::from_str(r#"{"playlists":[{"id":1,"pinned":true}]}"#).unwrap();
        assert!(old.chat_pins.is_empty());
        assert!(old.playlists[0].pinned);
    }
}

#[cfg(test)]
mod queue_repair_tests {
    use super::*;

    fn old_state() -> State {
        let tracks: Vec<_> = (0..131)
            .map(|message| Track {
                chat: 7,
                message,
                ..Default::default()
            })
            .collect();
        let mut state = State {
            queue_version: 0,
            queue: tracks[20..].to_vec(),
            ..Default::default()
        };
        state.session.start(&tracks, 0, false);
        state.session.remaining.reverse();
        state
    }
    #[test]
    fn repairs_duplicated_legacy_tail_without_losing_tracks_or_order() {
        let mut state = old_state();
        let before = state.session.remaining.clone();
        let legacy = state.queue.clone();
        assert!(state.repair_legacy_queue());
        assert!(state.queue.is_empty());
        assert_eq!(state.session.remaining, before);
        assert_eq!(state.previous_queue, legacy);
        state.session.reorder(false);
        assert_eq!(state.session.remaining[0].message, 1);
        assert_eq!(state.session.remaining.last().unwrap().message, 130);
        assert!(!state.repair_legacy_queue());
        let saved: State = serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(saved.previous_queue, legacy);
        assert_eq!(saved.queue_version, 1);
    }
    #[test]
    fn leaves_explicit_or_nonduplicated_queues_untouched() {
        let mut state = old_state();
        state.queue.swap(0, 1);
        let queue = state.queue.clone();
        assert!(!state.repair_legacy_queue());
        assert_eq!(state.queue, queue);
        let mut state = old_state();
        state.session.remaining.clear();
        assert!(!state.repair_legacy_queue());
        assert_eq!(state.queue.len(), 111);
        let mut state = old_state();
        state.queue_version = 1;
        assert!(!state.repair_legacy_queue());
        assert_eq!(state.queue.len(), 111);
        let mut state = old_state();
        state.queue = state.queue[105..].to_vec();
        assert!(!state.repair_legacy_queue());
        assert_eq!(state.queue.len(), 6);
    }
    #[test]
    fn missing_version_is_detected_but_new_queues_are_versioned() {
        let old: State = serde_json::from_str(r#"{"queue":[]}"#).unwrap();
        assert_eq!(old.queue_version, 0);
        assert_eq!(State::default().queue_version, 1);
    }
}
