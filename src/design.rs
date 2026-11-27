use eframe::egui;
fastframe_icons::icons! {
 pub enum Icon {
prefix:"gramwave-",directory:"../assets/",Play=>lucide "play",Pause=>lucide "pause",Next => "next",Previous => "previous",More=>lucide "ellipsis",Download => "download",Trash=>lucide "trash-2",Queue => "queue",Settings=>lucide "settings",Search=>lucide "search",Close=>lucide "x",Shuffle => "shuffle",Reverse => "reverse",Repeat => "repeat",Volume=>lucide "volume-2",Muted=>lucide "volume-x",Minimize=>lucide "minus",Maximize=>lucide "square",Restore=>lucide "copy"}
}
pub const ROW_HEIGHT: f32 = 60.;
pub const HEADER_HEIGHT: f32 = 44.;
pub const APP_ICON: &[u8] = include_bytes!("../assets/gramwave.svg");
pub const GREEN: egui::Color32 = egui::Color32::from_rgb(82, 214, 154);
pub struct Palette {
    pub background: egui::Color32,
    pub panel: egui::Color32,
    pub raised: egui::Color32,
    pub text: egui::Color32,
    pub muted: egui::Color32,
    pub accent: egui::Color32,
    pub selected: egui::Color32,
    pub track_selection: egui::Color32,
    pub error: egui::Color32,
}
impl Palette {
    pub fn track_fill(&self, current: bool, selected: bool) -> Option<egui::Color32> {
        match (current, selected) {
            (true, true) => Some(self.selected.lerp_to_gamma(self.track_selection, 0.5)),
            (true, false) => Some(self.selected),
            (false, true) => Some(self.track_selection),
            (false, false) => None,
        }
    }
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
            track_selection: C::from_rgb(203, 233, 229),
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
            track_selection: C::from_rgb(26, 63, 60),
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
    v.text_edit_bg_color = Some(p.raised);
    v.override_text_color = Some(p.text);
    v.weak_text_color = Some(p.muted);
    v.hyperlink_color = p.accent;
    v.error_fg_color = p.error;
    v.widgets.inactive.bg_fill = p.raised;
    v.widgets.inactive.weak_bg_fill = p.raised;
    for widgets in [
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widgets.bg_fill = p.selected;
        widgets.weak_bg_fill = p.selected;
    }
    for widgets in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widgets.bg_stroke = egui::Stroke::NONE;
        widgets.corner_radius = egui::CornerRadius::same(8);
        widgets.expansion = 0.;
    }
    v.selection.bg_fill = p.selected;
    v.selection.stroke = egui::Stroke::new(0., p.accent);
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8., 8.);
        s.spacing.button_padding = egui::vec2(8., 8.);
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
        egui::Button::image(icon.image(ui.visuals().text_color(), 20.).alt_text(&label))
            .frame_when_inactive(false)
            .min_size(egui::vec2(36., 36.)),
    )
    .clicked()
}
pub fn playback_button(ui: &mut egui::Ui, icon: Icon, label: &'static str) -> bool {
    playback_control(ui, Some(icon), label)
}

pub fn playback_control(ui: &mut egui::Ui, icon: Option<Icon>, label: &'static str) -> bool {
    let label = crate::tr(label);
    let mut spinner_ink = egui::Color32::TRANSPARENT;
    let response = accent_button(ui, |ink| {
        let button = if let Some(icon) = icon {
            egui::Button::image(icon.image(ink, 22.).alt_text(&label))
        } else {
            spinner_ink = ink;
            egui::Button::new("")
        };
        button.corner_radius(22).min_size(egui::Vec2::splat(44.))
    });
    if icon.is_none() {
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
        });
        egui::Spinner::new().color(spinner_ink).paint_at(
            ui,
            egui::Rect::from_center_size(response.rect.center(), egui::Vec2::splat(22.)),
        );
        return response.on_hover_text(label).clicked();
    }
    response.clicked()
}
fn accent_button<'a>(
    ui: &mut egui::Ui,
    button: impl FnOnce(egui::Color32) -> egui::Button<'a>,
) -> egui::Response {
    let p = colors(ui);
    let ink = if ui.visuals().dark_mode {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
    };
    ui.scope(move |ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.inactive.weak_bg_fill = p.accent;
        widgets.hovered.weak_bg_fill = p.accent.lerp_to_gamma(p.text, 0.08);
        widgets.active.weak_bg_fill = p.accent.lerp_to_gamma(p.text, 0.18);
        ui.add(button(ink))
    })
    .inner
}
pub fn toggle(ui: &mut egui::Ui, icon: Icon, active: bool, badge: &str, hint: String) -> bool {
    let p = colors(ui);
    let response = ui
        .scope(|ui| {
            let widgets = &mut ui.visuals_mut().widgets;
            widgets.inactive.weak_bg_fill = p.selected;
            if active {
                widgets.hovered.weak_bg_fill = p.raised;
                widgets.active.weak_bg_fill = p.raised;
            }
            ui.add(
                egui::Button::image(
                    icon.image(if active { p.accent } else { p.muted }, 20.)
                        .alt_text(&hint),
                )
                .frame_when_inactive(active)
                .min_size(egui::vec2(36., 36.)),
            )
        })
        .inner;
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
    response.clicked()
}
pub fn primary(ui: &mut egui::Ui, label: String) -> egui::Response {
    let width = ui.available_width();
    accent_button(ui, |ink| {
        egui::Button::new(egui::RichText::new(label).color(ink))
            .corner_radius(10)
            .min_size(egui::vec2(width, 40.))
    })
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
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.inactive.weak_bg_fill = if active { p.selected } else { p.panel };
        if active {
            widgets.hovered.weak_bg_fill = p.raised;
            widgets.active.weak_bg_fill = p.raised;
        }
        ui.add(
            egui::Button::new(
                egui::RichText::new(if active {
                    format!("✓ {label}")
                } else {
                    label
                })
                .color(if active { p.accent } else { p.text }),
            )
            .corner_radius(8)
            .min_size(egui::vec2(72., 36.)),
        )
        .clicked()
    })
    .inner
}

pub(crate) fn slider_rail(rect: egui::Rect) -> egui::Rect {
    rect.shrink2(egui::vec2(6., 0.))
}

pub(crate) fn repaint_after(ui: &egui::Ui, delay: std::time::Duration) {
    // egui subtracts predicted_dt when scheduling the repaint.
    let frame = ui.input(|input| std::time::Duration::from_secs_f32(input.predicted_dt));
    ui.ctx().request_repaint_after(delay + frame);
}

/// A labelled slider with a generous hit target and explicit keyboard handling.
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
    let rail = slider_rail(rect);
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
    let radius = if response.has_focus() { 8. } else { 6. };
    ui.painter()
        .circle_filled(egui::pos2(x, y), radius, p.accent);
    response
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

pub(crate) fn middle_scroll_active(ctx: &egui::Context, owner: egui::Id) -> bool {
    ctx.data(|data| data.get_temp::<AutoScroll>(egui::Id::new("middle-autoscroll")))
        .is_some_and(|active| active.owner == owner)
}

#[cfg(test)]
#[path = "design/tests.rs"]
mod tests;
