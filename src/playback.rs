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
impl From<fastframe_now_playing::Repeat> for RepeatMode {
    fn from(value: fastframe_now_playing::Repeat) -> Self {
        match value {
            fastframe_now_playing::Repeat::Off => Self::Off,
            fastframe_now_playing::Repeat::Playlist => Self::All,
            fastframe_now_playing::Repeat::Track => Self::One,
        }
    }
}
impl From<RepeatMode> for fastframe_now_playing::Repeat {
    fn from(value: RepeatMode) -> Self {
        match value {
            RepeatMode::Off => Self::Off,
            RepeatMode::All => Self::Playlist,
            RepeatMode::One => Self::Track,
        }
    }
}
pub fn same(a: &Track, b: &Track) -> bool {
    (a.chat, a.message) == (b.chat, b.message)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub snapshot: Vec<Track>,
    pub remaining: Vec<Track>,
    pub next_cycle: Vec<Track>, // legacy import only
    pub current: Option<Track>,
    pub history: Vec<Track>,
    pub preceding: Vec<Track>,
    pub source: Option<i64>,
    serial: u64,
}
impl Session {
    fn entry(&mut self, mut track: Track) -> Track {
        self.serial += 1;
        track.entry_id = self.serial;
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
        self.preceding = self.snapshot[..index].to_vec();
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
        let track = self.remaining.drain(..=index).next_back()?;
        Some(self.select(track))
    }
    fn select(&mut self, track: Track) -> Track {
        if let Some(old) = self.current.replace(track.clone()) {
            self.history.push(old);
        }
        track
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
mod tests {
    use super::*;
    fn tracks() -> Vec<Track> {
        (0..5)
            .map(|message| Track {
                message,
                chat: 7,
                ..Default::default()
            })
            .collect()
    }
    fn order(s: &Session) -> Vec<i64> {
        s.remaining.iter().map(|t| t.message).collect()
    }
    #[test]
    fn previous_from_middle_survives_restart_and_returns_forward() {
        let mut s = Session::default();
        let mut q = vec![];
        s.start(&tracks(), 3, false);
        let mut s: Session = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s.previous(&mut q).unwrap().message, 2);
        assert_eq!(s.previous(&mut q).unwrap().message, 1);
        assert_eq!(
            s.advance(&mut q, RepeatMode::Off, false, false)
                .unwrap()
                .message,
            2
        );
        assert_eq!(
            s.advance(&mut q, RepeatMode::Off, false, false)
                .unwrap()
                .message,
            3
        );
        assert_eq!(order(&s), [4]);
        s.clear();
        assert!(s.previous(&mut q).is_none());
        s.start(&tracks(), 2, true);
        assert_eq!(s.previous(&mut q).unwrap().message, 1);
    }
    #[test]
    fn edits_duplicates_history_and_repeat() {
        let t = tracks();
        let mut s = Session::default();
        let mut q = vec![];
        s.start(&t, 0, false);
        s.insert(t[2].clone(), true);
        s.insert(t[2].clone(), false);
        assert_eq!(order(&s), [2, 1, 2, 3, 4, 2]);
        assert_ne!(s.remaining[0].entry_id, s.remaining[2].entry_id);
        s.move_before(5, 1);
        s.remove(0);
        assert_eq!(order(&s), [2, 1, 2, 3, 4]);
        assert_eq!(s.jump_to(&mut q, 2).unwrap().message, 2);
        assert_eq!(s.previous(&mut q).unwrap().message, 0);
        assert_eq!(
            s.advance(&mut q, RepeatMode::Off, false, false)
                .unwrap()
                .message,
            2
        );
        assert_eq!(s.history.len(), 1);
        s.clear();
        assert!(s.remaining.is_empty());
        assert_eq!(s.snapshot.len(), 1);
        assert!(s.advance(&mut q, RepeatMode::Off, false, true).is_none());
        assert_eq!(
            s.advance(&mut q, RepeatMode::One, false, true)
                .unwrap()
                .message,
            2
        );
    }
    #[test]
    fn shuffle_edits_restore_and_persist() {
        let mut s = Session::default();
        s.start(&tracks(), 0, false);
        s.reorder(true);
        let anchor = s.remaining[0].entry_id;
        s.insert(tracks()[0].clone(), true);
        let id = s.remaining[0].entry_id;
        s.reorder(false);
        let at = s.remaining.iter().position(|t| t.entry_id == id).unwrap();
        assert_eq!(s.remaining[at + 1].entry_id, anchor);
        let removed = s.remaining[1].entry_id;
        s.remove(1);
        let expected = order(&s);
        let mut restored: Session =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(order(&restored), expected);
        assert_eq!(restored.current, s.current);
        let mut q = vec![];
        for message in expected {
            assert_eq!(
                restored
                    .advance(&mut q, RepeatMode::Off, false, true)
                    .unwrap()
                    .message,
                message
            );
        }
        restored.advance(&mut q, RepeatMode::All, false, true);
        assert!(!restored.snapshot.iter().any(|t| t.entry_id == removed));
    }
    #[test]
    fn empty_single_repeats_and_replacement() {
        let mut s = Session::default();
        let mut q = vec![];
        assert!(s.start(&[], 0, false).is_none());
        assert!(s.advance(&mut q, RepeatMode::All, true, true).is_none());
        s.insert(tracks()[0].clone(), false);
        assert!(s.current.is_none());
        assert_eq!(
            s.advance(&mut q, RepeatMode::Off, false, false)
                .unwrap()
                .message,
            0
        );
        assert_eq!(
            s.advance(&mut q, RepeatMode::All, true, true)
                .unwrap()
                .message,
            0
        );
        s.start(&tracks(), 3, false);
        assert!(s.history.is_empty());
        assert_eq!(order(&s), [4]);
        assert_eq!(
            s.advance(&mut q, RepeatMode::One, false, true)
                .unwrap()
                .message,
            3
        );
        assert_eq!(
            s.advance(&mut q, RepeatMode::One, false, false)
                .unwrap()
                .message,
            4
        );
        assert!(s.advance(&mut q, RepeatMode::Off, false, true).is_none());
        for _ in 0..20 {
            let last = s.current.clone().unwrap();
            let next = s.advance(&mut q, RepeatMode::All, true, true).unwrap();
            if s.remaining.len() == s.snapshot.len() - 1 {
                assert!(!same(&last, &next));
            }
        }
    }
    #[test]
    fn legacy_import_preserves_visible_duplicates() {
        let mut s = Session {
            remaining: tracks()[..2].to_vec(),
            next_cycle: tracks()[..1].to_vec(),
            ..Default::default()
        };
        s.import(tracks()[..1].to_vec());
        assert_eq!(order(&s), [0, 0, 1, 0]);
        let mut ids: Vec<_> = s.remaining.iter().map(|t| t.entry_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 4);
        assert!(s.history.is_empty());
    }
}
