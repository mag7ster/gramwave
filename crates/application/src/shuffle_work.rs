use crate::{
    tasks::{Operation, RequestId},
    telegram::{ShuffleRanks, ShuffleSample},
    workflow::Effect,
    AppCore, AppError,
};
use gramwave_domain::PlaylistOrder;

type Source = (i64, [bool; 3]);
type Range = (PlaylistOrder, i64);

#[derive(Default)]
pub(crate) struct ShuffleWork {
    requests: Vec<(Operation, Source, Option<Range>)>,
    cache: Vec<(Source, Option<Range>, ShuffleSample)>,
}

impl ShuffleWork {
    pub(crate) fn cancel(&mut self) {
        for (operation, ..) in self.requests.drain(..) {
            operation.cancel.cancel();
        }
        self.cache.clear();
    }
}

impl AppCore {
    pub(crate) fn warm_shuffle(&mut self, chat: i64, effects: &mut Vec<Effect>) {
        let Some(playlist) = self
            .state
            .playlists
            .iter()
            .find(|playlist| playlist.id == chat)
        else {
            return;
        };
        let media = std::array::from_fn(|kind| playlist.filters[kind] && !playlist.done[kind]);
        let source = (chat, media);
        if media.contains(&true)
            && !self
                .shuffle_work
                .cache
                .iter()
                .any(|(key, ..)| *key == source)
        {
            self.request_shuffle(source, None, effects);
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
            let index = work
                .requests
                .iter()
                .position(|(op, ..)| Some(op.key()) != self.queue_work.shuffle_request)?;
            work.requests.remove(index).0.cancel.cancel();
        }
        let ranks: Option<ShuffleRanks> = work
            .cache
            .iter()
            .find(|(key, ..)| *key == source)
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

    pub(crate) fn sample_active_queue(&mut self, starting: bool, effects: &mut Vec<Effect>) {
        let Some((chat, media, ceiling)) = self.state.session.shuffle_source() else {
            if !starting && self.state.session.source.is_some() {
                self.state.session.spread_shuffled_queue();
                self.record_session_change();
            }
            return;
        };
        let order = self.state.session.playlist_order();
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
            self.apply_shuffle_sample(tracks, starting);
        } else {
            self.queue_work.shuffle_request = self.request_shuffle(source, range, effects);
        }
    }

    pub(crate) fn receive_shuffle_sample(
        &mut self,
        request: RequestId,
        result: Result<ShuffleSample, AppError>,
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
        if let Ok(mut sample) = result {
            if active && self.state.shuffle {
                self.apply_shuffle_sample(std::mem::take(&mut sample.tracks), false);
            }
            self.shuffle_work
                .cache
                .retain(|(key, bounds, _)| (*key, *bounds) != (source, range));
            if self.shuffle_work.cache.len() == 2 {
                self.shuffle_work.cache.remove(0);
            }
            self.shuffle_work.cache.push((source, range, sample));
        }
    }

    fn apply_shuffle_sample(&mut self, tracks: Vec<crate::media::MediaTrack>, starting: bool) {
        let tracks = tracks.into_iter().filter_map(|track| {
            if self.library.source_changed(track.id()) {
                return None;
            }
            self.library.cache_track(&track);
            Some(crate::model::Track::from(track).into())
        });
        self.state.session.add_shuffle_sample(tracks, starting);
        self.playback.repeat_cycle = None;
        self.record_session_change();
    }
}
