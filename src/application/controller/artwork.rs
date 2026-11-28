use super::{covers::CoverKey, DesktopApp, Event, PlaylistCacheKey, Track};
use eframe::egui;
use gramwave_application::{
    library::LibraryCommand,
    media::MediaTrack,
    tasks::{JobClass, Operation, RequestId},
    AppError,
};
use std::{
    collections::{HashMap, VecDeque},
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};

const NEARBY_ROWS: usize = 30;
const SCAN_BATCH: usize = 64;
const BACKGROUND_REQUESTS: usize = 2;
const SEARCH_AVATAR_ROWS: usize = 64;
const SEARCH_AVATAR_REQUESTS: usize = 4;
const HEAD_ROWS: usize = 24;
const HEAD_RETRY: Duration = Duration::from_secs(15);
const HEAD_FRESH: Duration = crate::telegram::PLAYLIST_HEAD_AGE;

pub(super) struct PlaylistView {
    key: PlaylistCacheKey,
    tracks: Arc<Vec<Track>>,
    visible: Range<usize>,
}

fn nearby_rows(visible: Range<usize>, len: usize, count: usize) -> impl Iterator<Item = usize> {
    (0..count).flat_map(move |distance| {
        [
            visible.end.checked_add(distance).filter(|&i| i < len),
            visible.start.checked_sub(distance + 1),
        ]
        .into_iter()
        .flatten()
    })
}

#[derive(Default)]
struct SearchArtwork {
    chats: Vec<i64>,
    visible: Option<Range<usize>>,
    next: usize,
    catalog_running: bool,
}

#[derive(Default)]
struct Hover {
    observation: Option<(i64, bool, bool)>,
    candidate: Option<(i64, Instant)>,
    targets: VecDeque<i64>,
}

impl Hover {
    fn update(&mut self, now: Instant) -> Option<Duration> {
        let Some((chat, fast, scrolling)) = self.observation else {
            self.candidate = None;
            return None;
        };
        let candidate = self.candidate.get_or_insert((chat, now));
        if chat != candidate.0 || scrolling {
            *candidate = (chat, now);
        }
        let delay = Duration::from_millis(if fast { 85 } else { 45 });
        let remaining = delay.saturating_sub(now.saturating_duration_since(candidate.1));
        if remaining.is_zero() {
            self.targets.retain(|&id| id != chat);
            self.targets.push_front(chat);
        }
        Some(remaining)
    }
}

#[derive(Default)]
pub(super) struct Prefetch {
    connection: Option<u64>,
    scope: Option<(u64, PlaylistCacheKey)>,
    next: usize,
    unfiltered: Option<(PlaylistCacheKey, Arc<Vec<Track>>)>,
    head_tracks: HashMap<i64, Vec<Track>>,
    head_tracks_version: u64,
    hover: Hover,
    search: SearchArtwork,
    heads: HashMap<(i64, usize), Operation>,
    pages: HashMap<(i64, usize), (Instant, Vec<MediaTrack>)>,
}

impl DesktopApp {
    pub(crate) fn clear_hover_observation(&mut self) {
        self.artwork_prefetch.hover.observation = None;
        self.artwork_prefetch.search.visible = None;
        self.covers.get_mut().begin_frame();
    }

    pub(crate) fn note_chat_hover(&mut self, chat: i64, fast: bool, scrolling: bool) {
        self.artwork_prefetch.hover.observation = Some((chat, fast, scrolling));
    }

    pub(crate) fn note_search_artwork(&mut self, chats: Vec<i64>, visible: Range<usize>) {
        let catalog_running = self.state.library().catalog_running();
        let search = &mut self.artwork_prefetch.search;
        if search.chats != chats || search.catalog_running != catalog_running {
            search.next = 0;
            search.chats = chats;
            search.catalog_running = catalog_running;
        }
        search.visible = Some(visible);
    }

    pub(crate) fn note_playlist_view(
        &mut self,
        selected: usize,
        tracks: Arc<Vec<Track>>,
        visible: Range<usize>,
    ) {
        self.artwork_view = Some(PlaylistView {
            key: self.playlist_cache_key(selected, &self.search),
            tracks,
            visible,
        });
    }

    pub(super) fn request_playlist_artwork(
        &mut self,
        ctx: &egui::Context,
        view: Option<PlaylistView>,
        now: Instant,
    ) {
        let connection = self.tg.as_ref().map(|tg| tg.connection);
        if self.artwork_prefetch.connection != connection {
            for operation in self.artwork_prefetch.heads.values() {
                operation.cancel.cancel();
            }
            self.artwork_prefetch.heads.clear();
            self.artwork_prefetch.pages.clear();
            self.artwork_prefetch.search.next = 0;
            self.artwork_prefetch.connection = connection;
        }
        if let Some(delay) = self
            .artwork_prefetch
            .hover
            .update(now)
            .filter(|delay| !delay.is_zero())
        {
            ctx.request_repaint_after(delay);
        }
        if !self.chat_search_active {
            self.artwork_prefetch
                .hover
                .targets
                .retain(|chat| Some(*chat) == self.opened_chat);
        }
        while self.artwork_prefetch.hover.targets.len() > 2 {
            let old = self.artwork_prefetch.hover.targets.pop_back().unwrap();
            if Some(old) != self.opened_chat {
                self.artwork_prefetch.heads.retain(|(chat, _), operation| {
                    if *chat == old {
                        operation.cancel.cancel();
                        false
                    } else {
                        true
                    }
                });
            }
        }
        self.artwork_prefetch
            .pages
            .retain(|_, (at, _)| now.saturating_duration_since(*at) < HEAD_FRESH);
        let current = self
            .state
            .view()
            .session
            .current()
            .map(|track| track.into());
        if let Some(track) = current {
            self.prefetch_cover(ctx, &track, now, true);
        }
        let hover: Vec<_> = self
            .artwork_prefetch
            .hover
            .targets
            .iter()
            .copied()
            .collect();
        if let Some((chat, _, false)) = self.artwork_prefetch.hover.observation {
            self.warm_chat_art(ctx, chat, now, false);
        }
        if let Some(&chat) = hover.first() {
            self.warm_chat_art(ctx, chat, now, true);
            let priority = self
                .artwork_prefetch
                .hover
                .observation
                .is_some_and(|(id, _, scrolling)| id == chat && !scrolling);
            self.request_head(chat, priority);
        }
        self.warm_search_artwork(ctx, now);
        if let Some(view) = &view {
            for i in nearby_rows(view.visible.clone(), view.tracks.len(), NEARBY_ROWS) {
                self.prefetch_cover(ctx, &view.tracks[i], now, true);
            }
        }
        let timeline = self.state.view().session.timeline();
        let upcoming: Vec<Track> = (timeline.upcoming_start()..timeline.len())
            .take(HEAD_ROWS)
            .filter_map(|i| timeline.get(i).map(Into::into))
            .collect();
        for track in upcoming {
            self.prefetch_cover(ctx, &track, now, true);
        }
        if let Some(&chat) = hover.get(1) {
            self.warm_chat_art(ctx, chat, now, true);
            let current_running = hover.first().is_some_and(|current| {
                self.artwork_prefetch
                    .heads
                    .keys()
                    .any(|(id, _)| id == current)
            });
            self.request_head(chat, current_running);
        }
        let state = self.state.view();
        let mut heads: Vec<_> = state
            .chat_pins
            .iter()
            .chain(&state.recent_chats)
            .copied()
            .collect();
        let mut seen = std::collections::HashSet::new();
        heads.retain(|chat| seen.insert(*chat) && !hover.contains(chat));
        for chat in heads {
            self.warm_chat_art(ctx, chat, now, true);
            self.request_head(chat, false);
        }
        if let Some(view) = view {
            let mut key = view.key.clone();
            key.search.clear();
            let tracks = if view.key.search.is_empty() {
                view.tracks
            } else {
                let unfiltered = &mut self.artwork_prefetch.unfiltered;
                if unfiltered.as_ref().is_none_or(|(old, _)| *old != key) {
                    let tracks = self
                        .state
                        .view()
                        .playlists
                        .iter()
                        .find(|playlist| playlist.id == key.id)
                        .into_iter()
                        .flat_map(|playlist| playlist.tracks.iter())
                        .filter(|track| key.filters[usize::from(track.kind)])
                        .cloned()
                        .map(Into::into)
                        .collect();
                    *unfiltered = Some((key.clone(), Arc::new(tracks)));
                }
                unfiltered.as_ref().unwrap().1.clone()
            };
            let scope = (self.state.library().connection(), key);
            if self.artwork_prefetch.scope.as_ref() != Some(&scope) {
                self.artwork_prefetch.scope = Some(scope);
                self.artwork_prefetch.next = 0;
            }
            let start = self.artwork_prefetch.next;
            let end = (start + SCAN_BATCH).min(tracks.len());
            for i in start..end {
                if self.prefetch_cover(ctx, &tracks[i], now, true)
                    && self.artwork_prefetch.next == i
                {
                    self.artwork_prefetch.next += 1;
                }
            }
            if self.artwork_prefetch.next == end && end < tracks.len() {
                ctx.request_repaint();
            }
        } else {
            self.artwork_prefetch.scope = None;
        }
    }

    fn warm_search_artwork(&mut self, ctx: &egui::Context, now: Instant) {
        let Some(visible) = self
            .artwork_prefetch
            .search
            .visible
            .clone()
            .filter(|_| self.chat_search_active)
        else {
            return;
        };
        let mut search = std::mem::take(&mut self.artwork_prefetch.search);
        let len = search.chats.len();
        for i in visible
            .clone()
            .chain(nearby_rows(visible, len, SEARCH_AVATAR_ROWS))
        {
            self.warm_chat_cover(ctx, search.chats[i], now, SEARCH_AVATAR_REQUESTS);
        }
        let end = (search.next + SCAN_BATCH).min(len);
        for i in search.next..end {
            if self.warm_chat_cover(ctx, search.chats[i], now, BACKGROUND_REQUESTS)
                && search.next == i
            {
                search.next += 1;
            }
        }
        if search.next == end && end < len {
            ctx.request_repaint();
        }
        self.artwork_prefetch.search = search;
    }

    fn warm_chat_cover(
        &mut self,
        ctx: &egui::Context,
        chat: i64,
        now: Instant,
        limit: usize,
    ) -> bool {
        let _ = self.chat_cover(ctx, chat);
        let library = self.state.library();
        let art = library.playlist_artwork(chat);
        let force = art
            .path
            .as_deref()
            .is_some_and(|path| self.cover_needs_file(path));
        if let Some(path) = art.path.as_deref().filter(|_| !force) {
            return self.cover_decode_ready(path) || self.cover_decode_failed(path);
        }
        if art.file <= 0
            || self.tg.is_none()
            || !library.connected()
            || library
                .artwork_retry_at(art.file)
                .is_some_and(|at| at > now)
        {
            return true;
        }
        if library.media_requests_in_flight() < limit {
            self.library_command(LibraryCommand::RequestArtwork {
                id: art.file,
                now,
                force,
            });
        }
        false
    }

    fn warm_chat_art(&mut self, ctx: &egui::Context, chat: i64, now: Instant, network: bool) {
        self.warm_chat_cover(
            ctx,
            chat,
            now,
            if network { BACKGROUND_REQUESTS } else { 0 },
        );
        for track in self.chat_head_tracks(chat) {
            self.prefetch_cover(ctx, &track, now, network);
        }
        let mut preview: Vec<_> = (0..3)
            .filter(|&kind| playlist_filter(&self.state, chat, kind))
            .filter_map(|kind| self.artwork_prefetch.pages.get(&(chat, kind)))
            .flat_map(|(_, tracks)| tracks.iter())
            .collect();
        preview.sort_by_key(|track| std::cmp::Reverse(track.message));
        let preview: Vec<_> = preview.into_iter().take(HEAD_ROWS).cloned().collect();
        for track in preview {
            if self.state.library().source_changed(track.id()) {
                continue;
            }
            let path = self
                .artwork_path(track.image_file)
                .or(track.image_path.as_deref());
            let _ = self.cover_texture(ctx, CoverKey::Track(track.id()), path, false);
            if network
                && path.is_none()
                && self.state.library().media_requests_in_flight() < BACKGROUND_REQUESTS
            {
                self.library_command(LibraryCommand::RequestArtwork {
                    id: track.image_file,
                    now,
                    force: false,
                });
            }
        }
    }

    fn chat_head_tracks(&mut self, chat: i64) -> Vec<Track> {
        let version = self.state.library_version();
        let prefetch = &mut self.artwork_prefetch;
        if prefetch.head_tracks_version != version {
            prefetch.head_tracks_version = version;
            prefetch.head_tracks.clear();
        }
        if let Some(tracks) = prefetch.head_tracks.get(&chat) {
            return tracks.clone();
        }
        let Some(playlist) = self.state.view().playlists.iter().find(|p| p.id == chat) else {
            return Vec::new();
        };
        let enabled = playlist
            .tracks
            .iter()
            .filter(|track| playlist.filters[usize::from(track.kind)]);
        let tracks: Vec<_> = enabled.take(HEAD_ROWS).cloned().map(Into::into).collect();
        if prefetch.head_tracks.len() < 64 {
            prefetch.head_tracks.insert(chat, tracks.clone());
        }
        tracks
    }

    pub(crate) fn warm_initial_covers(&mut self, ctx: &egui::Context) {
        self.covers.get_mut().begin_frame();
        if let Some(track) = self.state.view().session.current() {
            let _ = self.track_cover(ctx, &track.into());
        }
        if let Some(chat) = self.opened_chat {
            let _ = self.chat_cover(ctx, chat);
            for track in self.chat_head_tracks(chat) {
                let _ = self.track_cover(ctx, &track);
            }
        }
        self.pump_cover_decodes(ctx);
    }

    fn request_head(&mut self, chat: i64, priority: bool) {
        let Some(tg) = self.tg.clone() else { return };
        if !self.state.library().connected()
            || self.state.library().indexing_chat() == Some(chat)
            || self.artwork_prefetch.heads.len() >= if priority { 2 } else { 1 }
            || self
                .artwork_prefetch
                .heads
                .keys()
                .any(|(id, _)| *id == chat)
        {
            return;
        }
        let Some(playlist) = self.state.view().playlists.iter().find(|p| p.id == chat) else {
            return;
        };
        let kind = (0..3).find(|&kind| {
            playlist.filters[kind]
                && (!playlist.done[kind] || playlist.cursors[kind] != 0)
                && !playlist
                    .tracks
                    .iter()
                    .any(|track| usize::from(track.kind) == kind)
                && !self.artwork_prefetch.pages.contains_key(&(chat, kind))
        });
        let Some(kind) = kind else { return };
        let connection = tg.connection;
        let operation = Operation::new(connection);
        let key = operation.key();
        let cancel = operation.cancel.clone();
        self.artwork_prefetch
            .heads
            .insert((chat, kind), operation.clone());
        let reserved = tg.reserve_playlist_head(chat, crate::discovery::MEDIA_FILTERS[kind]);
        self.operation_event(
            JobClass::Background,
            operation,
            async move {
                let result =
                    crate::discovery::warm_playlist_head(&tg, chat, kind, &reserved, &cancel).await;
                Event::PlaylistHead(connection, chat, kind, key, result)
            },
            move |error| Event::PlaylistHead(connection, chat, kind, key, Err(error)),
        );
    }

    pub(super) fn playlist_head_finished(
        &mut self,
        connection: u64,
        chat: i64,
        kind: usize,
        request: RequestId,
        result: Result<Vec<MediaTrack>, AppError>,
    ) {
        let pending = self.artwork_prefetch.heads.get(&(chat, kind));
        if self.tg.as_ref().map(|tg| tg.connection) != Some(connection)
            || !pending.is_some_and(|operation| operation.key() == request)
        {
            return;
        }
        self.artwork_prefetch.heads.remove(&(chat, kind));
        let now = Instant::now();
        if self.artwork_prefetch.pages.len() >= 48 {
            if let Some(oldest) = self
                .artwork_prefetch
                .pages
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(key, _)| *key)
            {
                self.artwork_prefetch.pages.remove(&oldest);
            }
        }
        let (at, tracks) = match result {
            Ok(tracks) => (now, tracks),
            Err(_) => (now - (HEAD_FRESH - HEAD_RETRY), Vec::new()),
        };
        self.artwork_prefetch
            .pages
            .insert((chat, kind), (at, tracks));
    }

    fn prefetch_cover(
        &mut self,
        ctx: &egui::Context,
        track: &Track,
        now: Instant,
        network: bool,
    ) -> bool {
        let track = self.media_track(track.metadata());
        let ready = self.track_cover(ctx, &track).is_some();
        let library = self.state.library();
        let command = if track.file != 0 {
            let id = track.image_file;
            if id <= 0 || library.artwork_retry_at(id).is_some_and(|at| at > now) {
                return true;
            }
            if let Some(path) = track.image_path.as_deref() {
                if !self.cover_needs_file(path) {
                    return ready || self.cover_decode_failed(path);
                }
            }
            LibraryCommand::RequestArtwork {
                id,
                now,
                force: track.image_path.is_some(),
            }
        } else {
            if library
                .media_retry_at(track.id())
                .is_some_and(|at| at > now)
            {
                return true;
            }
            LibraryCommand::RequestMedia {
                track: track.metadata(),
                now,
            }
        };
        if network
            && self.tg.is_some()
            && library.connected()
            && library.media_requests_in_flight() < BACKGROUND_REQUESTS
        {
            self.library_command(command);
        }
        false
    }
}

fn playlist_filter(state: &gramwave_application::AppCore, chat: i64, kind: usize) -> bool {
    state
        .view()
        .playlists
        .iter()
        .any(|p| p.id == chat && p.filters[kind])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::telegram::Telegram;
    use gramwave_application::{
        model::{Playlist, State},
        AppCore,
    };
    use serde_json::json;
    use std::{thread, time::Duration};

    #[test]
    fn hover_promotes_the_new_target_and_reuses_the_previous_one() {
        let now = Instant::now();
        let mut hover = Hover::default();
        let mut update = |chat, millis, fast, scroll| {
            hover.observation = Some((chat, fast, scroll));
            hover.update(now + Duration::from_millis(millis));
            hover.targets.clone()
        };
        assert!(update(1, 0, true, false).is_empty());
        assert!(update(2, 40, true, false).is_empty());
        assert!(update(2, 120, true, false).is_empty());
        assert_eq!(update(2, 125, true, false), [2]);
        assert_eq!(update(1, 130, false, true), [2]);
        assert_eq!(update(1, 200, false, true), [2]);
        assert_eq!(update(1, 245, false, false), [1, 2]);
        assert_eq!(update(2, 250, false, false), [1, 2]);
        assert_eq!(update(2, 295, false, false), [2, 1]);
    }

    #[test]
    fn opening_a_warmed_chat_exposes_tracks_before_its_refresh_completes() {
        use gramwave_application::commands::AppCommand;

        let mut app = DesktopApp::new_at(Default::default(), true);
        app.state = AppCore::new(State {
            playlists: vec![Playlist {
                id: 42,
                title: "Warm".into(),
                url: "https://t.me/example".into(),
                filters: [true, false, false],
                ..Default::default()
            }],
            ..Default::default()
        });
        let (send, calls) = std::sync::mpsc::channel();
        let tg = Telegram::mock_rpc(move |request| {
            if request["@type"] == "getChatSparseMessagePositions" {
                return json!({"total_count":0,"positions":[]});
            }
            send.send(request["@type"].as_str().unwrap().to_owned())
                .unwrap();
            match request["@type"].as_str() {
                Some("searchChatMessages") => json!({"messages": [{
                    "id": 53,"content":{"@type":"messageAudio","audio":{"audio":{"id":54}}}
                }, {
                    "id": 51,"content":{"@type":"messageAudio","audio":{"audio":{"id":52}}}
                }], "next_from_message_id": 0}),
                _ => panic!("Unexpected request {request}"),
            }
        });
        app.library_command(LibraryCommand::Connected(tg.connection));
        app.tg = Some(tg);
        let ctx = egui::Context::default();
        app.chat_search_active = true;
        app.note_chat_hover(42, false, false);
        let now = Instant::now();
        app.request_playlist_artwork(&ctx, None, now);
        app.request_playlist_artwork(&ctx, None, now + Duration::from_millis(45));
        let deadline = now + Duration::from_secs(3);
        while app.artwork_prefetch.heads.contains_key(&(42, 0)) {
            app.process(&ctx);
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        assert!(app.state.view().playlists[0].tracks.is_empty());
        app.library_command(LibraryCommand::Source(
            gramwave_application::telegram::TelegramUpdate::Deleted {
                chat: 42,
                messages: vec![53],
                from_cache: false,
            },
        ));
        app.execute(AppCommand::OpenSearchPlaylist(42));
        assert_eq!(app.state.view().playlists[0].tracks.len(), 1);
        assert_eq!(app.state.view().playlists[0].tracks[0].message, 51);
        while app.state.library().indexing_chat().is_some() {
            app.process(&ctx);
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(calls.try_iter().collect::<Vec<_>>(), ["searchChatMessages"]);
    }

    #[test]
    fn prefetch_prioritizes_a_jump_and_finishes_the_open_playlist() {
        const TRACKS: i64 = 104;
        let mut app = DesktopApp::new_at(Default::default(), true);
        app.state = AppCore::new(State {
            playlists: vec![Playlist {
                id: 42,
                filters: [true; 3],
                done: [true; 3],
                tracks: (1..=TRACKS)
                    .map(|message| gramwave_application::model::Track {
                        chat: 42,
                        message,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        });
        let cover = tempfile::NamedTempFile::new().unwrap();
        let path = cover.path().to_string_lossy().into_owned();
        let (send, downloads) = std::sync::mpsc::channel();
        let tg = Telegram::mock_rpc(move |request| match request["@type"].as_str() {
            Some("getMessage") => {
                assert_eq!(request["chat_id"], 42);
                let message = request["message_id"].as_i64().unwrap();
                json!({"id":message,"content":{"@type":"messageAudio","audio":{
                    "audio":{"id":2000+message},"album_cover_thumbnail":{"file":{"id":1000+message}}
                }}})
            }
            Some("downloadFile") => {
                let id = request["file_id"].as_i64().unwrap();
                assert!((1001..=1000 + TRACKS).contains(&id));
                send.send(id).unwrap();
                json!({"@type":"file","id":id,"local":{"path":path,"is_downloading_completed":true}})
            }
            _ => panic!("Unexpected request {request}"),
        });
        app.library_command(LibraryCommand::Connected(tg.connection));
        app.tg = Some(tg);
        let ctx = egui::Context::default();
        let tracks = app.playlist_tracks(0);
        let tick = |app: &mut DesktopApp, visible: Range<usize>| {
            let now = Instant::now();
            app.process(&ctx);
            for track in &tracks[visible.clone()] {
                let view = app.media_track(track.metadata());
                app.image_wanted.push(view.image_file);
                if view.file == 0 {
                    app.track_wanted.push(view);
                }
            }
            app.note_playlist_view(0, tracks.clone(), visible);
            app.request_visible_media(&ctx);
            now
        };
        tick(&mut app, 0..1);
        let deadline = Instant::now() + Duration::from_secs(8);
        while app.artwork_path(1081).is_none() {
            assert!(tick(&mut app, 80..81) < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(
            app.artwork_path(1048).is_none(),
            "distant background row ran ahead of the jump"
        );
        while app.artwork_prefetch.next < TRACKS as usize {
            assert!(tick(&mut app, 80..81) < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        let mut ids: Vec<_> = downloads.try_iter().collect();
        ids.sort_unstable();
        assert_eq!(ids, (1001..=1000 + TRACKS).collect::<Vec<_>>());
        assert!(Arc::ptr_eq(&tracks, &app.playlist_tracks(0)));
    }
}
