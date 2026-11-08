//! Import-only structures for historical state documents.
use crate::model::{State, Track};
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LegacyImport {
    #[serde(flatten)]
    pub state: State,
    pub queue: Vec<Track>,
    #[serde(default)]
    pub queue_version: u8,
    pub previous_queue: Vec<Track>,
    pub pending_pins: Vec<Track>,
}
impl Default for LegacyImport {
    fn default() -> Self {
        Self {
            state: State::default(),
            queue: vec![],
            queue_version: 1,
            previous_queue: vec![],
            pending_pins: vec![],
        }
    }
}
impl std::ops::Deref for LegacyImport {
    type Target = State;
    fn deref(&self) -> &State {
        &self.state
    }
}
impl std::ops::DerefMut for LegacyImport {
    fn deref_mut(&mut self) -> &mut State {
        &mut self.state
    }
}
impl LegacyImport {
    /// Older releases stored the automatic tail in `queue`; the first session
    /// release misclassified it as manual. Repair only a substantial, exact
    /// suffix already present in the automatic continuation. Keep a backup.
    pub fn repair_legacy_queue(&mut self) -> bool {
        if self.queue_version >= 1 {
            return false;
        }
        self.queue_version = 1;
        let count = self.queue.len();
        if count < 8 || count > self.session.snapshot().len() {
            return false;
        }
        let suffix = &self.session.snapshot().to_vec()[self.session.snapshot().len() - count..];
        let automatic = self.session.remaining();
        if self.queue.iter().zip(suffix).all(|(a, b)| a.id() == b.id())
            && self
                .queue
                .iter()
                .all(|q| automatic.iter().any(|a| q.id() == a.id()))
        {
            self.previous_queue = std::mem::take(&mut self.queue);
            return true;
        }
        false
    }

    pub fn finish(mut self) -> State {
        self.repair_legacy_queue();
        if self.queue_version < 2 {
            let queue = std::mem::take(&mut self.queue);
            self.state.session.import(queue);
        }
        self.state
    }
}
#[cfg(test)]
mod queue_repair_tests {
    use super::*;

    fn old_state() -> LegacyImport {
        let tracks: Vec<_> = (0..131)
            .map(|message| Track {
                chat: 7,
                message,
                ..Default::default()
            })
            .collect();
        let mut state = LegacyImport {
            queue_version: 0,
            queue: tracks[20..].to_vec(),
            ..Default::default()
        };
        state.session.start(&tracks, 0, false);
        state.session.reorder(true);
        state
    }
    #[test]
    fn repairs_duplicated_legacy_tail_without_losing_tracks_or_order() {
        let mut state = old_state();
        let before = state.session.remaining().to_vec();
        let legacy = state.queue.clone();
        assert!(state.repair_legacy_queue());
        assert!(state.queue.is_empty());
        assert_eq!(state.session.remaining(), before);
        assert_eq!(state.previous_queue, legacy);
        state.session.reorder(false);
        assert_eq!(state.session.remaining()[0].message, 1);
        assert_eq!(
            state.session.remaining().iter().last().unwrap().message,
            130
        );
        assert!(!state.repair_legacy_queue());
        let saved: LegacyImport =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(saved.previous_queue, legacy);
        assert_eq!(saved.queue_version, 1);
    }
    #[test]
    fn leaves_explicit_or_nonduplicated_queues_untouched() {
        let mut state = old_state();
        state.queue.swap(0, 1);
        let queue = state.queue.clone();
        assert!(!state.repair_legacy_queue());
        assert_eq!(state.queue, queue);
        let mut state = old_state();
        state.session.clear();
        assert!(!state.repair_legacy_queue());
        assert_eq!(state.queue.len(), 111);
        let mut state = old_state();
        state.queue_version = 1;
        assert!(!state.repair_legacy_queue());
        assert_eq!(state.queue.len(), 111);
        let mut state = old_state();
        state.queue = state.queue[105..].to_vec();
        assert!(!state.repair_legacy_queue());
        assert_eq!(state.queue.len(), 6);
    }
    #[test]
    fn missing_version_is_detected_but_new_queues_are_versioned() {
        let old: LegacyImport = serde_json::from_str(r#"{"queue":[]}"#).unwrap();
        assert_eq!(old.queue_version, 0);
        assert_eq!(LegacyImport::default().queue_version, 1);
    }
}
