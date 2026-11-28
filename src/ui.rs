use crate::adapters::tray::{Event as TrayEvent, Tray};
use crate::application::commands::AppCommand;
use crate::{design, tr, App, RepeatMode};
use eframe::egui;
use std::time::{Duration, Instant};

pub(crate) struct TrayFocus {
    deadline: Instant,
    hide_if_denied: bool,
}

pub(crate) struct InlineNotice {
    text: String,
    until: Instant,
    offline: bool,
}
impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        #[cfg(target_os = "linux")]
        if let Some(tray) = &self.tray {
            tray.dispatch_window_events();
        }
        if let Some(delay) = self.background(ctx, true) {
            ctx.request_repaint_after(delay);
        }
    }
    fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
        self.render(root);
    }
}

impl App {
    pub(crate) fn restored_position(
        &self,
        snapshot: crate::application::ports::PlayerSnapshot,
    ) -> Option<f64> {
        use gramwave_application::playback::PlaybackState;
        let position_ms = self.state.view().session.position_ms();
        let loaded = matches!(
            self.state.playback().state(),
            PlaybackState::Playing | PlaybackState::Paused
        ) && (snapshot.duration > 0. || snapshot.checkpoint_position.is_some());
        (position_ms > 0 && !loaded).then_some(position_ms as f64 / 1000.)
    }

    pub(crate) fn process_player_events(&mut self) {
        use gramwave_application::ports::PlaybackEvent;
        let events = self.player.as_ref().map(|p| p.drain()).unwrap_or_default();
        let generation = self.state.playback().generation();
        for event in events {
            if event == PlaybackEvent::Loaded {
                if let Some((id, path)) = self.local_playback.take() {
                    self.core_event(
                        gramwave_application::workflow::AppEvent::LocalPlaybackUsed { id, path },
                    );
                }
            }
            self.core_event(gramwave_application::workflow::AppEvent::Player(event));
            if self.state.playback().generation() != generation {
                break;
            }
        }
    }

    pub(crate) fn set_inline_notice(&mut self, text: String, offline: bool) {
        self.inline_notice = Some(InlineNotice {
            text,
            until: Instant::now() + Duration::from_secs(5),
            offline,
        });
    }

    pub(crate) fn inline_notice(&self, ctx: &egui::Context, offline_only: bool) -> Option<&str> {
        let notice = self.inline_notice.as_ref()?;
        let delay = notice.until.checked_duration_since(Instant::now())?;
        if offline_only && !notice.offline {
            return None;
        }
        ctx.request_repaint_after(delay);
        Some(&notice.text)
    }

    pub(crate) fn settings_ui(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let p = design::palette(self.state.view().light);
        let viewport = ctx.content_rect();
        let compact = viewport.height() < 300. || viewport.width() < 400.;
        let margin = if compact { 8. } else { 24. };
        let width = (viewport.width() - margin * 2. - 16.).clamp(100., 520.);
        let height = (viewport.height() - margin * 2. - 12.).clamp(54., 720.);
        let mut close = false;
        let mut changed = false;
        let mut light = self.state.view().light;
        let mut filters = self.state.view().filters;
        let mut cache_policy = self.state.view().cache_policy;
        let response = egui::Modal::new(egui::Id::new("player-settings"))
            .frame(
                egui::Frame::new()
                    .fill(p.panel)
                    .corner_radius(16)
                    .inner_margin(margin),
            )
            .show(ctx, |ui| {
                crate::screens::reveal_layout(ui, "settings-layout", compact);
                ui.set_width(width);
                ui.set_height(height);
                ui.spacing_mut().scroll.floating = false;
                ui.spacing_mut().scroll.bar_width = 4.;
                ui.horizontal(|ui| {
                    ui.heading(
                        egui::RichText::new(tr("Settings"))
                            .size(if compact { 18. } else { 26. })
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = design::button(ui, design::Icon::Close, "Close settings");
                    });
                });
                ui.add_space(if compact { 2. } else { 12. });
                egui::ScrollArea::vertical()
                    .id_salt("settings-content")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .max_height(ui.available_height().max(24.))
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
                        self.startup_settings_ui(ui);
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
                        if self.cache_settings_ui(ui, &mut cache_policy) {
                            changed = true;
                            self.cache_policy_changed();
                        }
                        self.offline_settings_ui(ui);
                        ui.add_space(4.);
                        ui.small(
                            egui::RichText::new(tr(
                                "Online presence is always disabled by this player.",
                            ))
                            .weak(),
                        );
                    });
            });
        if changed {
            let mut settings = gramwave_application::persistence::Settings::from(self.state.view());
            settings.light = light;
            settings.filters = filters;
            settings.cache_policy = cache_policy;
            self.execute(AppCommand::Settings(settings));
            self.persist();
        }
        if self.offline_confirm.is_none() && (close || response.should_close()) {
            self.settings_open = false;
            self.cache_settings_closed();
            self.offline_selected.clear();
        }
    }

    fn offline_settings_ui(&mut self, ui: &mut egui::Ui) {
        let offline = self.state.offline();
        let mut playlists = std::collections::BTreeMap::<i64, (usize, u64, bool)>::new();
        for copy in offline.copies.values().filter(|copy| !copy.cached) {
            let usage = playlists.entry(copy.track.chat).or_default();
            usage.0 += 1;
            usage.1 += copy.size.max(
                self.cache
                    .offline_sizes
                    .get(&copy.path)
                    .copied()
                    .unwrap_or(0),
            );
        }
        for track in offline.pending.values() {
            playlists.entry(track.chat).or_default().2 = true;
        }
        for policy in offline
            .policies
            .values()
            .filter(|p| p.enabled && p.filters.iter().any(|f| *f))
        {
            playlists.entry(policy.chat).or_default().2 |= policy
                .done
                .iter()
                .zip(&policy.head)
                .any(|(done, head)| !done || head.active);
        }
        self.offline_selected
            .retain(|chat| playlists.contains_key(chat));
        design::settings_section(ui, tr("Offline storage"), |ui| {
            if self.options.scenario.as_deref() == Some("offline-storage")
                && self.options.screenshot.is_some()
                && !self.preview_captured
            {
                ui.scroll_to_cursor(Some(egui::Align::Min));
            }
            if let Some(notice) = self.inline_notice(ui.ctx(), true) {
                ui.weak(notice);
            }
            if playlists.is_empty() {
                ui.weak(tr("No offline tracks saved yet."));
                return;
            }
            if playlists.values().any(|(_, _, downloading)| *downloading)
                && ui.button(tr("Cancel offline downloads")).clicked()
            {
                self.execute(AppCommand::CancelOffline);
            }
            for (&chat, &(count, size, downloading)) in &playlists {
                let title = self.offline_playlist_title(chat);
                ui.push_id(chat, |ui| {
                    ui.horizontal(|ui| {
                        let mut selected = self.offline_selected.contains(&chat);
                        let checkbox = ui
                            .scope(|ui| {
                                let colors = design::colors(ui);
                                let widgets = &mut ui.visuals_mut().widgets;
                                widgets.inactive.bg_stroke = egui::Stroke::new(1., colors.muted);
                                widgets.inactive.bg_fill = colors.panel;
                                widgets.inactive.corner_radius = 4.into();
                                widgets.hovered.corner_radius = 4.into();
                                widgets.active.corner_radius = 4.into();
                                ui.checkbox(&mut selected, "")
                            })
                            .inner;
                        checkbox.labelled_by(
                            ui.vertical(|ui| {
                                let title_response = ui
                                    .add(egui::Label::new(&title).truncate())
                                    .on_hover_text(&title);
                                let mut status = crate::i18n::ngettext(
                                    "{count} saved track",
                                    "{count} saved tracks",
                                    count.try_into().unwrap_or(u32::MAX),
                                )
                                .replace("{count}", &count.to_string());
                                if size != 0 {
                                    status.push_str(&format!(
                                        " · {}",
                                        crate::application::cache::cache_size(size)
                                    ));
                                }
                                if downloading {
                                    status.push_str(&format!(" · {}", tr("Downloading…")));
                                }
                                ui.small(status);
                                title_response.id
                            })
                            .inner,
                        );
                        if selected {
                            self.offline_selected.insert(chat);
                        } else {
                            self.offline_selected.remove(&chat);
                        }
                    });
                });
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !self.offline_selected.is_empty(),
                        egui::Button::new(tr("Remove selected downloads")),
                    )
                    .clicked()
                {
                    self.offline_confirm = Some(self.offline_selected.iter().copied().collect());
                }
                if ui.button(tr("Remove all downloads")).clicked() {
                    self.offline_confirm = Some(playlists.keys().copied().collect());
                }
            });
        });
    }

    fn offline_playlist_title(&self, chat: i64) -> String {
        self.state
            .view()
            .playlists
            .iter()
            .find(|playlist| playlist.id == chat)
            .map(|playlist| playlist.title.clone())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| tr("Chat {id}").replace("{id}", &chat.to_string()))
    }

    pub(crate) fn offline_confirmation_ui(&mut self, ctx: &egui::Context) {
        let Some(chats) = self.offline_confirm.as_ref() else {
            return;
        };
        let message = if let [chat] = chats.as_slice() {
            tr("Remove downloads for “{playlist}”?")
                .replace("{playlist}", &self.offline_playlist_title(*chat))
        } else {
            tr("Remove downloads for {count} playlists?")
                .replace("{count}", &chats.len().to_string())
        };
        let result = egui::Modal::new(egui::Id::new("confirm-offline-removal")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 32.).clamp(100., 360.));
            ui.heading(tr("Remove offline downloads?"));
            ui.label(message);
            ui.weak(tr("Recent tracks may stay in the automatic cache."));
            ui.horizontal_wrapped(|ui| {
                let cancel = ui.button(tr("Cancel"));
                if ctx.data_mut(|data| data.get_temp::<bool>(ui.id().with("focused")).is_none()) {
                    cancel.request_focus();
                    ctx.data_mut(|data| data.insert_temp(ui.id().with("focused"), true));
                }
                (
                    cancel.clicked(),
                    ui.button(tr("Remove downloads")).clicked(),
                    ui.id().with("focused"),
                )
            })
            .inner
        });
        let (cancel, remove, focus) = result.inner;
        if remove || cancel || result.should_close() {
            ctx.data_mut(|data| data.remove::<bool>(focus));
            let chats = self.offline_confirm.take().unwrap_or_default();
            if remove {
                self.offline_selected.retain(|chat| !chats.contains(chat));
                self.execute(AppCommand::RemoveOfflinePlaylists(chats));
            }
        }
    }
}

impl App {
    fn show_window(&self, ctx: &egui::Context, window_present: bool) {
        if self.options.headless {
            return;
        }
        if !window_present {
            self.show_requested
                .store(true, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn window_focused(ctx: &egui::Context) -> bool {
        ctx.input(|i| {
            i.viewport().focused.unwrap_or(i.focused) && i.viewport().minimized != Some(true)
        })
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        self.tray_focus = None;
        self.hidden = true;
        if let Some(tray) = &self.tray {
            tray.set_window(None);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn request_tray_focus(
        &mut self,
        ctx: &egui::Context,
        token: Option<String>,
        hide_if_denied: bool,
    ) {
        if let Some(pending) = &mut self.tray_focus {
            pending.hide_if_denied &= hide_if_denied;
            return;
        }
        self.show_window(ctx, true);
        if Self::window_focused(ctx) {
            return;
        }
        #[cfg(target_os = "linux")]
        if self.wayland
            && !self
                .tray
                .as_ref()
                .is_some_and(|tray| tray.activate_window(token.as_deref()))
        {
            if hide_if_denied {
                self.hide_window(ctx);
            }
            return;
        }
        self.await_tray_focus(Instant::now(), hide_if_denied);
    }

    fn await_tray_focus(&mut self, sent_at: Instant, hide_if_denied: bool) {
        let deadline = sent_at + Duration::from_millis(400);
        match &mut self.tray_focus {
            Some(pending) => {
                pending.deadline = pending.deadline.max(deadline);
                pending.hide_if_denied &= hide_if_denied;
            }
            None => {
                self.tray_focus = Some(TrayFocus {
                    deadline,
                    hide_if_denied,
                })
            }
        }
    }
    fn settle_tray_focus(&mut self, ctx: &egui::Context, window_present: bool) -> Option<Instant> {
        let pending = self.tray_focus.as_ref()?;
        if !window_present
            || self.quitting
            || self.hidden
            || Self::window_focused(ctx)
            || ctx.input(|i| i.viewport().close_requested())
        {
            self.tray_focus = None;
            return None;
        }
        if Instant::now() >= pending.deadline {
            let hide = pending.hide_if_denied;
            self.tray_focus = None;
            if hide {
                self.hide_window(ctx);
            }
            return None;
        }
        Some(pending.deadline)
    }

    fn handle_tray_event(&mut self, ctx: &egui::Context, window_present: bool, event: TrayEvent) {
        if self.quitting {
            return;
        }
        match event {
            TrayEvent::FocusRequested {
                window,
                sent_at,
                hide_if_denied,
            } => {
                if window_present
                    && !self.hidden
                    && self
                        .tray
                        .as_ref()
                        .is_some_and(|tray| tray.targets_window(&window))
                {
                    self.await_tray_focus(sent_at, hide_if_denied);
                }
            }
            TrayEvent::Toggle(_) if window_present && !self.hidden && Self::window_focused(ctx) => {
                self.hide_window(ctx);
            }
            TrayEvent::Toggle(token) if window_present && !self.hidden => {
                self.request_tray_focus(ctx, token, true);
            }
            TrayEvent::Show(token) if window_present && !self.hidden => {
                self.request_tray_focus(ctx, token, false);
            }
            TrayEvent::Toggle(token) | TrayEvent::Show(token) => {
                self.reopen_token = token;
                self.show_window(ctx, false);
            }
            TrayEvent::Quit => {
                self.execute(AppCommand::Quit);
                if window_present {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            TrayEvent::PlayPause => {
                self.execute(AppCommand::Pause(!self.state.playback().state().paused()));
            }
            TrayEvent::Next => self.execute(AppCommand::Next),
        }
    }

    pub fn background(&mut self, ctx: &egui::Context, window_present: bool) -> Option<Duration> {
        self.process(ctx);
        self.process_player_events();
        self.process_control();
        if self.quitting {
            if window_present {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return None;
        }
        let now = std::time::Instant::now();
        if window_present
            && !self.demo
            && matches!(
                self.state.auth().state(),
                gramwave_application::telegram::AuthorizationState::Starting
                    | gramwave_application::telegram::AuthorizationState::Parameters
            )
        {
            self.warm_initial_covers(ctx);
        }
        let cache = self.maintain_cache(now);
        if !window_present || self.hidden {
            self.state.finish_page_browse_frame();
        }
        let offline = self.pump_offline(now);

        if window_present
            && !self.hidden
            && !self.quitting
            && self
                .show_requested
                .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            self.show_window(ctx, window_present);
        }

        self.frames += 1;
        let mut smoke = None;
        if self.options.smoke_test && !self.options.headless {
            let screenshot_at = self.preview_started + Duration::from_secs(2);
            let quit_at = self.preview_started + Duration::from_millis(3500);
            if now >= screenshot_at && !self.preview_captured {
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
                            .unwrap_or_else(|| "/tmp/gramwave-preview.png".into()),
                        &bytes,
                        image.width() as u32,
                        image.height() as u32,
                        image::ColorType::Rgba8,
                    );
                }
            }
            if now >= quit_at {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                smoke = Some(if self.preview_captured {
                    quit_at
                } else {
                    screenshot_at
                });
            }
        }
        if let Some(tray) = &self.tray {
            for event in tray.events() {
                self.handle_tray_event(ctx, window_present, event);
            }
        }
        if window_present
            && ctx.input(|i| i.viewport().close_requested())
            && !self.quitting
            && self.tray.as_ref().is_some_and(Tray::is_shown)
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
                Command::Raise => self.show_window(ctx, window_present),
                _ => {}
            }
        }
        self.process_player_events();

        let mut mpris = None;
        let snapshot = self
            .player
            .as_ref()
            .map(|p| p.snapshot())
            .unwrap_or_default();
        let position = self
            .restored_position(snapshot)
            .unwrap_or(snapshot.position);
        if let Some(media) = &mut self.media {
            let playback = if self.state.view().session.current().is_none() {
                Playback::Stopped
            } else if self.state.playback().state().paused()
                || self.state.playback().state().loading()
            {
                Playback::Paused
            } else {
                Playback::Playing
            };
            if playback == Playback::Playing && self.player.is_some() {
                mpris = Some(now + Duration::from_millis(200));
            }
            media.update(MediaState {
                playback,
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
                position: Duration::try_from_secs_f64(position).unwrap_or_default(),
                volume: Some(self.state.view().volume as f64 / 100.),
                shuffle: Some(self.state.view().shuffle),
                repeat: Some(crate::adapters::system::repeat_to_system(
                    self.state.view().repeat,
                )),
                ..Default::default()
            });
        }

        offline
            .into_iter()
            .chain(cache)
            .chain(smoke)
            .chain(mpris)
            .chain(self.settle_tray_focus(ctx, window_present))
            .min()
            .map(|at| at.saturating_duration_since(std::time::Instant::now()))
    }
}

impl App {
    pub fn shutdown_core(&mut self) -> Result<(), String> {
        if self.shutdown_finished {
            return Ok(());
        }
        self.shutdown_finished = true;
        self.covers.get_mut().save();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let remaining = || deadline.saturating_duration_since(std::time::Instant::now());
        self.process(&egui::Context::default());
        self.quitting = true;
        self.process_player_events();
        if let Some(player) = &self.player {
            self.state.save_playback_position(player.snapshot());
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui::ViewportCommand as Window;
    use fastframe_shell::Resident;

    #[test]
    fn tray_toggle_opens_raises_and_hides_without_quitting() {
        let mut app = App::new_at(std::path::PathBuf::new(), true);
        let dispatch = |app: &mut App, present, focused, minimized, event| {
            let ctx = egui::Context::default();
            let mut input = egui::RawInput {
                focused,
                ..Default::default()
            };
            let viewport = input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
            viewport.focused = Some(focused);
            viewport.minimized = Some(minimized);
            let mut event = Some(event);
            let mut output = ctx.run_ui(input, |ui| {
                if let Some(event) = event.take() {
                    app.handle_tray_event(ui.ctx(), present, event);
                }
            });
            output.textures_delta.clear();
            output
                .viewport_output
                .remove(&egui::ViewportId::ROOT)
                .unwrap()
                .commands
        };
        let commands = dispatch(&mut app, false, false, false, TrayEvent::Toggle(None));
        assert!(!commands.contains(&Window::Focus));
        assert!(!commands.contains(&Window::Close));
        assert!(app
            .show_requested
            .swap(false, std::sync::atomic::Ordering::Relaxed));

        for (focused, minimized) in [(false, false), (true, true)] {
            let commands = dispatch(&mut app, true, focused, minimized, TrayEvent::Toggle(None));
            assert!(commands.contains(&Window::Minimized(false)));
            assert!(commands.contains(&Window::Focus));
            assert!(!commands.contains(&Window::Close));
            assert!(!app.hidden);
            app.tray_focus = None;
        }
        let commands = dispatch(&mut app, true, true, false, TrayEvent::Show(None));
        assert!(commands.contains(&Window::Focus));
        assert!(!commands.contains(&Window::Close));

        let commands = dispatch(&mut app, true, true, false, TrayEvent::Toggle(None));
        assert!(commands.contains(&Window::Close));
        assert_eq!(app.closed(), fastframe_shell::Closed::Hide);
        app.window_gone();
        dispatch(
            &mut app,
            false,
            false,
            false,
            TrayEvent::Toggle(Some("token".into())),
        );
        assert_eq!(
            app.headless_frame(&egui::Context::default()),
            fastframe_shell::Headless::Show
        );
        assert_eq!(app.reopen_token.take().as_deref(), Some("token"));
        assert!(app.reopen_token.take().is_none());
    }

    #[test]
    fn tray_toggle_without_a_wayland_activation_path_hides_immediately() {
        let mut app = App::new_at(std::path::PathBuf::new(), true);
        app.wayland = true;
        let ctx = egui::Context::default();
        let input = || egui::RawInput {
            focused: false,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input(), |ui| {
            app.handle_tray_event(ui.ctx(), true, TrayEvent::Show(None));
            assert!(!app.hidden);
            app.handle_tray_event(ui.ctx(), true, TrayEvent::Toggle(None));
        });
        output.textures_delta.clear();
        assert!(app.hidden);
        assert!(app.tray_focus.is_none());
        assert!(output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&Window::Close));
    }

    #[test]
    fn tray_focus_timeout_only_hides_the_same_unfocused_window_after_a_toggle() {
        let mut app = App::new_at(std::path::PathBuf::new(), true);
        for (present, focused, hide_if_denied, should_hide) in [
            (true, false, true, true),
            (true, true, true, false),
            (true, false, false, false),
            (false, false, true, false),
        ] {
            app.hidden = false;
            app.await_tray_focus(Instant::now() - Duration::from_secs(1), hide_if_denied);
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    focused,
                    ..Default::default()
                },
                |ui| {
                    assert!(app.settle_tray_focus(ui.ctx(), present).is_none());
                },
            );
            output.textures_delta.clear();
            assert_eq!(app.hidden, should_hide);
            assert!(app.tray_focus.is_none());
        }
    }
}
