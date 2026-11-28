use super::{
    art,
    keyboard::{self, empty_label, focus_outline, ListNavigation, MenuItems},
    list_scroll_area, paint_current_track_marker, queue_scroll, selection, track_list_frame,
    track_menu_button, track_more_button, WorkspacePane, TRACK_ROW_PADDING,
};
use crate::application::commands::AppCommand;
use crate::{design, tr, App};
use design::{button, Icon};
use eframe::egui;
use gramwave_domain::{
    playback::{QueueRangeEdit, QueueTimeline},
    QueueEntryId,
};
use std::collections::HashSet;

#[derive(Clone, Copy)]
struct QueueDrag {
    id: egui::Id,
    from: usize,
    entry: QueueEntryId,
    current: Option<(usize, QueueEntryId)>,
    grab: egui::Vec2,
    slot: usize,
}

struct LiftedQueueRow {
    from: usize,
    slot: usize,
    rect: egui::Rect,
}

fn queue_row_rect(list: egui::Rect, stride: f32, slot: f32) -> egui::Rect {
    egui::Rect::from_min_size(
        list.min + egui::vec2(0., stride * slot),
        egui::vec2(list.width(), design::ROW_HEIGHT),
    )
}

fn queue_drag(
    ui: &egui::Ui,
    list: egui::Rect,
    stride: f32,
    queue: QueueTimeline<'_>,
    reorder: (egui::Id, (u64, u64, u64, u64)),
    row_id: impl Fn(usize) -> egui::Id,
    allow_start: bool,
) -> (Option<LiftedQueueRow>, Option<AppCommand>) {
    let (key, row_revision) = reorder;
    let ctx = ui.ctx();
    let current = queue
        .current_index()
        .map(|i| (i, queue.get(i).unwrap().entry_id));
    let mut drag = ctx.data(|data| data.get_temp::<QueueDrag>(key));
    let (down, released, cancelled, origin, pointer) = ui.input(|i| {
        (
            i.pointer.primary_down(),
            i.pointer.primary_released(),
            !i.focused || i.key_pressed(egui::Key::Escape),
            i.pointer.press_origin(),
            i.pointer.interact_pos(),
        )
    });
    if drag.is_none() && down && allow_start && !cancelled {
        drag = ctx.dragged_id().zip(origin).and_then(|(id, origin)| {
            let slot = (0..queue.len()).find(|&i| row_id(i) == id)?;
            let track = queue.get(slot)?;
            let response = ctx.read_response(id)?;
            if queue_cover_rect(response.rect).contains(origin)
                || pointer.is_none_or(|pos| pos.distance(origin) < 4.)
            {
                return None;
            }
            Some(QueueDrag {
                id,
                from: slot,
                entry: track.entry_id,
                current,
                grab: origin - response.rect.min,
                slot,
            })
        });
    }
    let Some(mut drag) = drag else {
        return (None, None);
    };
    let id = drag.id;
    let stop = || {
        ctx.data_mut(|data| data.remove::<QueueDrag>(key));
        if ctx.is_being_dragged(id) {
            ctx.stop_dragging();
        }
    };
    let from = drag.from;
    let Some(pointer) = pointer.filter(|_| {
        drag.current == current
            && queue
                .get(from)
                .is_some_and(|row| row.entry_id == drag.entry)
    }) else {
        stop();
        return (None, None);
    };
    let dragging = down && ctx.is_being_dragged(id);
    let dropping = released && ctx.drag_stopped_id() == Some(id);
    if !dragging && !dropping && !cancelled {
        stop();
        return (None, None);
    }
    let viewport = selection::drag_viewport(ui, list);
    let inside = viewport.contains(pointer);
    let rect = egui::Rect::from_min_size(
        pointer - drag.grab,
        egui::vec2(list.width(), design::ROW_HEIGHT),
    );
    if inside {
        drag.slot = ((rect.top() - list.top()) / stride).round() as usize;
    }
    drag.slot = drag.slot.min(queue.len() - 1);
    let slot = drag.slot;
    if cancelled || dropping {
        stop();
        // Start the settling animation where the lifted row was released.
        ctx.animate_value_with_time(
            id.with(("y", row_revision)),
            (rect.top() - list.top()) / stride,
            0.,
        );
        ctx.animate_value_with_time(id.with(("x", row_revision)), rect.left() - list.left(), 0.);
        if !cancelled && inside && from != slot {
            return (
                Some(LiftedQueueRow { from, slot, rect }),
                Some(AppCommand::MoveQueueRow {
                    from,
                    entry_id: drag.entry,
                    to: slot,
                }),
            );
        }
        return (None, None);
    }

    ctx.data_mut(|data| data.insert_temp(key, drag));
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    if inside {
        selection::scroll_drag_edge(ui, list, viewport, pointer.y);
    }
    (Some(LiftedQueueRow { from, slot, rect }), None)
}

fn repeated_upcoming(queue: QueueTimeline<'_>) -> HashSet<QueueEntryId> {
    let mut seen = HashSet::new();
    queue
        .upcoming_ids()
        .filter(|&id| !seen.insert(id))
        .collect()
}

fn queue_row_id(
    queue: QueueTimeline<'_>,
    repeated: &HashSet<QueueEntryId>,
    key: egui::Id,
    i: usize,
) -> egui::Id {
    let id = queue.get(i).unwrap().entry_id;
    if i < queue.past_len() {
        key.with(("past", i, id))
    } else if i < queue.upcoming_start() {
        key.with(("current", id))
    } else if repeated.contains(&id) {
        key.with(("upcoming", i, id))
    } else {
        key.with(id)
    }
}

pub(super) fn queue_cover_rect(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(rect.left() + TRACK_ROW_PADDING, rect.center().y - 22.),
        egui::Vec2::splat(44.),
    )
}

fn queue_row_contents(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    track: &crate::Track,
    artwork: Option<egui::load::SizedTexture>,
    past: bool,
    current: bool,
    highlighted: bool,
) -> egui::Response {
    let inset = rect.shrink2(egui::vec2(TRACK_ROW_PADDING, 6.));
    let center_y = inset.center().y;
    let cover = queue_cover_rect(rect);
    let cover_pos = cover.min;
    let mut cover_ui = ui.new_child(egui::UiBuilder::new().max_rect(cover));
    if past {
        cover_ui.multiply_opacity(0.62);
    }
    art(&mut cover_ui, &track.title, artwork, 44.);
    let palette = design::colors(ui);
    if current {
        ui.painter().rect_filled(
            egui::Rect::from_center_size(
                egui::pos2(rect.left() + 1.5, center_y),
                egui::vec2(3., 30.),
            ),
            2.,
            palette.accent,
        );
        let size = egui::Vec2::splat(16.);
        let marker = egui::Rect::from_min_size(cover_pos + egui::Vec2::splat(28.), size);
        ui.painter().rect_filled(marker, 4., palette.panel);
        ui.put(
            marker,
            Icon::Play
                .image(palette.accent, 12.)
                .alt_text(tr("Current track")),
        );
    }

    let labels_left = cover_pos.x + 52.;
    let labels_right = inset.right() - 44.;
    if labels_right > labels_left {
        let labels_rect = egui::Rect::from_min_max(
            egui::pos2(labels_left, center_y - 19.),
            egui::pos2(labels_right, center_y + 19.),
        );
        let mut labels = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(labels_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        labels.set_clip_rect(ui.clip_rect().intersect(labels_rect));
        labels.spacing_mut().item_spacing.y = 4.;
        let mut title = egui::RichText::new(&track.title);
        if past {
            title = title.color(palette.muted);
        } else if current {
            title = title.strong();
        }
        labels.add(
            egui::Label::new(title)
                .truncate()
                .show_tooltip_when_elided(false),
        );
        labels.add(
            egui::Label::new(egui::RichText::new(&track.artist).weak())
                .truncate()
                .show_tooltip_when_elided(false),
        );
    }

    let label = tr("Queue");
    let response = ui.place(
        egui::Rect::from_min_size(
            egui::pos2(inset.right() - 36., center_y - 18.),
            egui::Vec2::splat(36.),
        ),
        track_more_button(
            egui::Button::image(
                Icon::More
                    .image(ui.visuals().text_color(), 18.)
                    .alt_text(&label),
            )
            .frame_when_inactive(false)
            .sense(egui::Sense::click_and_drag()),
            palette.raised,
            highlighted,
        ),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
    });
    response
}

impl App {
    pub(super) fn queue_screen(&mut self, ui: &mut egui::Ui, extend_rows: bool) {
        let empty = self.state.view().session.timeline().is_empty();
        let has_current = self.state.view().session.current().is_some();
        let preview_history =
            self.demo && self.options.scenario.as_deref() == Some("queue-history");
        let shuffle_generation = self.state.playback().queue_shuffle_generation();
        let (scroll_key, mut follow, realign, shuffled) = queue_scroll::QueueScroll::load(
            ui.ctx(),
            preview_history,
            self.state.playback().queue_start_generation(),
            shuffle_generation,
        );
        ui.horizontal(|ui| {
            if empty {
                let heading = empty_label(ui, egui::RichText::new(tr("Queue")).heading());
                self.keyboard.empty(ui, WorkspacePane::Queue, &heading);
                focus_outline(ui, &heading);
            } else {
                ui.heading(tr("Queue"));
            }
            if extend_rows {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if button(ui, Icon::Close, "Close queue") {
                        self.set_queue_panel_open(false);
                        ui.ctx().request_repaint();
                    }
                });
            }
        });
        let status = ui
            .allocate_exact_size(egui::vec2(ui.available_width(), 24.), egui::Sense::hover())
            .0;
        let mut status_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(status)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        status_ui.spacing_mut().interact_size.y = status.height();
        status_ui.spacing_mut().button_padding.y = 2.;
        if has_current && !follow.following {
            let back = status_ui.button(tr("Back to current track"));
            #[cfg(test)]
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::new("test-queue-return"), back.rect));
            if back.clicked() {
                follow.following = true;
                ui.request_repaint();
            }
        } else {
            status_ui.weak(if self.state.view().shuffle {
                tr("Shuffled order")
            } else {
                tr("Playlist order")
            });
            #[cfg(test)]
            ui.ctx()
                .data_mut(|data| data.remove::<egui::Rect>(egui::Id::new("test-queue-return")));
        }
        if empty {
            ui.label(tr("Your queue is empty"));
        }
        let mut action = None;
        let mut reordering = false;
        let mut edited_range = None;
        let mut removed_range = None;
        let now = self.queue_notice_time(ui);
        let notice_bounds = ui
            .max_rect()
            .intersect(ui.clip_rect())
            .with_min_y(ui.cursor().top());
        #[cfg(test)]
        self.test_queue_rows.clear();
        let mut list_frame = if extend_rows {
            track_list_frame()
        } else {
            egui::Frame::NONE
        }
        .begin(ui);
        let list_ui = &mut list_frame.content_ui;
        let session = &self.state.view().session;
        let queue = session.timeline();
        let boundary = queue.upcoming_start() + session.available_len();
        let spacing = list_ui.spacing().item_spacing.y;
        let stride = design::ROW_HEIGHT + spacing;
        let count = queue.len();
        let rows_height = (stride * count as f32 - spacing).max(0.);
        let current = queue.current_index();
        let anchor = current
            .or_else(|| (queue.upcoming_start() < queue.len()).then_some(queue.upcoming_start()))
            .map(|i| (queue.get(i).unwrap().entry_id, i));
        let target = if shuffled && !has_current {
            0.
        } else {
            anchor.map_or_else(
                || (rows_height - list_ui.available_height().max(64.)).max(0.),
                |(_, i)| i as f32 * stride,
            )
        };
        let area_id = list_ui.make_persistent_id(egui::IdSalt::new("queue-list"));
        let generation = self.state.queue_reverse_generation();
        let frame_time = list_ui.input(|input| input.time);
        let viewport_height = list_ui.available_height();
        let max = target.max(rows_height - viewport_height);
        let reversed = follow.reverse(
            (generation, realign),
            queue,
            stride,
            max,
            viewport_height,
            frame_time,
        );
        if reversed || shuffled {
            egui::scroll_area::State::default().store(list_ui.ctx(), area_id);
        }
        if shuffled {
            design::stop_middle_scroll(list_ui.ctx(), area_id);
        }
        let page_generation = self.state.queue_page_generation();
        let sample_generation = self.state.playback().shuffle_sample_generation();
        follow.queue_updated(
            queue,
            (page_generation, boundary, sample_generation),
            stride,
            max,
            realign || reversed,
        );
        let keyboard_down = super::page_browse::keyboard_down(list_ui);
        let wheel_moving = list_ui.input(|input| input.smooth_scroll_delta().y != 0.);
        let fade = follow.fade(list_ui);
        let middle = design::middle_scroll_active(list_ui.ctx(), area_id);
        let (offset, busy) =
            follow.start(list_ui, stride, target, anchor, realign || reversed, middle);
        if follow.following {
            list_ui.spacing_mut().scroll.fade.strength = 0.;
        }
        let padded_to = if follow.following {
            target
        } else {
            target.max(offset.min(rows_height - design::ROW_HEIGHT))
        };
        let mut selection_scrolled = false;
        let mut area = list_scroll_area(list_ui);
        if shuffled {
            area = area.scroll_source(egui::scroll_area::ScrollSource::NONE);
        }
        let row_revision = (
            generation,
            shuffle_generation,
            page_generation,
            sample_generation,
        );
        let queue_scroll = area
            .id_salt("queue-list")
            .vertical_scroll_offset(offset)
            .show_viewport(list_ui, |ui, viewport| {
                ui.multiply_opacity(fade);
                let queue = self.state.view().session.timeline();
                let height = if anchor.is_some() && padded_to > 0. {
                    rows_height.max(padded_to + viewport.height())
                } else {
                    rows_height
                };
                let (list, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), height),
                    egui::Sense::hover(),
                );
                let key = ui.id().with("reorder");
                let nav = ListNavigation::new(WorkspacePane::Queue, queue.len(), stride);
                let repeated = repeated_upcoming(queue);
                let key_at = |i: usize| queue_row_id(queue, &repeated, key, i);
                let scope = selection::QUEUE;
                self.selection.present_queue(ui.ctx(), queue.len(), key_at);
                let rows = list.with_max_y(list.top() + rows_height);
                if shuffled {
                    self.selection.stop_pointer(scope);
                } else {
                    let (focus, scrolling) =
                        self.selection
                            .pointer(ui, scope, queue.len(), key_at, rows, stride);
                    selection_scrolled = scrolling;
                    if let Some(i) = focus {
                        ui.memory_mut(|memory| memory.request_focus(key_at(i)));
                    }
                    if let Some(i) = self.keyboard.step(
                        ui,
                        nav,
                        true,
                        key_at,
                        Some((&mut self.selection, scope)),
                    ) {
                        let rect = queue_row_rect(list, stride, i as f32);
                        if !ui.is_rect_visible(rect) {
                            self.keyboard.reveal(ui, rect, nav, i);
                        }
                    }
                }
                let allow_drag =
                    !shuffled && !self.selection.tracking(scope) && !self.selection.block_click();
                let (lifted, moved) = if shuffled {
                    let ctx = ui.ctx();
                    let drag = ctx.data_mut(|data| {
                        let drag = data.get_temp::<QueueDrag>(key);
                        data.remove::<QueueDrag>(key);
                        drag
                    });
                    if drag.is_some_and(|drag| ctx.is_being_dragged(drag.id)) {
                        ctx.stop_dragging();
                    }
                    (None, None)
                } else {
                    queue_drag(
                        ui,
                        list,
                        stride,
                        queue,
                        (key, row_revision),
                        key_at,
                        allow_drag && !reversed,
                    )
                };
                reordering = lifted.is_some();
                action = moved;
                let remove = |range: std::ops::RangeInclusive<usize>| AppCommand::RemoveQueueRows {
                    expected: range.map(|i| (i, queue.get(i).unwrap().entry_id)).collect(),
                    now,
                };
                if lifted.is_none() && allow_drag {
                    if let Some(row) = self
                        .keyboard
                        .queue_delete(ui.ctx())
                        .and_then(|id| (0..queue.len()).find(|&i| key_at(i) == id))
                    {
                        let range = self.selection.queue_range().unwrap_or(row..=row);
                        removed_range = Some((key, *range.start()));
                        action = Some(remove(range));
                    }
                }
                let palette = design::colors(ui);
                if let Some(lifted) = &lifted {
                    ui.painter().rect_filled(
                        queue_row_rect(list, stride, lifted.slot as f32),
                        8.,
                        palette.raised,
                    );
                }
                // A lifted entry shifts its neighbours by at most one slot.
                let top = ((ui.clip_rect().top() - list.top()) / stride) as usize;
                let bottom = ((ui.clip_rect().bottom() - list.top()) / stride).ceil() as usize;
                let animation_time = ui.style().animation_time;
                for i in top.saturating_sub(2)..bottom.saturating_add(2).min(queue.len()) {
                    let entry = queue.get(i).unwrap();
                    let past = i < queue.past_len();
                    let current = queue.current_index() == Some(i);
                    let mut slot = i;
                    if let Some(lifted) = &lifted {
                        if i == lifted.from {
                            continue;
                        }
                        slot -= usize::from(i > lifted.from);
                        slot += usize::from(slot >= lifted.slot);
                    }
                    let id = key_at(i);
                    let y = ui.ctx().animate_value_with_time(
                        id.with(("y", row_revision)),
                        slot as f32,
                        animation_time,
                    );
                    let mut rect = queue_row_rect(list, stride, y);
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let x = ui.ctx().animate_value_with_time(
                        id.with(("x", row_revision)),
                        0.,
                        animation_time,
                    );
                    rect = rect.translate(egui::vec2(x, 0.));
                    let track = self.media_track(entry.into());
                    self.image_wanted.push(track.image_file);
                    if track.file == 0 {
                        self.track_wanted.push(track.clone());
                    }
                    let row = ui.interact(rect, id, egui::Sense::click_and_drag());
                    if i == boundary {
                        ui.painter().hline(
                            rect.x_range(),
                            rect.top(),
                            egui::Stroke::new(2., palette.accent),
                        );
                        row.clone()
                            .on_hover_text(tr("More tracks will play before this row"));
                    }
                    if !shuffled {
                        self.keyboard.row(ui, nav, i, id, false, (&row, rect));
                    }
                    row.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            ui.is_enabled(),
                            &track.title,
                        )
                    });
                    let selected_row = self.selection.contains(scope, id);
                    if lifted.is_none() && row.hovered() {
                        ui.ctx().set_cursor_icon(
                            if ui.input(|input| {
                                !input.modifiers.ctrl
                                    && !input.modifiers.shift
                                    && input
                                        .pointer
                                        .hover_pos()
                                        .is_some_and(|pos| !queue_cover_rect(rect).contains(pos))
                            }) {
                                egui::CursorIcon::Grab
                            } else {
                                egui::CursorIcon::PointingHand
                            },
                        );
                    }
                    let fill = palette
                        .track_fill(current, selected_row)
                        .unwrap_or_else(|| {
                            if row.has_focus() {
                                palette.selected
                            } else if row.hovered() {
                                palette.raised
                            } else {
                                palette.panel
                            }
                        });
                    ui.painter().rect_filled(rect, 8., fill);
                    let mut row_ui =
                        ui.new_child(egui::UiBuilder::new().id(id.with("body")).max_rect(rect));
                    let menu = queue_row_contents(
                        &mut row_ui,
                        rect,
                        &track,
                        self.track_cover(ui.ctx(), &track),
                        past,
                        current,
                        current || selected_row || row.has_focus(),
                    );
                    if !shuffled {
                        self.keyboard.row(ui, nav, i, id, true, (&menu, rect));
                        self.selection.start(
                            &row,
                            &menu,
                            scope,
                            (i, id),
                            self.keyboard.source_key(nav.pane),
                            Some(queue_cover_rect(rect)),
                        );
                    }
                    if selected_row {
                        selection::paint(ui, rect);
                    }
                    focus_outline(&row_ui, &row);
                    focus_outline(&row_ui, &menu);
                    #[cfg(test)]
                    self.test_queue_rows
                        .insert(entry.entry_id, (rect, menu.rect, row.id));
                    if shuffled {
                        continue;
                    }
                    let contents = |ui: &mut egui::Ui, items: &mut MenuItems| {
                        if let Some(range) = self
                            .selection
                            .queue_range()
                            .filter(|_| selected_row && self.selection.is_group())
                        {
                            let (start, count) = (*range.start(), range.clone().count());
                            for (label, edit) in [
                                (tr("Move up"), QueueRangeEdit::MoveUp),
                                (tr("Move down"), QueueRangeEdit::MoveDown),
                                (tr("Reverse selected tracks"), QueueRangeEdit::Reverse),
                            ] {
                                if let Some(destination) =
                                    edit.destination(start, count, queue.len())
                                {
                                    let button = track_menu_button(ui, label);
                                    items.push(ui, &button, None);
                                    if button.clicked() {
                                        let expected = range
                                            .clone()
                                            .map(|at| queue.get(at).unwrap().entry_id)
                                            .collect();
                                        action = Some(AppCommand::EditQueueRange {
                                            start,
                                            expected,
                                            edit,
                                        });
                                        let relative = if edit == QueueRangeEdit::Reverse {
                                            *range.end() - i
                                        } else {
                                            i - start
                                        };
                                        edited_range = Some((
                                            key,
                                            destination..=destination + count - 1,
                                            destination + relative,
                                        ));
                                        ui.close();
                                    }
                                }
                            }
                            let button = track_menu_button(ui, tr("Remove selected tracks"));
                            items.push(ui, &button, None);
                            if button.clicked() {
                                removed_range = Some((key, start));
                                action = Some(remove(range));
                                ui.close();
                            }
                            return;
                        }
                        let move_to = |to| AppCommand::MoveQueueRow {
                            from: i,
                            entry_id: entry.entry_id,
                            to,
                        };
                        for (available, label, command) in [
                            (i > 0, tr("Move up"), move_to(i.saturating_sub(1))),
                            (i + 1 < queue.len(), tr("Move down"), move_to(i + 1)),
                            (true, tr("Remove"), remove(i..=i)),
                        ] {
                            if available {
                                let button = track_menu_button(ui, label);
                                items.push(ui, &button, None);
                                if button.clicked() {
                                    if matches!(command, AppCommand::RemoveQueueRows { .. }) {
                                        removed_range = Some((key, i));
                                    }
                                    action = Some(command);
                                    ui.close();
                                }
                            }
                        }
                    };
                    let (handled, popup, keep_click) =
                        keyboard::show_track_menu(&row, &menu, selected_row, contents);
                    self.selection.remember_popup(popup, keep_click);
                    if lifted.is_none()
                        && !handled
                        && removed_range.is_none()
                        && !self.selection.block_click()
                        && row.clicked()
                    {
                        if row.clicked_by(egui::PointerButton::Primary) {
                            self.selection.clear(ui.ctx());
                        }
                        if !current {
                            action = Some(AppCommand::PlayQueueRow {
                                index: i,
                                entry_id: entry.entry_id,
                            });
                            if row.has_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter))
                            {
                                ui.memory_mut(|m| {
                                    m.request_focus(key.with(("current", entry.entry_id)))
                                });
                            }
                        }
                    }
                }
                if let Some(lifted) = lifted {
                    let track = self.media_track(queue.get(lifted.from).unwrap().into());
                    self.image_wanted.push(track.image_file);
                    let id = key.with("lifted");
                    let mut overlay = ui.new_child(
                        egui::UiBuilder::new()
                            .id(id)
                            .layer_id(egui::LayerId::new(egui::Order::Tooltip, id))
                            .max_rect(lifted.rect),
                    );
                    overlay.visuals_mut().disabled_alpha = 1.;
                    overlay.disable();
                    overlay.set_clip_rect(ui.ctx().content_rect());
                    overlay.painter().add(
                        egui::epaint::Shadow {
                            offset: [0, 5],
                            blur: 16,
                            spread: 0,
                            color: egui::Color32::from_black_alpha(65),
                        }
                        .as_shape(lifted.rect, 8),
                    );
                    overlay
                        .painter()
                        .rect_filled(lifted.rect, 8., palette.raised);
                    let current = queue.current_index() == Some(lifted.from);
                    queue_row_contents(
                        &mut overlay,
                        lifted.rect,
                        &track,
                        self.track_cover(ui.ctx(), &track),
                        lifted.from < queue.past_len(),
                        current,
                        current,
                    );
                }
                viewport.min.y
            });
        paint_current_track_marker(list_ui, &queue_scroll, current, count, stride);
        list_frame.end(ui);
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new("test-queue-scroll"), queue_scroll.id);
            data.insert_temp(
                queue_scroll.id.with("test-viewport"),
                queue_scroll.inner_rect,
            );
        });
        if !shuffled {
            design::middle_scroll(
                ui,
                queue_scroll.id,
                queue_scroll.inner_rect,
                queue_scroll.content_size.y,
            );
        }
        let middle = design::middle_scroll_active(ui.ctx(), queue_scroll.id);
        follow.finish(
            ui,
            &queue_scroll,
            target,
            busy,
            reordering || reversed || shuffled,
            middle,
        );
        let preview_paging = self.demo
            && matches!(
                self.options.scenario.as_deref(),
                Some("queue-continuation" | "queue-continuation-top")
            );
        if (!self.demo || preview_paging) && !self.settings_open && self.offline_confirm.is_none() {
            let session = &self.state.view().session;
            let top_boundary = session.timeline().past_len() - session.available_past_len();
            let pages = [
                session.page_request(false).is_some(),
                session.page_request(true).is_some(),
            ];
            let viewport = queue_scroll.inner_rect;
            let offset = follow.offset();
            let lookahead = (gramwave_application::page_browse::QUEUE_PREFETCH_ROWS + 1) as f32;
            let keyboard_up = ui.input(|input| {
                [egui::Key::ArrowUp, egui::Key::PageUp, egui::Key::Home]
                    .iter()
                    .any(|&key| input.key_pressed(key))
            });
            let pointer = middle
                || ui.input(|input| {
                    input.pointer.primary_down()
                        || input
                            .pointer
                            .hover_pos()
                            .is_some_and(|point| viewport.contains(point))
                            && wheel_moving
                });
            for (before, has_page, edge, key) in [
                (true, pages[1], top_boundary, keyboard_up),
                (false, pages[0], boundary, keyboard_down),
            ] {
                if !has_page {
                    continue;
                }
                let end = edge as f32 * stride;
                let ahead = if before { lookahead - 1. } else { lookahead };
                let signal = super::page_browse::demand(
                    ui,
                    viewport,
                    (queue_scroll.inner, offset),
                    end,
                    (ahead * stride, (ahead + 8.) * stride),
                    pointer || key,
                    before,
                );
                if !shuffled
                    && !reversed
                    && !preview_paging
                    && self.state.browse_queue_on(before, signal)
                {
                    ui.request_repaint();
                }
                super::page_browse::indicator(
                    ui,
                    viewport,
                    offset,
                    end.min(rows_height),
                    if preview_paging {
                        gramwave_application::page_browse::PageStatus::Loading
                    } else {
                        self.state.queue_page_status_on(before)
                    },
                    before,
                );
            }
        }
        follow.remember(
            self.state.view().session.timeline(),
            queue_scroll.inner_rect.height(),
        );
        if selection_scrolled {
            follow.browse();
        }
        if let Some(action) = action {
            let selected = match action {
                AppCommand::PlayQueueRow { entry_id, .. } => Some(entry_id),
                _ => None,
            };
            self.execute(action);
            if let Some(id) = selected {
                let queue = self.state.view().session.timeline();
                if let Some(index) = queue.current_index() {
                    if queue.get(index).is_some_and(|entry| entry.entry_id == id) {
                        follow.selected((id, index), target, frame_time);
                    }
                }
            }
            if let Some((key, start)) = removed_range {
                self.selection.clear(ui.ctx());
                let queue = self.state.view().session.timeline();
                if let Some(last) = queue.len().checked_sub(1) {
                    let id = queue_row_id(queue, &repeated_upcoming(queue), key, start.min(last));
                    self.keyboard.focus_row(WorkspacePane::Queue, id);
                } else {
                    self.keyboard.focus_empty(WorkspacePane::Queue);
                }
            }
            if let Some((key, range, focus)) = edited_range {
                let queue = self.state.view().session.timeline();
                let repeated = repeated_upcoming(queue);
                let key_at = |i| queue_row_id(queue, &repeated, key, i);
                if *range.end() < queue.len() {
                    self.selection.set_queue(range, key_at);
                    self.keyboard.focus_row(WorkspacePane::Queue, key_at(focus));
                }
            }
            ui.request_repaint();
            if selected.is_none() {
                self.persist();
            }
        }
        ui.ctx()
            .data_mut(|data| data.insert_temp(scroll_key, follow));
        self.queue_removal_notices(ui, notice_bounds, now);
    }
}
