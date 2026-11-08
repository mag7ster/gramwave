use crate::*;
impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.background(ctx, true);
    }
    fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
        self.render(root);
    }
}
impl App {
    pub(crate) fn settings_ui(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let p = design::palette(self.state.light);
        let width = (ctx.content_rect().width() - 80.).clamp(280., 520.);
        let mut close = false;
        let mut changed = false;
        let mut remove = None;
        let response = egui::Modal::new(egui::Id::new("player-settings"))
            .frame(
                egui::Frame::new()
                    .fill(p.panel)
                    .corner_radius(16)
                    .inner_margin(24.),
            )
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.set_height((ctx.content_rect().height() - 112.).clamp(200., 720.));
                ui.spacing_mut().scroll.floating = false;
                ui.spacing_mut().scroll.bar_width = 4.;
                ui.horizontal(|ui| {
                    ui.heading(egui::RichText::new(tr("Settings")).size(26.).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = design::button(ui, design::Icon::Close, "Close settings");
                    });
                });
                ui.add_space(12.);
                egui::ScrollArea::vertical()
                    .id_salt("settings-content")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .max_height((ctx.content_rect().height() - 160.).max(140.))
                    .show(ui, |ui| {
                        design::settings_section(ui, tr("Appearance"), |ui| {
                            ui.weak(tr("Choose the player’s color scheme."));
                            ui.horizontal(|ui| {
                                if design::choice(ui, tr("Dark"), !self.state.light) {
                                    self.state.light = false;
                                    changed = true;
                                }
                                if design::choice(ui, tr("Light"), self.state.light) {
                                    self.state.light = true;
                                    changed = true;
                                }
                            });
                        });
                        design::settings_section(ui, tr("Playback"), |ui| {
                            ui.label(tr("Shuffle"));
                            ui.horizontal(|ui| {
                                if design::choice(ui, tr("Off"), !self.state.shuffle) {
                                    self.set_shuffle(false);
                                }
                                if design::choice(ui, tr("On"), self.state.shuffle) {
                                    self.set_shuffle(true);
                                }
                            });
                            ui.add_space(4.);
                            ui.label(tr("Repeat"));
                            ui.horizontal_wrapped(|ui| {
                                for (mode, label) in [
                                    (RepeatMode::Off, tr("Off")),
                                    (RepeatMode::All, tr("All tracks")),
                                    (RepeatMode::One, tr("One track")),
                                ] {
                                    if design::choice(ui, label, self.state.repeat == mode) {
                                        self.set_repeat(mode);
                                    }
                                }
                            });
                            ui.add_space(4.);
                            ui.label(tr("Default audio filters"));
                            ui.horizontal_wrapped(|ui| {
                                for (i, label) in
                                    [tr("Music"), tr("Audio files"), tr("Voice messages")]
                                        .into_iter()
                                        .enumerate()
                                {
                                    if design::choice(ui, label, self.state.filters[i]) {
                                        self.state.filters[i] = !self.state.filters[i];
                                        changed = true;
                                    }
                                }
                            });
                            if ui.button(tr("Apply to all playlists")).clicked() {
                                for playlist in &mut self.state.playlists {
                                    playlist.filters = self.state.filters;
                                }
                                changed = true;
                            }
                        });
                        design::settings_section(ui, tr("Offline storage"), |ui| {
                            ui.label(tr("Automatic cache · 2 GiB"));
                            ui.weak(tr("Saved offline tracks are kept separately."));
                            if ui.button(tr("Clear automatic cache")).clicked() {
                                self.clear_automatic_cache();
                            }
                            if !self.state.pending_pins.is_empty() {
                                ui.weak(tr("Downloading offline tracks…"));
                                if ui.button(tr("Cancel offline downloads")).clicked() {
                                    self.cancel_offline_downloads();
                                    changed = true;
                                }
                            }
                            let mut any = false;
                            for t in self.state.playlists.iter().flat_map(|p| &p.tracks) {
                                if let Some(path) = &t.pinned {
                                    if !any {
                                        ui.add_space(8.);
                                        ui.strong(tr("Saved tracks"));
                                    }
                                    any = true;
                                    ui.horizontal(|ui| {
                                        ui.add_sized(
                                            [ui.available_width() - 44., 36.],
                                            egui::Label::new(&t.title).truncate(),
                                        )
                                        .on_hover_text(&t.title);
                                        if design::button(
                                            ui,
                                            design::Icon::Close,
                                            "Remove offline copy",
                                        ) {
                                            remove = Some((t.chat, t.message, path.clone()));
                                        }
                                    });
                                }
                            }
                            if !any {
                                ui.weak(tr("No offline tracks saved yet."));
                            }
                        });
                        ui.add_space(4.);
                        ui.small(
                            egui::RichText::new(tr(
                                "Online presence is always disabled by this player.",
                            ))
                            .weak(),
                        );
                    });
            });
        if let Some((chat, message, path)) = remove {
            if std::path::Path::new(&path).starts_with(self.root.join("offline")) {
                match std::fs::remove_file(&path) {
                    Ok(()) => {
                        for p in &mut self.state.playlists {
                            if p.id == chat {
                                p.pinned = false;
                                for t in &mut p.tracks {
                                    if t.message == message {
                                        t.pinned = None;
                                    }
                                }
                            }
                        }
                        changed = true;
                    }
                    Err(e) => {
                        self.error = e.to_string();
                        self.error_retry = None;
                    }
                }
            }
        }
        if changed {
            self.persist();
        }
        if close || response.should_close() {
            self.settings_open = false;
        }
    }
    fn clear_automatic_cache(&self) {
        self.send(json!({"@type":"optimizeStorage","size":0,"ttl":0,"count":0,"immunity_delay":0,"file_types":[],"chat_ids":[],"exclude_chat_ids":[],"return_deleted_file_statistics":true,"chat_limit":0}));
    }
    fn cancel_offline_downloads(&mut self) {
        self.pin_cancel
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        for t in self.state.pending_pins.iter().filter(|t| t.file > 0) {
            self.send(
                json!({"@type":"cancelDownloadFile","file_id":t.file,"only_if_pending":true}),
            );
        }
        self.state.pending_pins.clear();
    }
}

impl App {
    pub fn background(&mut self, ctx: &egui::Context, window_present: bool) {
        self.process(ctx);
        if let Some(tg) = &self.tg {
            for t in self.track_wanted.clone() {
                let key = (t.chat, t.message);
                if self.track_requests.len() >= 4
                    || self.track_requests.contains(&key)
                    || self
                        .track_retry
                        .get(&key)
                        .is_some_and(|at| *at > std::time::Instant::now())
                {
                    continue;
                }
                self.track_requests.insert(key);
                let tg = tg.clone();
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(Event::TrackReady(key, media_loader::resolve(&tg, &t)));
                });
            }
            for id in self.image_wanted.clone() {
                if id == 0 || self.image_requests.len() >= 4 {
                    continue;
                }
                if self
                    .artwork_path(id)
                    .is_some_and(|path| std::path::Path::new(path).is_file())
                {
                    continue;
                }
                if self
                    .image_retry
                    .get(&id)
                    .is_some_and(|at| *at > std::time::Instant::now())
                {
                    continue;
                }
                if self.image_requests.insert(id) {
                    let tg = tg.clone();
                    let tx = self.tx.clone();
                    std::thread::spawn(move || {
                        let result = tg
                            .call(json!({
                                "@type": "downloadFile", "file_id": id, "priority": 1,
                                "offset": 0, "limit": 0, "synchronous": true
                            }))
                            .and_then(|v| {
                                if v["local"]["is_downloading_completed"] == true {
                                    v["local"]["path"]
                                        .as_str()
                                        .filter(|p| !p.is_empty())
                                        .map(str::to_owned)
                                        .ok_or_else(|| "Image path missing".into())
                                } else {
                                    Err("Image download incomplete".into())
                                }
                            });
                        let _ = tx.send(Event::Artwork(id, result));
                    });
                }
            }
        }

        if self.auth["@type"] == "authorizationStateWaitPhoneNumber"
            && !self.auth_pending
            && self.qr.is_empty()
            && self.auth_error.is_empty()
        {
            self.submit_auth(json!({"@type":"requestQrCodeAuthentication","other_user_ids":[]}));
        }
        if window_present
            && self
                .show_requested
                .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        self.frames += 1;
        if std::env::args().any(|a| a == "--smoke-test") {
            if self.preview_started.elapsed() > Duration::from_secs(2) && !self.preview_captured {
                self.preview_captured = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            for event in ctx.input(|i| i.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = event {
                    let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    let _ = image::save_buffer(
                        demo::preview_path(),
                        &bytes,
                        image.width() as u32,
                        image.height() as u32,
                        image::ColorType::Rgba8,
                    );
                }
            }
            if self.preview_started.elapsed() > Duration::from_millis(3500) {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if let Some(tray) = &self.tray {
            for event in tray.events() {
                match event {
                    fastframe_tray::Event::Menu("quit") => {
                        self.quitting = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    fastframe_tray::Event::Toggle
                    | fastframe_tray::Event::Show
                    | fastframe_tray::Event::Menu("show") => {
                        self.show_requested
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    fastframe_tray::Event::Menu("play") => {
                        self.set_paused(!self.paused);
                    }
                    fastframe_tray::Event::Menu("next") => self.next(),
                    _ => {}
                }
            }
        }
        if window_present
            && ctx.input(|i| i.viewport().close_requested())
            && !self.quitting
            && self
                .tray
                .as_ref()
                .is_some_and(fastframe_tray::Tray::is_shown)
        {
            self.hidden = true;
        }

        use fastframe_now_playing::{Command, Playback, State as MediaState, Track as MediaTrack};
        for command in self.media.commands() {
            match command {
                Command::Next => self.next(),
                Command::PlayPause | Command::Pause | Command::Play => {
                    let paused = match command {
                        Command::Play => false,
                        Command::Pause => true,
                        _ => !self.paused,
                    };
                    self.set_paused(paused);
                }

                Command::SeekBy(ms) => {
                    let pos = self
                        .player
                        .as_ref()
                        .map(|p| p.number("time-pos"))
                        .unwrap_or(0.);
                    self.seek(pos + ms as f64 / 1000.);
                }
                Command::SetPosition { track_id, position } => {
                    if self
                        .state
                        .session
                        .current
                        .as_ref()
                        .is_some_and(|t| format!("{}-{}", t.chat, t.message) == track_id)
                    {
                        self.seek(position.as_secs_f64());
                    }
                }
                Command::SetVolume(v) => self.set_volume((v * 100.) as f32),
                Command::SetShuffle(v) => self.set_shuffle(v),
                Command::SetRepeat(v) => self.set_repeat(v.into()),
                Command::Previous => self.previous(),
                Command::Stop => {
                    self.playback_generation += 1;
                    if let Some(p) = &self.player {
                        p.cmd(&["stop"]);
                    }
                    self.state.session.current = None;
                    self.loading = false;
                    self.seek_preview = None;
                    self.paused = true;
                }
                Command::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Command::Raise => ctx.send_viewport_cmd(egui::ViewportCommand::Focus),
                _ => {}
            }
        }
        self.media.update(MediaState {
            playback: if self.state.session.current.is_none() {
                Playback::Stopped
            } else if self.paused || self.loading {
                Playback::Paused
            } else {
                Playback::Playing
            },
            track: self.state.session.current.as_ref().map(|t| MediaTrack {
                id: format!("{}-{}", t.chat, t.message),
                title: t.title.clone(),
                artists: vec![t.artist.clone()],
                duration: Some(Duration::from_secs(t.duration)),
                ..Default::default()
            }),
            position: Duration::from_secs_f64(
                self.player
                    .as_ref()
                    .map(|p| p.number("time-pos"))
                    .unwrap_or(0.)
                    .max(0.),
            ),
            volume: Some(self.state.volume as f64 / 100.),
            shuffle: Some(self.state.shuffle),
            repeat: Some(self.state.repeat.into()),
            ..Default::default()
        });

        ctx.request_repaint_after(Duration::from_millis(200));
        let events = self.player.as_ref().map(|p| p.drain()).unwrap_or_default();
        for event in events {
            match event {
                audio::PlaybackEvent::Loaded => self.loading = false,
                audio::PlaybackEvent::Finished if self.state.session.current.is_some() => {
                    if !self.loading {
                        self.advance(true);
                    }
                }

                audio::PlaybackEvent::Failed(error) => {
                    self.error = format!("{}: {error}", tr("Unable to play track"));
                    self.error_retry = self
                        .state
                        .session
                        .current
                        .clone()
                        .map(RetryAction::Playback);
                    self.loading = false;
                    self.paused = true;
                }
                _ => {}
            }
        }
    }
}

impl App {
    pub fn shutdown_core(&mut self) {
        self.persist();
        self.db.flush();
        if let Some(a) = &self.server {
            a.store(false, std::sync::atomic::Ordering::Relaxed)
        }
        if let Some(tg) = &self.tg {
            tg.request(json!({"@type":"close"}));
        }
    }
}
