use crate::*;
impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.process(ctx);

        if let Some(p) = &self.player {
            p.drain();
        }
        self.frames += 1;
        if std::env::args().any(|a| a == "--smoke-test") {
            if self.frames == 5 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            for event in ctx.input(|i| i.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = event {
                    let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    let _ = image::save_buffer(
                        "/tmp/tg-player-preview.png",
                        &bytes,
                        image.width() as u32,
                        image.height() as u32,
                        image::ColorType::Rgba8,
                    );
                }
            }
            if self.frames > 25 {
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
                        self.hidden = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    fastframe_tray::Event::Menu("play") => {
                        self.paused = !self.paused;
                        if let Some(p) = &self.player {
                            p.cmd(&["set", "pause", if self.paused { "yes" } else { "no" }]);
                        }
                    }
                    fastframe_tray::Event::Menu("next") => self.next(),
                    _ => {}
                }
            }
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quitting
            && self
                .tray
                .as_ref()
                .is_some_and(fastframe_tray::Tray::is_shown)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hidden = true;
        }

        use fastframe_now_playing::{Command, Playback, State as MediaState, Track as MediaTrack};
        for command in self.media.commands() {
            match command {
                Command::Next => self.next(),
                Command::PlayPause | Command::Pause | Command::Play => {
                    self.paused = match command {
                        Command::Play => false,
                        Command::Pause => true,
                        _ => !self.paused,
                    };
                    if let Some(p) = &self.player {
                        p.cmd(&["set", "pause", if self.paused { "yes" } else { "no" }]);
                    }
                }
                Command::SeekBy(ms) => {
                    if let Some(p) = &self.player {
                        p.cmd(&["seek", &(ms as f64 / 1000.).to_string(), "relative"]);
                    }
                }
                Command::SetPosition { track_id, position } => {
                    if self
                        .current
                        .as_ref()
                        .is_some_and(|t| format!("{}-{}", t.chat, t.message) == track_id)
                    {
                        if let Some(p) = &self.player {
                            p.cmd(&["seek", &position.as_secs_f64().to_string(), "absolute"]);
                        }
                    }
                }
                Command::SetVolume(v) => {
                    self.volume = (v * 100.) as f32;
                    if let Some(p) = &self.player {
                        p.cmd(&["set", "volume", &self.volume.to_string()]);
                    }
                }
                Command::SetShuffle(v) => self.shuffle = v,
                Command::SetRepeat(v) => self.repeat = v != fastframe_now_playing::Repeat::Off,
                Command::Previous => self.previous(),
                Command::Stop => {
                    if let Some(p) = &self.player {
                        p.cmd(&["stop"]);
                    }
                    self.current = None;
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
            playback: if self.current.is_none() {
                Playback::Stopped
            } else if self.paused {
                Playback::Paused
            } else {
                Playback::Playing
            },
            track: self.current.as_ref().map(|t| MediaTrack {
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
            volume: Some(self.volume as f64 / 100.),
            shuffle: Some(self.shuffle),
            ..Default::default()
        });

        ctx.request_repaint_after(Duration::from_millis(200));
        let eof = self.player.as_ref().is_some_and(|p| p.flag("eof-reached"));
        if !eof
            && self
                .player
                .as_ref()
                .is_some_and(|p| p.number("duration") > 0.)
        {
            self.started = true;
        }
        if eof && self.started && !self.paused {
            self.started = false;
            if self.repeat || !self.state.queue.is_empty() {
                self.next();
            } else {
                self.current = None;
            }
        }
    }
    fn ui(&mut self, root_ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = &root_ui.ctx().clone();
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
        if self.state.light {
            ctx.set_visuals(egui::Visuals::light())
        } else {
            let mut v = egui::Visuals::dark();
            v.override_text_color = Some(egui::Color32::from_rgb(225, 231, 241));
            v.weak_text_color = Some(egui::Color32::from_rgb(145, 158, 181));
            v.extreme_bg_color = egui::Color32::from_rgb(24, 29, 41);
            v.widgets.inactive.bg_fill = egui::Color32::from_rgb(37, 45, 61);
            v.panel_fill = egui::Color32::from_rgb(17, 20, 29);
            v.window_fill = egui::Color32::from_rgb(24, 28, 39);
            v.selection.bg_fill = egui::Color32::from_rgb(45, 118, 125);
            ctx.set_visuals(v)
        }
        egui::Panel::top("header").show(root_ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(
                    egui::RichText::new("◉  telegram / music")
                        .color(egui::Color32::from_rgb(99, 215, 196)),
                );
                ui.separator();
                ui.label(tr("Your music. Your chats."));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("☼").clicked() {
                        self.state.light = !self.state.light;
                        self.persist()
                    }
                    if ui.button(tr("Settings")).clicked() {
                        self.settings_open = !self.settings_open;
                    }
                    if ui.button(tr("Queue")).clicked() {
                        self.queue_open = !self.queue_open
                    }
                    if ui
                        .add_enabled(!self.busy, egui::Button::new(tr("Refresh")))
                        .clicked()
                    {
                        self.sync()
                    }
                });
            });
        });
        egui::Panel::bottom("player")
            .min_size(105.)
            .show(root_ui, |ui| {
                ui.add_space(8.);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.strong(
                            self.current
                                .as_ref()
                                .map(|t| t.title.clone())
                                .unwrap_or_else(|| tr("Choose a track")),
                        );
                        ui.weak(
                            self.current
                                .as_ref()
                                .map(|t| t.artist.clone())
                                .unwrap_or_else(|| tr("Ready to play")),
                        );
                    });
                    ui.separator();
                    if ui.button("⏮").on_hover_text(tr("Previous track")).clicked() {
                        self.previous();
                    }
                    if ui.button(if self.paused { "▶" } else { "Ⅱ" }).clicked() {
                        self.paused = !self.paused;
                        if let Some(p) = &self.player {
                            p.cmd(&["set", "pause", if self.paused { "yes" } else { "no" }]);
                        }
                    }
                    if ui.button(tr("Next ⏭")).clicked() {
                        self.next()
                    }
                    ui.toggle_value(&mut self.shuffle, tr("Shuffle"));
                    ui.toggle_value(&mut self.repeat, tr("Repeat"));
                    if ui
                        .add(egui::Slider::new(&mut self.volume, 0.0..=100.).text(tr("Volume")))
                        .changed()
                    {
                        if let Some(p) = &self.player {
                            p.cmd(&["set", "volume", &self.volume.to_string()]);
                        }
                    }
                });
                let duration = self
                    .player
                    .as_ref()
                    .map(|p| p.number("duration"))
                    .unwrap_or(0.);
                let mut position = self
                    .player
                    .as_ref()
                    .map(|p| p.number("time-pos"))
                    .unwrap_or(0.);
                if duration > 0.
                    && ui
                        .add(egui::Slider::new(&mut position, 0.0..=duration).text(tr("seconds")))
                        .drag_stopped()
                {
                    if let Some(p) = &self.player {
                        p.cmd(&["seek", &position.to_string(), "absolute"]);
                    }
                }
                ui.weak(&self.status);
                if !self.last_link.is_empty() {
                    ui.hyperlink_to(tr("Open in Telegram"), &self.last_link);
                }
            });
        egui::Panel::left("library")
            .default_size(245.)
            .show(root_ui, |ui| {
                ui.add_space(16.);
                ui.heading(tr("Library"));
                ui.label(
                    egui::RichText::new(tr("PLAYLISTS FROM TELEGRAM"))
                        .small()
                        .weak(),
                );
                ui.add_space(12.);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, p) in self.state.playlists.iter().enumerate() {
                        if p.tracks.iter().any(|t| p.filters[t.kind])
                            && ui
                                .selectable_label(
                                    self.selected == i,
                                    format!(
                                        "♫  {}\n     {}",
                                        p.title,
                                        i18n::track_count(p.tracks.len())
                                    ),
                                )
                                .clicked()
                        {
                            self.selected = i
                        }
                    }
                });
                ui.separator();
                ui.weak(tr("Read only · no online presence"));
            });
        if self.queue_open {
            egui::Panel::right("queue")
                .default_size(240.)
                .show(root_ui, |ui| {
                    ui.add_space(16.);
                    ui.heading(tr("Up next"));
                    let mut play = None;
                    let mut remove = None;
                    let mut up = None;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (i, t) in self.state.queue.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.small_button("▶").clicked() {
                                    play = Some(i)
                                }
                                ui.vertical(|ui| {
                                    ui.label(&t.title);
                                    ui.weak(&t.artist);
                                });
                                if i > 0 && ui.small_button("↑").clicked() {
                                    up = Some(i)
                                }
                                if ui.small_button("×").clicked() {
                                    remove = Some(i)
                                }
                            });
                            ui.separator();
                        }
                    });
                    if let Some(i) = play {
                        let t = self.state.queue.remove(i);
                        self.play(t)
                    }
                    if let Some(i) = remove {
                        self.state.queue.remove(i);
                    }
                    if let Some(i) = up {
                        self.state.queue.swap(i, i - 1);
                    }
                });
        }
        self.settings_ui(ctx);
        egui::CentralPanel::default().show(root_ui, |ui| {
            self.auth_ui(ui);
            self.library_ui(ui);
        });
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
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
impl App {
    fn auth_ui(&mut self, ui: &mut egui::Ui) {
        if self.demo {
            return;
        }
        if self.tg.is_none() {
            ui.heading(tr("Connect Telegram"));
            ui.label(tr(
                "An independent session. Listening does not announce you online.",
            ));
            ui.hyperlink_to(
                tr("Get an API ID and API hash"),
                "https://my.telegram.org/apps",
            );
            ui.label(tr("API ID"));
            ui.text_edit_singleline(&mut self.state.api_id);
            ui.label(tr("API hash"));
            ui.add(egui::TextEdit::singleline(&mut self.state.api_hash).password(true));
            if ui
                .add_enabled(
                    self.state.api_id.parse::<i32>().is_ok() && !self.state.api_hash.is_empty(),
                    egui::Button::new(tr("Connect")),
                )
                .clicked()
            {
                self.connect();
            }
            ui.label(tr("Your saved library remains available offline."));
            ui.separator();
            return;
        }
        let state = self.auth["@type"].as_str().unwrap_or("").to_owned();
        if state == "authorizationStateReady" {
            return;
        }
        ui.heading(tr("Sign in"));
        match state.as_str() {
            "authorizationStateWaitPhoneNumber"
            | "authorizationStateWaitOtherDeviceConfirmation" => {
                ui.label(tr(
                    "Scan this code in Telegram → Settings → Devices → Link Desktop Device.",
                ));
                if ui.button(tr("Show QR code")).clicked() {
                    self.send(json!({"@type":"requestQrCodeAuthentication","other_user_ids":[]}));
                }
                if let Ok(code) = qrcode::QrCode::new(self.qr.as_bytes()) {
                    if !self.qr.is_empty() {
                        let width = code.width();
                        let cell = 4.;
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2((width + 8) as f32 * cell, (width + 8) as f32 * cell),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(rect, 0., egui::Color32::WHITE);
                        for y in 0..width {
                            for x in 0..width {
                                if code[(x, y)] == qrcode::Color::Dark {
                                    ui.painter().rect_filled(
                                        egui::Rect::from_min_size(
                                            rect.min
                                                + egui::vec2(
                                                    (x + 4) as f32 * cell,
                                                    (y + 4) as f32 * cell,
                                                ),
                                            egui::vec2(cell, cell),
                                        ),
                                        0.,
                                        egui::Color32::BLACK,
                                    );
                                }
                            }
                        }
                    }
                }
                ui.horizontal(|ui| { ui.add(egui::TextEdit::singleline(&mut self.phone).hint_text(tr("Phone number with country code")));
                    if ui.button(tr("Sign in with phone")).clicked() {self.send(json!({"@type":"setAuthenticationPhoneNumber","phone_number":self.phone,"settings":null}));}
                });
            }
            "authorizationStateWaitCode" | "authorizationStateWaitEmailCode" => {
                ui.label(tr("Enter the verification code sent by Telegram."));
                ui.text_edit_singleline(&mut self.code);
                if ui.button(tr("Submit code")).clicked() {
                    self.send(if state == "authorizationStateWaitEmailCode" {
                        json!({
                        "@type":"checkAuthenticationEmailCode","code":{
                        "@type":"emailAddressAuthenticationCode","code":self.code}
                        }
                        )
                    } else {
                        json!({
                        "@type":"checkAuthenticationCode","code":self.code}
                        )
                    });

                    self.code.clear();
                }
                if state == "authorizationStateWaitCode"
                    && self.auth["code_info"]["next_type"].is_object()
                    && ui.button(tr("Resend code")).clicked()
                {
                    self.send(json!({"@type":"resendAuthenticationCode","reason":null}));
                }
            }
            "authorizationStateWaitPassword" => {
                ui.label(tr("Two-step verification password"));
                ui.weak(self.auth["password_hint"].as_str().unwrap_or(""));
                ui.add(egui::TextEdit::singleline(&mut self.password).password(true));
                if ui.button(tr("Submit password")).clicked() {
                    self.send(
                        json!({"@type":"checkAuthenticationPassword","password":self.password}),
                    );
                    self.password.clear();
                }
            }
            "authorizationStateWaitEmailAddress" => {
                ui.label(tr("Telegram requires an email address to continue."));
                ui.text_edit_singleline(&mut self.phone);
                if ui.button(tr("Submit email address")).clicked() {
                    self.send(
                        json!({"@type":"setAuthenticationEmailAddress","email_address":self.phone}),
                    );
                }
            }
            "authorizationStateWaitRegistration" => {
                ui.label(tr(
                    "Create your Telegram account in the official app, then sign in here.",
                ));
            }
            "authorizationStateClosed" => {
                ui.label(tr("Session closed. Restart to connect again."));
            }
            _ => {
                ui.label(tr("Connecting to Telegram…"));
            }
        }
        ui.separator();
    }
    fn library_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(20.);
        let Some(p) = self.state.playlists.get_mut(self.selected) else {
            ui.heading(tr("Your music will appear here"));
            ui.weak(tr("Chats without matching audio are hidden automatically."));
            return;
        };
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(110., 110.), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, 16., egui::Color32::from_rgb(35, 113, 121));
            ui.painter()
                .circle_filled(rect.center(), 33., egui::Color32::from_rgb(22, 65, 79));
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "♫",
                egui::FontId::proportional(38.),
                egui::Color32::from_rgb(147, 236, 215),
            );
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(tr("TELEGRAM PLAYLIST"))
                        .small()
                        .color(egui::Color32::from_rgb(99, 215, 196)),
                );
                ui.heading(egui::RichText::new(&p.title).size(32.));
                ui.weak(
                    tr("{tracks} · read only")
                        .replace("{tracks}", &i18n::track_count(p.tracks.len())),
                );
            });
        });
        ui.add_space(16.);
        ui.horizontal(|ui| {
            for (i, name) in [tr("Music"), tr("Audio files"), tr("Voice messages")]
                .iter()
                .enumerate()
            {
                ui.toggle_value(&mut p.filters[i], name.as_str());
            }
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text(tr("Search titles, artists and filenames…"))
                .desired_width(f32::INFINITY),
        );
        let search = self.search.to_lowercase();
        let mut tracks: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| {
                p.filters[t.kind]
                    && format!("{} {}", t.title, t.artist)
                        .to_lowercase()
                        .contains(&search)
            })
            .cloned()
            .collect();
        match self.sort {
            1 => tracks.sort_by(|a, b| a.title.cmp(&b.title)),
            2 => tracks.sort_by(|a, b| a.artist.cmp(&b.artist)),
            _ => {}
        }
        let mut action = None;
        ui.horizontal(|ui| {
            if !p.url.is_empty() {
                ui.hyperlink_to(tr("Open source chat"), &p.url);
                if ui.button(tr("Copy chat link")).clicked() {
                    ui.ctx().copy_text(p.url.clone());
                }
            }
            if ui.button(tr("▶ Play all")).clicked() {
                action = Some((0, 0));
            }
            if ui.button(tr("↓ Save playlist")).clicked() {
                p.pinned = true;
                action = Some((1, 0));
            }
            egui::ComboBox::from_id_salt("sort")
                .selected_text(tr(["Newest first", "Title", "Artist"][self.sort]))
                .show_ui(ui, |ui| {
                    for (i, label) in ["Newest first", "Title", "Artist"].iter().enumerate() {
                        ui.selectable_value(&mut self.sort, i, tr(label));
                    }
                });
        });
        ui.add_space(12.);
        egui::ScrollArea::vertical().show_rows(ui, 52., tracks.len(), |ui, range| {
            for i in range {
                let t = &tracks[i];
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("{:02}", i + 1)).weak());
                    ui.vertical(|ui| {
                        if ui
                            .selectable_label(
                                self.current
                                    .as_ref()
                                    .is_some_and(|x| x.chat == t.chat && x.message == t.message),
                                &t.title,
                            )
                            .double_clicked()
                        {
                            action = Some((2, i));
                        }
                        ui.weak(&t.artist);
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("↗")
                            .on_hover_text(tr("Copy message link"))
                            .clicked()
                        {
                            action = Some((3, i));
                        }
                        if ui
                            .small_button(if t.pinned.is_some() { "✓" } else { "↓" })
                            .on_hover_text(tr("Save for offline listening"))
                            .clicked()
                        {
                            action = Some((4, i));
                        }
                        if ui
                            .small_button("+")
                            .on_hover_text(tr("Add to queue"))
                            .clicked()
                        {
                            action = Some((5, i));
                        }
                        if ui
                            .small_button("⤒")
                            .on_hover_text(tr("Play next"))
                            .clicked()
                        {
                            action = Some((6, i));
                        }
                        ui.weak(format!("{}:{:02}", t.duration / 60, t.duration % 60));
                    });
                });
                ui.separator();
            }
        });
        if let Some((kind, i)) = action {
            match kind {
                0 => {
                    self.state.queue = tracks.iter().skip(1).cloned().collect();
                    if let Some(t) = tracks.first() {
                        self.play(t.clone());
                    }
                }
                1 => {
                    for t in tracks {
                        self.pin(t);
                    }
                }
                2 => {
                    self.state.queue = tracks.iter().skip(i + 1).cloned().collect();
                    self.play(tracks[i].clone());
                }
                3 => self.link(&tracks[i]),
                4 => self.pin(tracks[i].clone()),
                6 => self.state.queue.insert(0, tracks[i].clone()),
                _ => self.state.queue.push(tracks[i].clone()),
            }
        }
    }
}

impl App {
    fn settings_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new(tr("Settings")).open(&mut open).show(ctx,|ui|{
            ui.heading(tr("Default audio filters"));
            for(i,label)in [tr("Music"),tr("Audio files"),tr("Voice messages")].iter().enumerate(){ui.checkbox(&mut self.state.filters[i],label.as_str());}
            if ui.button(tr("Apply defaults to all playlists")).clicked(){for p in &mut self.state.playlists{p.filters=self.state.filters;}self.persist();}
            ui.separator();ui.label(tr("Automatic cache: 2 GiB. Offline copies are kept separately."));
            if ui.button(tr("Clear automatic cache")).clicked(){
self.send(json!({
"@type":"optimizeStorage","size":0,"ttl":0,"count":0,"immunity_delay":0,"file_types":[],"chat_ids":[],"exclude_chat_ids":[],"return_deleted_file_statistics":true,"chat_limit":0}
));
}

            if !self.state.pending_pins.is_empty() && ui.button(tr("Cancel offline downloads")).clicked(){
                self.pin_cancel.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
                for t in &self.state.pending_pins{self.send(json!({"@type":"cancelDownloadFile","file_id":t.file,"only_if_pending":true}));}
                self.state.pending_pins.clear();self.persist();
            }
            ui.heading(tr("Offline library"));
            let mut remove=None;
            egui::ScrollArea::vertical().max_height(220.).show(ui,|ui|{
                for p in &self.state.playlists{
for t in &p.tracks{
if let Some(path)=&t.pinned{
ui.horizontal(|ui|{
ui.label(&t.title);
if ui.button(tr("Remove offline copy")).clicked(){
remove=Some((t.chat,t.message,path.clone()));
}
}
);
}
}
}

            });
            if let Some((chat,message,path))=remove{
if std::path::Path::new(&path).starts_with(self.root.join("offline")){
let _=std::fs::remove_file(path);
for p in &mut self.state.playlists{
if p.id==chat{
p.pinned=false;
for t in &mut p.tracks{
if t.message==message{
t.pinned=None;
}
}
}
}
self.persist();
}
}

            ui.separator();ui.label(tr("Online presence is always disabled by this player."));
        });
        self.settings_open = open;
    }
}
