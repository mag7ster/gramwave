//! Library operations and connection-scoped media. Desktop code only reads these views.
use crate::{
    media::{MediaPatch, MediaPlaylist, MediaTrack, RuntimeArtwork, RuntimeMedia},
    model::{Playlist, Track},
    tasks::{Operation, RequestId},
    telegram::{PersonalChatList, TelegramFile, TelegramUpdate},
    AppError,
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};
pub(crate) const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
use tg_player_domain::TrackId;

pub enum LibraryCommand {
    Connected(u64),
    InvalidateConnection,
    RefreshCatalog,
    Search {
        query: String,
        now: Instant,
        force: bool,
    },
    Open {
        chat: i64,
        refresh: bool,
    },
    RequestMedia {
        track: Track,
        now: Instant,
    },
    RequestArtwork {
        id: i64,
        now: Instant,
        force: bool,
    },
    MergeChat(MediaPlaylist),
    Source(TelegramUpdate),
}
pub enum LibraryEvent {
    Catalog {
        request: RequestId,
        result: Result<ChatCatalog, AppError>,
    },
    Search {
        request: RequestId,
        query: String,
        result: Result<Vec<MediaPlaylist>, AppError>,
    },
    Indexed {
        request: RequestId,
        result: Result<(), AppError>,
    },
    Page {
        request: RequestId,
        playlist: MediaPlaylist,
    },
    Patch {
        request: RequestId,
        patch: MediaPatch,
    },
    Media {
        request: RequestId,
        now: Instant,
        connection: u64,
        id: TrackId,
        revision: u64,
        result: Result<MediaTrack, AppError>,
    },
    Artwork {
        request: RequestId,
        now: Instant,
        connection: u64,
        id: i64,
        result: Result<String, AppError>,
    },
}
#[derive(Debug, Default)]
pub struct ChatCatalog {
    pub chats: Vec<MediaPlaylist>,
    pub lists: [HashSet<i64>; 2],
    pub saved_messages: Option<i64>,
}
pub enum LibraryEffect {
    Media {
        operation: Operation,
        connection: u64,
        track: Track,
        revision: u64,
    },
    Artwork {
        operation: Operation,
        connection: u64,
        id: i64,
    },
    Catalog {
        operation: Operation,
        filters: [bool; 3],
    },
    Search {
        operation: Operation,
        query: String,
        filters: [bool; 3],
    },
    Index {
        operation: Operation,
        playlist: Playlist,
        refresh: bool,
    },
    Failed {
        error: AppError,
        retry: LibraryRetry,
    },
    ForgetArtwork(String),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryRetry {
    Catalog,
    Chat(i64),
}
#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub ids: Vec<i64>,
    pub error: Option<AppError>,
    pub sent: bool,
    pub(crate) since: Option<Instant>,
    pub(crate) operation: Option<Operation>,
}
impl SearchState {
    pub fn loading(&self) -> bool {
        self.operation.is_some()
    }
}
pub(crate) struct IndexOperation {
    pub operation: Operation,
    pub chat: i64,
}
pub(crate) struct MediaOperation {
    pub operation: Operation,
    pub connection: u64,
    pub revision: u64,
}
#[derive(Default)]
pub struct LibraryCore {
    version: u64,
    media_version: u64,
    generation: u64,
    connection: Option<u64>,
    pub(crate) catalog: Option<Operation>,
    pub(crate) catalog_updates: HashMap<(i64, PersonalChatList), bool>,
    pub(crate) index: Option<IndexOperation>,
    pub(crate) search: SearchState,
    pub(crate) sources: HashSet<TrackId>,
    revisions: HashMap<TrackId, u64>,
    runtime: HashMap<TrackId, RuntimeMedia>,
    artwork: HashMap<i64, String>,
    chat_artwork: HashMap<i64, RuntimeArtwork>,
    pub(crate) media_requests: HashMap<TrackId, MediaOperation>,
    pub(crate) artwork_requests: HashMap<i64, Operation>,
    pub(crate) media_retry: HashMap<TrackId, Instant>,
    pub(crate) artwork_retry: HashMap<i64, Instant>,
}
impl LibraryCore {
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn media_version(&self) -> u64 {
        self.media_version
    }
    pub fn connection(&self) -> u64 {
        self.connection.unwrap_or(0)
    }
    pub fn connected(&self) -> bool {
        self.connection.is_some()
    }
    pub fn catalog_running(&self) -> bool {
        self.catalog.is_some()
    }
    pub fn indexing_chat(&self) -> Option<i64> {
        self.index.as_ref().map(|index| index.chat)
    }
    pub fn search(&self) -> &SearchState {
        &self.search
    }
    pub fn search_deadline(&self) -> Option<Instant> {
        if self.connected() && !self.search.sent && !self.search.query.is_empty() {
            self.search.since.map(|since| since + SEARCH_DEBOUNCE)
        } else {
            None
        }
    }
    pub fn media_retry_at(&self, id: TrackId) -> Option<Instant> {
        self.media_retry.get(&id).copied()
    }
    pub fn media_requests_in_flight(&self) -> usize {
        self.media_requests.len() + self.artwork_requests.len()
    }
    pub fn artwork_retry_at(&self, id: i64) -> Option<Instant> {
        self.artwork_retry.get(&id).copied()
    }
    pub fn revision(&self, id: TrackId) -> u64 {
        self.revisions.get(&id).copied().unwrap_or(0)
    }
    pub fn source_changed(&self, id: TrackId) -> bool {
        self.sources.contains(&id)
    }
    pub fn artwork_path(&self, id: i64) -> Option<&str> {
        self.artwork.get(&id).map(String::as_str)
    }
    pub fn runtime_media(&self, id: TrackId) -> Option<&RuntimeMedia> {
        self.runtime.get(&id)
    }
    pub fn media_track(&self, track: Track) -> MediaTrack {
        let mut track = MediaTrack::from(track);
        if let Some(media) = self.runtime.get(&track.id()) {
            track.file = media.file;
            track.cached = media.cached.clone();
            track.image_file = media.image_file;
            track.image_path = media.image_path.clone();
        }
        track.image_path = self
            .artwork_path(track.image_file)
            .map(str::to_owned)
            .or(track.image_path);
        track
    }
    pub fn playlist_artwork(&self, chat: i64) -> RuntimeArtwork {
        let mut art = self.chat_artwork.get(&chat).cloned().unwrap_or_default();
        art.path = self.artwork_path(art.file).map(str::to_owned).or(art.path);
        art
    }
    pub(crate) fn cache_playlist_artwork(&mut self, playlist: &MediaPlaylist) {
        let mut artwork = playlist.artwork();
        self.register_local_artwork(artwork.file, &mut artwork.path);
        self.chat_artwork.insert(playlist.id, artwork);
        self.media_version = self.media_version.wrapping_add(1);
    }
    pub(crate) fn changed(&mut self) {
        self.version = self.version.wrapping_add(1);
    }
    pub(crate) fn invalidate_track(&mut self, id: TrackId) {
        *self.revisions.entry(id).or_default() += 1;
        self.sources.insert(id);
        if let Some(pending) = self.media_requests.remove(&id) {
            pending.operation.cancel.cancel();
        }
        self.media_retry.remove(&id);
        self.runtime.remove(&id);
        self.media_version = self.media_version.wrapping_add(1);
    }
    pub(crate) fn protected(&self, chat: i64) -> HashSet<i64> {
        self.sources
            .iter()
            .filter(|id| id.chat == chat)
            .map(|id| id.message)
            .collect()
    }
    pub(crate) fn operation(&mut self) -> Operation {
        self.generation = self.generation.wrapping_add(1);
        Operation::new(self.generation)
    }
    pub(crate) fn connect(&mut self, connection: u64) {
        if self.connection == Some(connection) {
            return;
        }
        self.invalidate();
        self.connection = Some(connection);
    }
    pub(crate) fn cancel_operations(&mut self) {
        if let Some(op) = self.catalog.take() {
            op.cancel.cancel();
        }
        self.catalog_updates.clear();
        self.cancel_index();
        if let Some(op) = self.search.operation.take() {
            op.cancel.cancel();
        }
        self.search.sent = false;
        for (_, pending) in self.media_requests.drain() {
            pending.operation.cancel.cancel();
        }
        for (_, operation) in self.artwork_requests.drain() {
            operation.cancel.cancel();
        }
    }
    pub(crate) fn cancel_index(&mut self) {
        if let Some(index) = self.index.take() {
            index.operation.cancel.cancel();
        }
    }
    pub(crate) fn invalidate(&mut self) {
        self.cancel_operations();
        self.connection = None;
        self.sources.clear();
        self.revisions.clear();
        self.media_retry.clear();
        self.artwork_retry.clear();
        self.runtime.clear();
        self.artwork.clear();
        self.chat_artwork.clear();
        self.search.ids.clear();
        self.search.error = None;
        self.media_version = self.media_version.wrapping_add(1);
    }
    pub(crate) fn accepts_catalog(&self, request: RequestId) -> bool {
        self.catalog.as_ref().is_some_and(|op| op.key() == request)
    }
    pub(crate) fn accepts_index(&self, request: RequestId, chat: Option<i64>) -> bool {
        self.index.as_ref().is_some_and(|index| {
            index.operation.key() == request && chat.is_none_or(|chat| chat == index.chat)
        })
    }
    pub(crate) fn cache_track(&mut self, track: &MediaTrack) {
        // A metadata-only update cannot replace valid connection-local media.
        if track.file == 0
            && track.cached.is_none()
            && track.image_file == 0
            && track.image_path.is_none()
        {
            return;
        }
        let mut media = track.runtime();
        self.register_local_artwork(media.image_file, &mut media.image_path);
        self.runtime.insert(track.id(), media);
        self.media_version = self.media_version.wrapping_add(1);
    }
    fn register_local_artwork(&mut self, id: i64, path: &mut Option<String>) {
        if id > 0 {
            if let Some(path) = path
                .take()
                .filter(|_| !self.artwork_requests.contains_key(&id))
            {
                self.artwork.entry(id).or_insert(path);
            }
        }
    }
    pub(crate) fn forget_artwork(&mut self, id: i64) -> Option<String> {
        let path = self.artwork.remove(&id)?;
        self.media_version = self.media_version.wrapping_add(1);
        Some(path)
    }
    pub(crate) fn cache_artwork(&mut self, id: i64, path: String) -> Option<String> {
        if self.artwork.get(&id) == Some(&path) {
            return None;
        }
        self.media_version = self.media_version.wrapping_add(1);
        self.artwork.insert(id, path)
    }
    pub(crate) fn update_file(&mut self, file: TelegramFile) -> Option<String> {
        let evicted = !file.complete && file.path.is_none();
        let mut forgotten = None;
        if file.complete {
            if let Some(path) = &file.path {
                forgotten = self.cache_artwork(file.id, path.clone());
            }
        } else if evicted {
            forgotten = self.forget_artwork(file.id);
            self.artwork_retry.remove(&file.id);
        }
        let mut changed = false;
        for media in self.runtime.values_mut() {
            if media.file == file.id {
                let cached = if file.complete {
                    file.path.clone()
                } else {
                    None
                };
                if media.cached != cached {
                    media.cached = cached;
                    changed = true;
                }
            }
        }
        if changed {
            self.media_version = self.media_version.wrapping_add(1);
        }
        forgotten
    }
}

pub(crate) fn metadata_track(track: MediaTrack) -> Track {
    track.into()
}
