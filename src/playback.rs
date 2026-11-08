//! Playback order is a snapshot; browsing and pagination cannot mutate a cycle.
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
    pub next_cycle: Vec<Track>,
    #[serde(skip)]
    pub current: Option<Track>,
    #[serde(skip)]
    pub history: Vec<Track>,
}
impl Session {
    pub fn start(&mut self, tracks: &[Track], index: usize, shuffle: bool) -> Option<Track> {
        let selected = tracks.get(index)?.clone();
        self.snapshot = tracks.to_vec();
        self.history.clear();
        self.next_cycle.clear();
        self.remaining = if shuffle {
            tracks
                .iter()
                .filter(|t| !same(t, &selected))
                .cloned()
                .collect()
        } else {
            tracks.iter().skip(index + 1).cloned().collect()
        };
        self.reorder(shuffle);
        self.current = Some(selected.clone());
        Some(selected)
    }
    pub fn reorder(&mut self, shuffle: bool) {
        for tracks in [&mut self.remaining, &mut self.next_cycle] {
            if shuffle {
                shuffle_tracks(tracks);
            } else {
                tracks.sort_by_key(|t| {
                    self.snapshot
                        .iter()
                        .position(|s| same(s, t))
                        .unwrap_or(usize::MAX)
                });
            }
        }
    }
    pub fn prepare_cycle(&mut self, repeat: RepeatMode, shuffle: bool) {
        if repeat != RepeatMode::All || !self.remaining.is_empty() {
            self.next_cycle.clear();
            return;
        }
        if self.next_cycle.is_empty() {
            self.next_cycle = self.snapshot.clone();
            if shuffle {
                shuffle_tracks(&mut self.next_cycle);
            }
        }
        if shuffle
            && self.next_cycle.len() > 1
            && self
                .current
                .as_ref()
                .is_some_and(|c| same(c, &self.next_cycle[0]))
        {
            self.next_cycle.swap(0, 1);
        }
    }
    /// Jump forward in the displayed sequence, consuming skipped entries.
    pub fn jump_to(&mut self, manual: &mut Vec<Track>, index: usize) -> Option<Track> {
        if index >= manual.len() + self.remaining.len() + self.next_cycle.len() {
            return None;
        }
        let track = if index < manual.len() {
            manual.drain(..=index).next_back()?
        } else {
            let index = index - manual.len();
            manual.clear();
            if index < self.remaining.len() {
                self.remaining.drain(..=index).next_back()?
            } else {
                let index = index - self.remaining.len();
                self.remaining = std::mem::take(&mut self.next_cycle);
                self.remaining.drain(..=index).next_back()?
            }
        };
        Some(self.select(track))
    }
    pub fn select(&mut self, track: Track) -> Track {
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
        self.prepare_cycle(repeat, shuffle);
        if !manual.is_empty() {
            let track = self.select(manual.remove(0));
            self.prepare_cycle(repeat, shuffle);
            return Some(track);
        }
        if self.remaining.is_empty() && repeat == RepeatMode::All {
            self.remaining = std::mem::take(&mut self.next_cycle);
        }
        if self.remaining.is_empty() {
            if let Some(old) = self.current.take() {
                self.history.push(old);
            }
            None
        } else {
            let track = self.remaining.remove(0);
            let track = self.select(track);
            self.prepare_cycle(repeat, shuffle);
            Some(track)
        }
    }
    pub fn previous(&mut self, manual: &mut Vec<Track>) -> Option<Track> {
        let previous = self.history.pop()?;
        if let Some(current) = self.current.replace(previous.clone()) {
            manual.insert(0, current);
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
                ..Default::default()
            })
            .collect()
    }
    #[test]
    fn repeat_one_only_repeats_eof_and_all_restarts_snapshot() {
        let tracks = tracks();
        let mut s = Session::default();
        let mut q = vec![];
        s.start(&tracks, 3, false);
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
        assert_eq!(
            s.advance(&mut q, RepeatMode::All, false, true)
                .unwrap()
                .message,
            0
        );
    }
    #[test]
    fn manual_priority_history_and_new_context() {
        let tracks = tracks();
        let mut s = Session::default();
        let mut q = vec![Track {
            message: 9,
            ..Default::default()
        }];
        s.start(&tracks, 2, false);
        assert_eq!(
            s.advance(&mut q, RepeatMode::Off, false, false)
                .unwrap()
                .message,
            9
        );
        assert_eq!(s.previous(&mut q).unwrap().message, 2);
        assert_eq!(
            s.advance(&mut q, RepeatMode::Off, false, false)
                .unwrap()
                .message,
            9
        );
        assert_eq!(
            s.advance(&mut q, RepeatMode::Off, false, false)
                .unwrap()
                .message,
            3
        );
        q.push(tracks[0].clone());
        s.start(&tracks, 4, false);
        assert_eq!(
            s.advance(&mut q, RepeatMode::All, false, false)
                .unwrap()
                .message,
            0
        );
        assert_eq!(
            s.advance(&mut q, RepeatMode::All, false, false)
                .unwrap()
                .message,
            0
        );
        assert!(q.is_empty());
    }
    #[test]
    fn shuffle_has_no_repeats_and_toggle_preserves_remaining_set() {
        for _ in 0..32 {
            let tracks = tracks();
            let mut s = Session::default();
            let mut q = vec![];
            s.start(&tracks, 2, true);
            let mut played = vec![2];
            played.push(
                s.advance(&mut q, RepeatMode::Off, true, false)
                    .unwrap()
                    .message,
            );
            let current = s.current.clone();
            let history = s.history.clone();
            s.reorder(false);
            assert!(s.remaining.windows(2).all(|w| w[0].message < w[1].message));
            assert_eq!(s.current, current);
            assert_eq!(s.history, history);
            s.reorder(true);
            while let Some(t) = s.advance(&mut q, RepeatMode::Off, true, true) {
                played.push(t.message);
            }
            played.sort();
            assert_eq!(played, vec![0, 1, 2, 3, 4]);
            s.start(&tracks, 0, true);
            for _ in 0..4 {
                s.advance(&mut q, RepeatMode::All, true, true);
            }
            let last = s.current.clone().unwrap();
            assert!(!same(
                &last,
                &s.advance(&mut q, RepeatMode::All, true, true).unwrap()
            ));
        }
    }
    #[test]
    fn next_cycle_preview_matches_playback_and_off_cancels_it() {
        let mut s = Session::default();
        let mut q = vec![];
        s.start(&tracks(), 4, false);
        s.prepare_cycle(RepeatMode::All, true);
        let preview = s.next_cycle.clone();
        assert_eq!(preview.len(), 5);
        assert!(!same(&preview[0], s.current.as_ref().unwrap()));
        for expected in preview {
            assert_eq!(
                s.advance(&mut q, RepeatMode::All, true, true).unwrap(),
                expected
            );
        }
        assert_eq!(s.next_cycle.len(), 5);
        s.prepare_cycle(RepeatMode::Off, true);
        assert!(s.next_cycle.is_empty());
        assert!(s.advance(&mut q, RepeatMode::Off, true, true).is_none());
        for mode in [RepeatMode::Off, RepeatMode::All, RepeatMode::One] {
            assert_eq!(
                RepeatMode::from(fastframe_now_playing::Repeat::from(mode)),
                mode
            );
        }
        assert_eq!(RepeatMode::Off.next().next().next(), RepeatMode::Off);
    }
    #[test]
    fn choosing_a_later_queue_track_continues_after_it() {
        let tracks = tracks();
        let mut s = Session::default();
        let mut manual = vec![tracks[3].clone()];
        s.start(&tracks, 0, false);
        assert_eq!(s.jump_to(&mut manual, 3).unwrap().message, 3);
        assert!(manual.is_empty());
        assert_eq!(
            s.advance(&mut manual, RepeatMode::Off, false, false)
                .unwrap()
                .message,
            4
        );
        assert!(s
            .advance(&mut manual, RepeatMode::Off, false, false)
            .is_none());
        s.start(&tracks, 4, false);
        s.prepare_cycle(RepeatMode::All, false);
        assert_eq!(s.jump_to(&mut manual, 2).unwrap().message, 2);
        assert_eq!(
            s.advance(&mut manual, RepeatMode::Off, false, false)
                .unwrap()
                .message,
            3
        );
    }
    #[test]
    fn empty_single_and_persistence() {
        let mut s = Session::default();
        let mut q = vec![];
        assert!(s.start(&[], 0, false).is_none());
        assert!(s.advance(&mut q, RepeatMode::All, true, true).is_none());
        s.start(&tracks()[..1], 0, true);
        assert_eq!(
            s.advance(&mut q, RepeatMode::All, true, true)
                .unwrap()
                .message,
            0
        );
        assert!(s.advance(&mut q, RepeatMode::Off, true, true).is_none());
        let old: crate::model::State =
            serde_json::from_str(r#"{"queue":[{"message":42}]}"#).unwrap();
        assert_eq!(old.queue[0].message, 42);
        assert_eq!(old.repeat, RepeatMode::Off);
        let state = crate::model::State {
            session: s,
            repeat: RepeatMode::One,
            shuffle: true,
            volume: 23.,
            ..Default::default()
        };
        let saved: crate::model::State =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert!(saved.session.current.is_none());
        assert_eq!(saved.volume, 23.);
        assert_eq!(saved.repeat, RepeatMode::One);
        assert!(saved.shuffle);
    }
}
