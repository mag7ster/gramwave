use crate::application::commands::AppCommand;
use crate::{design, tr, App, RepeatMode};
use design::{button, Icon};
use eframe::egui;
use std::time::Duration;
impl App {
    pub(crate) fn render(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        self.image_wanted.clear();
        self.track_wanted.clear();
        design::style(&ctx, self.state.view().light);
        self.poll_chat_search(&ctx);
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
        if self.options.preview_auth.is_some()
            || (!self.demo
                && *self.state.auth().state()
                    != tg_player_application::telegram::AuthorizationState::Ready)
        {
            egui::CentralPanel::default().show(root, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(50.);
                    ui.vertical_centered(|ui| {
                        ui.heading(egui::RichText::new("TG Player").size(32.));
                        ui.weak(tr("Your Telegram. Your soundtrack."));
                    });
                    ui.add_space(28.);
                    ui.horizontal(|ui| {
                        ui.add_space(((ui.available_width() - 440.) / 2.).max(0.));
                        ui.allocate_ui_with_layout(
                            egui::vec2(440., 0.),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                egui::Frame::new()
                                    .fill(design::palette(self.state.view().light).panel)
                                    .corner_radius(16)
                                    .inner_margin(28.)
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
        egui::Panel::bottom("transport")
            .exact_size(96.)
            .show(root, |ui| {
                self.transport(ui);
            });
        if ctx.content_rect().width() < 750. {
            egui::Panel::top("compact-library").show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.menu_button(tr("Your library"), |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.chat_query)
                                .hint_text(tr("Find a chat")),
                        );
                        let mut open = None;
                        for i in self.state.view().sidebar_chats(true, &self.chat_query) {
                            if ui.button(&self.state.view().playlists[i].title).clicked() {
                                open = Some(i);
                            }
                        }
                        if let Some(i) = open {
                            self.open_chat(i);
                            ui.close();
                        }
                    });
                    if button(ui, Icon::Settings, "Settings") {
                        self.settings_open = true;
                    }
                    ui.menu_button("…", |ui| {
                        if ui.button(tr("Quit TG Player")).clicked() {
                            self.quitting = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
            });
        } else {
            egui::Panel::left("sidebar")
                .exact_size(if ctx.content_rect().width() < 1000. {
                    200.
                } else {
                    232.
                })
                .frame(
                    egui::Frame::new()
                        .fill(design::palette(self.state.view().light).panel)
                        .inner_margin(16.),
                )
                .show(root, |ui| {
                    ui.add_space(8.);
                    ui.heading(egui::RichText::new("TG Player").size(23.).strong());
                    ui.add_space(16.);
                    ui.horizontal(|ui| {
                        ui.strong(tr("Your library"));
                        ui.menu_button("…", |ui| {
                            if ui.button(tr("Settings")).clicked() {
                                self.settings_open = true;
                                ui.close();
                            }
                            if ui.button(tr("Quit TG Player")).clicked() {
                                self.quitting = true;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                    });
                    ui.add_space(12.);
                    ui.horizontal(|ui| {
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.chat_query)
                                .id(egui::Id::new("chat-search"))
                                .hint_text(tr("Find a chat"))
                                .desired_width(ui.available_width() - 28.),
                        );
                        if response.clicked() || response.gained_focus() {
                            self.chat_search_active = true;
                        }
                        if self.chat_search_active
                            && (button(ui, Icon::Close, "Close search")
                                || ui.input(|i| i.key_pressed(egui::Key::Escape)))
                        {
                            self.chat_search_active = false;
                            self.chat_query.clear();
                            response.surrender_focus();
                        }
                    });
                    let mut indices = self
                        .state
                        .view()
                        .sidebar_chats(self.chat_search_active, &self.chat_query);
                    if self.chat_search_active {
                        for (i, p) in self.state.view().playlists.iter().enumerate() {
                            if self.state.library().search().ids.contains(&p.id)
                                && !indices.contains(&i)
                            {
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
                    let chats_scroll =
                        egui::ScrollArea::vertical()
                            .id_salt("chat-list")
                            .show(ui, |ui| {
                                for i in indices {
                                    let p = &self.state.view().playlists[i];
                                    let id = p.id;
                                    let pinned = self.state.view().chat_pins.contains(&id);
                                    let frame = egui::Frame::new()
                                        .fill(if self.opened_chat == Some(id) {
                                            design::palette(self.state.view().light).selected
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
                                                    self.state
                                                        .library()
                                                        .playlist_artwork(p.id)
                                                        .path
                                                        .as_deref(),
                                                    40.,
                                                );
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(&p.title).strong(),
                                                    )
                                                    .truncate(),
                                                );
                                            });
                                        })
                                        .response;
                                    let response = ui.interact(
                                        frame.rect,
                                        ui.id().with(("chat", id)),
                                        egui::Sense::click(),
                                    );
                                    if response.has_focus() {
                                        ui.painter().rect_stroke(
                                            response.rect,
                                            8.,
                                            egui::Stroke::new(1., design::colors(ui).accent),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
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
                        self.chat_search_active = false;
                        self.chat_query.clear();
                        ui.ctx()
                            .memory_mut(|m| m.surrender_focus(egui::Id::new("chat-search")));
                    }
                });
        }
        if self.queue_open {
            if root.available_width() < 800. {
                let mut open = true;
                egui::Window::new(tr("Queue"))
                    .open(&mut open)
                    .default_width(290.)
                    .max_height((ctx.content_rect().height() - 150.).max(100.))
                    .title_bar(false)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::RIGHT_TOP, [-16., 16.])
                    .show(&ctx, |ui| self.queue_screen(ui));
                if !open {
                    self.queue_open = false;
                }
            } else {
                egui::Panel::right("queue")
                    .default_size(290.)
                    .frame(
                        egui::Frame::new()
                            .fill(design::palette(self.state.view().light).panel)
                            .inner_margin(20.),
                    )
                    .show(root, |ui| self.queue_screen(ui));
            }
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(design::palette(self.state.view().light).background)
                    .inner_margin(24.),
            )
            .show(root, |ui| {
                if ctx.content_rect().height() < 500. {
                    egui::ScrollArea::vertical()
                        .id_salt("compact-content")
                        .show(ui, |ui| self.playlist_screen(ui));
                } else {
                    self.playlist_screen(ui);
                }
            });
        self.settings_ui(&ctx);
        self.error_ui(&ctx);
        if self.status != self.notice {
            self.notice = self.status.clone();
            self.notice_until = Some(std::time::Instant::now() + Duration::from_secs(5));
        }
        if self
            .notice_until
            .is_some_and(|t| t > std::time::Instant::now())
            && !self.notice.is_empty()
            && !self.settings_open
        {
            egui::Area::new(egui::Id::new("notification"))
                .anchor(egui::Align2::RIGHT_BOTTOM, [-20., -116.])
                .order(egui::Order::Foreground)
                .show(&ctx, |ui| {
                    egui::Frame::new()
                        .fill(design::palette(self.state.view().light).raised)
                        .corner_radius(8)
                        .inner_margin(14.)
                        .show(ui, |ui| {
                            ui.set_max_width(420.);
                            ui.label(&self.notice);
                        });
                });
        }
    }
    fn error_ui(&mut self, ctx: &egui::Context) {
        if !self.error.is_empty() {
            egui::Window::new(tr("Something went wrong"))
                .id(egui::Id::new("player-error"))
                .order(egui::Order::Foreground)
                .collapsible(false)
                .resizable(false)
                .default_width(360.)
                .show(ctx, |ui| {
                    ui.set_max_width(400.);
                    ui.label(&self.error);
                    ui.horizontal(|ui| {
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
        }
    }
    fn transport(&mut self, ui: &mut egui::Ui) {
        if let Some(t) = self
            .state
            .view()
            .session
            .current()
            .map(|t| self.media_track(t.into()))
        {
            self.image_wanted.push(t.image_file);
        }
        ui.spacing_mut().item_spacing.y = 4.;
        ui.add_space(8.);
        let width = ui.available_width() - 48.;
        let compact = width < 750.;
        let metadata_width = if compact {
            0.
        } else {
            (width * 0.25).min(300.)
        };
        let right_width = if compact {
            88.
        } else if width < 950. {
            176.
        } else {
            200.
        };
        let center_width = width - metadata_width - right_width - 24.;
        ui.horizontal(|ui| {
            ui.add_space(24.);
            if !compact {
                ui.allocate_ui(egui::vec2(metadata_width, 72.), |ui| {
                    ui.set_min_width(metadata_width);
                    ui.horizontal(|ui| {
                        let current = self
                            .state
                            .view()
                            .session
                            .current()
                            .map(|t| self.media_track(t.into()));
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
                                        .truncate(),
                                )
                                .on_hover_text(title);
                                ui.add(
                                    egui::Label::new(egui::RichText::new(&artist).weak())
                                        .truncate(),
                                )
                                .on_hover_text(artist);
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
                    ui.horizontal(|ui| {
                        ui.add_space((center_width - 212.).max(0.) / 2.);
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
                        if button(ui, Icon::Previous, "Previous") {
                            self.execute(AppCommand::Previous);
                        }
                        if button(
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
                            self.execute(AppCommand::Pause(
                                !self.state.playback().state().paused(),
                            ));
                        }
                        if button(ui, Icon::Next, "Next") {
                            self.execute(AppCommand::Next);
                        }
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
                    });
                    let duration = if self.state.playback().state().loading() {
                        0.
                    } else {
                        self.player
                            .as_ref()
                            .map(|p| p.snapshot().duration)
                            .unwrap_or(0.)
                    };
                    let duration = if self.demo {
                        self.state
                            .view()
                            .session
                            .current()
                            .as_ref()
                            .map(|t| t.duration as f64)
                            .unwrap_or(0.)
                    } else {
                        duration
                    };
                    let pos = if self.state.playback().state().loading() {
                        0.
                    } else {
                        self.player
                            .as_ref()
                            .map(|p| p.snapshot().position)
                            .unwrap_or(0.)
                    };
                    let mut preview = self.seek_preview.unwrap_or(pos);
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [42., 28.],
                            egui::Label::new(egui::RichText::new(clock(preview)).small()),
                        );
                        let enabled = duration.is_finite()
                            && duration > 0.
                            && self.state.view().session.current().is_some()
                            && !self.state.playback().state().loading();
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
                        ui.small(if enabled {
                            clock(duration)
                        } else {
                            "—:—".into()
                        });
                    });
                });
            });
            ui.allocate_ui(egui::vec2(right_width, 72.), |ui| {
                ui.set_min_width(right_width);
                ui.add_space(16.);
                ui.horizontal(|ui| {
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
                    if design::labelled_button(
                        ui,
                        if self.state.view().volume == 0. {
                            Icon::Muted
                        } else {
                            Icon::Volume
                        },
                        if self.state.view().volume == 0. {
                            tr("Unmute")
                        } else {
                            tr("Mute")
                        },
                    ) {
                        self.toggle_mute();
                    }
                    let mut volume = self.state.view().volume as f64;
                    if !compact
                        && design::slider(
                            ui,
                            &mut volume,
                            100.,
                            (right_width - 96.).min(104.),
                            tr("Volume"),
                            false,
                        )
                        .changed()
                    {
                        self.execute(AppCommand::Volume(volume as f32));
                    }
                });
            });
        });
    }
    fn playlist_screen(&mut self, ui: &mut egui::Ui) {
        if !self.demo && self.opened_chat.is_none() {
            ui.add_space(60.);
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
        self.image_wanted
            .push(self.state.library().playlist_artwork(p.id).file);
        let title = p.title.clone();
        let artwork = self.state.library().playlist_artwork(p.id).path;
        let url = p.url.clone();

        egui::Frame::new()
            .fill(design::palette(self.state.view().light).panel)
            .corner_radius(12)
            .inner_margin(16.)
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
                        let mut job = egui::text::LayoutJob::simple(
                            title.clone(),
                            egui::FontId::proportional(30.),
                            ui.visuals().text_color(),
                            ui.available_width(),
                        );
                        job.wrap.max_rows = 2;
                        job.wrap.break_anywhere = true;
                        ui.label(job).on_hover_text(&title);
                    });
                });
            });
        ui.add_space(18.);
        let mut toggle_pin = false;
        let mut play_all = false;
        let mut pin_all = false;
        ui.horizontal_wrapped(|ui| {
            play_all = ui
                .add(
                    egui::Button::image(Icon::Play.image(
                        if self.state.view().light {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::BLACK
                        },
                        22.,
                    ))
                    .fill(design::palette(self.state.view().light).accent)
                    .corner_radius(30)
                    .min_size(egui::vec2(44., 44.)),
                )
                .clicked();
            if ui
                .button(if self.state.view().chat_pins.contains(&chat_id) {
                    tr("Unpin chat")
                } else {
                    tr("Pin chat")
                })
                .clicked()
            {
                toggle_pin = true;
            }
            if button(ui, Icon::Download, "Save playlist offline") {
                pin_all = true;
            }
            ui.menu_button("…", |ui| {
                if !url.is_empty() {
                    ui.hyperlink_to(tr("Open source chat"), &url);
                    if ui.button(tr("Copy chat link")).clicked() {
                        ui.ctx().copy_text(url.clone());
                    }
                }
                if ui.button(tr("Refresh library")).clicked() {
                    self.open_chat(selected);
                    ui.close();
                }
            });
            ui.horizontal(|ui| {
                let orders = [tr("Newest first"), tr("Title"), tr("Artist")];
                egui::ComboBox::from_id_salt("order")
                    .selected_text(&orders[self.sort])
                    .show_ui(ui, |ui| {
                        for (i, label) in orders.iter().enumerate() {
                            ui.selectable_value(&mut self.sort, i, label);
                        }
                    });
            });
        });
        if toggle_pin {
            self.toggle_chat_pin(chat_id);
        }
        let before_filters = self.state.view().playlists[selected].filters;
        let mut filters = before_filters;
        ui.horizontal_wrapped(|ui| {
            ui.add(Icon::Search.image(egui::Color32::GRAY, 18.));
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text(tr("Search this playlist"))
                    .desired_width(200.),
            );

            for (i, label) in [tr("Music"), tr("Audio files"), tr("Voice messages")]
                .iter()
                .enumerate()
            {
                ui.toggle_value(&mut filters[i], label.as_str());
            }
        });
        ui.add_space(12.);
        if before_filters != filters {
            self.execute(AppCommand::Filters(chat_id, filters));
            self.open_chat(selected);
        }
        let tracks = self.playlist_tracks(selected);
        if play_all {
            self.execute(AppCommand::StartSession(
                tracks.iter().map(|t| t.metadata()).collect(),
                0,
            ));
        }
        if pin_all {
            self.execute(AppCommand::SavePlaylist(
                chat_id,
                self.state.view().playlists[selected].filters,
            ));
        }
        // This reserved status line avoids shifting the list as loading starts/ends.
        ui.allocate_ui(egui::vec2(ui.available_width(), 24.), |ui| {
            if self.state.library().indexing_chat() == Some(chat_id) {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.small(tr("Loading music from this chat…"));
                });
            }
        });
        if tracks.is_empty() {
            ui.add_space(24.);
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
        let scroll = egui::ScrollArea::vertical()
            .id_salt(("playlist-tracks", chat_id))
            .show_rows(ui, design::ROW_HEIGHT, tracks.len(), |ui, range| {
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
                    if active || hover > 0. {
                        ui.painter().rect_filled(
                            rect,
                            8.,
                            if active {
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
                    if response.has_focus() {
                        ui.painter().rect_stroke(
                            rect,
                            8.,
                            egui::Stroke::new(1., p.accent),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let mut menu_clicked = false;
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(rect.shrink2(egui::vec2(8., 6.)))
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
                            .map_or_else(|| (i + 1).to_string(), str::to_owned);
                            ui.add_sized(
                                [22., 36.],
                                egui::Label::new(egui::RichText::new(indicator).color(if active {
                                    p.accent
                                } else {
                                    p.muted
                                })),
                            );
                            art(ui, &t.title, t.image_path.as_deref(), 44.);
                            let offline = if t.pinned.is_some() {
                                Some(("✓", tr("Saved offline")))
                            } else if self
                                .state
                                .offline()
                                .pending
                                .values()
                                .any(|p| p.id() == t.id())
                            {
                                Some(("↓", tr("Downloading…")))
                            } else if self
                                .state
                                .offline()
                                .errors()
                                .contains_key(&(t.chat, t.message))
                            {
                                Some(("!", tr("Download failed · Save offline to retry")))
                            } else {
                                None
                            };
                            if let Some((mark, hint)) = offline {
                                let pos = egui::pos2(rect.left() + 78., rect.bottom() - 10.);
                                ui.painter().circle_filled(pos, 9., p.panel);
                                ui.painter().text(
                                    pos,
                                    egui::Align2::CENTER_CENTER,
                                    mark,
                                    egui::FontId::proportional(12.),
                                    p.accent,
                                );
                                ui.interact(
                                    egui::Rect::from_center_size(pos, egui::vec2(18., 18.)),
                                    response.id.with("offline"),
                                    egui::Sense::hover(),
                                )
                                .on_hover_text(hint);
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
                                };
                                let menu = ui.menu_button("…", &mut contents);
                                response.context_menu(&mut contents);
                                #[cfg(test)]
                                self.test_rows.insert(t.message, (rect, menu.response.rect));
                                #[cfg(test)]
                                self.test_row_ids.insert(t.message, response.id);
                                menu_clicked = menu.response.contains_pointer()
                                    || menu.response.has_focus()
                                    || menu.inner.is_some();
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
                0 => self.execute(AppCommand::StartSession(
                    tracks.iter().map(|t| t.metadata()).collect(),
                    i,
                )),
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
fn track_labels(ui: &mut egui::Ui, title: &str, artist: &str, active: bool) -> egui::Response {
    ui.allocate_ui(
        egui::vec2((ui.available_width() - 104.).max(40.), 40.),
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
                    .truncate(),
                )
                .on_hover_text(title);
                ui.add(egui::Label::new(egui::RichText::new(artist).weak()).truncate())
                    .on_hover_text(artist);
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

impl App {
    fn queue_screen(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(tr("Up next"));
            if button(ui, Icon::Close, "Close queue") {
                self.queue_open = false;
            }
        });
        ui.weak(if self.state.view().shuffle {
            tr("Shuffled order")
        } else {
            tr("Playlist order")
        });
        ui.small(tr("Playback uses the tracks loaded when you started."));
        if ui.button(tr("Clear queue")).clicked() {
            self.execute(AppCommand::Clear);
            self.persist();
        }
        if let Some(t) = &self.state.view().session.current() {
            ui.label(format!("{}: {}", tr("Now playing"), t.title));
        }
        if self.state.view().session.remaining().is_empty() {
            ui.label(tr("Your queue is empty"));
        }
        let mut play = None;
        let mut remove = None;
        let mut up = None;
        let mut down = None;
        let mut moved = None;
        let queue_scroll = egui::ScrollArea::vertical()
            .id_salt("queue-list")
            .show(ui, |ui| {
                for (i, t) in self.state.view().session.remaining().iter().enumerate() {
                    let context = ui.interact(
                        egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(ui.available_width(), 44.),
                        ),
                        ui.id().with(("queue-context", t.entry_id)),
                        egui::Sense::click(),
                    );
                    let row = ui.horizontal(|ui| {
                        let handle = ui.add(egui::Label::new("⠿").sense(egui::Sense::drag()));
                        if handle.drag_started() {
                            handle.dnd_set_drag_payload(i);
                        }
                        ui.add_sized(
                            [20., 36.],
                            egui::Label::new(
                                egui::RichText::new((i + 1).to_string()).weak().small(),
                            ),
                        );
                        if button(ui, Icon::Play, "Play") {
                            play = Some(t.entry_id);
                        }
                        ui.allocate_ui(
                            egui::vec2((ui.available_width() - 50.).max(60.), 44.),
                            |ui| {
                                ui.vertical(|ui| {
                                    ui.add(egui::Label::new(&t.title).truncate())
                                        .on_hover_text(&t.title);
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(&t.artist).weak())
                                            .truncate(),
                                    );
                                });
                            },
                        );
                        let mut contents = |ui: &mut egui::Ui| {
                            if i > 0 && ui.button(tr("Move up")).clicked() {
                                up = Some(i);
                                ui.close();
                            }
                            if i + 1 < self.state.view().session.remaining().len()
                                && ui.button(tr("Move down")).clicked()
                            {
                                down = Some(i);
                                ui.close();
                            }
                            if ui.button(tr("Remove")).clicked() {
                                remove = Some(i);
                                ui.close();
                            }
                        };
                        ui.menu_button("…", &mut contents);
                        context.context_menu(&mut contents);
                    });
                    if let Some(from) = row.response.dnd_release_payload::<usize>() {
                        moved = Some((*from, i));
                    }
                }
            });
        design::middle_scroll(
            ui,
            queue_scroll.id,
            queue_scroll.inner_rect,
            queue_scroll.content_size.y,
        );
        if let Some((from, to)) = moved {
            self.execute(AppCommand::Move(from, to));
            self.persist();
        }
        if let Some(id) = play {
            self.execute(AppCommand::PlayQueue(id));
        }
        if let Some(i) = remove {
            self.execute(AppCommand::Remove(i));
            self.persist();
        }
        if let Some(i) = up {
            self.execute(AppCommand::Move(i, i - 1));
            self.persist();
        }
        if let Some(i) = down {
            self.execute(AppCommand::Move(i, i + 2));
            self.persist();
        }
    }
}

#[cfg(test)]
#[path = "screens/tests.rs"]
mod tests;
