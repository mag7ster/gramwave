use crate::model::Track;
use gramwave_domain::playback::QueueRangeEdit;
use gramwave_domain::{QueueEntryId, RepeatMode};
#[derive(Clone, Copy)]
pub enum PlaylistStart {
    Ordered,
    Shuffled,
    Track(u64),
}

pub enum AppCommand {
    Connect,
    Retry,
    StartSession(Vec<Track>, usize),
    StartPlaylist {
        chat: i64,
        start: PlaylistStart,
        order: gramwave_domain::PlaylistOrder,
    },
    ReverseQueue,
    LoadOldest {
        chat: i64,
        retry: bool,
    },
    CancelOldestStart(i64),
    RetryQueue,
    Authenticate(crate::telegram::AuthRequest),
    SaveTrack(Track),
    SaveTracks(Vec<Track>),
    SavePlaylist(i64, [bool; 3]),
    RemoveOffline(gramwave_domain::TrackId, String),
    CancelOffline,
    RemoveOfflinePlaylists(Vec<i64>),
    PruneAudioCache,
    ClearAudioCache(Option<i64>),
    Settings(crate::persistence::Settings),
    Filters(i64, [bool; 3]),
    Insert(Track, bool),
    InsertTracks(Vec<Track>, bool),
    EditQueueRange {
        start: usize,
        expected: Vec<QueueEntryId>,
        edit: QueueRangeEdit,
    },
    MoveQueueRow {
        from: usize,
        entry_id: QueueEntryId,
        to: usize,
    },
    Remove(usize),
    RemoveQueueRows {
        expected: Vec<(usize, QueueEntryId)>,
        now: std::time::Instant,
    },
    UndoQueueRemoval {
        id: u64,
        now: std::time::Instant,
    },
    Clear,
    Stop,
    Quit,
    Next,
    Advance(bool),
    ToggleChatPin(i64),
    Previous,
    Pause(bool),
    Volume(f32),
    Shuffle(bool),
    Repeat(RepeatMode),
    Seek(f64),
    Play(Track),
    OpenPlaylist(i64),
    OpenSearchPlaylist(i64),
    RemoveRecentChat(i64),
    ClearRecentChats,
    SelectPlaylistRow(usize),
    PlayQueue(QueueEntryId),
    PlayQueueRow {
        index: usize,
        entry_id: QueueEntryId,
    },
}
