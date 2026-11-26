use super::{design, egui};
use egui::{Context, Id, Key, Modifiers, Pos2, Rect, Response, Ui};
use std::{collections::HashSet, ops::RangeInclusive, sync::Arc, time::Duration};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    Queue,
    Playlist(Id),
}

pub(super) const QUEUE: Scope = Scope::Queue;
const LONG_CLICK: Duration = Duration::from_millis(400);

struct Press {
    scope: Scope,
    index: usize,
    key: Id,
    origin: Pos2,
    time: f64,
    modifiers: Modifiers,
    focus: Option<Id>,
    reorder: bool,
    last_track: Option<usize>,
    base: Option<DragBase>,
}

struct DragBase {
    keys: HashSet<Id>,
    queue: Option<RangeInclusive<usize>>,
    anchor: Option<Id>,
}

#[derive(Default)]
pub(crate) struct Selection {
    scope: Option<Scope>,
    keys: HashSet<Id>,
    queue: Option<RangeInclusive<usize>>,
    queue_keys: Vec<Id>,
    anchor: Option<Id>,
    press: Option<Press>,
    blocked: bool,
    ctrl_pending: bool,
    pub(super) ctrl_tap: bool,
    shown: bool,
    playlist: Option<Arc<Vec<crate::Track>>>,
    popup: Option<Id>,
    keep_click: bool,
}

impl Selection {
    pub(super) fn begin(&mut self, ctx: &Context, allow_escape: bool) {
        self.shown = false;
        self.keep_click = false;
        self.ctrl_tap = false;
        ctx.input(|input| {
            if input.pointer.primary_pressed()
                || (!input.pointer.primary_down() && !input.pointer.primary_released())
            {
                self.blocked = false;
                if input.pointer.primary_pressed() {
                    self.press = None;
                }
            }
            if !input.focused {
                self.ctrl_pending = false;
                self.blocked |= self.press.take().is_some();
            }
            for event in &input.events {
                match event {
                    egui::Event::ModifiersChanged(modifiers) if !modifiers.ctrl => {
                        self.ctrl_tap = self.ctrl_pending && modifiers.is_none();
                        self.ctrl_pending = false;
                    }
                    egui::Event::ModifiersChanged(modifiers) => {
                        self.ctrl_pending = plain_ctrl(*modifiers);
                    }
                    egui::Event::Key { pressed: true, .. }
                    | egui::Event::PointerButton { pressed: true, .. } => {
                        self.ctrl_pending = false;
                        self.ctrl_tap = false;
                    }
                    _ => {}
                }
            }
        });
        if allow_escape
            && (self.scope.is_some() || self.press.is_some())
            && ctx.input(|i| i.key_pressed(Key::Escape))
        {
            self.clear(ctx);
        }
    }

    pub(super) fn finish(&mut self, ctx: &Context) {
        let outside_click = self.scope.is_some()
            && !self.keep_click
            && ctx.input(|i| {
                [egui::PointerButton::Primary, egui::PointerButton::Secondary]
                    .into_iter()
                    .any(|button| i.pointer.button_clicked(button)
                        && (button != egui::PointerButton::Primary || !self.blocked)
                        && i.events.iter().any(|event| matches!(event,
                            egui::Event::PointerButton { button: released, pressed: false, modifiers, .. }
                                if *released == button && modifiers.is_none()
                        )))
            });
        if outside_click || ((self.scope.is_some() || self.press.is_some()) && !self.shown) {
            self.clear(ctx);
        }
    }

    pub(super) fn clear(&mut self, ctx: &Context) {
        self.blocked |= self.press.take().is_some();
        if let Some(popup) = self.popup.take() {
            egui::Popup::close_id(ctx, popup);
        }
        self.scope = None;
        self.keys.clear();
        self.queue = None;
        self.queue_keys.clear();
        self.anchor = None;
        self.playlist = None;
    }

    pub(super) fn present_playlist(
        &mut self,
        ctx: &Context,
        scope: Scope,
        tracks: &Arc<Vec<crate::Track>>,
        key_at: impl Fn(usize) -> Id,
    ) {
        if self.scope.is_some_and(|old| old != QUEUE && old != scope)
            || self
                .press
                .as_ref()
                .is_some_and(|p| p.scope != QUEUE && p.scope != scope)
        {
            self.clear(ctx);
        }
        self.shown |=
            self.scope == Some(scope) || self.press.as_ref().is_some_and(|p| p.scope == scope);
        if self.active(scope)
            && self
                .playlist
                .as_ref()
                .is_none_or(|old| !Arc::ptr_eq(old, tracks))
        {
            let present: HashSet<_> = (0..tracks.len()).map(key_at).collect();
            self.keys.retain(|key| present.contains(key));
            if let Some(press) = self.press.as_mut().filter(|press| press.scope == scope) {
                if let Some(base) = &mut press.base {
                    base.keys.retain(|key| present.contains(key));
                }
            }
            if self.keys.is_empty() {
                self.clear(ctx);
            } else {
                self.playlist = Some(tracks.clone());
            }
        }
    }

    pub(super) fn present_queue(
        &mut self,
        ctx: &Context,
        len: usize,
        key_at: impl Fn(usize) -> Id,
    ) {
        self.shown |=
            self.scope == Some(QUEUE) || self.press.as_ref().is_some_and(|p| p.scope == QUEUE);
        if self.queue.as_ref().is_some_and(|range| {
            *range.end() >= len
                || !range
                    .clone()
                    .map(key_at)
                    .eq(self.queue_keys.iter().copied())
        }) {
            self.clear(ctx);
        }
    }

    pub(super) fn active(&self, scope: Scope) -> bool {
        self.scope == Some(scope) && !self.keys.is_empty()
    }

    pub(super) fn contains(&self, scope: Scope, key: Id) -> bool {
        self.scope == Some(scope) && self.keys.contains(&key)
    }

    pub(super) fn is_group(&self) -> bool {
        self.keys.len() > 1
    }

    pub(super) fn queue_range(&self) -> Option<RangeInclusive<usize>> {
        self.queue.clone()
    }

    pub(super) fn tracking(&self, scope: Scope) -> bool {
        self.press
            .as_ref()
            .is_some_and(|press| press.scope == scope)
    }

    pub(super) fn block_click(&self) -> bool {
        self.blocked
    }

    fn use_scope(&mut self, ctx: &Context, scope: Scope) {
        if self.scope != Some(scope) {
            // The other list may still own an open group menu.
            let press = self.press.take();
            self.clear(ctx);
            self.press = press;
            self.scope = Some(scope);
        }
        self.shown = true;
    }

    pub(super) fn set_queue(&mut self, range: RangeInclusive<usize>, key_at: impl Fn(usize) -> Id) {
        self.scope = Some(QUEUE);
        self.queue_keys = range.clone().map(key_at).collect();
        self.keys = self.queue_keys.iter().copied().collect();
        self.anchor = self.queue_keys.first().copied();
        self.queue = Some(range);
    }

    fn range(
        &mut self,
        ctx: &Context,
        scope: Scope,
        range: RangeInclusive<usize>,
        add: bool,
        key_at: impl Fn(usize) -> Id,
    ) {
        self.use_scope(ctx, scope);
        if scope == QUEUE {
            let range = match self.queue.as_ref().filter(|_| add) {
                Some(old) => (*old.start()).min(*range.start())..=(*old.end()).max(*range.end()),
                None => range,
            };
            let anchor = self.anchor;
            self.set_queue(range, key_at);
            self.anchor = anchor;
        } else {
            if !add {
                self.keys.clear();
            }
            self.keys.extend(range.map(key_at));
        }
    }

    pub(super) fn toggle(
        &mut self,
        ctx: &Context,
        scope: Scope,
        index: usize,
        key_at: impl Fn(usize) -> Id,
    ) {
        self.use_scope(ctx, scope);
        let key = key_at(index);
        if scope == QUEUE {
            match self.queue.clone() {
                Some(range) if range.contains(&index) => {
                    if range.start() == range.end() {
                        self.clear(ctx);
                        return;
                    } else if index == *range.start() {
                        self.set_queue(index + 1..=*range.end(), key_at);
                    } else if index == *range.end() {
                        self.set_queue(*range.start()..=index - 1, key_at);
                    }
                }
                _ => self.range(ctx, scope, index..=index, true, key_at),
            }
        } else if !self.keys.remove(&key) {
            self.keys.insert(key);
        } else if self.keys.is_empty() {
            self.clear(ctx);
            return;
        }
        self.anchor = Some(key);
    }

    pub(super) fn extend(
        &mut self,
        ctx: &Context,
        scope: Scope,
        len: usize,
        (from, to): (usize, usize),
        add: bool,
        key_at: impl Fn(usize) -> Id,
    ) {
        self.use_scope(ctx, scope);
        let anchor = if add {
            from
        } else {
            self.anchor
                .and_then(|key| (0..len).find(|&i| key_at(i) == key))
                .unwrap_or(from)
        };
        if !add || self.anchor.is_none() {
            self.anchor = Some(key_at(anchor));
        }
        self.range(ctx, scope, anchor.min(to)..=anchor.max(to), add, key_at);
    }

    pub(super) fn start(
        &mut self,
        row: &Response,
        menu: &Response,
        scope: Scope,
        (index, key): (usize, Id),
        focus: Option<Id>,
        cover: Option<Rect>,
    ) {
        let start = row.ctx.input(|i| {
            if !i.pointer.primary_pressed() || !row.is_pointer_button_down_on() {
                return None;
            }
            let pos = i.pointer.press_origin()?;
            let modifiers = i
                .events
                .iter()
                .find_map(|event| match event {
                    egui::Event::PointerButton {
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or(i.modifiers);
            (!menu.rect.contains(pos)).then_some((pos, modifiers, i.time))
        });
        if let Some((origin, modifiers, time)) = start {
            self.shown = true;
            self.press = Some(Press {
                scope,
                index,
                key,
                origin,
                time,
                modifiers,
                focus,
                reorder: modifiers.is_none() && cover.is_some_and(|rect| !rect.contains(origin)),
                last_track: None,
                base: (plain_ctrl(modifiers) && self.active(scope)).then(|| DragBase {
                    keys: if scope == QUEUE {
                        HashSet::new()
                    } else {
                        self.keys.clone()
                    },
                    queue: self.queue.clone(),
                    anchor: self.anchor,
                }),
            });
        }
    }

    pub(super) fn pointer(
        &mut self,
        ui: &Ui,
        scope: Scope,
        len: usize,
        key_at: impl Fn(usize) -> Id,
        list: Rect,
        stride: f32,
    ) -> (Option<usize>, bool) {
        let Some(mut press) = self.press.take_if(|press| press.scope == scope) else {
            return (None, false);
        };
        let (pos, hover, down, released, elapsed) = ui.input(|i| {
            (
                i.pointer.interact_pos(),
                i.pointer.hover_pos(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.time - press.time,
            )
        });
        if press.index >= len || key_at(press.index) != press.key {
            self.blocked = true;
            return (None, false);
        }
        let moved = pos.is_some_and(|pos| pos.distance(press.origin) >= 4.);
        let held = elapsed >= LONG_CLICK.as_secs_f64();
        if press.reorder && down && moved && ui.ctx().is_being_dragged(press.key) {
            return (None, false);
        }
        let viewport = drag_viewport(ui, list);
        let dragging = !press.reorder && (moved || self.blocked);
        let at = pos.and_then(|pos| hit_row(list, viewport, stride, len, pos, dragging));
        let scrolling = dragging
            && down
            && !released
            && hover.is_some_and(|pos| {
                (viewport.contains(pos) || !viewport.y_range().contains(pos.y))
                    && scroll_drag_edge(ui, list, viewport, pos.y)
            });
        self.blocked |= scrolling;
        let index = press.index;
        let mut focus = None;
        let add = plain_ctrl(press.modifiers);
        let reached_other_row = at.is_some_and(|at| at != index);
        if !press.reorder && (down || released) && (reached_other_row || press.last_track.is_some())
        {
            let first = press.last_track.is_none();
            if let Some(to) = at.filter(|at| Some(*at) != press.last_track) {
                self.drag_range(ui.ctx(), &press, to, &key_at);
                press.last_track = Some(to);
            }
            self.blocked = true;
            focus = if released {
                at
            } else {
                (first && !scrolling).then_some(index)
            };
        } else if released && (moved || held || self.blocked) {
            self.blocked = true;
        } else if released && at == Some(index) {
            if press.modifiers.shift {
                let from = press
                    .focus
                    .and_then(|key| (0..len).find(|&i| key_at(i) == key))
                    .unwrap_or(index);
                self.extend(ui.ctx(), scope, len, (from, index), false, &key_at);
                self.blocked = true;
                focus = Some(index);
            } else if add {
                self.toggle(ui.ctx(), scope, index, &key_at);
                self.blocked = true;
                focus = Some(index);
            } else if press.modifiers.is_none() {
                self.clear(ui.ctx());
            }
        }
        if down && !released {
            self.press = Some(press);
        }
        (focus, scrolling)
    }

    fn drag_range(
        &mut self,
        ctx: &Context,
        press: &Press,
        to: usize,
        key_at: impl Fn(usize) -> Id,
    ) {
        let base = press
            .base
            .as_ref()
            .filter(|base| base.queue.is_some() || !base.keys.is_empty());
        let index = press.index;
        if let Some(base) = base {
            self.use_scope(ctx, press.scope);
            self.keys.clone_from(&base.keys);
            self.queue.clone_from(&base.queue);
            self.anchor = base.anchor;
        }
        if to != index {
            self.range(
                ctx,
                press.scope,
                index.min(to)..=index.max(to),
                base.is_some(),
                key_at,
            );
            self.anchor = Some(press.key);
        } else if let Some(base) = base {
            if let Some(range) = &base.queue {
                self.set_queue(range.clone(), key_at);
                self.anchor = base.anchor;
            }
        } else {
            self.clear(ctx);
        }
    }

    pub(super) fn remember_popup(&mut self, popup: Option<Id>, keep_click: bool) {
        if popup.is_some() {
            self.popup = popup;
        }
        self.keep_click |= keep_click;
    }
}

pub(super) fn plain_ctrl(modifiers: Modifiers) -> bool {
    modifiers.ctrl && !modifiers.shift && !modifiers.alt && !modifiers.mac_cmd
}

pub(super) fn paint(ui: &Ui, rect: Rect) {
    ui.painter().rect_filled(
        Rect::from_min_size(
            egui::pos2(rect.right() - 3., rect.top() + 12.),
            egui::vec2(3., 36.),
        ),
        2.,
        design::colors(ui).accent,
    );
}

pub(super) fn drag_viewport(ui: &Ui, list: Rect) -> Rect {
    let clip = ui.clip_rect();
    Rect::from_x_y_ranges(list.x_range(), clip.y_range()).intersect(clip)
}

pub(super) fn scroll_drag_edge(ui: &Ui, list: Rect, viewport: Rect, y: f32) -> bool {
    if !viewport.is_positive() {
        return false;
    }
    let edge = (viewport.height() / 3.).min(36.);
    let top = (viewport.top() + edge - y).max(0.);
    let bottom = (y - viewport.bottom() + edge).max(0.);
    let dt = ui.input(|i| i.stable_dt.min(0.05));
    let delta = ((top - bottom) * 18. * dt).clamp(
        -(list.bottom() - viewport.bottom()).max(0.),
        (viewport.top() - list.top()).max(0.),
    );
    if delta == 0. {
        return false;
    }
    ui.scroll_with_delta_animation(egui::vec2(0., delta), egui::style::ScrollAnimation::none());
    ui.request_repaint();
    true
}

fn hit_row(
    list: Rect,
    viewport: Rect,
    stride: f32,
    len: usize,
    pos: Pos2,
    dragging: bool,
) -> Option<usize> {
    let visible = list.intersect(viewport);
    if !visible.is_positive() {
        return None;
    }
    if dragging && pos.y < viewport.top() {
        let top = visible.top() - list.top();
        let index = (top / stride) as usize + usize::from(top % stride >= design::ROW_HEIGHT);
        return (index < len && list.top() + (index as f32) * stride < visible.bottom())
            .then_some(index);
    }
    if dragging && pos.y > viewport.bottom() {
        let end = ((visible.bottom() - list.top()) / stride).ceil() as usize;
        return end.min(len).checked_sub(1);
    }
    if !visible.contains(pos) {
        return None;
    }
    let y = pos.y - list.top();
    let index = (y / stride) as usize;
    (index < len && y % stride < design::ROW_HEIGHT).then_some(index)
}

pub(super) fn scope(chat: i64, search: &str, sort: usize, filters: [bool; 3]) -> Scope {
    Scope::Playlist(Id::new((chat, search, sort, filters)))
}
