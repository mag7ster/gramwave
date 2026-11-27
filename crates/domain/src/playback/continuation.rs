use super::{PlaylistOrder, QueueEntry, QueueEntryId, QueueRangeEdit, Session, TrackMetadata};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Continuation {
    cursors: [Option<i64>; 3],
    ceiling: i64,
    selected: i64,
    pending: Vec<TrackMetadata>,
    pub(super) deferred: BTreeSet<QueueEntryId>,
}

impl Continuation {
    fn next_page(&self, order: PlaylistOrder) -> Option<(usize, i64)> {
        self.cursors
            .iter()
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

    fn frontier(&self, order: PlaylistOrder) -> i64 {
        self.next_page(order).map_or(i64::MAX, |(_, cursor)| {
            if cursor == 0 {
                i64::MIN
            } else if order == PlaylistOrder::OldestFirst {
                order.key(cursor).saturating_add(1)
            } else {
                order.key(cursor)
            }
        })
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
        let selected = tracks.get(index)?.message;
        let ceiling = if shuffle {
            tracks
                .iter()
                .map(|track| track.message)
                .min_by_key(|&id| order.key(id))
                .unwrap_or(selected)
        } else {
            selected
        };
        let mut continuation = Continuation {
            cursors,
            ceiling,
            selected,
            pending: Vec::new(),
            deferred: BTreeSet::new(),
        };
        let frontier = continuation.frontier(order);
        let mut chosen = 0;
        let mut known = Vec::with_capacity(tracks.len());
        for (at, track) in tracks.into_iter().enumerate() {
            let id = track.message;
            if at == index {
                chosen = known.len();
            }
            if id != selected && order.key(id) >= order.key(ceiling) && order.key(id) >= frontier {
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
        Some(current)
    }

    pub fn playlist_page_request(&self) -> Option<(i64, usize, i64)> {
        let (kind, cursor) = self.continuation.as_ref()?.next_page(self.playlist_order)?;
        Some((self.source?, kind, cursor))
    }

    pub fn available_len(&self) -> usize {
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
            if id != continuation.selected
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
        let before = self
            .order
            .iter()
            .position(|id| continuation.deferred.contains(id));
        let done = continuation.next_page(order).is_none();
        let at = self.available_len();
        let mut ids: Vec<_> = ready.into_iter().map(|track| self.entry(track)).collect();
        let before = before.unwrap_or(self.order.len());
        self.order.splice(before..before, ids.iter().copied());
        if shuffle {
            ids.shuffle(&mut rand::thread_rng());
        }
        self.remaining.splice(at..at, ids);
        if done {
            self.continuation = None;
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
