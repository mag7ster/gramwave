use super::*;

fn queue_order(h: &Harness) -> Vec<QueueEntryId> {
    h.app
        .state
        .view()
        .session
        .remaining()
        .iter()
        .map(|entry| entry.entry_id)
        .collect()
}

#[test]
fn queue_drag_from_artwork_commits_only_on_drop() {
    let mut h = queue_harness(5);
    let order = queue_order(&h);
    let first = h.app.test_queue_rows[&order[0]].0;
    let third = h.app.test_queue_rows[&order[2]].0;
    let current = h.app.state.view().session.current().unwrap().entry_id;
    let grab = egui::pos2(first.left() + 24., first.center().y);
    let target = egui::pos2(grab.x, third.center().y);
    h.pointer(grab, true);
    h.frame(vec![egui::Event::PointerMoved(target)]);
    h.frame(vec![]);
    assert_eq!(queue_order(&h), order);

    h.pointer(target, false);
    assert_eq!(queue_order(&h), [order[1], order[2], order[0], order[3]]);
    assert_eq!(
        h.app.state.view().session.current().unwrap().entry_id,
        current
    );
    h.app.shutdown_core().unwrap();
}

#[test]
fn queue_drag_upward_selects_the_occurrence_and_cancels_outside_the_list() {
    let mut h = queue_harness(5);
    let duplicate = (&h.app.state.view().session.remaining()[0]).into();
    h.app.execute(AppCommand::Insert(duplicate, false));
    h.frame(vec![]);
    let order = queue_order(&h);
    let last = *order.last().unwrap();
    let grab = h.app.test_queue_rows[&last].0.center();
    let target = egui::pos2(grab.x, h.app.test_queue_rows[&order[0]].0.center().y);
    h.pointer(grab, true);
    h.frame(vec![egui::Event::PointerMoved(target)]);
    h.pointer(target - egui::vec2(350., 0.), false);
    assert_eq!(queue_order(&h), order);
    h.time += 0.25;
    h.frame(vec![]);
    h.frame(vec![]);

    h.pointer(grab, true);
    h.frame(vec![egui::Event::PointerMoved(target)]);
    h.pointer(target, false);
    assert_eq!(
        queue_order(&h),
        [last, order[0], order[1], order[2], order[3]]
    );
    h.app.shutdown_core().unwrap();
}

#[test]
fn queue_row_plays_on_click_or_enter() {
    for target in ["artwork", "title", "row", "keyboard"] {
        let mut h = queue_harness(5);
        let entry = queue_order(&h)[1];
        let (row, menu, id) = h.app.test_queue_rows[&entry];
        if target == "keyboard" {
            h.ctx.memory_mut(|m| m.request_focus(id));
            h.key(egui::Key::Enter);
        } else {
            let center_y = row.center().y;
            let pos = match target {
                "artwork" => egui::pos2(row.left() + 24., center_y),
                "title" => egui::pos2(row.left() + 68., center_y - 10.),
                _ => egui::pos2(menu.left() - 15., center_y),
            };
            h.click(pos);
        }
        assert_eq!(
            h.app.state.view().session.current().unwrap().entry_id,
            entry,
            "{target}"
        );
        h.app.shutdown_core().unwrap();
    }
}

#[test]
fn queue_menu_and_stationary_drag_leave_playback_unchanged() {
    let mut h = queue_harness(5);
    let order = queue_order(&h);
    let current = h.app.state.view().session.current().unwrap().entry_id;
    let (row, menu, _) = h.app.test_queue_rows[&order[0]];
    assert!((row.right() - menu.right() - TRACK_ROW_PADDING).abs() < 1.);
    let target = h.app.test_queue_rows[&order[2]].0.center();
    h.pointer(menu.center(), true);
    h.frame(vec![egui::Event::PointerMoved(target)]);
    assert!(h.app.test_queue_rows.contains_key(&order[0]));
    h.pointer(target, false);
    assert!(!egui::Popup::is_any_open(&h.ctx));
    h.click(menu.center());
    assert!(egui::Popup::is_any_open(&h.ctx));
    h.key(egui::Key::Escape);
    assert!(!egui::Popup::is_any_open(&h.ctx));
    for pressed in [true, false] {
        h.frame(vec![egui::Event::PointerButton {
            pos: row.center(),
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }]);
    }
    assert!(egui::Popup::is_any_open(&h.ctx));
    h.key(egui::Key::Escape);
    h.pointer(row.center(), true);
    h.time += 1.;
    h.frame(vec![]);
    assert!(!h.app.test_queue_rows.contains_key(&order[0]));
    h.pointer(row.center(), false);
    assert_eq!(queue_order(&h), order);
    assert_eq!(
        h.app.state.view().session.current().unwrap().entry_id,
        current,
    );
    h.app.shutdown_core().unwrap();
}

#[test]
fn queue_drag_scrolls_at_the_edge_and_escape_keeps_the_order() {
    let mut h = queue_harness(24);
    let order = queue_order(&h);
    let last_visible = *h.app.test_queue_rows.keys().last().unwrap();
    let from = h.app.test_queue_rows[&order[0]].0.center();
    let panel = egui::panel::PanelState::load(&h.ctx, egui::Id::new("queue")).unwrap();
    let edge = egui::pos2(from.x, panel.outer_rect.bottom() - 24.);
    h.pointer(from, true);
    h.frame(vec![egui::Event::PointerMoved(edge)]);
    for _ in 0..12 {
        h.frame(vec![]);
    }
    assert!(*h.app.test_queue_rows.keys().last().unwrap() > last_visible);
    h.key(egui::Key::Escape);
    h.pointer(edge, false);
    assert_eq!(queue_order(&h), order);
    h.app.shutdown_core().unwrap();
}
