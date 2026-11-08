//! Queue occurrences live once. Navigation and original order reference stable occurrence IDs.
use crate::model::{QueueEntry, QueueEntryId, TrackMetadata};
use rand::{seq::SliceRandom, Rng};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}
impl RepeatMode {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}
pub fn same(a: &TrackMetadata, b: &TrackMetadata) -> bool {
    a.id() == b.id()
}
#[derive(Clone, Copy)]
pub struct QueueView<'a> {
    ids: &'a [QueueEntryId],
    entries: &'a BTreeMap<QueueEntryId, QueueEntry>,
}
impl<'a> QueueView<'a> {
    pub fn len(self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(self) -> bool {
        self.ids.is_empty()
    }
    pub fn iter(
        self,
    ) -> impl ExactSizeIterator<Item = &'a QueueEntry> + DoubleEndedIterator + Clone {
        self.ids.iter().map(|id| &self.entries[id])
    }
    pub fn first(self) -> Option<&'a QueueEntry> {
        self.get(0)
    }
    pub fn get(self, i: usize) -> Option<&'a QueueEntry> {
        self.ids.get(i).map(|id| &self.entries[id])
    }
    pub fn to_vec(self) -> Vec<QueueEntry> {
        self.iter().cloned().collect()
    }
}
impl std::ops::Index<usize> for QueueView<'_> {
    type Output = QueueEntry;
    fn index(&self, i: usize) -> &QueueEntry {
        &self.entries[&self.ids[i]]
    }
}
impl std::fmt::Debug for QueueView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}
impl PartialEq<Vec<QueueEntry>> for QueueView<'_> {
    fn eq(&self, other: &Vec<QueueEntry>) -> bool {
        self.iter().eq(other)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Session {
    #[serde(serialize_with = "serialize_entries")]
    entries: BTreeMap<QueueEntryId, QueueEntry>,
    order: Vec<QueueEntryId>,
    remaining: Vec<QueueEntryId>,
    current: Option<QueueEntryId>,
    history: Vec<QueueEntryId>,
    preceding: Vec<QueueEntryId>,
    pub source: Option<i64>,
    serial: u64,
}
impl Session {
    fn view<'a>(&'a self, ids: &'a [QueueEntryId]) -> QueueView<'a> {
        QueueView {
            ids,
            entries: &self.entries,
        }
    }
    pub fn snapshot(&self) -> QueueView<'_> {
        self.view(&self.order)
    }
    pub fn remaining(&self) -> QueueView<'_> {
        self.view(&self.remaining)
    }
    pub fn history(&self) -> QueueView<'_> {
        self.view(&self.history)
    }
    pub fn preceding(&self) -> QueueView<'_> {
        self.view(&self.preceding)
    }
    pub fn current(&self) -> Option<&QueueEntry> {
        self.current.map(|id| &self.entries[&id])
    }
    pub fn stop(&mut self) {
        self.current = None;
        self.collect();
    }
    pub fn set_current(&mut self, track: TrackMetadata, entry_id: QueueEntryId) {
        if self.entries.contains_key(&entry_id) {
            self.current = Some(entry_id);
        } else {
            let id = self.entry(track);
            self.order.push(id);
            self.current = Some(id);
        }
    }
    fn entry(&mut self, track: TrackMetadata) -> QueueEntryId {
        self.serial = self
            .serial
            .checked_add(1)
            .expect("queue identity exhausted");
        let id = QueueEntryId(self.serial);
        self.entries.insert(
            id,
            QueueEntry {
                entry_id: id,
                track,
            },
        );
        id
    }
    pub fn start<T: Clone + Into<TrackMetadata>>(
        &mut self,
        tracks: &[T],
        index: usize,
        shuffle: bool,
    ) -> Option<QueueEntry> {
        tracks.get(index)?;
        self.entries.clear();
        self.order = tracks
            .iter()
            .cloned()
            .map(|t| self.entry(t.into()))
            .collect();
        self.current = Some(self.order[index]);
        self.remaining = if shuffle {
            self.order
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .map(|(_, id)| *id)
                .collect()
        } else {
            self.order[index + 1..].to_vec()
        };
        self.history.clear();
        self.preceding = if shuffle {
            vec![]
        } else {
            self.order[..index].to_vec()
        };
        self.source = tracks.first().cloned().map(|t| t.into().chat);
        self.reorder(shuffle);
        self.current().cloned()
    }
    pub fn import<T: Into<TrackMetadata>>(&mut self, manual: Vec<T>) {
        let tracks: Vec<TrackMetadata> = manual.into_iter().map(Into::into).collect();
        for track in tracks.into_iter().rev() {
            self.insert(track, true);
        }
    }

    pub fn insert(&mut self, track: impl Into<TrackMetadata>, next: bool) {
        let id = self.entry(track.into());
        let before = next.then(|| self.remaining.first().copied()).flatten();
        let at = before
            .and_then(|id| self.order.iter().position(|old| *old == id))
            .unwrap_or(self.order.len());
        self.order.insert(at, id);
        if next {
            self.remaining.insert(0, id)
        } else {
            self.remaining.push(id)
        }
    }
    pub fn remove(&mut self, index: usize) {
        if index >= self.remaining.len() {
            return;
        }
        let id = self.remaining.remove(index);
        self.order.retain(|old| *old != id);
        self.history.retain(|old| *old != id);
        self.preceding.retain(|old| *old != id);
        self.collect();
    }
    pub fn move_before(&mut self, from: usize, to: usize) {
        if from >= self.remaining.len() || to > self.remaining.len() || from == to {
            return;
        }
        let id = self.remaining.remove(from);
        let to = if from < to { to - 1 } else { to };
        let before = self.remaining.get(to).copied();
        self.order.retain(|old| *old != id);
        let at = before
            .and_then(|id| self.order.iter().position(|old| *old == id))
            .unwrap_or(self.order.len());
        self.order.insert(at, id);
        self.remaining.insert(to, id);
    }
    pub fn clear(&mut self) {
        self.remaining.clear();
        self.preceding.clear();
        self.history.clear();
        self.order = self.current.into_iter().collect();
        self.collect();
    }
    fn collect(&mut self) {
        let live: std::collections::HashSet<_> = self
            .order
            .iter()
            .chain(self.current.iter())
            .chain(&self.history)
            .chain(&self.preceding)
            .chain(&self.remaining)
            .copied()
            .collect();
        self.entries.retain(|id, _| live.contains(id));
    }
    pub fn reorder(&mut self, shuffle: bool) {
        self.reorder_with(shuffle, &mut rand::thread_rng());
    }
    pub fn reorder_with(&mut self, shuffle: bool, rng: &mut impl Rng) {
        if shuffle {
            self.remaining.shuffle(rng)
        } else {
            let order: std::collections::HashMap<_, _> = self
                .order
                .iter()
                .enumerate()
                .map(|(i, id)| (*id, i))
                .collect();
            self.remaining
                .sort_by_key(|id| order.get(id).copied().unwrap_or(usize::MAX));
        }
    }
    pub fn jump_to(&mut self, index: usize) -> Option<QueueEntry> {
        if index >= self.remaining.len() {
            return None;
        }
        let mut skipped: Vec<_> = self.remaining.drain(..=index).collect();
        let id = skipped.pop()?;
        if let Some(old) = self.current.replace(id) {
            self.history.push(old);
        }
        self.history.extend(skipped);
        self.current().cloned()
    }
    pub fn advance(&mut self, repeat: RepeatMode, shuffle: bool, eof: bool) -> Option<QueueEntry> {
        if eof && repeat == RepeatMode::One && self.current.is_some() {
            return self.current().cloned();
        }
        if self.remaining.is_empty() && repeat == RepeatMode::All {
            self.remaining = self.order.clone();
            self.reorder(shuffle);
            if shuffle
                && self
                    .current()
                    .is_some_and(|c| self.remaining().first().is_some_and(|t| same(c, t)))
            {
                let different = self
                    .remaining()
                    .iter()
                    .position(|t| !same(t, self.current().unwrap()));
                if let Some(i) = different {
                    self.remaining.swap(0, i);
                }
            }
        }
        self.jump_to(0)
    }
    pub fn previous(&mut self) -> Option<QueueEntry> {
        let id = self.history.pop().or_else(|| self.preceding.pop())?;
        self.remaining.retain(|old| *old != id);
        if let Some(current) = self.current.replace(id) {
            self.remaining.insert(0, current);
        }
        self.current().cloned()
    }
}
fn serialize_entries<S: serde::Serializer>(
    entries: &BTreeMap<QueueEntryId, QueueEntry>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    entries.values().collect::<Vec<_>>().serialize(serializer)
}
#[derive(Deserialize)]
struct StoredSession {
    entries: Vec<QueueEntry>,
    order: Vec<QueueEntryId>,
    remaining: Vec<QueueEntryId>,
    current: Option<QueueEntryId>,
    history: Vec<QueueEntryId>,
    preceding: Vec<QueueEntryId>,
    source: Option<i64>,
    serial: u64,
}
#[derive(Clone, Deserialize)]
struct LegacyTrack {
    #[serde(default)]
    entry_id: QueueEntryId,
    #[serde(flatten)]
    track: TrackMetadata,
}
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct LegacySession {
    snapshot: Vec<LegacyTrack>,
    remaining: Vec<LegacyTrack>,
    next_cycle: Vec<LegacyTrack>,
    current: Option<LegacyTrack>,
    history: Vec<LegacyTrack>,
    preceding: Vec<LegacyTrack>,
    source: Option<i64>,
    serial: u64,
}
impl<'de> Deserialize<'de> for Session {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Format {
            Modern(StoredSession),
            Legacy(LegacySession),
        }
        match Format::deserialize(deserializer)? {
            Format::Modern(s) => {
                let count = s.entries.len();
                let entries: BTreeMap<_, _> =
                    s.entries.into_iter().map(|e| (e.entry_id, e)).collect();
                if entries.len() != count {
                    return Err(serde::de::Error::custom("duplicate queue entry ID"));
                }
                if entries
                    .iter()
                    .any(|(id, entry)| *id != entry.entry_id || id.0 == 0)
                    || s.order
                        .iter()
                        .chain(&s.remaining)
                        .chain(s.current.iter())
                        .chain(&s.history)
                        .chain(&s.preceding)
                        .any(|id| !entries.contains_key(id))
                {
                    return Err(serde::de::Error::custom("invalid queue references"));
                }
                let serial = s
                    .serial
                    .max(entries.keys().map(|id| id.0).max().unwrap_or(0));
                Ok(Self {
                    entries,
                    order: s.order,
                    remaining: s.remaining,
                    current: s.current,
                    history: s.history,
                    preceding: s.preceding,
                    source: s.source,
                    serial,
                })
            }
            Format::Legacy(s) => {
                let mut result = Self {
                    serial: s.serial,
                    source: s.source,
                    ..Self::default()
                };
                fn import(result: &mut Session, t: LegacyTrack) -> QueueEntryId {
                    if t.entry_id.0 > 0 {
                        result.serial = result.serial.max(t.entry_id.0);
                        result.entries.entry(t.entry_id).or_insert(QueueEntry {
                            entry_id: t.entry_id,
                            track: t.track,
                        });
                        t.entry_id
                    } else {
                        result.entry(t.track)
                    }
                }
                result.serial = result.serial.max(
                    s.snapshot
                        .iter()
                        .chain(&s.remaining)
                        .chain(s.current.iter())
                        .chain(&s.history)
                        .chain(&s.preceding)
                        .map(|t| t.entry_id.0)
                        .max()
                        .unwrap_or(0),
                );
                result.order = s
                    .snapshot
                    .into_iter()
                    .map(|t| import(&mut result, t))
                    .collect();
                let had_snapshot = !result.order.is_empty();
                let base_len = result.order.len().max(1);
                fn reference(
                    result: &mut Session,
                    t: LegacyTrack,
                    used: &mut std::collections::HashSet<QueueEntryId>,
                    hint: Option<usize>,
                    append: bool,
                    reuse: bool,
                ) -> QueueEntryId {
                    let id = if t.entry_id.0 > 0 {
                        import(result, t)
                    } else {
                        let hinted = hint
                            .and_then(|i| result.order.get(i))
                            .copied()
                            .filter(|id| result.entries[id].id() == t.track.id());
                        let candidate = hinted.or_else(|| {
                            result.order.iter().copied().find(|id| {
                                !used.contains(id) && result.entries[id].id() == t.track.id()
                            })
                        });
                        match candidate.or_else(|| {
                            reuse
                                .then(|| {
                                    result
                                        .order
                                        .iter()
                                        .copied()
                                        .find(|id| result.entries[id].id() == t.track.id())
                                })
                                .flatten()
                        }) {
                            Some(id) => id,
                            None => {
                                let id = result.entry(t.track);
                                if append {
                                    result.order.push(id);
                                }
                                id
                            }
                        }
                    };
                    used.insert(id);
                    id
                }
                let preceding_len = s.preceding.len();
                let history_len = s.history.len();
                let mut preceding_used = std::collections::HashSet::new();
                result.preceding = s
                    .preceding
                    .into_iter()
                    .enumerate()
                    .map(|(index, t)| {
                        reference(
                            &mut result,
                            t,
                            &mut preceding_used,
                            had_snapshot.then_some(index),
                            false,
                            true,
                        )
                    })
                    .collect();
                let mut history_used = std::collections::HashSet::new();
                result.history = s
                    .history
                    .into_iter()
                    .enumerate()
                    .map(|(index, t)| {
                        reference(
                            &mut result,
                            t,
                            &mut history_used,
                            had_snapshot.then_some((preceding_len + index) % base_len),
                            false,
                            true,
                        )
                    })
                    .collect();
                let current_hint = had_snapshot.then_some((preceding_len + history_len) % base_len);
                result.current = s.current.map(|t| {
                    reference(
                        &mut result,
                        t,
                        &mut preceding_used,
                        current_hint,
                        !had_snapshot,
                        true,
                    )
                });
                let mut remaining_used = std::collections::HashSet::new();
                result.remaining = s
                    .remaining
                    .into_iter()
                    .chain(s.next_cycle)
                    .map(|t| {
                        if t.entry_id.0 == 0 && !had_snapshot {
                            let id = import(&mut result, t);
                            result.order.push(id);
                            remaining_used.insert(id);
                            return id;
                        }
                        let mut reserved: std::collections::HashSet<QueueEntryId> =
                            preceding_used.union(&remaining_used).copied().collect();
                        let available = result
                            .order
                            .iter()
                            .copied()
                            .find(|id| {
                                !reserved.contains(id) && result.entries[id].id() == t.track.id()
                            })
                            .or_else(|| {
                                result.order.iter().copied().find(|id| {
                                    !remaining_used.contains(id)
                                        && result.entries[id].id() == t.track.id()
                                })
                            });
                        let hint =
                            available.and_then(|id| result.order.iter().position(|old| *old == id));
                        let id = reference(&mut result, t, &mut reserved, hint, true, false);
                        remaining_used.insert(id);
                        id
                    })
                    .collect();
                if !had_snapshot {
                    for id in result
                        .current
                        .into_iter()
                        .chain(result.remaining.iter().copied())
                    {
                        if !result.order.contains(&id) {
                            result.order.push(id);
                        }
                    }
                }
                Ok(result)
            }
        }
    }
}
#[cfg(test)]
#[path = "playback/tests.rs"]
mod tests;
