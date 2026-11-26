use super::keyboard::focus_outline;
use crate::application::commands::AppCommand;
use crate::{design, tr, App};
use eframe::egui;
use std::time::{Duration, Instant};

impl App {
    pub(super) fn queue_notice_time(&self, _ui: &egui::Ui) -> Instant {
        #[cfg(test)]
        return self.preview_started + Duration::from_secs_f64(_ui.input(|i| i.time));
        #[cfg(not(test))]
        Instant::now()
    }

    pub(super) fn queue_removal_notices(
        &mut self,
        ui: &mut egui::Ui,
        bounds: egui::Rect,
        now: Instant,
    ) {
        const HEIGHT: f32 = 42.;
        const STRIDE: f32 = HEIGHT + 6.;
        let bounds = bounds.shrink2(egui::vec2(4., 4.));
        let palette = design::colors(ui);
        let mut undo = None;
        for (slot, notice) in self.state.playback().queue_removals(now).enumerate() {
            let remaining = notice.expires.saturating_duration_since(now);
            let shift = slot as f32 * STRIDE;
            let target = bounds.bottom() - shift;
            if target - HEIGHT < bounds.top() {
                break;
            }

            let id = ui.make_persistent_id(("queue-removal", notice.id));
            let hit_id = ui.make_persistent_id(("queue-removal-slot", slot));
            let offset = ui.ctx().animate_value_with_time(id, shift, 0.06);
            let entrance = (1. - (5. - remaining.as_secs_f32()) / 0.16).clamp(0., 1.);
            let bottom = bounds.bottom() - offset + 20. * entrance.powi(2);
            let at = |bottom| {
                egui::Rect::from_min_max(
                    egui::pos2(bounds.left(), bottom - HEIGHT),
                    egui::pos2(bounds.right(), bottom),
                )
            };
            let painted = at(bottom);
            let hit = at(target);
            let clip = if entrance > 0. {
                bounds.with_max_y(target)
            } else {
                bounds
            };
            let painter = ui.painter().with_clip_rect(clip);
            ui.interact(hit, hit_id, egui::Sense::CLICK);

            painter.rect(
                painted,
                8.,
                palette.panel,
                egui::Stroke::new(1., palette.raised),
                egui::StrokeKind::Inside,
            );
            let label = crate::i18n::ngettext(
                "Track removed",
                "{count} tracks removed",
                notice.count as u32,
            )
            .replace("{count}", &notice.count.to_string());
            let button_rect = |rect: egui::Rect| {
                egui::Rect::from_min_max(
                    egui::pos2(rect.right() - 68., rect.top() + 4.),
                    rect.right_bottom() - egui::vec2(5., 5.),
                )
            };
            let button = ui.interact(button_rect(hit), hit_id.with("undo"), egui::Sense::click());
            let pressed = button.id.with("pressed");
            if button.is_pointer_button_down_on()
                && ui.input(|input| input.pointer.primary_pressed())
            {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(pressed, notice.id));
            }
            let undo_label = tr("Undo");
            button.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &undo_label)
            });
            let button_paint = button_rect(painted);
            painter.rect_filled(button_paint, 7., ui.style().interact(&button).bg_fill);
            painter.text(
                button_paint.center(),
                egui::Align2::CENTER_CENTER,
                undo_label,
                egui::TextStyle::Button.resolve(ui.style()),
                palette.text,
            );
            focus_outline(ui, &button);
            let label_clip = painted.with_max_x(button_paint.left() - 5.);
            painter.with_clip_rect(label_clip.intersect(clip)).text(
                painted.left_center() + egui::vec2(10., 0.),
                egui::Align2::LEFT_CENTER,
                label,
                egui::TextStyle::Body.resolve(ui.style()),
                palette.text,
            );
            let left = painted.left_bottom() + egui::vec2(8., -3.);
            let width = (painted.width() - 16.) * (remaining.as_secs_f32() / 5.).clamp(0., 1.);
            painter.line_segment([left, left + egui::vec2(width, 0.)], (2., palette.accent));
            if button.clicked()
                && (!button.clicked_by(egui::PointerButton::Primary)
                    || ui.ctx().data(|data| data.get_temp(pressed)) == Some(notice.id))
            {
                undo = Some(notice.id);
            }
            design::repaint_after(ui, remaining.min(Duration::from_millis(32)));
        }
        if let Some(id) = undo {
            self.execute(AppCommand::UndoQueueRemoval { id, now });
            ui.request_repaint();
        }
    }
}
