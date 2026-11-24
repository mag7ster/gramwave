use super::*;
use egui::{Id, Key, Modifiers};

fn focused(h: &Harness) -> Option<Id> {
    h.ctx.memory(|m| m.focused())
}

fn row(h: &Harness) -> (WorkspacePane, Id, bool) {
    h.app.keyboard.focused(&h.ctx).expect("focused row")
}

fn focus(h: &mut Harness, id: Id) {
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.frame(vec![]);
}

#[test]
fn tab_focuses_find_a_chat_and_enter_opens_it() {
    let mut h = Harness::new();
    h.app.pane = WorkspacePane::Library;
    h.settle();
    let trigger = Id::new(CHAT_SEARCH_TRIGGER);
    for _ in 0..25 {
        h.key(Key::Tab);
        if focused(&h) == Some(trigger) {
            break;
        }
    }
    assert_eq!(focused(&h), Some(trigger));
    assert!(!h.app.chat_search_active);

    h.key(Key::Enter);
    assert!(h.app.chat_search_active);
    assert_eq!(focused(&h), Some(Id::new("chat-search")));
    h.frame(vec![egui::Event::Text("Jazz".into())]);
    assert_eq!(h.app.chat_query, "Jazz");
    h.key(Key::Escape);
    assert!(!h.app.chat_search_active);
    assert!(h.app.chat_query.is_empty());
    assert_eq!(focused(&h), Some(trigger));
    h.key(Key::Tab);
    assert_eq!(row(&h).0, WorkspacePane::Library);
}

#[test]
fn tab_and_arrows_reveal_rows_across_virtual_list_boundaries() {
    let mut h = queue_harness(30);
    h.size.y = 550.;
    h.settle();
    let playlist = h.app.test_row_ids[&0];
    focus(&mut h, playlist);
    h.key(Key::Tab);
    let menu = focused(&h).unwrap();
    assert_eq!(row(&h).0, WorkspacePane::Playlist);
    assert!(row(&h).2);
    h.key(Key::Enter);
    assert!(egui::Popup::is_any_open(&h.ctx));
    assert_ne!(focused(&h), Some(menu));
    assert_eq!(h.app.state.view().session.current().unwrap().message, 1);
    h.key(Key::Escape);
    assert_eq!(focused(&h), Some(menu));
    h.key_with(Key::Tab, Modifiers::SHIFT);
    assert_eq!(focused(&h), Some(playlist));

    for i in 1..=12 {
        h.key(Key::Tab);
        assert!(row(&h).2);
        h.key(Key::Tab);
        let id = h.app.test_row_ids[&i];
        assert_eq!(focused(&h), Some(id));
        let r = h.ctx.read_response(id).unwrap();
        assert!((r.rect.height() - r.interact_rect.height()).abs() < 1.);
        h.frame(vec![]);
        let next = h.ctx.read_response(h.app.test_row_ids[&(i + 1)]).unwrap();
        assert!(
            (next.rect.height() - next.interact_rect.height()).abs() < 1.,
            "next row after {i} is clipped: {next:?}"
        );
    }
    for i in (0..12).rev() {
        h.key(Key::ArrowUp);
        assert_eq!(focused(&h), Some(h.app.test_row_ids[&i]));
    }

    h.key(Key::ArrowRight);
    let current = h.app.state.view().session.current().unwrap().entry_id;
    assert_eq!(focused(&h), Some(h.app.test_queue_rows[&current].2));
    h.key(Key::ArrowDown);
    let queue: Vec<_> = h
        .app
        .state
        .view()
        .session
        .remaining()
        .iter()
        .map(|entry| entry.entry_id)
        .collect();
    for (i, entry) in queue[0..12].iter().enumerate() {
        let id = h.app.test_queue_rows[entry].2;
        assert_eq!(focused(&h), Some(id));
        let r = h.ctx.read_response(id).unwrap();
        assert!((r.rect.height() - r.interact_rect.height()).abs() < 1.);
        if i == 11 {
            h.key_with(Key::Tab, Modifiers::SHIFT);
            assert!(row(&h).2);
            assert_eq!(row(&h).1, h.app.test_queue_rows[&queue[i - 1]].2);
        } else {
            h.key(Key::ArrowDown);
        }
    }
}

#[test]
fn chat_focus_scrolls_through_pins_without_opening_them() {
    let mut h = Harness::new();
    h.app.pane = WorkspacePane::Library;
    h.size.y = 375.;
    let mut state = h.app.state.view().clone();
    for id in 5..20 {
        state.playlists.push(crate::Playlist {
            id,
            title: format!("Chat {id}"),
            ..Default::default()
        });
        state.chat_pins.push(id);
    }
    h.app.state = tg_player_application::AppCore::new(state);
    h.settle();
    focus(&mut h, Id::new(CHAT_SEARCH_TRIGGER));
    h.key(Key::Tab);
    for id in [0, 2, 4].into_iter().chain(5..20) {
        let (pane, key, _) = row(&h);
        assert_eq!((pane, key), (WorkspacePane::Library, Id::new(id as i64)));
        let r = h.ctx.read_response(focused(&h).unwrap()).unwrap();
        assert!((r.rect.height() - r.interact_rect.height()).abs() < 1.);
        if id < 19 {
            h.key(if id % 2 == 0 {
                Key::Tab
            } else {
                Key::ArrowDown
            });
        }
    }
    assert_eq!(h.app.opened_chat, Some(0));
}

#[test]
fn arrows_switch_panels_without_opening_chat_or_changing_playback() {
    for width in [850., 1240.] {
        let mut h = queue_harness(5);
        h.size.x = width;
        h.app.set_queue_panel_open(false);
        h.app.pane = WorkspacePane::Library;
        h.settle();
        focus(&mut h, Id::new(CHAT_SEARCH_TRIGGER));
        h.key(Key::Tab);
        h.key(Key::ArrowDown);
        assert_eq!(row(&h).0, WorkspacePane::Library);
        h.key(Key::ArrowRight);
        assert_eq!(row(&h).0, WorkspacePane::Playlist);
        assert_eq!(h.app.opened_chat, Some(0));
        h.key(Key::ArrowRight);
        h.settle();
        assert_eq!(row(&h).0, WorkspacePane::Queue);
        assert_eq!(h.app.state.view().queue_panel_open, width >= 1040.);
        assert_eq!(h.app.state.view().session.current().unwrap().message, 1);
        assert_eq!(h.app.state.view().session.remaining().len(), 4);
        h.key(Key::ArrowLeft);
        assert_eq!(row(&h).0, WorkspacePane::Playlist);
        h.key(Key::ArrowLeft);
        assert_eq!(row(&h).0, WorkspacePane::Library);

        h.app.execute(AppCommand::Stop);
        h.key(Key::ArrowRight);
        h.key(Key::ArrowRight);
        h.settle();
        assert_eq!(row(&h).0, WorkspacePane::Queue);
        h.key(Key::ArrowLeft);
        assert_eq!(row(&h).0, WorkspacePane::Playlist);
    }
}

#[test]
fn docked_tab_order_passes_the_playlist_before_the_queue() {
    let mut h = queue_harness(3);
    h.size.y = 1900.;
    h.settle();
    focus(&mut h, Id::new(CHAT_SEARCH_TRIGGER));
    let mut panes = Vec::new();
    for _ in 0..110 {
        h.key(Key::Tab);
        if let Some((pane, _, _)) = h.app.keyboard.focused(&h.ctx) {
            if panes.last() != Some(&pane) {
                panes.push(pane);
            }
            if pane == WorkspacePane::Queue {
                break;
            }
        }
    }
    assert_eq!(
        panes,
        [
            WorkspacePane::Library,
            WorkspacePane::Playlist,
            WorkspacePane::Queue
        ]
    );
}
