use crate::{design, tr};
use eframe::egui;
use gramwave_application::page_browse::{Browse, PageStatus};

pub(super) fn keyboard_down(ui: &egui::Ui) -> bool {
    ui.input(|input| {
        [egui::Key::ArrowDown, egui::Key::PageDown, egui::Key::End]
            .iter()
            .any(|&key| input.key_pressed(key))
    })
}

pub(super) fn wheel_moving_down(ui: &egui::Ui) -> bool {
    ui.input(|input| input.smooth_scroll_delta().y < 0.)
}

pub(super) fn demand(
    ui: &egui::Ui,
    viewport: egui::Rect,
    offsets: (f32, f32),
    boundary: f32,
    lookahead: (f32, f32),
    moved_by_user: bool,
    upward: bool,
) -> Browse {
    let (before, after) = offsets;
    let (before, after, boundary) = if upward {
        (-before, -after, -boundary)
    } else {
        (before, after, boundary)
    };
    let (request, abandon) = lookahead;
    if viewport.height() <= 0. || after + abandon < boundary {
        return Browse::Away;
    }
    let wheel_down = ui.input(|input| {
        input
            .pointer
            .hover_pos()
            .is_some_and(|pos| viewport.contains(pos))
            && input.events.iter().any(|event| {
                matches!(event,
                    egui::Event::MouseWheel { delta, modifiers, .. }
                        if (if upward { delta.y > 0. } else { delta.y < 0. }) && !modifiers.ctrl && !modifiers.shift
                )
            })
    });
    let down = wheel_down || (moved_by_user && after > before + 0.01);
    if down && after + request >= boundary && (after <= boundary || before <= boundary) {
        Browse::Down
    } else {
        Browse::Nearby
    }
}

pub(super) fn indicator(
    ui: &mut egui::Ui,
    viewport: egui::Rect,
    offset: f32,
    boundary: f32,
    status: PageStatus,
    upward: bool,
) {
    let outside = if upward {
        offset > boundary + 1.
    } else {
        offset + viewport.height() + 1. < boundary
    };
    if viewport.height() < 22. || outside {
        return;
    }
    let edge = viewport.top() + boundary - offset;
    let (y, clip) = if upward {
        let top = viewport.top() - 22.;
        let clip = egui::Rect::from_min_max(egui::pos2(viewport.left(), top), viewport.max);
        ((edge - 11.).clamp(top + 11., viewport.bottom() - 11.), clip)
    } else {
        (
            edge.clamp(viewport.top() + 11., viewport.bottom() - 11.),
            viewport,
        )
    };
    let clip = clip.intersect(ui.clip_rect());
    let center = egui::pos2(viewport.center().x, y);
    match status {
        PageStatus::Idle => {}
        PageStatus::Loading => {
            let rect = egui::Rect::from_center_size(center, egui::Vec2::splat(20.));
            let mut overlay = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            overlay.set_clip_rect(clip);
            overlay
                .painter()
                .circle_filled(center, 10., design::palette(false).raised);
            egui::Spinner::new()
                .color(egui::Color32::WHITE)
                .paint_at(&overlay, rect.shrink(4.));
            overlay
                .interact(
                    rect,
                    overlay.id().with("page-progress"),
                    egui::Sense::hover(),
                )
                .widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::ProgressIndicator,
                        true,
                        tr("Waiting for more tracks…"),
                    )
                });
        }
        PageStatus::Unavailable => {
            let painter = ui.painter().with_clip_rect(clip);
            let palette = design::colors(ui);
            let text = painter.layout_no_wrap(
                tr("More tracks are unavailable"),
                egui::FontId::proportional(13.),
                palette.text,
            );
            let rect = egui::Rect::from_center_size(center, text.size() + egui::vec2(12., 4.));
            painter.rect_filled(rect, 4., palette.panel);
            painter.galley(center - text.size() * 0.5, text, palette.text);
        }
    }
}
