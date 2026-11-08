use crate::{
    core::{AppCore, CoreCommand, LibraryPatch},
    library::{
        metadata_track, IndexOperation, LibraryCommand, LibraryCore, LibraryEffect, LibraryEvent,
        LibraryRetry, MediaOperation,
    },
    media::{MediaPlaylist, MediaTrack},
    model::Track,
    telegram::TelegramUpdate,
    workflow::Effect,
};
use std::{collections::HashSet, time::Duration};

impl AppCore {
    pub fn library(&self) -> &LibraryCore {
        &self.library
    }
    pub fn cancel_library_operations(&mut self) {
        self.library.cancel_operations();
    }
    pub fn library_command(&mut self, command: LibraryCommand) -> Vec<Effect> {
        let mut effects = Vec::new();
        match command {
            LibraryCommand::Connected(connection) => self.library.connect(connection),
            LibraryCommand::InvalidateConnection => self.library.invalidate(),
            LibraryCommand::RefreshCatalog => {
                if self.library.connected() && !self.library.catalog_running() {
                    let operation = self.library.operation();
                    self.library.catalog = Some(operation.clone());
                    effects.extend([
                        Effect::Notice(crate::notice::Notice::LoadingChats),
                        Effect::Library(LibraryEffect::Catalog {
                            operation,
                            filters: self.state.filters,
                        }),
                    ]);
                }
            }
            LibraryCommand::Search { query, now, force } => {
                let query = query.trim().to_owned();
                if self.library.search.query != query {
                    if let Some(op) = self.library.search.operation.take() {
                        op.cancel.cancel();
                    }
                    self.library.search.query = query.clone();
                    self.library.search.since = Some(now);
                    self.library.search.sent = false;
                    self.library.search.ids.clear();
                    self.library.search.error = None;
                }
                if !query.is_empty()
                    && self.library.connected()
                    && (!self.library.search.sent || force)
                    && (force
                        || self.library.search.since.is_none_or(|since| {
                            now.saturating_duration_since(since) >= Duration::from_millis(250)
                        }))
                {
                    if let Some(op) = self.library.search.operation.take() {
                        op.cancel.cancel();
                    }
                    let operation = self.library.operation();
                    self.library.search.operation = Some(operation.clone());
                    self.library.search.sent = true;
                    self.library.search.error = None;
                    effects.push(Effect::Library(LibraryEffect::Search {
                        operation,
                        query,
                        filters: self.state.filters,
                    }));
                }
            }
            LibraryCommand::Open { chat, refresh } => {
                self.library.cancel_index();
                if refresh {
                    self.handle(CoreCommand::Recover(chat));
                }
                if let Some(playlist) = self.state.playlists.iter().find(|p| p.id == chat).cloned()
                {
                    if self.library.connected() && (refresh || playlist.has_more()) {
                        let operation = self.library.operation();
                        self.library.index = Some(IndexOperation {
                            operation: operation.clone(),
                            chat,
                        });
                        effects.push(Effect::Library(LibraryEffect::Index {
                            operation,
                            playlist,
                            refresh,
                        }));
                    }
                }
            }
            LibraryCommand::RequestMedia { track, now } => {
                let id = track.id();
                if self.library.connected()
                    && self.library.media_requests.len() < 4
                    && !self.library.media_requests.contains_key(&id)
                    && self
                        .library
                        .runtime_media(id)
                        .is_none_or(|media| media.file == 0)
                    && self
                        .library
                        .media_retry
                        .get(&id)
                        .is_none_or(|retry| *retry <= now)
                {
                    let operation = self.library.operation();
                    let connection = self.library.connection();
                    let revision = self.library.revision(id);
                    self.library.media_requests.insert(
                        id,
                        MediaOperation {
                            operation: operation.clone(),
                            connection,
                            revision,
                        },
                    );
                    effects.push(Effect::Library(LibraryEffect::Media {
                        operation,
                        connection,
                        track,
                        revision,
                    }));
                }
            }
            LibraryCommand::RequestArtwork { id, now, force } => {
                if id != 0
                    && self.library.connected()
                    && self.library.artwork_requests.len() < 4
                    && !self.library.artwork_requests.contains_key(&id)
                    && (force || self.library.artwork_path(id).is_none())
                    && self
                        .library
                        .artwork_retry
                        .get(&id)
                        .is_none_or(|retry| *retry <= now)
                {
                    let operation = self.library.operation();
                    let connection = self.library.connection();
                    self.library.artwork_requests.insert(id, operation.clone());
                    effects.push(Effect::Library(LibraryEffect::Artwork {
                        operation,
                        connection,
                        id,
                    }));
                }
            }
            LibraryCommand::MergeChat(playlist) => self.merge_library_chat(playlist),
            LibraryCommand::Source(update) => match update {
                TelegramUpdate::File(file) => self.library.update_file(file),
                TelegramUpdate::Chat(mut playlist) => {
                    playlist.filters = self.state.filters;
                    self.merge_library_chat(playlist);
                }
                TelegramUpdate::ChatTitle(chat, title) => {
                    if let Some(playlist) = self.state.playlists.iter().find(|p| p.id == chat) {
                        let mut playlist = playlist.clone();
                        playlist.title = title;
                        self.handle(CoreCommand::MergeChat(playlist));
                    }
                }
                TelegramUpdate::Message { id, track } => {
                    self.offline_source_changed(id);
                    if let Some(track) = track {
                        self.library.cache_track(&track);
                        if self.offline.policies.values().any(|p| {
                            p.enabled && p.chat == track.chat && p.filters[usize::from(track.kind)]
                        }) {
                            self.offline.enqueue(metadata_track(track.clone()), false);
                        }
                        self.handle(CoreCommand::Patch(
                            LibraryPatch {
                                chat: id.chat,
                                tracks: vec![metadata_track(track)],
                                ..Default::default()
                            },
                            HashSet::new(),
                        ));
                    } else {
                        self.handle(CoreCommand::Delete(id.chat, vec![id.message]));
                    }
                }
                TelegramUpdate::Deleted {
                    chat,
                    messages,
                    from_cache,
                } => {
                    if !from_cache {
                        for &message in &messages {
                            self.offline_source_changed(tg_player_domain::TrackId {
                                chat,
                                message,
                            });
                        }
                        self.handle(CoreCommand::Delete(chat, messages));
                    }
                }
                TelegramUpdate::Authorization(_) => {}
            },
        }
        self.library_persistence_effect(&mut effects);
        effects
    }
    pub fn library_event(&mut self, event: LibraryEvent) -> Vec<Effect> {
        let mut effects = Vec::new();
        match event {
            LibraryEvent::Catalog { request, result } => {
                if !self.library.accepts_catalog(request) {
                    return effects;
                }
                self.library.catalog = None;
                match result {
                    Ok(chats) => {
                        for playlist in chats {
                            self.merge_library_chat(playlist);
                        }
                        effects.push(Effect::Notice(crate::notice::Notice::OpenChatPrompt));
                    }
                    Err(error) => effects.push(Effect::Library(LibraryEffect::Failed {
                        error,
                        retry: LibraryRetry::Catalog,
                    })),
                }
            }
            LibraryEvent::Search {
                request,
                query,
                result,
            } => {
                if self.library.search.query != query
                    || !self
                        .library
                        .search
                        .operation
                        .as_ref()
                        .is_some_and(|op| op.key() == request)
                {
                    return effects;
                }
                self.library.search.operation = None;
                match result {
                    Ok(chats) => {
                        self.library.search.ids = chats.iter().map(|p| p.id).collect();
                        for playlist in chats {
                            self.merge_library_chat(playlist);
                        }
                    }
                    Err(error) => self.library.search.error = Some(error),
                }
            }
            LibraryEvent::Indexed { request, result } => {
                if !self.library.accepts_index(request, None) {
                    return effects;
                }
                let chat = self.library.index.take().unwrap().chat;
                match result {
                    Ok(()) => effects.push(Effect::Notice(crate::notice::Notice::MusicLoaded)),
                    Err(error) => effects.push(Effect::Library(LibraryEffect::Failed {
                        error,
                        retry: LibraryRetry::Chat(chat),
                    })),
                }
            }
            LibraryEvent::Page { request, playlist } => {
                if !self.library.accepts_index(request, Some(playlist.id)) {
                    return effects;
                }
                let protected = self.library.protected(playlist.id);
                for track in playlist
                    .tracks
                    .iter()
                    .filter(|t| !protected.contains(&t.message))
                {
                    self.library.cache_track(track);
                }
                self.handle(CoreCommand::Page(playlist.into(), protected));
            }
            LibraryEvent::Patch { request, patch } => {
                if !self.library.accepts_index(request, Some(patch.chat)) {
                    return effects;
                }
                let protected = self.library.protected(patch.chat);
                for track in patch
                    .tracks
                    .iter()
                    .filter(|t| !protected.contains(&t.message))
                {
                    self.library.cache_track(track);
                }
                self.handle(CoreCommand::Patch(patch.into(), protected));
            }
            LibraryEvent::Media {
                request,
                now,
                connection,
                id,
                revision,
                result,
            } => {
                let current = self.library.media_requests.get(&id).is_some_and(|pending| {
                    pending.operation.key() == request
                        && pending.connection == connection
                        && pending.revision == revision
                });
                if !current
                    || self.library.connection() != connection
                    || self.library.revision(id) != revision
                {
                    return effects;
                }
                self.library.media_requests.remove(&id);
                match result {
                    Ok(track) if track.id() == id && track.file > 0 => {
                        self.library.media_retry.remove(&id);
                        self.library.cache_track(&track);
                    }
                    _ => {
                        self.library
                            .media_retry
                            .insert(id, now + Duration::from_secs(15));
                    }
                }
            }
            LibraryEvent::Artwork {
                request,
                now,
                connection,
                id,
                result,
            } => {
                if self.library.connection() != connection
                    || !self
                        .library
                        .artwork_requests
                        .get(&id)
                        .is_some_and(|operation| operation.key() == request)
                {
                    return effects;
                }
                self.library.artwork_requests.remove(&id);
                match result {
                    Ok(path) => {
                        self.library.artwork_retry.remove(&id);
                        self.library.cache_artwork(id, path.clone());
                        effects.push(Effect::Library(LibraryEffect::ForgetArtwork(path)));
                    }
                    Err(_) => {
                        self.library
                            .artwork_retry
                            .insert(id, now + Duration::from_secs(5));
                    }
                }
            }
        }
        self.library_persistence_effect(&mut effects);
        effects
    }
    fn merge_library_chat(&mut self, playlist: MediaPlaylist) {
        for track in &playlist.tracks {
            self.library.cache_track(track);
        }
        self.library.cache_playlist_artwork(&playlist);
        self.handle(CoreCommand::MergeChat(playlist.into()));
    }
    fn library_persistence_effect(&mut self, effects: &mut Vec<Effect>) {
        let changes = self.take_changes();
        if !changes.is_empty() {
            effects.push(Effect::Persist(Box::new(changes)));
        }
    }
    /// Decorate one metadata record for a desktop renderer or adapter request.
    pub fn media_track(&self, track: Track) -> MediaTrack {
        self.library.media_track(track)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        library::{LibraryEffect, LibraryEvent},
        model::{Playlist, State},
        tasks::RequestId,
        AppError,
    };
    use std::time::Instant;
    fn track(message: i64) -> Track {
        Track {
            chat: 42,
            message,
            title: format!("Track {message}"),
            ..Default::default()
        }
    }
    fn core() -> AppCore {
        let mut core = AppCore::new(State {
            playlists: vec![Playlist {
                id: 42,
                tracks: vec![track(7); 3],
                filters: [true, false, false],
                ..Default::default()
            }],
            ..Default::default()
        });
        core.library_command(LibraryCommand::Connected(1));
        core
    }
    fn media(core: &mut AppCore, message: i64, now: Instant) -> RequestId {
        core.library_command(LibraryCommand::RequestMedia {
            track: track(message),
            now,
        })
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Library(LibraryEffect::Media { operation, .. }) => Some(operation.key()),
            _ => None,
        })
        .unwrap()
    }
    fn artwork(core: &mut AppCore, id: i64, now: Instant, force: bool) -> RequestId {
        core.library_command(LibraryCommand::RequestArtwork { id, now, force })
            .into_iter()
            .find_map(|effect| match effect {
                Effect::Library(LibraryEffect::Artwork { operation, .. }) => Some(operation.key()),
                _ => None,
            })
            .unwrap()
    }
    fn index(core: &mut AppCore) -> RequestId {
        core.library_command(LibraryCommand::Open {
            chat: 42,
            refresh: true,
        })
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Library(LibraryEffect::Index { operation, .. }) => Some(operation.key()),
            _ => None,
        })
        .unwrap()
    }
    fn catalog(core: &mut AppCore) -> RequestId {
        core.library_command(LibraryCommand::RefreshCatalog)
            .into_iter()
            .find_map(|effect| match effect {
                Effect::Library(LibraryEffect::Catalog { operation, .. }) => Some(operation.key()),
                _ => None,
            })
            .unwrap()
    }
    fn search(core: &mut AppCore, query: &str, now: Instant) -> RequestId {
        core.library_command(LibraryCommand::Search {
            query: query.into(),
            now,
            force: true,
        })
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Library(LibraryEffect::Search { operation, .. }) => Some(operation.key()),
            _ => None,
        })
        .unwrap()
    }
    #[test]
    fn superseded_index_pages_and_terminal_events_cannot_mutate_new_work() {
        let mut core = core();
        let old = index(&mut core);
        let cancelled = core
            .library
            .index
            .as_ref()
            .unwrap()
            .operation
            .cancel
            .clone();
        let new = index(&mut core);
        assert!(cancelled.is_cancelled());
        core.library_event(LibraryEvent::Page {
            request: old,
            playlist: Playlist {
                id: 42,
                tracks: vec![track(999)],
                ..Default::default()
            }
            .into(),
        });
        core.library_event(LibraryEvent::Indexed {
            request: old,
            result: Err(AppError::Cancelled),
        });
        assert_eq!(core.library.index.as_ref().unwrap().operation.key(), new);
        assert!(core.state.playlists[0]
            .tracks
            .iter()
            .all(|t| t.message == 7));
        core.library_event(LibraryEvent::Indexed {
            request: new,
            result: Ok(()),
        });
        assert!(core.library.index.is_none());
    }
    #[test]
    fn reconnect_invalidates_catalog_search_index_and_runtime_media() {
        let mut core = core();
        let old_catalog = catalog(&mut core);
        let old_index = index(&mut core);
        let old_search = search(&mut core, "radio", Instant::now());
        let mut media = MediaTrack::from(track(7));
        media.file = 99;
        core.library.cache_track(&media);
        core.library_command(LibraryCommand::Connected(2));
        assert!(core.library.runtime_media(media.id()).is_none());
        let new_catalog = catalog(&mut core);
        let new_index = index(&mut core);
        let new_search = search(&mut core, "radio", Instant::now());
        for event in [
            LibraryEvent::Catalog {
                request: old_catalog,
                result: Err(AppError::Unavailable("old catalog")),
            },
            LibraryEvent::Indexed {
                request: old_index,
                result: Err(AppError::Unavailable("old index")),
            },
            LibraryEvent::Search {
                request: old_search,
                query: "radio".into(),
                result: Err(AppError::Unavailable("old search")),
            },
        ] {
            assert!(core.library_event(event).is_empty());
        }
        assert_eq!(core.library.catalog.as_ref().unwrap().key(), new_catalog);
        assert_eq!(
            core.library.index.as_ref().unwrap().operation.key(),
            new_index
        );
        assert_eq!(
            core.library.search.operation.as_ref().unwrap().key(),
            new_search
        );
        assert!(core.library.search.error.is_none());
    }
    #[test]
    fn search_debounces_and_checks_job_identity_as_well_as_generation() {
        let mut core = core();
        let now = Instant::now();
        assert!(core
            .library_command(LibraryCommand::Search {
                query: "  radio  ".into(),
                now,
                force: false
            })
            .is_empty());
        assert!(!core.library.search.loading());
        let effects = core.library_command(LibraryCommand::Search {
            query: "radio".into(),
            now: now + Duration::from_millis(250),
            force: false,
        });
        let request = effects
            .into_iter()
            .find_map(|e| match e {
                Effect::Library(LibraryEffect::Search { operation, .. }) => Some(operation.key()),
                _ => None,
            })
            .unwrap();
        let wrong = RequestId {
            job: crate::tasks::JobId(request.job.0 + 10_000),
            ..request
        };
        core.library_event(LibraryEvent::Search {
            request: wrong,
            query: "radio".into(),
            result: Err(AppError::Cancelled),
        });
        assert!(core.library.search.loading());
        assert!(core.library.search.error.is_none());
        core.library_command(LibraryCommand::Search {
            query: String::new(),
            now,
            force: false,
        });
        core.library_event(LibraryEvent::Search {
            request,
            query: "radio".into(),
            result: Ok(vec![Playlist {
                id: 99,
                ..Default::default()
            }
            .into()]),
        });
        assert!(!core.state.playlists.iter().any(|p| p.id == 99));
    }
    #[test]
    fn live_edits_preserve_occurrences_queue_metadata_and_win_over_index_pages() {
        let mut core = core();
        let original = core.state.playlists[0].tracks.clone();
        let ids: Vec<_> = original.iter().map(|t| t.library_id).collect();
        core.handle(CoreCommand::Start(original, 1));
        let request = index(&mut core);
        let mut edited = MediaTrack::from(track(7));
        edited.title = "Edited".into();
        edited.file = 99;
        core.library_command(LibraryCommand::Source(TelegramUpdate::Message {
            id: edited.id(),
            track: Some(edited),
        }));
        core.library_event(LibraryEvent::Patch {
            request,
            patch: LibraryPatch {
                chat: 42,
                tracks: vec![track(7)],
                removed: vec![7],
                ..Default::default()
            }
            .into(),
        });
        assert_eq!(
            core.state.playlists[0]
                .tracks
                .iter()
                .map(|t| t.library_id)
                .collect::<Vec<_>>(),
            ids
        );
        assert!(core.state.playlists[0]
            .tracks
            .iter()
            .all(|t| t.title == "Edited"));
        assert!(core
            .state
            .session
            .snapshot()
            .iter()
            .all(|t| t.title == "Track 7"));
        assert_eq!(core.media_track(track(7)).file, 99);
    }
    #[test]
    fn deletion_keeps_ready_copy_and_stale_page_cannot_restore_source() {
        let mut core = core();
        let request = index(&mut core);
        let id = track(7).id();
        core.offline.publish(track(7), "ready".into());
        core.library_command(LibraryCommand::Source(TelegramUpdate::Deleted {
            chat: 42,
            messages: vec![7],
            from_cache: false,
        }));
        core.library_event(LibraryEvent::Page {
            request,
            playlist: Playlist {
                id: 42,
                tracks: vec![track(7)],
                ..Default::default()
            }
            .into(),
        });
        assert!(core.state.playlists[0].tracks.is_empty());
        assert_eq!(core.offline.path(id), Some("ready"));
    }
    #[test]
    fn stale_media_results_cannot_replace_live_edit_or_new_connection() {
        let mut core = core();
        let id = track(7).id();
        let now = Instant::now();
        let request = media(&mut core, 7, now);
        let artwork_request = artwork(&mut core, 1, now, false);
        let mut old = MediaTrack::from(track(7));
        old.file = 11;
        let mut edited = MediaTrack::from(track(7));
        edited.file = 22;
        core.library_command(LibraryCommand::Source(TelegramUpdate::Message {
            id,
            track: Some(edited),
        }));
        core.library_event(LibraryEvent::Media {
            request,
            now,
            connection: 1,
            id,
            revision: 0,
            result: Ok(old.clone()),
        });
        assert_eq!(core.media_track(track(7)).file, 22);
        core.library_command(LibraryCommand::Connected(2));
        core.library_event(LibraryEvent::Media {
            request,
            now,
            connection: 1,
            id,
            revision: 0,
            result: Ok(old),
        });
        assert_eq!(core.media_track(track(7)).file, 0);
        core.library_event(LibraryEvent::Artwork {
            request: artwork_request,
            now,
            connection: 1,
            id: 1,
            result: Ok("old".into()),
        });
        assert!(core.library.artwork_path(1).is_none());
    }
    #[test]
    fn media_only_updates_change_cache_version_without_persistence_writes() {
        let mut core = core();
        let id = track(7).id();
        let now = Instant::now();
        let request = media(&mut core, 7, now);
        let before = core.library.media_version();
        let mut media = MediaTrack::from(track(7));
        media.file = 99;
        let effects = core.library_event(LibraryEvent::Media {
            request,
            now,
            connection: 1,
            id,
            revision: 0,
            result: Ok(media),
        });
        assert!(effects.is_empty());
        assert!(core.library.media_version() > before);
        let effects = core.library_command(LibraryCommand::Source(TelegramUpdate::File(
            crate::telegram::TelegramFile {
                id: 99,
                path: Some("cached".into()),
                complete: true,
                ..Default::default()
            },
        )));
        assert!(effects.is_empty());
        assert_eq!(core.media_track(track(7)).cached.as_deref(), Some("cached"));
        assert!(core.state.playlists[0]
            .tracks
            .iter()
            .all(|t| t.title == "Track 7"));
    }
    #[test]
    fn media_requests_are_bounded_deduplicated_and_stale_completions_cannot_free_new_slots() {
        let mut core = core();
        let now = Instant::now();
        let mut requests = Vec::new();
        for message in 1..=4 {
            requests.push(media(&mut core, message, now));
        }
        for message in [1, 5] {
            assert!(core
                .library_command(LibraryCommand::RequestMedia {
                    track: track(message),
                    now
                })
                .is_empty());
        }
        let wrong = RequestId {
            job: crate::tasks::JobId(requests[0].job.0 + 1000),
            ..requests[0]
        };
        core.library_event(LibraryEvent::Media {
            request: wrong,
            now,
            connection: 1,
            id: track(1).id(),
            revision: 0,
            result: Err(AppError::Cancelled),
        });
        assert_eq!(core.library.media_requests.len(), 4);
        let mut resolved = MediaTrack::from(track(1));
        resolved.file = 99;
        core.library_event(LibraryEvent::Media {
            request: requests[0],
            now,
            connection: 1,
            id: track(1).id(),
            revision: 0,
            result: Ok(resolved),
        });
        let new = media(&mut core, 5, now);
        core.library_event(LibraryEvent::Media {
            request: requests[0],
            now,
            connection: 1,
            id: track(1).id(),
            revision: 0,
            result: Err(AppError::Cancelled),
        });
        assert_eq!(core.library.media_requests.len(), 4);
        assert_eq!(
            core.library.media_requests[&track(5).id()].operation.key(),
            new
        );
        assert!(!core.library.media_retry.contains_key(&track(1).id()));
        let tokens: Vec<_> = core
            .library
            .media_requests
            .values()
            .map(|pending| pending.operation.cancel.clone())
            .collect();
        core.cancel_library_operations();
        assert!(tokens.iter().all(|token| token.is_cancelled()));
        assert!(core.library.media_requests.is_empty());
    }
    #[test]
    fn media_failure_retries_after_deadline_and_live_edit_cancels_old_request() {
        let mut core = core();
        let now = Instant::now();
        let old = media(&mut core, 7, now);
        core.library_event(LibraryEvent::Media {
            request: old,
            now,
            connection: 1,
            id: track(7).id(),
            revision: 0,
            result: Err(AppError::Unavailable("media")),
        });
        assert!(core
            .library_command(LibraryCommand::RequestMedia {
                track: track(7),
                now: now + Duration::from_secs(14)
            })
            .is_empty());
        let new = media(&mut core, 7, now + Duration::from_secs(15));
        assert_ne!(new, old);
        let token = core.library.media_requests[&track(7).id()]
            .operation
            .cancel
            .clone();
        core.library_command(LibraryCommand::Source(TelegramUpdate::Message {
            id: track(7).id(),
            track: Some(track(7).into()),
        }));
        assert!(token.is_cancelled());
        let replacement = media(&mut core, 7, now + Duration::from_secs(15));
        core.library_event(LibraryEvent::Media {
            request: new,
            now,
            connection: 1,
            id: track(7).id(),
            revision: 0,
            result: Err(AppError::Cancelled),
        });
        assert_eq!(
            core.library.media_requests[&track(7).id()].operation.key(),
            replacement
        );
        assert!(!core.library.media_retry.contains_key(&track(7).id()));
    }
    #[test]
    fn artwork_requests_bound_retry_and_allow_refetch_after_cache_eviction() {
        let mut core = core();
        let now = Instant::now();
        let old = artwork(&mut core, 1, now, false);
        for id in 2..=4 {
            artwork(&mut core, id, now, false);
        }
        assert!(core
            .library_command(LibraryCommand::RequestArtwork {
                id: 5,
                now,
                force: false
            })
            .is_empty());
        core.library_event(LibraryEvent::Artwork {
            request: old,
            now,
            connection: 1,
            id: 1,
            result: Err(AppError::Unavailable("artwork")),
        });
        assert!(core
            .library_command(LibraryCommand::RequestArtwork {
                id: 1,
                now: now + Duration::from_secs(4),
                force: false
            })
            .is_empty());
        let new = artwork(&mut core, 1, now + Duration::from_secs(5), false);
        core.library_event(LibraryEvent::Artwork {
            request: old,
            now,
            connection: 1,
            id: 1,
            result: Ok("stale".into()),
        });
        assert_eq!(core.library.artwork_requests[&1].key(), new);
        assert!(core.library.artwork_path(1).is_none());
        let effects = core.library_event(LibraryEvent::Artwork {
            request: new,
            now,
            connection: 1,
            id: 1,
            result: Ok("cached".into()),
        });
        assert!(effects.iter().any(|effect|matches!(effect,Effect::Library(LibraryEffect::ForgetArtwork(path)) if path=="cached")));
        assert!(core
            .library_command(LibraryCommand::RequestArtwork {
                id: 1,
                now,
                force: false
            })
            .is_empty());
        let forced = artwork(&mut core, 1, now, true);
        assert_ne!(forced, new);
        let token = core.library.artwork_requests[&1].cancel.clone();
        core.library_command(LibraryCommand::Connected(2));
        assert!(token.is_cancelled());
        core.library_event(LibraryEvent::Artwork {
            request: forced,
            now,
            connection: 1,
            id: 1,
            result: Ok("old connection".into()),
        });
        assert!(core.library.artwork_path(1).is_none());
    }
    #[test]
    fn metadata_pages_keep_connection_artwork_and_persistence_contains_no_runtime_fields() {
        let mut core = core();
        core.library_command(LibraryCommand::MergeChat(MediaPlaylist {
            id: 42,
            title: "Chat".into(),
            image_file: 77,
            image_path: Some("/runtime/avatar".into()),
            ..Default::default()
        }));
        let request = index(&mut core);
        core.library_event(LibraryEvent::Page {
            request,
            playlist: core.view().playlists[0].clone().into(),
        });
        let art = core.library().playlist_artwork(42);
        assert_eq!(art.file, 77);
        assert_eq!(art.path.as_deref(), Some("/runtime/avatar"));
        let saved = serde_json::to_value(core.view()).unwrap();
        assert!(saved["playlists"][0].get("image_file").is_none());
        assert!(saved["playlists"][0].get("image_path").is_none());
        for track in saved["playlists"][0]["tracks"].as_array().unwrap() {
            for field in ["file", "cached", "pinned", "image_file", "image_path"] {
                assert!(track.get(field).is_none());
            }
        }
    }
}
