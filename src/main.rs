#[cfg(test)]
mod artwork_tests;
mod auth;
mod design;
mod discovery;
mod i18n;
mod lifecycle;
mod screens;
mod storage;
mod ui;
use i18n::tr;
mod audio;
mod model;
mod telegram;
use eframe::egui;
use model::{Playlist, State, Track};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::mpsc, time::Duration};
enum Event {
    Progress(String),
    Artwork(i64, Result<String, String>),
    Playlist(u64, Playlist),
    Catalog(Result<Vec<Playlist>, String>),
    Indexed(u64, Result<(), String>),
    Error(String),
    AuthResult(u64, Result<(), String>),
    Pinned(i64, i64, String),
    PinFailed(i64, i64, String),
    Link(String),
}
struct App {
    state: State,
    db: storage::Store,
    root: PathBuf,
    tg: Option<telegram::Telegram>,
    updates: Option<mpsc::Receiver<Value>>,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    player: Option<audio::Player>,
    stream_base: String,
    server: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    selected: usize,
    opened_chat: Option<i64>,
    indexing_chat: Option<i64>,
    index_epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
    chat_query: String,
    chat_search_active: bool,
    image_wanted: Vec<i64>,
    image_retry: std::collections::HashMap<i64, std::time::Instant>,
    search: String,
    status: String,
    notice: String,
    notice_until: Option<std::time::Instant>,
    auth: Value,
    phone: String,
    code: String,
    password: String,
    qr: String,
    current: Option<Track>,
    history: Vec<Track>,
    paused: bool,
    volume: f32,
    queue_open: bool,
    demo: bool,
    busy: bool,
    media: fastframe_now_playing::NowPlaying,
    pin_jobs: Option<mpsc::Sender<(u64, Track)>>,
    sort: usize,
    tray: Option<fastframe_tray::Tray>,
    hidden: bool,
    show_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
    auth_pending: bool,
    auth_token: u64,
    last_auth_step: String,
    image_requests: std::collections::HashSet<i64>,
    auth_error: String,
    resend_at: Option<std::time::Instant>,
    quitting: bool,
    frames: usize,
    preview_started: std::time::Instant,
    preview_captured: bool,
    settings_open: bool,
    pin_cancel: std::sync::Arc<std::sync::atomic::AtomicU64>,
    last_link: String,
    shuffle: bool,
    repeat: bool,
    started: bool,
}
fn root() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("tg-player")
}
impl App {
    fn new() -> Self {
        let root = root();
        std::fs::create_dir_all(&root).expect("create app data");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).ok();
        }
        let (db, mut state) =
            storage::Store::open(&root.join("library.sqlite")).expect("open library");
        let demo = std::env::args().any(|s| s == "--demo");
        if demo {
            state.api_id.clear();
            state.api_hash.clear();
            state.queue.clear();
            state.playlists = (0..5)
                .map(|i| Playlist {
                    id: i,
                    title: [
                        "Midnight frequencies",
                        "Saved Messages",
                        "Jazz & coffee",
                        "Electronic archive",
                        "Music discoveries",
                    ][i as usize]
                        .into(),
                    filters: [true, true, false],
                    tracks: (0..40)
                        .map(|j| Track {
                            chat: i,
                            message: j,
                            title: format!(
                                "{} {:02}",
                                [
                                    "Afterglow",
                                    "Pacific",
                                    "Blue hour",
                                    "Signals",
                                    "Slow motion"
                                ][j as usize % 5],
                                j + 1
                            ),
                            artist: ["Nils Frahm", "Bonobo", "Ólafur Arnalds", "Tycho"]
                                [j as usize % 4]
                                .into(),
                            duration: 180 + j as u64 * 3,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                })
                .collect();
        }
        let (tx, rx) = mpsc::channel();
        Self {
            state,
            db,
            root,
            tg: None,
            updates: None,
            tx,
            rx,
            player: audio::Player::new().ok(),
            stream_base: String::new(),
            server: None,
            selected: 0,
            opened_chat: None,
            indexing_chat: None,
            index_epoch: Default::default(),
            chat_query: String::new(),
            chat_search_active: false,
            image_wanted: Vec::new(),
            image_retry: Default::default(),
            search: String::new(),
            notice: String::new(),
            notice_until: None,
            status: if demo {
                tr("Demo mode · not connected to Telegram")
            } else {
                tr("Connect Telegram to discover your music")
            },
            auth: Value::Null,
            phone: String::new(),
            code: String::new(),
            password: String::new(),
            qr: if std::env::args().any(|a| a == "--preview-auth=qr") {
                "tg://login?token=DEMO".into()
            } else {
                String::new()
            },
            current: None,
            history: Vec::new(),
            paused: false,
            volume: 75.,
            queue_open: false,
            demo,
            busy: false,
            media: fastframe_now_playing::NowPlaying::start(
                fastframe_now_playing::App::new("tgplayer", "TG Player"),
                || {},
            ),
            pin_jobs: None,
            sort: 0,
            tray: fastframe_tray::Tray::spawn(
                fastframe_tray::Config {
                    id: "tgplayer",
                    title: "TG Player".into(),
                    icon: tray_icon,
                    template_icon: None,
                    themed_icon: false,
                    menu: vec![
                        fastframe_tray::MenuItem::action("show", tr("Show player")),
                        fastframe_tray::MenuItem::action("play", tr("Play / pause")),
                        fastframe_tray::MenuItem::action("next", tr("Next")),
                        fastframe_tray::MenuItem::Separator,
                        fastframe_tray::MenuItem::action("quit", tr("Quit")),
                    ],
                },
                || {},
            ),
            hidden: false,
            show_requested: Default::default(),
            auth_pending: false,
            auth_token: 0,
            last_auth_step: String::new(),
            image_requests: Default::default(),
            auth_error: if std::env::args().any(|a| a == "--preview-auth=error") {
                tr("That code is incorrect. Please try again.")
            } else {
                String::new()
            },
            resend_at: None,
            quitting: false,
            frames: 0,
            preview_started: std::time::Instant::now(),
            preview_captured: false,
            settings_open: false,
            pin_cancel: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            last_link: String::new(),
            shuffle: false,
            repeat: false,
            started: false,
        }
    }
    fn persist(&self) {
        if !self.demo {
            self.db.save(self.state.clone());
        }
    }
    fn connect(&mut self) {
        self.qr.clear();
        self.auth_error.clear();
        self.auth_pending = false;
        self.auth_token += 1;

        match telegram::Telegram::start() {
            Ok((tg, rx)) => {
                tg.offline();
                tg.request(json!({"@type":"setLogVerbosityLevel","new_verbosity_level":0}));
                tg.request(json!({"@type":"setOption","name":"use_storage_optimizer","value":{"@type":"optionValueBoolean","value":true}}));
                tg.request(json!({"@type":"setOption","name":"storage_max_files_size","value":{"@type":"optionValueInteger","value":2147483648i64}}));
                let (jobs, rx_jobs) = mpsc::channel::<(u64, Track)>();
                let downloader = tg.clone();
                let cancel = self.pin_cancel.clone();
                let events = self.tx.clone();
                let dir = self.root.join("offline");
                std::thread::spawn(move || {
                    while let Ok((generation, t)) = rx_jobs.recv() {
                        if cancel.load(std::sync::atomic::Ordering::Relaxed) != generation {
                            continue;
                        }
                        let result = (|| -> Result<String, String> {
                            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                            downloader.call(json!({"@type":"downloadFile","file_id":t.file,"priority":8,"offset":0,"limit":0,"synchronous":false}))?;
                            let mut complete = None;
                            for _ in 0..6000 {
                                if cancel.load(std::sync::atomic::Ordering::Relaxed) != generation {
                                    return Err(tr("Download cancelled"));
                                }
                                let v =
                                    downloader.call(json!({"@type":"getFile","file_id":t.file}))?;
                                if v["local"]["is_downloading_completed"] == true {
                                    complete = Some(v);
                                    break;
                                }
                                let _ = events.send(Event::Progress(format!(
                                    "Offline: {} · {} / {} bytes",
                                    t.title, v["local"]["downloaded_size"], v["size"]
                                )));
                                std::thread::sleep(Duration::from_millis(100));
                            }
                            let v = complete.ok_or(tr("Download timed out. Try saving again."))?;
                            let path = dir.join(format!("{}-{}", t.chat, t.message));
                            let temp = path.with_extension("part");
                            std::fs::copy(
                                v["local"]["path"].as_str().ok_or("File unavailable")?,
                                &temp,
                            )
                            .map_err(|e| e.to_string())?;
                            std::fs::rename(temp, &path).map_err(|e| e.to_string())?;
                            Ok(path.to_string_lossy().into())
                        })();
                        let _ = events.send(match result {
                            Ok(path) => Event::Pinned(t.chat, t.message, path),
                            Err(e) => Event::PinFailed(t.chat, t.message, e),
                        });
                    }
                });
                self.pin_jobs = Some(jobs);
                match audio::serve(tg.clone()) {
                    Ok((p, a)) => {
                        self.stream_base = p;
                        self.server = Some(a)
                    }
                    Err(e) => self.status = e,
                }
                self.tg = Some(tg);
                self.updates = Some(rx);
                self.status = tr("Connecting…")
            }
            Err(e) => self.status = e,
        }
    }
    fn send(&self, v: Value) {
        if let Some(t) = &self.tg {
            let t = t.clone();
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                if let Err(e) = t.call(v) {
                    let _ = tx.send(Event::Error(e));
                }
            });
        }
    }
    fn sync(&mut self) {
        if self.busy {
            return;
        }
        let Some(tg) = self.tg.clone() else { return };
        self.busy = true;
        self.status = tr("Loading chat list…");
        let tx = self.tx.clone();
        let filters = self.state.filters;
        std::thread::spawn(move || {
            let _ = tx.send(Event::Catalog(discovery::catalog(&tg, filters)));
        });
    }
    fn open_chat(&mut self, index: usize) {
        self.selected = index;
        let Some(mut p) = self.state.playlists.get(index).cloned() else {
            return;
        };
        self.opened_chat = Some(p.id);
        let generation = self
            .index_epoch
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        self.indexing_chat = None;
        if self.demo || !p.has_more() {
            return;
        }
        let Some(tg) = self.tg.clone() else { return };
        self.indexing_chat = Some(p.id);
        let epoch = self.index_epoch.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            if p.url.is_empty() {
                if let Ok(chat) = tg.call(json!({"@type":"getChat","chat_id":p.id})) {
                    let ty = &chat["type"];
                    let peer=match ty["@type"].as_str().unwrap_or(""){"chatTypeSupergroup"=>tg.call(json!({"@type":"getSupergroup","supergroup_id":ty["supergroup_id"]})).ok(),"chatTypePrivate"=>tg.call(json!({"@type":"getUser","user_id":ty["user_id"]})).ok(),_=>None};
                    if let Some(peer) = peer {
                        if let Some(username) = peer["usernames"]["active_usernames"][0].as_str() {
                            p.url = format!("https://t.me/{username}");
                        }
                    }
                }
            }
            let result = discovery::index(
                p,
                &epoch,
                generation,
                |v| tg.call(v),
                |p| {
                    let _ = tx.send(Event::Playlist(generation, p));
                },
            );
            let _ = tx.send(Event::Indexed(generation, result));
        });
    }
    fn play(&mut self, t: Track) {
        if let Some(old) = &self.current {
            if (old.chat, old.message) != (t.chat, t.message) {
                self.history.push(old.clone());
            }
        }
        if self.demo {
            self.status = tr("Demo tracks are fictional");
            self.current = Some(t);
            return;
        }
        if let Some(p) = &self.player {
            let path = t
                .pinned
                .clone()
                .or(t.cached.clone())
                .filter(|p| std::path::Path::new(p).is_file())
                .unwrap_or_else(|| format!("{}/{}", self.stream_base, t.file));
            p.cmd(&["loadfile", &path, "replace"]);
            p.cmd(&["set", "pause", "no"]);
            p.cmd(&["set", "volume", &self.volume.to_string()]);
            self.current = Some(t);
            self.paused = false;
            self.started = false;
            if let (Some(tg), Some(next)) = (&self.tg, self.state.queue.first()) {
                tg.request(json!({"@type":"downloadFile","file_id":next.file,"priority":1,"offset":0,"limit":1048576,"synchronous":false}));
            }
        } else {
            self.status = tr("Playback requires libmpv.so.2")
        }
    }
    fn previous(&mut self) {
        if self
            .player
            .as_ref()
            .is_some_and(|p| p.number("time-pos") > 3.)
        {
            if let Some(p) = &self.player {
                p.cmd(&["seek", "0", "absolute"]);
            }
            return;
        }
        if let Some(t) = self.history.pop() {
            if let Some(current) = self.current.take() {
                self.state.queue.insert(0, current);
            }
            self.play(t);
        }
    }
    fn next(&mut self) {
        if self.repeat {
            if let Some(t) = self.current.clone() {
                self.play(t);
                return;
            }
        }
        if !self.state.queue.is_empty() {
            let index = if self.shuffle {
                (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos() as usize)
                    % self.state.queue.len()
            } else {
                0
            };
            let t = self.state.queue.remove(index);
            self.play(t)
        }
    }
    fn pin(&mut self, t: Track) {
        if t.pinned.is_some()
            || self
                .state
                .pending_pins
                .iter()
                .any(|x| x.chat == t.chat && x.message == t.message)
        {
            return;
        }
        self.state.pending_pins.push(t.clone());
        self.persist();
        if let Some(jobs) = &self.pin_jobs {
            let _ = jobs.send((
                self.pin_cancel.load(std::sync::atomic::Ordering::Relaxed),
                t,
            ));
        }
    }
    fn link(&self, t: &Track) {
        let Some(tg) = self.tg.clone() else { return };
        let tx = self.tx.clone();
        let (chat, message) = (t.chat, t.message);
        std::thread::spawn(move || {
            let r=tg.call(json!({"@type":"getMessageLink","chat_id":chat,"message_id":message,"media_timestamp":0,"for_album":false,"in_message_thread":false}));
            let _ = tx.send(match r {
                Ok(v) => Event::Link(v["link"].as_str().unwrap_or("").into()),
                Err(_) => Event::Error(tr("Telegram does not provide a link for this message")),
            });
        });
    }
    fn apply_artwork(&mut self, id: i64, path: &str) {
        for p in &mut self.state.playlists {
            if p.image_file == id {
                p.image_path = Some(path.into());
            }
            for t in &mut p.tracks {
                if t.image_file == id {
                    t.image_path = Some(path.into());
                }
            }
        }
        for t in self.state.queue.iter_mut().chain(self.current.iter_mut()) {
            if t.image_file == id {
                t.image_path = Some(path.into());
            }
        }
    }
    fn artwork_path(&self, id: i64) -> Option<&str> {
        self.state
            .playlists
            .iter()
            .find_map(|p| {
                (p.image_file == id)
                    .then_some(p.image_path.as_deref())
                    .flatten()
                    .or_else(|| {
                        p.tracks.iter().find_map(|t| {
                            (t.image_file == id)
                                .then_some(t.image_path.as_deref())
                                .flatten()
                        })
                    })
            })
            .or_else(|| {
                self.state
                    .queue
                    .iter()
                    .chain(self.current.iter())
                    .find_map(|t| {
                        (t.image_file == id)
                            .then_some(t.image_path.as_deref())
                            .flatten()
                    })
            })
    }

    fn toggle_chat_pin(&mut self, id: i64) {
        self.state.toggle_chat_pin(id);
        self.persist();
    }
    fn process(&mut self, ctx: &egui::Context) {
        if let Some(error) = self.db.error() {
            self.status = error;
        }
        let mut updates = Vec::new();
        if let Some(rx) = &self.updates {
            while let Ok(v) = rx.try_recv() {
                updates.push(v)
            }
        }
        for v in updates {
            match v["@type"].as_str().unwrap_or("") {
                "updateAuthorizationState" => {
                    self.auth = v["authorization_state"].clone();
                    if self.auth["@type"] == "authorizationStateWaitCode" {
                        self.resend_at = Some(
                            std::time::Instant::now()
                                + Duration::from_secs(
                                    self.auth["code_info"]["timeout"].as_u64().unwrap_or(0),
                                ),
                        );
                    }
                    if self.auth["@type"] == "authorizationStateWaitPhoneNumber" {
                        self.submit_auth(
                            json!({"@type":"requestQrCodeAuthentication","other_user_ids":[]}),
                        );
                    }
                    if self.auth["@type"] == "authorizationStateClosed" {
                        self.tg = None;
                        self.auth_pending = false;
                    }
                    match self.auth["@type"].as_str().unwrap_or("") {
                        "authorizationStateWaitTdlibParameters" => {
                            let id = self.state.api_id.parse::<i32>().unwrap_or(0);
                            let path = self.root.join("telegram");
                            self.submit_auth(json!({
"@type":"setTdlibParameters","database_directory":path,"files_directory":self.root.join("cache"),"use_message_database":true,"use_file_database":true,"use_chat_info_database":true,"use_secret_chats":false,"api_id":id,"api_hash":self.state.api_hash,"system_language_code":"en","device_model":"TG Player","system_version":"Linux","application_version":"0.1.0"}

));
                        }
                        "authorizationStateWaitOtherDeviceConfirmation" => {
                            self.qr = self.auth["link"].as_str().unwrap_or("").into()
                        }
                        "authorizationStateReady" => {
                            self.persist();
                            if let Some(jobs) = &self.pin_jobs {
                                for t in &self.state.pending_pins {
                                    let _ = jobs.send((
                                        self.pin_cancel.load(std::sync::atomic::Ordering::Relaxed),
                                        t.clone(),
                                    ));
                                }
                            }
                            self.sync()
                        }
                        _ => {}
                    }
                }
                "updateFile" => {
                    let f = &v["file"];
                    if f["local"]["is_downloading_completed"] == true {
                        if let (Some(id), Some(path)) =
                            (f["id"].as_i64(), f["local"]["path"].as_str())
                        {
                            self.apply_artwork(id, path);
                        }
                    }
                    let id = f["id"].as_i64().unwrap_or(0);
                    let path = if f["local"]["is_downloading_completed"] == true {
                        f["local"]["path"].as_str().map(str::to_owned)
                    } else {
                        None
                    };
                    for p in &mut self.state.playlists {
                        for t in &mut p.tracks {
                            if t.file == id {
                                t.cached = path.clone();
                            }
                        }
                    }
                }
                "updateChatTitle" => {
                    if let Some(p) = self
                        .state
                        .playlists
                        .iter_mut()
                        .find(|p| Some(p.id) == v["chat_id"].as_i64())
                    {
                        p.title = v["title"].as_str().unwrap_or("").into();
                    }
                }
                "updateMessageContent" => {
                    let chat = v["chat_id"].as_i64().unwrap_or(0);
                    let id = v["message_id"].as_i64().unwrap_or(0);
                    let m = json!({"id":id,"content":v["new_content"]});
                    if let Some(p) = self.state.playlists.iter_mut().find(|p| {
                        p.id == chat
                            && ((p.done.iter().any(|d| *d) || p.cursors.iter().any(|c| *c != 0))
                                || self.opened_chat == Some(chat))
                    }) {
                        let pinned = p
                            .tracks
                            .iter()
                            .find(|t| t.message == id)
                            .and_then(|t| t.pinned.clone());
                        p.tracks.retain(|t| t.message != id);
                        if let Some(mut t) = model::track(chat, &m) {
                            t.pinned = pinned;
                            p.tracks.push(t);
                            p.tracks.sort_by_key(|t| std::cmp::Reverse(t.message));
                        }
                    }
                }
                "updateNewMessage" => {
                    let m = &v["message"];
                    let chat = m["chat_id"].as_i64().unwrap_or(0);
                    if let Some(t) = model::track(chat, m) {
                        if let Some(p) = self.state.playlists.iter_mut().find(|p| {
                            p.id == chat
                                && ((p.done.iter().any(|d| *d)
                                    || p.cursors.iter().any(|c| *c != 0))
                                    || self.opened_chat == Some(chat))
                        }) {
                            if !p.tracks.iter().any(|x| x.message == t.message) {
                                p.tracks.insert(0, t);
                            }
                        }
                    }
                }
                "updateDeleteMessages" => {
                    if let Some(p) = self
                        .state
                        .playlists
                        .iter_mut()
                        .find(|p| Some(p.id) == v["chat_id"].as_i64())
                    {
                        p.tracks.retain(|t| {
                            !v["message_ids"]
                                .as_array()
                                .is_some_and(|a| a.contains(&json!(t.message)))
                        });
                    }
                }
                _ => {}
            }
        }
        while let Ok(e) = self.rx.try_recv() {
            match e {
                Event::Artwork(id, result) => {
                    self.image_requests.remove(&id);
                    let path = match result {
                        Ok(path) => {
                            self.image_retry.remove(&id);
                            ctx.forget_image(&format!("file://{path}"));
                            path
                        }
                        Err(_) => {
                            self.image_retry
                                .insert(id, std::time::Instant::now() + Duration::from_secs(5));
                            continue;
                        }
                    };
                    self.apply_artwork(id, &path);
                }
                Event::Progress(s) => self.status = s,
                Event::AuthResult(token, result) => {
                    if token == self.auth_token {
                        self.auth_pending = false;
                        self.password.clear();
                        match result {
                            Ok(()) => {
                                self.auth_error.clear();
                                self.code.clear();
                            }
                            Err(e) => {
                                self.auth_error = auth::friendly_error(&e);
                                self.last_auth_step.clear();
                            }
                        }
                    }
                }
                Event::Error(s) => {
                    self.status = s;
                    self.busy = false
                }
                Event::Link(s) => {
                    self.last_link = s.clone();
                    ctx.copy_text(s);
                    self.status = tr("Link copied")
                }
                Event::PinFailed(c, m, error) => {
                    self.state
                        .pending_pins
                        .retain(|t| t.chat != c || t.message != m);
                    self.status = error;
                    self.persist();
                }
                Event::Pinned(c, m, path) => {
                    for p in &mut self.state.playlists {
                        for t in &mut p.tracks {
                            if t.chat == c && t.message == m {
                                t.pinned = Some(path.clone())
                            }
                        }
                    }
                    self.state
                        .pending_pins
                        .retain(|t| t.chat != c || t.message != m);
                    self.status = tr("Saved for offline listening");
                    self.persist()
                }
                Event::Indexed(g, result) => {
                    if g == self.index_epoch.load(std::sync::atomic::Ordering::Relaxed) {
                        self.indexing_chat = None;
                        self.status = result.err().unwrap_or_else(|| tr("Music loaded"));
                    }
                }
                Event::Catalog(result) => {
                    self.busy = false;
                    match result {
                        Err(e) => self.status = e,
                        Ok(chats) => {
                            for p in chats {
                                if let Some(old) =
                                    self.state.playlists.iter_mut().find(|x| x.id == p.id)
                                {
                                    old.title = p.title;
                                    if old.image_file != p.image_file {
                                        old.image_path = None;
                                    }
                                    old.image_file = p.image_file;
                                } else {
                                    self.state.playlists.push(p);
                                }
                            }
                            self.status = tr("Open a chat to load its music");
                            self.persist();
                        }
                    }
                }
                Event::Playlist(g, mut p) => {
                    if g != self.index_epoch.load(std::sync::atomic::Ordering::Relaxed) {
                        continue;
                    }
                    if let Some(old) = self.state.playlists.iter_mut().find(|x| x.id == p.id) {
                        p.image_path = p.image_path.or(old.image_path.clone());
                        p.filters = old.filters;
                        p.pinned = old.pinned;
                        for t in &mut p.tracks {
                            t.image_path = t.image_path.clone().or_else(|| {
                                old.tracks
                                    .iter()
                                    .find(|x| x.message == t.message)
                                    .and_then(|x| x.image_path.clone())
                            });
                            t.pinned = old
                                .tracks
                                .iter()
                                .find(|x| x.message == t.message)
                                .and_then(|x| x.pinned.clone())
                        }
                        *old = p
                    } else {
                        self.state.playlists.push(p)
                    }
                    self.persist()
                }
            }
        }
    }
}

fn main() -> eframe::Result {
    lifecycle::run()
}

fn tray_icon(size: usize) -> Vec<u8> {
    let mut pixels = vec![0; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let i = (y * size + x) * 4;
            let dx = x as f32 - size as f32 / 2.;
            let dy = y as f32 - size as f32 / 2.;
            if dx * dx + dy * dy < (size as f32 * 0.44).powi(2) {
                pixels[i..i + 4].copy_from_slice(&[99, 215, 196, 255]);
            }
        }
    }
    pixels
}
