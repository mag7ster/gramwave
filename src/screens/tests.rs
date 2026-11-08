use super::*;
#[cfg(test)]
mod regression {
    use super::*;
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
    use crate::Track;
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
        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 0.02;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut output = self.ctx.run_ui(input, |ui| self.app.render(ui));
            output.textures_delta.clear();
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
    fn choosing_an_upcoming_playlist_row_rebuilds_queue_for_mouse_keyboard_and_menu() {
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
            let row = h.app.test_rows[&4].0;
            h.time += 1.;
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
        let search = egui::Id::new("chat-search");
        h.ctx.memory_mut(|m| m.request_focus(search));
        h.frame(vec![]);
        let paused = h.app.state.playback().state().paused();
        h.key(egui::Key::Space);
        assert_eq!(h.app.state.playback().state().paused(), paused);
        let mut reached_row = false;
        for _ in 0..60 {
            h.key(egui::Key::Tab);
            let focused = h.ctx.memory(|m| m.focused());
            if focused
                .and_then(|id| h.ctx.read_response(id))
                .is_some_and(|r| r.rect == row)
            {
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
