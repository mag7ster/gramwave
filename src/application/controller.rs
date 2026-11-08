use crate::{application, streaming, telegram, tr, Track};
use eframe::egui;
use std::{path::PathBuf, sync::mpsc, time::Duration};
use tg_player_application::telegram::TelegramUpdate;
use tg_player_application::AppCore;
pub(crate) enum Event {
    Noop,
    Application(tg_player_application::workflow::AppEvent),
    Error(tg_player_application::AppError),
    Link(String),
    ForgetArtwork(String),
}
pub(crate) enum RetryAction {
    Playback(tg_player_application::model::Track),
    Chat(i64),
    Offline(tg_player_application::model::Track),
    Catalog,
}
#[derive(PartialEq, Eq)]
struct PlaylistCacheKey {
    id: i64,
    library: u64,
    media: u64,
    offline: u64,
    filters: [bool; 3],
    search: String,
    sort: usize,
}
struct PlaylistCache {
    key: PlaylistCacheKey,
    tracks: std::sync::Arc<Vec<Track>>,
}
pub(crate) struct DesktopApp {
    playlist_cache: std::cell::RefCell<Option<PlaylistCache>>,
    pub(crate) options: crate::launch::LaunchOptions,
    pub(crate) progress: std::sync::Arc<std::sync::Mutex<Option<(u64, String)>>>,
    pub(crate) tasks: application::tasks::Scheduler,
    #[cfg(test)]
    pub(crate) test_rows: std::collections::BTreeMap<i64, (egui::Rect, egui::Rect)>,
    #[cfg(test)]
    pub(crate) test_row_ids: std::collections::BTreeMap<i64, egui::Id>,
    #[cfg(test)]
    pub(crate) test_play_buttons: std::collections::BTreeMap<i64, egui::Rect>,
    #[cfg(test)]
    pub(crate) test_seek: Option<(egui::Rect, egui::Id)>,
    #[cfg(test)]
    pub(crate) test_seeks: Vec<f64>,
    pub(crate) state: AppCore,
    pub(crate) db: Option<Box<dyn application::ports::Repository>>,
    pub(crate) root: PathBuf,
    pub(crate) tg: Option<telegram::Telegram>,
    pub(crate) updates: Option<mpsc::Receiver<TelegramUpdate>>,
    pub(crate) tx: mpsc::Sender<Event>,
    pub(crate) rx: mpsc::Receiver<Event>,
    pub(crate) control: Option<mpsc::Receiver<application::control::Pending>>,
    pub(crate) player: Option<Box<dyn application::ports::PlayerBackend>>,
    pub(crate) stream_base: String,
    pub(crate) server: Option<streaming::Server>,
    pub(crate) opened_chat: Option<i64>,
    pub(crate) chat_query: String,
    pub(crate) chat_search_active: bool,
    pub(crate) image_wanted: Vec<i64>,
    pub(crate) track_wanted: Vec<Track>,
    pub(crate) search: String,
    pub(crate) status: String,
    pub(crate) notice: String,
    pub(crate) notice_until: Option<std::time::Instant>,
    pub(crate) api_id_input: String,
    pub(crate) api_hash_input: String,
    pub(crate) phone: String,
    pub(crate) code: String,
    pub(crate) password: String,
    pub(crate) qr: String,
    pub(crate) seek_preview: Option<f64>,
    pub(crate) error: String,
    pub(crate) error_retry: Option<RetryAction>,
    pub(crate) queue_open: bool,
    pub(crate) demo: bool,
    pub(crate) media: Option<fastframe_now_playing::NowPlaying>,
    pub(crate) sort: usize,
    pub(crate) tray: Option<fastframe_tray::Tray>,
    pub(crate) hidden: bool,
    pub(crate) show_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(crate) last_auth_step: String,
    pub(crate) auth_error: String,
    pub(crate) resend_at: Option<std::time::Instant>,
    pub(crate) quitting: bool,
    pub(crate) shutdown_finished: bool,
    pub(crate) shutdown_error: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    pub(crate) player_error: String,
    pub(crate) frames: usize,
    pub(crate) preview_started: std::time::Instant,
    pub(crate) preview_captured: bool,
    pub(crate) settings_open: bool,
    pub(crate) last_link: String,
}
impl DesktopApp {
    pub(crate) fn new() -> Result<Self, String> {
        let options = crate::launch::options().clone();
        let services = crate::bootstrap::Services::for_launch(&options)?;
        Ok(Self::from_services(services, options))
    }
    #[cfg(test)]
    pub(crate) fn new_at(root: PathBuf, demo: bool) -> Self {
        let services = crate::bootstrap::Services::open(root, demo, false).unwrap();
        Self::from_services(services, Default::default())
    }
    fn from_services(
        services: crate::bootstrap::Services,
        options: crate::launch::LaunchOptions,
    ) -> Self {
        let crate::bootstrap::Services {
            root,
            core: state,
            repository: db,
            player,
            player_error,
            tasks,
            media,
            tray,
            demo,
        } = services;
        let (tx, rx) = mpsc::channel();
        Self {
            playlist_cache: Default::default(),
            options: options.clone(),
            progress: Default::default(),
            tasks,
            #[cfg(test)]
            test_rows: Default::default(),
            #[cfg(test)]
            test_row_ids: Default::default(),
            #[cfg(test)]
            test_play_buttons: Default::default(),
            #[cfg(test)]
            test_seek: None,
            #[cfg(test)]
            test_seeks: vec![],
            api_id_input: state.view().api_id.clone(),
            api_hash_input: state.view().api_hash.clone(),
            state,
            db,
            root,
            tg: None,
            updates: None,
            tx,
            rx,
            control: None,
            player,
            player_error,
            shutdown_finished: false,
            shutdown_error: Default::default(),
            stream_base: String::new(),
            server: None,
            opened_chat: None,
            chat_query: String::new(),
            chat_search_active: false,
            image_wanted: Vec::new(),
            track_wanted: Vec::new(),
            search: String::new(),
            notice: String::new(),
            notice_until: None,
            status: if demo {
                tr("Demo mode · not connected to Telegram")
            } else {
                tr("Connect Telegram to discover your music")
            },
            phone: String::new(),
            code: String::new(),
            password: String::new(),
            qr: if options.preview_auth.as_deref() == Some("qr") {
                "tg://login?token=DEMO".into()
            } else {
                String::new()
            },
            seek_preview: None,
            error: String::new(),
            error_retry: None,
            queue_open: false,
            demo,
            media,
            sort: 0,
            tray,
            hidden: false,
            show_requested: Default::default(),
            last_auth_step: String::new(),
            auth_error: if options.preview_auth.as_deref() == Some("error") {
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
            last_link: String::new(),
        }
    }
    pub(crate) fn persist(&mut self) {
        let changes = self.state.take_changes();
        if let Some(db) = &self.db {
            if let Err(error) = db.apply(changes.clone()) {
                self.state.retain_changes(changes);
                self.error = error.to_string();
            }
        }
    }
    pub(crate) fn library_command(
        &mut self,
        command: tg_player_application::library::LibraryCommand,
    ) {
        let effects = self.state.library_command(command);
        self.run_effects(effects);
    }
    pub(crate) fn connect(&mut self) {
        self.core_command(tg_player_application::commands::AppCommand::Connect);
    }
    pub(crate) fn connect_native(&mut self) {
        self.qr.clear();
        self.auth_error.clear();
        self.stream_base.clear();
        if let Some(server) = self.server.take() {
            server.stop();
        }
        match crate::bootstrap::connect(self.tasks.handle()) {
            Ok(connection) => {
                match connection.streaming {
                    Ok((base, server)) => {
                        self.stream_base = base;
                        self.server = Some(server);
                    }
                    Err(error) => self.error = error,
                }
                self.library_command(tg_player_application::library::LibraryCommand::Connected(
                    connection.telegram.connection,
                ));
                self.tg = Some(connection.telegram);
                self.updates = Some(connection.updates);
                self.status = tr("Connecting…");
            }
            Err(error) => self.error = error,
        }
    }
    pub(crate) fn poll_chat_search(&mut self, ctx: &egui::Context) {
        let query = if self.chat_search_active {
            self.chat_query.clone()
        } else {
            String::new()
        };
        self.library_command(tg_player_application::library::LibraryCommand::Search {
            query,
            now: std::time::Instant::now(),
            force: false,
        });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
    pub(crate) fn retry_chat_search(&mut self) {
        self.library_command(tg_player_application::library::LibraryCommand::Search {
            query: self.chat_query.clone(),
            now: std::time::Instant::now(),
            force: true,
        });
    }
    pub(crate) fn sync(&mut self) {
        self.library_command(tg_player_application::library::LibraryCommand::RefreshCatalog);
    }
    pub(crate) fn open_chat(&mut self, index: usize) {
        self.load_chat(index, true);
    }
    pub(crate) fn load_chat(&mut self, index: usize, refresh: bool) {
        let Some(chat) = self.state.view().playlists.get(index).map(|p| p.id) else {
            return;
        };
        if self.opened_chat != Some(chat) {
            self.search.clear();
        }
        self.opened_chat = Some(chat);
        self.library_command(tg_player_application::library::LibraryCommand::Open {
            chat,
            refresh,
        });
    }
    pub(crate) fn play(&mut self, track: impl Into<Track>) {
        let track: Track = track.into();
        self.core_command(tg_player_application::commands::AppCommand::Play(
            track.into(),
        ));
    }
    pub(crate) fn retry_error(&mut self) {
        self.error.clear();
        match self.error_retry.take() {
            Some(RetryAction::Playback(t)) => self.play(t),
            Some(RetryAction::Chat(id)) => {
                if let Some(i) = self.state.view().playlists.iter().position(|p| p.id == id) {
                    self.open_chat(i);
                }
            }
            Some(RetryAction::Offline(t)) => self.pin(t),
            Some(RetryAction::Catalog) => self.sync(),
            None => {}
        }
    }
    pub(crate) fn start_session<T: Clone + Into<tg_player_application::model::Track>>(
        &mut self,
        tracks: &[T],
        index: usize,
    ) {
        self.core_command(tg_player_application::commands::AppCommand::StartSession(
            tracks.iter().cloned().map(Into::into).collect(),
            index,
        ));
    }
    pub(crate) fn choose_session(&mut self, tracks: &[Track], index: usize) {
        self.start_session(tracks, index);
    }
    pub(crate) fn playlist_tracks(&self, selected: usize) -> std::sync::Arc<Vec<Track>> {
        let p = &self.state.view().playlists[selected];
        let key = PlaylistCacheKey {
            id: p.id,
            library: self.state.library_version(),
            media: self.state.library().media_version(),
            offline: self.state.offline().version(),
            filters: p.filters,
            search: self.search.clone(),
            sort: self.sort,
        };
        if let Some(cache) = self
            .playlist_cache
            .borrow()
            .as_ref()
            .filter(|c| c.key == key)
        {
            return cache.tracks.clone();
        }

        let query = self.search.to_lowercase();
        let mut tracks: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| {
                p.filters[usize::from(t.kind)]
                    && format!("{} {}", t.title, t.artist)
                        .to_lowercase()
                        .contains(&query)
            })
            .cloned()
            .collect();
        match self.sort {
            1 => tracks.sort_by(|a, b| a.title.cmp(&b.title)),
            2 => tracks.sort_by(|a, b| a.artist.cmp(&b.artist)),
            _ => {}
        }
        let tracks = std::sync::Arc::new(tracks.into_iter().map(|t| self.media_track(t)).collect());
        *self.playlist_cache.borrow_mut() = Some(PlaylistCache {
            key,
            tracks: std::sync::Arc::clone(&tracks),
        });
        tracks
    }
    pub(crate) fn select_playlist_row(&mut self, index: usize) {
        let Some(selected) = self
            .opened_chat
            .and_then(|id| self.state.view().playlists.iter().position(|p| p.id == id))
        else {
            return;
        };
        let tracks = self.playlist_tracks(selected);
        let Some(track) = tracks.get(index) else {
            return;
        };
        if self
            .state
            .view()
            .session
            .current()
            .as_ref()
            .is_some_and(|c| c.id() == track.id())
        {
            self.set_paused(!self.state.playback().state().paused());
        } else {
            self.choose_session(&tracks, index);
        }
    }
    pub(crate) fn set_paused(&mut self, value: bool) {
        self.core_command(tg_player_application::commands::AppCommand::Pause(value));
    }
    pub(crate) fn toggle_mute(&mut self) {
        self.set_volume(if self.state.view().volume == 0. {
            self.state.view().unmuted_volume.max(1.)
        } else {
            0.
        });
    }
    pub(crate) fn set_volume(&mut self, value: f32) {
        self.core_command(tg_player_application::commands::AppCommand::Volume(value));
    }
    pub(crate) fn pin(&mut self, track: impl Into<tg_player_application::model::Track>) {
        self.core_command(tg_player_application::commands::AppCommand::SaveTrack(
            track.into(),
        ));
    }
    pub(crate) fn link(&self, t: &Track) {
        let Some(tg) = self.tg.clone() else { return };
        let (chat, message) = (t.chat, t.message);
        self.async_event(
            tg_player_application::tasks::JobClass::Interactive,
            0,
            async move {
                match tg.message_link(chat, message, &Default::default()).await {
                    Ok(link) => Event::Link(link),
                    Err(e) => Event::Error(e),
                }
            },
            Event::Error,
        );
    }
    pub(crate) fn media_track(&self, track: tg_player_application::model::Track) -> Track {
        let mut view: Track = self.state.media_track(track).into();
        view.pinned = self.state.offline().path(view.id()).map(str::to_owned);
        view
    }
    pub(crate) fn artwork_path(&self, id: i64) -> Option<&str> {
        self.state.library().artwork_path(id)
    }
    pub(crate) fn toggle_chat_pin(&mut self, id: i64) {
        self.core_command(tg_player_application::commands::AppCommand::ToggleChatPin(
            id,
        ));
    }
    pub(crate) fn process(&mut self, ctx: &egui::Context) -> usize {
        let mut processed = 0;
        if let Some((generation, status)) = self.progress.lock().unwrap().take() {
            if generation == self.state.offline().generation() {
                self.status = status;
            }
        }
        if let Some(error) = self.db.as_ref().and_then(|db| db.error()) {
            self.error_retry = None;
            self.error = error.to_string();
        }
        let mut updates = Vec::new();
        if let Some(rx) = &self.updates {
            updates.extend(rx.try_iter().take(128));
        }
        for update in updates {
            processed += 1;
            match update {
                TelegramUpdate::Authorization(state) => self.core_event(
                    tg_player_application::workflow::AppEvent::Authorization(state),
                ),
                update => self.library_command(
                    tg_player_application::library::LibraryCommand::Source(update),
                ),
            }
        }
        for _ in 0..128 {
            let Ok(e) = self.rx.try_recv() else {
                break;
            };
            processed += 1;
            match e {
                Event::Noop => {}
                Event::ForgetArtwork(path) => ctx.forget_image(&format!("file://{path}")),
                Event::Application(event) => self.core_event(event),
                Event::Error(s) => {
                    self.error = s.to_string();
                    self.error_retry = None;
                }
                Event::Link(s) => {
                    self.last_link = s.clone();
                    ctx.copy_text(s);
                    self.status = tr("Link copied")
                }
            }
        }
        processed
    }
}

pub(crate) fn tray_icon(size: usize) -> Vec<u8> {
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

#[cfg(test)]
#[path = "controller/tests.rs"]
mod tests;
