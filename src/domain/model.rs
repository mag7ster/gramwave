use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "usize", into = "usize")]
pub enum MediaKind {
    #[default]
    Audio,
    Document,
    Voice,
}
impl From<MediaKind> for usize {
    fn from(value: MediaKind) -> usize {
        value as usize
    }
}
impl TryFrom<usize> for MediaKind {
    type Error = String;
    fn try_from(value: usize) -> Result<Self, String> {
        match value {
            0 => Ok(Self::Audio),
            1 => Ok(Self::Document),
            2 => Ok(Self::Voice),
            _ => Err(format!("unknown media kind: {value}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrackId {
    pub chat: i64,
    pub message: i64,
}
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct QueueEntryId(pub u64);
impl Track {
    pub fn id(&self) -> TrackId {
        TrackId {
            chat: self.chat,
            message: self.message,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    pub entry_id: QueueEntryId,
    pub chat: i64,
    pub message: i64,
    #[serde(skip)]
    pub file: i64,
    pub title: String,
    pub artist: String,
    pub duration: u64,
    pub kind: MediaKind,
    pub pinned: Option<String>,
    #[serde(skip)]
    pub cached: Option<String>,
    #[serde(skip)]
    pub image_file: i64,
    #[serde(skip)]
    pub image_path: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Playlist {
    pub id: i64,
    pub title: String,
    pub url: String,
    #[serde(skip)]
    pub image_file: i64,
    #[serde(skip)]
    pub image_path: Option<String>,
    pub tracks: Vec<Track>,
    pub filters: [bool; 3],
    pub pinned: bool,
    pub cursors: [i64; 3],
    pub done: [bool; 3],
}
impl Playlist {
    pub fn merge_tracks(&mut self, tracks: Vec<Track>, protected: &std::collections::HashSet<i64>) {
        for mut t in tracks {
            if protected.contains(&t.message) {
                continue;
            }
            if let Some(old) = self.tracks.iter_mut().find(|old| old.message == t.message) {
                t.pinned = old.pinned.clone();
                if t.file == 0 {
                    t.file = old.file;
                    t.cached = old.cached.clone();
                    t.image_file = old.image_file;
                }
                if t.image_file == old.image_file {
                    t.image_path = t.image_path.or(old.image_path.clone());
                }
                if *old != t {
                    *old = t;
                }
            } else {
                self.tracks.push(t);
            }
        }
        self.tracks.sort_by_key(|t| std::cmp::Reverse(t.message));
    }
    pub fn recover_empty_index(&mut self) {
        // Older versions could drop cached message metadata while keeping the
        // pagination cursors. Start over only when opening that empty library.
        if self.tracks.is_empty() && self.cursors.iter().any(|c| *c != 0) {
            self.cursors = [0; 3];
            self.done = [false; 3];
        }
    }
    pub fn has_more(&self) -> bool {
        self.filters
            .iter()
            .zip(self.done)
            .any(|(enabled, done)| *enabled && !done)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub playlists: Vec<Playlist>,
    pub chat_pins: Vec<i64>,
    // Legacy queue remains the explicit manual queue.
    pub queue: Vec<Track>,
    #[serde(default)]
    pub queue_version: u8,
    pub previous_queue: Vec<Track>,
    pub session: crate::playback::Session,
    pub repeat: crate::playback::RepeatMode,
    pub shuffle: bool,
    pub volume: f32,
    pub unmuted_volume: f32,
    pub api_id: String,
    pub api_hash: String,
    pub light: bool,
    pub pending_pins: Vec<Track>,
    #[serde(default = "default_filters")]
    pub filters: [bool; 3],
}
impl Default for State {
    fn default() -> Self {
        Self {
            playlists: Vec::new(),
            chat_pins: Vec::new(),
            queue: Vec::new(),
            queue_version: 1,
            previous_queue: Vec::new(),
            session: Default::default(),
            repeat: Default::default(),
            shuffle: false,
            volume: 75.,
            unmuted_volume: 75.,
            api_id: String::new(),
            api_hash: String::new(),
            light: false,
            pending_pins: Vec::new(),
            filters: default_filters(),
        }
    }
}
impl State {
    /// Older releases stored the automatic tail in `queue`; the first session
    /// release misclassified it as manual. Repair only a substantial, exact
    /// suffix already present in the automatic continuation. Keep a backup.
    pub fn repair_legacy_queue(&mut self) -> bool {
        if self.queue_version >= 1 {
            return false;
        }
        self.queue_version = 1;
        let count = self.queue.len();
        if count < 8 || count > self.session.snapshot.len() {
            return false;
        }
        let suffix = &self.session.snapshot[self.session.snapshot.len() - count..];
        let automatic = &self.session.remaining;
        if self
            .queue
            .iter()
            .zip(suffix)
            .all(|(a, b)| crate::playback::same(a, b))
            && self
                .queue
                .iter()
                .all(|q| automatic.iter().any(|a| crate::playback::same(q, a)))
        {
            self.previous_queue = std::mem::take(&mut self.queue);
            return true;
        }
        false
    }

    pub fn sidebar_chats(&self, searching: bool, query: &str) -> Vec<usize> {
        if searching {
            let query = query.trim().to_lowercase();
            self.playlists
                .iter()
                .enumerate()
                .filter(|(_, p)| p.title.to_lowercase().contains(&query))
                .map(|(i, _)| i)
                .collect()
        } else {
            self.chat_pins
                .iter()
                .filter_map(|id| self.playlists.iter().position(|p| p.id == *id))
                .collect()
        }
    }
    pub fn toggle_chat_pin(&mut self, id: i64) {
        if self.chat_pins.contains(&id) {
            self.chat_pins.retain(|x| *x != id);
        } else {
            self.chat_pins.push(id);
        }
    }
}
pub fn default_filters() -> [bool; 3] {
    [true, true, false]
}

#[cfg(test)]
#[path = "model/tests.rs"]
mod tests;
