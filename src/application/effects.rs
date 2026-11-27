use crate::{App, Event};
use gramwave_application::{
    ports::{PlayerCommand, QueuedPlayback},
    tasks::{JobClass, Operation, RequestId},
    workflow::{AppEvent, Effect, PreparedPlayback},
    AppError,
};
impl App {
    pub(crate) fn core_event(&mut self, event: AppEvent) {
        let removed = match &event {
            AppEvent::OfflineRemoved {
                path,
                result: Ok(()),
                ..
            } if self.state.offline().removing.contains_key(path) => Some(path.clone()),
            _ => None,
        };
        let effects = self.state.event(event);
        if let Some(path) = removed.filter(|path| !self.state.offline().removing.contains_key(path))
        {
            self.cache_file_removed(&path);
        }
        self.run_effects(effects);
    }
    pub(crate) fn core_command(&mut self, command: gramwave_application::commands::AppCommand) {
        if self
            .player
            .as_ref()
            .is_some_and(|player| player.prepares_streamed_next())
        {
            self.process_player_events();
        }
        let snapshot = self
            .player
            .as_ref()
            .map(|p| p.snapshot())
            .unwrap_or_default();
        self.core_event(AppEvent::Snapshot(snapshot));
        let effects = self.state.command(command);
        self.run_effects(effects);
        self.wake.wake();
    }
    pub(crate) fn run_effects(&mut self, effects: Vec<Effect>) {
        let mut advanced = false;
        for effect in effects {
            match effect {
                Effect::Library(effect) => self.run_library_effect(effect),
                Effect::ScanOffline {
                    operation,
                    chat,
                    kind,
                    cursor,
                } => {
                    let request = operation.key();
                    if let Some(tg) = self.tg.clone() {
                        self.operation_event(
                            JobClass::Scan,
                            operation.clone(),
                            async move {
                                let result = crate::discovery::offline_page(
                                    &tg,
                                    chat,
                                    kind,
                                    cursor,
                                    &operation.cancel,
                                )
                                .await;
                                Event::Application(AppEvent::OfflinePage {
                                    request,
                                    chat,
                                    kind,
                                    result,
                                })
                            },
                            move |e| {
                                Event::Application(AppEvent::OfflinePage {
                                    request,
                                    chat,
                                    kind,
                                    result: Err(e),
                                })
                            },
                        );
                    } else {
                        self.core_event(AppEvent::OfflinePage {
                            request,
                            chat,
                            kind,
                            result: Err(AppError::Unavailable("Telegram")),
                        });
                    }
                }
                Effect::DownloadOffline { operation, track } => {
                    let request = operation.key();
                    let id = track.id();
                    if let Some(tg) = self.tg.clone() {
                        let dir = self.root.join("offline");
                        let progress = self.progress.clone();
                        self.operation_event(
                            JobClass::Offline,
                            operation.clone(),
                            async move {
                                let result = crate::adapters::offline::download(
                                    &tg,
                                    &track,
                                    &dir,
                                    request.generation,
                                    &progress,
                                    &operation.cancel,
                                )
                                .await;
                                Event::Application(AppEvent::OfflineDownloaded {
                                    request,
                                    id,
                                    result,
                                })
                            },
                            move |e| {
                                Event::Application(AppEvent::OfflineDownloaded {
                                    request,
                                    id,
                                    result: Err(e),
                                })
                            },
                        );
                    } else {
                        self.core_event(AppEvent::OfflineDownloaded {
                            request,
                            id,
                            result: Err(AppError::Unavailable("Telegram")),
                        });
                    }
                }
                Effect::RemoveOffline { operation, path } => {
                    let request = operation.key();
                    let failed_path = path.clone();
                    let root = self.root.join("offline");
                    let barrier = self.db.as_ref().map(|db| db.commit_barrier());
                    let demo = self.demo;
                    self.operation_event(
                        JobClass::Background,
                        operation.clone(),
                        async move {
                            use gramwave_application::ports::OfflineStorage;
                            let result = async {
                                if let Some(barrier) = barrier {
                                    barrier?
                                        .await
                                        .map_err(|e| AppError::Storage(e.to_string()))??;
                                }
                                if demo {
                                    return Ok(());
                                }
                                crate::adapters::offline::FileStorage::new(root)
                                    .remove(&path)
                                    .await
                            }
                            .await;
                            Event::Application(AppEvent::OfflineRemoved {
                                request,
                                path,
                                result,
                            })
                        },
                        move |e| {
                            Event::Application(AppEvent::OfflineRemoved {
                                request,
                                path: failed_path.clone(),
                                result: Err(e),
                            })
                        },
                    );
                }
                Effect::OfflineFailed(id, error) => {
                    self.error_retry = self
                        .state
                        .offline()
                        .pending
                        .get(&id)
                        .cloned()
                        .map(crate::RetryAction::Offline);
                    self.error = error.to_string();
                }
                Effect::Authenticate {
                    operation,
                    request: auth_request,
                } => {
                    let request = operation.key();
                    if let Some(tg) = self.tg.clone() {
                        self.operation_event(
                            JobClass::Interactive,
                            operation.clone(),
                            async move {
                                use gramwave_application::telegram::TelegramGateway;
                                let result = tg.authenticate(auth_request, &operation.cancel).await;
                                Event::Application(AppEvent::Authenticated { request, result })
                            },
                            move |e| {
                                Event::Application(AppEvent::Authenticated {
                                    request,
                                    result: Err(e),
                                })
                            },
                        );
                    } else {
                        self.core_event(AppEvent::Authenticated {
                            request,
                            result: Err(AppError::Unavailable("Telegram")),
                        });
                    }
                }
                Effect::AuthorizationChanged(state) => {
                    use gramwave_application::telegram::AuthorizationState;
                    self.password.clear();
                    match state {
                        AuthorizationState::Code { timeout, .. } => {
                            self.resend_at = Some(
                                std::time::Instant::now() + std::time::Duration::from_secs(timeout),
                            )
                        }
                        AuthorizationState::Confirmation(link) => self.qr = link,
                        AuthorizationState::Ready => {
                            if let Some(chat) = self.opened_chat {
                                self.library_command(
                                    gramwave_application::library::LibraryCommand::Open {
                                        chat,
                                        refresh: true,
                                    },
                                );
                            }
                        }
                        _ => {}
                    }
                }
                Effect::AuthenticationFinished(result) => {
                    self.password.clear();
                    match result {
                        Ok(()) => {
                            self.auth_error.clear();
                            self.code.clear();
                        }
                        Err(e) => {
                            self.auth_error = crate::auth::friendly_error(&e.to_string());
                            self.last_auth_step.clear();
                        }
                    }
                }
                Effect::ParametersRequested => {
                    if let Some((api_id, api_hash)) = crate::auth::TELEGRAM_CREDENTIALS {
                        self.submit_auth(gramwave_application::telegram::AuthRequest::Parameters(
                            gramwave_application::telegram::TelegramParameters {
                                api_id,
                                api_hash: api_hash.into(),
                                database: self.root.join("telegram").to_string_lossy().into(),
                                files: self.root.join("cache").to_string_lossy().into(),
                            },
                        ));
                    }
                }
                Effect::Connect => self.connect_native(),
                Effect::Quit => self.quitting = true,
                Effect::RetryRequested => self.retry_error(),
                Effect::SelectPlaylistRow(index) => self.select_playlist_row(index),
                Effect::OpenedPlaylist(chat) => {
                    if self.opened_chat != Some(chat) {
                        self.search.clear();
                    }
                    self.opened_chat = Some(chat);
                }
                Effect::Disconnect => {
                    let pending = self.preparing_next.is_some();
                    self.cancel_next_preparation();
                    if self.queued_playback.take().is_some() || pending {
                        if let Some(player) = &self.player {
                            let _ = player.command(PlayerCommand::QueueNext(None));
                        }
                    }
                    if let Some(server) = &self.server {
                        server.stop();
                    }
                    self.tg = None;
                    self.cache = Default::default();
                    self.stream_base.clear();
                    self.server.take();
                }

                Effect::Prefetch(track) => {
                    if self
                        .player
                        .as_ref()
                        .is_some_and(|player| player.prepares_streamed_next())
                    {
                        continue;
                    }
                    if self
                        .state
                        .offline()
                        .local_path(
                            track.id(),
                            gramwave_application::offline::unix_seconds(
                                std::time::SystemTime::now(),
                            ),
                            self.state.view().cache_policy.days,
                        )
                        .is_some()
                    {
                        continue;
                    }
                    let track = self.media_track(track);
                    if track.file > 0 {
                        if let Some(tg) = self.tg.clone() {
                            self.async_event(
                                JobClass::Background,
                                0,
                                async move {
                                    tg.prefetch(track.file).await;
                                    Event::Noop
                                },
                                |_| Event::Noop,
                            );
                        }
                    }
                }
                Effect::Player(command) => {
                    match &command {
                        PlayerCommand::Load { path, .. } => self.set_playback_path(path),
                        PlayerCommand::Stop => {
                            self.queued_playback = None;
                            self.cancel_next_preparation();
                        }
                        _ => {}
                    }
                    if let Some(player) = &self.player {
                        let result = player.command(command);
                        if let Err(error) = result {
                            self.core_event(AppEvent::Player(
                                gramwave_application::ports::PlaybackEvent::Failed(error),
                            ));
                        }
                    }
                }
                Effect::PlaybackAdvanced(path) => {
                    advanced = true;
                    self.set_playback_path(&path);
                    self.local_playback = None;
                    self.error.clear();
                    self.error_retry = self
                        .state
                        .view()
                        .session
                        .current()
                        .map(|track| crate::RetryAction::Playback(track.into()));
                }
                Effect::ClearSeek => self.seek_preview = None,
                Effect::Notice(message) => {
                    use gramwave_application::notice::Notice;
                    self.status = super::notices::notice_text(message);
                    if matches!(message, Notice::OfflineSaved | Notice::OfflineInterrupted) {
                        self.set_inline_notice(self.status.clone(), true);
                    }
                }
                Effect::Error(error) => self.error = error.to_string(),
                Effect::Persist(changes) => {
                    if let Some(db) = &self.db {
                        if let Err(error) = db.apply((*changes).clone()) {
                            self.core_event(AppEvent::PersistenceRejected(changes, error));
                        }
                    }
                }
                Effect::PreparePlayback {
                    operation,
                    track,
                    offline,
                    revision,
                } => {
                    let request = operation.key();
                    if operation.cancel.is_cancelled() {
                        continue;
                    }
                    let track = self.state.media_track(track);
                    self.error.clear();
                    self.error_retry = Some(crate::RetryAction::Playback(track.metadata()));
                    if self.demo {
                        self.core_event(AppEvent::PlaybackPrepared {
                            request,
                            revision,
                            result: Ok(PreparedPlayback { track, path: None }),
                        });
                        continue;
                    }
                    if self.player.is_none() {
                        self.core_event(AppEvent::PlaybackPrepared {
                            request,
                            revision,
                            result: Err(AppError::Player(self.player_error.clone())),
                        });
                        continue;
                    }
                    let tg = self.tg.clone();
                    let base = self.stream_base.clone();
                    self.operation_event(
                        JobClass::Playback,
                        operation.clone(),
                        async move {
                            let cancel = &operation.cancel;
                            let result = async {
                                let mut track = track;
                                if let Some(path) =
                                    offline.filter(|p| std::path::Path::new(p).is_file())
                                {
                                    return Ok(PreparedPlayback {
                                        track,
                                        path: Some(path),
                                    });
                                }
                                let _cache = match &tg {
                                    Some(tg) => Some(tg.cache_read(cancel).await?),
                                    None => None,
                                };
                                let path = {
                                    track
                                        .cached
                                        .clone()
                                        .filter(|p| std::path::Path::new(p).is_file())
                                };
                                let path = if let Some(path) = path {
                                    path
                                } else {
                                    if track.file == 0 {
                                        let tg =
                                            tg.as_ref().ok_or(AppError::Unavailable("Telegram"))?;
                                        track = crate::media_loader::resolve_async(
                                            tg,
                                            &track.metadata(),
                                            cancel,
                                        )
                                        .await?;
                                    }
                                    playback_location(&base, track.file, track.cached.as_deref())?
                                };
                                Ok(PreparedPlayback {
                                    track,
                                    path: Some(path),
                                })
                            }
                            .await;
                            Event::Application(AppEvent::PlaybackPrepared {
                                request,
                                revision,
                                result,
                            })
                        },
                        move |error| {
                            Event::Application(AppEvent::PlaybackPrepared {
                                request,
                                revision,
                                result: Err(error),
                            })
                        },
                    );
                }
            }
        }
        if self
            .player
            .as_ref()
            .is_some_and(|player| player.prepares_streamed_next())
        {
            self.prepare_streamed_next(advanced);
        } else if let Some(player) = &self.player {
            let next = self.state.next_local_playback();
            if advanced || next != self.queued_playback {
                let _ = player.command(PlayerCommand::QueueNext(next.clone()));
                self.queued_playback = next;
            }
        }
    }

    fn cancel_next_preparation(&mut self) {
        if let Some((_, _, operation)) = self.preparing_next.take() {
            operation.cancel.cancel();
        }
    }

    fn prepare_streamed_next(&mut self, advanced: bool) {
        if self.state.playback().promoting_next() {
            return;
        }
        let next = self.state.next_streamed_playback();
        if !advanced
            && next.as_ref().is_some_and(|(track, revision, _)| {
                self.preparing_next
                    .as_ref()
                    .is_some_and(|(entry_id, old_revision, _)| {
                        *entry_id == track.entry_id && old_revision == revision
                    })
            })
        {
            return;
        }
        let pending = self.preparing_next.is_some();
        self.cancel_next_preparation();
        let previous = self.queued_playback.take();
        let Some((mut track, revision, offline)) = next else {
            if (pending || previous.is_some()) && !advanced {
                let _ = self
                    .player
                    .as_ref()
                    .unwrap()
                    .command(PlayerCommand::QueueNext(None));
            }
            return;
        };
        let entry_id = track.entry_id;
        let build = |path| QueuedPlayback {
            entry_id,
            revision,
            path,
        };
        let path = offline
            .filter(|path| std::path::Path::new(path).is_file())
            .or_else(|| {
                playback_location(&self.stream_base, track.file, track.cached.as_deref()).ok()
            });
        let operation = Operation::new(self.state.playback().generation());
        let request = operation.key();
        self.preparing_next = Some((entry_id, revision, operation.clone()));
        if let Some(path) = path {
            self.next_playback_prepared(request, Some(build(path)));
        } else {
            let Some(tg) = self.tg.clone().filter(|_| !self.stream_base.is_empty()) else {
                self.next_playback_prepared(request, None);
                self.cancel_next_preparation();
                return;
            };
            let _ = self
                .player
                .as_ref()
                .unwrap()
                .command(PlayerCommand::QueueNextPending);
            let base = self.stream_base.clone();
            let cancel = operation.cancel.clone();
            self.operation_event(
                JobClass::Background,
                operation,
                async move {
                    let prepared = async {
                        track = crate::media_loader::resolve_async(&tg, &track.metadata(), &cancel)
                            .await
                            .ok()?;
                        let path =
                            playback_location(&base, track.file, track.cached.as_deref()).ok()?;
                        Some(QueuedPlayback {
                            entry_id,
                            revision,
                            path,
                        })
                    }
                    .await;
                    Event::NextPlaybackPrepared(request, prepared)
                },
                move |_| Event::NextPlaybackPrepared(request, None),
            );
        }
    }

    pub(crate) fn next_playback_prepared(
        &mut self,
        request: RequestId,
        next: Option<QueuedPlayback>,
    ) {
        if self
            .preparing_next
            .as_ref()
            .is_some_and(|(_, _, operation)| operation.key() == request)
        {
            if let Some(player) = &self.player {
                let _ = player.command(PlayerCommand::QueueNext(next.clone()));
                self.queued_playback = next;
            }
        }
    }

    fn set_playback_path(&mut self, path: &str) {
        use super::cache::CachePlayback;
        let offline = self
            .state
            .view()
            .session
            .current()
            .filter(|entry| {
                self.state
                    .offline()
                    .copies
                    .get(&entry.id())
                    .is_some_and(|copy| copy.path == path)
            })
            .map(|entry| (entry.id(), path.to_owned()));
        self.cache_playback = if offline.is_some() {
            CachePlayback::Offline
        } else if std::path::Path::new(path).is_absolute() {
            CachePlayback::Local
        } else {
            CachePlayback::Stream
        };
        self.local_playback = offline;
    }
}

fn playback_location(base: &str, file: i64, cached: Option<&str>) -> Result<String, AppError> {
    if let Some(path) = cached.filter(|p| std::path::Path::new(p).is_file()) {
        return Ok(path.into());
    }
    if base.is_empty() || file <= 0 {
        return Err(AppError::Unavailable("Streaming"));
    }
    Ok(format!("{base}/{file}"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_media_plays_without_streaming_and_network_media_requires_endpoint() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_str().unwrap();
        assert_eq!(playback_location("", 7, Some(path)), Ok(path.into()));
        assert_eq!(
            playback_location("", 7, None),
            Err(AppError::Unavailable("Streaming"))
        );
        assert_eq!(
            playback_location("", 7, Some("/missing-cache-file")),
            Err(AppError::Unavailable("Streaming"))
        );
        assert_eq!(
            playback_location("http://127.0.0.1:1234/token", 7, None),
            Ok("http://127.0.0.1:1234/token/7".into())
        );
    }
}
