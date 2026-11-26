//! Explicit persistence contracts. Runtime media never enters a settings record.
use crate::model::{Playlist, StartupPlaylist, State, Track};
use gramwave_domain::{RepeatMode, Session};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CachePolicy {
    pub days: u32,
    pub size_mib: u32,
}
impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            days: 7,
            size_mib: 1024,
        }
    }
}
impl CachePolicy {
    pub fn normalized(self) -> Self {
        Self {
            days: self.days.clamp(1, 365),
            size_mib: self.size_mib.clamp(64, 16384),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub chat_pins: Vec<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recent_chats: Vec<i64>,
    pub last_opened_chat: Option<i64>,
    pub startup_playlist: StartupPlaylist,
    pub last_played_chat: Option<i64>,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub volume: f32,
    pub unmuted_volume: f32,
    pub light: bool,
    pub queue_panel_open: bool,
    pub cache_policy: CachePolicy,
    pub filters: [bool; 3],
}
impl Default for Settings {
    fn default() -> Self {
        Self::from(&State::default())
    }
}
impl From<&State> for Settings {
    fn from(s: &State) -> Self {
        Self {
            chat_pins: s.chat_pins.clone(),
            recent_chats: s.recent_chats.clone(),
            last_opened_chat: s.last_opened_chat,
            startup_playlist: s.startup_playlist,
            last_played_chat: s.last_played_chat,
            repeat: s.repeat,
            shuffle: s.shuffle,
            volume: s.volume,
            unmuted_volume: s.unmuted_volume,
            light: s.light,
            queue_panel_open: s.queue_panel_open,
            cache_policy: s.cache_policy,
            filters: s.filters,
        }
    }
}
impl Settings {
    pub fn apply(self, s: &mut State) {
        s.chat_pins = self.chat_pins;
        s.recent_chats = self.recent_chats;
        s.last_opened_chat = self.last_opened_chat;
        s.startup_playlist = self.startup_playlist;
        s.last_played_chat = self.last_played_chat;
        s.repeat = self.repeat;
        s.shuffle = self.shuffle;
        s.volume = self.volume;
        s.unmuted_volume = self.unmuted_volume;
        s.light = self.light;
        s.queue_panel_open = self.queue_panel_open;
        s.cache_policy = self.cache_policy.normalized();
        s.filters = self.filters;
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaylistHeader {
    pub entry_serial: u64,
    pub id: i64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji_status: Option<crate::model::EmojiStatus>,
    #[serde(default, skip_serializing_if = "crate::model::PersonalChat::is_empty")]
    pub personal: crate::model::PersonalChat,
    pub url: String,
    pub filters: [bool; 3],
    pub pinned: bool,
    pub cursors: [i64; 3],
    pub done: [bool; 3],
}
impl From<&Playlist> for PlaylistHeader {
    fn from(p: &Playlist) -> Self {
        Self {
            entry_serial: p.entry_serial,
            id: p.id,
            title: p.title.clone(),
            emoji_status: p.emoji_status,
            personal: p.personal,
            url: p.url.clone(),
            filters: p.filters,
            pinned: p.pinned,
            cursors: p.cursors,
            done: p.done,
        }
    }
}
#[derive(Clone, Debug)]
pub struct TrackChange {
    pub playlist: i64,
    pub track: Track,
}
#[derive(Clone, Debug, Default)]
pub struct ChangeSet {
    pub offline: Option<crate::offline::OfflineDelta>,
    pub settings: Option<Settings>,
    pub session: Option<Session>,
    pub library_order: Option<Vec<i64>>,
    pub headers: Vec<PlaylistHeader>,
    pub tracks: Vec<TrackChange>,
    pub track_order: Vec<(i64, Vec<u64>)>,
}
impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.offline.is_none()
            && self.settings.is_none()
            && self.session.is_none()
            && self.library_order.is_none()
            && self.headers.is_empty()
            && self.tracks.is_empty()
            && self.track_order.is_empty()
    }
    /// Coalesce only before a flush barrier. A newer value supersedes the same entity.
    pub fn merge(&mut self, newer: Self) {
        if let Some(offline) = newer.offline {
            match &mut self.offline {
                Some(current) => current.merge(offline),
                None => self.offline = Some(offline),
            }
        }
        if newer.settings.is_some() {
            self.settings = newer.settings;
        }
        if newer.session.is_some() {
            self.session = newer.session;
        }
        if newer.library_order.is_some() {
            self.library_order = newer.library_order;
        }
        for header in newer.headers {
            self.headers.retain(|h| h.id != header.id);
            self.headers.push(header);
        }
        for (id, order) in newer.track_order {
            let present: std::collections::HashSet<_> = order.iter().copied().collect();
            self.tracks
                .retain(|t| t.playlist != id || present.contains(&t.track.library_id));
            self.track_order.retain(|(old, _)| *old != id);
            self.track_order.push((id, order));
        }
        for track in newer.tracks {
            self.tracks.retain(|t| {
                (t.playlist, t.track.library_id) != (track.playlist, track.track.library_id)
            });
            self.tracks.push(track);
        }
    }
    pub fn between(old: Option<&State>, new: &State) -> Self {
        let settings = Settings::from(new);
        let order: Vec<_> = new.playlists.iter().map(|p| p.id).collect();
        let mut changes = Self {
            settings: old
                .is_none_or(|s| Settings::from(s) != settings)
                .then_some(settings),
            session: old
                .is_none_or(|s| s.session != new.session)
                .then(|| new.session.clone()),
            library_order: old
                .is_none_or(|s| s.playlists.iter().map(|p| p.id).ne(order.iter().copied()))
                .then_some(order),
            ..Self::default()
        };
        for p in &new.playlists {
            let previous = old.and_then(|s| s.playlists.iter().find(|old| old.id == p.id));
            let header = PlaylistHeader::from(p);
            if previous.is_none_or(|p| PlaylistHeader::from(p) != header) {
                changes.headers.push(header);
            }
            let previous_tracks: std::collections::HashMap<_, _> = previous
                .into_iter()
                .flat_map(|p| p.tracks.iter())
                .map(|t| (t.library_id, t))
                .collect();
            for track in &p.tracks {
                if previous_tracks.get(&track.library_id).copied() != Some(track) {
                    changes.tracks.push(TrackChange {
                        playlist: p.id,
                        track: track.clone(),
                    });
                }
            }
            let order: Vec<_> = p.tracks.iter().map(|t| t.library_id).collect();
            if previous.is_none_or(|p| {
                p.tracks
                    .iter()
                    .map(|t| t.library_id)
                    .ne(order.iter().copied())
            }) {
                changes.track_order.push((p.id, order));
            }
        }
        changes
    }
}

#[cfg(test)]
mod tests;
