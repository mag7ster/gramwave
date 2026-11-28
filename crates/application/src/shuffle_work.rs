use crate::{
    commands::PlaylistStart,
    media::MediaTrack,
    model::Track,
    tasks::{Operation, RequestId},
    telegram::{ShuffleRanks, ShuffleSample},
    workflow::Effect,
    AppCore, AppError,
};
use gramwave_domain::PlaylistOrder;
use std::time::{Duration, Instant};

type Source = (i64, [bool; 3]);
type Range = (PlaylistOrder, i64);

struct PendingStart {
    request: RequestId,
    source: Source,
    order: PlaylistOrder,
    deadline: Instant,
}

#[derive(Default)]
pub(crate) struct ShuffleWork {
    requests: Vec<(Operation, Source, Option<Range>)>,
    cache: Vec<(Source, Option<Range>, ShuffleSample)>,
    open_chat: Option<i64>,
    start: Option<PendingStart>,
}

impl ShuffleWork {
    pub(crate) fn cancel(&mut self) {
        for (operation, ..) in self.requests.drain(..) {
            operation.cancel.cancel();
        }
        self.cache.clear();
        self.start = None;
    }
}

impl AppCore {
    fn shuffle_start_source(&self, chat: i64) -> Option<Source> {
        self.state
            .playlists
            .iter()
            .find(|playlist| playlist.id == chat && playlist.has_more())
            .map(|playlist| (chat, playlist.filters))
    }

    pub(crate) fn open_shuffle(&mut self, chat: i64, effects: &mut Vec<Effect>) {
        if !self.shuffle_start_loading(chat) {
            self.cancel_shuffle_start(None);
        }
        self.shuffle_work.open_chat = Some(chat);
        self.warm_shuffle(chat, effects);
    }

    fn warm_shuffle(&mut self, chat: i64, effects: &mut Vec<Effect>) {
        if let Some(source) = self.shuffle_start_source(chat) {
            if !self.shuffle_work.cache.iter().any(|(key, range, sample)| {
                *key == source && range.is_none() && !sample.tracks.is_empty()
            }) {
                self.request_shuffle(source, None, effects);
            }
        }
    }

    pub fn shuffle_start_loading(&self, chat: i64) -> bool {
        self.shuffle_work
            .start
            .as_ref()
            .is_some_and(|start| start.source.0 == chat)
    }

    pub(crate) fn cancel_shuffle_start(&mut self, chat: Option<i64>) {
        if chat.is_none_or(|chat| self.shuffle_start_loading(chat)) {
            self.shuffle_work.start = None;
        }
    }

    pub fn shuffle_start_deadline(&self) -> Option<Instant> {
        self.shuffle_work.start.as_ref().map(|start| start.deadline)
    }

    pub(crate) fn start_shuffled_playlist(
        &mut self,
        chat: i64,
        order: PlaylistOrder,
        effects: &mut Vec<Effect>,
    ) -> Option<Track> {
        let source = self.shuffle_start_source(chat);
        if self
            .shuffle_work
            .start
            .as_ref()
            .is_some_and(|start| Some(start.source) == source && start.order == order)
        {
            return None;
        }
        self.cancel_shuffle_start(None);
        if let Some(source) = source {
            if let Some((_, _, sample)) =
                self.shuffle_work
                    .cache
                    .iter_mut()
                    .find(|(key, range, sample)| {
                        *key == source && range.is_none() && !sample.tracks.is_empty()
                    })
            {
                let tracks = std::mem::take(&mut sample.tracks);
                let current = self.finish_shuffle_start(chat, order, tracks);
                self.replenish_shuffle(chat, effects);
                return current;
            }
            if let Some(request) = self.request_shuffle(source, None, effects) {
                self.shuffle_work.start = Some(PendingStart {
                    request,
                    source,
                    order,
                    deadline: Instant::now() + Duration::from_secs(5),
                });
                return None;
            }
        }
        self.finish_shuffle_start(chat, order, Vec::new())
    }

    fn finish_shuffle_start(
        &mut self,
        chat: i64,
        order: PlaylistOrder,
        mut tracks: Vec<MediaTrack>,
    ) -> Option<Track> {
        let source = self.shuffle_start_source(chat);
        tracks.retain(|track| {
            track.chat == chat
                && source.is_some_and(|(_, filters)| filters[usize::from(track.kind)])
                && !self.library.source_changed(track.id())
        });
        let mut tracks = tracks.into_iter();
        let first = tracks.next().map(|track| {
            self.library.cache_track(&track);
            Track::from(track)
        });
        let current = self.start_playlist_queue(chat, PlaylistStart::Shuffled, order, first)?;
        self.apply_shuffle_sample(tracks);
        Some(current)
    }

    pub(crate) fn replenish_shuffle(&mut self, chat: i64, effects: &mut Vec<Effect>) {
        if self.shuffle_work.open_chat == Some(chat) {
            self.warm_shuffle(chat, effects);
        }
    }

    pub(crate) fn expire_shuffle_start(&mut self, now: Instant, effects: &mut Vec<Effect>) {
        if let Some(start) = self
            .shuffle_work
            .start
            .take_if(|start| start.deadline <= now)
        {
            if let Some(track) = self.finish_shuffle_start(start.source.0, start.order, Vec::new())
            {
                self.prepare_playback(track, effects);
                self.persistence_effect(effects);
            }
        }
    }

    fn request_shuffle(
        &mut self,
        source: Source,
        range: Option<Range>,
        effects: &mut Vec<Effect>,
    ) -> Option<RequestId> {
        if !self.library.connected() {
            return None;
        }
        let work = &mut self.shuffle_work;
        if let Some((operation, ..)) = work
            .requests
            .iter()
            .find(|(_, key, bounds)| *key == source && (*bounds == range || bounds.is_none()))
        {
            return Some(operation.key());
        }
        if work.requests.len() == 2 {
            let index = work.requests.iter().position(|(op, ..)| {
                Some(op.key()) != self.queue_work.shuffle_request
                    && work
                        .start
                        .as_ref()
                        .is_none_or(|start| start.request != op.key())
            })?;
            work.requests.remove(index).0.cancel.cancel();
        }
        let ranks: Option<ShuffleRanks> = work
            .cache
            .iter()
            .find(|(key, _, sample)| {
                *key == source
                    && sample
                        .ranks
                        .iter()
                        .any(|(_, positions)| !positions.is_empty())
            })
            .map(|(_, _, sample)| sample.ranks.clone());
        let operation = self.library.operation();
        effects.push(Effect::ShuffleSample {
            operation: operation.clone(),
            chat: source.0,
            media: source.1,
            ranks,
            range,
        });
        let id = operation.key();
        work.requests.push((operation, source, range));
        Some(id)
    }

    pub(crate) fn sample_active_queue(&mut self, effects: &mut Vec<Effect>) {
        let Some((chat, media, ceiling)) = self.state.session.shuffle_source() else {
            if self.state.session.source.is_some() {
                self.state.session.spread_shuffled_queue();
                self.record_session_change();
            }
            return;
        };
        let order = self.state.session.playlist_source_order();
        let range = match (order, ceiling) {
            (PlaylistOrder::NewestFirst, i64::MAX) | (PlaylistOrder::OldestFirst, i64::MIN) => None,
            _ => Some((order, ceiling)),
        };
        let source = (chat, media);
        if let Some((_, _, cached)) =
            self.shuffle_work
                .cache
                .iter_mut()
                .find(|(key, bounds, sample)| {
                    *key == source
                        && !sample.tracks.is_empty()
                        && (*bounds == range
                            || bounds.is_none()
                                && sample.tracks.iter().all(|track| {
                                    range.is_none_or(|(order, ceiling)| {
                                        order.key(track.message) >= order.key(ceiling)
                                    })
                                }))
                })
        {
            let tracks = std::mem::take(&mut cached.tracks);
            self.apply_shuffle_sample(tracks);
            self.replenish_shuffle(chat, effects);
        } else {
            self.queue_work.shuffle_request = self.request_shuffle(source, range, effects);
        }
    }

    pub(crate) fn receive_shuffle_sample(
        &mut self,
        request: RequestId,
        result: Result<ShuffleSample, AppError>,
        effects: &mut Vec<Effect>,
    ) {
        let Some(index) = self
            .shuffle_work
            .requests
            .iter()
            .position(|(op, ..)| op.key() == request)
        else {
            return;
        };
        let (_, source, range) = self.shuffle_work.requests.remove(index);
        let active = self.queue_work.shuffle_request == Some(request);
        if active {
            self.queue_work.shuffle_request = None;
        }
        self.expire_shuffle_start(Instant::now(), effects);
        let start = self
            .shuffle_work
            .start
            .take_if(|start| start.request == request);
        let mut tracks = Vec::new();
        if let Ok(mut sample) = result {
            if start.is_some() || active && self.state.shuffle {
                tracks = std::mem::take(&mut sample.tracks);
            }
            self.shuffle_work
                .cache
                .retain(|(key, bounds, _)| (*key, *bounds) != (source, range));
            if self.shuffle_work.cache.len() == 2 {
                self.shuffle_work.cache.remove(0);
            }
            self.shuffle_work.cache.push((source, range, sample));
        }
        let consumed = !tracks.is_empty();
        if let Some(start) = start {
            if let Some(track) = self.finish_shuffle_start(source.0, start.order, tracks) {
                self.prepare_playback(track, effects);
            }
        } else if active {
            self.apply_shuffle_sample(tracks);
        }
        if consumed {
            self.replenish_shuffle(source.0, effects);
        }
    }

    fn apply_shuffle_sample(&mut self, tracks: impl IntoIterator<Item = MediaTrack>) {
        let tracks = tracks.into_iter().filter_map(|track| {
            if self.library.source_changed(track.id()) {
                return None;
            }
            self.library.cache_track(&track);
            Some(crate::model::Track::from(track).into())
        });
        if self.state.session.add_shuffle_sample(tracks, false) {
            self.playback.shuffle_sample_generation =
                self.playback.shuffle_sample_generation.wrapping_add(1);
            self.playback.repeat_cycle = None;
            self.record_session_change();
        }
    }
}
