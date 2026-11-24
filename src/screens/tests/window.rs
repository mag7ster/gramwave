use super::*;

#[test]
fn window_header_keeps_the_app_name_visible_after_a_long_track_title() {
    let ctx = egui::Context::default();
    egui_extras::install_image_loaders(&ctx);
    design::install(&ctx);
    design::style(&ctx, false);
    let long = "A long track title ".repeat(80);
    let texts = |title| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(850., 550.),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            window_header(ui, title, true);
        });
        output.textures_delta.clear();
        output
            .shapes
            .into_iter()
            .filter_map(|shape| match shape.shape {
                egui::Shape::Text(text) => {
                    Some((text.galley.text().to_owned(), text.galley.elided))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(texts(None), [("Gramwave".into(), false)]);
    assert_eq!(
        texts(Some(&long)),
        [(long.clone(), true), ("— Gramwave".into(), false)]
    );
}

#[test]
fn clicking_the_header_icon_opens_the_application_menu() {
    let ctx = egui::Context::default();
    egui_extras::install_image_loaders(&ctx);
    design::install(&ctx);
    design::style(&ctx, false);
    let frame = |events| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(850., 550.),
            )),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            window_header(ui, Some("Track"), true);
        });
        output.textures_delta.clear();
    };
    frame(vec![]);
    let pos = egui::pos2(26., 22.);
    for pressed in [true, false] {
        frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
    }
    assert!(egui::Popup::is_any_open(&ctx));
}

#[test]
fn header_drags_again_when_the_compositor_keeps_the_release() {
    let ctx = egui::Context::default();
    design::install(&ctx);
    design::style(&ctx, false);
    let mut time = 0.;
    let mut frame = |events| {
        time += 0.02;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(850., 550.),
            )),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            window_header(ui, Some("Track"), true);
        });
        output.textures_delta.clear();
        output
            .viewport_output
            .remove(&egui::ViewportId::ROOT)
            .unwrap()
            .commands
    };
    let start = egui::pos2(220., 22.);
    frame(vec![egui::Event::PointerMoved(start)]);
    for _ in 0..2 {
        let begin = frame(vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        assert!(!begin.contains(&egui::ViewportCommand::StartDrag));
        let moved = frame(vec![egui::Event::PointerMoved(start + egui::vec2(16., 0.))]);
        assert!(moved.contains(&egui::ViewportCommand::StartDrag));
        frame(vec![egui::Event::PointerGone]);
    }
}

#[test]
fn side_panels_keep_the_dragged_width_with_an_empty_queue() {
    let mut h = Harness::new();
    h.size = egui::vec2(1100., 800.);
    h.app.queue_open = true;
    h.settle();

    for (name, left, offsets) in [
        ("sidebar", true, [85., -100.]),
        ("queue", false, [-85., 100.]),
    ] {
        let id = egui::Id::new(name);
        let panel =
            |ctx: &egui::Context| egui::panel::PanelState::load(ctx, id).unwrap().outer_rect;
        for offset in offsets {
            let before = panel(&h.ctx);
            let from = if left {
                before.right_center()
            } else {
                before.left_center()
            };
            let to = from + egui::vec2(offset, 0.);
            h.pointer(from, true);
            h.frame(vec![egui::Event::PointerMoved(to)]);
            h.pointer(to, false);
            h.frame(vec![]);
            let expected = before.width() + if left { offset } else { -offset };
            let actual = panel(&h.ctx).width();
            assert!(
                (actual - expected).abs() < 1.,
                "{name}: {actual} != {expected}"
            );
            assert!(h.ctx.memory(|m| m.area_rect("compact-queue").is_none()));
        }
    }
    h.app.shutdown_core().unwrap();
}

#[test]
fn resizing_switches_between_docked_and_compact_library() {
    let mut h = Harness::new();
    let library_visible = |h: &Harness| h.ctx.read_response(egui::Id::new("chat-search")).is_some();
    assert!(!library_visible(&h));
    h.size.x = 1100.;
    h.settle();
    assert!(library_visible(&h));
    h.size.x = 850.;
    h.settle();
    assert!(!library_visible(&h));
    h.app.shutdown_core().unwrap();
}

#[test]
fn compact_tabs_show_one_view_and_opening_a_chat_reveals_its_tracks() {
    let mut h = Harness::new();
    h.app.queue_open = true;
    let tab = |h: &Harness, pane| -> egui::Rect {
        h.ctx
            .data(|d| d.get_temp(egui::Id::new(("test-pane", pane))))
            .unwrap()
    };
    h.click(tab(&h, WorkspacePane::Queue).center());
    assert_eq!(h.app.pane, WorkspacePane::Queue);
    assert!(h.ctx.memory(|m| m.area_rect("compact-queue").is_none()));
    h.settle();
    assert!(h.ctx.read_response(egui::Id::new("track-search")).is_none());
    h.click(tab(&h, WorkspacePane::Library).center());
    assert_eq!(h.app.pane, WorkspacePane::Library);
    h.frame(vec![]);
    let search = h
        .ctx
        .read_response(egui::Id::new("chat-search"))
        .unwrap()
        .rect;
    h.click(egui::pos2(search.left() + 22., search.bottom() + 36.));
    assert_eq!(h.app.pane, WorkspacePane::Playlist);
    h.frame(vec![]);
    assert!(h.ctx.read_response(egui::Id::new("track-search")).is_some());
    h.app.shutdown_core().unwrap();
}

#[test]
fn short_workspace_keeps_a_full_track_row_above_the_player() {
    let mut h = Harness::new();
    h.size = egui::vec2(650., 260.);
    h.app.test_rows.clear();
    for _ in 0..2 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, h.size)),
            ..Default::default()
        };
        let mut output = h.ctx.run_ui(input, |ui| {
            window_header(ui, None, true);
            h.app.render(ui);
        });
        output.textures_delta.clear();
    }
    let track = h.app.test_rows[&0].0;
    let transport = egui::panel::PanelState::load(&h.ctx, egui::Id::new("transport")).unwrap();
    assert!(track.bottom() <= transport.outer_rect.top(), "{track:?}");
}

#[test]
fn scrollbars_stay_outside_the_track_rows_and_menus() {
    let mut h = queue_harness(24);
    for width in [1240., 900.] {
        h.size.x = width;
        h.app.pane = WorkspacePane::Playlist;
        h.app.test_rows.clear();
        for _ in 0..10 {
            h.frame(vec![]);
        }
        let bar = |ctx: &egui::Context, key| {
            let scroll: egui::Id = ctx.data(|d| d.get_temp(egui::Id::new(key))).unwrap();
            ctx.read_response(scroll.with(1)).unwrap().rect
        };
        let playlist = bar(&h.ctx, "test-playlist-scroll");
        for (row, menu) in h.app.test_rows.values() {
            assert!(row.right() <= playlist.left() && menu.right() <= playlist.left());
            assert!((row.right() - menu.right() - TRACK_ROW_PADDING).abs() < 1.);
        }
        h.app.pane = WorkspacePane::Queue;
        for _ in 0..10 {
            h.frame(vec![]);
        }
        let queue = bar(&h.ctx, "test-queue-scroll");
        for (row, menu, _) in h.app.test_queue_rows.values() {
            assert!(row.right() <= queue.left() && menu.right() <= queue.left());
        }
    }
}

#[test]
fn sidebar_search_reveals_matching_chats_and_escape_restores_pins() {
    let mut h = Harness::new();
    h.app.pane = WorkspacePane::Library;
    h.settle();
    let visible = |output: &egui::FullOutput, title| {
        output.shapes.iter().any(|painted| {
            matches!(&painted.shape, egui::Shape::Text(text)
                if text.galley.text() == title
                    && text.visual_bounding_rect().intersect(painted.clip_rect).is_positive())
        })
    };
    let pinned = h.frame(vec![]);
    assert!(visible(&pinned, "Jazz & coffee"));
    assert!(!visible(&pinned, "Saved Messages"));

    let search = h.ctx.read_response(egui::Id::new("chat-search")).unwrap();
    h.click(search.rect.center());
    h.settle();
    let all = h.frame(vec![]);
    assert!(visible(&all, "Saved Messages"));
    assert!(visible(&all, "Electronic archive"));

    let filtered = h.frame(vec![egui::Event::Text("Electronic".into())]);
    assert!(visible(&filtered, "Electronic archive"));
    assert!(!visible(&filtered, "Saved Messages"));

    h.key(egui::Key::Escape);
    h.settle();
    let pinned_again = h.frame(vec![]);
    assert!(h.app.chat_query.is_empty());
    assert!(!h.app.chat_search_active);
    assert!(visible(&pinned_again, "Jazz & coffee"));
    assert!(!visible(&pinned_again, "Electronic archive"));
    h.app.shutdown_core().unwrap();
}

#[test]
fn offline_confirmation_supports_cancel_and_confirm_from_the_keyboard() {
    let mut h = Harness::new();
    h.size = egui::vec2(365., 650.);
    crate::demo::apply(&mut h.app, "offline-storage");
    let id = h.app.state.view().playlists[0].tracks[0].id();
    h.app.offline_confirm = Some(vec![id.chat]);
    h.settle();

    h.key(egui::Key::Enter);
    assert!(h.app.settings_open);
    assert!(h.app.offline_confirm.is_none());
    assert!(h.app.state.offline().path(id).is_some());

    h.app.offline_confirm = Some(vec![id.chat]);
    h.settle();
    h.key(egui::Key::Tab);
    h.key(egui::Key::Enter);
    assert!(h.app.settings_open);
    assert!(h.app.offline_confirm.is_none());
    assert!(h.app.state.offline().path(id).is_none());
    assert!(h.app.state.offline().copies[&id].cached);
}
