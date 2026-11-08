use crate::*;
use design::{button, Icon, GREEN};
impl App {
    pub(crate) fn render(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        design::style(&ctx, self.state.light);
        if !ctx.egui_wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                self.paused = !self.paused;
                if let Some(p) = &self.player {
                    p.cmd(&["set", "pause", if self.paused { "yes" } else { "no" }]);
                }
            }
            for (key, amount) in [(egui::Key::ArrowRight, "5"), (egui::Key::ArrowLeft, "-5")] {
                if ctx.input(|i| i.key_pressed(key)) {
                    if let Some(p) = &self.player {
                        p.cmd(&["seek", amount, "relative"]);
                    }
                }
            }
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
                                    .fill(if self.state.light {
                                        egui::Color32::from_gray(235)
                                    } else {
                                        egui::Color32::from_rgb(26, 26, 26)
                                    })
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
            return;
        }
        egui::Panel::bottom("transport")
            .exact_size(96.)
            .show(root, |ui| {
                self.transport(ui);
            });
        egui::Panel::left("sidebar")
            .exact_size(240.)
            .frame(
                egui::Frame::new()
                    .fill(if self.state.light {
                        egui::Color32::from_gray(240)
                    } else {
                        egui::Color32::from_rgb(10, 10, 10)
                    })
                    .inner_margin(16.),
            )
            .show(root, |ui| {
                ui.add_space(8.);
                ui.heading(egui::RichText::new("TG Player").size(23.).strong());
                ui.add_space(28.);
                ui.horizontal(|ui| {
                    ui.strong(tr("Your library"));
                    if button(ui, Icon::Settings, "Settings") {
                        self.settings_open = true;
                    }
                });
                ui.add_space(12.);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, p) in self.state.playlists.iter().enumerate() {
                        if !p.tracks.iter().any(|t| p.filters[t.kind]) {
                            continue;
                        }
                        let selected = self.selected == i;
                        let frame = egui::Frame::new()
                            .fill(if selected {
                                egui::Color32::from_gray(38)
                            } else {
                                egui::Color32::TRANSPARENT
                            })
                            .corner_radius(8)
                            .inner_margin(8.);
                        let response = frame
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    art(ui, &p.title, p.image_path.as_deref(), 40.);
                                    ui.vertical(|ui| {
                                        ui.set_min_width(ui.available_width());
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&p.title).strong(),
                                            )
                                            .truncate(),
                                        );
                                        ui.weak(i18n::track_count(p.tracks.len()));
                                    });
                                });
                            })
                            .response
                            .interact(egui::Sense::click());
                        if response.clicked() {
                            self.selected = i;
                        }
                        ui.add_space(3.);
                    }
                });
                ui.add_space(16.);
                if ui.button(tr("Quit TG Player")).clicked() {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        if self.queue_open {
            if root.available_width() < 800. {
                let mut open = true;
                egui::Window::new(tr("Queue"))
                    .open(&mut open)
                    .default_width(290.)
                    .anchor(egui::Align2::RIGHT_TOP, [-16., 16.])
                    .show(&ctx, |ui| self.queue_screen(ui));
                if !open {
                    self.queue_open = false;
                }
            } else {
                egui::Panel::right("queue")
                    .default_size(290.)
                    .frame(egui::Frame::new().inner_margin(20.))
                    .show(root, |ui| self.queue_screen(ui));
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(24.))
            .show(root, |ui| {
                self.playlist_screen(ui);
            });
        self.settings_ui(&ctx);
        if self.status != self.notice {
            self.notice = self.status.clone();
            self.notice_until = Some(std::time::Instant::now() + Duration::from_secs(5));
        }
        if self
            .notice_until
            .is_some_and(|t| t > std::time::Instant::now())
            && !self.notice.is_empty()
        {
            egui::Area::new(egui::Id::new("notification"))
                .anchor(egui::Align2::RIGHT_BOTTOM, [-20., -116.])
                .order(egui::Order::Foreground)
                .show(&ctx, |ui| {
                    egui::Frame::new()
                        .fill(egui::Color32::from_gray(40))
                        .corner_radius(8)
                        .inner_margin(14.)
                        .show(ui, |ui| {
                            ui.set_max_width(420.);
                            ui.label(&self.notice);
                        });
                });
        }
    }
    fn transport(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 4.;
        ui.add_space(12.);
        let width = ui.available_width();
        ui.horizontal(|ui| {
            ui.allocate_ui(egui::vec2(width * 0.25, 72.), |ui| {
                ui.horizontal(|ui| {
                    art(
                        ui,
                        self.current
                            .as_ref()
                            .map(|t| t.title.as_str())
                            .unwrap_or("TG"),
                        self.current.as_ref().and_then(|t| t.image_path.as_deref()),
                        56.,
                    );
                    ui.vertical(|ui| {
                        ui.strong(
                            self.current
                                .as_ref()
                                .map(|t| t.title.clone())
                                .unwrap_or_else(|| tr("Choose your soundtrack")),
                        );
                        ui.weak(
                            self.current
                                .as_ref()
                                .map(|t| t.artist.clone())
                                .unwrap_or_else(|| tr("Select a track to get started")),
                        );
                    });
                });
            });
            ui.allocate_ui(egui::vec2(width * 0.48, 72.), |ui| {
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        ui.add_space((ui.available_width() - 220.).max(0.) / 2.);
                        if button(ui, Icon::Shuffle, "Shuffle") {
                            self.shuffle = !self.shuffle;
                        }
                        if button(ui, Icon::Previous, "Previous") {
                            self.previous();
                        }
                        if ui
                            .add(
                                egui::Button::image(
                                    if self.paused || self.current.is_none() {
                                        Icon::Play
                                    } else {
                                        Icon::Pause
                                    }
                                    .image(egui::Color32::BLACK, 22.),
                                )
                                .fill(egui::Color32::WHITE)
                                .corner_radius(24)
                                .min_size(egui::vec2(42., 42.)),
                            )
                            .clicked()
                        {
                            self.paused = !self.paused;
                            if let Some(p) = &self.player {
                                p.cmd(&["set", "pause", if self.paused { "yes" } else { "no" }]);
                            }
                        }
                        if button(ui, Icon::Next, "Next") {
                            self.next();
                        }
                        if button(ui, Icon::Repeat, "Repeat") {
                            self.repeat = !self.repeat;
                        }
                    });
                    let duration = self
                        .player
                        .as_ref()
                        .map(|p| p.number("duration"))
                        .unwrap_or(0.)
                        .max(1.);
                    let mut pos = self
                        .player
                        .as_ref()
                        .map(|p| p.number("time-pos"))
                        .unwrap_or(0.);
                    ui.horizontal(|ui| {
                        ui.small(clock(pos));
                        ui.style_mut().spacing.slider_width = (ui.available_width() - 45.).max(60.);
                        if ui
                            .add(egui::Slider::new(&mut pos, 0.0..=duration).show_value(false))
                            .drag_stopped()
                        {
                            if let Some(p) = &self.player {
                                p.cmd(&["seek", &pos.to_string(), "absolute"]);
                            }
                        }
                        ui.small(clock(if self.current.is_none() { 0. } else { duration }));
                    });
                });
            });
            ui.allocate_ui(egui::vec2(width * 0.24, 72.), |ui| {
                ui.horizontal(|ui| {
                    if button(ui, Icon::Queue, "Queue") {
                        self.queue_open = !self.queue_open;
                    }
                    ui.style_mut().spacing.slider_width = 90.;
                    if ui
                        .add(egui::Slider::new(&mut self.volume, 0.0..=100.).show_value(false))
                        .changed()
                    {
                        if let Some(p) = &self.player {
                            p.cmd(&["set", "volume", &self.volume.to_string()]);
                        }
                    }
                });
            });
        });
    }
    fn playlist_screen(&mut self, ui: &mut egui::Ui) {
        let Some(p) = self.state.playlists.get(self.selected) else {
            ui.add_space(80.);
            ui.heading(tr("Your next favourite is waiting."));
            ui.weak(&self.status);
            return;
        };
        let title = p.title.clone();
        let artwork = p.image_path.clone();
        let url = p.url.clone();
        let count = p.tracks.len();
        egui::Frame::new()
            .fill(egui::Color32::from_rgb(33, 67, 49))
            .corner_radius(12)
            .inner_margin(24.)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    art(ui, &title, artwork.as_deref(), 150.);
                    ui.add_space(16.);
                    ui.vertical(|ui| {
                        ui.add_space(18.);
                        ui.small(tr("TELEGRAM PLAYLIST"));
                        ui.add(
                            egui::Label::new(egui::RichText::new(&title).size(36.).strong()).wrap(),
                        );
                        ui.weak(i18n::track_count(count));
                    });
                });
            });
        ui.add_space(18.);
        let mut play_all = false;
        let mut pin_all = false;
        ui.horizontal(|ui| {
            play_all = ui
                .add(
                    egui::Button::image(Icon::Play.image(egui::Color32::BLACK, 26.))
                        .fill(GREEN)
                        .corner_radius(30)
                        .min_size(egui::vec2(56., 56.)),
                )
                .clicked();
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
                    self.sync();
                    ui.close();
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::ComboBox::from_id_salt("order")
                    .selected_text(tr(["Newest first", "Title", "Artist"][self.sort]))
                    .show_ui(ui, |ui| {
                        for (i, label) in ["Newest first", "Title", "Artist"].iter().enumerate() {
                            ui.selectable_value(&mut self.sort, i, tr(label));
                        }
                    });
            });
        });
        ui.horizontal(|ui| {
            ui.add(Icon::Search.image(egui::Color32::GRAY, 18.));
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text(tr("Search this playlist"))
                    .desired_width(260.),
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
            self.state.queue = tracks.iter().skip(1).cloned().collect();
            if let Some(t) = tracks.first() {
                self.play(t.clone());
            }
        }
        if pin_all {
            self.state.playlists[self.selected].pinned = true;
            for t in &tracks {
                self.pin(t.clone());
            }
        }
        let mut action = None;
        egui::ScrollArea::vertical().show_rows(ui, 56., tracks.len(), |ui, range| {
            for i in range {
                let t = &tracks[i];
                let active = self
                    .current
                    .as_ref()
                    .is_some_and(|c| c.chat == t.chat && c.message == t.message);
                egui::Frame::new()
                    .fill(if active {
                        egui::Color32::from_rgb(27, 45, 33)
                    } else {
                        egui::Color32::TRANSPARENT
                    })
                    .corner_radius(6)
                    .inner_margin(6.)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [24., 36.],
                                egui::Label::new(
                                    egui::RichText::new(format!("{}", i + 1)).color(if active {
                                        GREEN
                                    } else {
                                        egui::Color32::GRAY
                                    }),
                                ),
                            );
                            art(ui, &t.title, t.image_path.as_deref(), 36.);
                            let response = ui
                                .allocate_ui(
                                    egui::vec2((ui.available_width() - 100.).max(150.), 40.),
                                    |ui| {
                                        ui.set_min_width(ui.available_width());
                                        ui.vertical(|ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(&t.title).color(
                                                        if active {
                                                            GREEN
                                                        } else {
                                                            egui::Color32::WHITE
                                                        },
                                                    ),
                                                )
                                                .truncate(),
                                            );
                                            ui.weak(&t.artist);
                                        });
                                    },
                                )
                                .response
                                .interact(egui::Sense::click());
                            if response.double_clicked() {
                                action = Some((0, i));
                            }
                            ui.small(clock(t.duration as f64));
                            ui.menu_button("…", |ui| {
                                for (kind, label) in [
                                    (0, "Play"),
                                    (1, "Play next"),
                                    (2, "Add to queue"),
                                    (3, "Save offline"),
                                    (4, "Copy message link"),
                                ] {
                                    if ui.button(tr(label)).clicked() {
                                        action = Some((kind, i));
                                        ui.close();
                                    }
                                }
                            });
                        });
                    });
            }
        });
        if let Some((kind, i)) = action {
            match kind {
                0 => {
                    self.state.queue = tracks.iter().skip(i + 1).cloned().collect();
                    self.play(tracks[i].clone());
                }
                1 => self.state.queue.insert(0, tracks[i].clone()),
                2 => self.state.queue.push(tracks[i].clone()),
                3 => self.pin(tracks[i].clone()),
                _ => self.link(&tracks[i]),
            }
        }
    }
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
        ui.add(
            egui::Image::new(format!("file://{path}"))
                .fit_to_exact_size(egui::vec2(size, size))
                .corner_radius(6),
        );
    } else {
        cover(ui, title, size);
    }
}

impl App {
    fn queue_screen(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(tr("Up next"));
            if button(ui, Icon::Close, "Close queue") {
                self.queue_open = false;
            }
        });
        let mut play = None;
        let mut remove = None;
        let mut up = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, t) in self.state.queue.iter().enumerate() {
                ui.horizontal(|ui| {
                    if button(ui, Icon::Play, "Play") {
                        play = Some(i);
                    }
                    ui.vertical(|ui| {
                        ui.label(&t.title);
                        ui.weak(&t.artist);
                    });
                    ui.menu_button("…", |ui| {
                        if ui.button(tr("Move up")).clicked() {
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
            let t = self.state.queue.remove(i);
            self.play(t);
        }
        if let Some(i) = remove {
            self.state.queue.remove(i);
        }
        if let Some(i) = up {
            if i > 0 {
                self.state.queue.swap(i, i - 1);
            }
        }
    }
}
