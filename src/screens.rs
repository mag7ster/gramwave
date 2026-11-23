use crate::application::commands::AppCommand;
use crate::{design, tr, App, RepeatMode};
use design::{button, Icon};
use eframe::egui;
use std::time::Duration;
use tg_player_domain::{playback::QueueView, QueueEntryId};

const TRACK_SCROLL_GUTTER: i8 = 12;
const TRACK_ROW_PADDING: f32 = 4.;

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub(crate) enum WorkspacePane {
    Library,
    #[default]
    Playlist,
    Queue,
}

fn search_field(
    ui: &mut egui::Ui,
    query: &mut String,
    id: &'static str,
    hint: String,
    mut active: Option<&mut bool>,
) {
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
    if let Some(active) = active.as_deref_mut() {
        *active |= field.clicked() || edit.clicked() || edit.gained_focus();
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
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if close.clicked()
            || (escape && (active.is_some() || edit.has_focus() || edit.lost_focus()))
        {
            if let Some(active) = active {
                *active = false;
            }
            query.clear();
            edit.surrender_focus();
        }
    }
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
        design::style(&ctx, self.state.view().light);
        root.set_style(ctx.global_style());
        if !self.settings_open
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
                    != tg_player_application::telegram::AuthorizationState::Ready);
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
        let controls_only = size.y < 150. || size.x < 300.;
        let player_only = controls_only || size.y < 260.;
        let dense = size.y < 350.;

        if player_only {
            egui::CentralPanel::default().show(root, |ui| {
                if controls_only {
                    self.seek_preview = None;
                    let rect = ui.available_rect_before_wrap();
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(egui::Rect::from_center_size(
                                rect.center(),
                                egui::vec2(132., 44.),
                            ))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| self.transport_buttons(ui, false),
                    );
                } else {
                    ui.add_space((ui.available_height() - 80.).max(0.) / 2.);
                    self.transport(ui, false, true);
                }
            });
        } else {
            let transport = egui::Panel::bottom("transport");
            if dense {
                transport
                    .exact_size(52.)
                    .frame(
                        egui::Frame::new()
                            .fill(design::colors(root).background)
                            .inner_margin(4.),
                    )
                    .show(root, |ui| {
                        self.seek_preview = None;
                        self.compact_transport_controls(ui, dock);
                    });
            } else {
                transport
                    .exact_size(96.)
                    .show(root, |ui| self.transport(ui, dock, false));
            }
            if dock {
                self.desktop_panes(root, size.x, header_visible);
            } else {
                egui::Panel::top("compact-navigation")
                    .frame(
                        egui::Frame::new()
                            .fill(design::colors(root).panel)
                            .inner_margin(4.),
                    )
                    .show(root, |ui| self.pane_switcher(ui, header_visible));
            }

            let inset = if compact_heading { 10 } else { 24 };
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(design::colors(root).background)
                        .inner_margin(egui::Margin {
                            right: (inset - TRACK_SCROLL_GUTTER).max(4),
                            top: if dense { 4 } else { inset },
                            bottom: if dense { 4 } else { inset },
                            ..egui::Margin::same(inset)
                        }),
                )
                .show(root, |ui| {
                    if dense {
                        ui.spacing_mut().item_spacing.y = 4.;
                    }
                    if dock {
                        self.playlist_screen(ui, compact_heading, dense);
                    } else {
                        match self.pane {
                            WorkspacePane::Library => self.library_screen(ui),
                            WorkspacePane::Playlist => {
                                self.playlist_screen(ui, compact_heading, dense)
                            }
                            WorkspacePane::Queue => self.queue_screen(ui, false),
                        }
                    }
                });
        }
        self.settings_ui(&ctx);
        self.error_ui(&ctx);
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
                    for (pane, label) in [
                        (WorkspacePane::Library, tr("Library")),
                        (WorkspacePane::Playlist, tr("Tracks")),
                        (WorkspacePane::Queue, tr("Queue")),
                    ] {
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
                        #[cfg(test)]
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(egui::Id::new(("test-pane", pane)), response.rect)
                        });
                        if response.clicked() {
                            self.pane = pane;
                        }
                    }
                });
        });
    }

    fn desktop_panes(&mut self, root: &mut egui::Ui, width: f32, header_visible: bool) {
        const PLAYLIST_MIN_WIDTH: f32 = 400.;
        const QUEUE_MIN_WIDTH: f32 = 250.;
        let reserve = PLAYLIST_MIN_WIDTH + QUEUE_MIN_WIDTH;
        let palette = design::colors(root);
        egui::Panel::left("sidebar")
            .default_size(232.)
            .size_range(180. ..=(width - reserve).min(400.))
            .frame(egui::Frame::new().fill(palette.panel).inner_margin(16.))
            .show(root, |ui| {
                ui.take_available_width();
                if !header_visible {
                    let (_, action) = application_menu(ui, true);
                    self.apply_application_menu(ui.ctx(), action);
                }
                self.library_screen(ui);
            });
        if self.queue_open {
            let max_width =
                (root.available_width() - PLAYLIST_MIN_WIDTH).clamp(QUEUE_MIN_WIDTH, 480.);
            egui::Panel::right("queue")
                .default_size(290.)
                .size_range(QUEUE_MIN_WIDTH..=max_width)
                .frame(
                    egui::Frame::new()
                        .fill(palette.panel)
                        .inner_margin(egui::Margin {
                            right: 20 - TRACK_SCROLL_GUTTER,
                            ..egui::Margin::same(20)
                        }),
                )
                .show(root, |ui| {
                    ui.take_available_width();
                    self.queue_screen(ui, true);
                });
        }
    }

    fn library_screen(&mut self, ui: &mut egui::Ui) {
        search_field(
            ui,
            &mut self.chat_query,
            "chat-search",
            tr("Find a chat"),
            Some(&mut self.chat_search_active),
        );
        let mut indices = self
            .state
            .view()
            .sidebar_chats(self.chat_search_active, &self.chat_query);
        if self.chat_search_active {
            for (i, p) in self.state.view().playlists.iter().enumerate() {
                if self.state.library().search().ids.contains(&p.id) && !indices.contains(&i) {
                    indices.push(i);
                }
            }
            if self.state.library().search().loading() {
                ui.weak(tr("Searching chats…"));
            }
            if self.state.library().search().error.is_some() {
                ui.label(
                    self.state
                        .library()
                        .search()
                        .error
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                );
                if ui.button(tr("Retry search")).clicked() {
                    self.retry_chat_search();
                }
            }
        }
        if indices.is_empty() {
            ui.weak(if self.chat_search_active {
                tr("No chats found")
            } else {
                tr("Pin chats here, or find one using search.")
            });
        }
        let mut open = None;
        let mut toggle = None;
        let chats_scroll = egui::ScrollArea::vertical()
            .id_salt("chat-list")
            .show(ui, |ui| {
                for i in indices {
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
                        .inner_margin(8.)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                art(
                                    ui,
                                    &p.title,
                                    self.state.library().playlist_artwork(p.id).path.as_deref(),
                                    40.,
                                );
                                chat_title(ui, &p.title, p.emoji_status.is_some(), 14., 1);
                                self.chat_emoji(ui, p.emoji_status, 18., None);
                            });
                        })
                        .response;
                    let response = ui.interact(frame.rect, row_id, egui::Sense::click());
                    if ui.is_rect_visible(response.rect) {
                        self.image_wanted
                            .push(self.state.library().playlist_artwork(p.id).file);
                    }
                    if response.clicked() {
                        open = Some(i);
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
                    });
                    ui.add_space(3.);
                }
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
        if let Some(i) = open {
            self.open_chat(i);
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
    fn transport_buttons(&mut self, ui: &mut egui::Ui, extended: bool) {
        if extended {
            let shuffle_hint = if self.state.view().shuffle {
                tr("Shuffle on · Turn off")
            } else {
                tr("Shuffle off · Turn on")
            };
            if design::toggle(
                ui,
                Icon::Shuffle,
                self.state.view().shuffle,
                "",
                shuffle_hint,
            ) {
                self.execute(AppCommand::Shuffle(!self.state.view().shuffle));
            }
        }
        if button(ui, Icon::Previous, "Previous") {
            self.execute(AppCommand::Previous);
        }
        if design::playback_button(
            ui,
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
        if button(ui, Icon::Next, "Next") {
            self.execute(AppCommand::Next);
        }
        if extended {
            let hint = match self.state.view().repeat {
                RepeatMode::Off => tr("Repeat off · Repeat all"),
                RepeatMode::All => tr("Repeat all · Repeat one"),
                RepeatMode::One => tr("Repeat one · Turn off"),
            };
            if design::toggle(
                ui,
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
        if design::toggle(
            ui,
            Icon::Queue,
            self.queue_open,
            "",
            if self.queue_open {
                tr("Queue open · Close queue")
            } else {
                tr("Queue closed · Open queue")
            },
        ) {
            self.queue_open = !self.queue_open;
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

    fn compact_transport_controls(&mut self, ui: &mut egui::Ui, dock: bool) {
        let gap = ui.spacing().item_spacing.x;
        let buttons = 44. + 5. * (36. + gap) + if dock { 36. + gap } else { 0. };
        let available = ui.available_width();
        let slider_width = (available - buttons - gap).min(104.);
        let show_slider = slider_width >= 72.;
        let width = buttons + if show_slider { gap + slider_width } else { 0. };
        ui.horizontal(|ui| {
            ui.add_space((available - width).max(0.) / 2.);
            self.transport_buttons(ui, true);
            self.mute_button(ui);
            if show_slider {
                self.volume_slider(ui, slider_width);
            }
            if dock {
                self.queue_button(ui);
            }
        });
    }

    fn transport(&mut self, ui: &mut egui::Ui, dock: bool, player_only: bool) {
        let current = self
            .state
            .view()
            .session
            .current()
            .map(|t| self.media_track(t.into()));
        if let Some(t) = &current {
            self.image_wanted.push(t.image_file);
            if t.file == 0 {
                self.track_wanted.push(t.clone());
            }
        }
        ui.spacing_mut().item_spacing.y = 4.;
        ui.add_space(8.);
        let compact = player_only || ui.available_width() < 798.;
        let width = ui.available_width() - if compact { 0. } else { 48. };
        let metadata_width = if compact {
            0.
        } else {
            (width * 0.25).min(300.)
        };
        let right_width = if dock { 200. } else { 144. };
        let center_width = if compact {
            width
        } else {
            width - metadata_width - right_width - 24.
        };
        ui.horizontal(|ui| {
            if !compact {
                ui.add_space(24.);
                ui.allocate_ui(egui::vec2(metadata_width, 72.), |ui| {
                    ui.set_min_width(metadata_width);
                    ui.horizontal(|ui| {
                        let t = current.as_ref();
                        art(
                            ui,
                            t.map(|t| t.title.as_str()).unwrap_or("TG"),
                            t.and_then(|t| t.image_path.as_deref()),
                            48.,
                        );
                        ui.allocate_ui(egui::vec2((metadata_width - 60.).max(60.), 56.), |ui| {
                            ui.vertical(|ui| {
                                let title = t
                                    .map(|t| t.title.clone())
                                    .unwrap_or_else(|| tr("Choose your soundtrack"));
                                let artist = t
                                    .map(|t| t.artist.clone())
                                    .unwrap_or_else(|| tr("Select a track to get started"));
                                ui.add(
                                    egui::Label::new(egui::RichText::new(&title).strong())
                                        .truncate()
                                        .show_tooltip_when_elided(false),
                                );
                                ui.add(
                                    egui::Label::new(egui::RichText::new(&artist).weak())
                                        .truncate()
                                        .show_tooltip_when_elided(false),
                                );
                                if self.state.playback().state().loading() {
                                    ui.small(tr("Loading track…"));
                                }
                            });
                        });
                    });
                });
            }
            ui.allocate_ui(egui::vec2(center_width, 72.), |ui| {
                ui.set_min_width(center_width);
                ui.vertical_centered(|ui| {
                    if compact {
                        self.compact_transport_controls(ui, false);
                    } else {
                        ui.horizontal(|ui| {
                            ui.add_space((center_width - 220.).max(0.) / 2.);
                            self.transport_buttons(ui, true);
                        });
                    }
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
                    let enabled = restored.is_none() && has_duration;
                    if enabled && self.player.is_some() && !playback.paused() {
                        ui.ctx().request_repaint_after(Duration::from_millis(16));
                    }
                    let position = if let Some(position) = restored {
                        position
                    } else if enabled {
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
                                        (center_width - 100.).max(40.),
                                        tr("Seek"),
                                        true,
                                    )
                                })
                                .inner
                            })
                            .inner;
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
                });
            });
            if !compact {
                ui.allocate_ui(egui::vec2(right_width, 72.), |ui| {
                    ui.set_min_width(right_width);
                    ui.add_space(16.);
                    ui.horizontal(|ui| {
                        if dock {
                            self.queue_button(ui);
                        }
                        self.mute_button(ui);
                        self.volume_slider(
                            ui,
                            (right_width - if dock { 96. } else { 48. }).min(104.),
                        );
                    });
                });
            }
        });
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

    fn playlist_screen(&mut self, ui: &mut egui::Ui, compact: bool, dense: bool) {
        if !self.demo && self.opened_chat.is_none() {
            if !compact {
                ui.add_space(60.);
            }
            ui.heading(tr("Choose a chat"));
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
        let mut filters = before_filters;
        let filter_labels = [tr("Music"), tr("Audio files"), tr("Voice messages")];
        let mut play_all = false;
        let mut pin_all = false;
        if compact && !dense {
            ui.horizontal(|ui| {
                art(ui, &title, artwork.as_deref(), 32.);
                chat_title(ui, &title, emoji_status.is_some(), 18., 1);
                self.chat_emoji(ui, emoji_status, 24., None);
            });
        } else if !compact {
            egui::Frame::new()
                .fill(design::colors(ui).panel)
                .corner_radius(12)
                .inner_margin(16.)
                .outer_margin(egui::Margin {
                    right: TRACK_SCROLL_GUTTER,
                    ..egui::Margin::ZERO
                })
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        art(
                            ui,
                            &title,
                            artwork.as_deref(),
                            if ui.available_width() < 650. {
                                72.
                            } else {
                                112.
                            },
                        );
                        ui.add_space(16.);
                        ui.vertical(|ui| {
                            ui.add_space(4.);
                            ui.small(tr("TELEGRAM PLAYLIST"));
                            ui.horizontal(|ui| {
                                let line_end =
                                    chat_title(ui, &title, emoji_status.is_some(), 30., 2);
                                let anchor = line_end + egui::vec2(ui.spacing().item_spacing.x, 0.);
                                self.chat_emoji(ui, emoji_status, 24., Some(anchor));
                            });
                        });
                    });
                });
            ui.add_space(18.);
            ui.horizontal_wrapped(|ui| {
                let play = ui.add_enabled_ui(can_play, |ui| {
                    design::playback_button(ui, Icon::Play, "Play all")
                });
                play_all = play.inner;
                #[cfg(test)]
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new("test-play-all"), play.response.rect)
                });
                pin_all = button(ui, Icon::Download, "Save playlist offline");
                self.sort_playlist(ui);
                ui.menu_button("…", |ui| {
                    self.playlist_menu(ui, selected, chat_id, &url);
                    self.playlist_pin_action(ui, chat_id);
                });
            });
        }
        ui.horizontal(|ui| {
            let inline_filters = !compact && ui.available_width() >= 620.;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if inline_filters {
                    for (filter, label) in filters.iter_mut().zip(&filter_labels).rev() {
                        ui.toggle_value(filter, label.as_str());
                    }
                } else {
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
                            pin_all = ui.button(tr("Save playlist offline")).clicked();
                            if play_all || pin_all {
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
                search_field(
                    ui,
                    &mut self.search,
                    "track-search",
                    tr("Search this playlist"),
                    None,
                );
            });
        });
        ui.add_space(if compact { 2. } else { 12. });
        if before_filters != filters {
            self.execute(AppCommand::Filters(chat_id, filters));
            self.open_chat(selected);
        }
        let tracks = self.playlist_tracks(selected);
        if play_all {
            self.play_playlist_from(selected, None);
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
        if !compact || loading || notice.is_some() {
            // Reserve the status line in the full layout to avoid shifting the list while loading.
            ui.allocate_ui(egui::vec2(ui.available_width(), 24.), |ui| {
                if let Some(notice) = notice {
                    ui.weak(notice);
                } else if loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.small(tr("Loading music from this chat…"));
                    });
                }
            });
        }
        if tracks.is_empty() {
            if !compact {
                ui.add_space(24.);
            }
            ui.heading(
                if !self.state.view().playlists[selected]
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
                },
            );
            if ui.button(tr("Refresh library")).clicked() {
                self.open_chat(selected);
            }
        }
        let mut action = None;
        let mut list_frame = track_list_frame().begin(ui);
        let list_ui = &mut list_frame.content_ui;
        let scroll = track_scroll_area(list_ui)
            .id_salt(("playlist-tracks", chat_id))
            .show_rows(list_ui, design::ROW_HEIGHT, tracks.len(), |ui, range| {
                for i in range {
                    let t = &tracks[i];
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
                        ui.id().with(("track", t.chat, t.message)),
                        egui::Sense::click(),
                    );
                    let p = design::colors(ui);
                    let hover = ui.ctx().animate_bool_with_time(
                        response.id.with("hover"),
                        response.hovered(),
                        0.14,
                    );
                    if active || response.has_focus() || hover > 0. {
                        ui.painter().rect_filled(
                            rect,
                            8.,
                            if active || response.has_focus() {
                                p.selected
                            } else {
                                p.raised.gamma_multiply(hover)
                            },
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
                    let mut menu_clicked = false;
                    ui.scope_builder(
                        egui::UiBuilder::new()
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
                            art(ui, &t.title, t.image_path.as_deref(), 44.);
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
                                if !response.hovered() && !response.has_focus() {
                                    ui.visuals_mut().widgets.inactive.fg_stroke.color =
                                        egui::Color32::TRANSPARENT;
                                    ui.visuals_mut().override_text_color = None;
                                }
                                let mut contents = |ui: &mut egui::Ui| {
                                    ui.visuals_mut().override_text_color = Some(p.text);
                                    ui.visuals_mut().widgets.inactive.fg_stroke.color = p.text;
                                    for (kind, label) in [
                                        (0, tr("Play")),
                                        (1, tr("Play next")),
                                        (2, tr("Add to queue")),
                                        (3, tr("Save offline")),
                                        (4, tr("Copy message link")),
                                        (5, tr("Copy title")),
                                    ] {
                                        let button = ui.button(label);
                                        #[cfg(test)]
                                        if kind == 0 {
                                            self.test_play_buttons.insert(t.message, button.rect);
                                        }
                                        if button.clicked() {
                                            action = Some((kind, i));
                                            ui.close();
                                        }
                                    }
                                    ui.separator();
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
                                    self.track_caption(ui, t.caption.as_ref());
                                };
                                let (menu, popup) =
                                    egui::containers::menu::MenuButton::from_button(
                                        egui::Button::new("…").min_size(egui::Vec2::splat(36.)),
                                    )
                                    .ui(ui, &mut contents);
                                response.context_menu(&mut contents);
                                #[cfg(test)]
                                self.test_rows.insert(t.message, (rect, menu.rect));
                                #[cfg(test)]
                                self.test_row_ids.insert(t.message, response.id);
                                menu_clicked =
                                    menu.contains_pointer() || menu.has_focus() || popup.is_some();
                            });
                        },
                    );
                    if response.clicked() && !menu_clicked {
                        response.request_focus();
                    }
                    if !menu_clicked
                        && ((response.clicked() && !response.double_clicked())
                            || (response.has_focus()
                                && ui.input_mut(|i| {
                                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                                })))
                    {
                        action = Some((6, i));
                    }
                }
            });
        list_frame.end(ui);
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("test-playlist-scroll"), scroll.id));
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
        if let Some((kind, i)) = action {
            match kind {
                0 => self.play_playlist_from(selected, Some(tracks[i].library_id)),
                6 => self.execute(AppCommand::SelectPlaylistRow(i)),
                5 => ui.ctx().copy_text(tracks[i].title.clone()),
                1 => self.execute(AppCommand::Insert(tracks[i].metadata(), true)),
                2 => self.execute(AppCommand::Insert(tracks[i].metadata(), false)),
                3 => self.execute(AppCommand::SaveTrack(tracks[i].metadata())),
                _ => self.link(&tracks[i]),
            }
            self.persist();
        }
    }
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

fn track_scroll_area(ui: &mut egui::Ui) -> egui::ScrollArea {
    let scroll = &mut ui.spacing_mut().scroll;
    scroll.floating = false;
    scroll.bar_inner_margin = 4.;
    scroll.bar_width = f32::from(TRACK_SCROLL_GUTTER) - scroll.bar_inner_margin;
    scroll.bar_outer_margin = 0.;
    egui::ScrollArea::vertical().auto_shrink([false, true])
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
    let width = ui.available_width()
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
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
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
        egui::Sense::click_and_drag(),
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
            let compact = rect.width() < 300.;
            let controls_width = if compact { 48. } else { 132. };
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
            if let Some(title) = title.filter(|_| !compact) {
                let suffix = egui::WidgetText::from(egui::RichText::new("— Gramwave").weak())
                    .into_galley(
                        &text_ui,
                        Some(egui::TextWrapMode::Extend),
                        f32::INFINITY,
                        egui::TextStyle::Body,
                    );
                let title_width = (text_ui.available_width()
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
                text_ui.add(egui::Label::new(track).show_tooltip_when_elided(false));
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
            let count = if compact { 1 } else { commands.len() };
            for (icon, label, command) in commands.into_iter().take(count) {
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

fn art(ui: &mut egui::Ui, title: &str, path: Option<&str>, size: f32) {
    if let Some(path) = path {
        let image = egui::Image::new(format!("file://{path}"))
            .fit_to_exact_size(egui::vec2(size, size))
            .corner_radius(6);
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
    entry: QueueEntryId,
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
    queue: QueueView<'_>,
    key: egui::Id,
) -> (Option<LiftedQueueRow>, Option<AppCommand>) {
    let ctx = ui.ctx();
    let mut drag = ctx.data(|data| data.get_temp::<QueueDrag>(key));
    let (down, released, cancelled, origin, pointer, dt) = ui.input(|i| {
        (
            i.pointer.primary_down(),
            i.pointer.primary_released(),
            !i.focused || i.key_pressed(egui::Key::Escape),
            i.pointer.press_origin(),
            i.pointer.interact_pos(),
            i.stable_dt.min(0.05),
        )
    });
    if drag.is_none() && down {
        drag = ctx.drag_started_id().zip(origin).and_then(|(id, origin)| {
            let (slot, track) = queue
                .iter()
                .enumerate()
                .find(|(_, track)| key.with(track.entry_id) == id)?;
            let response = ctx.read_response(id)?;
            Some(QueueDrag {
                entry: track.entry_id,
                grab: origin - response.rect.min,
                slot,
            })
        });
    }
    let Some(mut drag) = drag else {
        return (None, None);
    };
    let id = key.with(drag.entry);
    let stop = || {
        ctx.data_mut(|data| data.remove::<QueueDrag>(key));
        if ctx.is_being_dragged(id) {
            ctx.stop_dragging();
        }
    };
    let Some((pointer, from)) = pointer.zip(queue.iter().position(|t| t.entry_id == drag.entry))
    else {
        stop();
        return (None, None);
    };
    let dragging = down && ctx.is_being_dragged(id);
    let dropping = released && ctx.drag_stopped_id() == Some(id);
    if !dragging && !dropping && !cancelled {
        stop();
        return (None, None);
    }
    let viewport = egui::Rect::from_x_y_ranges(list.x_range(), ui.clip_rect().y_range())
        .intersect(ui.clip_rect());
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
                Some(AppCommand::Move(from, slot + usize::from(slot > from))),
            );
        }
        return (None, None);
    }

    ctx.data_mut(|data| data.insert_temp(key, drag));
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    if inside {
        let edge = (viewport.height() / 3.).min(36.);
        let top = (viewport.top() + edge - pointer.y).max(0.);
        let bottom = (pointer.y - viewport.bottom() + edge).max(0.);
        let speed = (bottom - top) * 18.;
        if (speed < 0. && list.top() < viewport.top())
            || (speed > 0. && list.bottom() > viewport.bottom())
        {
            ui.scroll_with_delta_animation(
                egui::vec2(0., -speed * dt),
                egui::style::ScrollAnimation::none(),
            );
            ctx.request_repaint();
        }
    }
    (Some(LiftedQueueRow { from, slot, rect }), None)
}

fn queue_row_contents(ui: &mut egui::Ui, rect: egui::Rect, track: &crate::Track) -> egui::Response {
    let inset = rect.shrink2(egui::vec2(TRACK_ROW_PADDING, 6.));
    let center_y = inset.center().y;
    let cover_pos = egui::pos2(inset.left(), center_y - 22.);
    let mut cover_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_size(cover_pos, egui::Vec2::splat(44.))),
    );
    art(
        &mut cover_ui,
        &track.title,
        track.image_path.as_deref(),
        44.,
    );

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
        labels.add(
            egui::Label::new(&track.title)
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
        ui.horizontal(|ui| {
            ui.heading(tr("Up next"));
            if extend_rows {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if button(ui, Icon::Close, "Close queue") {
                        self.queue_open = false;
                    }
                });
            }
        });
        ui.weak(if self.state.view().shuffle {
            tr("Shuffled order")
        } else {
            tr("Playlist order")
        });
        if self.state.view().session.remaining().is_empty() {
            ui.label(tr("Your queue is empty"));
        }
        let mut action = None;
        #[cfg(test)]
        self.test_queue_rows.clear();
        let mut list_frame = if extend_rows {
            track_list_frame()
        } else {
            egui::Frame::NONE
        }
        .begin(ui);
        let list_ui = &mut list_frame.content_ui;
        let queue_scroll = track_scroll_area(list_ui)
            .id_salt("queue-list")
            .show(list_ui, |ui| {
                let queue = self.state.view().session.remaining();
                let spacing = ui.spacing().item_spacing.y;
                let stride = design::ROW_HEIGHT + spacing;
                let (list, _) = ui.allocate_exact_size(
                    egui::vec2(
                        ui.available_width(),
                        (stride * queue.len() as f32 - spacing).max(0.),
                    ),
                    egui::Sense::hover(),
                );
                let key = ui.id().with("reorder");
                let (lifted, moved) = queue_drag(ui, list, stride, queue, key);
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
                    let entry = &queue[i];
                    let mut slot = i;
                    if let Some(lifted) = &lifted {
                        if i == lifted.from {
                            continue;
                        }
                        slot -= usize::from(i > lifted.from);
                        slot += usize::from(slot >= lifted.slot);
                    }
                    let id = key.with(entry.entry_id);
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
                    row.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            ui.is_enabled(),
                            &track.title,
                        )
                    });
                    if lifted.is_none() && row.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let fill = if row.has_focus() {
                        palette.selected
                    } else if row.hovered() {
                        palette.raised
                    } else {
                        palette.panel
                    };
                    ui.painter().rect_filled(rect, 8., fill);
                    let mut row_ui =
                        ui.new_child(egui::UiBuilder::new().id(id.with("body")).max_rect(rect));
                    let menu = queue_row_contents(&mut row_ui, rect, &track);
                    #[cfg(test)]
                    self.test_queue_rows
                        .insert(entry.entry_id, (rect, menu.rect, row.id));
                    if lifted.is_none() && !menu.clicked() && row.clicked() {
                        action = Some(AppCommand::PlayQueue(entry.entry_id));
                    }
                    let mut contents = |ui: &mut egui::Ui| {
                        if i > 0 && ui.button(tr("Move up")).clicked() {
                            action = Some(AppCommand::Move(i, i - 1));
                            ui.close();
                        }
                        if i + 1 < queue.len() && ui.button(tr("Move down")).clicked() {
                            action = Some(AppCommand::Move(i, i + 2));
                            ui.close();
                        }
                        if ui.button(tr("Remove")).clicked() {
                            action = Some(AppCommand::Remove(i));
                            ui.close();
                        }
                    };
                    egui::Popup::menu(&menu).show(&mut contents);
                    row.context_menu(&mut contents);
                }
                if let Some(lifted) = lifted {
                    let track = self.media_track((&queue[lifted.from]).into());
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
                    queue_row_contents(&mut overlay, lifted.rect, &track);
                }
            });
        list_frame.end(ui);
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("test-queue-scroll"), queue_scroll.id));
        design::middle_scroll(
            ui,
            queue_scroll.id,
            queue_scroll.inner_rect,
            queue_scroll.content_size.y,
        );
        if let Some(action) = action {
            let persist = !matches!(action, AppCommand::PlayQueue(_));
            self.execute(action);
            if persist {
                self.persist();
            }
        }
    }
}

#[cfg(test)]
#[path = "screens/tests.rs"]
mod tests;
