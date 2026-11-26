use super::*;

// A wheel detent can land on either side of the row's top edge.
const SNAP_DISTANCE: f32 = design::ROW_HEIGHT / 2.;

#[derive(Clone, Default)]
pub(super) struct QueueScroll {
    frame: Option<u64>,
    pub(super) following: bool,
    offset: f32,
    stride: f32,
    rect: Option<egui::Rect>,
    anchor: Option<(QueueEntryId, usize)>,
    animation: Option<(f32, f64)>,
    snap_at: Option<f64>,
}

impl QueueScroll {
    pub(super) fn load(ctx: &egui::Context, preview_history: bool) -> (egui::Id, Self, bool) {
        let key = egui::Id::new("queue-scroll-mode");
        let mut scroll: Self = ctx.data(|data| data.get_temp(key)).unwrap_or_default();
        let frame = ctx.cumulative_frame_nr();
        let opened = scroll.frame.is_none_or(|last| frame > last + 1);
        if opened {
            scroll = Self {
                following: !preview_history,
                ..Self::default()
            };
        }
        scroll.frame = Some(frame);
        (key, scroll, opened)
    }

    pub(super) fn start(
        &mut self,
        ui: &egui::Ui,
        stride: f32,
        target: f32,
        anchor: Option<(QueueEntryId, usize)>,
        opened: bool,
        middle: bool,
    ) -> (f32, bool) {
        if self.stride != stride && self.stride > 0. {
            self.offset *= stride / self.stride;
            self.animation = None;
        }
        self.stride = stride;
        if self.anchor != anchor {
            self.anchor = anchor;
            self.animation = None;
            self.snap_at = None;
        }
        if opened && self.following {
            self.offset = target;
            return (target, false);
        }
        let (wheel, scrolling, pointer, now) = ui.input(|input| {
            let over = |pos: Option<egui::Pos2>| {
                pos.is_some_and(|pos| self.rect.is_some_and(|rect| rect.contains(pos)))
            };
            let pointer = input.pointer.primary_down() && over(input.pointer.press_origin());
            let hovered = over(input.pointer.hover_pos());
            (
                hovered && input.smooth_scroll_delta() != egui::Vec2::ZERO,
                (hovered || self.snap_at.is_some()) && input.is_scrolling(),
                pointer,
                input.time,
            )
        });
        if self.following && (wheel || middle) && self.animation.is_some() {
            self.following = false;
        }
        let busy = wheel || scrolling || pointer || middle;
        if !self.following || busy {
            self.animation = None;
            return (self.offset, busy);
        }
        if (self.offset - target).abs() < 0.25 {
            self.animation = None;
            return (target, busy);
        }
        let (origin, start) = *self.animation.get_or_insert((self.offset, now));
        let progress = ((now - start) / 0.18).clamp(0., 1.) as f32;
        if progress < 1. {
            ui.request_repaint();
        } else {
            self.animation = None;
        }
        (
            egui::lerp(origin..=target, 1. - (1. - progress).powi(3)),
            busy,
        )
    }

    pub(super) fn browse(&mut self) {
        self.following = false;
        self.animation = None;
        self.snap_at = None;
    }

    pub(super) fn finish(
        &mut self,
        ui: &egui::Ui,
        output: &egui::scroll_area::ScrollAreaOutput<f32>,
        target: f32,
        busy: bool,
        reordering: bool,
        middle: bool,
    ) {
        let state = egui::scroll_area::State::load(ui.ctx(), output.id).unwrap_or(output.state);
        self.rect = Some(
            output
                .inner_rect
                .expand2(egui::vec2(f32::from(TRACK_SCROLL_GUTTER), 0.)),
        );
        self.offset = state.offset.y;
        let now = ui.input(|input| input.time);
        let max = (output.content_size.y - output.inner_rect.height()).max(0.);
        let before = output.inner.clamp(0., max);
        let manual = (self.offset - before).abs() > 0.01;
        let distance = (self.offset - target).abs();
        if reordering {
            self.animation = None;
            self.snap_at = None;
        } else if manual {
            self.following = false;
            self.animation = None;
            self.snap_at = (distance <= SNAP_DISTANCE
                && (self.snap_at.is_some() || distance < (before - target).abs()))
            .then_some(now + 0.2);
        } else if let Some(deadline) = self.snap_at {
            if distance > SNAP_DISTANCE {
                self.snap_at = None;
            } else if busy || middle || state.velocity().y != 0. {
                self.snap_at = Some(now + 0.2);
                ui.ctx().request_repaint_after(Duration::from_millis(200));
            } else if now >= deadline {
                self.following = true;
                self.snap_at = None;
                ui.request_repaint();
            } else {
                ui.ctx()
                    .request_repaint_after(Duration::from_secs_f64(deadline - now));
            }
        }
        if manual {
            ui.request_repaint();
        }
    }
}
