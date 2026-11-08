use crate::{App, Event};
use tg_player_application::{
    tasks::JobClass,
    workflow::{AppEvent, Effect, PreparedPlayback},
    AppError,
};
impl App {
    pub(crate) fn core_event(&mut self, event: AppEvent) {
        let effects = self.state.event(event);
        self.run_effects(effects);
    }
    pub(crate) fn core_command(&mut self, command: tg_player_application::commands::AppCommand) {
        let snapshot = self
            .player
            .as_ref()
            .map(|p| p.snapshot())
            .unwrap_or_default();
        self.core_event(AppEvent::Snapshot(snapshot));
        let effects = self.state.command(command);
        self.run_effects(effects);
    }
    pub(crate) fn run_effects(&mut self, effects: Vec<Effect>) {
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
                    self.operation_event(
                        JobClass::Background,
                        operation.clone(),
                        async move {
                            use tg_player_application::ports::OfflineStorage;
                            let result = crate::adapters::offline::FileStorage::new(root)
                                .remove(&path)
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
                                use tg_player_application::telegram::TelegramGateway;
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
                    use tg_player_application::telegram::AuthorizationState;
                    self.password.clear();
                    match state {
                        AuthorizationState::Code { timeout, .. } => {
                            self.resend_at = Some(
                                std::time::Instant::now() + std::time::Duration::from_secs(timeout),
                            )
                        }
                        AuthorizationState::Confirmation(link) => self.qr = link,
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
                    self.submit_auth(tg_player_application::telegram::AuthRequest::Parameters(
                        tg_player_application::telegram::TelegramParameters {
                            api_id: self.state.view().api_id.parse().unwrap_or(0),
                            api_hash: self.state.view().api_hash.clone(),
                            database: self.root.join("telegram").to_string_lossy().into(),
                            files: self.root.join("cache").to_string_lossy().into(),
                        },
                    ))
                }
                Effect::Connect => self.connect_native(),
                Effect::ClearCache => self.clear_automatic_cache(),
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
                    if let Some(server) = &self.server {
                        server.stop();
                    }
                    self.tg = None;
                    self.stream_base.clear();
                    self.server.take();
                }

                Effect::Prefetch(track) => {
                    let track = self.media_track(track);
                    if track.file > 0 {
                        if let Some(tg) = &self.tg {
                            tg.prefetch(track.file);
                        }
                    }
                }
                Effect::Player(command) => {
                    if let Some(player) = &self.player {
                        let result = player.command(command);
                        if let Err(error) = result {
                            self.core_event(AppEvent::Player(
                                tg_player_application::ports::PlaybackEvent::Failed(error),
                            ));
                        }
                    }
                }
                Effect::ClearSeek => self.seek_preview = None,
                Effect::Notice(message) => self.status = super::notices::notice_text(message),
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
                                let path = offline
                                    .filter(|p| std::path::Path::new(p).is_file())
                                    .or_else(|| {
                                        track
                                            .cached
                                            .clone()
                                            .filter(|p| std::path::Path::new(p).is_file())
                                    });
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
    }
}

fn playback_location(base: &str, file: i64, cached: Option<&str>) -> Result<String, AppError> {
    if let Some(path) = cached.filter(|p| std::path::Path::new(p).is_file()) {
        return Ok(path.into());
    }
    if base.is_empty() {
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
