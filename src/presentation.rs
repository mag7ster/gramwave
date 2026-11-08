//! Desktop-only row model. Persistent application metadata never contains these file paths.
use tg_player_application::{media::MediaTrack, model::Track as Metadata};
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Track {
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
    pub pinned: Option<String>,
}
impl Track {
    pub fn id(&self) -> tg_player_domain::TrackId {
        tg_player_domain::TrackId {
            chat: self.chat,
            message: self.message,
        }
    }
    pub fn metadata(&self) -> Metadata {
        self.clone().into()
    }
    pub fn transport(self) -> MediaTrack {
        MediaTrack {
            library_id: self.library_id,
            entry_id: self.entry_id,
            chat: self.chat,
            message: self.message,
            title: self.title,
            artist: self.artist,
            duration: self.duration,
            kind: self.kind,
            file: self.file,
            cached: self.cached,
            image_file: self.image_file,
            image_path: self.image_path,
        }
    }
}
impl From<MediaTrack> for Track {
    fn from(t: MediaTrack) -> Self {
        Self {
            library_id: t.library_id,
            entry_id: t.entry_id,
            chat: t.chat,
            message: t.message,
            title: t.title,
            artist: t.artist,
            duration: t.duration,
            kind: t.kind,
            file: t.file,
            cached: t.cached,
            image_file: t.image_file,
            image_path: t.image_path,
            pinned: None,
        }
    }
}
impl From<Metadata> for Track {
    fn from(t: Metadata) -> Self {
        MediaTrack::from(t).into()
    }
}
impl From<Track> for Metadata {
    fn from(t: Track) -> Self {
        t.transport().into()
    }
}
impl From<tg_player_domain::QueueEntry> for Track {
    fn from(t: tg_player_domain::QueueEntry) -> Self {
        Metadata::from(t).into()
    }
}
impl From<&tg_player_domain::QueueEntry> for Track {
    fn from(t: &tg_player_domain::QueueEntry) -> Self {
        Metadata::from(t).into()
    }
}
