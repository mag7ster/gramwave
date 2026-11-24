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

fn scroll_offset(h: &Harness) -> f32 {
    let id = h
        .ctx
        .data(|data| data.get_temp(egui::Id::new("test-queue-scroll")))
        .unwrap();
    egui::scroll_area::State::load(&h.ctx, id).unwrap().offset.y
}

fn scroll_queue(h: &mut Harness, at: egui::Pos2, delta: f32) {
    h.frame(vec![
        egui::Event::PointerMoved(at),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., delta),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
}

fn return_button(h: &Harness) -> Option<egui::Rect> {
    h.ctx
        .data(|data| data.get_temp(egui::Id::new("test-queue-return")))
}

#[test]
fn queue_realigns_from_either_side_of_a_wheel_step() {
    for steps in [1, 2] {
        let mut h = queue_harness(30);
        let current = queue_order(&h)[3];
        h.app.execute(AppCommand::PlayQueue(current));
        h.settle();
        h.settle();
        let anchor = scroll_offset(&h);
        let at = h.app.test_queue_rows[&current].0.center();

        scroll_queue(&mut h, at, 64.);
        for _ in 0..3 {
            h.settle();
        }
        assert!(return_button(&h).is_some());
        for _ in 0..steps {
            h.frame(vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0., -1.),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        for _ in 0..4 {
            h.settle();
        }
        assert!(
            (scroll_offset(&h) - anchor).abs() < 1.,
            "after {steps} steps: {} instead of {anchor}",
            scroll_offset(&h),
        );
        assert!(return_button(&h).is_none());
    }
}

#[test]
fn reopening_the_queue_aligns_the_last_track_in_both_layouts() {
    for width in [1240., 850.] {
        let mut h = queue_harness(6);
        let order = queue_order(&h);
        let stride =
            h.app.test_queue_rows[&order[1]].0.top() - h.app.test_queue_rows[&order[0]].0.top();
        h.app.set_queue_panel_open(false);
        h.size.x = width;
        h.frame(vec![]);
        h.app.execute(AppCommand::PlayQueue(order[4]));
        h.app.set_queue_panel_open(true);
        h.app.pane = WorkspacePane::Queue;
        h.frame(vec![]);
        assert!((scroll_offset(&h) - 5. * stride).abs() < 1.);
        assert!(h.app.test_queue_rows.contains_key(&order[4]));
        assert!(return_button(&h).is_none());
    }
}

#[test]
fn queue_follows_selection_but_preserves_user_scroll_until_return() {
    let mut h = queue_harness(30);
    let order = queue_order(&h);
    let stride =
        h.app.test_queue_rows[&order[1]].0.top() - h.app.test_queue_rows[&order[0]].0.top();
    h.click(h.app.test_queue_rows[&order[3]].0.center());
    h.frame(vec![]);
    h.frame(vec![]);
    assert!((0. ..4. * stride).contains(&scroll_offset(&h)));
    h.settle();
    assert!((scroll_offset(&h) - 4. * stride).abs() < 1.);
    assert!(return_button(&h).is_none());

    let at = h.app.test_queue_rows[&order[3]].0.center();
    scroll_queue(&mut h, at, 2. * stride);
    h.settle();
    h.settle();
    h.settle();
    let browsing = scroll_offset(&h);
    assert!(browsing < 3. * stride);
    assert!(return_button(&h).is_some());
    h.app.execute(AppCommand::Next);
    h.settle();
    assert_eq!(scroll_offset(&h), browsing);
    h.click(h.app.test_queue_rows[&order[5]].0.center());
    h.settle();
    assert_eq!(scroll_offset(&h), browsing);
    assert_eq!(
        h.app.state.view().session.current().unwrap().entry_id,
        order[5]
    );

    h.click(return_button(&h).unwrap().center());
    h.settle();
    assert!((scroll_offset(&h) - 6. * stride).abs() < 1.);
    assert!(return_button(&h).is_none());

    scroll_queue(&mut h, at, stride);
    h.settle();
    h.settle();
    h.settle();
    let delta = scroll_offset(&h) - 6. * stride;
    scroll_queue(&mut h, at, delta);
    scroll_queue(&mut h, at, -stride);
    h.settle();
    h.settle();
    h.settle();
    assert!(scroll_offset(&h) > 6.5 * stride);
    assert!(return_button(&h).is_some());
    let delta = scroll_offset(&h) - 6. * stride + 2.;
    scroll_queue(&mut h, at, delta);
    for _ in 0..3 {
        h.settle();
    }
    assert!((scroll_offset(&h) - 6. * stride).abs() < 1.);
    assert!(return_button(&h).is_none());
}

#[test]
fn queue_drags_current_across_the_boundary_and_can_select_the_new_past() {
    let mut h = queue_harness(8);
    let current = h.app.state.view().session.current().unwrap().entry_id;
    let order = queue_order(&h);
    let from = h.app.test_queue_rows[&current].0.center();
    let to = h.app.test_queue_rows[&order[1]].0.center();
    h.pointer(from, true);
    h.frame(vec![egui::Event::PointerMoved(to)]);
    h.pointer(to, false);
    h.settle();
    assert_eq!(queue_order(&h), order[2..]);
    assert_eq!(
        h.app.state.view().session.current().unwrap().entry_id,
        current
    );
    let anchor = scroll_offset(&h);
    assert!(anchor > 100.);

    scroll_queue(&mut h, from, anchor);
    h.settle();
    h.settle();
    h.settle();
    let browsing = scroll_offset(&h);
    h.click(h.app.test_queue_rows[&order[0]].0.center());
    h.settle();
    assert_eq!(scroll_offset(&h), browsing);
    assert_eq!(
        h.app.state.view().session.current().unwrap().entry_id,
        order[0]
    );
    assert_eq!(queue_order(&h)[..2], [order[1], current]);
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
