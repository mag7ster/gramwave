use eframe::egui;
fastframe_icons::icons! {
 pub enum Icon {
prefix:"tgplayer-",directory:"../assets/",Play=>lucide "play",Pause=>lucide "pause",Next => "next",Previous => "previous",More=>lucide "ellipsis",Download => "download",Queue => "queue",Settings=>lucide "settings",Search=>lucide "search",Close=>lucide "x",Shuffle => "shuffle",Repeat => "repeat",Volume=>lucide "volume-2",Muted=>lucide "volume-x"}
}
pub const ROW_HEIGHT: f32 = 60.;
pub const GREEN: egui::Color32 = egui::Color32::from_rgb(82, 214, 154);
pub struct Palette {
    pub background: egui::Color32,
    pub panel: egui::Color32,
    pub raised: egui::Color32,
    pub text: egui::Color32,
    pub muted: egui::Color32,
    pub accent: egui::Color32,
    pub selected: egui::Color32,
    pub error: egui::Color32,
}
pub fn palette(light: bool) -> Palette {
    use egui::Color32 as C;
    if light {
        Palette {
            background: C::from_rgb(246, 248, 247),
            panel: C::WHITE,
            raised: C::from_rgb(229, 235, 232),
            text: C::from_rgb(24, 30, 27),
            muted: C::from_rgb(81, 94, 86),
            accent: C::from_rgb(0, 112, 66),
            selected: C::from_rgb(211, 238, 224),
            error: C::from_rgb(164, 37, 44),
        }
    } else {
        Palette {
            background: C::from_rgb(16, 18, 20),
            panel: C::from_rgb(24, 27, 31),
            raised: C::from_rgb(34, 38, 43),
            text: C::from_rgb(243, 245, 247),
            muted: C::from_rgb(168, 176, 186),
            accent: GREEN,
            selected: C::from_rgb(28, 65, 48),
            error: C::from_rgb(255, 155, 155),
        }
    }
}
pub fn colors(ui: &egui::Ui) -> Palette {
    palette(!ui.visuals().dark_mode)
}
pub fn install(ctx: &egui::Context) {
    fastframe_icons::install::<Icon>(ctx);
}
pub fn style(ctx: &egui::Context, light: bool) {
    let p = palette(light);
    let mut v = if light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    v.panel_fill = p.background;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.panel;
    v.override_text_color = Some(p.text);
    v.weak_text_color = Some(p.muted);
    v.hyperlink_color = p.accent;
    v.error_fg_color = p.error;
    v.widgets.inactive.bg_fill = p.raised;
    v.widgets.inactive.weak_bg_fill = p.raised;
    v.widgets.hovered.bg_fill = p.selected;
    v.widgets.active.bg_fill = p.selected;
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1., p.accent);
    v.widgets.active.bg_stroke = egui::Stroke::new(1., p.accent);
    v.widgets.inactive.corner_radius = egui::CornerRadius::same(8);
    v.widgets.hovered.corner_radius = egui::CornerRadius::same(8);
    v.widgets.active.corner_radius = egui::CornerRadius::same(8);
    v.selection.bg_fill = p.selected;
    v.selection.stroke = egui::Stroke::new(1., p.accent);
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8., 8.);
        s.spacing.button_padding = egui::vec2(10., 8.);
        s.spacing.interact_size = egui::vec2(36., 36.);
        s.interaction.selectable_labels = false;
        s.animation_time = 0.14;
        s.text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.));
        s.text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.));
        s.text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(12.));
    });
}
pub fn button(ui: &mut egui::Ui, icon: Icon, label: &'static str) -> bool {
    labelled_button(ui, icon, crate::tr(label))
}
pub fn labelled_button(ui: &mut egui::Ui, icon: Icon, label: String) -> bool {
    ui.add(
        egui::Button::image(icon.image(ui.visuals().text_color(), 20.))
            .frame(false)
            .min_size(egui::vec2(36., 36.)),
    )
    .on_hover_text(label)
    .clicked()
}
pub fn toggle(ui: &mut egui::Ui, icon: Icon, active: bool, badge: &str, hint: String) -> bool {
    let p = colors(ui);
    let response = ui.add(
        egui::Button::image(icon.image(if active { p.accent } else { p.muted }, 20.))
            .fill(if active {
                p.selected
            } else {
                egui::Color32::TRANSPARENT
            })
            .min_size(egui::vec2(36., 36.)),
    );
    if active {
        if badge.is_empty() {
            ui.painter().circle_filled(
                response.rect.center_bottom() - egui::vec2(0., 4.),
                2.,
                p.accent,
            );
        } else {
            ui.painter().text(
                response.rect.right_top() + egui::vec2(-5., 5.),
                egui::Align2::CENTER_CENTER,
                badge,
                egui::FontId::proportional(11.),
                p.accent,
            );
        }
    }
    response.on_hover_text(hint).clicked()
}
pub fn primary(ui: &mut egui::Ui, label: String) -> egui::Response {
    let p = colors(ui);
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(if ui.visuals().dark_mode {
            egui::Color32::BLACK
        } else {
            egui::Color32::WHITE
        }))
        .fill(p.accent)
        .corner_radius(10)
        .min_size(egui::vec2(ui.available_width(), 40.)),
    )
}
pub fn settings_section(ui: &mut egui::Ui, title: String, contents: impl FnOnce(&mut egui::Ui)) {
    let p = colors(ui);
    egui::Frame::new()
        .fill(p.raised)
        .corner_radius(12)
        .inner_margin(16.)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.visuals_mut().widgets.inactive.weak_bg_fill = p.panel;
            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
            ui.label(egui::RichText::new(title).size(17.).strong());
            ui.add_space(4.);
            contents(ui);
        });
    ui.add_space(4.);
}
pub fn choice(ui: &mut egui::Ui, label: String, active: bool) -> bool {
    let p = colors(ui);
    ui.add(
        egui::Button::new(
            egui::RichText::new(if active {
                format!("✓ {label}")
            } else {
                label
            })
            .color(if active { p.accent } else { p.text }),
        )
        .fill(if active { p.selected } else { p.panel })
        .stroke(egui::Stroke::NONE)
        .corner_radius(8)
        .min_size(egui::vec2(72., 36.)),
    )
    .clicked()
}

/// A labelled slider with a generous hit target, preview and explicit keyboard handling.
pub fn slider(
    ui: &mut egui::Ui,
    value: &mut f64,
    max: f64,
    width: f32,
    label: String,
    time: bool,
) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(
        egui::vec2(width.max(28.), 28.),
        egui::Sense::click_and_drag(),
    );
    response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), *value, &label));
    let rail = rect.shrink2(egui::vec2(6., 0.));
    let before = *value;
    if response.is_pointer_button_down_on() {
        response.request_focus();
        if let Some(pos) = response.interact_pointer_pos() {
            *value = ((pos.x - rail.left()) / rail.width()).clamp(0., 1.) as f64 * max;
        }
    }
    if response.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
        ui.input_mut(|i| {
            for (key, delta) in [
                (egui::Key::ArrowRight, 1.),
                (egui::Key::ArrowUp, 1.),
                (egui::Key::ArrowLeft, -1.),
                (egui::Key::ArrowDown, -1.),
            ] {
                if i.consume_key(egui::Modifiers::NONE, key) {
                    *value += delta * if time { 5. } else { 1. };
                }
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Home) {
                *value = 0.;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::End) {
                *value = max;
            }
        });
    }
    *value = value.clamp(0., max.max(0.));
    if *value != before {
        response.mark_changed();
    }
    let p = colors(ui);
    let y = rect.center().y;
    let x = rail.left() + rail.width() * (*value / max.max(1.)) as f32;
    ui.painter().line_segment(
        [egui::pos2(rail.left(), y), egui::pos2(rail.right(), y)],
        egui::Stroke::new(4., p.muted),
    );
    ui.painter().line_segment(
        [egui::pos2(rail.left(), y), egui::pos2(x, y)],
        egui::Stroke::new(4., p.accent),
    );
    ui.painter().circle_filled(egui::pos2(x, y), 6., p.accent);
    if response.has_focus() && time {
        ui.painter()
            .circle_stroke(egui::pos2(x, y), 9., egui::Stroke::new(1., p.accent));
    } else if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            4.,
            egui::Stroke::new(1., p.accent),
            egui::StrokeKind::Inside,
        );
    }
    let preview = response
        .hover_pos()
        .map(|pos| ((pos.x - rail.left()) / rail.width()).clamp(0., 1.) as f64 * max)
        .unwrap_or(*value);
    response.on_hover_text(if time {
        format!(
            "{label}: {}:{:02}",
            preview as u64 / 60,
            preview as u64 % 60
        )
    } else {
        format!("{label}: {:.0}%", *value)
    })
}

#[derive(Clone)]
struct AutoScroll {
    owner: egui::Id,
    anchor: egui::Pos2,
    dragged: bool,
    last_frame: u64,
}

/// Firefox-style autoscroll: distance from the anchor controls velocity.
pub fn middle_scroll(ui: &egui::Ui, output_id: egui::Id, rect: egui::Rect, content_height: f32) {
    let key = egui::Id::new("middle-autoscroll");
    let frame = ui.ctx().cumulative_frame_nr();
    let blocked = key.with("blocked-frame");
    if ui.ctx().data(|d| d.get_temp::<u64>(blocked)) == Some(frame) {
        return;
    }
    let mut active = ui.ctx().data(|d| d.get_temp::<AutoScroll>(key));
    let (pressed, released, cancel, focused, pos, dt) = ui.input(|i| {
        (
            i.pointer.button_pressed(egui::PointerButton::Middle),
            i.pointer.button_released(egui::PointerButton::Middle),
            i.key_pressed(egui::Key::Escape)
                || i.pointer.button_pressed(egui::PointerButton::Primary)
                || i.pointer.button_pressed(egui::PointerButton::Secondary)
                || i.raw
                    .events
                    .iter()
                    .any(|e| matches!(e, egui::Event::MouseWheel { .. })),
            i.focused,
            i.pointer.hover_pos(),
            i.stable_dt.min(0.05),
        )
    });
    if !focused
        || cancel
        || active
            .as_ref()
            .is_some_and(|s| s.owner == output_id && pressed)
    {
        ui.ctx().data_mut(|d| {
            d.remove::<AutoScroll>(key);
            d.insert_temp(blocked, frame);
        });
        return;
    }
    if active.is_none() && pressed && pos.is_some_and(|p| rect.contains(p)) {
        active = Some(AutoScroll {
            owner: output_id,
            anchor: pos.unwrap(),
            dragged: false,
            last_frame: frame,
        });
    }
    let Some(mut active) = active.filter(|s| s.owner == output_id) else {
        return;
    };
    active.last_frame = frame;
    let displacement = pos.map_or(0., |p| p.y - active.anchor.y);
    let distance = (displacement.abs() - 8.).max(0.);
    active.dragged |= distance > 0.;
    // A stationary click latches the mode; holding and moving also works.
    if released && active.dragged {
        ui.ctx().data_mut(|d| {
            d.remove::<AutoScroll>(key);
            d.insert_temp(blocked, frame);
        });
        return;
    }
    if let Some(mut state) = egui::scroll_area::State::load(ui.ctx(), output_id) {
        let speed = displacement.signum() * (distance * 8. + distance * distance * 0.12).min(5000.);
        state.offset.y =
            (state.offset.y + speed * dt).clamp(0., (content_height - rect.height()).max(0.));
        state.store(ui.ctx(), output_id);
    }
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(egui::Order::Tooltip, key));
    let p = colors(ui);
    painter.circle_filled(active.anchor, 13., p.panel);
    painter.circle_stroke(active.anchor, 13., egui::Stroke::new(1., p.accent));
    painter.text(
        active.anchor,
        egui::Align2::CENTER_CENTER,
        "↕",
        egui::FontId::proportional(18.),
        p.accent,
    );
    ui.ctx().set_cursor_icon(egui::CursorIcon::AllScroll);
    ui.ctx().data_mut(|d| d.insert_temp(key, active));
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(16));
}

#[cfg(test)]
mod tests {
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
