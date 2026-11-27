//! Application decisions and adapter effects. This module has no desktop dependencies.
use crate::{
    commands::AppCommand,
    core::{AppCore, CoreCommand},
    model::Track,
    persistence::ChangeSet,
    ports::{PlaybackEvent, PlayerCommand, PlayerSnapshot, QueuedPlayback},
    tasks::{Operation, RequestId},
    AppError,
};

pub enum Effect {
    Library(crate::library::LibraryEffect),
    QueuePage {
        operation: Operation,
        chat: i64,
        kind: usize,
        cursor: i64,
        order: gramwave_domain::PlaylistOrder,
        browsing: bool,
    },
    ScanOffline {
        operation: Operation,
        chat: i64,
        kind: usize,
        cursor: i64,
    },
    DownloadOffline {
        operation: Operation,
        track: Track,
    },
    RemoveOffline {
        operation: Operation,
        path: String,
    },
    OfflineFailed(gramwave_domain::TrackId, AppError),
    Authenticate {
        operation: Operation,
        request: crate::telegram::AuthRequest,
    },
    AuthorizationChanged(crate::telegram::AuthorizationState),
    AuthenticationFinished(Result<(), AppError>),
    ParametersRequested,
    Connect,
    Quit,
    RetryRequested,
    SelectPlaylistRow(usize),
    OpenedPlaylist(i64),
    Disconnect,
    PreparePlayback {
        operation: Operation,
        track: Track,
        offline: Option<String>,
        revision: u64,
    },
    Player(PlayerCommand),
    PlaybackAdvanced(String),
    Prefetch(Track),
    Persist(Box<ChangeSet>),
    Notice(crate::notice::Notice),
    Error(AppError),
    ClearSeek,
}
pub enum AppEvent {
    Library(crate::library::LibraryEvent),
    QueuePage {
        request: RequestId,
        browsing: bool,
        result: Result<crate::telegram::MediaPage, AppError>,
    },
    OfflinePage {
        request: RequestId,
        chat: i64,
        kind: usize,
        result: Result<crate::telegram::MediaPage, AppError>,
    },
    OfflineDownloaded {
        request: RequestId,
        id: gramwave_domain::TrackId,
        result: Result<crate::offline::PublishedFile, AppError>,
    },
    OfflineHandoffFinished {
        id: gramwave_domain::TrackId,
        path: String,
    },
    LocalPlaybackUsed {
        id: gramwave_domain::TrackId,
        path: String,
    },
    OfflineRemoved {
        request: RequestId,
        path: String,
        result: Result<(), AppError>,
    },
    Authorization(crate::telegram::AuthorizationState),
    Authenticated {
        request: RequestId,
        result: Result<(), AppError>,
    },
    PlaybackPrepared {
        request: RequestId,
        revision: u64,
        result: Result<PreparedPlayback, AppError>,
    },
    Player(PlaybackEvent),
    Snapshot(PlayerSnapshot),
    PersistenceRejected(Box<ChangeSet>, AppError),
}
pub struct PreparedPlayback {
    pub track: crate::media::MediaTrack,
    pub path: Option<String>,
}
impl AppCore {
    pub fn pump(&mut self, now: std::time::Instant, online: bool) -> Vec<Effect> {
        let opening = self
            .playback
            .state
            .loading()
            .then(|| self.playback.target.as_ref().map(Track::id))
            .flatten();
        let mut effects = self.offline.pump(now, online, opening);
        if online {
            self.pump_queue(&mut effects);
            self.pump_oldest_start(&mut effects);
        }
        effects
    }

    pub fn shutdown_operations(&mut self) {
        self.playback.cancel();
        self.queue_work.cancel_page();
        self.auth.cancel();
        self.offline.cancel_work();
        self.library.cancel_operations();
    }

    pub(crate) fn offline_source_changed(&mut self, id: gramwave_domain::TrackId) {
        self.invalidate_track(id);
        self.offline.cancel_attempt(id);
        self.offline.source_changed(id);
        if self.offline.copies.get(&id).is_some_and(|copy| copy.cached) {
            self.offline.discard(id);
        }
    }
    pub fn auth(&self) -> &crate::auth::AuthCore {
        &self.auth
    }

    pub(crate) fn invalidate_track(&mut self, id: gramwave_domain::TrackId) {
        self.library.invalidate_track(id);
    }
    pub fn playback(&self) -> &crate::playback::PlaybackCore {
        &self.playback
    }
    pub fn enable_streamed_next(&mut self, enabled: bool) {
        self.playback.streamed_next = enabled;
    }

    pub fn next_streamed_playback(
        &mut self,
    ) -> Option<(crate::media::MediaTrack, u64, Option<String>)> {
        use crate::playback::PlaybackState;
        use gramwave_domain::RepeatMode;
        if !self.playback.streamed_next
            || !matches!(
                self.playback.state,
                PlaybackState::Playing | PlaybackState::Paused
            )
        {
            return None;
        }
        let session = &self.state.session;
        let current = session.current()?;
        let next = if self.state.repeat == RepeatMode::One
            && !self.queue_reversing()
            && !self.queue_waiting()
        {
            current
        } else if self.queue_blocks_advance(false) {
            return None;
        } else if let Some(next) = session.remaining().first() {
            next
        } else if self.state.repeat == RepeatMode::All {
            let cycle = self
                .playback
                .repeat_cycle
                .get_or_insert_with(|| session.repeat_cycle(self.state.shuffle));
            session.repeat_cycle_first(cycle)?
        } else {
            return None;
        };
        let revision = self.revision(next.id());
        let offline = self
            .offline
            .local_path(
                next.id(),
                crate::offline::unix_seconds(std::time::SystemTime::now()),
                self.state.cache_policy.days,
            )
            .map(str::to_owned);
        Some((self.media_track(next.into()), revision, offline))
    }

    pub fn next_local_playback(&self) -> Option<QueuedPlayback> {
        use crate::playback::PlaybackState;
        use gramwave_domain::RepeatMode;
        if !matches!(
            self.playback.state,
            PlaybackState::Playing | PlaybackState::Paused
        ) {
            return None;
        }
        let session = &self.state.session;
        let current = session.current()?;
        let next = if self.state.repeat == RepeatMode::One
            && !self.queue_reversing()
            && !self.queue_waiting()
        {
            current
        } else if self.queue_blocks_advance(false) {
            return None;
        } else {
            session.remaining().first().or_else(|| {
                // A shuffled repeat cycle is decided when the current track ends.
                if self.state.repeat == RepeatMode::All && !self.state.shuffle {
                    session.snapshot().first()
                } else {
                    None
                }
            })?
        };
        let path = self
            .offline
            .local_path(
                next.id(),
                crate::offline::unix_seconds(std::time::SystemTime::now()),
                self.state.cache_policy.days,
            )
            .map(str::to_owned)
            .or_else(|| self.library.runtime_media(next.id())?.cached.clone())?;
        Some(QueuedPlayback {
            entry_id: next.entry_id,
            revision: self.revision(next.id()),
            path,
        })
    }
    pub fn command(&mut self, command: AppCommand) -> Vec<Effect> {
        let mut effects = Vec::new();
        let advancing = matches!(&command, AppCommand::Next | AppCommand::Advance(_));
        if advancing {
            let eof = matches!(&command, AppCommand::Advance(true));
            if self.queue_blocks_advance(eof) {
                self.queue_work.waiting = Some(eof && self.queue_work.waiting != Some(false));
                if !eof {
                    self.queue_work.error = None;
                }
                return effects;
            }
            self.queue_work.waiting = None;
        }
        let mut play = None;
        let mut prepared_next = None;
        let mut pause_after_load = None;
        match command {
            AppCommand::Connect => {
                if !self.library.connected() {
                    self.auth.cancel();
                    self.playback.cancel();
                    self.queue_work.cancel_page();
                    self.library.invalidate();
                    self.auth
                        .transition(crate::telegram::AuthorizationState::Starting);
                    effects.extend([
                        Effect::Connect,
                        Effect::Notice(crate::notice::Notice::Connecting),
                    ]);
                }
            }
            AppCommand::Quit => effects.push(Effect::Quit),
            AppCommand::Retry => effects.push(Effect::RetryRequested),
            AppCommand::SelectPlaylistRow(index) => effects.push(Effect::SelectPlaylistRow(index)),
            command @ (AppCommand::OpenPlaylist(chat) | AppCommand::OpenSearchPlaylist(chat))
                if self.state.playlists.iter().any(|p| p.id == chat) =>
            {
                let promote = matches!(command, AppCommand::OpenSearchPlaylist(_))
                    && self.state.recent_chats.first() != Some(&chat);
                if self.state.last_opened_chat != Some(chat) || promote {
                    let mut settings = crate::persistence::Settings::from(&self.state);
                    settings.last_opened_chat = Some(chat);
                    if promote {
                        settings.recent_chats.retain(|&id| id != chat);
                        settings.recent_chats.insert(0, chat);
                    }
                    self.handle(CoreCommand::Settings(settings));
                }
                effects.push(Effect::OpenedPlaylist(chat));
                effects.extend(self.library_command(crate::library::LibraryCommand::Open {
                    chat,
                    refresh: true,
                }));
            }
            AppCommand::RemoveRecentChat(chat) if self.state.recent_chats.contains(&chat) => {
                let mut settings = crate::persistence::Settings::from(&self.state);
                settings.recent_chats.retain(|&id| id != chat);
                self.handle(CoreCommand::Settings(settings));
            }
            AppCommand::ClearRecentChats if !self.state.recent_chats.is_empty() => {
                let mut settings = crate::persistence::Settings::from(&self.state);
                settings.recent_chats.clear();
                self.handle(CoreCommand::Settings(settings));
            }
            AppCommand::SaveTrack(track) => self.save_tracks([track]),
            AppCommand::SaveTracks(tracks) => self.save_tracks(tracks),
            AppCommand::SavePlaylist(chat, filters) => {
                self.prune_audio_cache();
                self.offline.cancel_scan();
                self.offline.work.retry.retain(|id, _| id.chat != chat);
                self.offline.save_playlist(chat, filters);
                if let Some(playlist) = self
                    .state
                    .playlists
                    .iter()
                    .find(|playlist| playlist.id == chat)
                {
                    for track in &playlist.tracks {
                        if filters[usize::from(track.kind)] {
                            self.offline.enqueue(track.clone(), false);
                        }
                    }
                }
                self.offline.work.scan_retry = None;
                effects.push(Effect::Notice(crate::notice::Notice::OfflineDownloading));
            }
            AppCommand::CancelOffline => {
                self.offline.suspend();
                self.offline.cancel();
                effects.push(Effect::Notice(crate::notice::Notice::OfflineInterrupted));
            }
            AppCommand::RemoveOffline(id, path) => {
                if self.offline.path(id) == Some(path.as_str()) {
                    self.offline.cancel_attempt(id);
                    self.offline.unpin(
                        id,
                        crate::offline::unix_seconds(std::time::SystemTime::now()),
                        self.state.cache_policy.days,
                    );
                    self.prune_audio_cache();
                }
            }
            AppCommand::RemoveOfflinePlaylists(chats) => {
                self.offline.remove_playlists(
                    &chats,
                    crate::offline::unix_seconds(std::time::SystemTime::now()),
                    self.state.cache_policy.days,
                );
                self.prune_audio_cache();
            }
            AppCommand::PruneAudioCache => self.prune_audio_cache(),
            AppCommand::ClearAudioCache(chat) => self.offline.clear_cache(chat),
            AppCommand::Authenticate(request) => {
                if let Some(operation) = self.auth.begin() {
                    effects.push(Effect::Authenticate { operation, request });
                }
            }
            AppCommand::StartSession(tracks, index) => {
                self.queue_work.reset();
                play = self.handle(CoreCommand::Start(tracks, index));
                if play.is_some() {
                    self.playback.queue_removals.clear();
                }
            }
            AppCommand::StartPlaylist {
                chat,
                library_id,
                order,
            } => {
                play = self.start_playlist_queue(chat, library_id, order);
                self.pump_oldest_start(&mut effects);
            }
            AppCommand::LoadOldest { chat, retry } => {
                self.request_oldest(chat, retry, &mut effects)
            }
            AppCommand::CancelOldestStart(chat) => self.cancel_oldest_start(chat),
            AppCommand::ReverseQueue => self.request_queue_reverse(),
            AppCommand::RetryQueue => self.retry_queue_page(),
            AppCommand::Play(track) => {
                self.queue_work.waiting = None;
                play = Some(track);
            }
            AppCommand::PlayQueue(id) => {
                self.queue_work.waiting = None;
                play = self.handle(CoreCommand::Jump(id));
            }
            AppCommand::PlayQueueRow { index, entry_id } => {
                self.queue_work.waiting = None;
                play = self.handle(CoreCommand::PlayQueueRow(index, entry_id))
            }
            AppCommand::Next => {
                let current = self.state.session.current().map(|track| track.entry_id);
                prepared_next = self
                    .next_streamed_playback()
                    .map(|(track, revision, _)| (track.entry_id, revision))
                    .filter(|(entry, _)| Some(*entry) != current);
                play = self.handle(CoreCommand::Advance(false));
            }
            AppCommand::Advance(eof) => play = self.handle(CoreCommand::Advance(eof)),
            AppCommand::Previous => {
                if !self.playback.state.loading() && self.playback.snapshot.position > 3. {
                    self.queue_work.waiting = None;
                    return vec![Effect::Player(PlayerCommand::Seek(0.)), Effect::ClearSeek];
                }
                play = self.handle(CoreCommand::Previous);
                if play.is_some() {
                    self.queue_work.waiting = None;
                }
            }
            AppCommand::Stop => {
                self.queue_work.reset();
                self.playback.cancel();
                self.playback.state = crate::playback::PlaybackState::Stopped;
                self.handle(CoreCommand::Stop);
                effects.extend([Effect::Player(PlayerCommand::Stop), Effect::ClearSeek]);
            }
            AppCommand::Pause(paused) => {
                if self.state.session.current().is_some() {
                    if !paused
                        && self.playback.state.paused()
                        && !self.queue_waiting()
                        && !self.playback.state.loading()
                        && (matches!(
                            self.playback.state,
                            crate::playback::PlaybackState::Stopped
                                | crate::playback::PlaybackState::Failed
                        ) || (self.playback.snapshot.duration <= 0.
                            && self.playback.snapshot.checkpoint_position.is_none()
                            && !self.playback.snapshot.seeking))
                    {
                        play = self.state.session.current().map(Into::into);
                    } else {
                        self.playback.state.set_paused(paused);
                        effects.push(Effect::Player(PlayerCommand::Pause(paused)));
                    }
                }
            }
            AppCommand::Volume(value) if value.is_finite() => {
                self.handle(CoreCommand::Volume(value));
                effects.push(Effect::Player(PlayerCommand::Volume(self.state.volume)));
            }
            AppCommand::Shuffle(value) => {
                self.handle(CoreCommand::Shuffle(value));
            }
            AppCommand::Repeat(value) => {
                self.handle(CoreCommand::Repeat(value));
            }
            AppCommand::Seek(value) if value.is_finite() => {
                if !self.playback.state.loading() {
                    effects.push(Effect::Player(PlayerCommand::Seek(value.max(0.))));
                }
                effects.push(Effect::ClearSeek);
            }
            AppCommand::Settings(value) => {
                self.handle(CoreCommand::Settings(value));
            }
            AppCommand::Filters(id, value) => {
                self.cancel_oldest_start(id);
                self.handle(CoreCommand::Filters(id, value));
            }
            AppCommand::Insert(track, next) => {
                self.handle(CoreCommand::Insert(track, next));
            }
            AppCommand::InsertTracks(tracks, next) => {
                self.handle(CoreCommand::InsertTracks(tracks, next));
            }
            AppCommand::EditQueueRange {
                start,
                expected,
                edit,
            } => {
                self.handle(CoreCommand::EditQueueRange(start, expected, edit));
            }
            AppCommand::MoveQueueRow { from, entry_id, to } => {
                self.handle(CoreCommand::MoveQueueRow(from, entry_id, to));
            }
            AppCommand::Remove(index) => {
                self.handle(CoreCommand::Remove(index));
            }
            AppCommand::RemoveQueueRows { expected, now } => {
                use crate::playback::{PendingQueueRemoval, QueueRemovalNotice};
                use std::time::Duration;

                self.playback.expire_queue_removals(now);
                self.save_playback_position(self.playback.snapshot);
                let pending = self.playback.queue_removals.iter().map(|entry| &entry.rows);
                if let Some(rows) = self.state.session.remove_timeline_rows(&expected, pending) {
                    self.playback.repeat_cycle = None;
                    let paused = self.playback.state.paused();
                    if rows.removed_current() {
                        if !self.queue_blocks_advance(false) {
                            play = self.state.session.jump_to(0).map(Into::into);
                        }
                        if play.is_some() {
                            pause_after_load = Some(paused);
                        } else {
                            if self.queue_blocks_advance(false) {
                                self.queue_work.waiting = Some(false);
                            }
                            self.stop_playback(&mut effects);
                            if self.queue_waiting() && paused {
                                self.playback.state = crate::playback::PlaybackState::Paused;
                            }
                        }
                    }
                    self.playback.removal_serial += 1;
                    self.playback.queue_removals.push(PendingQueueRemoval {
                        notice: QueueRemovalNotice {
                            id: self.playback.removal_serial,
                            count: expected.len(),
                            expires: now + Duration::from_secs(5),
                        },
                        rows,
                        paused,
                    });
                    self.record_session_change();
                }
            }
            AppCommand::UndoQueueRemoval { id, now } => {
                self.playback.expire_queue_removals(now);
                if let Some(index) = self
                    .playback
                    .queue_removals
                    .iter()
                    .position(|removal| removal.notice.id == id)
                {
                    let removal = self.playback.queue_removals.remove(index);
                    self.playback.repeat_cycle = None;
                    let pending = self.playback.queue_removals.iter().map(|entry| &entry.rows);
                    if self
                        .state
                        .session
                        .restore_timeline_rows(removal.rows, pending)
                    {
                        self.queue_work.waiting = None;
                        play = self.state.session.current().map(Into::into);
                        pause_after_load = Some(removal.paused);
                    }
                    self.record_session_change();
                }
            }
            AppCommand::Clear => {
                self.queue_work.reset();
                self.playback.queue_removals.clear();
                self.handle(CoreCommand::Clear);
            }
            AppCommand::ToggleChatPin(id) => {
                self.handle(CoreCommand::ToggleChatPin(id));
            }
            AppCommand::Volume(_)
            | AppCommand::Seek(_)
            | AppCommand::OpenPlaylist(_)
            | AppCommand::OpenSearchPlaylist(_)
            | AppCommand::RemoveRecentChat(_)
            | AppCommand::ClearRecentChats => {}
        }
        if let Some(track) = play {
            if let Some((entry_id, revision)) = prepared_next.filter(|&(entry, revision)| {
                entry == track.entry_id && revision == self.revision(track.id())
            }) {
                let request = self.playback.begin(track, revision).key();
                self.playback.promoting_next = true;
                effects.extend([
                    Effect::ClearSeek,
                    Effect::Player(PlayerCommand::PromoteNext {
                        request,
                        entry_id,
                        revision,
                    }),
                ]);
            } else {
                self.prepare_playback(track, &mut effects);
            }
            if let Some(paused) = pause_after_load {
                self.playback.state.set_paused(paused);
            }
        } else if advancing {
            self.stop_playback(&mut effects);
        }
        self.resume_queue_wait(&mut effects);
        self.persistence_effect(&mut effects);
        effects
    }
    fn stop_playback(&mut self, effects: &mut Vec<Effect>) {
        self.playback.cancel();
        self.playback.state = crate::playback::PlaybackState::Stopped;
        effects.extend([Effect::Player(PlayerCommand::Stop), Effect::ClearSeek]);
    }
    fn save_tracks(&mut self, tracks: impl IntoIterator<Item = Track>) {
        self.prune_audio_cache();
        for track in tracks {
            self.offline.work.retry.remove(&track.id());
            self.offline
                .work
                .errors
                .remove(&(track.chat, track.message));
            self.offline.enqueue(track, true);
        }
        self.offline.work.scan_retry = None;
    }

    fn prune_audio_cache(&mut self) {
        self.offline.prune_cache(
            crate::offline::unix_seconds(std::time::SystemTime::now()),
            self.state.cache_policy,
        );
    }

    pub(crate) fn prepare_playback(&mut self, track: Track, effects: &mut Vec<Effect>) {
        let revision = self.revision(track.id());
        let operation = self.playback.begin(track.clone(), revision);
        let offline = self
            .offline
            .local_path(
                track.id(),
                crate::offline::unix_seconds(std::time::SystemTime::now()),
                self.state.cache_policy.days,
            )
            .map(str::to_owned);
        effects.extend([
            Effect::ClearSeek,
            Effect::Player(PlayerCommand::Stop),
            Effect::Notice(crate::notice::Notice::LoadingTrack),
            Effect::PreparePlayback {
                operation,
                track,
                offline,
                revision,
            },
        ]);
    }
    pub fn event(&mut self, event: AppEvent) -> Vec<Effect> {
        let mut effects = Vec::new();
        match event {
            AppEvent::Library(event) => effects.extend(self.library_event(event)),
            AppEvent::QueuePage {
                request,
                result,
                browsing,
            } => {
                if browsing {
                    self.receive_oldest(request, result, &mut effects);
                } else {
                    self.receive_queue_page(request, result, &mut effects);
                }
            }
            AppEvent::OfflinePage {
                request,
                chat,
                kind,
                result,
            } => {
                if !self
                    .offline
                    .work
                    .scan
                    .as_ref()
                    .is_some_and(|(op, c, k)| op.key() == request && *c == chat && *k == kind)
                {
                    return effects;
                }
                self.offline.work.scan = None;
                match result {
                    Ok(mut page) => {
                        page.tracks
                            .retain(|track| !self.library.source_changed(track.id()));
                        self.offline.scan_page(chat, kind, page);
                    }
                    Err(error) => {
                        self.offline.work.scan_retry =
                            Some(std::time::Instant::now() + std::time::Duration::from_secs(30));
                        effects.push(Effect::Error(error));
                    }
                }
            }
            AppEvent::OfflineDownloaded {
                request,
                id,
                result,
            } => effects.extend(self.offline.downloaded(
                request,
                id,
                result,
                std::time::Instant::now(),
            )),
            AppEvent::OfflineHandoffFinished { id, path } => self.offline.finish_handoff(id, &path),
            AppEvent::LocalPlaybackUsed { id, path } => self.offline.mark_used(
                id,
                &path,
                crate::offline::unix_seconds(std::time::SystemTime::now()),
            ),
            AppEvent::OfflineRemoved {
                request,
                path,
                result,
            } => {
                if !self
                    .offline
                    .work
                    .removals
                    .get(&path)
                    .is_some_and(|op| op.key() == request)
                {
                    return effects;
                }
                self.offline.work.removals.remove(&path);
                match result {
                    Ok(()) => {
                        self.offline.finish_removal(&path);
                        self.offline.work.removal_retry.remove(&path);
                    }
                    Err(error) => {
                        self.offline.work.removal_retry.insert(
                            path,
                            std::time::Instant::now() + std::time::Duration::from_secs(30),
                        );
                        effects.push(Effect::Error(error));
                    }
                }
            }
            AppEvent::Authorization(state) => {
                if !self.auth.transition(state.clone()) {
                    if matches!(state, crate::telegram::AuthorizationState::Code { .. }) {
                        effects.push(Effect::AuthorizationChanged(state));
                    }
                    return effects;
                }
                effects.push(Effect::AuthorizationChanged(state.clone()));
                use crate::telegram::AuthorizationState;
                match state {
                    AuthorizationState::Phone => effects.extend(
                        self.command(AppCommand::Authenticate(crate::telegram::AuthRequest::Qr)),
                    ),
                    AuthorizationState::Parameters => effects.push(Effect::ParametersRequested),
                    AuthorizationState::Ready => {
                        self.offline.reconnect();
                        effects.extend(
                            self.library_command(crate::library::LibraryCommand::RefreshCatalog),
                        );
                    }
                    AuthorizationState::Closed => {
                        self.queue_work.cancel_page();
                        self.playback.cancel();
                        self.library.invalidate();
                        self.offline.suspend();
                        effects.push(Effect::Disconnect);
                    }
                    _ => {}
                }
            }
            AppEvent::Authenticated { request, result } => {
                if !self.auth.accepts(request) {
                    return effects;
                }
                self.auth.active = None;
                effects.push(Effect::AuthenticationFinished(result));
            }
            AppEvent::Snapshot(value) => self.playback.snapshot = value,
            AppEvent::PersistenceRejected(changes, error) => {
                self.retain_changes(*changes);
                return vec![Effect::Error(error)];
            }
            AppEvent::PlaybackPrepared {
                request,
                revision,
                result,
            } => {
                if !self.playback.accepts(request) {
                    return effects;
                }
                if let Some(track) = self.playback.target.clone() {
                    if self.revision(track.id()) != revision {
                        self.prepare_playback(track, &mut effects);
                        return effects;
                    }
                }
                match result {
                    Ok(prepared) => {
                        if self.revision(prepared.track.id()) != revision {
                            self.prepare_playback(prepared.track.into(), &mut effects);
                            return effects;
                        }
                        self.playback.active = None;
                        self.library.cache_track(&prepared.track);
                        self.handle(CoreCommand::Current(crate::library::metadata_track(
                            prepared.track,
                        )));
                        self.playback.target = self.state.session.current().map(Into::into);
                        if let Some(next) = self
                            .state
                            .session
                            .remaining()
                            .first()
                            .filter(|_| !self.queue_blocks_advance(false))
                        {
                            effects.push(Effect::Prefetch(next.into()));
                        }
                        if let Some(path) = prepared.path {
                            let mut start_ms = self.state.session.position_ms();
                            if self.state.session.current().is_some_and(|track| {
                                track.duration > 0
                                    && start_ms
                                        >= track.duration.saturating_add(1).saturating_mul(1000)
                            }) {
                                start_ms = 0;
                            }
                            effects.extend([
                                Effect::Player(PlayerCommand::Load { path, start_ms }),
                                Effect::Player(PlayerCommand::Pause(self.playback.state.paused())),
                                Effect::Player(PlayerCommand::Volume(self.state.volume)),
                                Effect::Notice(crate::notice::Notice::Clear),
                            ]);
                        } else {
                            self.playback.state.set_loading(false);
                            effects.push(Effect::Notice(crate::notice::Notice::DemoTracks));
                        }
                    }
                    Err(error) => {
                        self.playback.active = None;
                        effects.push(self.playback.fail(error));
                    }
                }
            }
            AppEvent::Player(PlaybackEvent::NextBuffering(request))
                if self.playback.promoting_next && self.playback.accepts(request) =>
            {
                self.playback.waiting_for_next_pcm = true;
            }
            AppEvent::Player(PlaybackEvent::NextUnavailable(request))
                if self.playback.promoting_next && self.playback.accepts(request) =>
            {
                if let Some(track) = self.playback.target.clone() {
                    let paused = self.playback.state.paused();
                    self.prepare_playback(track, &mut effects);
                    self.playback.state.set_paused(paused);
                }
            }
            AppEvent::Player(
                event @ (PlaybackEvent::Loaded
                | PlaybackEvent::Advanced(_)
                | PlaybackEvent::NextPromoted(_, _)),
            ) => {
                let advanced = match event {
                    PlaybackEvent::NextPromoted(request, next) => {
                        if !self.playback.promoting_next || !self.playback.accepts(request) {
                            return effects;
                        }
                        let Some(track) = self.playback.target.clone() else {
                            return effects;
                        };
                        if track.entry_id != next.entry_id
                            || self.revision(track.id()) != next.revision
                        {
                            let paused = self.playback.state.paused();
                            self.prepare_playback(track, &mut effects);
                            self.playback.state.set_paused(paused);
                            return effects;
                        }
                        self.playback.promoting_next = false;
                        self.playback.waiting_for_next_pcm = false;
                        self.playback.active = None;
                        Some(next)
                    }
                    PlaybackEvent::Advanced(next) => {
                        if self.playback.state.loading() || self.state.session.current().is_none() {
                            return effects;
                        }
                        let expected = if self.playback.streamed_next {
                            self.next_streamed_playback()
                                .is_some_and(|(track, revision, _)| {
                                    track.entry_id == next.entry_id && revision == next.revision
                                })
                        } else {
                            self.next_local_playback().as_ref() == Some(&next)
                        };
                        if !expected {
                            let blocked = self.queue_blocks_advance(true);
                            let mut effects = self.command(AppCommand::Advance(true));
                            if blocked {
                                effects.push(Effect::Player(PlayerCommand::Stop));
                            }
                            return effects;
                        }
                        self.playback.target = self.handle(CoreCommand::Advance(true));
                        self.playback.revision = next.revision;
                        Some(next)
                    }
                    PlaybackEvent::Loaded if self.playback.promoting_next => return effects,
                    _ => None,
                };
                let was_advanced = advanced.is_some();
                if let Some(next) = advanced {
                    if let Some(track) = &self.playback.target {
                        self.offline.mark_used(
                            track.id(),
                            &next.path,
                            crate::offline::unix_seconds(std::time::SystemTime::now()),
                        );
                    }
                    if let Some(next) = self.state.session.remaining().first() {
                        effects.push(Effect::Prefetch(next.into()));
                    }
                    effects.extend([
                        Effect::PlaybackAdvanced(next.path),
                        Effect::ClearSeek,
                        Effect::Notice(crate::notice::Notice::Clear),
                    ]);
                }
                if self.playback.state.loading() || was_advanced {
                    let chat = self.state.session.current().map(|track| track.chat);
                    if chat.is_some() && self.state.last_played_chat != chat {
                        let mut settings = crate::persistence::Settings::from(&self.state);
                        settings.last_played_chat = chat;
                        self.handle(CoreCommand::Settings(settings));
                    }
                }
                self.playback.state.set_loading(false);
            }
            AppEvent::Player(PlaybackEvent::Finished)
                if self.state.session.current().is_some() && !self.playback.state.loading() =>
            {
                return self.command(AppCommand::Advance(true));
            }
            AppEvent::Player(PlaybackEvent::Failed(error)) => {
                effects.push(self.playback.fail(error))
            }
            AppEvent::Player(_) => {}
        }
        self.persistence_effect(&mut effects);
        effects
    }
    fn persistence_effect(&mut self, effects: &mut Vec<Effect>) {
        let changes = self.take_changes();
        if !changes.is_empty() {
            effects.push(Effect::Persist(Box::new(changes)));
        }
    }
    pub(crate) fn revision(&self, id: gramwave_domain::TrackId) -> u64 {
        self.library.revision(id)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "workflow/review_tests.rs"]
mod review_tests;
