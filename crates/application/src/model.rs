pub use gramwave_domain::{MediaKind, QueueEntryId, TrackId};
use serde::{Deserialize, Serialize};
impl Track {
    pub fn id(&self) -> TrackId {
        TrackId {
            chat: self.chat,
            message: self.message,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmojiStatus {
    pub id: i64,
    pub expires_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionEmoji {
    pub bytes: std::ops::Range<usize>,
    pub id: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caption {
    pub text: String,
    pub emoji: Vec<CaptionEmoji>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<CaptionLink>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionLink {
    pub bytes: std::ops::Range<usize>,
    pub url: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<Caption>,
    pub duration: u64,
    pub kind: MediaKind,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Playlist {
    pub entry_serial: u64,
    pub id: i64,
    pub title: String,
    pub emoji_status: Option<EmojiStatus>,
    pub personal: PersonalChat,
    pub url: String,
    pub tracks: Vec<Track>,
    pub filters: [bool; 3],
    pub pinned: bool,
    pub cursors: [i64; 3],
    pub done: [bool; 3],
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PersonalChat {
    pub lists: [bool; 2],
    pub saved_messages: bool,
}
impl PersonalChat {
    pub fn is_empty(&self) -> bool {
        !self.saved_messages && !self.lists.contains(&true)
    }
}
impl Playlist {
    pub fn in_chat_search(&self) -> bool {
        !self.personal.is_empty() && !crate::telegram::is_channel_direct_messages(self.id)
    }

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
    pub recent_chats: Vec<i64>,
    pub last_opened_chat: Option<i64>,
    pub startup_playlist: StartupPlaylist,
    pub last_played_chat: Option<i64>,
    pub session: gramwave_domain::playback::Session,
    pub repeat: gramwave_domain::playback::RepeatMode,
    pub shuffle: bool,
    pub volume: f32,
    pub unmuted_volume: f32,
    pub api_id: String,
    pub api_hash: String,
    pub light: bool,
    pub queue_panel_open: bool,
    pub cache_policy: crate::persistence::CachePolicy,
    #[serde(default = "default_filters")]
    pub filters: [bool; 3],
}
impl Default for State {
    fn default() -> Self {
        Self {
            playlists: Vec::new(),
            chat_pins: Vec::new(),
            recent_chats: Vec::new(),
            last_opened_chat: None,
            startup_playlist: StartupPlaylist::default(),
            last_played_chat: None,
            session: Default::default(),
            repeat: Default::default(),
            shuffle: false,
            volume: 75.,
            unmuted_volume: 75.,
            api_id: String::new(),
            api_hash: String::new(),
            light: false,
            queue_panel_open: false,
            cache_policy: Default::default(),
            filters: default_filters(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupPlaylist {
    #[default]
    LastOpened,
    LastPlayed,
}

impl State {
    pub fn startup_chat(&self) -> Option<i64> {
        let last_played = match self.startup_playlist {
            StartupPlaylist::LastOpened => None,
            StartupPlaylist::LastPlayed => self
                .last_played_chat
                .or_else(|| self.session.current().map(|track| track.chat)),
        };
        last_played
            .into_iter()
            .chain(self.last_opened_chat)
            .chain(self.session.source)
            .filter(|&id| !crate::telegram::is_channel_direct_messages(id))
            .find(|id| self.playlists.iter().any(|playlist| playlist.id == *id))
    }

    pub fn sidebar_chats(&self, searching: bool, query: &str) -> Vec<usize> {
        if searching {
            let query = query.trim().to_lowercase();
            self.playlists
                .iter()
                .enumerate()
                .filter(|(_, p)| p.in_chat_search() && p.title.to_lowercase().contains(&query))
                .map(|(i, _)| i)
                .collect()
        } else {
            self.chat_pins
                .iter()
                .filter(|&&id| !crate::telegram::is_channel_direct_messages(id))
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

impl From<Track> for gramwave_domain::TrackMetadata {
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
impl From<gramwave_domain::QueueEntry> for Track {
    fn from(entry: gramwave_domain::QueueEntry) -> Self {
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
impl From<&gramwave_domain::QueueEntry> for Track {
    fn from(entry: &gramwave_domain::QueueEntry) -> Self {
        entry.clone().into()
    }
}
