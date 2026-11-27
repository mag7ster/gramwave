use crate::{
    commands::AppCommand,
    model::Track,
    tasks::{Operation, RequestId},
    telegram::MediaPage,
    workflow::Effect,
    AppCore, AppError,
};
use gramwave_domain::{PlaylistOrder, RepeatMode};

#[derive(Default)]
pub(crate) struct QueueWork {
    active: Option<(Operation, usize, i64)>,
    pub(crate) waiting: Option<bool>,
    reversing: bool,
    oldest_start: Option<i64>,
    retry_reverse: bool,
    pub(crate) error: Option<AppError>,
}

impl QueueWork {
    pub(crate) fn cancel_page(&mut self) {
        if let Some((operation, _, _)) = self.active.take() {
            operation.cancel.cancel();
        }
        self.error = None;
    }

    pub(crate) fn reset(&mut self) {
        self.cancel_page();
        *self = Self::default();
    }
}

impl AppCore {
    pub fn queue_reversing(&self) -> bool {
        self.queue_work.reversing
    }

    pub fn queue_waiting(&self) -> bool {
        self.queue_work.waiting.is_some()
    }

    pub fn queue_page_error(&self) -> Option<&AppError> {
        self.queue_work.error.as_ref()
    }

    pub(crate) fn queue_blocks_advance(&self, eof: bool) -> bool {
        self.queue_work.reversing
            || (self.state.session.playlist_page_request().is_some()
                && self.state.session.available_len() == 0
                && !(eof
                    && !self.queue_waiting()
                    && self.state.repeat == RepeatMode::One
                    && self.state.session.current().is_some()))
    }

    pub fn queue_reverse_generation(&self) -> u64 {
        self.playback.queue_reverse_generation
    }

    pub fn oldest_loading(&self, chat: i64) -> bool {
        self.library
            .oldest
            .as_ref()
            .is_some_and(|(_, id, _)| *id == chat)
    }

    pub fn oldest_error(&self, chat: i64) -> Option<&AppError> {
        self.library
            .oldest_error
            .as_ref()
            .filter(|(id, _)| *id == chat)
            .map(|(_, error)| error)
    }

    pub(crate) fn cancel_oldest_start(&mut self, chat: i64) {
        if self.queue_work.oldest_start == Some(chat) {
            self.queue_work.oldest_start = None;
        }
    }

    pub(crate) fn start_playlist_queue(
        &mut self,
        chat: i64,
        library_id: Option<u64>,
        order: PlaylistOrder,
    ) -> Option<Track> {
        self.queue_work.oldest_start = None;
        if self.oldest_error(chat).is_some() {
            self.library.oldest_error = None;
        }
        if library_id.is_none()
            && order == PlaylistOrder::OldestFirst
            && self
                .state
                .playlists
                .iter()
                .find(|p| p.id == chat)
                .is_some_and(|p| p.has_more())
        {
            self.queue_work.oldest_start = Some(chat);
            return None;
        }
        self.start_ordered_playlist(chat, library_id, order)
    }

    fn start_ordered_playlist(
        &mut self,
        chat: i64,
        library_id: Option<u64>,
        order: PlaylistOrder,
    ) -> Option<Track> {
        let playlist = self.state.playlists.iter().find(|p| p.id == chat)?;
        let mut tracks: Vec<_> = playlist
            .tracks
            .iter()
            .filter(|t| playlist.filters[usize::from(t.kind)])
            .collect();
        if order == PlaylistOrder::OldestFirst {
            tracks.reverse();
        }
        let index = tracks
            .iter()
            .position(|t| library_id.is_none_or(|id| id == t.library_id))?;
        let selected = tracks[index].message;
        let cursors = std::array::from_fn(|kind| {
            (playlist.filters[kind] && !playlist.done[kind]).then(|| match order {
                PlaylistOrder::NewestFirst => playlist.cursors[kind],
                PlaylistOrder::OldestFirst => playlist.oldest_cursors[kind].max(selected),
            })
        });
        let tracks = tracks.into_iter().cloned().map(Into::into).collect();
        let current =
            self.state
                .session
                .start_playlist(tracks, index, cursors, self.state.shuffle, order)?;
        self.queue_work.reset();
        self.playback.repeat_cycle = None;
        self.playback.queue_removals.clear();
        self.record_session_change();
        Some(current.into())
    }

    pub(crate) fn pump_oldest_start(&mut self, effects: &mut Vec<Effect>) {
        let Some(chat) = self.queue_work.oldest_start else {
            return;
        };
        let Some(playlist) = self.state.playlists.iter().find(|p| p.id == chat) else {
            self.queue_work.oldest_start = None;
            return;
        };
        let frontier = (0..3)
            .filter(|&kind| playlist.filters[kind] && !playlist.done[kind])
            .map(|kind| playlist.oldest_cursors[kind])
            .min()
            .unwrap_or(i64::MAX);
        let oldest = playlist
            .tracks
            .iter()
            .rev()
            .find(|track| playlist.filters[usize::from(track.kind)]);
        if oldest.is_some_and(|track| track.message <= frontier) || !playlist.has_more() {
            self.queue_work.oldest_start = None;
            if let Some(track) = self.start_ordered_playlist(chat, None, PlaylistOrder::OldestFirst)
            {
                self.prepare_playback(track, effects);
            }
        } else {
            self.request_oldest(chat, false, effects);
        }
    }

    pub(crate) fn request_oldest(&mut self, chat: i64, retry: bool, effects: &mut Vec<Effect>) {
        if retry && self.oldest_error(chat).is_some() {
            self.library.oldest_error = None;
        }
        if self.library.oldest.is_some()
            || !self.library.connected()
            || self.oldest_error(chat).is_some()
        {
            return;
        }
        let Some(p) = self.state.playlists.iter().find(|p| p.id == chat) else {
            return;
        };
        let Some(kind) = (0..3)
            .filter(|&kind| p.filters[kind] && !p.done[kind])
            .min_by_key(|&kind| p.oldest_cursors[kind])
        else {
            return;
        };
        let operation = self.library.operation();
        self.library.oldest = Some((operation.clone(), chat, kind));
        effects.push(Effect::QueuePage {
            operation,
            chat,
            kind,
            cursor: p.oldest_cursors[kind],
            order: PlaylistOrder::OldestFirst,
            browsing: true,
        });
    }

    pub(crate) fn receive_oldest(
        &mut self,
        request: RequestId,
        result: Result<MediaPage, AppError>,
        effects: &mut Vec<Effect>,
    ) {
        let Some((_, chat, kind)) = self
            .library
            .oldest
            .take_if(|(op, _, _)| op.key() == request)
        else {
            return;
        };
        match result {
            Ok(page) => {
                let protected = self.library.protected(chat);
                let tracks = page
                    .tracks
                    .into_iter()
                    .filter(|track| !protected.contains(&track.message))
                    .map(|track| {
                        self.library.cache_track(&track);
                        Track::from(track)
                    })
                    .collect();
                self.handle(crate::core::CoreCommand::Patch(
                    crate::core::LibraryPatch {
                        chat,
                        tracks,
                        oldest_page: Some((kind, page.next)),
                        ..Default::default()
                    },
                    protected,
                ));
                self.pump_oldest_start(effects);
            }
            Err(error) => self.library.oldest_error = Some((chat, error)),
        }
    }

    pub(crate) fn request_queue_reverse(&mut self) {
        self.queue_work.retry_reverse = false;
        if self.state.session.playlist_page_request().is_some() {
            self.queue_work.reversing = true;
            self.queue_work.error = None;
        } else {
            self.reverse_queue();
        }
    }

    pub(crate) fn retry_queue_page(&mut self) {
        self.queue_work.error = None;
        if std::mem::take(&mut self.queue_work.retry_reverse) {
            self.request_queue_reverse();
        }
    }

    fn queue_page_failed(&mut self, error: AppError, effects: &mut Vec<Effect>) {
        self.queue_work.error = Some(error);
        self.queue_work.retry_reverse = std::mem::take(&mut self.queue_work.reversing);
        self.resume_queue_wait(effects);
    }

    fn reverse_queue(&mut self) {
        self.queue_work.reversing = false;
        if self.state.session.reverse_queue() {
            self.playback.queue_reverse_generation =
                self.playback.queue_reverse_generation.wrapping_add(1);
            self.playback.repeat_cycle = None;
            self.record_session_change();
        }
    }

    pub(crate) fn pump_queue(&mut self, effects: &mut Vec<Effect>) {
        if self.queue_work.active.is_some() || self.queue_work.error.is_some() {
            return;
        }
        let session = &self.state.session;
        if !self.queue_work.reversing
            && !self.queue_waiting()
            && (session.current().is_none() || session.available_len() > 3)
        {
            return;
        }
        if let Some((chat, kind, cursor)) = session.playlist_page_request() {
            let operation = self.library.operation();
            self.queue_work.active = Some((operation.clone(), kind, cursor));
            effects.push(Effect::QueuePage {
                operation,
                chat,
                kind,
                cursor,
                order: session.playlist_order(),
                browsing: false,
            });
        }
    }

    pub(crate) fn receive_queue_page(
        &mut self,
        request: RequestId,
        result: Result<MediaPage, AppError>,
        effects: &mut Vec<Effect>,
    ) {
        let Some((_, kind, cursor)) = self
            .queue_work
            .active
            .as_ref()
            .filter(|(op, _, _)| op.key() == request)
        else {
            return;
        };
        let (kind, cursor) = (*kind, *cursor);
        self.queue_work.active = None;
        let page = match result {
            Ok(page)
                if page.next == 0
                    || cursor == 0
                    || self.state.session.playlist_order().key(page.next)
                        > self.state.session.playlist_order().key(cursor) =>
            {
                page
            }
            Ok(_) => {
                self.queue_page_failed(
                    AppError::Protocol("Queue pagination did not advance".into()),
                    effects,
                );
                return;
            }
            Err(error) => {
                self.queue_page_failed(error, effects);
                return;
            }
        };
        let tracks = page.tracks.into_iter().filter_map(|track| {
            if self.library.source_changed(track.id()) {
                return None;
            }
            self.library.cache_track(&track);
            Some(Track::from(track).into())
        });
        self.state
            .session
            .add_playlist_page(kind, page.next, tracks, self.state.shuffle);
        self.record_session_change();
        if self.queue_work.reversing && self.state.session.playlist_page_request().is_none() {
            self.reverse_queue();
        }
        self.resume_queue_wait(effects);
    }

    pub(crate) fn resume_queue_wait(&mut self, effects: &mut Vec<Effect>) {
        if let Some(eof) = self
            .queue_work
            .waiting
            .filter(|&eof| !self.queue_blocks_advance(eof))
        {
            self.queue_work.waiting = None;
            let paused = self.playback.state == crate::playback::PlaybackState::Paused;
            effects.extend(self.command(AppCommand::Advance(eof)));
            if paused && self.playback.state.loading() {
                self.playback.state.set_paused(true);
            }
        }
    }
}

#[cfg(test)]
mod tests;
