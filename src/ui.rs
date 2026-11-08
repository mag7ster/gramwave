use crate::*;
impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.background(ctx, true);
    }
    fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
        self.render(root);
    }
}
impl App {}
impl App {
    pub(crate) fn settings_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new(tr("Settings")).open(&mut open).show(ctx,|ui|{
            ui.checkbox(&mut self.state.light,tr("Light appearance"));
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
                for t in self.state.pending_pins.iter().filter(|t| t.file > 0){self.send(json!({"@type":"cancelDownloadFile","file_id":t.file,"only_if_pending":true}));}
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
                        "/tmp/tg-player-preview.png",
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
                    self.playback_generation += 1;
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
        let events = self.player.as_ref().map(|p| p.drain()).unwrap_or_default();
        for event in events {
            match event {
                audio::PlaybackEvent::Finished if self.current.is_some() => {
                    if self.repeat || !self.state.queue.is_empty() {
                        self.next();
                    } else {
                        self.current = None;
                    }
                }
                audio::PlaybackEvent::Failed(error) => {
                    self.status = format!("{}: {error}", tr("Unable to play track"));
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
