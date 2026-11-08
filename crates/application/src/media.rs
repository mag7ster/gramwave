//! Typed transient transport records. Persistent library and queue metadata live in model.
use crate::{
    core::LibraryPatch,
    model::{Playlist, Track},
};
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuntimeMedia {
    pub file: i64,
    pub cached: Option<String>,
    pub image_file: i64,
    pub image_path: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuntimeArtwork {
    pub file: i64,
    pub path: Option<String>,
}
/// One gateway result: metadata plus connection-local TDLib media references.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaTrack {
    pub library_id: u64,
    pub entry_id: tg_player_domain::QueueEntryId,
    pub chat: i64,
    pub message: i64,
    pub title: String,
    pub artist: String,
    pub duration: u64,
    pub kind: tg_player_domain::MediaKind,
    pub file: i64,
    pub cached: Option<String>,
    pub image_file: i64,
    pub image_path: Option<String>,
}
impl MediaTrack {
    pub fn id(&self) -> tg_player_domain::TrackId {
        tg_player_domain::TrackId {
            chat: self.chat,
            message: self.message,
        }
    }
    pub fn metadata(&self) -> Track {
        self.clone().into()
    }
    pub fn runtime(&self) -> RuntimeMedia {
        RuntimeMedia {
            file: self.file,
            cached: self.cached.clone(),
            image_file: self.image_file,
            image_path: self.image_path.clone(),
        }
    }
}
impl From<Track> for MediaTrack {
    fn from(track: Track) -> Self {
        Self {
            library_id: track.library_id,
            entry_id: track.entry_id,
            chat: track.chat,
            message: track.message,
            title: track.title,
            artist: track.artist,
            duration: track.duration,
            kind: track.kind,
            ..Default::default()
        }
    }
}
impl From<MediaTrack> for Track {
    fn from(track: MediaTrack) -> Self {
        Self {
            library_id: track.library_id,
            entry_id: track.entry_id,
            chat: track.chat,
            message: track.message,
            title: track.title,
            artist: track.artist,
            duration: track.duration,
            kind: track.kind,
        }
    }
}
/// A fetched playlist page; runtime artwork and media never enter Playlist serialization.
#[derive(Clone, Debug, Default)]
pub struct MediaPlaylist {
    pub entry_serial: u64,
    pub id: i64,
    pub title: String,
    pub url: String,
    pub image_file: i64,
    pub image_path: Option<String>,
    pub tracks: Vec<MediaTrack>,
    pub filters: [bool; 3],
    pub pinned: bool,
    pub cursors: [i64; 3],
    pub done: [bool; 3],
}
impl MediaPlaylist {
    pub fn artwork(&self) -> RuntimeArtwork {
        RuntimeArtwork {
            file: self.image_file,
            path: self.image_path.clone(),
        }
    }
}
impl From<Playlist> for MediaPlaylist {
    fn from(p: Playlist) -> Self {
        Self {
            entry_serial: p.entry_serial,
            id: p.id,
            title: p.title,
            url: p.url,
            tracks: p.tracks.into_iter().map(Into::into).collect(),
            filters: p.filters,
            pinned: p.pinned,
            cursors: p.cursors,
            done: p.done,
            ..Default::default()
        }
    }
}
impl From<MediaPlaylist> for Playlist {
    fn from(p: MediaPlaylist) -> Self {
        Self {
            entry_serial: p.entry_serial,
            id: p.id,
            title: p.title,
            url: p.url,
            tracks: p.tracks.into_iter().map(Into::into).collect(),
            filters: p.filters,
            pinned: p.pinned,
            cursors: p.cursors,
            done: p.done,
        }
    }
}
#[derive(Default)]
pub struct MediaPatch {
    pub chat: i64,
    pub tracks: Vec<MediaTrack>,
    pub removed: Vec<i64>,
    pub url: Option<String>,
    pub pagination: [Option<(i64, bool)>; 3],
}
impl From<LibraryPatch> for MediaPatch {
    fn from(p: LibraryPatch) -> Self {
        Self {
            chat: p.chat,
            tracks: p.tracks.into_iter().map(Into::into).collect(),
            removed: p.removed,
            url: p.url,
            pagination: p.pagination,
        }
    }
}
impl From<MediaPatch> for LibraryPatch {
    fn from(p: MediaPatch) -> Self {
        Self {
            chat: p.chat,
            tracks: p.tracks.into_iter().map(Into::into).collect(),
            removed: p.removed,
            url: p.url,
            pagination: p.pagination,
        }
    }
}
