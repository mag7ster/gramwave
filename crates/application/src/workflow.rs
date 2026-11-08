//! Application decisions and adapter effects. This module has no desktop dependencies.
use crate::{
    commands::AppCommand,
    core::{AppCore, CoreCommand},
    model::Track,
    persistence::ChangeSet,
    ports::{PlaybackEvent, PlayerCommand, PlayerSnapshot},
    tasks::{Operation, RequestId},
    AppError,
};

pub enum Effect {
    Library(crate::library::LibraryEffect),
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
    OfflineFailed(tg_player_domain::TrackId, AppError),
    Authenticate {
        operation: Operation,
        request: crate::telegram::AuthRequest,
    },
    AuthorizationChanged(crate::telegram::AuthorizationState),
    AuthenticationFinished(Result<(), AppError>),
    ParametersRequested,
    Connect,
    ClearCache,
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
    Prefetch(Track),
    Persist(Box<ChangeSet>),
    Notice(crate::notice::Notice),
    Error(AppError),
    ClearSeek,
}
pub enum AppEvent {
    Library(crate::library::LibraryEvent),
    OfflinePage {
        request: RequestId,
        chat: i64,
        kind: usize,
        result: Result<crate::telegram::MediaPage, AppError>,
    },
    OfflineDownloaded {
        request: RequestId,
        id: tg_player_domain::TrackId,
        result: Result<String, AppError>,
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
        self.offline.pump(now, online)
    }

    pub fn shutdown_operations(&mut self) {
        self.playback.cancel();
        self.auth.cancel();
        self.offline.cancel_work();
        self.library.cancel_operations();
    }

    pub(crate) fn offline_source_changed(&mut self, id: tg_player_domain::TrackId) {
        self.invalidate_track(id);
        self.offline.cancel_attempt(id);
        self.offline.source_changed(id);
    }
    pub fn auth(&self) -> &crate::auth::AuthCore {
        &self.auth
    }

    pub(crate) fn invalidate_track(&mut self, id: tg_player_domain::TrackId) {
        self.library.invalidate_track(id);
    }
    pub fn playback(&self) -> &crate::playback::PlaybackCore {
        &self.playback
    }
    pub fn command(&mut self, command: AppCommand) -> Vec<Effect> {
        let mut effects = Vec::new();
        let advancing = matches!(&command, AppCommand::Next | AppCommand::Advance(_));
        let mut play = None;
        match command {
            AppCommand::Connect => {
                if !self.library.connected() {
                    self.auth.cancel();
                    self.playback.cancel();
                    self.library.invalidate();
                    self.auth
                        .transition(crate::telegram::AuthorizationState::Starting);
                    effects.extend([
                        Effect::Connect,
                        Effect::Notice(crate::notice::Notice::Connecting),
                    ]);
                }
            }
            AppCommand::ClearCache => effects.push(Effect::ClearCache),
            AppCommand::Quit => effects.push(Effect::Quit),
            AppCommand::Retry => effects.push(Effect::RetryRequested),
            AppCommand::SelectPlaylistRow(index) => effects.push(Effect::SelectPlaylistRow(index)),
            AppCommand::OpenPlaylist(chat) => {
                effects.push(Effect::OpenedPlaylist(chat));
                effects.extend(self.library_command(crate::library::LibraryCommand::Open {
                    chat,
                    refresh: true,
                }));
            }
            AppCommand::SaveTrack(track) => {
                self.offline.work.retry.remove(&track.id());
                self.offline
                    .work
                    .errors
                    .remove(&(track.chat, track.message));
                self.offline.enqueue(track, true);
                self.offline.work.scan_retry = None;
            }
            AppCommand::SavePlaylist(chat, filters) => {
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
                    self.offline.remove(id);
                }
            }
            AppCommand::Authenticate(request) => {
                if let Some(operation) = self.auth.begin() {
                    effects.push(Effect::Authenticate { operation, request });
                }
            }
            AppCommand::StartSession(tracks, index) => {
                play = self.handle(CoreCommand::Start(tracks, index))
            }
            AppCommand::Play(track) => play = Some(track),
            AppCommand::PlayQueue(id) => play = self.handle(CoreCommand::Jump(id)),
            AppCommand::Next => play = self.handle(CoreCommand::Advance(false)),
            AppCommand::Advance(eof) => play = self.handle(CoreCommand::Advance(eof)),
            AppCommand::Previous => {
                if !self.playback.state.loading() && self.playback.snapshot.position > 3. {
                    return vec![Effect::Player(PlayerCommand::Seek(0.)), Effect::ClearSeek];
                }
                play = self.handle(CoreCommand::Previous);
            }
            AppCommand::Stop => {
                self.playback.cancel();
                self.playback.state = crate::playback::PlaybackState::Stopped;
                self.handle(CoreCommand::Stop);
                effects.extend([Effect::Player(PlayerCommand::Stop), Effect::ClearSeek]);
            }
            AppCommand::Pause(paused) => {
                if self.state.session.current().is_some() {
                    if !paused
                        && self.playback.state.paused()
                        && !self.playback.state.loading()
                        && (self.playback.state == crate::playback::PlaybackState::Failed
                            || self.playback.snapshot.duration <= 0.)
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
                self.handle(CoreCommand::Filters(id, value));
            }
            AppCommand::Insert(track, next) => {
                self.handle(CoreCommand::Insert(track, next));
            }
            AppCommand::Move(from, to) => {
                self.handle(CoreCommand::Move(from, to));
            }
            AppCommand::Remove(index) => {
                self.handle(CoreCommand::Remove(index));
            }
            AppCommand::Clear => {
                self.handle(CoreCommand::Clear);
            }
            AppCommand::ToggleChatPin(id) => {
                self.handle(CoreCommand::ToggleChatPin(id));
            }
            AppCommand::Volume(_) | AppCommand::Seek(_) => {}
        }
        if let Some(track) = play {
            self.prepare_playback(track, &mut effects);
        } else if advancing {
            self.playback.cancel();
            self.playback.state = crate::playback::PlaybackState::Stopped;
            effects.extend([Effect::Player(PlayerCommand::Stop), Effect::ClearSeek]);
        }
        self.persistence_effect(&mut effects);
        effects
    }
    pub(crate) fn prepare_playback(&mut self, track: Track, effects: &mut Vec<Effect>) {
        let revision = self.revision(track.id());
        let operation = self.playback.begin(track.clone(), revision);
        let offline = self.offline.path(track.id()).map(str::to_owned);
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
                        if let Some(next) = self.state.session.remaining().first() {
                            effects.push(Effect::Prefetch(next.into()));
                        }
                        if let Some(path) = prepared.path {
                            effects.extend([
                                Effect::Player(PlayerCommand::Load(path)),
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
            AppEvent::Player(PlaybackEvent::Loaded) => self.playback.state.set_loading(false),
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
    pub(crate) fn revision(&self, id: tg_player_domain::TrackId) -> u64 {
        self.library.revision(id)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "workflow/review_tests.rs"]
mod review_tests;
