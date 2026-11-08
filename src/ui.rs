use crate::application::commands::AppCommand;
use crate::{design, tr, App, Event, RepeatMode};
use eframe::egui;
use std::time::Duration;
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
        let p = design::palette(self.state.view().light);
        let width = (ctx.content_rect().width() - 80.).clamp(280., 520.);
        let mut close = false;
        let mut changed = false;
        let mut light = self.state.view().light;
        let mut filters = self.state.view().filters;
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
                                if design::choice(ui, tr("Dark"), !self.state.view().light) {
                                    light = false;
                                    changed = true;
                                }
                                if design::choice(ui, tr("Light"), self.state.view().light) {
                                    light = true;
                                    changed = true;
                                }
                            });
                        });
                        design::settings_section(ui, tr("Playback"), |ui| {
                            ui.label(tr("Shuffle"));
                            ui.horizontal(|ui| {
                                if design::choice(ui, tr("Off"), !self.state.view().shuffle) {
                                    self.execute(AppCommand::Shuffle(false));
                                }
                                if design::choice(ui, tr("On"), self.state.view().shuffle) {
                                    self.execute(AppCommand::Shuffle(true));
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
                                    if design::choice(ui, label, self.state.view().repeat == mode) {
                                        self.execute(AppCommand::Repeat(mode));
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
                                    if design::choice(ui, label, self.state.view().filters[i]) {
                                        filters[i] = !filters[i];
                                        changed = true;
                                    }
                                }
                            });
                            if ui.button(tr("Apply to all playlists")).clicked() {
                                let ids: Vec<_> =
                                    self.state.view().playlists.iter().map(|p| p.id).collect();
                                for id in ids {
                                    self.execute(AppCommand::Filters(id, filters));
                                }
                                changed = true;
                            }
                        });
                        design::settings_section(ui, tr("Offline storage"), |ui| {
                            ui.label(tr("Automatic cache · 2 GiB"));
                            ui.weak(tr("Saved offline tracks are kept separately."));
                            if ui.button(tr("Clear automatic cache")).clicked() {
                                self.execute(AppCommand::ClearCache);
                            }
                            if !self.state.offline().pending.is_empty() {
                                ui.weak(tr("Downloading offline tracks…"));
                                if ui.button(tr("Cancel offline downloads")).clicked() {
                                    self.execute(AppCommand::CancelOffline);
                                    changed = true;
                                }
                            }
                            let mut any = false;
                            for copy in self.state.offline().copies.values() {
                                let t = &copy.track;
                                {
                                    let path = &copy.path;
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
            self.execute(AppCommand::RemoveOffline(
                crate::domain::TrackId { chat, message },
                path,
            ));
        }
        if changed {
            let mut settings =
                tg_player_application::persistence::Settings::from(self.state.view());
            settings.light = light;
            settings.filters = filters;
            self.execute(AppCommand::Settings(settings));
            self.persist();
        }
        if close || response.should_close() {
            self.settings_open = false;
        }
    }
    pub(crate) fn clear_automatic_cache(&self) {
        if let Some(tg) = self.tg.clone() {
            self.async_event(
                tg_player_application::tasks::JobClass::Background,
                0,
                async move {
                    match tg.clear_cache(&Default::default()).await {
                        Ok(()) => Event::Noop,
                        Err(e) => Event::Error(e),
                    }
                },
                Event::Error,
            );
        }
    }
}

impl App {
    pub fn background(&mut self, ctx: &egui::Context, window_present: bool) {
        self.process(ctx);
        self.process_control();
        self.pump_offline();
        let now = std::time::Instant::now();
        for track in self.track_wanted.clone() {
            self.library_command(
                tg_player_application::library::LibraryCommand::RequestMedia {
                    track: track.into(),
                    now,
                },
            );
        }
        for id in self.image_wanted.clone() {
            let force = self
                .artwork_path(id)
                .is_some_and(|path| !std::path::Path::new(path).is_file());
            self.library_command(
                tg_player_application::library::LibraryCommand::RequestArtwork { id, now, force },
            );
        }

        if window_present
            && self
                .show_requested
                .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        self.frames += 1;
        if self.options.smoke_test {
            if self.preview_started.elapsed() > Duration::from_secs(2) && !self.preview_captured {
                self.preview_captured = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            for event in ctx.input(|i| i.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = event {
                    let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    let _ = image::save_buffer(
                        self.options
                            .screenshot
                            .clone()
                            .unwrap_or_else(|| "/tmp/tg-player-preview.png".into()),
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
                        self.execute(AppCommand::Quit);
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
                        self.execute(AppCommand::Pause(!self.state.playback().state().paused()));
                    }
                    fastframe_tray::Event::Menu("next") => self.execute(AppCommand::Next),
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
        for command in self
            .media
            .as_ref()
            .map(|m| m.commands())
            .unwrap_or_default()
        {
            match command {
                Command::Next => self.execute(AppCommand::Next),
                Command::PlayPause | Command::Pause | Command::Play => {
                    let paused = match command {
                        Command::Play => false,
                        Command::Pause => true,
                        _ => !self.state.playback().state().paused(),
                    };
                    self.execute(AppCommand::Pause(paused));
                }

                Command::SeekBy(ms) => {
                    let pos = self
                        .player
                        .as_ref()
                        .map(|p| p.snapshot().position)
                        .unwrap_or(0.);
                    self.execute(AppCommand::Seek(pos + ms as f64 / 1000.));
                }
                Command::SetPosition { track_id, position } => {
                    if self
                        .state
                        .view()
                        .session
                        .current()
                        .as_ref()
                        .is_some_and(|t| format!("{}-{}", t.chat, t.message) == track_id)
                    {
                        self.execute(AppCommand::Seek(position.as_secs_f64()));
                    }
                }
                Command::SetVolume(v) => self.execute(AppCommand::Volume((v * 100.) as f32)),
                Command::SetShuffle(v) => self.execute(AppCommand::Shuffle(v)),
                Command::SetRepeat(v) => self.execute(AppCommand::Repeat(
                    crate::adapters::system::repeat_from_system(v),
                )),
                Command::Previous => self.execute(AppCommand::Previous),
                Command::Stop => self.execute(AppCommand::Stop),
                Command::Quit => {
                    self.execute(AppCommand::Quit);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Command::Raise => ctx.send_viewport_cmd(egui::ViewportCommand::Focus),
                _ => {}
            }
        }
        if let Some(media) = &mut self.media {
            media.update(MediaState {
                playback: if self.state.view().session.current().is_none() {
                    Playback::Stopped
                } else if self.state.playback().state().paused()
                    || self.state.playback().state().loading()
                {
                    Playback::Paused
                } else {
                    Playback::Playing
                },
                track: self
                    .state
                    .view()
                    .session
                    .current()
                    .as_ref()
                    .map(|t| MediaTrack {
                        id: format!("{}-{}", t.chat, t.message),
                        title: t.title.clone(),
                        artists: vec![t.artist.clone()],
                        duration: Some(Duration::from_secs(t.duration)),
                        ..Default::default()
                    }),
                position: Duration::from_secs_f64(
                    self.player
                        .as_ref()
                        .map(|p| p.snapshot().position)
                        .unwrap_or(0.)
                        .max(0.),
                ),
                volume: Some(self.state.view().volume as f64 / 100.),
                shuffle: Some(self.state.view().shuffle),
                repeat: Some(crate::adapters::system::repeat_to_system(
                    self.state.view().repeat,
                )),
                ..Default::default()
            });
        }

        ctx.request_repaint_after(Duration::from_millis(200));
        let events = self.player.as_ref().map(|p| p.drain()).unwrap_or_default();
        for event in events {
            self.core_event(tg_player_application::workflow::AppEvent::Player(event));
        }
    }
}

impl App {
    pub fn shutdown_core(&mut self) -> Result<(), String> {
        if self.shutdown_finished {
            return Ok(());
        }
        self.shutdown_finished = true;
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let remaining = || deadline.saturating_duration_since(std::time::Instant::now());
        self.process(&egui::Context::default());
        self.quitting = true;
        self.state.shutdown_operations();
        self.state.cancel_library_operations();
        if let Some(server) = &self.server {
            server.stop();
        }
        let mut errors = Vec::new();
        if let Some(mut player) = self.player.take() {
            if let Err(e) = player.shutdown(remaining()) {
                errors.push(e.to_string());
            }
        }
        if let Some(tg) = &self.tg {
            if let Err(e) = tg.shutdown(remaining()) {
                errors.push(e.to_string());
            }
        }
        if let Some(server) = &mut self.server {
            if let Err(e) = server.shutdown(remaining()) {
                errors.push(e.to_string());
            }
        }
        if let Err(e) = self.tasks.shutdown(remaining()) {
            errors.push(e.to_string());
        }
        self.updates.take();
        while self.process(&egui::Context::default()) > 0 && std::time::Instant::now() < deadline {}
        self.persist();
        if let Some(db) = &mut self.db {
            if let Err(e) = db.flush(remaining()) {
                errors.push(e.to_string());
            }
            if let Err(e) = db.shutdown(remaining()) {
                errors.push(e.to_string());
            }
        }
        if std::time::Instant::now() > deadline {
            errors.push("Shutdown timed out".into());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}
