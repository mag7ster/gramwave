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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrackMetadata {
    pub chat: i64,
    pub message: i64,
    pub title: String,
    pub artist: String,
    pub duration: u64,
    pub kind: MediaKind,
}
impl TrackMetadata {
    pub fn id(&self) -> TrackId {
        TrackId {
            chat: self.chat,
            message: self.message,
        }
    }
}
pub type Track = TrackMetadata;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub entry_id: QueueEntryId,
    pub track: TrackMetadata,
}
impl std::ops::Deref for QueueEntry {
    type Target = TrackMetadata;
    fn deref(&self) -> &Self::Target {
        &self.track
    }
}
