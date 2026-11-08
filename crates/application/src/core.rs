use crate::{
    model::{Playlist, State, Track},
    persistence::{ChangeSet, Settings},
};
use std::collections::HashSet;
use tg_player_domain::{QueueEntryId, RepeatMode};

#[derive(Default)]
pub struct LibraryPatch {
    pub chat: i64,
    pub tracks: Vec<Track>,
    pub removed: Vec<i64>,
    pub url: Option<String>,
    pub pagination: [Option<(i64, bool)>; 3],
}
impl LibraryPatch {
    pub fn apply(self, p: &mut Playlist, protected: &HashSet<i64>) {
        p.tracks
            .retain(|t| protected.contains(&t.message) || !self.removed.contains(&t.message));
        p.merge_tracks(self.tracks, protected);
        if let Some(url) = self.url {
            p.url = url;
        }
        for (kind, value) in self.pagination.into_iter().enumerate() {
            if let Some((cursor, done)) = value {
                p.cursors[kind] = cursor;
                p.done[kind] = done;
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
    Move(usize, usize),
    Remove(usize),
    Clear,
    Jump(QueueEntryId),
    Advance(bool),
    Previous,
    Stop,
    Current(Track),
    MergeChat(Playlist),
    Page(Playlist, HashSet<i64>),
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
    pub fn library_version(&self) -> u64 {
        self.library.version()
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
            CoreCommand::Page(p, protected) => {
                return self.reduce_library(Library::Page(p, protected))
            }
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
            CoreCommand::Move(from, to) => Playback::Move(from, to),
            CoreCommand::Remove(index) => Playback::Remove(index),
            CoreCommand::Clear => Playback::Clear,
            CoreCommand::Jump(id) => Playback::Jump(id),
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
