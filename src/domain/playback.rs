//! One editable playback sequence. Snapshot is its base order; remaining is the
//! exact upcoming order. Entry IDs identify occurrences independently of messages.
use crate::model::Track;
use serde::{Deserialize, Serialize};
use std::hash::BuildHasher;

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
pub fn same(a: &Track, b: &Track) -> bool {
    a.id() == b.id()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub snapshot: Vec<Track>,
    pub remaining: Vec<Track>,
    pub next_cycle: Vec<Track>, // legacy import only
    pub current: Option<Track>,
    // Backward navigation, including positions skipped by a queue jump.
    // Keep the serialized name for existing saved sessions.
    pub history: Vec<Track>,
    pub preceding: Vec<Track>,
    pub source: Option<i64>,
    serial: u64,
}
impl Session {
    fn entry(&mut self, mut track: Track) -> Track {
        self.serial += 1;
        track.entry_id = crate::domain::model::QueueEntryId(self.serial);
        track
    }
    pub fn start(&mut self, tracks: &[Track], index: usize, shuffle: bool) -> Option<Track> {
        tracks.get(index)?;
        self.snapshot = tracks.iter().cloned().map(|t| self.entry(t)).collect();
        self.current = Some(self.snapshot[index].clone());
        self.remaining = if shuffle {
            self.snapshot
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .map(|(_, t)| t.clone())
                .collect()
        } else {
            self.snapshot[index + 1..].to_vec()
        };
        self.history.clear();
        self.preceding = if shuffle {
            Vec::new()
        } else {
            self.snapshot[..index].to_vec()
        };
        self.next_cycle.clear();
        self.source = tracks.first().map(|t| t.chat);
        self.reorder(shuffle);
        self.current.clone()
    }
    pub fn import(&mut self, manual: Vec<Track>) {
        let tracks: Vec<_> = manual
            .into_iter()
            .chain(std::mem::take(&mut self.remaining))
            .chain(std::mem::take(&mut self.next_cycle))
            .collect();
        self.snapshot = tracks.into_iter().map(|t| self.entry(t)).collect();
        self.remaining = self.snapshot.clone();
        self.history.clear();
        self.preceding.clear();
    }
    pub fn insert(&mut self, track: Track, next: bool) {
        let track = self.entry(track);
        let before = next
            .then(|| self.remaining.first().map(|t| t.entry_id))
            .flatten();
        let at = before
            .and_then(|id| self.snapshot.iter().position(|t| t.entry_id == id))
            .unwrap_or(self.snapshot.len());
        self.snapshot.insert(at, track.clone());
        if next {
            self.remaining.insert(0, track);
        } else {
            self.remaining.push(track);
        }
    }
    pub fn remove(&mut self, index: usize) {
        if index < self.remaining.len() {
            let id = self.remaining.remove(index).entry_id;
            self.snapshot.retain(|t| t.entry_id != id);
            self.history.retain(|t| t.entry_id != id);
            self.preceding.retain(|t| t.entry_id != id);
        }
    }
    pub fn move_before(&mut self, from: usize, to: usize) {
        if from >= self.remaining.len() || to > self.remaining.len() || from == to {
            return;
        }
        let track = self.remaining.remove(from);
        let to = if from < to { to - 1 } else { to };
        let before = self.remaining.get(to).map(|t| t.entry_id);
        self.snapshot.retain(|t| t.entry_id != track.entry_id);
        let at = before
            .and_then(|id| self.snapshot.iter().position(|t| t.entry_id == id))
            .unwrap_or(self.snapshot.len());
        self.snapshot.insert(at, track.clone());
        self.remaining.insert(to, track);
    }
    pub fn clear(&mut self) {
        self.remaining.clear();
        self.preceding.clear();
        self.history.clear();
        self.next_cycle.clear();
        self.snapshot = self.current.iter().cloned().collect();
    }
    pub fn reorder(&mut self, shuffle: bool) {
        if shuffle {
            shuffle_tracks(&mut self.remaining);
        } else {
            self.remaining.sort_by_key(|t| {
                self.snapshot
                    .iter()
                    .position(|s| s.entry_id == t.entry_id)
                    .unwrap_or(usize::MAX)
            });
        }
    }
    pub fn prepare_cycle(&mut self, _repeat: RepeatMode, _shuffle: bool) {}
    pub fn jump_to(&mut self, _manual: &mut Vec<Track>, index: usize) -> Option<Track> {
        if index >= self.remaining.len() {
            return None;
        }
        let mut skipped: Vec<_> = self.remaining.drain(..=index).collect();
        let track = skipped.pop()?;
        if let Some(old) = self.current.replace(track.clone()) {
            self.history.push(old);
        }
        self.history.extend(skipped);
        Some(track)
    }
    pub fn advance(
        &mut self,
        manual: &mut Vec<Track>,
        repeat: RepeatMode,
        shuffle: bool,
        eof: bool,
    ) -> Option<Track> {
        if eof && repeat == RepeatMode::One && self.current.is_some() {
            return self.current.clone();
        }
        if self.remaining.is_empty() && repeat == RepeatMode::All {
            self.remaining = self.snapshot.clone();
            self.reorder(shuffle);
            if shuffle
                && self
                    .current
                    .as_ref()
                    .is_some_and(|c| self.remaining.first().is_some_and(|t| same(c, t)))
            {
                if let Some(i) = self
                    .remaining
                    .iter()
                    .position(|t| !same(t, self.current.as_ref().unwrap()))
                {
                    self.remaining.swap(0, i);
                }
            }
        }
        self.jump_to(manual, 0)
    }
    pub fn previous(&mut self, _manual: &mut Vec<Track>) -> Option<Track> {
        let previous = self.history.pop().or_else(|| self.preceding.pop())?;
        self.remaining.retain(|t| t.entry_id != previous.entry_id);
        if let Some(current) = self.current.replace(previous.clone()) {
            self.remaining.insert(0, current);
        }
        Some(previous)
    }
}
fn shuffle_tracks(tracks: &mut [Track]) {
    let random = std::collections::hash_map::RandomState::new();
    for i in (1..tracks.len()).rev() {
        let j = random.hash_one(i) as usize % (i + 1);
        tracks.swap(i, j);
    }
}

#[cfg(test)]
#[path = "playback/tests.rs"]
mod tests;
