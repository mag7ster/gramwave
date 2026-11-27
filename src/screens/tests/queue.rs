use super::*;
use crate::screens::queue::queue_cover_rect;
use gramwave_domain::QueueEntryId;

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
fn queue_drag_from_row_body_commits_only_on_drop() {
    let mut h = queue_harness(5);
    let order = queue_order(&h);
    let first = h.app.test_queue_rows[&order[0]].0;
    let third = h.app.test_queue_rows[&order[2]].0;
    let current = h.app.state.view().session.current().unwrap().entry_id;
    let grab = first.center();
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
fn queue_menu_and_long_press_leave_playback_unchanged() {
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
    let cover = queue_cover_rect(row).center();
    h.pointer(cover, true);
    h.time += 0.5;
    h.frame(vec![]);
    assert!(h.app.selection.queue_range().is_none());
    h.pointer(cover, false);
    assert!(h.app.selection.queue_range().is_none());
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

#[test]
fn queue_reverses_selected_rows_without_restarting_the_current_track() {
    let mut h = queue_harness(6);
    let current = h.app.state.view().session.current().unwrap().entry_id;
    let order = queue_order(&h);
    let from = queue_cover_rect(h.app.test_queue_rows[&current].0).center();
    let to = h.app.test_queue_rows[&order[1]].0.center();
    h.pointer(from, true);
    h.frame(vec![egui::Event::PointerMoved(to)]);
    h.pointer(to, false);
    assert_eq!(h.app.selection.queue_range(), Some(0..=2));
    h.key(egui::Key::Enter);
    let reverse = menu_text(&mut h, "Reverse selected tracks")[0];
    h.click(reverse);
    let session = &h.app.state.view().session;
    assert_eq!(session.timeline().current_index(), Some(2));
    assert_eq!(session.current().unwrap().entry_id, current);
    assert_eq!(session.timeline().get(0).unwrap().entry_id, order[1]);
    assert_eq!(session.timeline().get(1).unwrap().entry_id, order[0]);
    assert_eq!(h.app.selection.queue_range(), Some(0..=2));
    h.key(egui::Key::Escape);
    assert!(h.app.selection.queue_range().is_none());
}

#[test]
fn ctrl_selection_in_queue_fills_gaps_and_preserves_the_middle() {
    let mut h = queue_harness(6);
    let order = queue_order(&h);
    for index in [0, 3, 1] {
        let row = h.app.test_queue_rows[&order[index]].0.center();
        h.click_with(row, egui::PointerButton::Primary, egui::Modifiers::CTRL);
    }
    assert_eq!(h.app.selection.queue_range(), Some(1..=4));
    let last = h.app.test_queue_rows[&order[3]].0.center();
    h.click_with(last, egui::PointerButton::Primary, egui::Modifiers::CTRL);
    assert_eq!(h.app.selection.queue_range(), Some(1..=3));
}

#[test]
fn queue_modifier_drag_selects_from_body_and_one_selected_row_keeps_remove() {
    use egui::{Key, Modifiers, PointerButton};
    let mut h = queue_harness(6);
    let order = queue_order(&h);
    let (from, _, id) = h.app.test_queue_rows[&order[0]];
    h.click_with(from.center(), PointerButton::Primary, Modifiers::CTRL);
    h.key(Key::Enter);
    assert!(!menu_text(&mut h, "Remove").is_empty());
    assert!(menu_text(&mut h, "Reverse selected tracks").is_empty());
    h.key(Key::Escape);
    assert_eq!(h.ctx.memory(|m| m.focused()), Some(id));

    let start = from.center();
    let end = h.app.test_queue_rows[&order[2]].0.center();
    h.frame(vec![
        egui::Event::PointerMoved(start),
        egui::Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::CTRL,
        },
    ]);
    h.frame(vec![egui::Event::PointerMoved(end)]);
    h.pointer(end, false);
    assert_eq!(h.app.selection.queue_range(), Some(1..=3));
    assert_eq!(queue_order(&h), order);
}

#[test]
fn queue_cover_drag_shrinks_and_ctrl_restores_the_original_range() {
    use egui::{Modifiers, PointerButton};
    let mut h = queue_harness(7);
    let current = h.app.state.view().session.current().unwrap().entry_id;
    let order = queue_order(&h);
    let pos: Vec<_> = order
        .iter()
        .take(5)
        .map(|id| queue_cover_rect(h.app.test_queue_rows[id].0).center())
        .collect();

    h.pointer(pos[2], true);
    for (i, expected) in [
        (4, Some(3..=5)),
        (3, Some(3..=4)),
        (2, None),
        (1, Some(2..=3)),
        (2, None),
    ] {
        h.frame(vec![egui::Event::PointerMoved(pos[i])]);
        assert_eq!(h.app.selection.queue_range(), expected);
    }
    h.pointer(pos[2], false);
    assert!(h.app.selection.queue_range().is_none());

    h.click_with(pos[0], PointerButton::Primary, Modifiers::CTRL);
    h.frame(vec![
        egui::Event::PointerMoved(pos[2]),
        egui::Event::PointerButton {
            pos: pos[2],
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::CTRL,
        },
    ]);
    for (i, range) in [(4, 1..=5), (3, 1..=4), (2, 1..=1)] {
        h.frame(vec![egui::Event::PointerMoved(pos[i])]);
        assert_eq!(h.app.selection.queue_range(), Some(range));
    }
    h.pointer(pos[2], false);
    assert_eq!(h.app.selection.queue_range(), Some(1..=1));
    assert_eq!(
        h.app.state.view().session.current().unwrap().entry_id,
        current
    );
    assert_eq!(queue_order(&h), order);
}

#[test]
fn queue_edge_scroll_arms_on_movement_and_stops_after_release() {
    let mut h = queue_harness(30);
    let current = queue_order(&h)[10];
    h.app.execute(AppCommand::PlayQueue(current));
    h.settle();
    let (viewport, initial) = h.list_scroll("test-queue-scroll");
    let cover = queue_cover_rect(h.app.test_queue_rows[&current].0).center();
    let top = egui::pos2(cover.x, viewport.top() - 5.);
    let farther = top - egui::vec2(45., 42.);
    let bottom = egui::pos2(cover.x, viewport.bottom() + 5.);
    let start = h
        .app
        .state
        .view()
        .session
        .timeline()
        .current_index()
        .unwrap();

    h.pointer(cover, true);
    h.settle();
    assert_eq!(scroll_offset(&h), initial);
    assert!(h.app.selection.queue_range().is_none());

    h.frame(vec![egui::Event::PointerMoved(top)]);
    assert!(h.app.selection.queue_range().is_none());
    let near = scroll_offset(&h);
    assert!(near < initial);
    h.frame(vec![egui::Event::PointerMoved(farther)]);
    assert!(near - scroll_offset(&h) > initial - near);
    h.settle();
    let up = scroll_offset(&h);
    assert!(up < initial - design::ROW_HEIGHT);
    let selection = h.app.selection.queue_range().unwrap();
    assert!(*selection.start() < start);
    assert_eq!(*selection.end(), start);

    h.frame(vec![egui::Event::PointerMoved(bottom)]);
    h.settle();
    assert!(scroll_offset(&h) > up);
    assert!(*h.app.selection.queue_range().unwrap().end() > start);
    let stopped = scroll_offset(&h);
    h.frame(vec![egui::Event::PointerMoved(viewport.center())]);
    h.settle();
    assert_eq!(scroll_offset(&h), stopped);
    h.frame(vec![egui::Event::PointerMoved(bottom)]);
    assert!(scroll_offset(&h) > stopped);
    h.pointer(bottom, false);
    let released = scroll_offset(&h);
    h.settle();
    assert_eq!(scroll_offset(&h), released);
    assert!(return_button(&h).is_some());
    assert_eq!(
        h.app.state.view().session.current().unwrap().entry_id,
        current
    );
}

#[test]
fn queue_menu_and_delete_stack_independent_undo_buttons_at_the_bottom() {
    use egui::{Key, Modifiers, PointerButton};
    let mut h = queue_harness(6);
    let order = queue_order(&h);

    let first = h.app.test_queue_rows[&order[0]].0.center();
    let second = h.app.test_queue_rows[&order[1]].0.center();
    for row in [first, second] {
        h.click_with(row, PointerButton::Primary, Modifiers::CTRL);
    }
    h.key(Key::Enter);
    let remove = menu_text(&mut h, "Remove selected tracks")[0];
    h.click(remove);
    assert_eq!(queue_order(&h), order[2..]);
    h.settle();

    h.key(Key::Delete);
    assert_eq!(queue_order(&h), order[3..]);
    assert_eq!(menu_text(&mut h, "2 tracks removed").len(), 1);
    assert_eq!(menu_text(&mut h, "Track removed").len(), 1);
    let mut undo = menu_text(&mut h, "Undo");
    assert_eq!(undo.len(), 2);
    undo.sort_by(|a, b| b.y.total_cmp(&a.y));
    assert!(undo[0].y - undo[1].y > 25.);
    h.click(undo[0]);
    h.click(undo[0]);
    assert_eq!(queue_order(&h), order);
    assert!(menu_text(&mut h, "Undo").is_empty());
}

#[test]
fn queue_undo_expires_independently_and_drops_the_next_button() {
    use egui::{Key, Modifiers, PointerButton};
    let mut h = queue_harness(4);
    let order = queue_order(&h);
    let menu = h.app.test_queue_rows[&order[0]].1.center();
    h.click_with(menu, PointerButton::Primary, Modifiers::NONE);
    let remove = menu_text(&mut h, "Remove")[0];
    h.click(remove);
    h.settle();
    let bottom = menu_text(&mut h, "Undo")[0];
    h.time += 3.;
    h.key(Key::Delete);
    assert_eq!(queue_order(&h), order[2..]);
    h.time += 2.;
    h.settle();
    assert_eq!(menu_text(&mut h, "Undo"), [bottom]);
    h.click(bottom);
    assert_eq!(queue_order(&h), order[1..]);
}

#[test]
fn full_queue_reverse_preserves_clipped_rows_and_keeps_follow_mode() {
    let mut h = queue_harness(30);
    let order = queue_order(&h);
    let stride =
        h.app.test_queue_rows[&order[1]].0.top() - h.app.test_queue_rows[&order[0]].0.top();
    let row_height = design::ROW_HEIGHT;
    let key = "test-queue-scroll";
    let (viewport, offset) = h.list_scroll(key);
    let clip = (10..50)
        .map(|value| value as f32)
        .find(|clip| {
            let bottom = (viewport.height() + clip) % stride;
            (4. ..row_height - 4.).contains(&bottom)
        })
        .unwrap();
    scroll_queue(&mut h, viewport.center(), offset - 8. * stride - clip);
    for _ in 0..4 {
        h.settle();
    }
    let (viewport, _) = h.list_scroll(key);
    assert!(return_button(&h).is_some());
    let current = h.app.state.view().session.current().unwrap().entry_id;
    assert!(!h.app.test_queue_rows.contains_key(&current));
    let clipped: Vec<_> = h
        .app
        .test_queue_rows
        .iter()
        .filter_map(|(&id, (rect, _, _))| {
            let visible = viewport.intersect(*rect).height();
            (visible > 0. && visible < row_height).then_some((
                id,
                visible,
                rect.top() < viewport.top(),
            ))
        })
        .collect();
    assert_eq!(clipped.len(), 2);

    h.app.execute(AppCommand::ReverseQueue);
    h.frame(vec![]);
    let (viewport, _) = h.list_scroll(key);
    for (id, visible, was_above) in clipped {
        let row = h.app.test_queue_rows[&id].0;
        assert!(
            (viewport.intersect(row).height() - visible).abs() < 1.,
            "partially visible row changed its visible height after reversal"
        );
        assert_eq!(row.bottom() > viewport.bottom(), was_above);
    }
    assert!(return_button(&h).is_some());

    h.click(return_button(&h).unwrap().center());
    h.settle();
    h.app.execute(AppCommand::ReverseQueue);
    h.frame(vec![]);
    let current = h.app.state.view().session.current().unwrap().entry_id;
    assert!(return_button(&h).is_none());
    assert!(h
        .list_scroll(key)
        .0
        .contains(h.app.test_queue_rows[&current].0.center()));
}

#[test]
fn playing_a_queue_row_near_the_top_reuses_the_snap_distance() {
    for (distance, should_snap) in [(-16., true), (16., true), (40., false)] {
        let mut h = queue_harness(30);
        let order = queue_order(&h);
        let stride =
            h.app.test_queue_rows[&order[1]].0.top() - h.app.test_queue_rows[&order[0]].0.top();
        h.app.execute(AppCommand::PlayQueue(order[11]));
        h.settle();
        h.settle();

        let target = 6. * stride;
        let (viewport, offset) = h.list_scroll("test-queue-scroll");
        scroll_queue(&mut h, viewport.center(), offset - target - distance);
        for _ in 0..4 {
            h.settle();
        }
        let before_click = scroll_offset(&h);
        assert!(return_button(&h).is_some());

        let row = h.app.test_queue_rows[&order[5]].0;
        let viewport = h.list_scroll("test-queue-scroll").0;
        h.click(row.intersect(viewport).center());
        assert_eq!(
            h.app.state.view().session.current().unwrap().entry_id,
            order[5]
        );
        for _ in 0..4 {
            h.settle();
        }

        let expected = if should_snap { target } else { before_click };
        assert!((scroll_offset(&h) - expected).abs() < 1.);
        assert_eq!(return_button(&h).is_none(), should_snap);
    }
}

#[test]
fn queue_reverse_anchors_the_current_row_only_outside_the_edge_zones() {
    let row_height = design::ROW_HEIGHT;
    for (edge_distance, top_edge, keep_current) in [
        (130., true, true),
        (100., false, true),
        (45., true, false),
        (45., false, false),
        (16., true, false),
        (12., false, false),
        (-16., true, false),
        (-12., false, false),
    ] {
        let mut h = queue_harness(30);
        let mut settings = gramwave_application::persistence::Settings::from(h.app.state.view());
        settings.light = !top_edge;
        h.app.execute(AppCommand::Settings(settings));
        h.settle();
        let order = queue_order(&h);
        let current = order[10];
        let stride =
            h.app.test_queue_rows[&order[1]].0.top() - h.app.test_queue_rows[&order[0]].0.top();
        let (viewport, offset) = h.list_scroll("test-queue-scroll");
        let current_top = if top_edge {
            edge_distance
        } else {
            viewport.height() - row_height - edge_distance
        };
        let target = 11. * stride - current_top;
        scroll_queue(&mut h, viewport.center(), offset - target);
        for _ in 0..4 {
            h.settle();
        }
        h.app.execute(AppCommand::PlayQueue(current));
        h.settle();

        let viewport = h.list_scroll("test-queue-scroll").0;
        let before = h.app.test_queue_rows[&current].0;
        assert!((before.top() - viewport.top() - current_top).abs() < 1.);
        assert!(return_button(&h).is_some());

        let initial_offset = scroll_offset(&h);
        h.app.execute(AppCommand::ReverseQueue);
        h.frame(vec![]);

        let viewport_after = h.list_scroll("test-queue-scroll").0;
        let after = h.app.test_queue_rows[&current].0;
        let expected_top = if keep_current {
            current_top
        } else {
            viewport_after.height() - row_height - current_top
        };
        assert!(
            (after.top() - viewport_after.top() - expected_top).abs() < 1.,
            "first reverse placed the current row incorrectly at {edge_distance}px from the edge"
        );
        assert!(return_button(&h).is_some());

        h.app.execute(AppCommand::ReverseQueue);
        h.frame(vec![]);
        let (viewport_again, offset_again) = h.list_scroll("test-queue-scroll");
        let again = h.app.test_queue_rows[&current].0;
        assert!((offset_again - initial_offset).abs() < 1.);
        assert!((again.top() - viewport_again.top() - current_top).abs() < 1.);
    }
}
