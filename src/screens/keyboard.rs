use super::WorkspacePane;
use eframe::egui::{self, Id, Key, Modifiers, Rect, Response, Ui};
use std::collections::HashMap;

#[derive(Clone, Copy)]
struct Row {
    pane: WorkspacePane,
    key: Id,
    menu: bool,
}

#[derive(Clone, Copy)]
pub(super) struct ListNavigation {
    pub(super) pane: WorkspacePane,
    len: usize,
    stride: f32,
}

impl ListNavigation {
    pub(super) fn new(pane: WorkspacePane, len: usize, stride: f32) -> Self {
        Self { pane, len, stride }
    }
}

#[derive(Default)]
pub(crate) struct Keyboard {
    rows: HashMap<Id, Row>,
    source: Option<Row>,
    destination: Option<WorkspacePane>,
    target: Option<Row>,
    backward: bool,
    enabled: bool,
}

impl Keyboard {
    #[cfg(test)]
    pub(super) fn focused(&self, ctx: &egui::Context) -> Option<(WorkspacePane, Id, bool)> {
        let id = ctx.memory(|m| m.focused())?;
        self.rows.get(&id).map(|r| (r.pane, r.key, r.menu))
    }

    pub(super) fn begin(&mut self, ctx: &egui::Context, enabled: bool) -> Option<WorkspacePane> {
        self.enabled = enabled;
        self.source = ctx
            .memory(|m| m.focused())
            .and_then(|id| self.rows.get(&id).copied());
        self.rows.clear();
        if !enabled || ctx.input(|i| i.pointer.any_pressed() || i.key_pressed(Key::Escape)) {
            self.destination = None;
            self.target = None;
        }
        if !enabled {
            return None;
        }
        if let Some(backward) = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Key {
                    key: Key::Tab,
                    pressed: true,
                    modifiers,
                    ..
                } => Some(modifiers.shift),
                _ => None,
            })
        }) {
            self.backward = backward;
        }
        let pane = self.source?.pane;
        let (left, right) = match pane {
            WorkspacePane::Library => (None, Some(WorkspacePane::Playlist)),
            WorkspacePane::Playlist => (Some(WorkspacePane::Library), Some(WorkspacePane::Queue)),
            WorkspacePane::Queue => (Some(WorkspacePane::Playlist), None),
        };
        let next = if take_key(ctx, Modifiers::NONE, Key::ArrowLeft) {
            left
        } else if take_key(ctx, Modifiers::NONE, Key::ArrowRight) {
            right
        } else {
            return None;
        };
        if let Some(next) = next {
            self.destination = Some(next);
            self.target = None;
            self.backward = false;
        }
        next
    }

    pub(super) fn step(
        &mut self,
        ui: &Ui,
        list: ListNavigation,
        has_menu: bool,
        key_at: impl Fn(usize) -> Id,
    ) -> Option<usize> {
        if !self.enabled {
            return None;
        }
        let ListNavigation { pane, len, .. } = list;
        let nav_pressed = ui.input(|i| {
            [Key::Tab, Key::ArrowUp, Key::ArrowDown]
                .iter()
                .any(|&key| i.key_pressed(key))
        });
        if let Some(source) = self.source.filter(|row| row.pane == pane && nav_pressed) {
            if let Some(i) = (0..len).find(|&i| key_at(i) == source.key) {
                let ctx = ui.ctx();
                let movement = if take_key(ctx, Modifiers::NONE, Key::ArrowUp) {
                    Some((i.saturating_sub(1), false, true))
                } else if take_key(ctx, Modifiers::NONE, Key::ArrowDown) {
                    Some(((i + 1).min(len - 1), false, false))
                } else if has_menu && !source.menu && take_key(ctx, Modifiers::NONE, Key::Tab) {
                    Some((i, true, false))
                } else if (!has_menu || source.menu)
                    && i + 1 < len
                    && take_key(ctx, Modifiers::NONE, Key::Tab)
                {
                    Some((i + 1, false, false))
                } else if source.menu && take_key(ctx, Modifiers::SHIFT, Key::Tab) {
                    Some((i, false, true))
                } else if i > 0 && take_key(ctx, Modifiers::SHIFT, Key::Tab) {
                    Some((i - 1, has_menu, true))
                } else {
                    None
                };
                if let Some((index, menu, backward)) = movement {
                    self.backward = backward;
                    self.target = Some(Row {
                        pane,
                        key: key_at(index),
                        menu,
                    });
                    return Some(index);
                }
            }
        }
        let target = self.target.filter(|row| row.pane == pane)?;
        let index = (0..len).find(|&i| key_at(i) == target.key);
        if index.is_none() {
            self.target = None;
        }
        index
    }

    pub(super) fn row(
        &mut self,
        ui: &Ui,
        list: ListNavigation,
        index: usize,
        key: Id,
        menu: bool,
        (response, rect): (&Response, Rect),
    ) {
        let ListNavigation { pane, stride, .. } = list;
        self.rows.insert(response.id, Row { pane, key, menu });
        if !self.enabled {
            return;
        }
        let clip = ui.clip_rect();
        let visible = rect.intersect(clip).is_positive();
        let targeted = self
            .target
            .is_some_and(|row| row.pane == pane && row.key == key && row.menu == menu);
        // Prefer a full row when entering a pane which is already scrolled.
        let entering = !menu
            && self.destination == Some(pane)
            && (rect.top() >= clip.top() || clip.height() < stride);
        if visible && (targeted || entering) {
            response.request_focus();
            self.target = None;
            self.destination = None;
        }
        if visible
            && (targeted || (response.gained_focus() && !ui.input(|i| i.pointer.any_released())))
        {
            self.reveal(ui, rect, list, index);
        }
    }

    pub(super) fn targets(&self, pane: WorkspacePane, key: Id) -> bool {
        self.target
            .is_some_and(|row| row.pane == pane && row.key == key)
    }

    pub(super) fn reveal(&self, ui: &Ui, row: Rect, list: ListNavigation, index: usize) {
        let ListNavigation { stride, len, .. } = list;
        let clip = ui.clip_rect();
        let neighbor = if self.backward && index > 0 {
            -stride
        } else if !self.backward && index + 1 < len {
            stride
        } else {
            0.
        };
        let pair = row.union(row.translate(egui::vec2(0., neighbor)));
        let rect = if pair.height() <= clip.height() {
            pair
        } else {
            row
        };
        let delta = if rect.top() < clip.top() {
            clip.top() - rect.top()
        } else if rect.bottom() > clip.bottom() {
            -(rect.bottom() - clip.bottom())
        } else {
            return;
        };
        ui.scroll_with_delta(egui::vec2(0., delta));
    }

    pub(super) fn empty(&mut self, ui: &Ui, pane: WorkspacePane, response: &Response) {
        self.rows.insert(
            response.id,
            Row {
                pane,
                key: response.id,
                menu: false,
            },
        );
        if self.enabled && self.destination == Some(pane) && ui.is_rect_visible(response.rect) {
            self.destination = None;
            response.request_focus();
        }
    }
}

pub(super) fn row_sense(ui: &Ui, rect: Rect) -> egui::Sense {
    if ui.is_rect_visible(rect) {
        egui::Sense::click()
    } else {
        egui::Sense::CLICK
    }
}

fn take_key(ctx: &egui::Context, modifiers: Modifiers, key: Key) -> bool {
    let consumed = ctx.input_mut(|i| {
        let count = i.events.len();
        i.events.retain(|event| {
            !matches!(event, egui::Event::Key {
                key: pressed_key, pressed: true, modifiers: pressed_modifiers, ..
            } if *pressed_key == key && pressed_modifiers.matches_exact(modifiers))
        });
        i.events.len() != count
    });
    if consumed {
        // egui reads focus-navigation keys before building any widgets.
        ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
    }
    consumed
}

pub(super) fn empty_label(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    ui.add(egui::Label::new(text).sense(egui::Sense::focusable_noninteractive()))
}

pub(super) fn focus_outline(ui: &Ui, response: &Response) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect,
            8.,
            egui::Stroke::new(1., crate::design::colors(ui).accent),
            egui::StrokeKind::Inside,
        );
    }
}

pub(super) fn popup_focus(button: &Response, was_open: bool, first: Option<Id>) {
    let ctx = &button.ctx;
    if button.has_focus() && button.clicked() && ctx.input(|i| i.key_pressed(Key::Enter)) {
        if let Some(id) = first {
            ctx.memory_mut(|m| m.request_focus(id));
        }
    } else if was_open && ctx.input(|i| i.key_pressed(Key::Escape)) {
        button.request_focus();
    }
}
