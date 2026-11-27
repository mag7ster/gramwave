use crate::{application, streaming, telegram, tr, Track};
use eframe::egui;
use gramwave_application::telegram::TelegramUpdate;
use gramwave_application::AppCore;
#[cfg(test)]
use std::time::Duration;
use std::{path::PathBuf, sync::mpsc};

mod artwork;
pub(crate) enum Event {
    Noop,
    NextPlaybackPrepared(
        application::tasks::RequestId,
        Option<application::ports::QueuedPlayback>,
    ),
    Cache {
        connection: u64,
        job: super::cache::CacheJob,
        result: Result<crate::telegram::CacheReport, gramwave_application::AppError>,
        offline_sizes: Option<std::collections::HashMap<String, u64>>,
    },
    Application(gramwave_application::workflow::AppEvent),
    CacheHandoff {
        id: crate::domain::TrackId,
        path: String,
        result: Result<(), gramwave_application::AppError>,
    },
    Error(gramwave_application::AppError),
    Link(String),
    SourceChatError(String),
    ForgetArtwork(String),
    Emoji(
        gramwave_application::tasks::RequestId,
        Vec<(i64, crate::adapters::emoji::EmojiAsset)>,
    ),
}
pub(crate) enum RetryAction {
    Playback(gramwave_application::model::Track),
    Chat(i64),
    Offline(gramwave_application::model::Track),
    Catalog,
}
#[derive(Clone, PartialEq, Eq)]
struct PlaylistCacheKey {
    id: i64,
    library: u64,
    filters: [bool; 3],
    search: String,
    sort: usize,
}
struct PlaylistCache {
    key: PlaylistCacheKey,
    tracks: std::sync::Arc<Vec<Track>>,
}
pub(crate) struct DesktopApp {
    pub(crate) wake: fastframe_shell::Waker,
    playlist_cache: std::cell::RefCell<Option<PlaylistCache>>,
    artwork_view: Option<artwork::PlaylistView>,
    artwork_prefetch: artwork::Prefetch,
    pub(crate) options: crate::launch::LaunchOptions,
    pub(crate) progress: std::sync::Arc<crate::adapters::offline::Progress>,
    pub(crate) tasks: application::tasks::Scheduler,
    #[cfg(test)]
    pub(crate) test_rows: std::collections::BTreeMap<i64, (egui::Rect, egui::Rect)>,
    #[cfg(test)]
    pub(crate) test_row_ids: std::collections::BTreeMap<i64, egui::Id>,
    #[cfg(test)]
    pub(crate) test_play_buttons: std::collections::BTreeMap<i64, egui::Rect>,
    #[cfg(test)]
    pub(crate) test_queue_rows: std::collections::BTreeMap<
        gramwave_domain::QueueEntryId,
        (egui::Rect, egui::Rect, egui::Id),
    >,
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
    pub(crate) cache_playback: super::cache::CachePlayback,
    pub(crate) local_playback: Option<(crate::domain::TrackId, String)>,
    pub(crate) queued_playback: Option<application::ports::QueuedPlayback>,
    pub(crate) preparing_next: Option<(
        crate::domain::QueueEntryId,
        u64,
        application::tasks::Operation,
    )>,
    pub(crate) handoff_running: bool,
    pub(crate) handoff_retry: std::collections::HashMap<String, std::time::Instant>,
    pub(crate) offline_selected: std::collections::BTreeSet<i64>,
    pub(crate) offline_confirm: Option<Vec<i64>>,
    pub(crate) stream_base: String,
    pub(crate) server: Option<streaming::Server>,
    pub(crate) opened_chat: Option<i64>,
    pub(crate) chat_query: String,
    pub(crate) chat_search_active: bool,
    pub(crate) image_wanted: Vec<i64>,
    pub(crate) custom_emoji: crate::emoji::CustomEmoji,
    pub(crate) track_wanted: Vec<Track>,
    pub(crate) search: String,
    pub(crate) status: String,
    pub(crate) inline_notice: Option<crate::ui::InlineNotice>,
    pub(crate) phone: String,
    pub(crate) code: String,
    pub(crate) password: String,
    pub(crate) qr: String,
    pub(crate) seek_preview: Option<f64>,
    pub(crate) show_remaining_time: bool,
    pub(crate) error: String,
    pub(crate) error_retry: Option<RetryAction>,
    pub(crate) pane: crate::screens::WorkspacePane,
    pub(crate) keyboard: crate::screens::Keyboard,
    pub(crate) selection: crate::screens::Selection,
    pub(crate) demo: bool,
    pub(crate) demo_artwork: Option<tempfile::TempDir>,
    pub(crate) media: Option<fastframe_now_playing::NowPlaying>,
    pub(crate) sort: usize,
    pub(crate) tray: Option<crate::adapters::tray::Tray>,
    pub(crate) hidden: bool,
    pub(crate) wayland: bool,
    pub(crate) reopen_token: Option<String>,
    pub(crate) tray_focus: Option<crate::ui::TrayFocus>,
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
    pub(crate) cache: super::cache::CacheState,
    pub(crate) last_link: String,
}
impl DesktopApp {
    pub(crate) fn new(wake: fastframe_shell::Waker) -> Result<Self, String> {
        let options = crate::launch::options().clone();
        let services = crate::bootstrap::Services::for_launch(&options, wake)?;
        Ok(Self::from_services(services, options))
    }
    #[cfg(test)]
    pub(crate) fn new_at(root: PathBuf, demo: bool) -> Self {
        let services =
            crate::bootstrap::Services::open(root, demo, false, false, Default::default()).unwrap();
        Self::from_services(services, Default::default())
    }
    fn from_services(
        services: crate::bootstrap::Services,
        options: crate::launch::LaunchOptions,
    ) -> Self {
        let crate::bootstrap::Services {
            wake,
            root,
            core: mut state,
            repository: db,
            player,
            player_error,
            tasks,
            media,
            tray,
            demo,
        } = services;
        state.enable_streamed_next(
            player
                .as_ref()
                .is_some_and(|player| player.prepares_streamed_next()),
        );
        let opened_chat = state.view().startup_chat();
        let (tx, rx) = mpsc::channel();
        Self {
            progress: std::sync::Arc::new(crate::adapters::offline::Progress::new(wake.clone())),
            wake,
            playlist_cache: Default::default(),
            artwork_view: None,
            artwork_prefetch: Default::default(),
            options: options.clone(),
            tasks,
            #[cfg(test)]
            test_rows: Default::default(),
            #[cfg(test)]
            test_row_ids: Default::default(),
            #[cfg(test)]
            test_play_buttons: Default::default(),
            #[cfg(test)]
            test_queue_rows: Default::default(),
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
            control: None,
            player,
            player_error,
            cache_playback: Default::default(),
            local_playback: None,
            queued_playback: None,
            preparing_next: None,
            handoff_running: false,
            handoff_retry: Default::default(),
            offline_selected: Default::default(),
            offline_confirm: None,
            shutdown_finished: false,
            shutdown_error: Default::default(),
            stream_base: String::new(),
            server: None,
            opened_chat,
            chat_query: String::new(),
            chat_search_active: false,
            image_wanted: Vec::new(),
            custom_emoji: Default::default(),
            track_wanted: Vec::new(),
            search: String::new(),
            inline_notice: None,
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
            show_remaining_time: false,
            error: String::new(),
            error_retry: None,
            pane: Default::default(),
            keyboard: Default::default(),
            selection: Default::default(),
            demo,
            demo_artwork: None,
            media,
            sort: 0,
            tray,
            hidden: false,
            wayland: false,
            reopen_token: None,
            tray_focus: None,
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
            cache: Default::default(),
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
        command: gramwave_application::library::LibraryCommand,
    ) {
        let effects = self.state.library_command(command);
        self.run_effects(effects);
    }
    pub(crate) fn connect(&mut self) {
        self.core_command(gramwave_application::commands::AppCommand::Connect);
    }
    pub(crate) fn connect_native(&mut self) {
        self.qr.clear();
        self.auth_error.clear();
        self.error.clear();
        self.stream_base.clear();
        if let Some(server) = self.server.take() {
            server.stop();
        }
        match crate::bootstrap::connect(self.tasks.handle(), self.wake.clone()) {
            Ok(connection) => {
                match connection.streaming {
                    Ok((base, server)) => {
                        self.stream_base = base;
                        self.server = Some(server);
                    }
                    Err(error) => self.error = error,
                }
                self.library_command(gramwave_application::library::LibraryCommand::Connected(
                    connection.telegram.connection,
                ));
                self.tg = Some(connection.telegram);
                self.cache = Default::default();
                self.updates = Some(connection.updates);
                self.status = tr("Connecting…");
            }
            Err(error) => self.error = error,
        }
    }
    pub(crate) fn chat_search_indices(&self, searching: bool, query: &str) -> Vec<usize> {
        let state = self.state.view();
        let mut indices = state.sidebar_chats(searching, query);
        let mut seen = std::collections::HashSet::new();
        indices.retain(|&i| seen.insert(state.playlists[i].id));
        let search = self.state.library().search();
        if searching && !query.is_empty() && search.query == query {
            indices.extend(
                state
                    .playlists
                    .iter()
                    .enumerate()
                    .filter_map(|(i, playlist)| {
                        (playlist.in_chat_search()
                            && search.ids.contains(&playlist.id)
                            && seen.insert(playlist.id))
                        .then_some(i)
                    }),
            );
        }
        indices
    }

    pub(crate) fn poll_chat_search(&mut self, ctx: &egui::Context) {
        let query = if self.chat_search_active {
            self.chat_query.clone()
        } else {
            String::new()
        };
        self.library_command(gramwave_application::library::LibraryCommand::Search {
            query,
            now: std::time::Instant::now(),
            force: false,
        });
        if let Some(at) = self.state.library().search_deadline() {
            ctx.request_repaint_after(at.saturating_duration_since(std::time::Instant::now()));
        }
    }
    pub(crate) fn request_visible_media(&mut self, ctx: &egui::Context) {
        use gramwave_application::library::LibraryCommand;
        let artwork_view = self.artwork_view.take();
        if self.settings_open {
            return;
        }
        self.request_custom_emoji(ctx);
        let now = std::time::Instant::now();
        let mut tracks = std::mem::take(&mut self.track_wanted);
        let mut images = std::mem::take(&mut self.image_wanted);
        let mut retry = None;
        for track in tracks.drain(..) {
            let id = track.id();
            self.library_command(LibraryCommand::RequestMedia {
                track: track.into(),
                now,
            });
            retry = retry
                .into_iter()
                .chain(
                    self.state
                        .library()
                        .media_retry_at(id)
                        .filter(|at| *at > now),
                )
                .min();
        }
        for id in images.drain(..) {
            let force = self
                .artwork_path(id)
                .is_some_and(|path| !std::path::Path::new(path).is_file());
            self.library_command(LibraryCommand::RequestArtwork { id, now, force });
            retry = retry
                .into_iter()
                .chain(
                    self.state
                        .library()
                        .artwork_retry_at(id)
                        .filter(|at| *at > now),
                )
                .min();
        }
        self.track_wanted = tracks;
        self.image_wanted = images;
        self.request_playlist_artwork(ctx, artwork_view, now);
        if let Some(at) = retry {
            ctx.request_repaint_after(at.saturating_duration_since(std::time::Instant::now()));
        }
    }
    pub(crate) fn retry_chat_search(&mut self) {
        self.library_command(gramwave_application::library::LibraryCommand::Search {
            query: self.chat_query.clone(),
            now: std::time::Instant::now(),
            force: true,
        });
    }
    pub(crate) fn sync(&mut self) {
        self.library_command(gramwave_application::library::LibraryCommand::RefreshCatalog);
    }
    pub(crate) fn open_chat(&mut self, index: usize) {
        self.load_chat(index, true);
    }
    pub(crate) fn load_chat(&mut self, index: usize, refresh: bool) {
        let Some(chat) = self.state.view().playlists.get(index).map(|p| p.id) else {
            return;
        };
        if refresh {
            self.core_command(gramwave_application::commands::AppCommand::OpenPlaylist(
                chat,
            ));
        } else {
            self.library_command(gramwave_application::library::LibraryCommand::Open {
                chat,
                refresh: false,
            });
        }
    }
    pub(crate) fn play(&mut self, track: impl Into<Track>) {
        let track: Track = track.into();
        self.core_command(gramwave_application::commands::AppCommand::Play(
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
            Some(RetryAction::Offline(t)) if self.state.offline().pending.contains_key(&t.id()) => {
                self.pin(t)
            }
            Some(RetryAction::Offline(_)) => {}
            Some(RetryAction::Catalog) => self.sync(),
            None => {}
        }
    }
    pub(crate) fn start_session<T: Clone + Into<gramwave_application::model::Track>>(
        &mut self,
        tracks: &[T],
        index: usize,
    ) {
        self.core_command(gramwave_application::commands::AppCommand::StartSession(
            tracks.iter().cloned().map(Into::into).collect(),
            index,
        ));
    }
    pub(crate) fn playlist_tracks(&self, selected: usize) -> std::sync::Arc<Vec<Track>> {
        self.playlist_tracks_matching(selected, &self.search)
    }
    fn playlist_cache_key(&self, selected: usize, search: &str) -> PlaylistCacheKey {
        let playlist = &self.state.view().playlists[selected];
        PlaylistCacheKey {
            id: playlist.id,
            library: self.state.library_version(),
            filters: playlist.filters,
            search: search.to_owned(),
            sort: self.sort,
        }
    }
    fn playlist_tracks_matching(
        &self,
        selected: usize,
        search: &str,
    ) -> std::sync::Arc<Vec<Track>> {
        let p = &self.state.view().playlists[selected];
        let key = self.playlist_cache_key(selected, search);
        if let Some(cache) = self
            .playlist_cache
            .borrow()
            .as_ref()
            .filter(|c| c.key == key)
        {
            return cache.tracks.clone();
        }

        let query = search.to_lowercase();
        let mut tracks: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| p.filters[usize::from(t.kind)])
            .collect();
        match self.sort {
            1 => tracks.sort_by(|a, b| a.title.cmp(&b.title)),
            2 => tracks.sort_by(|a, b| a.artist.cmp(&b.artist)),
            _ => {}
        }
        let tracks = std::sync::Arc::new(
            tracks
                .into_iter()
                .enumerate()
                .filter(|(_, t)| {
                    query.is_empty()
                        || format!("{} {}", t.title, t.artist)
                            .to_lowercase()
                            .contains(&query)
                })
                .map(|(index, t)| {
                    let mut track = Track::from(t.clone());
                    track.playlist_number = index + 1;
                    track
                })
                .collect(),
        );
        *self.playlist_cache.borrow_mut() = Some(PlaylistCache {
            key,
            tracks: std::sync::Arc::clone(&tracks),
        });
        tracks
    }
    pub(crate) fn play_playlist_from(&mut self, selected: usize, library_id: Option<u64>) {
        let tracks = self.playlist_tracks_matching(selected, "");
        let index = match library_id {
            Some(id) => tracks.iter().position(|track| track.library_id == id),
            None => (!tracks.is_empty()).then_some(0),
        };
        if let Some(index) = index {
            self.start_session(&tracks, index);
        }
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
            self.play_playlist_from(selected, Some(track.library_id));
        }
    }
    pub(crate) fn set_paused(&mut self, value: bool) {
        self.core_command(gramwave_application::commands::AppCommand::Pause(value));
    }
    pub(crate) fn toggle_mute(&mut self) {
        self.set_volume(if self.state.view().volume == 0. {
            self.state.view().unmuted_volume.max(1.)
        } else {
            0.
        });
    }
    pub(crate) fn set_volume(&mut self, value: f32) {
        self.core_command(gramwave_application::commands::AppCommand::Volume(value));
    }
    pub(crate) fn pin(&mut self, track: impl Into<gramwave_application::model::Track>) {
        self.core_command(gramwave_application::commands::AppCommand::SaveTrack(
            track.into(),
        ));
    }
    pub(crate) fn link(&self, t: &Track) {
        let Some(tg) = self.tg.clone() else { return };
        let (chat, message) = (t.chat, t.message);
        self.async_event(
            gramwave_application::tasks::JobClass::Interactive,
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
    pub(crate) fn media_track(&self, track: gramwave_application::model::Track) -> Track {
        let mut view: Track = self.state.media_track(track).into();
        view.pinned = self.state.offline().path(view.id()).map(str::to_owned);
        view
    }
    pub(crate) fn artwork_path(&self, id: i64) -> Option<&str> {
        self.state.library().artwork_path(id)
    }
    pub(crate) fn toggle_chat_pin(&mut self, id: i64) {
        self.core_command(gramwave_application::commands::AppCommand::ToggleChatPin(
            id,
        ));
    }
    pub(crate) fn process(&mut self, ctx: &egui::Context) -> usize {
        let mut processed = 0;
        if let Some((generation, status)) = self.progress.take() {
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
        if updates.len() == 128 {
            self.wake.wake();
        }
        for update in updates {
            processed += 1;
            match update {
                TelegramUpdate::Authorization(state) => self.core_event(
                    gramwave_application::workflow::AppEvent::Authorization(state),
                ),
                update => self.library_command(
                    gramwave_application::library::LibraryCommand::Source(update),
                ),
            }
        }
        self.custom_emoji
            .set_connection(self.emoji_telegram().map(|telegram| telegram.connection));
        let mut received = 0;
        for _ in 0..128 {
            let Ok(e) = self.rx.try_recv() else {
                break;
            };
            processed += 1;
            received += 1;
            match e {
                Event::Noop => {}
                Event::NextPlaybackPrepared(request, next) => {
                    self.next_playback_prepared(request, next)
                }
                Event::Cache {
                    connection,
                    job,
                    result,
                    offline_sizes,
                } => self.cache_finished(connection, job, result, offline_sizes),
                Event::ForgetArtwork(path) => ctx.forget_image(&format!("file://{path}")),
                Event::Emoji(request, assets) => self.custom_emoji.complete(request, assets),
                Event::CacheHandoff { id, path, result } => {
                    self.handoff_running = false;
                    if result.is_ok() {
                        self.handoff_retry.remove(&path);
                        self.core_event(
                            gramwave_application::workflow::AppEvent::OfflineHandoffFinished {
                                id,
                                path,
                            },
                        );
                    } else if self
                        .state
                        .offline()
                        .copies
                        .get(&id)
                        .is_some_and(|copy| copy.path == path)
                    {
                        self.handoff_retry.insert(
                            path,
                            std::time::Instant::now() + std::time::Duration::from_secs(30),
                        );
                    }
                }
                Event::Application(event) => self.core_event(event),
                Event::Error(s) => {
                    self.error = s.to_string();
                    self.error_retry = None;
                }
                Event::Link(s) => {
                    self.last_link = s.clone();
                    ctx.copy_text(s);
                    self.status = tr("Link copied");
                    self.set_inline_notice(self.status.clone(), false);
                }
                Event::SourceChatError(message) => self.set_inline_notice(message, false),
            }
        }
        if received == 128 {
            self.wake.wake();
        }
        processed
    }
}

pub(crate) fn tray_icon(size: usize) -> Vec<u8> {
    egui_extras::image::load_svg_bytes_with_size(
        crate::design::APP_ICON,
        egui::load::SizeHint::Width(size as u32),
        &Default::default(),
    )
    .expect("bundled Gramwave icon must be a valid SVG")
    .pixels
    .into_iter()
    .flat_map(|pixel| pixel.to_srgba_unmultiplied())
    .collect()
}

#[cfg(test)]
#[path = "controller/tests.rs"]
mod tests;
