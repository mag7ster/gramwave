use crate::*;
use design::{button, Icon};
impl App {
    pub(crate) fn render(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        self.image_wanted.clear();
        self.track_wanted.clear();
        design::style(&ctx, self.state.light);
        if !self.settings_open
            && !ctx.text_edit_focused()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space))
        {
            self.set_paused(!self.paused);
        }

        if ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::Q)) {
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if std::env::args().any(|a| a.starts_with("--preview-auth="))
            || (!self.demo && self.auth["@type"] != "authorizationStateReady")
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
                                    .fill(design::palette(self.state.light).panel)
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
                        for i in self.state.sidebar_chats(true, &self.chat_query) {
                            if ui.button(&self.state.playlists[i].title).clicked() {
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
                        .fill(design::palette(self.state.light).panel)
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
                    let indices = self
                        .state
                        .sidebar_chats(self.chat_search_active, &self.chat_query);
                    if indices.is_empty() {
                        ui.weak(if self.chat_search_active {
                            tr("No chats found")
                        } else {
                            tr("Pin chats here, or find one using search.")
                        });
                    }
                    let mut open = None;
                    let mut toggle = None;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for i in indices {
                            let p = &self.state.playlists[i];
                            let id = p.id;
                            let pinned = self.state.chat_pins.contains(&id);
                            let frame = egui::Frame::new()
                                .fill(if self.opened_chat == Some(id) {
                                    design::palette(self.state.light).selected
                                } else {
                                    egui::Color32::TRANSPARENT
                                })
                                .corner_radius(8)
                                .inner_margin(8.)
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        art(ui, &p.title, p.image_path.as_deref(), 40.);
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
                                self.image_wanted.push(p.image_file);
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
                            .fill(design::palette(self.state.light).panel)
                            .inner_margin(20.),
                    )
                    .show(root, |ui| self.queue_screen(ui));
            }
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(design::palette(self.state.light).background)
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
                        .fill(design::palette(self.state.light).raised)
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
                            self.retry_error();
                        }
                        if ui.button(tr("Dismiss")).clicked() {
                            self.error.clear();
                        }
                    });
                });
        }
    }
    fn transport(&mut self, ui: &mut egui::Ui) {
        if let Some(t) = &self.state.session.current {
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
                        let t = self.state.session.current.as_ref();
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
                                if self.loading {
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
                        let shuffle_hint = if self.state.shuffle {
                            tr("Shuffle on · Turn off")
                        } else {
                            tr("Shuffle off · Turn on")
                        };
                        if design::toggle(ui, Icon::Shuffle, self.state.shuffle, "", shuffle_hint) {
                            self.set_shuffle(!self.state.shuffle);
                        }
                        if button(ui, Icon::Previous, "Previous") {
                            self.previous();
                        }
                        if button(
                            ui,
                            if self.paused || self.state.session.current.is_none() {
                                Icon::Play
                            } else {
                                Icon::Pause
                            },
                            "Play / pause",
                        ) {
                            self.set_paused(!self.paused);
                        }
                        if button(ui, Icon::Next, "Next") {
                            self.next();
                        }
                        let hint = match self.state.repeat {
                            RepeatMode::Off => tr("Repeat off · Repeat all"),
                            RepeatMode::All => tr("Repeat all · Repeat one"),
                            RepeatMode::One => tr("Repeat one · Turn off"),
                        };
                        if design::toggle(
                            ui,
                            Icon::Repeat,
                            self.state.repeat != RepeatMode::Off,
                            if self.state.repeat == RepeatMode::One {
                                "1"
                            } else {
                                ""
                            },
                            hint,
                        ) {
                            self.set_repeat(self.state.repeat.next());
                        }
                    });
                    let duration = if self.loading {
                        0.
                    } else {
                        self.player
                            .as_ref()
                            .map(|p| p.number("duration"))
                            .unwrap_or(0.)
                    };
                    let duration = if self.demo {
                        self.state
                            .session
                            .current
                            .as_ref()
                            .map(|t| t.duration as f64)
                            .unwrap_or(0.)
                    } else {
                        duration
                    };
                    let pos = if self.loading {
                        0.
                    } else {
                        self.player
                            .as_ref()
                            .map(|p| p.number("time-pos"))
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
                            && self.state.session.current.is_some()
                            && !self.loading;
                        let response = ui
                            .add_enabled_ui(enabled, |ui| {
                                ui.push_id(("seek", self.playback_generation), |ui| {
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
                            self.seek(preview);
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
                        if self.state.volume == 0. {
                            Icon::Muted
                        } else {
                            Icon::Volume
                        },
                        if self.state.volume == 0. {
                            tr("Unmute")
                        } else {
                            tr("Mute")
                        },
                    ) {
                        self.toggle_mute();
                    }
                    let mut volume = self.state.volume as f64;
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
                        self.set_volume(volume as f32);
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

        let Some(p) = self.state.playlists.get(self.selected) else {
            ui.add_space(80.);
            ui.heading(tr("Your next favourite is waiting."));
            ui.weak(&self.status);
            return;
        };
        let chat_id = p.id;
        self.image_wanted.push(p.image_file);
        let title = p.title.clone();
        let artwork = p.image_path.clone();
        let url = p.url.clone();

        egui::Frame::new()
            .fill(design::palette(self.state.light).panel)
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
                        if self.state.light {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::BLACK
                        },
                        22.,
                    ))
                    .fill(design::palette(self.state.light).accent)
                    .corner_radius(30)
                    .min_size(egui::vec2(44., 44.)),
                )
                .clicked();
            if ui
                .button(if self.state.chat_pins.contains(&chat_id) {
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
                    self.open_chat(self.selected);
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
        let before_filters = self.state.playlists[self.selected].filters;
        ui.horizontal_wrapped(|ui| {
            ui.add(Icon::Search.image(egui::Color32::GRAY, 18.));
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text(tr("Search this playlist"))
                    .desired_width(200.),
            );
            let p = &mut self.state.playlists[self.selected];
            for (i, label) in [tr("Music"), tr("Audio files"), tr("Voice messages")]
                .iter()
                .enumerate()
            {
                ui.toggle_value(&mut p.filters[i], label.as_str());
            }
        });
        ui.add_space(12.);
        if before_filters != self.state.playlists[self.selected].filters {
            self.open_chat(self.selected);
        }
        let p = &self.state.playlists[self.selected];
        let q = self.search.to_lowercase();
        let mut tracks: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| {
                p.filters[t.kind]
                    && format!("{} {}", t.title, t.artist)
                        .to_lowercase()
                        .contains(&q)
            })
            .cloned()
            .collect();
        match self.sort {
            1 => tracks.sort_by(|a, b| a.title.cmp(&b.title)),
            2 => tracks.sort_by(|a, b| a.artist.cmp(&b.artist)),
            _ => {}
        }
        if play_all {
            self.start_session(&tracks, 0);
        }
        if pin_all {
            self.state.playlists[self.selected].pinned = true;
            for t in &tracks {
                self.pin(t.clone());
            }
        }
        // This reserved status line avoids shifting the list as loading starts/ends.
        ui.allocate_ui(egui::vec2(ui.available_width(), 24.), |ui| {
            if self.indexing_chat == Some(chat_id) {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.small(tr("Loading music from this chat…"));
                });
            }
        });
        if tracks.is_empty() {
            ui.add_space(24.);
            ui.heading(
                if !self.state.playlists[self.selected]
                    .filters
                    .iter()
                    .any(|v| *v)
                {
                    tr("All filters are off")
                } else if !self.search.is_empty() {
                    tr("No matching tracks")
                } else if self.indexing_chat.is_some() {
                    tr("Loading music…")
                } else {
                    tr("No music in this chat yet")
                },
            );
            if ui.button(tr("Refresh library")).clicked() {
                self.open_chat(self.selected);
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
                        .session
                        .current
                        .as_ref()
                        .is_some_and(|c| playback::same(c, t));
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
                                Some(if self.loading {
                                    "…"
                                } else if self.paused {
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
                            } else if self.state.pending_pins.iter().any(|p| playback::same(p, t)) {
                                Some(("↓", tr("Downloading…")))
                            } else if self.pin_errors.contains_key(&(t.chat, t.message)) {
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
                                let menu = ui.menu_button("…", |ui| {
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
                                        if ui.button(label).clicked() {
                                            action = Some((kind, i));
                                            ui.close();
                                        }
                                    }
                                    ui.separator();
                                    if t.pinned.is_some() {
                                        ui.label(tr("Saved offline"));
                                    } else if self
                                        .state
                                        .pending_pins
                                        .iter()
                                        .any(|p| playback::same(p, t))
                                    {
                                        ui.label(tr("Downloading…"));
                                    } else if let Some(error) =
                                        self.pin_errors.get(&(t.chat, t.message))
                                    {
                                        ui.label(tr("Download failed · Save offline to retry"));
                                        ui.label(error);
                                    }
                                });
                                #[cfg(test)]
                                self.test_rows.insert(t.message, (rect, menu.response.rect));
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
        let more = self.state.playlists[self.selected].has_more();
        if more && self.indexing_chat.is_none() && !self.demo {
            let at_bottom = scroll.state.offset.y > 0.
                && scroll.state.offset.y + scroll.inner_rect.height()
                    >= scroll.content_size.y - 120.;
            if ui.button(tr("Load more music")).clicked() || at_bottom {
                self.load_chat(self.selected, false);
            }
        }
        if let Some((kind, i)) = action {
            match kind {
                0 => self.start_session(&tracks, i),
                6 => {
                    if self
                        .state
                        .session
                        .current
                        .as_ref()
                        .is_some_and(|c| playback::same(c, &tracks[i]))
                    {
                        self.set_paused(!self.paused);
                    } else {
                        self.start_session(&tracks, i);
                    }
                }
                5 => ui.ctx().copy_text(tracks[i].title.clone()),
                1 => self.state.queue.insert(0, tracks[i].clone()),
                2 => self.state.queue.push(tracks[i].clone()),
                3 => self.pin(tracks[i].clone()),
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
    if let Some(path) = path.filter(|p| std::path::Path::new(p).is_file()) {
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
        ui.weak(if self.state.shuffle {
            tr("Shuffled order")
        } else {
            tr("Playlist order")
        });
        ui.small(tr("Playback uses the tracks loaded when you started."));
        if self.state.queue.is_empty()
            && self.state.session.remaining.is_empty()
            && self.state.session.next_cycle.is_empty()
        {
            ui.label(tr("Your queue is empty"));
        }
        let mut play = None;
        let mut remove = None;
        let mut up = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, t) in self
                .state
                .queue
                .iter()
                .chain(self.state.session.remaining.iter())
                .chain(self.state.session.next_cycle.iter())
                .enumerate()
            {
                if i == 0 && !self.state.queue.is_empty() {
                    ui.strong(tr("Added by you"));
                }
                if i == self.state.queue.len() && !self.state.session.remaining.is_empty() {
                    ui.strong(tr("From this session"));
                }
                if i == self.state.queue.len() + self.state.session.remaining.len()
                    && !self.state.session.next_cycle.is_empty()
                {
                    ui.strong(tr("Next cycle"));
                }
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [20., 36.],
                        egui::Label::new(egui::RichText::new((i + 1).to_string()).weak().small()),
                    );
                    if button(ui, Icon::Play, "Play") {
                        play = Some(i);
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
                    ui.menu_button("…", |ui| {
                        if i > 0 && i < self.state.queue.len() && ui.button(tr("Move up")).clicked()
                        {
                            up = Some(i);
                            ui.close();
                        }
                        if ui.button(tr("Remove")).clicked() {
                            remove = Some(i);
                            ui.close();
                        }
                    });
                });
            }
        });
        if let Some(i) = play {
            if let Some(t) = self.state.session.jump_to(&mut self.state.queue, i) {
                self.play(t);
                self.persist();
            }
        }
        if let Some(i) = remove {
            if i < self.state.queue.len() {
                self.state.queue.remove(i);
            } else {
                let index = i - self.state.queue.len();
                if index < self.state.session.remaining.len() {
                    self.state.session.remaining.remove(index)
                } else {
                    self.state
                        .session
                        .next_cycle
                        .remove(index - self.state.session.remaining.len())
                };
            }
            self.persist();
        }
        if let Some(i) = up {
            if i > 0 {
                self.state.queue.swap(i, i - 1);
                self.persist();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_metadata_fits_virtual_list_row_height() {
        let ctx = egui::Context::default();
        design::style(&ctx, false);
        let long = "A very long title or artist name ".repeat(40);
        let mut height = 0.;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(600.);
            let row = egui::Frame::new().inner_margin(6.).show(ui, |ui| {
                ui.set_min_height(48.);
                ui.horizontal(|ui| {
                    track_labels(ui, &long, &long, false);
                });
            });
            height = row.response.rect.height();
        });
        output.textures_delta.clear();
        assert!(
            (height - design::ROW_HEIGHT).abs() < 0.1,
            "Rendered row {height} differs from virtual row height 60"
        );
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    struct Harness {
        app: App,
        ctx: egui::Context,
        time: f64,
    }
    impl Harness {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("tg-player-ui-test-{}", std::process::id()));
            let mut app = App::new_at(root, true);
            app.player = None;
            app.opened_chat = Some(0);
            let ctx = egui::Context::default();
            design::install(&ctx);
            ctx.set_fonts(fastframe_fonts::FontSetup::default().definitions());
            let mut h = Self { app, ctx, time: 0. };
            h.frame(vec![]);
            h.frame(vec![]);
            h
        }
        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 0.02;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(850., 550.),
                )),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut output = self.ctx.run_ui(input, |ui| self.app.render(ui));
            output.textures_delta.clear();
        }
        fn pointer(&mut self, p: egui::Pos2, pressed: bool) {
            self.frame(vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        fn click(&mut self, p: egui::Pos2) {
            self.pointer(p, true);
            self.pointer(p, false);
        }
        fn key(&mut self, key: egui::Key) {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
    }
    #[test]
    fn row_mouse_keyboard_menu_and_seek_contract() {
        let mut h = Harness::new();
        let (row, menu) = h.app.test_rows[&0];
        assert!(
            row.right() <= 850. && row.bottom() < 454.,
            "row overflow: {row:?}"
        );
        assert_eq!(row.height(), design::ROW_HEIGHT);
        // Cover, title and blank row space all activate the same row.
        for x in [row.left() + 50., row.left() + 160., row.right() - 60.] {
            h.app.state.session.current = None;
            h.time += 1.;
            h.click(egui::pos2(x, row.center().y));
            assert_eq!(
                h.app.state.session.current.as_ref().map(|t| t.message),
                Some(0)
            );
            assert!(!h.app.paused);
        }
        h.time += 1.;
        h.click(row.center());
        assert!(h.app.paused);
        h.time += 1.;
        h.click(row.center());
        h.click(row.center());
        assert!(!h.app.paused, "double click must not pause after starting");
        h.time += 1.;
        h.click(menu.center());
        assert!(!h.app.paused, "menu must not activate row");
        h.key(egui::Key::Escape);
        h.time += 1.;
        h.click(row.center());
        assert!(h.app.paused);
        h.key(egui::Key::Enter);
        assert!(!h.app.paused);
        h.key(egui::Key::Space);
        assert!(h.app.paused, "Space on row toggles playback");
        // Click seeks once; a drag previews until release, independent of mpv position.
        let (rect, id) = h.app.test_seek.unwrap();
        h.time += 1.;
        h.click(rect.center());
        assert_eq!(h.app.test_seeks.len(), 1);
        assert!((h.app.test_seeks[0] - 90.).abs() < 1.);
        h.time += 1.;
        h.pointer(rect.center(), true);
        h.frame(vec![egui::Event::PointerMoved(egui::pos2(
            rect.right() - 8.,
            rect.center().y,
        ))]);
        assert_eq!(h.app.test_seeks.len(), 1);
        assert!(h.app.seek_preview.unwrap() > 170.);
        h.frame(vec![]);
        assert!(h.app.seek_preview.unwrap() > 170.);
        h.pointer(egui::pos2(rect.right() - 8., rect.center().y), false);
        assert_eq!(h.app.test_seeks.len(), 2);
        assert!(h.app.seek_preview.is_none());
        h.ctx.memory_mut(|m| m.request_focus(id));
        h.app.seek_preview = Some(90.);
        h.key(egui::Key::Home);
        assert_eq!(*h.app.test_seeks.last().unwrap(), 0.);
        h.key(egui::Key::End);
        assert_eq!(*h.app.test_seeks.last().unwrap(), 180.);
        h.key(egui::Key::ArrowRight);
        assert_eq!(*h.app.test_seeks.last().unwrap(), 5.); // backend position remains zero in demo
        h.time += 1.;
        h.pointer(rect.center(), true);
        h.app.next();
        assert!(h.app.seek_preview.is_none());
        let before = h.app.test_seeks.len();
        h.pointer(rect.center(), false);
        assert_eq!(
            h.app.test_seeks.len(),
            before,
            "releasing an old drag must not seek the next track"
        );
        // Unknown duration disables seeking, even while holding an old slider.
        h.app.start_session(&[Track::default()], 0);
        let before = h.app.test_seeks.len();
        h.time += 1.;
        h.click(rect.center());
        assert_eq!(h.app.test_seeks.len(), before);
        // Space is text while a search field owns focus.
        let search = egui::Id::new("chat-search");
        h.ctx.memory_mut(|m| m.request_focus(search));
        h.frame(vec![]);
        let paused = h.app.paused;
        h.key(egui::Key::Space);
        assert_eq!(h.app.paused, paused);
        let mut reached_row = false;
        for _ in 0..60 {
            h.key(egui::Key::Tab);
            let focused = h.ctx.memory(|m| m.focused());
            if focused
                .and_then(|id| h.ctx.read_response(id))
                .is_some_and(|r| r.rect == row)
            {
                reached_row = true;
                break;
            }
        }
        assert!(reached_row, "Tab must reach track rows");
        h.app.set_volume(32.);
        h.app.toggle_mute();
        assert_eq!(h.app.state.volume, 0.);
        h.app.toggle_mute();
        assert_eq!(h.app.state.volume, 32.);
    }
}
