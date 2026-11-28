use crate::application::commands::AppCommand;
use crate::identity::APP_NAME;
use crate::{design, tr, App};
use design::{button, Icon};
use eframe::egui;

mod keyboard;
mod library;
mod playlist;
mod queue;
mod queue_notices;
mod queue_scroll;
mod scroll_anchor;
mod selection;
mod transport;
pub(crate) use keyboard::Keyboard;
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
    #[cfg(test)]
    ui.data_mut(|data| data.insert_temp(egui::Id::new(("test-search-rect", id)), rect));
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
        self.clear_hover_observation();
        let track_menu_open = keyboard::prepare_track_menu(&ctx);
        design::style(&ctx, self.state.view().light);
        root.set_style(ctx.global_style());
        let filter_focused = ctx
            .data_mut(|data| {
                data.remove_temp::<Option<egui::Id>>(egui::Id::new(playlist::FILTER_FOCUS))
            })
            .flatten()
            .is_some_and(|id| ctx.memory(|memory| memory.has_focus(id)));
        if !track_menu_open
            && !self.settings_open
            && !filter_focused
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
                        ui.heading(egui::RichText::new(APP_NAME).size(32.));
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
                        bottom: if pane == Some(WorkspacePane::Library) {
                            vertical
                        } else {
                            0
                        },
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
                        bottom: 0,
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
}

fn track_more_button(
    button: egui::Button<'_>,
    neutral: egui::Color32,
    highlighted_row: bool,
) -> egui::Button<'_> {
    if highlighted_row {
        button.fill(neutral)
    } else {
        button
    }
}

fn full_menu_button(label: String, width: f32) -> egui::Button<'static> {
    egui::Button::new(label)
        .right_text("")
        .min_size(egui::vec2(width, 0.))
        .frame_when_inactive(false)
}

fn track_menu_button(ui: &mut egui::Ui, label: String) -> egui::Response {
    ui.add(full_menu_button(label, ui.available_width()))
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
    let label = tr("{app} menu").replace("{app}", APP_NAME);
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
                } else if ui
                    .button(tr("Quit {app}").replace("{app}", APP_NAME))
                    .clicked()
                {
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
                let suffix =
                    egui::WidgetText::from(egui::RichText::new(format!("— {APP_NAME}")).weak())
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
                text_ui.strong(APP_NAME);
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

fn clock(seconds: f64, hours: bool) -> String {
    let n = seconds.max(0.) as u64;
    if hours || n >= 3600 {
        format!("{}:{:02}:{:02}", n / 3600, n / 60 % 60, n % 60)
    } else {
        format!("{}:{:02}", n / 60, n % 60)
    }
}

fn clock_width(ui: &egui::Ui, text: String) -> f32 {
    ui.painter()
        .layout_no_wrap(
            text,
            egui::TextStyle::Small.resolve(ui.style()),
            ui.visuals().text_color(),
        )
        .size()
        .x
        .ceil()
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

fn art(ui: &mut egui::Ui, title: &str, texture: Option<egui::load::SizedTexture>, size: f32) {
    if let Some(texture) = texture {
        ui.add(
            egui::Image::from_texture(texture)
                .fit_to_exact_size(egui::Vec2::splat(size))
                .corner_radius(6),
        );
    } else {
        cover(ui, title, size);
    }
}

#[cfg(test)]
mod tests;
