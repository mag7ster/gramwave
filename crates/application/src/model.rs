use serde::{Deserialize, Serialize};
pub use tg_player_domain::{MediaKind, QueueEntryId, TrackId};
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
    pub library_id: u64,
    pub entry_id: QueueEntryId,
    pub chat: i64,
    pub message: i64,
    pub title: String,
    pub artist: String,
    pub duration: u64,
    pub kind: MediaKind,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Playlist {
    pub entry_serial: u64,
    pub id: i64,
    pub title: String,
    pub url: String,
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
            let mut found = false;
            for old in self
                .tracks
                .iter_mut()
                .filter(|old| old.message == t.message)
            {
                let mut fresh = t.clone();
                fresh.library_id = old.library_id;
                *old = fresh;
                found = true;
            }
            if !found {
                t.library_id = 0;
                self.tracks.push(t);
            }
        }
        self.tracks.sort_by_key(|t| std::cmp::Reverse(t.message));
        self.ensure_entry_ids();
    }
    pub fn ensure_entry_ids(&mut self) {
        let mut serial = self
            .entry_serial
            .max(self.tracks.iter().map(|t| t.library_id).max().unwrap_or(0));
        let mut seen = std::collections::HashSet::new();
        for track in &mut self.tracks {
            if track.library_id == 0 || !seen.insert(track.library_id) {
                serial = serial.checked_add(1).expect("library identity exhausted");
                track.library_id = serial;
                seen.insert(serial);
            }
        }
        self.entry_serial = serial;
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
    pub session: tg_player_domain::playback::Session,
    pub repeat: tg_player_domain::playback::RepeatMode,
    pub shuffle: bool,
    pub volume: f32,
    pub unmuted_volume: f32,
    pub api_id: String,
    pub api_hash: String,
    pub light: bool,
    #[serde(default = "default_filters")]
    pub filters: [bool; 3],
}
impl Default for State {
    fn default() -> Self {
        Self {
            playlists: Vec::new(),
            chat_pins: Vec::new(),
            session: Default::default(),
            repeat: Default::default(),
            shuffle: false,
            volume: 75.,
            unmuted_volume: 75.,
            api_id: String::new(),
            api_hash: String::new(),
            light: false,
            filters: default_filters(),
        }
    }
}
impl State {
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

impl From<Track> for tg_player_domain::TrackMetadata {
    fn from(t: Track) -> Self {
        Self {
            chat: t.chat,
            message: t.message,
            title: t.title,
            artist: t.artist,
            duration: t.duration,
            kind: t.kind,
        }
    }
}
impl From<tg_player_domain::QueueEntry> for Track {
    fn from(entry: tg_player_domain::QueueEntry) -> Self {
        Self {
            entry_id: entry.entry_id,
            chat: entry.track.chat,
            message: entry.track.message,
            title: entry.track.title,
            artist: entry.track.artist,
            duration: entry.track.duration,
            kind: entry.track.kind,
            ..Default::default()
        }
    }
}
impl From<&tg_player_domain::QueueEntry> for Track {
    fn from(entry: &tg_player_domain::QueueEntry) -> Self {
        entry.clone().into()
    }
}
