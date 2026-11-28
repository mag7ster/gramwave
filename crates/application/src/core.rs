use crate::{
    model::{Playlist, State, Track},
    persistence::{ChangeSet, Settings},
};
use gramwave_domain::playback::QueueRangeEdit;
use gramwave_domain::{QueueEntryId, RepeatMode};
use std::collections::HashSet;

#[derive(Default)]
pub struct LibraryPatch {
    pub chat: i64,
    pub tracks: Vec<Track>,
    pub removed: Vec<i64>,
    pub url: Option<String>,
    pub pagination: [Option<(i64, bool)>; 3],
    pub oldest_page: Option<(usize, i64)>,
}
impl LibraryPatch {
    pub fn apply(self, p: &mut Playlist, protected: &HashSet<i64>) {
        p.tracks
            .retain(|t| protected.contains(&t.message) || !self.removed.contains(&t.message));
        p.merge_tracks(self.tracks, protected);
        if let Some(url) = self.url {
            p.url = url;
        }
        if let Some((kind, cursor)) = self.oldest_page {
            p.oldest_cursors[kind] = p.oldest_cursors[kind].max(cursor);
            if cursor == 0 || (p.cursors[kind] > 0 && cursor >= p.cursors[kind]) {
                p.done[kind] = true;
            }
        }
        for (kind, value) in self.pagination.into_iter().enumerate() {
            if let Some((cursor, done)) = value.filter(|_| !p.done[kind]) {
                p.cursors[kind] = cursor;
                p.done[kind] = done || (cursor > 0 && p.oldest_cursors[kind] >= cursor);
            }
        }
    }
}
pub(crate) enum CoreCommand {
    Settings(Settings),
    Volume(f32),
    Shuffle(bool),
    Repeat(RepeatMode),
    ToggleChatPin(i64),
    Start(Vec<Track>, usize),
    Insert(Track, bool),
    InsertTracks(Vec<Track>, bool),
    EditQueueRange(usize, Vec<QueueEntryId>, QueueRangeEdit),
    MoveQueueRow(usize, QueueEntryId, usize),
    Remove(usize),
    Clear,
    Jump(QueueEntryId),
    PlayQueueRow(usize, QueueEntryId),
    Advance(bool),
    Previous,
    Stop,
    Current(Track),
    MergeChat(Playlist),
    Patch(LibraryPatch, HashSet<i64>),
    Filters(i64, [bool; 3]),
    Recover(i64),
    Delete(i64, Vec<i64>),
}
/// Main-thread reducer. UI receives shared views and sends commands; adapters receive deltas.
pub struct AppCore {
    pub(crate) offline: crate::offline::OfflineCore,
    pub(crate) auth: crate::auth::AuthCore,
    pub(crate) playback: crate::playback::PlaybackCore,
    pub(crate) shuffle_work: crate::shuffle_work::ShuffleWork,
    pub(crate) page_browse: crate::page_browse::PageBrowse,
    pub(crate) queue_work: crate::queue_work::QueueWork,
    pub(crate) library: crate::library::LibraryCore,
    pub(crate) state: State,
    pending: ChangeSet,
}
impl AppCore {
    pub fn new(mut state: State) -> Self {
        for p in &mut state.playlists {
            p.ensure_entry_ids();
        }
        Self {
            state,
            auth: Default::default(),
            playback: Default::default(),
            queue_work: Default::default(),
            page_browse: Default::default(),
            shuffle_work: Default::default(),
            library: Default::default(),
            offline: crate::offline::OfflineCore::new(Default::default()),
            pending: Default::default(),
        }
    }
    pub fn with_offline(state: State, data: crate::offline::OfflineData) -> Self {
        let mut core = Self::new(state);
        core.offline = crate::offline::OfflineCore::new(data);
        core
    }
    pub fn offline(&self) -> &crate::offline::OfflineCore {
        &self.offline
    }
    pub fn view(&self) -> &State {
        &self.state
    }
    pub fn save_playback_position(&mut self, snapshot: crate::ports::PlayerSnapshot) {
        use crate::playback::PlaybackState;
        let Some(current) = self.state.session.current() else {
            return;
        };
        if !matches!(
            self.playback.state,
            PlaybackState::Playing | PlaybackState::Paused
        ) || !self
            .playback
            .target
            .as_ref()
            .is_some_and(|track| track.entry_id == current.entry_id)
        {
            return;
        }
        let Some(position) = snapshot
            .checkpoint_position
            .filter(|position| position.is_finite() && *position >= 0. && !snapshot.seeking)
        else {
            return;
        };
        let position_ms = if snapshot.duration > 0. && position >= snapshot.duration {
            0
        } else {
            (position * 1000.).round() as u64
        };
        if position_ms != self.state.session.position_ms() {
            self.state.session.set_position_ms(position_ms);
            self.record_session_change();
        }
    }
    pub(crate) fn record_settings_change(&mut self) {
        self.pending.settings = Some(Settings::from(&self.state));
    }
    pub(crate) fn record_session_change(&mut self) {
        self.pending.session = Some(self.state.session.clone());
    }
    pub fn library_version(&self) -> u64 {
        self.library.version()
    }
    pub(crate) fn set_personal_chat(&mut self, index: usize, value: crate::model::PersonalChat) {
        let playlist = &mut self.state.playlists[index];
        if playlist.personal != value {
            playlist.personal = value;
            self.pending.merge(ChangeSet {
                headers: vec![crate::persistence::PlaylistHeader::from(&*playlist)],
                ..Default::default()
            });
        }
    }
    pub fn take_changes(&mut self) -> ChangeSet {
        if let Some(data) = self.offline.take_changes() {
            if let Some(pending) = &mut self.pending.offline {
                pending.merge(data);
            } else {
                self.pending.offline = Some(data);
            }
        }
        std::mem::take(&mut self.pending)
    }
    pub fn retain_changes(&mut self, mut changes: ChangeSet) {
        changes.merge(std::mem::take(&mut self.pending));
        self.pending = changes;
    }
    pub(crate) fn handle(&mut self, command: CoreCommand) -> Option<Track> {
        use crate::playback::PlaybackMutation as Playback;
        use library_mutations::LibraryMutation as Library;
        let playback = match command {
            CoreCommand::Settings(value) => {
                value.apply(&mut self.state);
                self.pending.settings = Some(Settings::from(&self.state));
                return None;
            }
            CoreCommand::ToggleChatPin(id) => return self.reduce_library(Library::TogglePin(id)),
            CoreCommand::MergeChat(p) => return self.reduce_library(Library::MergeChat(p)),
            CoreCommand::Patch(p, protected) => {
                return self.reduce_library(Library::Patch(p, protected))
            }
            CoreCommand::Filters(id, filters) => {
                return self.reduce_library(Library::Filters(id, filters))
            }
            CoreCommand::Recover(id) => return self.reduce_library(Library::Recover(id)),
            CoreCommand::Delete(id, messages) => {
                return self.reduce_library(Library::Delete(id, messages))
            }
            CoreCommand::Volume(value) => Playback::Volume(value),
            CoreCommand::Shuffle(value) => Playback::Shuffle(value),
            CoreCommand::Repeat(value) => Playback::Repeat(value),
            CoreCommand::Start(tracks, index) => Playback::Start(tracks, index),
            CoreCommand::Insert(track, next) => Playback::Insert(track, next),
            CoreCommand::InsertTracks(tracks, next) => Playback::InsertTracks(tracks, next),
            CoreCommand::EditQueueRange(start, ids, edit) => {
                Playback::EditQueueRange(start, ids, edit)
            }
            CoreCommand::MoveQueueRow(from, id, to) => Playback::MoveQueueRow(from, id, to),
            CoreCommand::Remove(index) => Playback::Remove(index),
            CoreCommand::Clear => Playback::Clear,
            CoreCommand::Jump(id) => Playback::Jump(id),
            CoreCommand::PlayQueueRow(index, id) => Playback::PlayQueueRow(index, id),
            CoreCommand::Advance(eof) => Playback::Advance(eof),
            CoreCommand::Previous => Playback::Previous,
            CoreCommand::Stop => Playback::Stop,
            CoreCommand::Current(track) => Playback::Current(track),
        };
        let (play, changes) = self.playback.reduce_playback(&mut self.state, playback);
        self.pending.merge(changes);
        play
    }
    fn reduce_library(&mut self, command: library_mutations::LibraryMutation) -> Option<Track> {
        let changes = self.library.reduce_library(&mut self.state, command);
        self.pending.merge(changes);
        None
    }
}
mod library_mutations;
