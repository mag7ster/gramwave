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
mod demo;
mod media_loader;
mod model;
mod playback;
use playback::RepeatMode;
mod telegram;
use eframe::egui;
use model::{Playlist, State, Track};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::mpsc, time::Duration};
enum Event {
    ChatSearch(u64, String, Result<Vec<Playlist>, String>),
    Progress(String),
    TrackReady((i64, i64), Result<Track, String>),
    PlaybackReady(u64, Result<Track, String>),
    Artwork(i64, Result<String, String>),
    Playlist(u64, Playlist),
    Refreshed(u64, discovery::Changes),
    Catalog(Result<Vec<Playlist>, String>),
    Indexed(u64, Result<(), String>),
    Error(String),
    AuthResult(u64, Result<(), String>),
    Pinned(i64, i64, String),
    PinFailed(i64, i64, String),
    Link(String),
}
enum RetryAction {
    Playback(Track),
    Chat(usize),
    Offline(Track),
    Catalog,
}
#[derive(Default)]
struct ChatSearch {
    query: String,
    generation: u64,
    since: Option<std::time::Instant>,
    sent: bool,
    ids: Vec<i64>,
    error: String,
    loading: bool,
}
struct App {
    chat_search: ChatSearch,
    #[cfg(test)]
    test_rows: std::collections::BTreeMap<i64, (egui::Rect, egui::Rect)>,
    #[cfg(test)]
    test_seek: Option<(egui::Rect, egui::Id)>,
    #[cfg(test)]
    test_seeks: Vec<f64>,
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
    source_changed: std::collections::HashSet<(i64, i64)>,
    chat_query: String,
    chat_search_active: bool,
    image_wanted: Vec<i64>,
    track_wanted: Vec<Track>,
    track_requests: std::collections::HashSet<(i64, i64)>,
    track_retry: std::collections::HashMap<(i64, i64), std::time::Instant>,
    playback_generation: u64,
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
    paused: bool,
    loading: bool,
    seek_preview: Option<f64>,
    error: String,
    error_retry: Option<RetryAction>,
    pin_errors: std::collections::HashMap<(i64, i64), String>,
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
        let demo = std::env::args().any(|s| s == "--demo");
        let root = if demo {
            std::env::temp_dir().join(format!("tg-player-demo-{}", std::process::id()))
        } else {
            root()
        };
        Self::new_at(root, demo)
    }
    fn new_at(root: PathBuf, demo: bool) -> Self {
        std::fs::create_dir_all(&root).expect("create app data");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).ok();
        }
        let (db, mut state) =
            storage::Store::open(&root.join("library.sqlite")).expect("open library");
        if demo {
            state = demo::state();
        } else {
            state.repair_legacy_queue();
            if state.queue_version < 2 {
                if state.previous_queue.is_empty() {
                    state.previous_queue = state.queue.clone();
                }
                state.session.import(std::mem::take(&mut state.queue));
                state.queue_version = 2;
            }
            db.save(state.clone());
        }

        let (tx, rx) = mpsc::channel();
        Self {
            chat_search: ChatSearch::default(),
            #[cfg(test)]
            test_rows: Default::default(),
            #[cfg(test)]
            test_seek: None,
            #[cfg(test)]
            test_seeks: vec![],
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
            source_changed: Default::default(),
            chat_query: String::new(),
            chat_search_active: false,
            image_wanted: Vec::new(),
            track_wanted: Vec::new(),
            track_requests: Default::default(),
            track_retry: Default::default(),
            playback_generation: 0,
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
            paused: true,
            loading: false,
            seek_preview: None,
            error: String::new(),
            error_retry: None,
            pin_errors: Default::default(),
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
                            let t = media_loader::resolve(&downloader, &t)?;
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
                    Err(e) => self.error = e,
                }
                self.tg = Some(tg);
                self.updates = Some(rx);
                self.status = tr("Connecting…")
            }
            Err(e) => self.error = e,
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
    fn merge_chat(&mut self, p: Playlist) {
        if let Some(old) = self.state.playlists.iter_mut().find(|x| x.id == p.id) {
            old.title = p.title;
            if old.image_file != p.image_file {
                old.image_path = None;
            }
            old.image_file = p.image_file;
        } else {
            self.state.playlists.push(p);
        }
    }
    fn poll_chat_search(&mut self, ctx: &egui::Context) {
        let query = if self.chat_search_active {
            self.chat_query.trim().to_owned()
        } else {
            String::new()
        };
        if query != self.chat_search.query {
            self.chat_search.query = query.clone();
            self.chat_search.generation += 1;
            self.chat_search.since = Some(std::time::Instant::now());
            self.chat_search.sent = false;
            self.chat_search.ids.clear();
            self.chat_search.error.clear();
            self.chat_search.loading = false;
        }
        if query.is_empty() || self.demo || self.chat_search.sent {
            return;
        }
        if self
            .chat_search
            .since
            .is_some_and(|t| t.elapsed() < Duration::from_millis(250))
        {
            ctx.request_repaint_after(Duration::from_millis(250));
            return;
        }
        let Some(tg) = self.tg.clone() else {
            return;
        };
        self.chat_search.sent = true;
        self.chat_search.loading = true;
        let generation = self.chat_search.generation;
        let tx = self.tx.clone();
        let filters = self.state.filters;
        std::thread::spawn(move || {
            let result = discovery::search_chats(&tg, &query, filters);
            let _ = tx.send(Event::ChatSearch(generation, query, result));
        });
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
        self.load_chat(index, true);
    }
    fn load_chat(&mut self, index: usize, refresh: bool) {
        self.selected = index;
        let Some(mut p) = self.state.playlists.get(index).cloned() else {
            return;
        };
        if self.opened_chat != Some(p.id) {
            self.search.clear();
        }
        if refresh {
            p.recover_empty_index();
            self.state.playlists[index] = p.clone();
        }
        self.opened_chat = Some(p.id);
        let generation = self
            .index_epoch
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        self.indexing_chat = None;
        self.source_changed.clear();
        if self.demo || (!refresh && !p.has_more()) {
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
            let result = if refresh {
                if !p.url.is_empty() {
                    let _ = tx.send(Event::Refreshed(
                        generation,
                        discovery::Changes {
                            chat: p.id,
                            url: Some(p.url.clone()),
                            ..Default::default()
                        },
                    ));
                }
                discovery::refresh(
                    p,
                    &epoch,
                    generation,
                    |v| tg.call(v),
                    |changes| {
                        let _ = tx.send(Event::Refreshed(generation, changes));
                    },
                )
            } else {
                discovery::index(
                    p,
                    &epoch,
                    generation,
                    |v| tg.call(v),
                    |p| {
                        let _ = tx.send(Event::Playlist(generation, p));
                    },
                )
            };
            let _ = tx.send(Event::Indexed(generation, result));
        });
    }
    fn play(&mut self, t: Track) {
        self.state
            .session
            .prepare_cycle(self.state.repeat, self.state.shuffle);
        self.playback_generation += 1;
        self.seek_preview = None;
        self.loading = true;
        self.paused = false;
        self.error.clear();
        self.error_retry = Some(RetryAction::Playback(t.clone()));
        if let Some(p) = &self.player {
            p.cmd(&["stop"]);
            p.drain();
        }
        if !self.demo
            && t.file == 0
            && !t
                .pinned
                .as_ref()
                .is_some_and(|p| std::path::Path::new(p).is_file())
        {
            let Some(tg) = self.tg.clone() else {
                self.loading = false;
                self.paused = true;
                self.error = tr("Telegram is not connected");
                return;
            };
            let generation = self.playback_generation;
            let tx = self.tx.clone();
            self.status = tr("Loading track…");
            std::thread::spawn(move || {
                let _ = tx.send(Event::PlaybackReady(
                    generation,
                    media_loader::resolve(&tg, &t),
                ));
            });
            return;
        }
        self.play_ready(t);
    }
    fn play_ready(&mut self, t: Track) {
        if self.demo {
            self.loading = false;
            self.status = tr("Demo tracks are fictional");
            self.state.session.current = Some(t);
            return;
        }
        if let Some(p) = &self.player {
            let path = t
                .pinned
                .clone()
                .filter(|p| std::path::Path::new(p).is_file())
                .or_else(|| {
                    t.cached
                        .clone()
                        .filter(|p| std::path::Path::new(p).is_file())
                })
                .unwrap_or_else(|| format!("{}/{}", self.stream_base, t.file));
            p.cmd(&["loadfile", &path, "replace"]);
            p.cmd(&["set", "pause", if self.paused { "yes" } else { "no" }]);
            p.cmd(&["set", "volume", &self.state.volume.to_string()]);
            self.state.session.current = Some(t);
            self.status.clear();
            if let (Some(tg), Some(next)) = (&self.tg, self.state.session.remaining.first()) {
                if next.file > 0 {
                    tg.request(json!({"@type":"downloadFile","file_id":next.file,"priority":1,"offset":0,"limit":1048576,"synchronous":false}));
                }
            }
        } else {
            self.error = tr("Playback requires libmpv.so.2");
            self.loading = false;
            self.paused = true;
        }
    }
    fn retry_error(&mut self) {
        self.error.clear();
        match self.error_retry.take() {
            Some(RetryAction::Playback(t)) => self.play(t),
            Some(RetryAction::Chat(i)) => self.open_chat(i),
            Some(RetryAction::Offline(t)) => self.pin(t),
            Some(RetryAction::Catalog) => self.sync(),
            None => {}
        }
    }
    fn start_session(&mut self, tracks: &[Track], index: usize) {
        if let Some(t) = self.state.session.start(tracks, index, self.state.shuffle) {
            self.play(t);
            self.persist();
        }
    }
    fn choose_session(&mut self, tracks: &[Track], index: usize) {
        let ahead = tracks
            .get(index)
            .filter(|t| self.state.session.source == Some(t.chat))
            .and_then(|t| {
                self.state
                    .session
                    .remaining
                    .iter()
                    .position(|q| playback::same(q, t))
            });
        let selected = if let Some(i) = ahead {
            self.state.session.jump_to(&mut self.state.queue, i)
        } else {
            self.state.session.start(tracks, index, self.state.shuffle)
        };
        if let Some(t) = selected {
            self.play(t);
            self.persist();
        }
    }
    fn set_paused(&mut self, paused: bool) {
        if self.state.session.current.is_none() {
            if !paused {
                self.next();
            }
            return;
        }
        if !paused
            && self.paused
            && !self.loading
            && self
                .player
                .as_ref()
                .is_none_or(|p| p.number("duration") <= 0.)
        {
            if let Some(t) = self.state.session.current.clone() {
                self.play(t);
                return;
            }
        }
        self.paused = paused;
        if let Some(p) = &self.player {
            p.cmd(&["set", "pause", if paused { "yes" } else { "no" }]);
        }
    }
    fn toggle_mute(&mut self) {
        self.set_volume(if self.state.volume == 0. {
            self.state.unmuted_volume.max(1.)
        } else {
            0.
        });
    }
    fn set_volume(&mut self, volume: f32) {
        if !volume.is_finite() {
            return;
        }
        self.state.volume = volume.clamp(0., 100.);
        if self.state.volume > 0. {
            self.state.unmuted_volume = self.state.volume;
        }
        if let Some(p) = &self.player {
            p.cmd(&["set", "volume", &self.state.volume.to_string()]);
        }
        self.persist();
    }
    fn set_shuffle(&mut self, shuffle: bool) {
        if self.state.shuffle != shuffle {
            self.state.shuffle = shuffle;
            self.state.session.reorder(shuffle);
            self.state.session.prepare_cycle(self.state.repeat, shuffle);
            self.persist();
        }
    }
    fn set_repeat(&mut self, repeat: RepeatMode) {
        self.state.repeat = repeat;
        self.state.session.prepare_cycle(repeat, self.state.shuffle);
        self.persist();
    }
    fn seek(&mut self, position: f64) {
        #[cfg(test)]
        self.test_seeks.push(position);
        self.seek_preview = None;
        if !self.loading {
            if let Some(p) = &self.player {
                p.cmd(&["seek", &position.max(0.).to_string(), "absolute+exact"]);
            }
        }
    }
    fn previous(&mut self) {
        if !self.loading
            && self
                .player
                .as_ref()
                .is_some_and(|p| p.number("time-pos") > 3.)
        {
            self.seek(0.);
            return;
        }
        if let Some(t) = self.state.session.previous(&mut self.state.queue) {
            self.play(t);
            self.persist();
        }
    }
    fn next(&mut self) {
        self.advance(false);
    }
    fn advance(&mut self, eof: bool) {
        if let Some(t) = self.state.session.advance(
            &mut self.state.queue,
            self.state.repeat,
            self.state.shuffle,
            eof,
        ) {
            self.play(t);
        } else {
            self.playback_generation += 1;
            self.loading = false;
            self.seek_preview = None;
            self.paused = true;
            if let Some(p) = &self.player {
                p.cmd(&["stop"]);
            }
        }
        self.persist();
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
        self.pin_errors.remove(&(t.chat, t.message));
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
    fn apply_track_media(&mut self, track: &Track) {
        for t in self
            .state
            .playlists
            .iter_mut()
            .flat_map(|p| p.tracks.iter_mut())
            .chain(self.state.queue.iter_mut())
            .chain(self.state.session.remaining.iter_mut())
            .chain(self.state.session.next_cycle.iter_mut())
            .chain(self.state.session.snapshot.iter_mut())
            .chain(self.state.session.history.iter_mut())
            .chain(self.state.session.preceding.iter_mut())
            .chain(self.state.pending_pins.iter_mut())
            .chain(self.state.session.current.iter_mut())
        {
            if (t.chat, t.message) == (track.chat, track.message) {
                t.file = track.file;
                t.cached = track.cached.clone();
                t.image_file = track.image_file;
                t.image_path = track.image_path.clone();
            }
        }
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
        for t in self
            .state
            .queue
            .iter_mut()
            .chain(self.state.session.remaining.iter_mut())
            .chain(self.state.session.next_cycle.iter_mut())
            .chain(self.state.session.snapshot.iter_mut())
            .chain(self.state.session.history.iter_mut())
            .chain(self.state.session.preceding.iter_mut())
            .chain(self.state.session.current.iter_mut())
        {
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
                    .chain(self.state.session.current.iter())
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
            self.error_retry = None;
            self.error = error;
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
                "updateNewChat" => {
                    if let Some(p) = discovery::chat_metadata(&v["chat"], self.state.filters) {
                        self.merge_chat(p);
                        self.persist();
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
                    self.source_changed.insert((chat, id));
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
                        self.source_changed.insert((chat, t.message));
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
                    if v["from_cache"] != true {
                        let chat = v["chat_id"].as_i64().unwrap_or(0);
                        for id in v["message_ids"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_i64)
                        {
                            self.source_changed.insert((chat, id));
                        }
                    }
                    self.state.apply_message_deletions(&v);
                }
                _ => {}
            }
        }
        while let Ok(e) = self.rx.try_recv() {
            match e {
                Event::TrackReady(key, result) => {
                    self.track_requests.remove(&key);
                    match result {
                        Ok(t) => {
                            self.track_retry.remove(&key);
                            self.apply_track_media(&t);
                        }
                        Err(_) => {
                            self.track_retry
                                .insert(key, std::time::Instant::now() + Duration::from_secs(15));
                        }
                    }
                }
                Event::PlaybackReady(generation, result) => {
                    if generation != self.playback_generation {
                        continue;
                    }
                    match result {
                        Ok(t) => {
                            self.apply_track_media(&t);
                            self.play_ready(t);
                        }
                        Err(e) => {
                            self.loading = false;
                            self.paused = true;
                            self.error = format!("{}: {e}", tr("Unable to play track"));
                        }
                    }
                }
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
                    self.error = s;
                    self.error_retry = None;
                    self.busy = false
                }
                Event::Link(s) => {
                    self.last_link = s.clone();
                    ctx.copy_text(s);
                    self.status = tr("Link copied")
                }
                Event::PinFailed(c, m, error) => {
                    self.error_retry = self
                        .state
                        .pending_pins
                        .iter()
                        .find(|t| t.chat == c && t.message == m)
                        .cloned()
                        .map(RetryAction::Offline);
                    self.state
                        .pending_pins
                        .retain(|t| t.chat != c || t.message != m);
                    self.pin_errors.insert((c, m), error.clone());
                    self.error = error;
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
                    for t in self
                        .state
                        .queue
                        .iter_mut()
                        .chain(self.state.session.snapshot.iter_mut())
                        .chain(self.state.session.remaining.iter_mut())
                        .chain(self.state.session.next_cycle.iter_mut())
                        .chain(self.state.session.current.iter_mut())
                        .chain(self.state.session.history.iter_mut())
                        .chain(self.state.session.preceding.iter_mut())
                    {
                        if t.chat == c && t.message == m {
                            t.pinned = Some(path.clone());
                        }
                    }
                    self.pin_errors.remove(&(c, m));
                    self.state
                        .pending_pins
                        .retain(|t| t.chat != c || t.message != m);
                    self.status = tr("Saved for offline listening");
                    self.persist()
                }
                Event::Indexed(g, result) => {
                    if g == self.index_epoch.load(std::sync::atomic::Ordering::Relaxed) {
                        self.indexing_chat = None;
                        match result {
                            Ok(()) => self.status = tr("Music loaded"),
                            Err(e) => {
                                self.error = e;
                                self.error_retry = Some(RetryAction::Chat(self.selected));
                            }
                        }
                    }
                }
                Event::ChatSearch(generation, query, result) => {
                    if generation != self.chat_search.generation
                        || !self.chat_search_active
                        || query != self.chat_query.trim()
                    {
                        continue;
                    }
                    self.chat_search.loading = false;
                    match result {
                        Ok(chats) => {
                            self.chat_search.ids = chats.iter().map(|p| p.id).collect();
                            for p in chats {
                                self.merge_chat(p);
                            }
                            self.persist();
                        }
                        Err(e) => self.chat_search.error = e,
                    }
                }
                Event::Catalog(result) => {
                    self.busy = false;
                    match result {
                        Err(e) => {
                            self.error = e;
                            self.error_retry = Some(RetryAction::Catalog);
                        }
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
                Event::Refreshed(g, changes) => {
                    if g != self.index_epoch.load(std::sync::atomic::Ordering::Relaxed) {
                        continue;
                    }
                    let protected = self
                        .source_changed
                        .iter()
                        .filter(|(chat, _)| *chat == changes.chat)
                        .map(|(_, id)| *id)
                        .collect();
                    if let Some(p) = self
                        .state
                        .playlists
                        .iter_mut()
                        .find(|p| p.id == changes.chat)
                    {
                        changes.apply(p, &protected);
                        self.persist();
                    }
                }
                Event::Playlist(g, p) => {
                    if g != self.index_epoch.load(std::sync::atomic::Ordering::Relaxed) {
                        continue;
                    }
                    let protected = self
                        .source_changed
                        .iter()
                        .filter(|(chat, _)| *chat == p.id)
                        .map(|(_, id)| *id)
                        .collect();
                    if let Some(old) = self.state.playlists.iter_mut().find(|x| x.id == p.id) {
                        old.merge_tracks(p.tracks, &protected);
                        old.cursors = p.cursors;
                        old.done = p.done;
                        if !p.url.is_empty() {
                            old.url = p.url;
                        }
                    } else {
                        self.state.playlists.push(p);
                    }
                    self.persist();
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
