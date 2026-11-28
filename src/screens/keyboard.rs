use super::{
    selection::{Scope, Selection},
    WorkspacePane,
};
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
        mut selection: Option<(&mut Selection, Scope)>,
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
        let ctrl_tap = selection
            .as_ref()
            .is_some_and(|(selection, _)| selection.ctrl_tap);
        let skip_menu = pane == WorkspacePane::Playlist
            && selection
                .as_ref()
                .is_some_and(|(selection, scope)| selection.active(*scope));
        let has_menu = has_menu && !skip_menu;
        if let Some(source) = self
            .source
            .filter(|row| row.pane == pane && (nav_pressed || ctrl_tap))
        {
            if let Some(i) = (0..len).find(|&i| key_at(i) == source.key) {
                let ctx = ui.ctx();
                if let Some((selection, scope)) = &mut selection {
                    if ctrl_tap {
                        selection.toggle(ctx, *scope, i, &key_at);
                        self.focus_row(pane, key_at(i));
                        return Some(i);
                    }
                    for (modifiers, add) in [(Modifiers::CTRL, true), (Modifiers::SHIFT, false)] {
                        for (key, backwards, index) in [
                            (Key::ArrowUp, true, i.saturating_sub(1)),
                            (Key::ArrowDown, false, (i + 1).min(len - 1)),
                        ] {
                            if take_key(ctx, modifiers, key) {
                                selection.extend(ctx, *scope, len, (i, index), add, &key_at);
                                self.focus_row(pane, key_at(index));
                                self.backward = backwards;
                                return Some(index);
                            }
                        }
                    }
                }
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
                } else if has_menu && source.menu && take_key(ctx, Modifiers::SHIFT, Key::Tab) {
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

    pub(super) fn source_key(&self, pane: WorkspacePane) -> Option<Id> {
        self.source
            .filter(|row| row.pane == pane)
            .map(|row| row.key)
    }

    pub(super) fn queue_delete(&self, ctx: &egui::Context) -> Option<Id> {
        let key = self.source_key(WorkspacePane::Queue)?;
        (self.enabled
            && !ctx.text_edit_focused()
            && ctx.input_mut(|input| {
                let count = input.events.len();
                input.events.retain(|event| {
                    !matches!(event, egui::Event::Key {
                    key: Key::Delete, pressed: true, repeat: false, modifiers, ..
                } if modifiers.is_none())
                });
                count != input.events.len()
            }))
        .then_some(key)
    }

    pub(super) fn focus_row(&mut self, pane: WorkspacePane, key: Id) {
        self.target = Some(Row {
            pane,
            key,
            menu: false,
        });
    }

    pub(super) fn focus_empty(&mut self, pane: WorkspacePane) {
        self.target = None;
        self.destination = Some(pane);
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
            } if *pressed_key == key && (if modifiers == Modifiers::CTRL {
                super::selection::plain_ctrl(*pressed_modifiers)
            } else {
                pressed_modifiers.matches_exact(modifiers)
            }))
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

#[derive(Clone, Default)]
pub(super) struct MenuItems(Vec<(Id, Option<Id>)>);

impl MenuItems {
    pub(super) fn push(&mut self, ui: &Ui, main: &Response, alternate: Option<&Response>) {
        if !main.enabled() {
            return;
        }
        focus_outline(ui, main);
        if let Some(alternate) = alternate {
            focus_outline(ui, alternate);
        }
        self.0.push((main.id, alternate.map(|button| button.id)));
    }

    pub(super) fn links(&mut self, links: impl IntoIterator<Item = Id>) {
        self.0.extend(links.into_iter().map(|id| (id, None)));
    }

    fn contains(&self, focused: Option<Id>) -> bool {
        focused.is_some_and(|id| {
            self.0
                .iter()
                .any(|&(main, alt)| id == main || Some(id) == alt)
        })
    }

    fn destination(&self, focused: Option<Id>, key: Key, backwards: bool) -> Option<Id> {
        let Some(focused) = focused.filter(|id| self.contains(Some(*id))) else {
            return match key {
                Key::Tab if backwards => self.0.last().map(|&(main, alt)| alt.unwrap_or(main)),
                Key::ArrowUp => self.0.last().map(|&(main, _)| main),
                Key::Tab | Key::ArrowDown => self.0.first().map(|&(main, _)| main),
                _ => None,
            };
        };
        if key == Key::Tab {
            let order: Vec<_> = self
                .0
                .iter()
                .flat_map(|&(main, alt)| [Some(main), alt].into_iter().flatten())
                .collect();
            let index = order.iter().position(|&id| id == focused)?;
            let step = if backwards { order.len() - 1 } else { 1 };
            return Some(order[(index + step) % order.len()]);
        }
        let row = self
            .0
            .iter()
            .position(|&(main, alt)| focused == main || Some(focused) == alt)?;
        let (main, alt) = self.0[row];
        let right = Some(focused) == alt;
        match key {
            Key::ArrowLeft => Some(main),
            Key::ArrowRight => alt,
            Key::ArrowUp | Key::ArrowDown => {
                let step = if key == Key::ArrowUp {
                    self.0.len() - 1
                } else {
                    1
                };
                let (main, alt) = self.0[(row + step) % self.0.len()];
                Some(if right { alt.unwrap_or(main) } else { main })
            }
            _ => None,
        }
    }
}

#[derive(Clone)]
struct TrackMenu {
    id: Id,
    opener: Id,
    items: MenuItems,
}

fn track_menu_key() -> Id {
    Id::new("track-popup-keyboard")
}

pub(super) fn prepare_track_menu(ctx: &egui::Context) -> bool {
    let Some(popup) = ctx.data(|data| data.get_temp::<TrackMenu>(track_menu_key())) else {
        return false;
    };
    if !egui::Popup::is_id_open(ctx, popup.id) {
        return false;
    }
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        egui::Popup::close_id(ctx, popup.id);
        ctx.memory_mut(|memory| memory.request_focus(popup.opener));
        return true;
    }
    if !popup.items.contains(ctx.memory(|m| m.focused())) {
        take_key(ctx, Modifiers::NONE, Key::Enter);
        take_key(ctx, Modifiers::NONE, Key::Space);
    }
    for (key, modifiers) in [
        (Key::Tab, Modifiers::NONE),
        (Key::Tab, Modifiers::SHIFT),
        (Key::ArrowUp, Modifiers::NONE),
        (Key::ArrowDown, Modifiers::NONE),
        (Key::ArrowLeft, Modifiers::NONE),
        (Key::ArrowRight, Modifiers::NONE),
    ] {
        if take_key(ctx, modifiers, key) {
            if let Some(id) =
                popup
                    .items
                    .destination(ctx.memory(|m| m.focused()), key, modifiers.shift)
            {
                ctx.memory_mut(|memory| memory.request_focus(id));
            }
        }
    }
    true
}

pub(super) fn show_track_menu(
    row: &Response,
    menu: &Response,
    selected: bool,
    mut contents: impl FnMut(&mut Ui, &mut MenuItems),
) -> (bool, Option<Id>, bool) {
    let ctx = &row.ctx;
    let enter = selected
        && row.has_focus()
        && !egui::Popup::is_any_open(ctx)
        && ctx.input(|i| i.key_pressed(Key::Enter));
    let menu_id = egui::Popup::default_response_id(menu);
    let row_id = egui::Popup::default_response_id(row);
    let was_open_menu = egui::Popup::is_id_open(ctx, menu_id);
    let was_open_row = egui::Popup::is_id_open(ctx, row_id);
    let mut popup = egui::Popup::menu(menu);
    if enter {
        popup = popup.open_memory(Some(egui::SetOpenCommand::Bool(true)));
    }
    let mut draw = |ui: &mut Ui| {
        ui.set_width(208.);
        crate::design::menu_rows(ui);
        let mut items = MenuItems::default();
        contents(ui, &mut items);
        items
    };
    let shown = popup.show(&mut draw).map(|response| {
        (
            menu_id,
            if enter { row.id } else { menu.id },
            was_open_menu,
            response,
        )
    });
    let context = egui::Popup::context_menu(row)
        .show(&mut draw)
        .map(|response| (row_id, row.id, was_open_row, response));
    let popup = context.or(shown);
    let keep_click = selected
        && (row.secondary_clicked()
            || menu.clicked_by(egui::PointerButton::Primary)
            || menu.secondary_clicked()
            || popup
                .as_ref()
                .is_some_and(|(_, _, _, popup)| !popup.response.clicked_elsewhere()));
    let handled = enter || menu.clicked() || popup.is_some();
    let selected_popup = selected
        .then(|| popup.as_ref().map(|(id, _, _, _)| *id))
        .flatten();
    if let Some((id, opener, was_open, response)) = popup {
        if egui::Popup::is_id_open(ctx, id) {
            let items = response.inner;
            let old = ctx.data(|data| data.get_temp::<TrackMenu>(track_menu_key()));
            let opener = old
                .filter(|old| was_open && old.id == id)
                .map_or(opener, |old| old.opener);
            if !was_open {
                ctx.memory_mut(|memory| {
                    if let Some(focused) = memory.focused() {
                        memory.surrender_focus(focused);
                    }
                });
            }
            ctx.data_mut(|data| {
                data.insert_temp(track_menu_key(), TrackMenu { id, opener, items })
            });
        }
    }
    (handled, selected_popup, keep_click)
}
