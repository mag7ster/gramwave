use crate::{workflow::Effect, AppCore, AppError};
use gramwave_domain::PlaylistOrder;
use std::time::{Duration, Instant};

pub const QUEUE_PREFETCH_ROWS: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Browse {
    Down,
    Nearby,
    Away,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageStatus {
    Idle,
    Loading,
    Unavailable,
}

#[derive(Default)]
pub(crate) struct PageRetry {
    failures: u32,
    pub(crate) at: Option<Instant>,
    terminal: bool,
}

impl PageRetry {
    pub(crate) fn failed(&mut self, error: &AppError, now: Instant) {
        if matches!(
            error,
            AppError::Telegram {
                code: 401 | 403 | 404,
                ..
            }
        ) {
            self.terminal = true;
            self.at = None;
            return;
        }
        let delay = (1_u64 << self.failures.min(4)).min(15);
        self.failures = self.failures.saturating_add(1);
        let wait = telegram_wait(error).unwrap_or_default();
        self.at = now.checked_add(Duration::from_secs(delay).max(wait));
    }

    pub(crate) fn ready(&self, now: Instant) -> bool {
        !self.terminal && self.at.is_none_or(|at| at <= now)
    }
}

fn telegram_wait(error: &AppError) -> Option<Duration> {
    let AppError::Telegram {
        code: 420 | 429,
        message,
    } = error
    else {
        return None;
    };
    let seconds = message
        .strip_prefix("FLOOD_WAIT_")
        .or_else(|| message.strip_prefix("FLOOD_PREMIUM_WAIT_"))
        .or_else(|| {
            message
                .split_once("retry after ")
                .map(|(_, seconds)| seconds)
        })
        .and_then(|suffix| {
            suffix
                .trim_start()
                .split(|c: char| !c.is_ascii_digit())
                .next()
        })
        .and_then(|seconds| seconds.parse::<u32>().ok())
        .map(u64::from)
        .unwrap_or(15);
    Some(Duration::from_secs(seconds))
}

#[derive(Default)]
struct Demand {
    seen: bool,
    pending: bool,
}

impl Demand {
    fn observe(&mut self, signal: Browse) -> bool {
        self.seen = true;
        let previous = self.pending;
        match signal {
            Browse::Down => self.pending = true,
            Browse::Away => self.pending = false,
            Browse::Nearby => {}
        }
        self.pending && !previous
    }

    fn end_frame(&mut self) {
        if !std::mem::take(&mut self.seen) {
            self.pending = false;
        }
    }
}

pub(crate) type PlaylistKey = (i64, PlaylistOrder, [bool; 3]);

#[derive(Default)]
pub(crate) struct PageBrowse {
    queue_revision: u64,
    queue: [Demand; 2],
    playlist: Option<PlaylistKey>,
    tracks: Demand,
    tracks_retry: PageRetry,
    telegram_cooldown: Option<Instant>,
}

impl AppCore {
    pub(crate) fn record_page_cooldown(&mut self, error: &AppError, now: Instant) {
        let next = telegram_wait(error).and_then(|wait| now.checked_add(wait));
        self.page_browse.telegram_cooldown = self.page_browse.telegram_cooldown.max(next);
    }

    pub(crate) fn paging_allowed(&self, now: Instant) -> bool {
        self.page_browse
            .telegram_cooldown
            .is_none_or(|at| now >= at)
    }

    pub fn browse_queue(&mut self, signal: Browse) -> bool {
        self.browse_queue_on(false, signal)
    }

    pub fn browse_queue_on(&mut self, before: bool, signal: Browse) -> bool {
        let revision = self.playback.queue_start_generation();
        if self.page_browse.queue_revision != revision {
            self.page_browse.queue_revision = revision;
            self.page_browse.queue = Default::default();
        }
        self.page_browse.queue[usize::from(before)].observe(signal)
    }

    pub fn browse_playlist(
        &mut self,
        chat: i64,
        order: PlaylistOrder,
        filters: [bool; 3],
        signal: Browse,
    ) -> bool {
        let key = (chat, order, filters);
        if self.page_browse.playlist != Some(key) {
            self.page_browse.playlist = Some(key);
            self.page_browse.tracks = Demand::default();
            self.page_browse.tracks_retry = PageRetry::default();
        }
        self.page_browse.tracks.observe(signal)
    }

    pub fn finish_page_browse_frame(&mut self) {
        for demand in &mut self.page_browse.queue {
            demand.end_frame();
        }
        self.page_browse.tracks.end_frame();
    }

    pub(crate) fn queue_browse_pending_on(&self, before: bool) -> bool {
        self.page_browse.queue[usize::from(before)].pending
            && self.page_browse.queue_revision == self.playback.queue_start_generation()
    }

    pub(crate) fn complete_queue_browse(&mut self) {
        self.complete_queue_browse_on(false);
        self.complete_queue_browse_on(true);
    }

    pub(crate) fn complete_queue_browse_on(&mut self, before: bool) {
        self.page_browse.queue[usize::from(before)].pending = false;
    }

    pub fn queue_page_status_on(&self, before: bool) -> PageStatus {
        if self.queue_work.page_retry.terminal {
            PageStatus::Unavailable
        } else if self.queue_work.page_active_on(&self.state.session, before)
            || self.queue_browse_pending_on(before)
            || if before {
                self.queue_work.waiting_previous
                    || self.queue_work.prefetch_previous
                        && self.state.session.available_past_len() <= QUEUE_PREFETCH_ROWS
            } else {
                self.queue_work.waiting.is_some()
            }
        {
            PageStatus::Loading
        } else {
            PageStatus::Idle
        }
    }

    pub fn playlist_page_status(&self, chat: i64, order: PlaylistOrder) -> PageStatus {
        let matches = self
            .page_browse
            .playlist
            .is_some_and(|key| (key.0, key.1) == (chat, order));
        if matches && self.page_browse.tracks_retry.terminal {
            PageStatus::Unavailable
        } else if self.library.indexing_chat() == Some(chat)
            || self.oldest_loading(chat)
            || matches && self.page_browse.tracks.pending
        {
            PageStatus::Loading
        } else {
            PageStatus::Idle
        }
    }

    pub(crate) fn playlist_page_finished(
        &mut self,
        key: PlaylistKey,
        result: &Result<(), AppError>,
    ) {
        let now = Instant::now();
        if let Err(error) = result {
            self.record_page_cooldown(error, now);
        }
        if self.page_browse.playlist != Some(key) {
            return;
        }
        if let Err(error) = result {
            if *error != AppError::Cancelled {
                self.page_browse.tracks_retry.failed(error, now);
            }
        } else {
            self.page_browse.tracks.pending = false;
            self.page_browse.tracks_retry = PageRetry::default();
        }
    }

    pub(crate) fn playlist_browse_key(
        &self,
        chat: i64,
        order: PlaylistOrder,
    ) -> Option<PlaylistKey> {
        self.page_browse
            .playlist
            .filter(|key| (key.0, key.1) == (chat, order) && self.page_browse.tracks.pending)
    }

    pub(crate) fn pump_playlist_browse(&mut self, now: Instant, effects: &mut Vec<Effect>) {
        let Some((chat, order, filters)) = self.page_browse.playlist else {
            return;
        };
        if !self.page_browse.tracks.pending
            || !self.page_browse.tracks_retry.ready(now)
            || self.library.index.is_some()
            || self.library.oldest.is_some()
        {
            return;
        }
        if !self
            .state
            .playlists
            .iter()
            .any(|p| p.id == chat && p.filters == filters && p.has_more())
        {
            self.page_browse.tracks.pending = false;
            return;
        }
        match order {
            PlaylistOrder::NewestFirst => self.request_playlist_index(chat, false, effects),
            PlaylistOrder::OldestFirst => self.request_oldest(chat, true, effects),
        }
    }

    pub fn page_retry_deadline(&self, now: Instant, online: bool) -> Option<Instant> {
        if !online {
            return None;
        }
        let queue = self.queue_needs_retry();
        let playlist = self.page_browse.tracks.pending;
        let oldest = self.oldest_start_retry_deadline();
        if (queue || playlist || oldest.is_some()) && !self.paging_allowed(now) {
            return self.page_browse.telegram_cooldown;
        }
        [
            oldest,
            queue.then_some(self.queue_work.page_retry.at).flatten(),
            playlist
                .then_some(self.page_browse.tracks_retry.at)
                .flatten(),
        ]
        .into_iter()
        .flatten()
        .filter(|at| *at > now)
        .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paging_backoff_grows_and_honors_telegram_cooldown() {
        let now = Instant::now();
        let mut retry = PageRetry::default();
        for seconds in [1, 2, 4, 8, 15, 15] {
            retry.failed(&AppError::Timeout("Telegram"), now);
            assert_eq!(retry.at, Some(now + Duration::from_secs(seconds)));
        }
        let mut core = AppCore::new(Default::default());
        core.record_page_cooldown(
            &AppError::Telegram {
                code: 429,
                message: "Too Many Requests: retry after 90 seconds".into(),
            },
            now,
        );
        core.browse_playlist(10, PlaylistOrder::NewestFirst, [true; 3], Browse::Down);
        core.browse_playlist(20, PlaylistOrder::NewestFirst, [true; 3], Browse::Down);
        assert!(!core.paging_allowed(now + Duration::from_secs(89)));
        assert!(core.paging_allowed(now + Duration::from_secs(90)));
    }
}
