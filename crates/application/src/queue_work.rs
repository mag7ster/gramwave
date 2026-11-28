use crate::{
    commands::{AppCommand, PlaylistStart},
    model::Track,
    tasks::{Operation, RequestId},
    telegram::MediaPage,
    workflow::Effect,
    AppCore, AppError,
};
use gramwave_domain::{PlaylistOrder, RepeatMode};

#[derive(Clone, Copy)]
struct QueuePageRequest {
    kind: usize,
    cursor: i64,
    order: PlaylistOrder,
    window: Option<bool>,
}

#[derive(Default)]
pub(crate) struct QueueWork {
    active: Option<(Operation, QueuePageRequest)>,
    pub(crate) waiting_previous: bool,
    pub(crate) waiting: Option<bool>,
    pub(crate) prefetch_previous: bool,
    oldest_start: Option<i64>,
    oldest_start_retry: crate::page_browse::PageRetry,
    pub(crate) error: Option<AppError>,
    pub(crate) page_retry: crate::page_browse::PageRetry,
    page_generation: u64,
    pub(crate) shuffle_request: Option<crate::tasks::RequestId>,
}

impl QueueWork {
    pub(crate) fn cancel_page(&mut self) {
        if let Some((operation, _)) = self.active.take() {
            operation.cancel.cancel();
        }
        self.error = None;
        self.page_retry = Default::default();
    }

    pub(crate) fn page_active_on(&self, session: &gramwave_domain::Session, before: bool) -> bool {
        self.active
            .as_ref()
            .is_some_and(|(_, page)| session.display_before(page.window) == before)
    }

    pub(crate) fn reset(&mut self) {
        self.cancel_page();
        *self = Self {
            page_generation: self.page_generation,
            ..Self::default()
        };
    }
}

impl AppCore {
    pub fn queue_waiting(&self) -> bool {
        self.queue_work.waiting.is_some() || self.queue_work.waiting_previous
    }

    pub fn queue_page_generation(&self) -> u64 {
        self.queue_work.page_generation
    }

    pub fn queue_page_error(&self) -> Option<&AppError> {
        self.queue_work.error.as_ref()
    }

    pub(crate) fn queue_blocks_advance(&self, eof: bool) -> bool {
        self.state.session.playlist_page_request().is_some()
            && self.state.session.available_len() == 0
            && !(eof
                && !self.queue_waiting()
                && self.state.repeat == RepeatMode::One
                && self.state.session.current().is_some())
    }

    pub fn queue_reverse_generation(&self) -> u64 {
        self.playback.queue_reverse_generation
    }

    pub fn oldest_loading(&self, chat: i64) -> bool {
        self.library
            .oldest
            .as_ref()
            .is_some_and(|(_, id, _, _)| *id == chat)
    }

    pub fn oldest_error(&self, chat: i64) -> Option<&AppError> {
        self.library
            .oldest_error
            .as_ref()
            .filter(|(id, _)| *id == chat)
            .map(|(_, error)| error)
    }

    pub(crate) fn cancel_oldest_start(&mut self, chat: Option<i64>) {
        if chat.is_none() || self.queue_work.oldest_start == chat {
            self.queue_work.oldest_start = None;
        }
    }

    pub(crate) fn start_playlist_queue(
        &mut self,
        chat: i64,
        start: PlaylistStart,
        order: PlaylistOrder,
        first_sample: Option<Track>,
    ) -> Option<Track> {
        self.queue_work.oldest_start = None;
        self.queue_work.oldest_start_retry = Default::default();
        if self.oldest_error(chat).is_some() {
            self.library.oldest_error = None;
        }
        if matches!(start, PlaylistStart::Ordered)
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
        self.start_ordered_playlist(chat, start, order, first_sample)
    }

    fn start_ordered_playlist(
        &mut self,
        chat: i64,
        start: PlaylistStart,
        order: PlaylistOrder,
        first_sample: Option<Track>,
    ) -> Option<Track> {
        let playlist = self.state.playlists.iter().find(|p| p.id == chat)?;
        let mut tracks: Vec<_> = playlist
            .tracks
            .iter()
            .filter(|t| playlist.filters[usize::from(t.kind)])
            .cloned()
            .collect();
        let sampled_message = first_sample.as_ref().map(|track| track.message);
        if let Some(track) = first_sample {
            let index = tracks.partition_point(|known| known.message > track.message);
            if tracks
                .get(index)
                .is_none_or(|known| known.message != track.message)
            {
                tracks.insert(index, track);
            }
        }
        if order == PlaylistOrder::OldestFirst {
            tracks.reverse();
        }
        let library_id = match start {
            PlaylistStart::Track(id) => Some(id),
            _ => None,
        };
        let index = tracks.iter().position(|t| {
            library_id.is_none_or(|id| id == t.library_id)
                && sampled_message.is_none_or(|id| id == t.message)
        })?;
        let cursors = std::array::from_fn(|kind| {
            (playlist.filters[kind] && !playlist.done[kind]).then(|| match order {
                PlaylistOrder::NewestFirst => playlist.cursors[kind],
                PlaylistOrder::OldestFirst => playlist.oldest_cursors[kind],
            })
        });
        let tracks = tracks.into_iter().map(Into::into).collect();
        let shuffle = match start {
            PlaylistStart::Ordered => false,
            PlaylistStart::Shuffled => true,
            PlaylistStart::Track(_) => self.state.shuffle,
        };
        let current = if matches!(start, PlaylistStart::Shuffled) && sampled_message.is_none() {
            self.state
                .session
                .start_shuffled_playlist(tracks, cursors, order)?
        } else {
            self.state
                .session
                .start_playlist(tracks, index, cursors, shuffle, order)?
        };
        if self.state.shuffle != shuffle {
            self.state.shuffle = shuffle;
            self.record_settings_change();
        }
        self.queue_work.reset();
        self.playback.repeat_cycle = None;
        self.playback.queue_started();
        self.record_session_change();
        Some(current.into())
    }

    pub(crate) fn pump_oldest_start(&mut self, now: std::time::Instant, effects: &mut Vec<Effect>) {
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
            if let Some(track) = self.start_ordered_playlist(
                chat,
                PlaylistStart::Ordered,
                PlaylistOrder::OldestFirst,
                None,
            ) {
                self.prepare_playback(track, effects);
            }
        } else if self.paging_allowed(now) && self.queue_work.oldest_start_retry.ready(now) {
            self.request_oldest(chat, true, effects);
        }
    }

    pub(crate) fn oldest_start_retry_deadline(&self) -> Option<std::time::Instant> {
        self.queue_work
            .oldest_start
            .and(self.queue_work.oldest_start_retry.at)
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
        self.library.oldest = Some((
            operation.clone(),
            chat,
            kind,
            self.playlist_browse_key(chat, PlaylistOrder::OldestFirst),
        ));
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
        let Some((_, chat, kind, browse)) = self
            .library
            .oldest
            .take_if(|(op, _, _, _)| op.key() == request)
        else {
            return;
        };
        let browse = browse.or_else(|| self.playlist_browse_key(chat, PlaylistOrder::OldestFirst));
        if let Some(key) = browse {
            self.playlist_page_finished(key, &result.as_ref().map(|_| ()).map_err(Clone::clone));
        }
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
                if self.queue_work.oldest_start == Some(chat) {
                    self.queue_work.oldest_start_retry = Default::default();
                }
                self.pump_oldest_start(std::time::Instant::now(), effects);
            }
            Err(error) => {
                self.record_page_cooldown(&error, std::time::Instant::now());
                if self.queue_work.oldest_start == Some(chat) {
                    self.queue_work
                        .oldest_start_retry
                        .failed(&error, std::time::Instant::now());
                }
                self.library.oldest_error = Some((chat, error));
            }
        }
    }

    pub(crate) fn request_queue_reverse(&mut self) {
        if self.state.session.reverse_queue() {
            self.queue_work.waiting_previous = false;
            self.queue_work.prefetch_previous = false;
            self.complete_queue_browse();
            self.playback.queue_reverse_generation =
                self.playback.queue_reverse_generation.wrapping_add(1);
            self.playback.repeat_cycle = None;
            self.record_session_change();
        }
    }

    pub(crate) fn retry_queue_page(&mut self) {
        self.queue_work.error = None;
        self.queue_work.page_retry = Default::default();
    }

    fn queue_page_failed(&mut self, error: AppError, effects: &mut Vec<Effect>) {
        if error == AppError::Cancelled {
            return;
        }
        let now = std::time::Instant::now();
        self.record_page_cooldown(&error, now);
        self.queue_work.page_retry.failed(&error, now);
        self.queue_work.error = Some(error);
        self.resume_queue_wait(effects);
    }

    pub(crate) fn queue_needs_retry(&self) -> bool {
        self.next_queue_page().is_some()
    }

    fn next_queue_page(&self) -> Option<(i64, QueuePageRequest)> {
        let session = &self.state.session;
        let order = session.playlist_source_order();
        let reversed = order != session.playlist_order();
        let request = |before| {
            let (chat, kind, cursor) = session.page_request(before)?;
            let source_before = before ^ reversed;
            let window = session.has_ordered_window().then_some(source_before);
            Some((
                chat,
                QueuePageRequest {
                    kind,
                    cursor,
                    window,
                    order: if window == Some(true) {
                        order.reversed()
                    } else {
                        order
                    },
                },
            ))
        };
        let forward_needed = self.queue_work.waiting.is_some()
            || self.queue_browse_pending_on(false)
            || (session.current().is_some()
                && session.available_len() <= crate::page_browse::QUEUE_PREFETCH_ROWS);
        let previous_needed = self.queue_browse_pending_on(true)
            || self.queue_work.prefetch_previous
                && session.available_past_len() <= crate::page_browse::QUEUE_PREFETCH_ROWS;
        if let Some(previous) = self
            .queue_work
            .waiting_previous
            .then(|| request(true))
            .flatten()
        {
            return Some(previous);
        }
        let initial = |before| session.needs_ordered_neighbors(before ^ reversed);
        if let Some(forward) = (forward_needed || initial(false))
            .then(|| request(false))
            .flatten()
        {
            return Some(forward);
        }
        (previous_needed || initial(true))
            .then(|| request(true))
            .flatten()
    }

    pub(crate) fn pump_queue(&mut self, now: std::time::Instant, effects: &mut Vec<Effect>) {
        if self.queue_work.active.is_some() || !self.queue_work.page_retry.ready(now) {
            return;
        }
        if let Some((chat, page)) = self.next_queue_page() {
            let operation = self.library.operation();
            self.queue_work.active = Some((operation.clone(), page));
            effects.push(Effect::QueuePage {
                operation,
                chat,
                kind: page.kind,
                cursor: page.cursor,
                order: page.order,
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
        let Some((_, page)) = self
            .queue_work
            .active
            .take_if(|(op, _)| op.key() == request)
        else {
            return;
        };
        let QueuePageRequest {
            kind,
            cursor,
            order,
            window,
        } = page;
        let page = match result {
            Ok(page)
                if page.next == 0 || cursor == 0 || order.key(page.next) > order.key(cursor) =>
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
        let display_before = self.state.session.display_before(window);
        self.complete_queue_browse_on(display_before);
        self.queue_work.page_retry = Default::default();
        self.queue_work.error = None;
        let before = self.state.session.timeline().len();
        let tracks = page.tracks.into_iter().filter_map(|track| {
            if self.library.source_changed(track.id()) {
                return None;
            }
            self.library.cache_track(&track);
            Some(Track::from(track).into())
        });
        if let Some(before) = window {
            self.state
                .session
                .add_ordered_page(before, kind, page.next, tracks);
        } else {
            self.state
                .session
                .add_playlist_page(kind, page.next, tracks, self.state.shuffle);
        }
        if self.state.session.timeline().len() > before {
            self.queue_work.page_generation = self.queue_work.page_generation.wrapping_add(1);
        }
        self.record_session_change();
        if self.state.session.available_past_len() > crate::page_browse::QUEUE_PREFETCH_ROWS
            || self.state.session.page_request(true).is_none()
        {
            self.queue_work.prefetch_previous = false;
        }
        self.resume_queue_wait(effects);
    }

    pub(crate) fn previous_neighbor_missing(&self) -> bool {
        let session = &self.state.session;
        session.available_past_len() == 0 && session.page_request(true).is_some()
    }

    pub(crate) fn resume_queue_wait(&mut self, effects: &mut Vec<Effect>) {
        if self.queue_work.waiting_previous && !self.previous_neighbor_missing() {
            self.queue_work.waiting_previous = false;
            let paused = self.playback.state == crate::playback::PlaybackState::Paused;
            if let Some(track) = self.handle(crate::core::CoreCommand::Previous) {
                self.prepare_playback(track, effects);
                if paused {
                    self.playback.state.set_paused(true);
                }
            }
        }
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
