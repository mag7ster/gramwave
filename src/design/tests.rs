use super::*;
#[cfg(test)]
mod regression {
    use super::*;
    #[test]
    fn middle_click_latches_and_distance_controls_continuous_scroll() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("test-scroll");
        let mut state = egui::scroll_area::State::default();
        state.offset.y = 100.;
        state.store(&ctx, id);
        let anchor = egui::pos2(40., 100.);
        let frame = |pos: egui::Pos2, button: Option<bool>| {
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if let Some(pressed) = button {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Middle,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ui| {
                    middle_scroll(
                        ui,
                        id,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(200., 200.)),
                        1000.,
                    );
                },
            );
            output.textures_delta.clear();
            egui::scroll_area::State::load(&ctx, id).unwrap().offset.y
        };
        assert_eq!(frame(anchor, Some(true)), 100.);
        assert_eq!(frame(anchor, Some(false)), 100.);
        let down = egui::pos2(40., 140.);
        let first = frame(down, None);
        assert!(first > 100.);
        let second = frame(down, None);
        assert!(second > first);
        let faster = frame(egui::pos2(40., 180.), None);
        assert!(faster - second > second - first);
        let up = frame(egui::pos2(40., 60.), None);
        assert!(up < faster);
        assert_eq!(frame(anchor, Some(true)), up);
        assert_eq!(frame(down, Some(false)), up);
    }
    fn luminance(c: egui::Color32) -> f32 {
        let channel = |v: u8| {
            let v = v as f32 / 255.;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }
    fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    #[test]
    fn both_palettes_meet_text_and_control_contrast() {
        for light in [false, true] {
            let p = palette(light);
            for surface in [p.background, p.panel, p.raised, p.selected] {
                assert!(contrast(p.text, surface) >= 4.5);
                assert!(contrast(p.muted, surface) >= 4.5);
                assert!(contrast(p.error, surface) >= 4.5);
                assert!(contrast(p.accent, surface) >= 3.);
            }
        }
    }
}
