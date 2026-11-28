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
    assert_eq!(texts(None), [(APP_NAME.into(), false)]);
    assert_eq!(
        texts(Some(&long)),
        [(long.clone(), true), (format!("— {APP_NAME}"), false)]
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
    h.app.set_queue_panel_open(true);
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
    let library_visible = |h: &Harness| {
        h.ctx
            .read_response(egui::Id::new(CHAT_SEARCH_TRIGGER))
            .is_some()
    };
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
    h.app.set_queue_panel_open(true);
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
        .read_response(egui::Id::new(CHAT_SEARCH_TRIGGER))
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
fn track_lists_reach_the_player_divider() {
    let mut h = queue_harness(30);
    for width in [1240., 900.] {
        h.size.x = width;
        for (pane, key) in [
            (WorkspacePane::Playlist, "test-playlist-scroll"),
            (WorkspacePane::Queue, "test-queue-scroll"),
        ] {
            h.app.pane = pane;
            h.settle();
            let (viewport, _) = h.list_scroll(key);
            let transport =
                egui::panel::PanelState::load(&h.ctx, egui::Id::new("transport")).unwrap();
            let scroll: egui::Id = h
                .ctx
                .data(|data| data.get_temp(egui::Id::new(key)).unwrap());
            let bar = h.ctx.read_response(scroll.with(1)).unwrap().rect;
            let divider = transport.outer_rect.top();
            assert!(
                (viewport.bottom() - divider).abs() <= 1.,
                "{key} viewport at width {width}"
            );
            assert!(
                (bar.bottom() - divider).abs() <= 1.,
                "{key} scrollbar at width {width}"
            );
        }
    }
}

#[test]
fn scrollbar_markers_follow_the_current_track_in_both_lists() {
    let mut h = queue_harness(30);
    let mut state = h.app.state.view().clone();
    state.playlists[0].tracks[1].kind = gramwave_domain::MediaKind::Document;
    h.app.state = gramwave_application::AppCore::new(state);
    let tracks = h.app.state.view().playlists[0].tracks.clone();
    h.app.start_session(&tracks, 0);
    h.settle();

    let (playlist, mark) = current_track_marker(&mut h, "test-playlist-scroll");
    let mark = mark.unwrap();
    let stride = design::ROW_HEIGHT + 8.;
    let expected = playlist.height() * design::ROW_HEIGHT
        / ((tracks.len() - 1) as f32 * stride + design::ROW_HEIGHT);
    assert_eq!(mark.width(), playlist.width());
    assert!((mark.height() - expected).abs() < 0.01);
    assert!((mark.top() - playlist.top()).abs() < 0.01);
    assert!((mark.right() - playlist.right()).abs() < 0.01);

    h.app.execute(AppCommand::Next);
    let (_, next) = current_track_marker(&mut h, "test-playlist-scroll");
    assert!(next.unwrap().top() > mark.bottom());

    h.app.execute(AppCommand::Filters(0, [true, false, false]));
    h.settle();
    let (bar, filtered) = current_track_marker(&mut h, "test-playlist-scroll");
    assert!(bar.is_positive());
    assert!(filtered.is_none());
    h.app.execute(AppCommand::Filters(0, [true, true, false]));
    h.app.search = "nonexistent track".into();
    h.settle();
    assert!(current_track_marker(&mut h, "test-playlist-scroll")
        .1
        .is_none());
    h.app.search.clear();
    h.app.opened_chat = Some(1);
    h.settle();
    assert!(current_track_marker(&mut h, "test-playlist-scroll")
        .1
        .is_none());

    let repeated = vec![tracks[0].clone(); 400];
    h.app.start_session(&repeated, 0);
    h.settle();
    let (queue, first) = current_track_marker(&mut h, "test-queue-scroll");
    let first = first.unwrap();
    assert_eq!(first.width(), queue.width());
    assert!((first.height() - 4.).abs() < 0.01);
    assert!((first.top() - queue.top()).abs() < 0.01);

    h.app.start_session(&repeated, repeated.len() - 1);
    h.settle();
    let (queue, last) = current_track_marker(&mut h, "test-queue-scroll");
    let last = last.unwrap();
    assert!((last.height() - 4.).abs() < 0.01);
    assert!((last.bottom() - queue.bottom()).abs() < 0.01);
}

#[test]
fn scrollbar_marker_changes_to_the_contrast_color_only_under_the_thumb() {
    for light in [false, true] {
        let mut h = queue_harness(30);
        let mut state = h.app.state.view().clone();
        state.light = light;
        let tracks = state.playlists[0].tracks.clone();
        h.app.state = gramwave_application::AppCore::new(state);
        let current = 20;
        h.app.start_session(&tracks, current);
        h.settle();
        let (bar, marker) = current_track_marker(&mut h, "test-playlist-scroll");
        let marker = marker.unwrap();
        let id = h.ctx.data(|data| {
            data.get_temp(egui::Id::new("test-playlist-scroll"))
                .unwrap()
        });
        let row_center = current as f32 * (design::ROW_HEIGHT + 8.) + design::ROW_HEIGHT / 2.;
        let resting_offset = row_center - bar.height() / 2.;
        let visuals = if light {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        for (step, (offset, overlap_fraction)) in [
            (0., 0.),
            (row_center - bar.height(), 0.5),
            (resting_offset, 1.),
            (resting_offset, 1.),
            (resting_offset, 1.),
        ]
        .into_iter()
        .enumerate()
        {
            let mut scroll = egui::scroll_area::State::load(&h.ctx, id).unwrap();
            scroll.offset.y = offset;
            scroll.store(&h.ctx, id);
            let (events, thumb_color) = match step {
                3 => (
                    vec![egui::Event::PointerMoved(marker.center())],
                    visuals.widgets.hovered.fg_stroke.color,
                ),
                4 => {
                    h.pointer(marker.center(), true);
                    (
                        vec![egui::Event::PointerMoved(
                            marker.center() + egui::vec2(0., 8.),
                        )],
                        visuals.widgets.active.fg_stroke.color,
                    )
                }
                _ => (vec![], visuals.widgets.inactive.fg_stroke.color),
            };
            let output = h.frame(events);
            let shapes: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|painted| match &painted.shape {
                    egui::Shape::Rect(rect) if bar.contains_rect(rect.rect) => {
                        Some((painted.clip_rect, rect))
                    }
                    _ => None,
                })
                .collect();
            let index = shapes
                .iter()
                .position(|(_, rect)| rect.rect == marker)
                .unwrap();
            let (_, base) = shapes[index];
            assert_eq!(base.fill, design::palette(light).selected);
            let (_, thumb) = shapes[index - 1];
            assert_eq!(thumb.fill, thumb_color);
            if step == 4 {
                assert!(h.list_scroll("test-playlist-scroll").1 > resting_offset);
            }
            let fraction = thumb.rect.intersect(marker).height().max(0.) / marker.height();
            assert!((fraction - overlap_fraction).abs() < 0.1, "{fraction}");
            let overlap = shapes.get(index + 1);
            if overlap_fraction == 0. {
                assert!(overlap.is_none());
            } else {
                let (clip, overlap) = overlap.unwrap();
                assert_eq!(*clip, marker);
                assert_eq!(overlap.rect, thumb.rect);
                let contrast = if light {
                    egui::Color32::from_rgb(80, 141, 114)
                } else {
                    egui::Color32::from_rgb(64, 111, 96)
                };
                assert_eq!(overlap.fill, contrast);
            }
        }
    }
}

fn current_track_marker(h: &mut Harness, key: &str) -> (egui::Rect, Option<egui::Rect>) {
    let output = h.frame(vec![]);
    let id: egui::Id = h
        .ctx
        .data(|data| data.get_temp(egui::Id::new(key)))
        .unwrap();
    let bar = h
        .ctx
        .read_response(id.with(1))
        .map_or(egui::Rect::NOTHING, |response| response.rect);
    let color = design::palette(h.app.state.view().light).selected;
    let mut markers = output.shapes.into_iter().filter_map(|painted| {
        let egui::Shape::Rect(shape) = painted.shape else {
            return None;
        };
        (shape.fill == color && bar.contains_rect(shape.rect)).then_some(shape.rect)
    });
    let marker = markers.next();
    assert!(markers.next().is_none());
    (bar, marker)
}

#[test]
fn scrollbars_stay_outside_the_rows_and_menus() {
    let mut h = queue_harness(24);
    let mut state = h.app.state.view().clone();
    state.playlists.extend((5..30).map(|id| crate::Playlist {
        id,
        title: format!("Chat {id}"),
        personal: gramwave_application::model::PersonalChat {
            lists: [true, false],
            ..Default::default()
        },
        ..Default::default()
    }));
    h.app.state = gramwave_application::AppCore::new(state);
    h.app.chat_search_active = true;
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
        h.app.pane = WorkspacePane::Library;
        h.settle();
        let (scroll, rows): (egui::Id, egui::Rect) = h
            .ctx
            .data(|d| d.get_temp(egui::Id::new("test-chat-scroll")))
            .unwrap();
        let chats = h.ctx.read_response(scroll.with(1)).unwrap().rect;
        assert_eq!(chats.width(), playlist.width());
        assert_eq!(chats.left() - rows.right(), 4.);
        if width == 1240. {
            let sidebar = egui::panel::PanelState::load(&h.ctx, egui::Id::new("sidebar")).unwrap();
            let gap = sidebar.outer_rect.right() - chats.right();
            assert!((0. ..=1.).contains(&gap), "sidebar scrollbar gap: {gap}");
        }
    }
}

#[test]
fn sidebar_hover_fills_the_chat_row_and_preserves_the_open_chat() {
    let mut h = Harness::new();
    h.size = egui::vec2(1240., 800.);
    h.settle();
    let row = |h: &Harness, id| {
        h.ctx.data(|data| {
            data.get_temp::<egui::Rect>(egui::Id::new(("test-chat-row", id)))
                .unwrap()
        })
    };
    let other = row(&h, 2_i64);
    let opened = row(&h, 0_i64);
    let palette = design::palette(h.app.state.view().light);
    let fill = |frame: &egui::FullOutput, row: egui::Rect, color| {
        frame.shapes.iter().any(|painted| {
            matches!(&painted.shape, egui::Shape::Rect(rect)
                if rect.fill == color && rect.rect == row)
        })
    };
    h.frame(vec![egui::Event::PointerMoved(
        other.right_center() - egui::vec2(2., 0.),
    )]);
    h.settle();
    let frame = h.frame(vec![]);
    assert!(fill(&frame, other, palette.raised));
    assert!(fill(&frame, opened, palette.selected));

    h.frame(vec![egui::Event::PointerMoved(opened.center())]);
    h.settle();
    let frame = h.frame(vec![]);
    assert!(!fill(&frame, other, palette.raised));
    assert!(fill(&frame, opened, palette.selected));
}

#[test]
fn chat_search_and_playlist_rows_share_the_same_edges_with_or_without_scroll() {
    let mut h = Harness::new();
    h.app.pane = WorkspacePane::Library;
    for width in [1240., 900.] {
        h.size.x = width;
        for height in [800., 430.] {
            h.size.y = height;
            for active in [false, true] {
                h.app.chat_search_active = active;
                h.settle();
                let (search, row): (egui::Rect, egui::Rect) = h.ctx.data(|data| {
                    (
                        data.get_temp(egui::Id::new(("test-search-rect", "chat-search")))
                            .unwrap(),
                        data.get_temp(egui::Id::new(("test-chat-row", 0_i64)))
                            .unwrap(),
                    )
                });
                assert!((search.left() - row.left()).abs() < 1.);
                assert!(
                    (search.right() - row.right()).abs() < 1.,
                    "{width} {height} {search:?} {row:?}"
                );
            }
        }
    }
}

#[test]
fn search_warms_chat_avatars_before_scrolling_to_them() {
    use gramwave_application::{library::LibraryCommand, media::MediaPlaylist};
    use std::time::{Duration, Instant};

    let mut h = Harness::new();
    h.app.pane = WorkspacePane::Library;
    h.app.chat_search_active = true;
    h.app.state = gramwave_application::AppCore::new(Default::default());
    let covers = tempfile::tempdir().unwrap();
    let image = image::RgbImage::from_pixel(8, 8, image::Rgb([30, 90, 160]));
    let mut paths = std::collections::HashMap::new();
    for chat in 100..180_i64 {
        let path = covers.path().join(format!("{chat}.png"));
        image.save(&path).unwrap();
        paths.insert(chat, path.to_str().unwrap().to_owned());
    }
    let (sent, requests) = std::sync::mpsc::channel();
    let tg = crate::adapters::telegram::Telegram::mock_rpc(move |request| {
        assert_eq!(request["@type"], "downloadFile", "{request}");
        let id = request["file_id"].as_i64().unwrap();
        sent.send(id).unwrap();
        serde_json::json!({
            "@type":"file", "id":id,
            "local":{"path": paths[&id], "is_downloading_completed":true}
        })
    });
    h.app
        .library_command(LibraryCommand::Connected(tg.connection));
    for chat in 100..180_i64 {
        h.app.library_command(LibraryCommand::Source(
            gramwave_application::telegram::TelegramUpdate::Chat(
                MediaPlaylist {
                    id: chat,
                    title: format!("Chat {chat}"),
                    image_file: if chat < 112 { 0 } else { chat },
                    done: [true; 3],
                    ..Default::default()
                },
                Some([true, false]),
            ),
        ));
    }
    h.app.tg = Some(tg);
    h.frame(vec![]);
    assert_eq!(h.app.state.library().media_requests_in_flight(), 4);
    let deadline = Instant::now() + Duration::from_secs(5);
    let texture = loop {
        h.app.process(&h.ctx);
        h.frame(vec![]);
        if let Some(texture) = h.app.chat_cover(&h.ctx, 179) {
            break texture.id;
        }
        assert!(
            Instant::now() < deadline,
            "distant search avatar did not warm"
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    let (scroll, viewport): (egui::Id, egui::Rect) = h
        .ctx
        .data(|data| data.get_temp(egui::Id::new("test-chat-scroll")).unwrap());
    let mut state = egui::scroll_area::State::load(&h.ctx, scroll).unwrap();
    assert_eq!(state.offset.y, 0.);
    let last: egui::Rect = h.ctx.data(|data| {
        data.get_temp(egui::Id::new(("test-chat-row", 179_i64)))
            .unwrap()
    });
    state.offset.y = last.bottom() - viewport.bottom();
    state.store(&h.ctx, scroll);
    let frame = h.frame(vec![]);
    assert!(frame.shapes.iter().any(|painted| {
        matches!(&painted.shape, egui::Shape::Rect(rect)
            if viewport.contains(rect.rect.center())
                && rect.brush.as_ref().is_some_and(|brush| brush.fill_texture_id == texture))
    }));
    assert_eq!(requests.try_iter().filter(|&id| id == 179).count(), 1);
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

    let search = h
        .ctx
        .read_response(egui::Id::new(CHAT_SEARCH_TRIGGER))
        .unwrap();
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
fn recent_chats_follow_sidebar_search_and_can_be_removed_or_cleared() {
    let mut h = Harness::new();
    let search = |h: &mut Harness| {
        h.app.pane = WorkspacePane::Library;
        h.settle();
        let field = h
            .ctx
            .read_response(egui::Id::new(CHAT_SEARCH_TRIGGER))
            .unwrap();
        h.click(field.rect.center());
        h.settle();
        h.frame(vec![])
    };
    let positions = |output: &egui::FullOutput, title| {
        output
            .shapes
            .iter()
            .filter_map(|painted| {
                if let egui::Shape::Text(text) = &painted.shape {
                    let rect = text.visual_bounding_rect().intersect(painted.clip_rect);
                    if text.galley.text() == title && rect.is_positive() {
                        return Some(rect.center());
                    }
                }
                None
            })
            .collect::<Vec<egui::Pos2>>()
    };
    let saved = "Saved Messages";
    let electronic = "Electronic archive";
    h.app.pane = WorkspacePane::Library;
    h.settle();
    let pinned = h.frame(vec![]);
    h.click(positions(&pinned, "Jazz & coffee")[0]);
    assert!(h.app.state.view().recent_chats.is_empty());

    let all = search(&mut h);
    assert!(positions(&all, "Recent").is_empty());
    h.click(positions(&all, saved)[0]);
    assert_eq!(h.app.state.view().recent_chats, [1]);

    let all = search(&mut h);
    assert_eq!(positions(&all, saved).len(), 1);
    assert!(positions(&all, saved)[0].y < positions(&all, electronic)[0].y);
    let results = h.frame(vec![egui::Event::Text("Electronic".into())]);
    assert!(positions(&results, "Recent").is_empty());
    h.click(positions(&results, electronic)[0]);
    assert_eq!(h.app.state.view().recent_chats, [3, 1]);

    let all = search(&mut h);
    h.click(positions(&all, saved)[0]);
    assert_eq!(h.app.state.view().recent_chats, [1, 3]);

    search(&mut h);
    let results = h.frame(vec![egui::Event::Text("Saved".into())]);
    assert!(positions(&results, "Recent").is_empty());
    assert_eq!(positions(&results, saved).len(), 1);
    for _ in 0..5 {
        h.key(egui::Key::Backspace);
    }
    assert!(h.app.chat_query.is_empty());
    let all = h.frame(vec![]);
    assert_eq!(positions(&all, "Recent").len(), 1);
    let pos = positions(&all, saved)[0];
    for pressed in [true, false] {
        h.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
    }
    let menu = h.frame(vec![]);
    h.click(positions(&menu, "Remove from Recent")[0]);
    assert_eq!(h.app.state.view().recent_chats, [3]);
    let all = h.frame(vec![]);
    assert_eq!(positions(&all, saved).len(), 1);
    assert!(positions(&all, electronic)[0].y < positions(&all, saved)[0].y);

    let clear_id = h.ctx.data(|data| {
        data.get_temp::<egui::Id>(egui::Id::new("test-clear-recent-chats"))
            .unwrap()
    });
    h.click(h.ctx.read_response(clear_id).unwrap().rect.center());
    let all = h.frame(vec![]);
    assert!(h.app.state.view().recent_chats.is_empty());
    assert!(positions(&all, "Recent").is_empty());
    assert_eq!(positions(&all, electronic).len(), 1);
    let mut state = h.app.state.view().clone();
    state.playlists.push(crate::Playlist {
        id: 99,
        title: "Saved public channel".into(),
        ..Default::default()
    });
    state.recent_chats.push(99);
    h.app.state = gramwave_application::AppCore::new(state);
    let all = h.frame(vec![]);
    assert!(positions(&all, "Recent").is_empty());
    assert!(positions(&all, "Saved public channel").is_empty());
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
