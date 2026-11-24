use super::*;
#[cfg(test)]
mod regression {
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
    fn long_metadata_fits_virtual_list_row_height() {
        let ctx = egui::Context::default();
        design::style(&ctx, false);
        let long = "A very long title or artist name ".repeat(40);
        let mut height = 0.;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(600.);
            let row = egui::Frame::new().inner_margin(6.).show(ui, |ui| {
                ui.set_min_height(48.);
                ui.horizontal(|ui| {
                    track_labels(ui, &long, &long, false);
                });
            });
            height = row.response.rect.height();
        });
        output.textures_delta.clear();
        assert!(
            (height - design::ROW_HEIGHT).abs() < 0.1,
            "Rendered row {height} differs from virtual row height 60"
        );
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use crate::application::ports::{PlaybackEvent, PlayerBackend, PlayerCommand, PlayerSnapshot};
    use crate::Track;
    use tg_player_application::AppError;

    struct Clock(PlayerSnapshot);

    impl PlayerBackend for Clock {
        fn command(&self, _: PlayerCommand) -> Result<(), AppError> {
            Ok(())
        }

        fn drain(&self) -> Vec<PlaybackEvent> {
            Vec::new()
        }

        fn snapshot(&self) -> PlayerSnapshot {
            self.0
        }
    }

    struct Harness {
        app: App,
        ctx: egui::Context,
        time: f64,
        size: egui::Vec2,
    }
    impl Harness {
        fn new() -> Self {
            let root = std::path::PathBuf::new();
            let mut app = App::new_at(root, true);
            app.player = None;
            app.opened_chat = Some(0);
            let ctx = egui::Context::default();
            design::install(&ctx);
            ctx.set_fonts(fastframe_fonts::FontSetup::default().definitions());
            let mut h = Self {
                app,
                ctx,
                time: 0.,
                size: egui::vec2(850., 550.),
            };
            h.frame(vec![]);
            h.frame(vec![]);
            h
        }
        fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
            self.time += 0.02;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut output = self.ctx.run_ui(input, |ui| self.app.render(ui));
            output.textures_delta.clear();
            output
        }
        fn settle(&mut self) {
            for _ in 0..10 {
                self.frame(vec![]);
            }
        }
        fn pointer(&mut self, p: egui::Pos2, pressed: bool) {
            self.frame(vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        fn click(&mut self, p: egui::Pos2) {
            self.pointer(p, true);
            self.pointer(p, false);
        }
        fn key(&mut self, key: egui::Key) {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]);
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
    fn resizing_slides_the_sidebar_as_the_track_view_reflows() {
        let mut h = Harness::new();
        let tracks_left = |h: &Harness| {
            h.ctx
                .read_response(egui::Id::new("track-search"))
                .unwrap()
                .rect
                .left()
        };
        let compact = tracks_left(&h);
        h.size.x = 1100.;
        h.frame(vec![]);
        h.frame(vec![]);
        let opening = tracks_left(&h);
        h.settle();
        let docked = tracks_left(&h);
        assert!(
            compact < opening && opening < docked,
            "{compact} < {opening} < {docked}"
        );

        h.size.x = 1039.;
        h.frame(vec![]);
        h.frame(vec![]);
        let closing = tracks_left(&h);
        h.settle();
        assert!(
            compact < closing && closing < docked,
            "{compact} < {closing} < {docked}"
        );
        assert_eq!(tracks_left(&h), compact);
        assert!(h.ctx.read_response(egui::Id::new("chat-search")).is_none());
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
        let marker = |output: egui::FullOutput| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill == design::palette(false).selected => {
                        Some(rect.rect)
                    }
                    _ => None,
                })
                .unwrap()
        };
        let moving = marker(h.frame(vec![]));
        let tracks = tab(&h, WorkspacePane::Playlist);
        let queue = tab(&h, WorkspacePane::Queue);
        assert!(tracks.left() < moving.left() && moving.left() < queue.left());
        h.settle();
        let settled = marker(h.frame(vec![]));
        assert!((settled.min - queue.min).length() < 1.);
        assert!((settled.max - queue.max).length() < 1.);
        h.frame(vec![]);
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
    fn queue_drag_from_artwork_moves_neighbours_before_committing() {
        let mut h = queue_harness(5);
        let order = queue_order(&h);
        let first = h.app.test_queue_rows[&order[0]].0;
        let second = h.app.test_queue_rows[&order[1]].0;
        let third = h.app.test_queue_rows[&order[2]].0;
        let current = h.app.state.view().session.current().unwrap().entry_id;
        let grab = egui::pos2(first.left() + 24., first.center().y);
        let target = egui::pos2(grab.x, third.center().y);
        h.pointer(grab, true);
        h.frame(vec![egui::Event::PointerMoved(target)]);
        h.frame(vec![]);
        assert_eq!(queue_order(&h), order);
        assert!(!h.app.test_queue_rows.contains_key(&order[0]));
        let shifted = h.app.test_queue_rows[&order[1]].0.top();
        assert!(first.top() < shifted && shifted < second.top());

        h.pointer(target, false);
        assert_eq!(queue_order(&h), [order[1], order[2], order[0], order[3]]);
        assert_eq!(
            h.app.state.view().session.current().unwrap().entry_id,
            current
        );
        h.app.shutdown_core().unwrap();
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
    fn playlist_heading_wraps_at_word_boundaries() {
        let mut h = Harness::new();
        h.size = egui::vec2(720., 620.);
        let title = "Evening piano improvisations from the mountain studio";
        let mut state = h.app.state.view().clone();
        state.playlists[0].title = title.into();
        h.app.state = tg_player_application::AppCore::new(state);
        let mut output = h.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, h.size)),
                time: Some(h.time + 0.02),
                ..Default::default()
            },
            |ui| h.app.render(ui),
        );
        output.textures_delta.clear();
        let heading = output
            .shapes
            .into_iter()
            .find_map(|shape| match shape.shape {
                egui::Shape::Text(text)
                    if text.galley.text() == title && text.galley.rows.len() == 2 =>
                {
                    Some(text.galley)
                }
                _ => None,
            })
            .expect("the playlist heading wraps to two lines");
        let lines: Vec<_> = heading
            .rows
            .iter()
            .map(|row| row.glyphs.iter().map(|glyph| glyph.chr).collect::<String>())
            .collect();
        assert_eq!(
            format!("{} {}", lines[0].trim_end(), lines[1].trim_start()),
            title
        );
    }

    #[test]
    fn playlist_status_follows_the_last_visible_title_row() {
        let mut h = Harness::new();
        h.size = egui::vec2(720., 620.);
        h.app.custom_emoji.set_demo();
        let wrapped = "Evening piano improvisations from the mountain studio";
        for (title, rows, elided) in [
            ("Quiet evening".into(), 1, false),
            (wrapped.into(), 2, false),
            (wrapped.repeat(4), 2, true),
        ] {
            let mut state = h.app.state.view().clone();
            state.playlists[0].title = title.clone();
            state.playlists[0].emoji_status = Some(tg_player_application::model::EmojiStatus {
                id: crate::adapters::emoji::DEMO_ID,
                expires_at: 0,
            });
            h.app.state = tg_player_application::AppCore::new(state);
            h.time += 0.1;
            let mut output = h.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, h.size)),
                    time: Some(h.time),
                    ..Default::default()
                },
                |ui| h.app.render(ui),
            );
            output.textures_delta.clear();
            let heading = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == title => Some(text),
                    _ => None,
                })
                .unwrap();
            assert_eq!(heading.galley.rows.len(), rows);
            assert_eq!(heading.galley.elided, elided);
            let line = heading
                .galley
                .rows
                .last()
                .unwrap()
                .rect()
                .translate(heading.pos.to_vec2());
            let badge = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh) => {
                        let bounds = mesh.calc_bounds();
                        (bounds.size() == egui::Vec2::splat(24.)).then_some(bounds)
                    }
                    _ => None,
                })
                .expect("animated status texture");
            assert!((badge.left() - line.right() - 8.).abs() < 1.);
            assert!((badge.center().y - line.center().y).abs() < 1.);
            assert!(badge.right() <= h.size.x - 24.);
        }
    }

    #[test]
    fn playlist_feedback_is_inline_and_elided_titles_have_no_tooltip() {
        let mut h = Harness::new();
        h.app.set_inline_notice("Link copied".into(), false);
        h.frame(vec![]);
        let row = h.app.test_rows[&0].0;
        let title = h.app.playlist_tracks(0)[0].title.clone();
        let pointer = egui::pos2(row.left() + 110., row.top() + 18.);
        h.frame(vec![egui::Event::PointerMoved(pointer)]);
        let mut output = h.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, h.size)),
                time: Some(h.time + 1.),
                ..Default::default()
            },
            |ui| h.app.render(ui),
        );
        output.textures_delta.clear();
        let mut titles = 0;
        let mut feedback = None;
        for shape in output.shapes {
            if let egui::Shape::Text(text) = shape.shape {
                if text.galley.text() == title {
                    titles += 1;
                } else if text.galley.text() == "Link copied" {
                    feedback = Some(text.pos);
                }
            }
        }
        assert_eq!(titles, 1, "the title should only be drawn in its row");
        let feedback = feedback.expect("copy confirmation should remain visible");
        assert!(feedback.y < row.top() && feedback.x >= row.left());
    }

    #[test]
    fn sidebar_search_keeps_its_width_and_escape_clears_the_query() {
        let mut h = Harness::new();
        h.app.pane = WorkspacePane::Library;
        h.frame(vec![]);
        h.frame(vec![]);
        let id = egui::Id::new("chat-search");
        let initial = h.ctx.read_response(id).unwrap().rect;
        h.click(initial.center());
        h.frame(vec![egui::Event::Text("music".into())]);
        assert!(h.app.chat_search_active);
        assert_eq!(h.app.chat_query, "music");
        assert_eq!(h.ctx.read_response(id).unwrap().rect, initial);
        h.key(egui::Key::Escape);
        assert!(!h.app.chat_search_active);
        assert!(h.app.chat_query.is_empty());
    }

    #[test]
    fn track_search_keeps_its_width_and_escape_clears_the_query() {
        let mut h = Harness::new();
        let id = egui::Id::new("track-search");
        let field = h.ctx.read_response(id).unwrap().rect;
        h.click(field.center());
        h.frame(vec![egui::Event::Text("Afterglow".into())]);
        assert_eq!(h.app.search, "Afterglow");
        assert_eq!(h.ctx.read_response(id).unwrap().rect, field);
        h.key(egui::Key::Escape);
        assert!(h.app.search.is_empty());
    }

    #[test]
    fn smallest_player_keeps_basic_playback_controls() {
        let mut h = queue_harness(3);
        h.size = egui::Vec2::from(crate::launch::MIN_WINDOW_SIZE);
        h.frame(vec![]);
        h.frame(vec![]);
        let first = h.app.state.view().session.current().unwrap().entry_id;
        let center = h.size.to_pos2() / 2.;
        h.click(center + egui::vec2(48., 0.));
        assert_ne!(
            h.app.state.view().session.current().unwrap().entry_id,
            first
        );
        h.click(center - egui::vec2(48., 0.));
        assert_eq!(
            h.app.state.view().session.current().unwrap().entry_id,
            first
        );
        assert!(h.ctx.read_response(egui::Id::new("track-search")).is_none());
    }

    #[test]
    fn compact_transport_volume_supports_drag_keyboard_and_mute() {
        let volume_id = egui::Id::new("test-volume");
        for (width, height) in [(760., 190.), (820., 320.), (1200., 190.)] {
            let mut h = Harness::new();
            h.size = egui::vec2(width, height);
            h.frame(vec![]);
            h.frame(vec![]);
            let (slider, id): (egui::Rect, egui::Id) =
                h.ctx.data(|data| data.get_temp(volume_id)).unwrap();
            assert!(slider.right() <= width / 2. + 190.);
            assert!(slider.bottom() <= height);
            h.pointer(slider.center(), true);
            let lower = slider.center() - egui::vec2(22., 0.);
            h.frame(vec![egui::Event::PointerMoved(lower)]);
            h.pointer(lower, false);
            let volume = h.app.state.view().volume;
            assert!((25. ..30.).contains(&volume));
            assert!(h.ctx.memory(|memory| memory.has_focus(id)));
            h.key(egui::Key::ArrowRight);
            assert_eq!(h.app.state.view().volume, volume + 1.);
            let mute = egui::pos2(slider.left() - 26., slider.center().y);
            h.click(mute);
            assert_eq!(h.app.state.view().volume, 0.);
            h.click(mute);
            assert_eq!(h.app.state.view().volume, volume + 1.);
        }

        let mut h = Harness::new();
        for size in [
            egui::vec2(320., 190.),
            crate::launch::MIN_WINDOW_SIZE.into(),
        ] {
            h.size = size;
            h.ctx
                .data_mut(|data| data.remove::<(egui::Rect, egui::Id)>(volume_id));
            h.frame(vec![]);
            assert!(h
                .ctx
                .data(|data| data.get_temp::<(egui::Rect, egui::Id)>(volume_id))
                .is_none());
        }
    }

    #[test]
    fn play_all_starts_the_playlist_when_the_search_has_no_results() {
        let mut h = Harness::new();
        h.size.y = 620.;
        let count = h.app.playlist_tracks(0).len();
        h.app.search = "nothing matches this".into();
        h.frame(vec![]);
        assert!(h.app.playlist_tracks(0).is_empty());
        let play: egui::Rect = h
            .ctx
            .data(|d| d.get_temp(egui::Id::new("test-play-all")))
            .unwrap();
        h.click(play.center());
        let session = &h.app.state.view().session;
        assert_eq!(session.current().unwrap().message, 0);
        assert_eq!(session.snapshot().len(), count);
    }

    #[test]
    fn playlist_pin_is_last_in_the_menu_after_the_sort_control() {
        let mut h = Harness::new();
        h.size.y = 620.;
        let render = |h: &mut Harness| {
            h.time += 0.02;
            let mut output = h.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, h.size)),
                    time: Some(h.time),
                    ..Default::default()
                },
                |ui| h.app.render(ui),
            );
            output.textures_delta.clear();
            output
                .shapes
                .into_iter()
                .filter_map(|shape| match shape.shape {
                    egui::Shape::Text(text) => Some((
                        text.galley.text().to_owned(),
                        text.visual_bounding_rect().center(),
                    )),
                    _ => None,
                })
                .fold(
                    std::collections::HashMap::new(),
                    |mut positions, (label, center)| {
                        positions.entry(label).or_insert(center);
                        positions
                    },
                )
        };
        let toolbar = render(&mut h);
        assert!(!toolbar.contains_key("Unpin chat"));
        let menu = toolbar["…"];
        assert!(toolbar["Newest first"].x < menu.x);
        h.click(menu);
        let actions = render(&mut h);
        let unpin = actions["Unpin chat"];
        assert!(unpin.y > actions["Refresh library"].y);
        h.click(unpin);
        assert!(!h.app.state.view().chat_pins.contains(&0));
        h.click(menu);
        assert!(render(&mut h).contains_key("Pin chat"));
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
    fn sidebar_search_reveals_all_chats_then_restores_pinned_chats() {
        let mut h = Harness::new();
        h.app.pane = WorkspacePane::Library;
        let chat = |output: &egui::FullOutput, title| {
            output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == title => {
                    Some((text.pos.y, text.fallback_color.a()))
                }
                _ => None,
            })
        };
        let pinned = h.frame(vec![]);
        assert!(chat(&pinned, "Saved Messages").is_none());

        let search = h.ctx.read_response(egui::Id::new("chat-search")).unwrap();
        h.click(search.rect.center());
        let (enter_y, enter_opacity) = chat(&h.frame(vec![]), "Saved Messages").unwrap();
        h.settle();
        let (rest_y, rest_opacity) = chat(&h.frame(vec![]), "Saved Messages").unwrap();
        assert!(
            enter_y > rest_y && enter_opacity < rest_opacity,
            "enter ({enter_y}, {enter_opacity}), rest ({rest_y}, {rest_opacity})"
        );

        h.key(egui::Key::Escape);
        let pinned_again = h.frame(vec![]);
        let (exit_y, exit_opacity) = chat(&pinned_again, "Jazz & coffee").unwrap();
        h.settle();
        let settled = h.frame(vec![]);
        let (rest_y, rest_opacity) = chat(&settled, "Jazz & coffee").unwrap();
        assert!(
            exit_y > rest_y && exit_opacity < rest_opacity,
            "exit ({exit_y}, {exit_opacity}), rest ({rest_y}, {rest_opacity})"
        );
        assert!(chat(&settled, "Saved Messages").is_none());
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

    fn queue_harness(count: i64) -> Harness {
        let mut h = Harness::new();
        h.size = egui::vec2(1240., 800.);
        h.app.queue_open = true;
        h.app.execute(AppCommand::StartSession(
            (1..=count)
                .map(|message| tg_player_application::model::Track {
                    message,
                    title: format!("Track {message}"),
                    artist: "Artist".into(),
                    duration: 180,
                    ..Default::default()
                })
                .collect(),
            0,
        ));
        h.settle();
        h
    }

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
    fn choosing_a_search_result_rebuilds_queue_for_mouse_keyboard_and_menu() {
        for activation in ["mouse", "keyboard", "menu"] {
            let mut h = Harness::new();
            h.size = egui::vec2(1240., 800.);
            h.frame(vec![]);
            h.time += 1.;
            h.click(h.app.test_rows[&1].0.center());
            assert_eq!(
                h.app
                    .state
                    .view()
                    .session
                    .current()
                    .as_ref()
                    .unwrap()
                    .message,
                1
            );
            h.app.search = "Slow motion 05".into();
            h.app.test_rows.clear();
            h.frame(vec![]);
            h.time += 1.;
            h.frame(vec![]);
            assert_eq!(h.app.test_rows.len(), 1);
            let row = h.app.test_rows[&4].0;
            match activation {
                "keyboard" => {
                    h.ctx
                        .memory_mut(|m| m.request_focus(h.app.test_row_ids[&4]));
                    h.key(egui::Key::Enter);
                }
                "menu" => {
                    h.click(h.app.test_rows[&4].1.center());
                    h.frame(vec![]);
                    h.click(h.app.test_play_buttons[&4].center());
                }
                _ => h.click(row.center()),
            }
            assert_eq!(
                h.app
                    .state
                    .view()
                    .session
                    .current()
                    .as_ref()
                    .unwrap()
                    .message,
                4
            );
            h.app.execute(AppCommand::Previous);
            assert_eq!(
                h.app
                    .state
                    .view()
                    .session
                    .current()
                    .as_ref()
                    .unwrap()
                    .message,
                3
            );
            h.app.execute(AppCommand::Next);
            assert_eq!(
                h.app
                    .state
                    .view()
                    .session
                    .current()
                    .as_ref()
                    .unwrap()
                    .message,
                4
            );
            h.app.shutdown_core().unwrap();
        }
    }
    #[test]
    fn right_click_opens_track_menu_without_starting_playback() {
        let mut h = Harness::new();
        let row = h.app.test_rows[&0].0;
        let p = row.center();
        for pressed in [true, false] {
            h.frame(vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        assert!(h.app.state.view().session.current().is_none());
        assert!(egui::Popup::is_any_open(&h.ctx));
        h.key(egui::Key::Escape);
    }
    #[test]
    fn restored_track_fetches_artwork_without_starting_playback() {
        use crate::adapters::telegram::Telegram;
        use serde_json::json;
        use tg_player_application::{
            library::LibraryCommand,
            model::{Playlist, State},
        };

        let mut h = Harness::new();
        let track = Track {
            chat: 42,
            message: 100,
            duration: 180,
            ..Default::default()
        };
        let id = track.id();
        let mut state = State {
            playlists: vec![Playlist {
                id: 0,
                done: [true; 3],
                ..Default::default()
            }],
            ..Default::default()
        };
        state.session.start(&[track.metadata()], 0, false);
        state.session.set_position_ms(37_000);
        h.app.state = tg_player_application::AppCore::new(state);

        let cover = tempfile::NamedTempFile::new().unwrap();
        let path = cover.path().to_string_lossy().into_owned();
        let downloaded = path.clone();
        let tg = Telegram::mock_rpc(move |request| match request["@type"].as_str() {
            Some("getMessage") => {
                assert_eq!(request["chat_id"], 42);
                assert_eq!(request["message_id"], 100);
                json!({"id":100,"content":{"@type":"messageAudio","audio":{
                    "title":"Restored", "duration":180, "audio":{"id":90},
                    "album_cover_thumbnail":{"file":{"id":7}}
                }}})
            }
            Some("downloadFile") => {
                assert_eq!(request["file_id"], 7);
                json!({"@type":"file","id":7,"local":{"path":downloaded,"is_downloading_completed":true}})
            }
            _ => panic!("unexpected request {request}"),
        });
        h.app
            .library_command(LibraryCommand::Connected(tg.connection));
        h.app.tg = Some(tg);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while h.app.artwork_path(7).is_none() {
            h.frame(vec![]);
            h.app.process(&h.ctx);
            assert!(std::time::Instant::now() < deadline, "cover did not load");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(h.app.artwork_path(7), Some(path.as_str()));
        assert_eq!(h.app.state.view().session.current().unwrap().id(), id);
        assert!(h.app.state.playback().state().paused());
        assert_eq!(h.app.state.view().session.position_ms(), 37_000);
        h.app.shutdown_core().unwrap();
    }

    #[test]
    fn restored_track_displays_elapsed_time_before_play() {
        let mut h = Harness::new();
        crate::demo::apply(&mut h.app, "resumed");
        let mut output = h.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, h.size)),
                ..Default::default()
            },
            |ui| h.app.render(ui),
        );
        output.textures_delta.clear();
        assert!(h.app.state.playback().state().paused());
        assert!(output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "0:37")
        }));
        h.click(h.app.test_seek.unwrap().0.center());
        assert!(h.app.test_seeks.is_empty());
    }

    #[test]
    fn timeline_repaint_interval_accounts_for_track_duration_and_pixel_density() {
        assert_eq!(seek_repaint_interval(180., 600.), Duration::from_millis(75));
        assert_eq!(
            seek_repaint_interval(180., 1200.),
            Duration::from_micros(37_500)
        );
        assert_eq!(
            seek_repaint_interval(5., 600.),
            Duration::from_secs_f64(1. / 60.)
        );
        assert_eq!(
            seek_repaint_interval(3600., 600.),
            Duration::from_millis(200)
        );
    }

    #[test]
    fn timeline_only_requests_frames_while_visible_and_playing() {
        let mut h = Harness::new();
        crate::demo::apply(&mut h.app, "playing");
        h.app.player = Some(Box::new(Clock(PlayerSnapshot {
            duration: 180.,
            ..Default::default()
        })));
        h.settle();
        let delay = h.frame(vec![]).viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        let rect = h.app.test_seek.unwrap().0;
        assert_eq!(
            delay,
            seek_repaint_interval(180., design::slider_rail(rect).width())
        );

        h.app.execute(AppCommand::Pause(true));
        h.settle();
        assert_eq!(
            h.frame(vec![]).viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            Duration::MAX
        );

        h.app.execute(AppCommand::Pause(false));
        h.size.y = 140.;
        h.settle();
        assert_eq!(
            h.frame(vec![]).viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            Duration::MAX
        );
    }

    #[test]
    fn timeline_uses_audio_clock_until_seek_preview_or_backend_seek() {
        let mut h = Harness::new();
        h.click(h.app.test_rows[&0].0.center());
        assert!(!h.app.state.playback().state().paused());
        h.frame(vec![]);
        let (_, seek_id) = h.app.test_seek.unwrap();

        for (audio_position, seeking, preview, expected) in [
            (Some(30.), false, None, 35.),
            (Some(30.), true, None, 17.),
            (None, false, None, 17.),
            (Some(30.), false, Some(50.), 55.),
            (Some(-1.), false, None, 5.),
        ] {
            h.app.player = Some(Box::new(Clock(PlayerSnapshot {
                position: 12.,
                audio_position,
                duration: 180.,
                seeking,
                ..Default::default()
            })));
            h.app.seek_preview = preview;
            h.ctx.memory_mut(|m| m.request_focus(seek_id));
            let prior_seeks = h.app.test_seeks.len();
            h.key(egui::Key::ArrowRight);
            assert_eq!(h.app.test_seeks.len(), prior_seeks + 1);
            assert_eq!(*h.app.test_seeks.last().unwrap(), expected);
        }
    }

    #[test]
    fn row_mouse_keyboard_menu_and_seek_contract() {
        let mut h = Harness::new();
        let (row, menu) = h.app.test_rows[&0];
        assert!(
            row.right() <= 850. && row.bottom() < 454.,
            "row overflow: {row:?}"
        );
        assert_eq!(row.height(), design::ROW_HEIGHT);
        // Cover, title and blank row space all activate the same row.
        for x in [row.left() + 50., row.left() + 160., row.right() - 60.] {
            h.app.execute(AppCommand::Stop);
            h.time += 1.;
            h.click(egui::pos2(x, row.center().y));
            assert_eq!(
                h.app
                    .state
                    .view()
                    .session
                    .current()
                    .as_ref()
                    .map(|t| t.message),
                Some(0)
            );
            assert!(!h.app.state.playback().state().paused());
        }
        h.time += 1.;
        h.click(row.center());
        assert!(h.app.state.playback().state().paused());
        h.time += 1.;
        h.click(row.center());
        h.click(row.center());
        assert!(
            !h.app.state.playback().state().paused(),
            "double click must not pause after starting"
        );
        h.time += 1.;
        h.click(menu.center());
        assert!(
            !h.app.state.playback().state().paused(),
            "menu must not activate row"
        );
        h.key(egui::Key::Escape);
        h.time += 1.;
        h.click(row.center());
        assert!(h.app.state.playback().state().paused());
        h.key(egui::Key::Enter);
        assert!(!h.app.state.playback().state().paused());
        h.key(egui::Key::Space);
        assert!(
            h.app.state.playback().state().paused(),
            "Space on row toggles playback"
        );
        // Click seeks once; a drag previews until release, independent of mpv position.
        let (rect, id) = h.app.test_seek.unwrap();
        h.time += 1.;
        h.click(rect.center());
        assert_eq!(h.app.test_seeks.len(), 1);
        assert!((h.app.test_seeks[0] - 90.).abs() < 1.);
        h.time += 1.;
        h.pointer(rect.center(), true);
        h.frame(vec![egui::Event::PointerMoved(egui::pos2(
            rect.right() - 8.,
            rect.center().y,
        ))]);
        assert_eq!(h.app.test_seeks.len(), 1);
        assert!(h.app.seek_preview.unwrap() > 170.);
        h.frame(vec![]);
        assert!(h.app.seek_preview.unwrap() > 170.);
        h.pointer(egui::pos2(rect.right() - 8., rect.center().y), false);
        assert_eq!(h.app.test_seeks.len(), 2);
        assert!(h.app.seek_preview.is_none());
        h.ctx.memory_mut(|m| m.request_focus(id));
        h.app.seek_preview = Some(90.);
        h.key(egui::Key::Home);
        assert_eq!(*h.app.test_seeks.last().unwrap(), 0.);
        h.key(egui::Key::End);
        assert_eq!(*h.app.test_seeks.last().unwrap(), 180.);
        h.key(egui::Key::ArrowRight);
        assert_eq!(*h.app.test_seeks.last().unwrap(), 5.); // backend position remains zero in demo
        h.time += 1.;
        h.pointer(rect.center(), true);
        h.app
            .execute(crate::application::commands::AppCommand::Next);
        assert!(h.app.seek_preview.is_none());
        let before = h.app.test_seeks.len();
        h.pointer(rect.center(), false);
        assert_eq!(
            h.app.test_seeks.len(),
            before,
            "releasing an old drag must not seek the next track"
        );
        // Unknown duration disables seeking, even while holding an old slider.
        h.app.start_session(&[Track::default()], 0);
        let before = h.app.test_seeks.len();
        h.time += 1.;
        h.click(rect.center());
        assert_eq!(h.app.test_seeks.len(), before);
        // Space is text while a search field owns focus.
        let search = egui::Id::new("track-search");
        h.ctx.memory_mut(|m| m.request_focus(search));
        h.frame(vec![]);
        let paused = h.app.state.playback().state().paused();
        h.key(egui::Key::Space);
        assert_eq!(h.app.state.playback().state().paused(), paused);
        let mut reached_row = false;
        for _ in 0..60 {
            h.key(egui::Key::Tab);
            let focused = h.ctx.memory(|m| m.focused());
            if h.app.test_row_ids.values().any(|&id| Some(id) == focused) {
                reached_row = true;
                break;
            }
        }
        assert!(reached_row, "Tab must reach track rows");
        h.app.set_volume(32.);
        h.app.toggle_mute();
        assert_eq!(h.app.state.view().volume, 0.);
        h.app.toggle_mute();
        assert_eq!(h.app.state.view().volume, 32.);
    }
}
