use super::{PlaylistOrder, QueueEntry, QueueEntryId, QueueRangeEdit, Session, TrackMetadata};
use rand::{seq::SliceRandom, Rng};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Continuation {
    cursors: [Option<i64>; 3],
    ceiling: i64,
    selected: i64,
    pending: Vec<TrackMetadata>,
    pub(super) deferred: BTreeSet<QueueEntryId>,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "stored_samples"
    )]
    // Associate early picks with their source rows until normal pagination catches up.
    pub(super) samples: BTreeMap<i64, Sample>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ordered_window: Option<Box<OrderedWindow>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Sample {
    id: QueueEntryId,
    pub(super) held: bool,
    #[serde(default)]
    fixed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct OrderedWindow {
    anchor: i64,
    before: [Option<i64>; 3],
    after: [Option<i64>; 3],
}

fn next_cursor(cursors: [Option<i64>; 3], order: PlaylistOrder) -> Option<(usize, i64)> {
    cursors
        .into_iter()
        .enumerate()
        .filter_map(|(kind, cursor)| cursor.map(|cursor| (kind, cursor)))
        .min_by_key(|&(_, cursor)| {
            if cursor == 0 {
                i64::MIN
            } else {
                order.key(cursor)
            }
        })
}

fn frontier(cursors: [Option<i64>; 3], order: PlaylistOrder) -> i64 {
    next_cursor(cursors, order).map_or(i64::MAX, |(_, cursor)| {
        if cursor == 0 {
            i64::MIN
        } else if order == PlaylistOrder::OldestFirst {
            order.key(cursor).saturating_add(1)
        } else {
            order.key(cursor)
        }
    })
}

impl OrderedWindow {
    fn cursors(&self, before: bool) -> [Option<i64>; 3] {
        if before {
            self.before
        } else {
            self.after
        }
    }
}

impl Continuation {
    fn next_page(&self, order: PlaylistOrder) -> Option<(usize, i64)> {
        next_cursor(self.cursors, order)
    }

    fn frontier(&self, order: PlaylistOrder) -> i64 {
        frontier(self.cursors, order)
    }
}

impl Session {
    pub fn playlist_order(&self) -> PlaylistOrder {
        self.playlist_order
    }

    pub fn start_playlist(
        &mut self,
        tracks: Vec<TrackMetadata>,
        index: usize,
        cursors: [Option<i64>; 3],
        shuffle: bool,
        order: PlaylistOrder,
    ) -> Option<QueueEntry> {
        self.start_playlist_inner(tracks, index, cursors, shuffle, order)
    }

    fn start_playlist_inner(
        &mut self,
        tracks: Vec<TrackMetadata>,
        index: usize,
        cursors: [Option<i64>; 3],
        shuffle: bool,
        order: PlaylistOrder,
    ) -> Option<QueueEntry> {
        let selected = tracks.get(index)?.message;
        let ceiling = match order {
            PlaylistOrder::NewestFirst => i64::MAX,
            PlaylistOrder::OldestFirst => i64::MIN,
        };
        let mut continuation = Continuation {
            cursors,
            ceiling,
            selected,
            pending: Vec::new(),
            deferred: BTreeSet::new(),
            samples: BTreeMap::new(),
            ordered_window: None,
        };
        let frontier = continuation.frontier(order);
        let mut chosen = 0;
        let mut known = Vec::with_capacity(tracks.len());
        for (at, track) in tracks.into_iter().enumerate() {
            let id = track.message;
            if at == index {
                chosen = known.len();
            }
            if id != selected && order.key(id) >= frontier {
                continuation.pending.push(track);
            } else {
                known.push(track);
            }
        }
        let current = self.start(&known, chosen, shuffle)?;
        self.playlist_order = order;
        self.continuation = continuation
            .next_page(order)
            .is_some()
            .then_some(continuation);
        if let Some(continuation) = &mut self.continuation {
            if order.key(selected) >= frontier {
                continuation.samples.insert(
                    selected,
                    Sample {
                        id: current.entry_id,
                        held: false,
                        fixed: false,
                    },
                );
                if !shuffle {
                    self.open_ordered_window();
                    self.rebuild_ordered();
                }
            }
        }
        Some(current)
    }

    pub fn start_shuffled_playlist(
        &mut self,
        tracks: Vec<TrackMetadata>,
        cursors: [Option<i64>; 3],
        order: PlaylistOrder,
    ) -> Option<QueueEntry> {
        if tracks.is_empty() {
            return None;
        }
        let mut rng = rand::thread_rng();
        let frontier = frontier(cursors, order);
        let ready = tracks.partition_point(|track| order.key(track.message) < frontier);
        let count = if ready == 0 { tracks.len() } else { ready };
        let index = rng.gen_range(0..count);
        let current = self.start_playlist_inner(tracks, index, cursors, true, order)?;
        if self.continuation.is_none() {
            self.spread_start();
        }
        Some(self.current().cloned().unwrap_or(current))
    }

    pub fn shuffle_source(&self) -> Option<(i64, [bool; 3], i64)> {
        let continuation = self.continuation.as_ref()?;
        Some((
            self.source?,
            continuation.cursors.map(|cursor| cursor.is_some()),
            continuation.ceiling,
        ))
    }

    pub fn add_shuffle_sample(
        &mut self,
        tracks: impl IntoIterator<Item = TrackMetadata>,
        starting: bool,
    ) {
        let Some(continuation) = &self.continuation else {
            return;
        };
        let order = self.playlist_order;
        let frontier = continuation.frontier(order);
        let ceiling = order.key(continuation.ceiling);
        let Some(track_chat) = self.source else {
            return;
        };
        let known: BTreeMap<_, _> = self
            .order
            .iter()
            .filter_map(|id| {
                let entry = &self.entries[id];
                (entry.chat == track_chat
                    && !self.manual.contains(id)
                    && !continuation.deferred.contains(id))
                .then_some((entry.message, *id))
            })
            .collect();
        let upcoming: HashSet<_> = self.remaining.iter().copied().collect();
        let mut picked = HashSet::new();
        let mut ids = Vec::new();
        for track in tracks {
            let message = track.message;
            if track.chat != track_chat || order.key(message) < ceiling || !picked.insert(message) {
                continue;
            }
            if let Some(&id) = known.get(&message) {
                if upcoming.contains(&id) || starting && self.current == Some(id) {
                    ids.push(id);
                }
                continue;
            }
            if order.key(message) < frontier
                || self
                    .continuation
                    .as_ref()
                    .unwrap()
                    .samples
                    .contains_key(&message)
            {
                continue;
            }
            let id = self.entry(track);
            let continuation = self.continuation.as_mut().unwrap();
            let before = self.order.iter().position(|old| {
                continuation.deferred.contains(old)
                    || continuation
                        .samples
                        .get(&self.entries[old].message)
                        .is_some_and(|sample| {
                            sample.id == *old
                                && !sample.fixed
                                && order.key(self.entries[old].message) > order.key(message)
                        })
            });
            self.order.insert(before.unwrap_or(self.order.len()), id);
            continuation.samples.insert(
                message,
                Sample {
                    id,
                    held: false,
                    fixed: false,
                },
            );
            ids.push(id);
        }
        if ids.is_empty() {
            return;
        }
        let selected: HashSet<_> = ids.iter().copied().collect();
        self.remaining.retain(|id| !selected.contains(id));
        if starting {
            let next = ids.remove(0);
            if let Some(old) = self
                .current
                .replace(next)
                .filter(|old| !selected.contains(old))
            {
                self.remaining.insert(0, old);
            }
        }
        self.remaining.splice(..0, ids);
    }

    pub(super) fn show_shuffle_samples(&mut self, shuffle: bool) {
        let Some(continuation) = &mut self.continuation else {
            return;
        };
        let mut shown = Vec::new();
        for sample in continuation.samples.values_mut() {
            if shuffle {
                if sample.held && self.entries.contains_key(&sample.id) {
                    shown.push(sample.id);
                }
                sample.held = false;
            } else if let Some(index) = self
                .remaining
                .iter()
                .position(|id| !sample.fixed && *id == sample.id)
            {
                self.remaining.remove(index);
                sample.held = true;
            }
        }
        self.remaining.splice(..0, shown);
    }

    fn spread_start(&mut self) {
        let mut rng = rand::thread_rng();
        let mut first = spread(&self.order, &mut rng);
        self.remaining.retain(|id| !first.contains(id));
        if let Some(old) = self.current.filter(|old| !first.contains(old)) {
            self.remaining.push(old);
            self.remaining.shuffle(&mut rng);
        }
        self.current = Some(first.remove(0));
        self.remaining.splice(..0, first);
    }

    pub fn spread_shuffled_queue(&mut self) {
        let upcoming: HashSet<_> = self.remaining.iter().copied().collect();
        let ordered: Vec<_> = self
            .order
            .iter()
            .copied()
            .filter(|id| upcoming.contains(id))
            .collect();
        let first = spread(&ordered, &mut rand::thread_rng());
        for id in &first {
            if let Some(index) = self.remaining.iter().position(|upcoming| upcoming == id) {
                self.remaining.remove(index);
            }
        }
        self.remaining.splice(..0, first);
    }

    pub(super) fn fix_shuffle_sample_positions(
        &mut self,
        moved: impl IntoIterator<Item = QueueEntryId>,
    ) {
        if let Some(continuation) = &mut self.continuation {
            let moved: HashSet<_> = moved.into_iter().collect();
            for sample in continuation.samples.values_mut() {
                sample.fixed |= moved.contains(&sample.id);
            }
        }
    }

    pub fn full_playlist_page_request(&self) -> Option<(i64, usize, i64)> {
        let (kind, cursor) = self.continuation.as_ref()?.next_page(self.playlist_order)?;
        Some((self.source?, kind, cursor))
    }

    pub fn playlist_page_request(&self) -> Option<(i64, usize, i64)> {
        if self.has_ordered_window() {
            self.ordered_page_request(false)
        } else {
            self.full_playlist_page_request()
        }
    }

    pub fn has_ordered_window(&self) -> bool {
        self.continuation
            .as_ref()
            .is_some_and(|c| c.ordered_window.is_some())
    }

    pub(super) fn open_ordered_window(&mut self) {
        let Some(continuation) = &mut self.continuation else {
            return;
        };
        let order = self.playlist_order;
        let anchor = self.current.and_then(|current| {
            if self.manual.contains(&current) || continuation.deferred.contains(&current) {
                self.order
                    .iter()
                    .position(|id| *id == current)
                    .and_then(|at| {
                        self.order[..at]
                            .iter()
                            .rev()
                            .find(|id| {
                                !self.manual.contains(id) && !continuation.deferred.contains(id)
                            })
                            .map(|id| self.entries[id].message)
                    })
            } else {
                Some(self.entries[&current].message)
            }
        });
        if let Some(anchor) = anchor.filter(|id| order.key(*id) >= continuation.frontier(order)) {
            let cursors = continuation.cursors.map(|cursor| cursor.map(|_| anchor));
            continuation.ordered_window = Some(Box::new(OrderedWindow {
                anchor,
                before: cursors,
                after: cursors,
            }));
        }
        for sample in continuation.samples.values_mut() {
            sample.held = !sample.fixed && Some(sample.id) != self.current;
        }
    }

    pub fn leave_ordered_window(&mut self) {
        if let Some(continuation) = &mut self.continuation {
            continuation.ordered_window = None;
        }
    }

    pub(super) fn ordered_visible(&self, id: QueueEntryId) -> bool {
        let Some(continuation) = &self.continuation else {
            return true;
        };
        let entry = &self.entries[&id];
        let sample = continuation
            .samples
            .get(&entry.message)
            .filter(|sample| sample.id == id);
        if self.manual.contains(&id)
            || continuation.deferred.contains(&id)
            || sample.is_some_and(|sample| sample.fixed)
        {
            return true;
        }
        let order = self.playlist_order;
        if let Some(window) = &continuation.ordered_window {
            return entry.message == window.anchor
                || order.key(entry.message) < frontier(window.after, order)
                    && order.reversed().key(entry.message)
                        < frontier(window.before, order.reversed());
        }
        !sample.is_some_and(|sample| sample.held)
    }

    pub fn ordered_page_request(&self, before: bool) -> Option<(i64, usize, i64)> {
        let window = self.continuation.as_ref()?.ordered_window.as_ref()?;
        let order = if before {
            self.playlist_order.reversed()
        } else {
            self.playlist_order
        };
        let (kind, cursor) = next_cursor(window.cursors(before), order)?;
        Some((self.source?, kind, cursor))
    }

    pub fn needs_ordered_neighbors(&self, before: bool) -> bool {
        self.continuation
            .as_ref()
            .and_then(|c| c.ordered_window.as_ref())
            .is_some_and(|window| window.cursors(before).contains(&Some(window.anchor)))
    }

    pub fn add_ordered_page(
        &mut self,
        before: bool,
        kind: usize,
        next: i64,
        tracks: impl IntoIterator<Item = TrackMetadata>,
    ) {
        let Some(continuation) = &mut self.continuation else {
            return;
        };
        let Some(window) = &mut continuation.ordered_window else {
            return;
        };
        let cursors = if before {
            &mut window.before
        } else {
            &mut window.after
        };
        cursors[kind] = (next != 0).then_some(next);
        let order = self.playlist_order;
        let scanned = continuation.frontier(order);
        let ceiling = order.key(continuation.ceiling);
        let known: HashSet<_> = self
            .order
            .iter()
            .filter(|id| !self.manual.contains(id) && !continuation.deferred.contains(id))
            .map(|id| self.entries[id].message)
            .chain(continuation.samples.keys().copied())
            .collect();
        let mut known = known;
        for track in tracks {
            let message = track.message;
            if self.source != Some(track.chat)
                || order.key(message) < scanned
                || order.key(message) < ceiling
                || !known.insert(message)
            {
                continue;
            }
            let before = self
                .order
                .iter()
                .position(|id| {
                    let continuation = self.continuation.as_ref().unwrap();
                    let entry = &self.entries[id];
                    continuation.deferred.contains(id)
                        || !self.manual.contains(id)
                            && !continuation
                                .samples
                                .get(&entry.message)
                                .is_some_and(|sample| sample.id == *id && sample.fixed)
                            && order.key(entry.message) > order.key(message)
                })
                .unwrap_or(self.order.len());
            let id = self.entry(track);
            self.order.insert(before, id);
            self.continuation.as_mut().unwrap().samples.insert(
                message,
                Sample {
                    id,
                    held: true,
                    fixed: false,
                },
            );
        }
        self.history.clear();
        self.rebuild_ordered();
    }

    pub fn available_len(&self) -> usize {
        if self.has_ordered_window() && self.ordered_page_request(false).is_none() {
            return self.remaining.len();
        }
        self.continuation
            .as_ref()
            .map_or(self.remaining.len(), |continuation| {
                self.remaining
                    .iter()
                    .rposition(|id| !continuation.deferred.contains(id))
                    .map_or(0, |index| index + 1)
            })
    }

    pub fn add_playlist_page(
        &mut self,
        kind: usize,
        next: i64,
        tracks: impl IntoIterator<Item = TrackMetadata>,
        shuffle: bool,
    ) {
        let Some(continuation) = &mut self.continuation else {
            return;
        };
        let order = self.playlist_order;
        let frontier = continuation.frontier(order);
        let mut pending: HashSet<_> = continuation
            .pending
            .iter()
            .map(|track| track.message)
            .collect();
        for track in tracks {
            let id = track.message;
            if (id != continuation.selected || continuation.samples.contains_key(&id))
                && order.key(id) >= order.key(continuation.ceiling)
                && order.key(id) >= frontier
                && pending.insert(id)
            {
                continuation.pending.push(track);
            }
        }
        continuation.cursors[kind] = (next != 0).then_some(next);
        // A page from one media filter can run ahead of the other filters.
        let frontier = continuation.frontier(order);
        continuation
            .pending
            .sort_by_key(|track| order.key(track.message));
        let ready = continuation
            .pending
            .partition_point(|track| order.key(track.message) < frontier);
        let ready: Vec<_> = continuation.pending.drain(..ready).collect();
        let done = continuation.next_page(order).is_none();
        let project = continuation.ordered_window.is_none();
        let mut samples = std::mem::take(&mut continuation.samples);
        let at = self.available_len();
        let mut ids = Vec::with_capacity(ready.len());
        if samples.is_empty() {
            let deferred = &self.continuation.as_ref().unwrap().deferred;
            let before = self
                .order
                .iter()
                .position(|id| deferred.contains(id))
                .unwrap_or(self.order.len());
            ids.extend(ready.into_iter().map(|track| self.entry(track)));
            self.order.splice(before..before, ids.iter().copied());
        } else {
            for track in ready {
                let message = track.message;
                if let Some(sample) = samples.remove(&message) {
                    if sample.held && self.entries.contains_key(&sample.id) {
                        ids.push(sample.id);
                    }
                } else {
                    let next_sample = self.order.iter().position(|id| {
                        let message_at = self.entries[id].message;
                        samples
                            .get(&message_at)
                            .is_some_and(|sample| sample.id == *id && !sample.fixed)
                            && order.key(message_at) > order.key(message)
                    });
                    let before = next_sample.or_else(|| {
                        let deferred = &self.continuation.as_ref().unwrap().deferred;
                        self.order.iter().position(|id| deferred.contains(id))
                    });
                    let id = self.entry(track);
                    self.order.insert(before.unwrap_or(self.order.len()), id);
                    ids.push(id);
                }
            }
        }
        self.continuation.as_mut().unwrap().samples = samples;
        if shuffle {
            ids.shuffle(&mut rand::thread_rng());
        }
        if project {
            self.remaining.splice(at..at, ids);
        }
        if done {
            let continuation = self.continuation.take().unwrap();
            for sample in continuation
                .samples
                .into_values()
                .filter(|sample| sample.held)
            {
                self.order.retain(|id| *id != sample.id);
            }
            self.collect();
        }
    }

    pub fn discard_playlist_track(&mut self, chat: i64, message: i64) -> bool {
        self.source == Some(chat)
            && self.continuation.as_mut().is_some_and(|continuation| {
                let before = continuation.pending.len();
                continuation
                    .pending
                    .retain(|track| track.message != message);
                before != continuation.pending.len()
            })
    }

    pub fn deferred(&self, index: usize) -> bool {
        index >= self.available_len() && index < self.remaining.len()
    }

    pub fn play_deferred(&mut self, index: usize) -> Option<QueueEntry> {
        if !self.deferred(index) {
            return None;
        }
        let id = self.remaining.remove(index);
        self.continuation.as_mut()?.deferred.remove(&id);
        self.remaining.insert(0, id);
        self.jump_to(0)
    }

    pub fn finish_ordered_reverse(&mut self) {
        if self.continuation.is_none() {
            self.history.clear();
            self.rebuild_ordered();
        }
    }

    pub fn reverse_queue(&mut self) -> bool {
        if self.continuation.is_some() {
            return false;
        }
        let timeline = self.timeline();
        let ids: Vec<_> = (0..timeline.len())
            .map(|index| timeline.get(index).unwrap().entry_id)
            .collect();
        if self.edit_timeline_range(0, &ids, QueueRangeEdit::Reverse) {
            self.playlist_order = self.playlist_order.reversed();
            true
        } else {
            false
        }
    }

    pub(super) fn restore_boundary(&mut self) {
        if let Some(continuation) = &self.continuation {
            self.remaining
                .sort_by_key(|id| continuation.deferred.contains(id));
        }
    }

    pub(super) fn set_boundary(&mut self, at: usize) {
        if let Some(continuation) = &mut self.continuation {
            for (index, &id) in self.remaining.iter().enumerate() {
                if index < at {
                    continuation.deferred.remove(&id);
                } else {
                    continuation.deferred.insert(id);
                }
            }
        }
    }
}

fn spread(ids: &[QueueEntryId], rng: &mut impl Rng) -> Vec<QueueEntryId> {
    let bands = ids.len().min(4);
    let mut first: Vec<_> = (0..bands)
        .map(|i| {
            let start = i * ids.len() / bands;
            let end = (i + 1) * ids.len() / bands;
            ids[rng.gen_range(start..end)]
        })
        .collect();
    first.shuffle(rng);
    first
}

mod stored_samples {
    use super::{BTreeMap, Sample};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        samples: &BTreeMap<i64, Sample>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(samples)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<i64, Sample>, D::Error> {
        Ok(Vec::<(i64, Sample)>::deserialize(deserializer)?
            .into_iter()
            .collect())
    }
}
