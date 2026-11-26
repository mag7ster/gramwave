use crate::application::commands::AppCommand;
use crate::{design, tr, App, RepeatMode};
use design::{button, Icon};
use eframe::egui;
use gramwave_domain::{
    playback::{QueueRangeEdit, QueueTimeline},
    QueueEntryId,
};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

mod keyboard;
mod queue_scroll;
mod selection;
pub(crate) use keyboard::Keyboard;
use keyboard::{empty_label, focus_outline, ListNavigation, MenuItems};
pub(crate) use selection::Selection;

const TRACK_SCROLL_GUTTER: i8 = 12;
const TRACK_ROW_PADDING: f32 = 4.;
const CHAT_SEARCH_TRIGGER: &str = "chat-search-trigger";

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub(crate) enum WorkspacePane {
    Library,
    #[default]
    Playlist,
    Queue,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlayerLayout {
    Controls,
    Mini,
    Dense,
    Full,
}

impl PlayerLayout {
    const MIN_WORKSPACE_HEIGHT: f32 = 260.;

    fn for_size(size: egui::Vec2) -> Self {
        if size.y < 150. || size.x < 300. {
            Self::Controls
        } else if size.y < Self::MIN_WORKSPACE_HEIGHT {
            Self::Mini
        } else if size.y < 350. {
            Self::Dense
        } else {
            Self::Full
        }
    }

    fn height(self) -> f32 {
        match self {
            Self::Controls => 44.,
            Self::Mini => 84.,
            Self::Dense => 52.,
            Self::Full => 96.,
        }
    }

    fn has_workspace(self) -> bool {
        matches!(self, Self::Dense | Self::Full)
    }

    fn margin(self) -> egui::Vec2 {
        match self {
            Self::Controls | Self::Mini => egui::vec2(8., 0.),
            Self::Dense => egui::vec2(4., 4.),
            Self::Full => egui::vec2(8., 2.),
        }
    }

    fn transport_rect(self, ctx: &egui::Context, viewport: egui::Rect) -> egui::Rect {
        let alignment = if self.has_workspace() { 1. } else { 0.5 };
        let anchor =
            ctx.animate_value_with_time(egui::Id::new("transport-anchor"), alignment, 0.17);
        let offset = ctx.animate_value_with_time(
            egui::Id::new("transport-offset"),
            -self.height() * alignment,
            0.17,
        );
        // Keep the main buttons on-screen while the timeline slides in below them.
        let controls_top = if matches!(self, Self::Mini | Self::Full) {
            8.
        } else {
            0.
        };
        let max_y = viewport.bottom() - self.margin().y - controls_top - Self::Controls.height();
        let y = (viewport.top() + viewport.height() * anchor + offset)
            .clamp(viewport.top(), max_y.max(viewport.top()));
        egui::Rect::from_min_size(
            egui::pos2(viewport.left(), y),
            egui::vec2(viewport.width(), self.height()),
        )
    }
}

fn change_progress<T: Copy + Eq + Send + Sync + 'static>(
    ui: &egui::Ui,
    id: &'static str,
    target: T,
    duration: f64,
) -> f32 {
    let id = egui::Id::new(id);
    let now = ui.input(|i| i.time);
    let started = ui.data_mut(|data| {
        let state = data.get_temp_mut_or(id, (target, f64::NEG_INFINITY));
        if state.0 != target {
            *state = (target, now);
        }
        state.1
    });
    let progress = ((now - started) / duration).clamp(0., 1.) as f32;
    if progress < 1. {
        ui.ctx().request_repaint();
    }
    progress
}

pub(crate) fn reveal_layout<T: Copy + Eq + Send + Sync + 'static>(
    ui: &mut egui::Ui,
    id: &'static str,
    target: T,
) {
    let progress = change_progress(ui, id, target, 0.16);
    ui.multiply_opacity(egui::lerp(
        0.7..=1.,
        egui::emath::easing::cubic_out(progress),
    ));
}

fn layout_fraction(ui: &egui::Ui, id: &'static str, target: bool) -> f32 {
    let value = ui
        .ctx()
        .animate_bool_with_time(egui::Id::new(id), target, 0.18);
    value * value * (3. - 2. * value)
}

fn horizontal_slot(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    left_center: egui::Pos2,
    size: egui::Vec2,
) -> egui::Ui {
    let rect = egui::Rect::from_min_size(left_center - egui::vec2(0., size.y / 2.), size);
    ui.new_child(
        egui::UiBuilder::new()
            .id(ui.id().with(id))
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    )
}

fn search_field(
    ui: &mut egui::Ui,
    query: &mut String,
    id: &'static str,
    hint: String,
    mut active: Option<&mut bool>,
) -> Option<egui::Response> {
    let (rect, field) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.), egui::Sense::CLICK);
    let p = design::colors(ui);
    ui.painter().rect_filled(rect, 9., p.raised);
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(8., 2.)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 6.;
    row.spacing_mut().button_padding = egui::Vec2::splat(7.);
    row.add(Icon::Search.image(p.muted, 18.));
    if let Some(active) = active.as_deref_mut().filter(|active| !**active) {
        row.weak(hint);
        let trigger = ui.interact(
            rect,
            egui::Id::new(CHAT_SEARCH_TRIGGER),
            egui::Sense::click(),
        );
        trigger.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tr("Find a chat"))
        });
        if trigger.clicked() {
            *active = true;
            ui.memory_mut(|m| m.request_focus(egui::Id::new(id)));
            ui.ctx().request_repaint();
        }
        return Some(trigger);
    }
    let width = (row.available_width() - 30. - row.spacing().item_spacing.x).max(0.);
    let edit = row.add(
        egui::TextEdit::singleline(query)
            .id(egui::Id::new(id))
            .hint_text(hint)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .vertical_align(egui::Align::Center)
            .desired_width(width)
            .min_size(egui::vec2(width, 36.)),
    );
    if field.clicked() {
        edit.request_focus();
    }
    if active.as_deref().copied().unwrap_or(!query.is_empty()) {
        let label = if active.is_some() {
            tr("Close search")
        } else {
            tr("Clear search")
        };
        let close = row.add(
            egui::Button::image(Icon::Close.image(p.muted, 16.).alt_text(&label))
                .frame_when_inactive(false)
                .min_size(egui::vec2(30., 36.)),
        );
        let escape = !egui::Popup::is_any_open(ui.ctx())
            && ui.memory(|m| m.is_above_modal_layer(ui.layer_id()))
            && ui.input(|i| i.key_pressed(egui::Key::Escape));
        if close.clicked()
            || (escape && (active.is_some() || edit.has_focus() || edit.lost_focus()))
        {
            if let Some(active) = active.as_deref_mut() {
                *active = false;
                ui.memory_mut(|m| m.request_focus(egui::Id::new(CHAT_SEARCH_TRIGGER)));
                ui.ctx().request_repaint();
            }
            query.clear();
            edit.surrender_focus();
        }
    }
    if let Some(active) = active {
        let accent = ui
            .ctx()
            .animate_bool(egui::Id::new(id).with("accent"), *active);
        if accent > 0. {
            ui.painter().rect_stroke(
                rect,
                9.,
                egui::Stroke::new(1., p.accent.gamma_multiply(0.7 * accent)),
                egui::StrokeKind::Inside,
            );
        }
    }
    None
}

impl App {
    fn apply_application_menu(
        &mut self,
        ctx: &egui::Context,
        action: Option<ApplicationMenuAction>,
    ) {
        match action {
            Some(ApplicationMenuAction::Settings) => self.settings_open = true,
            Some(ApplicationMenuAction::Quit) => {
                self.execute(AppCommand::Quit);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            None => {}
        }
    }

    pub(crate) fn render(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        self.image_wanted.clear();
        self.track_wanted.clear();
        let track_menu_open = keyboard::prepare_track_menu(&ctx);
        design::style(&ctx, self.state.view().light);
        root.set_style(ctx.global_style());
        if !track_menu_open
            && !self.settings_open
            && !ctx.text_edit_focused()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space))
        {
            self.execute(AppCommand::Pause(!self.state.playback().state().paused()));
        }

        if ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::Q)) {
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let needs_auth = self.options.preview_auth.is_some()
            || (!self.demo
                && *self.state.auth().state()
                    != gramwave_application::telegram::AuthorizationState::Ready);
        let header_visible = fastframe_titlebar::visible(&ctx);
        if header_visible {
            let action = window_header(
                root,
                self.state
                    .view()
                    .session
                    .current()
                    .map(|track| track.title.as_str()),
                !needs_auth,
            );
            self.apply_application_menu(&ctx, action);
        }
        if needs_auth {
            egui::CentralPanel::default().show(root, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let compact = ui.available_width() < 480. || ctx.content_rect().height() < 400.;
                    reveal_layout(ui, "auth-layout", compact);
                    ui.add_space(if compact { 6. } else { 50. });
                    ui.vertical_centered(|ui| {
                        ui.heading(egui::RichText::new("Gramwave").size(32.));
                        ui.weak(tr("Your Telegram. Your soundtrack."));
                    });
                    ui.add_space(if compact { 6. } else { 28. });
                    ui.horizontal(|ui| {
                        let width = ui.available_width().min(440.);
                        ui.add_space((ui.available_width() - width) / 2.);
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, 0.),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                egui::Frame::new()
                                    .fill(design::palette(self.state.view().light).panel)
                                    .corner_radius(16)
                                    .inner_margin(if compact { 10. } else { 28. })
                                    .show(ui, |ui| {
                                        self.auth_ui(ui);
                                    });
                            },
                        );
                    });
                });
            });
            self.error_ui(&ctx);
            return;
        }
        let size = ctx.content_rect().size();
        let dock = size.x >= 1040.;
        let compact_heading = size.y < 600. || size.x < 520.;
        let player = PlayerLayout::for_size(size);
        let viewport = root.available_rect_before_wrap();
        let transport_rect = player.transport_rect(&ctx, viewport);
        let workspace = player.has_workspace();
        let dense = player == PlayerLayout::Dense;
        self.selection.begin(
            &ctx,
            workspace && !self.settings_open && self.offline_confirm.is_none(),
        );
        if let Some(pane) = self.keyboard.begin(
            &ctx,
            workspace
                && !self.settings_open
                && self.offline_confirm.is_none()
                && !track_menu_open
                && !egui::Popup::is_any_open(&ctx),
        ) {
            if !dock {
                self.pane = pane;
            } else if pane == WorkspacePane::Queue {
                self.set_queue_panel_open(true);
            }
        }
        if workspace {
            // Leave space for a track row near the mini-player breakpoint.
            let min_height = PlayerLayout::Dense.height();
            let max_height = (min_height + size.y - PlayerLayout::MIN_WORKSPACE_HEIGHT)
                .min(PlayerLayout::Full.height());
            let height = (viewport.bottom() - transport_rect.top()).clamp(min_height, max_height);
            egui::Panel::bottom("transport")
                .exact_size(height)
                .frame(egui::Frame::new().fill(design::colors(root).background))
                .show_separator_line(false)
                .show(root, |_| {});
        }
        let show_navigation = workspace && !dock;
        let mut navigation_visible = show_navigation;
        egui::Panel::top("compact-navigation")
            .exact_size(44.)
            .frame(
                egui::Frame::new()
                    .fill(design::colors(root).panel)
                    .inner_margin(4.),
            )
            .show_collapsible(root, &mut navigation_visible, |ui| {
                if show_navigation {
                    self.pane_switcher(ui, header_visible);
                }
            });
        let queue = self.desktop_panes(root, size.x, workspace && dock, header_visible);

        let pane = workspace.then_some(if dock {
            WorkspacePane::Playlist
        } else {
            self.pane
        });
        let mut central = egui::CentralPanel::default();
        let narrow = layout_fraction(root, "workspace-narrow-inset", compact_heading);
        let shallow = layout_fraction(root, "workspace-shallow-inset", dense);
        if workspace {
            let inset = egui::lerp(24. ..=10., narrow).round() as i8;
            let vertical = egui::lerp(f32::from(inset)..=4., shallow)
                .min(4. + size.y - PlayerLayout::MIN_WORKSPACE_HEIGHT)
                .round() as i8;
            central = central.frame(
                egui::Frame::new()
                    .fill(design::colors(root).background)
                    .inner_margin(egui::Margin {
                        right: (inset - TRACK_SCROLL_GUTTER).max(4),
                        top: vertical,
                        bottom: vertical,
                        ..egui::Margin::same(inset)
                    }),
            );
        }
        central.show(root, |ui| {
            reveal_layout(
                ui,
                "workspace-layout",
                pane.map(|p| (dock, compact_heading, player, p)),
            );
            if dense {
                ui.spacing_mut().item_spacing.y = 4.;
            }
            match pane {
                Some(WorkspacePane::Library) => self.library_screen(ui, false),
                Some(WorkspacePane::Playlist) => self.playlist_screen(ui, compact_heading, dense),
                Some(WorkspacePane::Queue) => self.queue_screen(ui, false),
                None => {}
            }
        });
        if let Some(mut ui) = queue {
            self.queue_screen(&mut ui, true);
        }
        self.animated_transport(root, viewport, transport_rect, player, dock);
        self.settings_ui(&ctx);
        self.offline_confirmation_ui(&ctx);
        self.error_ui(&ctx);
        self.selection.finish(&ctx);
        self.poll_chat_search(&ctx);
        self.request_visible_media(&ctx);
    }

    fn animated_transport(
        &mut self,
        root: &mut egui::Ui,
        viewport: egui::Rect,
        rect: egui::Rect,
        player: PlayerLayout,
        dock: bool,
    ) {
        let seek_active = matches!(player, PlayerLayout::Mini | PlayerLayout::Full);
        let seek = layout_fraction(root, "transport-seek", seek_active);
        let clip = viewport.intersect(rect.with_max_y(rect.bottom().max(rect.top() + 84. * seek)));
        root.painter()
            .rect_filled(clip, 0., design::colors(root).background);
        if player.has_workspace() {
            let stroke = root.visuals().widgets.noninteractive.bg_stroke;
            root.painter()
                .hline(rect.x_range(), rect.top() + stroke.width / 2., stroke);
        }
        let mut ui = root.new_child(
            egui::UiBuilder::new()
                .id_salt("transport-controls")
                .max_rect(rect.shrink2(player.margin()))
                .sense(egui::Sense::CLICK)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        ui.set_clip_rect(clip);
        reveal_layout(&mut ui, "transport-layout", player.has_workspace());
        self.transport(&mut ui, player, dock, seek);
    }

    fn pane_switcher(&mut self, ui: &mut egui::Ui, header_visible: bool) {
        ui.horizontal(|ui| {
            if !header_visible {
                let (_, action) = application_menu(ui, true);
                self.apply_application_menu(ui.ctx(), action);
            }
            let p = design::colors(ui);
            egui::Frame::new()
                .corner_radius(10)
                .fill(p.background)
                .inner_margin(2)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.;
                    let width = ((ui.available_width() - 4.) / 3.).max(0.);
                    let marker = ui.painter().add(egui::Shape::Noop);
                    let mut tab_rects = [egui::Rect::NOTHING; 3];
                    ui.visuals_mut().selection.bg_fill = egui::Color32::TRANSPARENT;
                    for ((pane, label), rect) in [
                        (WorkspacePane::Library, tr("Library")),
                        (WorkspacePane::Playlist, tr("Tracks")),
                        (WorkspacePane::Queue, tr("Queue")),
                    ]
                    .into_iter()
                    .zip(&mut tab_rects)
                    {
                        let selected = self.pane == pane;
                        let response = ui.add_sized(
                            [width, 32.],
                            egui::Button::selectable(
                                selected,
                                egui::RichText::new(label).color(if selected {
                                    p.accent
                                } else {
                                    p.text
                                }),
                            )
                            .corner_radius(8)
                            .truncate(),
                        );
                        *rect = response.rect;
                        #[cfg(test)]
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(egui::Id::new(("test-pane", pane)), response.rect)
                        });
                        if response.clicked() {
                            self.pane = pane;
                        }
                    }
                    let tab = ui.ctx().animate_value_with_time(
                        ui.id().with("selected-tab"),
                        match self.pane {
                            WorkspacePane::Library => 0.,
                            WorkspacePane::Playlist => 1.,
                            WorkspacePane::Queue => 2.,
                        },
                        ui.style().animation_time,
                    );
                    let rect = tab_rects[tab.floor() as usize]
                        .lerp_towards(&tab_rects[tab.ceil() as usize], tab.fract());
                    ui.painter()
                        .set(marker, egui::Shape::rect_filled(rect, 8., p.selected));
                });
        });
    }

    fn desktop_panes(
        &mut self,
        root: &mut egui::Ui,
        width: f32,
        dock: bool,
        header_visible: bool,
    ) -> Option<egui::Ui> {
        const PLAYLIST_MIN_WIDTH: f32 = 400.;
        const QUEUE_MIN_WIDTH: f32 = 250.;
        let reserve = PLAYLIST_MIN_WIDTH + QUEUE_MIN_WIDTH;
        let palette = design::colors(root);
        let sidebar_max = (width - reserve).clamp(0., 400.);
        let mut library_visible = dock;
        egui::Panel::left("sidebar")
            .default_size(232.)
            .size_range(sidebar_max.min(180.)..=sidebar_max)
            .drag_to_open(false)
            .frame(egui::Frame::new().fill(palette.panel).inner_margin(16.))
            .show_collapsible(root, &mut library_visible, |ui| {
                if dock {
                    ui.take_available_width();
                    if !header_visible {
                        let (_, action) = application_menu(ui, true);
                        self.apply_application_menu(ui.ctx(), action);
                    }
                    self.library_screen(ui, true);
                }
            });
        let queue_max = (root.available_width() - PLAYLIST_MIN_WIDTH).clamp(0., 480.);
        let queue_open = dock && self.state.view().queue_panel_open;
        let mut queue_visible = queue_open;
        egui::Panel::right("queue")
            .default_size(290.)
            .size_range(queue_max.min(QUEUE_MIN_WIDTH)..=queue_max)
            .drag_to_open(false)
            .frame(
                egui::Frame::new()
                    .fill(palette.panel)
                    .inner_margin(egui::Margin {
                        right: 20 - TRACK_SCROLL_GUTTER,
                        ..egui::Margin::same(20)
                    }),
            )
            .show_collapsible(root, &mut queue_visible, |ui| {
                queue_open.then(|| {
                    // The panel reserves its space before the playlist registers its widgets.
                    let rect = ui.available_rect_before_wrap();
                    ui.take_available_space();
                    ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt("queue-content")
                            .max_rect(rect),
                    )
                })
            })
            .and_then(|panel| panel.inner)
    }

    fn library_screen(&mut self, ui: &mut egui::Ui, docked: bool) {
        let trigger = search_field(
            ui,
            &mut self.chat_query,
            "chat-search",
            tr("Find a chat"),
            Some(&mut self.chat_search_active),
        );
        let duration = if self.chat_search_active { 0.23 } else { 0.18 };
        let progress = change_progress(ui, "chat-list-mode", self.chat_search_active, duration);
        if docked {
            egui::Frame::NONE
                .inner_margin(egui::Margin {
                    right: -TRACK_SCROLL_GUTTER,
                    ..egui::Margin::ZERO
                })
                .show(ui, |ui| self.library_chats(ui, progress, trigger.as_ref()));
        } else {
            self.library_chats(ui, progress, trigger.as_ref());
        }
        if let Some(trigger) = trigger {
            focus_outline(ui, &trigger);
        }
    }

    fn library_chats(
        &mut self,
        ui: &mut egui::Ui,
        progress: f32,
        trigger: Option<&egui::Response>,
    ) {
        const ART_SIZE: f32 = 40.;
        const ROW_MARGIN: f32 = 8.;
        const ROW_HEIGHT: f32 = ART_SIZE + ROW_MARGIN * 2.;
        const ROW_GAP: f32 = 3.;
        const STAGGER: f32 = 0.45;
        let state = self.state.view();
        let query = self.chat_query.trim();
        let search = self.state.library().search();
        let mut indices = state.sidebar_chats(self.chat_search_active, query);
        let mut seen = HashSet::new();
        indices.retain(|&i| seen.insert(state.playlists[i].id));
        if self.chat_search_active && !query.is_empty() && search.query == query {
            indices.extend(state.playlists.iter().enumerate().filter_map(|(i, p)| {
                (p.in_chat_search() && search.ids.contains(&p.id) && seen.insert(p.id)).then_some(i)
            }));
            if search.loading() {
                ui.weak(tr("Searching chats…"));
            }
            if let Some(error) = &search.error {
                ui.label(error.to_string());
                if ui.button(tr("Retry search")).clicked() {
                    self.retry_chat_search();
                }
            }
        }
        if indices.is_empty() {
            let label = if self.chat_search_active {
                tr("No chats found")
            } else {
                tr("Pin chats here, or find one using search.")
            };
            if let Some(trigger) = trigger {
                self.keyboard.empty(ui, WorkspacePane::Library, trigger);
                ui.weak(label);
            } else {
                let response = empty_label(ui, label);
                self.keyboard.empty(ui, WorkspacePane::Library, &response);
                focus_outline(ui, &response);
            }
        }
        let mut recent = Vec::new();
        if self.chat_search_active && self.chat_query.trim().is_empty() {
            let state = self.state.view();
            if !state.recent_chats.is_empty() {
                let mut by_id: HashMap<_, _> = indices
                    .iter()
                    .map(|&i| (state.playlists[i].id, i))
                    .collect();
                recent.extend(state.recent_chats.iter().filter_map(|id| by_id.remove(id)));
                indices.retain(|&i| by_id.contains_key(&state.playlists[i].id));
            }
        }
        let recent_len = recent.len();
        recent.extend(indices);
        let mut open = None;
        let mut toggle = None;
        let mut recent_action = None;
        let chats_scroll = list_scroll_area(ui).id_salt("chat-list").show(ui, |ui| {
            if recent_len > 0 {
                ui.horizontal(|ui| {
                    ui.weak(tr("Recent"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = tr("Clear recent chats");
                        let icon = Icon::Trash
                            .image(design::colors(ui).muted, 14.)
                            .alt_text(&label);
                        let clear = ui
                            .add(
                                egui::Button::image(icon)
                                    .frame_when_inactive(false)
                                    .min_size(egui::vec2(28., 28.)),
                            )
                            .on_hover_text(label);
                        #[cfg(test)]
                        ui.data_mut(|data| {
                            data.insert_temp(egui::Id::new("test-clear-recent-chats"), clear.id)
                        });
                        if clear.clicked() {
                            recent_action = Some(AppCommand::ClearRecentChats);
                        }
                    });
                });
            }
            let clip = ui.clip_rect();
            let stride = ROW_HEIGHT + ROW_GAP + ui.spacing().item_spacing.y;
            let nav = ListNavigation::new(WorkspacePane::Library, recent.len(), stride);
            let key_at = |i: usize| egui::Id::new(self.state.view().playlists[recent[i]].id);
            self.keyboard.step(ui, nav, false, key_at, None);
            let first_visible =
                ((clip.top() - ui.next_widget_position().y) / stride).floor() as usize;
            let visible_slots = (clip.height() / stride).ceil().max(1.) as usize;
            for (row, i) in recent.into_iter().enumerate() {
                if recent_len > 0 && row == recent_len {
                    ui.separator();
                }
                let delay = STAGGER * row.saturating_sub(first_visible).min(visible_slots) as f32
                    / visible_slots as f32;
                let phase = ((progress - delay) / (1. - STAGGER)).clamp(0., 1.);
                let arrived = egui::emath::easing::cubic_out(phase);
                let origin = ui.next_widget_position()
                    + egui::vec2(ui.available_width() / 2., ROW_HEIGHT / 2.);
                let scale = egui::lerp(0.65..=1., arrived);
                let transform = egui::emath::TSTransform::new(
                    origin.to_vec2() * (1. - scale) + egui::vec2(0., -26. * (1. - arrived)),
                    scale,
                );
                let p = &self.state.view().playlists[i];
                let id = p.id;
                let pinned = self.state.view().chat_pins.contains(&id);
                let row_id = ui.id().with(("chat", id));
                let focused = ui.memory(|m| m.has_focus(row_id));
                let colors = design::colors(ui);
                let frame = egui::Frame::new()
                    .fill(if focused {
                        colors.raised
                    } else if self.opened_chat == Some(id) {
                        colors.selected
                    } else {
                        egui::Color32::TRANSPARENT
                    })
                    .corner_radius(8)
                    .inner_margin(ROW_MARGIN);
                let draw = |ui: &mut egui::Ui| {
                    frame
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                art(
                                    ui,
                                    &p.title,
                                    self.state.library().playlist_artwork(p.id).path.as_deref(),
                                    ART_SIZE,
                                );
                                chat_title(ui, &p.title, p.emoji_status.is_some(), 14., 1);
                                self.chat_emoji(ui, p.emoji_status, 18., None);
                            });
                        })
                        .response
                        .rect
                };
                let rect = if phase < 1. {
                    ui.with_visual_transform(transform, |ui| {
                        ui.set_clip_rect(transform.inverse() * clip);
                        ui.multiply_opacity((2. * phase).min(1.));
                        draw(ui)
                    })
                    .inner
                } else {
                    ui.scope(draw).inner
                };
                ui.add_space(ROW_GAP);
                if phase == 0. {
                    if self.keyboard.targets(nav.pane, egui::Id::new(id)) {
                        self.keyboard.reveal(ui, rect, nav, row);
                    }
                    continue;
                }
                let response = ui.interact(
                    transform * rect,
                    row_id,
                    keyboard::row_sense(ui, transform * rect),
                );
                if !ui.is_rect_visible(response.rect)
                    && self.keyboard.targets(nav.pane, egui::Id::new(id))
                {
                    self.keyboard.reveal(ui, rect, nav, row);
                }
                self.keyboard.row(
                    ui,
                    nav,
                    row,
                    egui::Id::new(id),
                    false,
                    (&response, response.rect),
                );
                if ui.is_rect_visible(response.rect) {
                    self.image_wanted
                        .push(self.state.library().playlist_artwork(p.id).file);
                }
                if response.clicked() {
                    open = Some(id);
                }
                response.context_menu(|ui| {
                    if ui
                        .button(if pinned {
                            tr("Unpin chat")
                        } else {
                            tr("Pin chat")
                        })
                        .clicked()
                    {
                        toggle = Some(id);
                        ui.close();
                    }
                    if row < recent_len && ui.button(tr("Remove from Recent")).clicked() {
                        recent_action = Some(AppCommand::RemoveRecentChat(id));
                        ui.close();
                    }
                });
            }
        });
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new("test-chat-scroll"),
                (chats_scroll.id, chats_scroll.inner_rect),
            )
        });
        design::middle_scroll(
            ui,
            chats_scroll.id,
            chats_scroll.inner_rect,
            chats_scroll.content_size.y,
        );
        if let Some(id) = toggle {
            self.toggle_chat_pin(id);
        }
        if let Some(command) = recent_action {
            self.execute(command);
        }
        if let Some(id) = open {
            self.execute(if self.chat_search_active {
                AppCommand::OpenSearchPlaylist(id)
            } else {
                AppCommand::OpenPlaylist(id)
            });
            self.pane = WorkspacePane::Playlist;
            ui.ctx().request_repaint();
            self.chat_search_active = false;
            self.chat_query.clear();
            ui.ctx()
                .memory_mut(|m| m.surrender_focus(egui::Id::new("chat-search")));
        }
    }
    fn error_ui(&mut self, ctx: &egui::Context) {
        if !self.error.is_empty() {
            let size = ctx.content_rect().size();
            let width = (size.x - 24.).clamp(100., 400.);
            let mut open = true;
            egui::Window::new(tr("Something went wrong"))
                .id(egui::Id::new("player-error"))
                .order(egui::Order::Foreground)
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .default_width(width.min(360.))
                .max_width(width)
                .max_height((size.y - 48.).max(24.))
                .vscroll(true)
                .show(ctx, |ui| {
                    ui.set_max_width(width);
                    ui.label(&self.error);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(tr("Copy error")).clicked() {
                            ui.ctx().copy_text(self.error.clone());
                        }
                        if self.error_retry.is_some() && ui.button(tr("Retry")).clicked() {
                            self.execute(AppCommand::Retry);
                        }
                        if ui.button(tr("Dismiss")).clicked() {
                            self.error.clear();
                        }
                    });
                });
            if !open {
                self.error.clear();
            }
        }
    }
    fn transport_buttons(
        &mut self,
        ui: &mut egui::Ui,
        center: egui::Pos2,
        shuffle: f32,
        repeat: f32,
    ) {
        let gap = ui.spacing().item_spacing.x;
        let half_width = 22. + 36. + gap;
        if shuffle > 0. {
            let point = center - egui::vec2(half_width + 36. + gap, 0.);
            let mut button = horizontal_slot(ui, "shuffle", point, egui::Vec2::splat(36.));
            button.multiply_opacity(shuffle);
            let shuffle_hint = if self.state.view().shuffle {
                tr("Shuffle on · Turn off")
            } else {
                tr("Shuffle off · Turn on")
            };
            if design::toggle(
                &mut button,
                Icon::Shuffle,
                self.state.view().shuffle,
                "",
                shuffle_hint,
            ) {
                self.execute(AppCommand::Shuffle(!self.state.view().shuffle));
            }
        }
        let mut playback = horizontal_slot(
            ui,
            "playback",
            center - egui::vec2(half_width, 0.),
            egui::vec2(2. * half_width, 44.),
        );
        if button(&mut playback, Icon::Previous, "Previous") {
            self.execute(AppCommand::Previous);
        }
        if design::playback_button(
            &mut playback,
            if self.state.playback().state().paused()
                || self.state.view().session.current().is_none()
            {
                Icon::Play
            } else {
                Icon::Pause
            },
            "Play / pause",
        ) {
            self.execute(AppCommand::Pause(!self.state.playback().state().paused()));
        }
        if button(&mut playback, Icon::Next, "Next") {
            self.execute(AppCommand::Next);
        }
        if repeat > 0. {
            let point = center + egui::vec2(half_width + gap, 0.);
            let mut button = horizontal_slot(ui, "repeat", point, egui::Vec2::splat(36.));
            button.multiply_opacity(repeat);
            let clip = button.clip_rect();
            button.set_clip_rect(clip.with_max_x(clip.right().min(point.x + (36. + gap) * repeat)));
            let hint = match self.state.view().repeat {
                RepeatMode::Off => tr("Repeat off · Repeat all"),
                RepeatMode::All => tr("Repeat all · Repeat one"),
                RepeatMode::One => tr("Repeat one · Turn off"),
            };
            if design::toggle(
                &mut button,
                Icon::Repeat,
                self.state.view().repeat != RepeatMode::Off,
                if self.state.view().repeat == RepeatMode::One {
                    "1"
                } else {
                    ""
                },
                hint,
            ) {
                self.execute(AppCommand::Repeat(self.state.view().repeat.next()));
            }
        }
    }

    fn queue_button(&mut self, ui: &mut egui::Ui) {
        let open = self.state.view().queue_panel_open;
        if design::toggle(
            ui,
            Icon::Queue,
            open,
            "",
            if open {
                tr("Queue open · Close queue")
            } else {
                tr("Queue closed · Open queue")
            },
        ) {
            self.set_queue_panel_open(!open);
            ui.ctx().request_repaint();
        }
    }

    fn mute_button(&mut self, ui: &mut egui::Ui) {
        let (icon, label) = if self.state.view().volume == 0. {
            (Icon::Muted, "Unmute")
        } else {
            (Icon::Volume, "Mute")
        };
        if button(ui, icon, label) {
            self.toggle_mute();
        }
    }

    fn volume_slider(&mut self, ui: &mut egui::Ui, width: f32) {
        let mut volume = self.state.view().volume as f64;
        let slider = design::slider(ui, &mut volume, 100., width, tr("Volume"), false);
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new("test-volume"), (slider.rect, slider.id));
        });
        if slider.changed() {
            self.execute(AppCommand::Volume(volume as f32));
        }
    }

    fn transport(&mut self, ui: &mut egui::Ui, player: PlayerLayout, dock: bool, seek: f32) {
        let area = ui.available_rect_before_wrap();
        let width = area.width();
        let gap = ui.spacing().item_spacing.x;
        let button_slot = 36. + gap;
        let basic_width = 44. + 2. * button_slot;
        let extended_width = basic_width + 2. * button_slot;
        let seek_active = matches!(player, PlayerLayout::Mini | PlayerLayout::Full);
        let expanded = player == PlayerLayout::Full && width >= 798.;
        let wide = layout_fraction(ui, "transport-wide", expanded);
        // Keep mute available after shuffle and repeat fold away.
        let mute = layout_fraction(ui, "transport-mute", width >= extended_width);
        let repeat = layout_fraction(
            ui,
            "transport-repeat",
            width >= extended_width + button_slot / 2.,
        );
        let shuffle = layout_fraction(
            ui,
            "transport-shuffle",
            width >= extended_width + button_slot,
        );
        let queue = layout_fraction(ui, "transport-queue", dock && player.has_workspace());
        let play_width = basic_width + button_slot * (shuffle + repeat);
        let compact_slider = (width - extended_width - button_slot * (1. + queue) - gap).min(104.);
        let slider = layout_fraction(ui, "transport-volume", expanded || compact_slider >= 72.);
        let compact_width =
            play_width + button_slot * (mute + queue) + (gap + compact_slider.max(72.)) * slider;
        let compact_left = (width - compact_width).max(0.) / 2.;
        let metadata_width = ((width - 48.) * 0.25).clamp(60., 300.);
        let wide_left = 24. + metadata_width + gap;
        let wide_center =
            (width - 48. - metadata_width - (144. + 56. * queue) - 24.).max(basic_width);
        let center_left = wide * wide_left;
        let center_width = egui::lerp(width..=wide_center, wide);
        let play_x = area.left()
            + egui::lerp(
                (compact_left + button_slot * shuffle + basic_width / 2.)
                    ..=(wide_left + wide_center / 2.),
                wide,
            )
            .clamp(
                basic_width / 2.,
                (width - basic_width / 2.).max(basic_width / 2.),
            );
        let play_y = (area.top() + 22. + 8. * seek).min(ui.clip_rect().bottom() - 22.);
        let right_x = area.left()
            + egui::lerp(
                (compact_left + play_width + gap)..=(wide_left + wide_center + gap),
                wide,
            );
        let volume_x = right_x + button_slot * queue;
        let volume_y = play_y + 12. * wide;

        let media = if seek_active || wide > 0. {
            self.state
                .view()
                .session
                .current()
                .map(|t| self.media_track(t.into()))
        } else {
            None
        };
        if let Some(track) = &media {
            self.image_wanted.push(track.image_file);
            if track.file == 0 {
                self.track_wanted.push(track.clone());
            }
        }
        if wide > 0. {
            let point = egui::pos2(area.left() + 24. - 14. * (1. - wide), play_y + 14.);
            let mut metadata =
                horizontal_slot(ui, "metadata", point, egui::vec2(metadata_width, 72.));
            metadata.multiply_opacity(wide);
            metadata.set_clip_rect(metadata.clip_rect().with_max_x(
                (play_x - basic_width / 2. - button_slot * shuffle - gap).max(area.left()),
            ));
            metadata.horizontal(|ui| {
                let track = media.as_ref();
                art(
                    ui,
                    track.map(|t| t.title.as_str()).unwrap_or("TG"),
                    track.and_then(|t| t.image_path.as_deref()),
                    48.,
                );
                ui.vertical(|ui| {
                    ui.set_width((metadata_width - 48. - gap).max(0.));
                    let title = track
                        .map(|t| t.title.clone())
                        .unwrap_or_else(|| tr("Choose your soundtrack"));
                    let artist = track
                        .map(|t| t.artist.clone())
                        .unwrap_or_else(|| tr("Select a track to get started"));
                    ui.add(
                        egui::Label::new(egui::RichText::new(title).strong())
                            .truncate()
                            .show_tooltip_when_elided(false),
                    );
                    ui.add(
                        egui::Label::new(egui::RichText::new(artist).weak())
                            .truncate()
                            .show_tooltip_when_elided(false),
                    );
                    if self.state.playback().state().loading() {
                        ui.small(tr("Loading track…"));
                    }
                });
            });
        }
        self.transport_buttons(ui, egui::pos2(play_x, play_y), shuffle, repeat);
        if queue > 0. {
            let point = egui::pos2(volume_x - button_slot, volume_y);
            let mut button = horizontal_slot(ui, "queue", point, egui::Vec2::splat(36.));
            button.multiply_opacity(queue);
            self.queue_button(&mut button);
        }
        if mute > 0. {
            let point = egui::pos2(volume_x, volume_y);
            let mut button = horizontal_slot(ui, "mute", point, egui::Vec2::splat(36.));
            button.multiply_opacity(mute);
            self.mute_button(&mut button);
        }
        if slider > 0. {
            let full_width = egui::lerp(compact_slider.max(72.)..=(96. + 8. * queue), wide);
            let slider_width = egui::lerp(28. ..=full_width, slider);
            let point = egui::pos2(
                volume_x + button_slot + (full_width - slider_width) / 2.,
                volume_y,
            );
            let mut volume = horizontal_slot(ui, "volume", point, egui::vec2(slider_width, 28.));
            volume.multiply_opacity(slider);
            self.volume_slider(&mut volume, slider_width);
        }
        if !seek_active {
            self.seek_preview = None;
        }
        if seek > 0. {
            let timeline_width = egui::lerp((center_width * 0.55).max(140.)..=center_width, seek);
            let point = egui::pos2(
                area.left() + center_left + (center_width - timeline_width) / 2.,
                play_y + egui::lerp(36. ..=40., seek),
            );
            let mut timeline =
                horizontal_slot(ui, "timeline", point, egui::vec2(timeline_width, 28.));
            timeline.multiply_opacity(seek);
            let ui = &mut timeline;
            let playback = self.state.playback().state();
            let snapshot = self
                .player
                .as_ref()
                .map(|player| player.snapshot())
                .unwrap_or_default();
            let current = self.state.view().session.current();
            let restored = self.restored_position(snapshot);
            let duration = if restored.is_some() {
                current.map_or(0., |track| track.duration as f64)
            } else if playback.loading() {
                0.
            } else if self.demo {
                current.as_ref().map_or(0., |track| track.duration as f64)
            } else {
                snapshot.duration
            };
            let has_duration = duration.is_finite() && duration > 0. && current.is_some();
            let enabled = seek_active && restored.is_none() && has_duration;
            let position = if let Some(position) = restored {
                position
            } else if has_duration {
                let sampled = if snapshot.seeking {
                    snapshot.position
                } else {
                    snapshot.audio_position.unwrap_or(snapshot.position)
                };
                sampled.max(0.).min(duration)
            } else {
                0.
            };
            let mut preview = self.seek_preview.unwrap_or(position);
            ui.horizontal(|ui| {
                ui.add_sized(
                    [42., 28.],
                    egui::Label::new(egui::RichText::new(clock(preview)).small()),
                );
                let response = ui
                    .add_enabled_ui(enabled, |ui| {
                        ui.push_id(("seek", self.state.playback().generation()), |ui| {
                            design::slider(
                                ui,
                                &mut preview,
                                duration.max(0.),
                                (timeline_width - 100.).max(40.),
                                tr("Seek"),
                                true,
                            )
                        })
                        .inner
                    })
                    .inner;
                if enabled
                    && self.player.is_some()
                    && !playback.paused()
                    && !snapshot.seeking
                    && !response.is_pointer_button_down_on()
                    && ui.is_rect_visible(response.rect)
                {
                    let rail_pixels =
                        design::slider_rail(response.rect).width() * ui.ctx().pixels_per_point();
                    design::repaint_after(ui, seek_repaint_interval(duration, rail_pixels));
                }
                #[cfg(test)]
                {
                    self.test_seek = Some((response.rect, response.id));
                }
                if enabled && (response.dragged() || response.is_pointer_button_down_on()) {
                    self.seek_preview = Some(preview);
                }
                if enabled
                    && (response.drag_stopped()
                        || response.clicked()
                        || (response.changed() && !response.is_pointer_button_down_on()))
                {
                    self.execute(AppCommand::Seek(preview));
                }
                ui.small(if has_duration {
                    clock(duration)
                } else {
                    "—:—".into()
                });
            });
        }
    }

    fn playlist_menu(&mut self, ui: &mut egui::Ui, selected: usize, chat_id: i64, url: &str) {
        if ui
            .add_enabled(!self.demo, egui::Button::new(tr("Open source chat")))
            .clicked()
        {
            self.open_source_chat(chat_id, url);
            ui.close();
        }
        if !url.is_empty() && ui.button(tr("Copy chat link")).clicked() {
            ui.ctx().copy_text(url.to_owned());
            ui.close();
        }
        if ui.button(tr("Refresh library")).clicked() {
            self.open_chat(selected);
            ui.close();
        }
    }

    fn sort_playlist(&mut self, ui: &mut egui::Ui) {
        let orders = [tr("Newest first"), tr("Title"), tr("Artist")];
        egui::ComboBox::from_id_salt("order")
            .selected_text(&orders[self.sort])
            .show_ui(ui, |ui| {
                for (i, label) in orders.iter().enumerate() {
                    ui.selectable_value(&mut self.sort, i, label);
                }
            });
    }

    fn playlist_pin_action(&mut self, ui: &mut egui::Ui, chat_id: i64) {
        let label = if self.state.view().chat_pins.contains(&chat_id) {
            tr("Unpin chat")
        } else {
            tr("Pin chat")
        };
        if ui.button(label).clicked() {
            self.toggle_chat_pin(chat_id);
            ui.close();
        }
    }

    fn playlist_heading(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        artwork: Option<&str>,
        emoji: Option<gramwave_application::model::EmojiStatus>,
        full: f32,
        visible: f32,
    ) {
        let width = ui.available_width() - f32::from(TRACK_SCROLL_GUTTER) * full;
        let padding = 16. * full;
        let full_cover = ui.ctx().animate_value_with_time(
            egui::Id::new("playlist-cover-size"),
            if ui.available_width() - f32::from(TRACK_SCROLL_GUTTER) - 32. < 650. {
                72.
            } else {
                112.
            },
            0.15,
        );
        let cover_size = egui::lerp(32. ..=full_cover, full);
        let font_size = egui::lerp(18. ..=30., full);
        let text_x = padding + cover_size + ui.spacing().item_spacing.x + 16. * full;
        let text_width = (width - text_x - padding).max(0.);
        let galley = chat_title_galley(
            ui,
            title,
            emoji.is_some(),
            font_size,
            if full > 0. { 2 } else { 1 },
            text_width,
        );
        let first_line = galley.rows.first().map_or(0., |row| row.size.y);
        let title_height = egui::lerp(first_line..=galley.size().y, full);
        let eyebrow = 22. * full;
        let inner_height = cover_size.max(title_height + eyebrow);
        let height = inner_height + padding * 2.;
        // A short workspace still needs room for its search field and a track row.
        let room = if visible < 1. {
            (ui.available_height() - design::ROW_HEIGHT - 60.).max(0.)
        } else {
            f32::INFINITY
        };
        let clip_height = (height * visible).min(room);
        if clip_height <= 0. {
            return;
        }
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(width, clip_height), egui::Sense::hover());
        let clip = ui.clip_rect().intersect(rect);
        ui.painter().with_clip_rect(clip).rect_filled(
            rect,
            12. * full,
            design::colors(ui).panel.gamma_multiply(full * visible),
        );
        let mid_y = rect.top() + padding + inner_height / 2.;
        let mut cover = horizontal_slot(
            ui,
            "playlist-cover",
            egui::pos2(rect.left() + padding, mid_y),
            egui::Vec2::splat(cover_size),
        );
        cover.set_clip_rect(clip);
        cover.multiply_opacity(visible);
        art(&mut cover, title, artwork, cover_size);
        let text_origin = egui::pos2(rect.left() + text_x, mid_y - (title_height + eyebrow) / 2.);
        if full > 0. {
            ui.painter().with_clip_rect(clip).text(
                text_origin,
                egui::Align2::LEFT_TOP,
                tr("TELEGRAM PLAYLIST"),
                egui::FontId::proportional(12.),
                design::colors(ui).muted.gamma_multiply(full * visible),
            );
        }
        let title_rect = egui::Rect::from_min_size(
            text_origin + egui::vec2(0., eyebrow),
            egui::vec2(text_width, title_height),
        );
        let mut label = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("playlist-title")
                .max_rect(title_rect),
        );
        label.set_clip_rect(clip);
        label.multiply_opacity(visible);
        let mut text = label.new_child(egui::UiBuilder::new().max_rect(title_rect));
        text.set_clip_rect(clip.with_max_y(clip.bottom().min(title_rect.bottom())));
        let end = show_chat_title(&mut text, galley);
        self.chat_emoji(
            &mut label,
            emoji,
            24.,
            Some(end + egui::vec2(ui.spacing().item_spacing.x, 0.)),
        );
    }

    fn playlist_screen(&mut self, ui: &mut egui::Ui, compact: bool, dense: bool) {
        if !self.demo && self.opened_chat.is_none() {
            if !compact {
                ui.add_space(60.);
            }
            let empty = empty_label(ui, egui::RichText::new(tr("Choose a chat")).heading());
            self.keyboard.empty(ui, WorkspacePane::Playlist, &empty);
            focus_outline(ui, &empty);
            ui.weak(tr("Music loads only when you open a chat."));
            return;
        }

        let Some(selected) = self
            .state
            .view()
            .playlists
            .iter()
            .position(|p| Some(p.id) == self.opened_chat.or(self.demo.then_some(0)))
        else {
            let empty = empty_label(ui, egui::RichText::new(tr("Choose a chat")).heading());
            self.keyboard.empty(ui, WorkspacePane::Playlist, &empty);
            focus_outline(ui, &empty);
            return;
        };
        let Some(p) = self.state.view().playlists.get(selected) else {
            ui.add_space(80.);
            ui.heading(tr("Your next favourite is waiting."));
            ui.weak(&self.status);
            return;
        };
        let chat_id = p.id;
        if !dense {
            self.image_wanted
                .push(self.state.library().playlist_artwork(p.id).file);
        }
        let title = p.title.clone();
        let artwork = self.state.library().playlist_artwork(p.id).path;
        let url = p.url.clone();
        let can_play = p
            .tracks
            .iter()
            .any(|track| p.filters[usize::from(track.kind)]);

        let emoji_status = p.emoji_status;
        let before_filters = p.filters;
        let has_saved = self
            .state
            .offline()
            .copies
            .values()
            .any(|copy| !copy.cached && copy.track.chat == chat_id);
        let fully_saved = has_saved && self.state.offline().playlist_complete(p);
        let can_download = before_filters.into_iter().any(|enabled| enabled);
        let mut remove_downloads = false;
        let mut filters = before_filters;
        let filter_labels = [tr("Music"), tr("Audio files"), tr("Voice messages")];
        let mut play_all = false;
        let mut pin_all = false;
        let full_heading = layout_fraction(ui, "playlist-heading-full", !compact);
        let show_heading = layout_fraction(ui, "playlist-heading-visible", !dense);
        if show_heading > 0. {
            self.playlist_heading(
                ui,
                &title,
                artwork.as_deref(),
                emoji_status,
                full_heading,
                show_heading,
            );
        }
        if full_heading > 0. && !dense {
            let gap = 18. * full_heading;
            let area = ui.available_rect_before_wrap();
            let rect = area.with_min_y(area.top() + gap);
            let mut actions = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("playlist-actions")
                    .max_rect(rect),
            );
            actions.multiply_opacity(full_heading);
            if full_heading < 1. {
                actions.set_clip_rect(
                    ui.clip_rect()
                        .intersect(rect.with_max_y(rect.top() + 44. * full_heading)),
                );
            }
            let row = actions
                .horizontal_wrapped(|ui| {
                    let play = ui.add_enabled_ui(can_play, |ui| {
                        design::playback_button(ui, Icon::Play, "Play all")
                    });
                    play_all = play.inner;
                    #[cfg(test)]
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(egui::Id::new("test-play-all"), play.response.rect)
                    });
                    if !fully_saved {
                        pin_all = ui
                            .add_enabled_ui(can_download, |ui| {
                                button(ui, Icon::Download, "Save playlist offline")
                            })
                            .inner;
                    }
                    if has_saved {
                        remove_downloads = button(ui, Icon::Trash, "Remove downloads");
                    }
                    self.sort_playlist(ui);
                    ui.menu_button("…", |ui| {
                        self.playlist_menu(ui, selected, chat_id, &url);
                        self.playlist_pin_action(ui, chat_id);
                    });
                })
                .response
                .rect;
            ui.add_space(gap + (row.height() + ui.spacing().item_spacing.y) * full_heading);
        }
        let inline_filters = !compact && ui.available_width() >= 620.;
        let inline = layout_fraction(ui, "playlist-filters-inline", inline_filters);
        let gap = ui.spacing().item_spacing.x;
        let widths = if inline == 0. {
            [40.; 3]
        } else {
            filter_labels.each_ref().map(|label| {
                let text = egui::WidgetText::from(label.as_str()).into_galley(
                    ui,
                    Some(egui::TextWrapMode::Extend),
                    f32::INFINITY,
                    egui::TextStyle::Button,
                );
                (text.size().x + 2. * ui.spacing().button_padding.x + 2.).ceil()
            })
        };
        let inline_width = widths.iter().sum::<f32>() + 2. * gap;
        let (toolbar, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.), egui::Sense::hover());
        let menu_x = toolbar.right() - 40.;
        let filter_left = toolbar.right() - egui::lerp(40. ..=inline_width, inline);
        let filter_left = filter_left.max(toolbar.left() + 40. + gap);
        let mut search = horizontal_slot(
            ui,
            "track-toolbar-search",
            toolbar.left_center(),
            egui::vec2(filter_left - toolbar.left() - gap, 40.),
        );
        let _ = search_field(
            &mut search,
            &mut self.search,
            "track-search",
            tr("Search this playlist"),
            None,
        );

        if inline > 0. {
            let mut slot_left = toolbar.right() - inline_width;
            for (index, ((label, filter), width)) in filter_labels
                .iter()
                .zip(&mut filters)
                .zip(widths)
                .enumerate()
            {
                let center = egui::lerp((menu_x + 20.)..=(slot_left + width / 2.), inline);
                let visible_width = egui::lerp(16. ..=width, inline);
                let mut slot = horizontal_slot(
                    ui,
                    ("inline-filter", index),
                    egui::pos2(center - visible_width / 2., toolbar.center().y),
                    egui::vec2(visible_width, 36.),
                );
                slot.set_clip_rect(ui.clip_rect().intersect(toolbar.with_min_x(filter_left)));
                slot.multiply_opacity(inline);
                if !inline_filters {
                    slot.visuals_mut().disabled_alpha = 1.;
                    slot.disable();
                }
                if slot
                    .add_sized(
                        [visible_width, 36.],
                        egui::Button::selectable(*filter, label.as_str()).truncate(),
                    )
                    .clicked()
                {
                    *filter = !*filter;
                }
                slot_left += width + gap;
            }
        }
        if inline < 1. {
            let mut menu = horizontal_slot(
                ui,
                "playlist-filter-menu",
                egui::pos2(menu_x, toolbar.center().y),
                egui::Vec2::splat(40.),
            );
            menu.multiply_opacity(1. - inline);
            if inline_filters {
                menu.visuals_mut().disabled_alpha = 1.;
                menu.disable();
            }
            let ui = &mut menu;
            let label = if compact {
                tr("Playlist options")
            } else {
                tr("Filters")
            };
            let p = design::colors(ui);
            egui::containers::menu::MenuButton::from_button(
                egui::Button::image(Icon::More.image(p.muted, 20.).alt_text(&label))
                    .min_size(egui::vec2(40., 40.)),
            )
            .ui(ui, |ui| {
                if compact {
                    play_all = ui
                        .add_enabled(can_play, egui::Button::new(tr("Play all")))
                        .clicked();
                    if !fully_saved {
                        pin_all = ui
                            .add_enabled(
                                can_download,
                                egui::Button::new(tr("Save playlist offline")),
                            )
                            .clicked();
                    }
                    if has_saved {
                        remove_downloads = ui.button(tr("Remove downloads")).clicked();
                    }
                    if play_all || pin_all || remove_downloads {
                        ui.close();
                    }
                    self.playlist_menu(ui, selected, chat_id, &url);
                    ui.separator();
                    self.sort_playlist(ui);
                }
                for (filter, label) in filters.iter_mut().zip(&filter_labels) {
                    ui.checkbox(filter, label.as_str());
                }
                if compact {
                    ui.separator();
                    self.playlist_pin_action(ui, chat_id);
                }
            })
            .0
            .on_hover_text(if dense {
                format!("{title}\n{label}")
            } else {
                label
            });
        }
        ui.add_space(egui::lerp(2. ..=12., full_heading));
        if before_filters != filters {
            self.execute(AppCommand::Filters(chat_id, filters));
            self.open_chat(selected);
        }
        let tracks = self.playlist_tracks(selected);
        if play_all {
            self.play_playlist_from(selected, None);
        }
        if remove_downloads {
            self.offline_confirm = Some(vec![chat_id]);
        }
        if pin_all {
            self.execute(AppCommand::SavePlaylist(
                chat_id,
                self.state.view().playlists[selected].filters,
            ));
        }
        let loading = self.state.library().indexing_chat() == Some(chat_id);
        let notice = (!self.settings_open)
            .then(|| self.inline_notice(ui.ctx(), false))
            .flatten();
        if full_heading > 0. || loading || notice.is_some() {
            // Reserve the status line in the full layout to avoid shifting the list while loading.
            let height = if loading || notice.is_some() {
                24.
            } else {
                24. * full_heading
            };
            ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
                if let Some(notice) = notice {
                    ui.weak(notice);
                } else if loading {
                    ui.small(tr("Loading music from this chat…"));
                }
            });
        }
        if tracks.is_empty() {
            if !compact {
                ui.add_space(24.);
            }
            let label = if !self.state.view().playlists[selected]
                .filters
                .iter()
                .any(|v| *v)
            {
                tr("All filters are off")
            } else if !self.search.is_empty() {
                tr("No matching tracks")
            } else if self.state.library().indexing_chat().is_some() {
                tr("Loading music…")
            } else {
                tr("No music in this chat yet")
            };
            let empty = empty_label(ui, egui::RichText::new(label).heading());
            self.keyboard.empty(ui, WorkspacePane::Playlist, &empty);
            focus_outline(ui, &empty);
            if ui.button(tr("Refresh library")).clicked() {
                self.open_chat(selected);
            }
        }
        let scope = selection::scope(chat_id, &self.search, self.sort, filters);
        let key_at = |i: usize| {
            let track = &tracks[i];
            egui::Id::new((chat_id, track.library_id, track.id()))
        };
        self.selection
            .present_playlist(ui.ctx(), scope, &tracks, key_at);
        let mut action = None;
        let mut batch = None;
        let mut list_frame = track_list_frame().begin(ui);
        let list_ui = &mut list_frame.content_ui;
        let scroll = list_scroll_area(list_ui)
            .id_salt(("playlist-tracks", chat_id))
            .show_rows(list_ui, design::ROW_HEIGHT, tracks.len(), |ui, range| {
                let stride = design::ROW_HEIGHT + ui.spacing().item_spacing.y;
                let nav = ListNavigation::new(WorkspacePane::Playlist, tracks.len(), stride);
                let height = (tracks.len() as f32 * stride - ui.spacing().item_spacing.y).max(0.);
                let list = egui::Rect::from_min_size(
                    ui.max_rect().left_top() - egui::vec2(0., range.start as f32 * stride),
                    egui::vec2(ui.available_width(), height),
                );
                let (focus, _) =
                    self.selection
                        .pointer(ui, scope, tracks.len(), key_at, list, stride);
                if let Some(i) = focus {
                    ui.memory_mut(|memory| memory.request_focus(ui.id().with(key_at(i))));
                }
                if let Some(i) =
                    self.keyboard
                        .step(ui, nav, true, key_at, Some((&mut self.selection, scope)))
                {
                    let rect = egui::Rect::from_min_size(
                        ui.max_rect().left_top()
                            + egui::vec2(0., (i as f32 - range.start as f32) * stride),
                        egui::vec2(ui.available_width(), design::ROW_HEIGHT),
                    );
                    if !ui.is_rect_visible(rect) {
                        self.keyboard.reveal(ui, rect, nav, i);
                    }
                }
                self.note_playlist_view(selected, tracks.clone(), range.clone());
                for i in range {
                    let mut view = self.media_track(tracks[i].metadata());
                    view.playlist_number = tracks[i].playlist_number;
                    let t = &view;
                    self.image_wanted.push(t.image_file);
                    if t.file == 0 {
                        self.track_wanted.push(t.clone());
                    }
                    let active = self
                        .state
                        .view()
                        .session
                        .current()
                        .as_ref()
                        .is_some_and(|c| c.id() == t.id());
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), design::ROW_HEIGHT),
                        egui::Sense::hover(),
                    );
                    // Register the row first; the later menu owns its independent hit target.
                    let response = ui.interact(
                        rect,
                        ui.id().with(key_at(i)),
                        if ui.is_rect_visible(rect) {
                            egui::Sense::click_and_drag()
                        } else {
                            egui::Sense::CLICK | egui::Sense::DRAG
                        },
                    );
                    let selected_row = self.selection.contains(scope, key_at(i));
                    self.keyboard
                        .row(ui, nav, i, key_at(i), false, (&response, rect));
                    let p = design::colors(ui);
                    let hover = ui.ctx().animate_bool_with_time(
                        response.id.with("hover"),
                        response.hovered(),
                        0.14,
                    );
                    let highlight = p
                        .track_fill(active, selected_row)
                        .or_else(|| response.has_focus().then_some(p.selected));
                    if highlight.is_some() || hover > 0. {
                        ui.painter().rect_filled(
                            rect,
                            8.,
                            highlight.unwrap_or_else(|| p.raised.gamma_multiply(hover)),
                        );
                    }
                    if active {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                rect.left_top() + egui::vec2(0., 12.),
                                egui::vec2(3., 36.),
                            ),
                            2.,
                            p.accent,
                        );
                    }
                    if selected_row {
                        selection::paint(ui, rect);
                    }
                    focus_outline(ui, &response);
                    let mut menu_clicked = false;
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .id_salt(key_at(i))
                            .max_rect(rect.shrink2(egui::vec2(TRACK_ROW_PADDING, 6.)))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| {
                            let indicator = if active {
                                Some(if self.state.playback().state().loading() {
                                    "…"
                                } else if self.state.playback().state().paused() {
                                    "Ⅱ"
                                } else {
                                    "▶"
                                })
                            } else {
                                None
                            }
                            .map_or_else(|| t.playlist_number.to_string(), str::to_owned);
                            ui.add_sized(
                                [22., 36.],
                                egui::Label::new(egui::RichText::new(indicator).color(if active {
                                    p.accent
                                } else {
                                    p.muted
                                })),
                            );
                            let cover_left = ui.next_widget_position().x;
                            art(ui, &t.title, t.image_path.as_deref(), TRACK_ARTWORK_SIZE);
                            let offline = if t.pinned.is_some() {
                                Some("✓")
                            } else if self
                                .state
                                .offline()
                                .pending
                                .values()
                                .any(|p| p.id() == t.id())
                            {
                                Some("↓")
                            } else if self
                                .state
                                .offline()
                                .errors()
                                .contains_key(&(t.chat, t.message))
                            {
                                Some("!")
                            } else {
                                None
                            };
                            if let Some(mark) = offline {
                                let pos = egui::pos2(cover_left + 40., rect.bottom() - 10.);
                                ui.painter().circle_filled(pos, 9., p.panel);
                                ui.painter().text(
                                    pos,
                                    egui::Align2::CENTER_CENTER,
                                    mark,
                                    egui::FontId::proportional(12.),
                                    p.accent,
                                );
                            }
                            track_labels(ui, &t.title, &t.artist, active);
                            ui.add_sized(
                                [44., 36.],
                                egui::Label::new(
                                    egui::RichText::new(if t.duration == 0 {
                                        "—:—".into()
                                    } else {
                                        clock(t.duration as f64)
                                    })
                                    .small(),
                                ),
                            );
                            ui.scope(|ui| {
                                ui.visuals_mut().widgets.inactive.weak_bg_fill =
                                    egui::Color32::TRANSPARENT;
                                if !response.hovered()
                                    && !response.has_focus()
                                    && !ui.memory(|m| m.has_focus(ui.next_auto_id()))
                                    && !self.keyboard.targets(nav.pane, key_at(i))
                                {
                                    ui.visuals_mut().widgets.inactive.fg_stroke.color =
                                        egui::Color32::TRANSPARENT;
                                    ui.visuals_mut().override_text_color = None;
                                }
                                let menu = ui.add(
                                    egui::Button::new("…")
                                        .min_size(egui::Vec2::splat(36.))
                                        .sense(if self.selection.active(scope) {
                                            egui::Sense::CLICK
                                        } else {
                                            keyboard::row_sense(ui, rect)
                                        }),
                                );
                                self.keyboard
                                    .row(ui, nav, i, key_at(i), true, (&menu, rect));
                                focus_outline(ui, &menu);
                                self.selection.start(
                                    &response,
                                    &menu,
                                    scope,
                                    (i, key_at(i)),
                                    self.keyboard.source_key(nav.pane),
                                    None,
                                );
                                let contents = |ui: &mut egui::Ui, items: &mut MenuItems| {
                                    ui.visuals_mut().override_text_color = Some(p.text);
                                    ui.visuals_mut().widgets.inactive.fg_stroke.color = p.text;
                                    if selected_row && self.selection.is_group() {
                                        if let Some(action) = playlist_selection_menu(ui, items) {
                                            batch = Some((action, response.id));
                                        }
                                        return;
                                    }
                                    for (kind, label) in [
                                        (0, tr("Play")),
                                        (1, tr("Play next")),
                                        (2, tr("Add to queue")),
                                        (
                                            3,
                                            if t.pinned.is_some() {
                                                tr("Remove offline download")
                                            } else {
                                                tr("Save offline")
                                            },
                                        ),
                                        (4, tr("Copy message link")),
                                        (5, tr("Copy title")),
                                    ] {
                                        let button = ui.button(label);
                                        items.push(ui, &button, None);
                                        #[cfg(test)]
                                        if kind == 0 {
                                            self.test_play_buttons.insert(t.message, button.rect);
                                        }
                                        if button.clicked() {
                                            action = Some((kind, i));
                                            ui.close();
                                        }
                                    }
                                    if offline.is_some() {
                                        ui.separator();
                                    }
                                    if t.pinned.is_some() {
                                        ui.label(tr("Saved offline"));
                                    } else if self
                                        .state
                                        .offline()
                                        .pending
                                        .values()
                                        .any(|p| p.id() == t.id())
                                    {
                                        ui.label(tr("Downloading…"));
                                    } else if let Some(error) =
                                        self.state.offline().errors().get(&(t.chat, t.message))
                                    {
                                        ui.label(tr("Download failed · Save offline to retry"));
                                        ui.label(error.to_string());
                                    }
                                    items.links(self.track_caption(ui, t.caption.as_ref()));
                                };
                                let (handled, popup, keep_click) = keyboard::show_track_menu(
                                    &response,
                                    &menu,
                                    selected_row,
                                    contents,
                                );
                                self.selection.remember_popup(popup, keep_click);
                                #[cfg(test)]
                                self.test_rows.insert(t.message, (rect, menu.rect));
                                #[cfg(test)]
                                self.test_row_ids.insert(t.message, response.id);
                                menu_clicked = handled;
                            });
                        },
                    );
                    if response.clicked() && !menu_clicked {
                        response.request_focus();
                    }
                    if !menu_clicked
                        && !self.selection.block_click()
                        && response.clicked()
                        && !response.double_clicked()
                    {
                        action = Some((6, i));
                    }
                }
            });
        list_frame.end(ui);
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new("test-playlist-scroll"), scroll.id);
            data.insert_temp(scroll.id.with("test-viewport"), scroll.inner_rect);
        });
        design::middle_scroll(ui, scroll.id, scroll.inner_rect, scroll.content_size.y);
        let more = self.state.view().playlists[selected].has_more();
        if more && self.state.library().indexing_chat().is_none() && !self.demo {
            let at_bottom = scroll.state.offset.y > 0.
                && scroll.state.offset.y + scroll.inner_rect.height()
                    >= scroll.content_size.y - 120.;
            if ui.button(tr("Load more music")).clicked() || at_bottom {
                self.load_chat(selected, false);
            }
        }
        if let Some((batch, row)) = batch {
            ui.memory_mut(|memory| memory.request_focus(row));
            let mut selected_tracks: Vec<_> = tracks
                .iter()
                .enumerate()
                .filter(|(i, _)| self.selection.contains(scope, key_at(*i)))
                .map(|(_, track)| track.metadata())
                .collect();
            match batch {
                PlaylistSelectionAction::Insert { next, reverse } => {
                    if reverse {
                        selected_tracks.reverse();
                    }
                    self.execute(AppCommand::InsertTracks(selected_tracks, next));
                }
                PlaylistSelectionAction::SaveOffline => {
                    self.execute(AppCommand::SaveTracks(selected_tracks))
                }
            }
        }
        if let Some((kind, i)) = action {
            match kind {
                0 => self.play_playlist_from(selected, Some(tracks[i].library_id)),
                6 => self.execute(AppCommand::SelectPlaylistRow(i)),
                5 => ui.ctx().copy_text(tracks[i].title.clone()),
                1 => self.execute(AppCommand::Insert(tracks[i].metadata(), true)),
                2 => self.execute(AppCommand::Insert(tracks[i].metadata(), false)),
                3 => {
                    if let Some(path) = self.state.offline().path(tracks[i].id()) {
                        self.execute(AppCommand::RemoveOffline(tracks[i].id(), path.to_owned()));
                    } else {
                        self.execute(AppCommand::SaveTrack(tracks[i].metadata()));
                    }
                }
                _ => self.link(&tracks[i]),
            }
            self.persist();
        }
    }
}
enum PlaylistSelectionAction {
    Insert { next: bool, reverse: bool },
    SaveOffline,
}

fn playlist_selection_menu(
    ui: &mut egui::Ui,
    items: &mut MenuItems,
) -> Option<PlaylistSelectionAction> {
    ui.set_width(184.);
    let mut action = None;
    for (next, label, reverse_label) in [
        (true, tr("Play next"), tr("Play next in reverse order")),
        (
            false,
            tr("Add to queue"),
            tr("Add to queue in reverse order"),
        ),
    ] {
        ui.horizontal(|ui| {
            let forward = ui.button(label);
            let reverse = ui
                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = design::colors(ui).raised;
                    let button = ui.add(egui::Button::new("⇅").min_size(egui::Vec2::splat(36.)));
                    button.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            ui.is_enabled(),
                            &reverse_label,
                        )
                    });
                    button.on_hover_text(reverse_label)
                })
                .inner;
            items.push(ui, &forward, Some(&reverse));
            if forward.clicked() || reverse.clicked() {
                action = Some(PlaylistSelectionAction::Insert {
                    next,
                    reverse: reverse.clicked(),
                });
                ui.close();
            }
        });
    }
    let save = ui.button(tr("Save offline"));
    items.push(ui, &save, None);
    if save.clicked() {
        action = Some(PlaylistSelectionAction::SaveOffline);
        ui.close();
    }
    action
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplicationMenuAction {
    Settings,
    Quit,
}

fn application_menu(
    ui: &mut egui::Ui,
    settings_available: bool,
) -> (egui::Rect, Option<ApplicationMenuAction>) {
    let label = tr("Gramwave menu");
    let (response, menu) = ui
        .scope(|ui| {
            ui.spacing_mut().button_padding = egui::Vec2::splat(6.);
            let icon = egui::Image::from_bytes("bytes://gramwave-window.svg", design::APP_ICON)
                .fit_to_exact_size(egui::Vec2::splat(24.))
                .alt_text(&label);
            egui::containers::menu::MenuButton::from_button(
                egui::Button::image(icon).frame_when_inactive(false),
            )
            .ui(ui, |ui| {
                let action = if settings_available && ui.button(tr("Settings")).clicked() {
                    Some(ApplicationMenuAction::Settings)
                } else if ui.button(tr("Quit Gramwave")).clicked() {
                    Some(ApplicationMenuAction::Quit)
                } else {
                    None
                };
                if action.is_some() {
                    ui.close();
                }
                action
            })
        })
        .inner;
    (response.rect, menu.and_then(|menu| menu.inner))
}

fn list_scroll_area(ui: &mut egui::Ui) -> egui::ScrollArea {
    let scroll = &mut ui.spacing_mut().scroll;
    scroll.floating = false;
    scroll.bar_inner_margin = 4.;
    scroll.bar_width = f32::from(TRACK_SCROLL_GUTTER) - scroll.bar_inner_margin;
    scroll.bar_outer_margin = 0.;
    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .animated(false)
}

fn track_list_frame() -> egui::Frame {
    egui::Frame::NONE.inner_margin(egui::Margin {
        left: -8,
        right: -4,
        ..egui::Margin::ZERO
    })
}

fn chat_title(
    ui: &mut egui::Ui,
    title: &str,
    emoji: bool,
    size: f32,
    max_rows: usize,
) -> egui::Pos2 {
    let galley = chat_title_galley(ui, title, emoji, size, max_rows, ui.available_width());
    show_chat_title(ui, galley)
}

fn chat_title_galley(
    ui: &mut egui::Ui,
    title: &str,
    emoji: bool,
    size: f32,
    max_rows: usize,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let width = width
        - if emoji {
            size.max(24.) + ui.spacing().item_spacing.x
        } else {
            0.
        };
    let mut job = egui::text::LayoutJob::simple(
        title.to_owned(),
        egui::FontId::proportional(size),
        ui.visuals().text_color(),
        width.max(0.),
    );
    job.wrap.max_rows = max_rows;
    job.wrap.break_anywhere = max_rows == 1;
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

fn show_chat_title(ui: &mut egui::Ui, galley: std::sync::Arc<egui::Galley>) -> egui::Pos2 {
    let line_end = galley
        .rows
        .last()
        .map_or(egui::Pos2::ZERO, |row| row.rect().right_center());
    let response = ui.add(egui::Label::new(galley).show_tooltip_when_elided(false));
    response.rect.min + line_end.to_vec2()
}

fn drag_window_caption(ui: &mut egui::Ui, rect: egui::Rect) {
    let response = ui.interact(
        rect,
        ui.id().with("window-caption"),
        egui::Sense::CLICK | egui::Sense::DRAG,
    );
    let primary = egui::PointerButton::Primary;
    if response.double_clicked_by(primary) {
        let maximized = ui.input(|input| input.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    } else if response.drag_started_by(primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        // The compositor takes over the gesture and may keep the release event.
        ui.ctx().stop_dragging();
    }
}

fn window_header(
    root: &mut egui::Ui,
    title: Option<&str>,
    settings_available: bool,
) -> Option<ApplicationMenuAction> {
    let p = design::colors(root);
    egui::Panel::top("window-header")
        .exact_size(design::HEADER_HEIGHT)
        .show_separator_line(false)
        .frame(egui::Frame::new().fill(p.panel))
        .show(root, |ui| {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), design::HEADER_HEIGHT),
                egui::Sense::hover(),
            );
            let expanded = layout_fraction(ui, "window-header-expanded", rect.width() >= 300.);
            let controls_width = egui::lerp(48. ..=132., expanded);
            let caption = egui::Rect::from_min_max(
                rect.left_top() + egui::vec2(8., 6.),
                rect.right_bottom() - egui::vec2(controls_width, 6.),
            );

            let mut text_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(caption)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            let (menu_rect, action) = application_menu(&mut text_ui, settings_available);
            drag_window_caption(ui, caption.with_min_x(menu_rect.right() + 1.));
            if let Some(title) = title.filter(|_| expanded > 0.) {
                let suffix = egui::WidgetText::from(egui::RichText::new("— Gramwave").weak())
                    .into_galley(
                        &text_ui,
                        Some(egui::TextWrapMode::Extend),
                        f32::INFINITY,
                        egui::TextStyle::Body,
                    );
                let title_width = expanded
                    * (text_ui.available_width()
                        - suffix.size().x
                        - text_ui.spacing().item_spacing.x)
                        .max(0.);
                let track = egui::WidgetText::from(egui::RichText::new(title).strong())
                    .into_galley(
                        &text_ui,
                        Some(egui::TextWrapMode::Truncate),
                        title_width,
                        egui::TextStyle::Body,
                    );
                let opacity = text_ui.opacity();
                text_ui.multiply_opacity(expanded);
                text_ui.add(egui::Label::new(track).show_tooltip_when_elided(false));
                text_ui.set_opacity(opacity);
                text_ui.add(egui::Label::new(suffix));
            } else {
                text_ui.strong("Gramwave");
            }

            let controls_rect = egui::Rect::from_min_max(
                egui::pos2(caption.right(), rect.top()),
                rect.right_bottom() - egui::vec2(8., 0.),
            );
            let mut controls = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(controls_rect)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
            );
            controls.set_clip_rect(controls.clip_rect().intersect(controls_rect));
            controls.spacing_mut().item_spacing.x = 4.;
            let maximized = controls.input(|i| i.viewport().maximized.unwrap_or(false));
            let (icon, label) = if maximized {
                (Icon::Restore, "Restore window")
            } else {
                (Icon::Maximize, "Maximize window")
            };
            let commands = [
                (Icon::Close, "Close window", egui::ViewportCommand::Close),
                (icon, label, egui::ViewportCommand::Maximized(!maximized)),
                (
                    Icon::Minimize,
                    "Minimize window",
                    egui::ViewportCommand::Minimized(true),
                ),
            ];
            for (index, (icon, label, command)) in commands.into_iter().enumerate() {
                if index > 0 {
                    if expanded == 0. {
                        break;
                    }
                    controls.set_opacity(ui.opacity() * expanded);
                }
                if button(&mut controls, icon, label) {
                    ui.ctx().send_viewport_cmd(command);
                }
            }
            action
        })
        .inner
}

fn track_labels(ui: &mut egui::Ui, title: &str, artist: &str, active: bool) -> egui::Response {
    ui.allocate_ui(
        egui::vec2(
            (ui.available_width() - 44. - 36. - 2. * ui.spacing().item_spacing.x).max(0.),
            40.,
        ),
        |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.;
            ui.vertical(|ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(title).color(if active {
                        design::colors(ui).accent
                    } else {
                        ui.visuals().text_color()
                    }))
                    .truncate()
                    .show_tooltip_when_elided(false),
                );
                ui.add(
                    egui::Label::new(egui::RichText::new(artist).weak())
                        .truncate()
                        .show_tooltip_when_elided(false),
                );
            });
        },
    )
    .response
}

fn seek_repaint_interval(duration: f64, rail_pixels: f32) -> Duration {
    let seconds_per_quarter_pixel = 0.25 * duration.max(1.) / f64::from(rail_pixels);
    Duration::from_secs_f64(seconds_per_quarter_pixel.clamp(1. / 60., 0.2))
}

fn clock(seconds: f64) -> String {
    let n = seconds.max(0.) as u64;
    format!("{}:{:02}", n / 60, n % 60)
}
fn cover(ui: &mut egui::Ui, title: &str, size: f32) {
    let seed = title.bytes().fold(0u8, u8::wrapping_add);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let color = egui::Color32::from_rgb(35 + seed % 65, 55 + seed % 85, 65 + seed % 100);
    ui.painter().rect_filled(rect, 6., color);
    ui.painter().circle_stroke(
        rect.center(),
        size * 0.30,
        egui::Stroke::new(size * 0.1, egui::Color32::from_white_alpha(25)),
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        title
            .chars()
            .next()
            .unwrap_or('T')
            .to_uppercase()
            .to_string(),
        egui::FontId::proportional(size * 0.4),
        egui::Color32::WHITE,
    );
}

pub(crate) const TRACK_ARTWORK_SIZE: f32 = 44.;

pub(crate) fn artwork_image(path: &str, size: f32) -> egui::Image<'static> {
    egui::Image::new(format!("file://{path}"))
        .fit_to_exact_size(egui::Vec2::splat(size))
        .corner_radius(6)
}

fn art(ui: &mut egui::Ui, title: &str, path: Option<&str>, size: f32) {
    if let Some(path) = path {
        let image = artwork_image(path, size);
        if matches!(
            image.load_for_size(ui.ctx(), egui::vec2(size, size)),
            Ok(egui::load::TexturePoll::Ready { .. })
        ) {
            ui.add(image);
            return;
        }
    }
    cover(ui, title, size);
}

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
    key: egui::Id,
    row_id: impl Fn(usize) -> egui::Id,
    allow_start: bool,
) -> (Option<LiftedQueueRow>, Option<AppCommand>) {
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
        ctx.animate_value_with_time(id.with("y"), (rect.top() - list.top()) / stride, 0.);
        ctx.animate_value_with_time(id.with("x"), rect.left() - list.left(), 0.);
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

fn queue_cover_rect(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(rect.left() + TRACK_ROW_PADDING, rect.center().y - 22.),
        egui::Vec2::splat(44.),
    )
}

fn queue_row_contents(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    track: &crate::Track,
    past: bool,
    current: bool,
) -> egui::Response {
    let inset = rect.shrink2(egui::vec2(TRACK_ROW_PADDING, 6.));
    let center_y = inset.center().y;
    let cover = queue_cover_rect(rect);
    let cover_pos = cover.min;
    let mut cover_ui = ui.new_child(egui::UiBuilder::new().max_rect(cover));
    if past {
        cover_ui.multiply_opacity(0.62);
    }
    art(
        &mut cover_ui,
        &track.title,
        track.image_path.as_deref(),
        44.,
    );
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
        egui::Button::image(
            Icon::More
                .image(ui.visuals().text_color(), 18.)
                .alt_text(&label),
        )
        .frame_when_inactive(false)
        .sense(egui::Sense::click_and_drag()),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
    });
    response
}

impl App {
    fn queue_screen(&mut self, ui: &mut egui::Ui, extend_rows: bool) {
        let empty = self.state.view().session.timeline().is_empty();
        let has_current = self.state.view().session.current().is_some();
        let preview_history =
            self.demo && self.options.scenario.as_deref() == Some("queue-history");
        let (scroll_key, mut follow, opened) =
            queue_scroll::QueueScroll::load(ui.ctx(), preview_history);
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
        #[cfg(test)]
        self.test_queue_rows.clear();
        let mut list_frame = if extend_rows {
            track_list_frame()
        } else {
            egui::Frame::NONE
        }
        .begin(ui);
        let list_ui = &mut list_frame.content_ui;
        let queue = self.state.view().session.timeline();
        let spacing = list_ui.spacing().item_spacing.y;
        let stride = design::ROW_HEIGHT + spacing;
        let rows_height = (stride * queue.len() as f32 - spacing).max(0.);
        let anchor = queue
            .current_index()
            .or_else(|| (queue.upcoming_start() < queue.len()).then_some(queue.upcoming_start()))
            .map(|i| (queue.get(i).unwrap().entry_id, i));
        let target = anchor.map_or_else(
            || (rows_height - list_ui.available_height().max(64.)).max(0.),
            |(_, i)| i as f32 * stride,
        );
        let area_id = list_ui.make_persistent_id("queue-list");
        let middle = design::middle_scroll_active(list_ui.ctx(), area_id);
        let (offset, busy) = follow.start(list_ui, stride, target, anchor, opened, middle);
        if follow.following {
            list_ui.spacing_mut().scroll.fade.strength = 0.;
        }
        let padded_to = if follow.following {
            target
        } else {
            target.max(offset.min(rows_height - design::ROW_HEIGHT))
        };
        let mut selection_scrolled = false;
        let queue_scroll = list_scroll_area(list_ui)
            .id_salt("queue-list")
            .vertical_scroll_offset(offset)
            .show_viewport(list_ui, |ui, viewport| {
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
                let (focus, scrolling) =
                    self.selection
                        .pointer(ui, scope, queue.len(), key_at, rows, stride);
                selection_scrolled = scrolling;
                if let Some(i) = focus {
                    ui.memory_mut(|memory| memory.request_focus(key_at(i)));
                }
                if let Some(i) =
                    self.keyboard
                        .step(ui, nav, true, key_at, Some((&mut self.selection, scope)))
                {
                    let rect = queue_row_rect(list, stride, i as f32);
                    if !ui.is_rect_visible(rect) {
                        self.keyboard.reveal(ui, rect, nav, i);
                    }
                }
                let allow_drag = !self.selection.tracking(scope) && !self.selection.block_click();
                let (lifted, moved) = queue_drag(ui, list, stride, queue, key, key_at, allow_drag);
                reordering = lifted.is_some();
                action = moved;
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
                    let y =
                        ui.ctx()
                            .animate_value_with_time(id.with("y"), slot as f32, animation_time);
                    let mut rect = queue_row_rect(list, stride, y);
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let x = ui
                        .ctx()
                        .animate_value_with_time(id.with("x"), 0., animation_time);
                    rect = rect.translate(egui::vec2(x, 0.));
                    let track = self.media_track(entry.into());
                    self.image_wanted.push(track.image_file);
                    if track.file == 0 {
                        self.track_wanted.push(track.clone());
                    }
                    let row = ui.interact(rect, id, egui::Sense::click_and_drag());
                    self.keyboard.row(ui, nav, i, id, false, (&row, rect));
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
                    let menu = queue_row_contents(&mut row_ui, rect, &track, past, current);
                    self.keyboard.row(ui, nav, i, id, true, (&menu, rect));
                    self.selection.start(
                        &row,
                        &menu,
                        scope,
                        (i, id),
                        self.keyboard.source_key(nav.pane),
                        Some(queue_cover_rect(rect)),
                    );
                    if selected_row {
                        selection::paint(ui, rect);
                    }
                    focus_outline(&row_ui, &row);
                    focus_outline(&row_ui, &menu);
                    #[cfg(test)]
                    self.test_queue_rows
                        .insert(entry.entry_id, (rect, menu.rect, row.id));
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
                                    let button = ui.button(label);
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
                            (
                                i >= queue.upcoming_start(),
                                tr("Remove"),
                                AppCommand::Remove(i.saturating_sub(queue.upcoming_start())),
                            ),
                        ] {
                            if available {
                                let button = ui.button(label);
                                items.push(ui, &button, None);
                                if button.clicked() {
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
                        lifted.from < queue.past_len(),
                        current,
                    );
                }
                viewport.min.y
            });
        list_frame.end(ui);
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new("test-queue-scroll"), queue_scroll.id);
            data.insert_temp(
                queue_scroll.id.with("test-viewport"),
                queue_scroll.inner_rect,
            );
        });
        design::middle_scroll(
            ui,
            queue_scroll.id,
            queue_scroll.inner_rect,
            queue_scroll.content_size.y,
        );
        let middle = design::middle_scroll_active(ui.ctx(), queue_scroll.id);
        follow.finish(ui, &queue_scroll, target, busy, reordering, middle);
        if selection_scrolled {
            follow.browse();
        }
        ui.ctx()
            .data_mut(|data| data.insert_temp(scroll_key, follow));
        if let Some(action) = action {
            let persist = !matches!(action, AppCommand::PlayQueueRow { .. });
            self.execute(action);
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
            if persist {
                self.persist();
            }
        }
    }
}

#[cfg(test)]
mod tests;
