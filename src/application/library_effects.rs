//! Execute library effects with native services; decisions live in LibraryCore.
use crate::{App, Event, RetryAction};
use tg_player_application::{
    core::LibraryPatch,
    library::{LibraryEffect, LibraryEvent, LibraryRetry},
    tasks::JobClass,
    workflow::AppEvent,
    AppError,
};

impl App {
    pub(crate) fn run_library_effect(&mut self, effect: LibraryEffect) {
        match effect {
            LibraryEffect::Media {
                operation,
                connection,
                track,
                revision,
            } => {
                let request = operation.key();
                let id = track.id();
                let Some(tg) = self.tg.clone() else {
                    self.core_event(AppEvent::Library(LibraryEvent::Media {
                        request,
                        connection,
                        id,
                        revision,
                        now: std::time::Instant::now(),
                        result: Err(AppError::Unavailable("Telegram")),
                    }));
                    return;
                };
                let cancel = operation.cancel.clone();
                self.operation_event(
                    JobClass::Background,
                    operation,
                    async move {
                        let result = crate::media_loader::resolve_async(&tg, &track, &cancel).await;
                        Event::Application(AppEvent::Library(LibraryEvent::Media {
                            request,
                            connection,
                            id,
                            revision,
                            result,
                            now: std::time::Instant::now(),
                        }))
                    },
                    move |error| {
                        Event::Application(AppEvent::Library(LibraryEvent::Media {
                            request,
                            connection,
                            id,
                            revision,
                            result: Err(error),
                            now: std::time::Instant::now(),
                        }))
                    },
                );
            }
            LibraryEffect::Artwork {
                operation,
                connection,
                id,
            } => {
                let request = operation.key();
                let Some(tg) = self.tg.clone() else {
                    self.core_event(AppEvent::Library(LibraryEvent::Artwork {
                        request,
                        connection,
                        id,
                        now: std::time::Instant::now(),
                        result: Err(AppError::Unavailable("Telegram")),
                    }));
                    return;
                };
                let cancel = operation.cancel.clone();
                self.operation_event(
                    JobClass::Background,
                    operation,
                    async move {
                        let result = tg.artwork(id, &cancel).await;
                        Event::Application(AppEvent::Library(LibraryEvent::Artwork {
                            request,
                            connection,
                            id,
                            result,
                            now: std::time::Instant::now(),
                        }))
                    },
                    move |error| {
                        Event::Application(AppEvent::Library(LibraryEvent::Artwork {
                            request,
                            connection,
                            id,
                            result: Err(error),
                            now: std::time::Instant::now(),
                        }))
                    },
                );
            }
            LibraryEffect::Catalog { operation, filters } => {
                let request = operation.key();
                let Some(tg) = self.tg.clone() else {
                    self.core_event(AppEvent::Library(LibraryEvent::Catalog {
                        request,
                        result: Err(AppError::Unavailable("Telegram")),
                    }));
                    return;
                };
                let cancel = operation.cancel.clone();
                self.operation_event(
                    JobClass::Background,
                    operation,
                    async move {
                        let result = crate::discovery::catalog(&tg, filters, &cancel).await;
                        Event::Application(AppEvent::Library(LibraryEvent::Catalog {
                            request,
                            result,
                        }))
                    },
                    move |error| {
                        Event::Application(AppEvent::Library(LibraryEvent::Catalog {
                            request,
                            result: Err(error),
                        }))
                    },
                );
            }
            LibraryEffect::Search {
                operation,
                query,
                filters,
            } => {
                let request = operation.key();
                let Some(tg) = self.tg.clone() else {
                    self.core_event(AppEvent::Library(LibraryEvent::Search {
                        request,
                        query,
                        result: Err(AppError::Unavailable("Telegram")),
                    }));
                    return;
                };
                let failed_query = query.clone();
                let cancel = operation.cancel.clone();
                self.operation_event(
                    JobClass::Background,
                    operation,
                    async move {
                        let result =
                            crate::discovery::search_chats(&tg, &query, filters, &cancel).await;
                        Event::Application(AppEvent::Library(LibraryEvent::Search {
                            request,
                            query,
                            result,
                        }))
                    },
                    move |error| {
                        Event::Application(AppEvent::Library(LibraryEvent::Search {
                            request,
                            query: failed_query.clone(),
                            result: Err(error),
                        }))
                    },
                );
            }
            LibraryEffect::Index {
                operation,
                playlist,
                refresh,
            } => {
                let request = operation.key();
                let Some(tg) = self.tg.clone() else {
                    self.core_event(AppEvent::Library(LibraryEvent::Indexed {
                        request,
                        result: Err(AppError::Unavailable("Telegram")),
                    }));
                    return;
                };
                let tx = self.tx.clone();
                let cancel = operation.cancel.clone();
                self.operation_event(
                    JobClass::Background,
                    operation,
                    async move {
                        if refresh && playlist.url.is_empty() {
                            if let Ok(url) = tg.chat_url(playlist.id, &cancel).await {
                                let _ = tx.send(Event::Application(AppEvent::Library(
                                    LibraryEvent::Patch {
                                        request,
                                        patch: LibraryPatch {
                                            chat: playlist.id,
                                            url: Some(url),
                                            ..Default::default()
                                        }
                                        .into(),
                                    },
                                )));
                            }
                        }
                        let result = if refresh {
                            crate::discovery::refresh_library(&tg, playlist, &cancel, |patch| {
                                let _ = tx.send(Event::Application(AppEvent::Library(
                                    LibraryEvent::Patch { request, patch },
                                )));
                            })
                            .await
                        } else {
                            crate::discovery::index_library(&tg, playlist, &cancel, |playlist| {
                                let _ = tx.send(Event::Application(AppEvent::Library(
                                    LibraryEvent::Page { request, playlist },
                                )));
                            })
                            .await
                        };
                        Event::Application(AppEvent::Library(LibraryEvent::Indexed {
                            request,
                            result,
                        }))
                    },
                    move |error| {
                        Event::Application(AppEvent::Library(LibraryEvent::Indexed {
                            request,
                            result: Err(error),
                        }))
                    },
                );
            }
            LibraryEffect::Failed { error, retry } => {
                self.error = error.to_string();
                self.error_retry = Some(match retry {
                    LibraryRetry::Catalog => RetryAction::Catalog,
                    LibraryRetry::Chat(chat) => RetryAction::Chat(chat),
                });
            }
            LibraryEffect::ForgetArtwork(path) => {
                let _ = self.tx.send(Event::ForgetArtwork(path));
            }
        }
    }
}
