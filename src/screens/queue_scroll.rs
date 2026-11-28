use super::{
    scroll_anchor::{row_clear_of_edges, ScrollAnchor},
    TRACK_SCROLL_GUTTER,
};
use crate::design;
use eframe::egui;
use gramwave_domain::{playback::QueueTimeline, QueueEntryId, TrackId};
use std::time::Duration;

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
    generation: u64,
    update: Option<(u64, usize, u64)>,
    start_generation: u64,
    shuffle_generation: u64,
    fade_start: Option<f64>,
    visible: Option<ScrollAnchor<(usize, QueueEntryId, TrackId)>>,
    interior_current: Option<(QueueEntryId, f32)>,
}

impl QueueScroll {
    pub(super) fn load(
        ctx: &egui::Context,
        preview_history: bool,
        start_generation: u64,
        shuffle_generation: u64,
    ) -> (egui::Id, Self, bool, bool) {
        let key = egui::Id::new("queue-scroll-mode");
        let mut scroll: Self = ctx.data(|data| data.get_temp(key)).unwrap_or_default();
        let frame = ctx.cumulative_frame_nr();
        let opened = scroll.frame.is_none_or(|last| frame > last + 1);
        let started = scroll.start_generation != start_generation;
        let shuffled = scroll.shuffle_generation != shuffle_generation;
        let realign = opened || started || shuffled;
        if realign {
            scroll = Self {
                following: started || shuffled || !preview_history,
                start_generation,
                shuffle_generation,
                ..Self::default()
            };
        }
        scroll.frame = Some(frame);
        (key, scroll, realign, shuffled)
    }

    pub(super) fn reverse(
        &mut self,
        revision: (u64, bool),
        queue: QueueTimeline<'_>,
        stride: f32,
        max: f32,
        height: f32,
        now: f64,
    ) -> bool {
        let (generation, opened) = revision;
        if self.generation == generation || opened {
            self.generation = generation;
            return false;
        }
        self.generation = generation;
        self.stride = stride;
        self.animation = None;
        self.snap_at = None;
        self.fade_start = Some(now);
        if !self.following {
            let current_offset = self.interior_current.and_then(|(id, top)| {
                let index = queue.current_index()?;
                (queue.get(index)?.entry_id == id)
                    .then(|| (index as f32 * stride - top).clamp(0., max.max(0.)))
            });
            self.offset = current_offset
                .or_else(|| {
                    let visible = self.visible.as_ref()?;
                    let find = |&(index, entry_id, track_id): &(usize, QueueEntryId, TrackId)| {
                        let mirrored = queue.len().saturating_sub(index + 1);
                        if queue
                            .get(mirrored)
                            .is_some_and(|entry| entry.entry_id == entry_id)
                        {
                            return Some(mirrored);
                        }
                        (0..queue.len())
                            .filter(|&i| queue.get(i).unwrap().entry_id == entry_id)
                            .min_by_key(|&i| i.abs_diff(mirrored))
                            .or_else(|| {
                                queue
                                    .get(mirrored)
                                    .filter(|entry| entry.id() == track_id)
                                    .map(|_| mirrored)
                            })
                    };
                    visible.mirrored_offset(stride, max, height, design::ROW_HEIGHT, find)
                })
                .unwrap_or_else(|| self.offset.min(max.max(0.)));
        }
        true
    }

    pub(super) fn offset(&self) -> f32 {
        self.offset
    }

    pub(super) fn queue_updated(
        &mut self,
        queue: QueueTimeline<'_>,
        update: (u64, usize, u64),
        stride: f32,
        max: f32,
        realign: bool,
    ) {
        let Some((previous_page, boundary, previous_sample)) = self.update.replace(update) else {
            return;
        };
        let (page, new_boundary, sample) = update;
        let sampled = previous_sample != sample;
        if realign || previous_page == page && !sampled {
            return;
        }
        let current_moved =
            self.anchor
                .zip(queue.current_index())
                .is_some_and(|((id, previous), index)| {
                    index != previous && queue.get(index).is_some_and(|row| row.entry_id == id)
                });
        if self.following {
            if current_moved {
                self.offset = queue.current_index().unwrap_or_default() as f32 * stride;
                self.stride = stride;
            }
            return;
        }
        if !sampled && !current_moved && self.offset < boundary as f32 * self.stride {
            return;
        }
        let first = if current_moved || sampled {
            0
        } else {
            new_boundary
        };
        if let Some(offset) = self.visible.as_ref().and_then(|anchor| {
            anchor.offset(stride, max, |&(old, id, _)| {
                (first..queue.len())
                    // An upcoming row moved toward the front by the sample should not pull the viewport with it.
                    .filter(|&i| !sampled || i >= old)
                    .filter(|&i| queue.get(i).is_some_and(|entry| entry.entry_id == id))
                    .min_by_key(|&i| i.abs_diff(old))
            })
        }) {
            self.offset = offset;
            self.stride = stride;
        } else if sampled {
            self.offset = self.offset.clamp(0., max.max(0.));
        }
    }

    pub(super) fn fade(&mut self, ui: &egui::Ui) -> f32 {
        let Some(start) = self.fade_start else {
            return 1.;
        };
        let progress = ((ui.input(|i| i.time) - start) / 0.15).clamp(0., 1.) as f32;
        if progress < 1. {
            ui.request_repaint();
        } else {
            self.fade_start = None;
        }
        egui::lerp(0.6..=1., 1. - (1. - progress).powi(2))
    }

    pub(super) fn remember(&mut self, queue: QueueTimeline<'_>, height: f32) {
        let current = queue.current_index().map(|index| {
            let top = index as f32 * self.stride - self.offset;
            (queue.get(index).unwrap().entry_id, top)
        });
        self.interior_current = current.filter(|&(_, top)| row_clear_of_edges(top, height));
        self.visible = Some(ScrollAnchor::capture(
            queue.len(),
            self.stride,
            self.offset,
            height,
            |i| {
                let entry = queue.get(i).unwrap();
                (i, entry.entry_id, entry.id())
            },
        ));
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

    pub(super) fn selected(
        &mut self,
        anchor: (QueueEntryId, usize),
        previous_target: f32,
        now: f64,
    ) {
        if !self.following && self.anchor != Some(anchor) {
            self.anchor = Some(anchor);
            let distance = (self.offset - anchor.1 as f32 * self.stride).abs();
            self.arm_snap((self.offset - previous_target).abs(), distance, now);
        }
    }

    fn arm_snap(&mut self, previous_distance: f32, distance: f32, now: f64) {
        self.snap_at = (distance <= SNAP_DISTANCE
            && (self.snap_at.is_some() || distance < previous_distance))
            .then_some(now + 0.2);
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
            self.arm_snap((before - target).abs(), distance, now);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_inserted_before_the_manual_tail_preserves_the_browsed_row() {
        use gramwave_domain::{PlaylistOrder, Session, TrackMetadata};
        let track = |message| TrackMetadata {
            chat: 1,
            message,
            ..Default::default()
        };
        let mut session = Session::default();
        session.start_playlist(
            vec![track(100), track(95)],
            0,
            [Some(90), None, None],
            true,
            PlaylistOrder::NewestFirst,
        );
        session.insert_many((200..212).map(track), false);
        let stride = 68.;
        let initial = 4.5 * stride;
        let mut scroll = QueueScroll {
            stride,
            offset: initial,
            ..Default::default()
        };
        let height = 3. * stride;
        scroll.queue_updated(session.timeline(), (0, 2, 0), stride, 20. * stride, false);
        scroll.remember(session.timeline(), height);
        let row = session.timeline().get(5).unwrap().entry_id;

        session.add_playlist_page(0, 60, [80, 75].map(track), true);
        scroll.queue_updated(session.timeline(), (1, 4, 0), stride, 20. * stride, false);
        assert_eq!(scroll.offset(), initial + 2. * stride);
        assert_eq!(session.timeline().get(7).unwrap().entry_id, row);
    }

    #[test]
    fn selection_needs_a_closer_target_or_an_existing_snap() {
        let mut scroll = QueueScroll {
            offset: 80.,
            stride: 68.,
            ..Default::default()
        };
        scroll.selected((QueueEntryId(1), 1), 72., 1.);
        assert!(scroll.snap_at.is_none());
        scroll.arm_snap(13., 12., 2.);
        scroll.selected((QueueEntryId(2), 1), 68., 3.);
        assert_eq!(scroll.snap_at, Some(3.2));
    }
}
