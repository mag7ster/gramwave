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
