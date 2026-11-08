use crate::{
    model::{State, Track},
    persistence::{ChangeSet, Settings},
    ports::PlayerSnapshot,
    tasks::{Operation, RequestId},
    AppError,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Loading {
        paused: bool,
    },
    Playing,
    Paused,
    Failed,
}
impl PlaybackState {
    pub fn loading(self) -> bool {
        matches!(self, Self::Loading { .. })
    }
    pub fn paused(self) -> bool {
        matches!(
            self,
            Self::Stopped | Self::Paused | Self::Failed | Self::Loading { paused: true }
        )
    }
    pub fn set_loading(&mut self, loading: bool) {
        if loading {
            *self = Self::Loading {
                paused: self.paused(),
            };
        } else if let Self::Loading { paused } = *self {
            *self = if paused { Self::Paused } else { Self::Playing };
        }
    }
    pub fn set_paused(&mut self, paused: bool) {
        *self = if self.loading() {
            Self::Loading { paused }
        } else if paused {
            Self::Paused
        } else {
            Self::Playing
        };
    }
}

#[derive(Default)]
pub struct PlaybackCore {
    pub(crate) state: PlaybackState,
    pub(crate) generation: u64,
    pub(crate) active: Option<Operation>,
    pub(crate) target: Option<Track>,
    pub(crate) revision: u64,
    pub(crate) snapshot: PlayerSnapshot,
}
impl PlaybackCore {
    pub fn state(&self) -> PlaybackState {
        self.state
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub(crate) fn begin(&mut self, track: Track, revision: u64) -> Operation {
        self.cancel();
        self.state = PlaybackState::Loading { paused: false };
        self.target = Some(track);
        self.revision = revision;
        let operation = Operation::new(self.generation);
        self.active = Some(operation.clone());
        operation
    }
    pub(crate) fn accepts(&self, key: RequestId) -> bool {
        self.active.as_ref().is_some_and(|op| op.key() == key)
    }
    pub(crate) fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(op) = self.active.take() {
            op.cancel.cancel();
        }
    }
    pub(crate) fn fail(&mut self, error: AppError) -> super::workflow::Effect {
        self.cancel();
        self.state = PlaybackState::Failed;
        super::workflow::Effect::Error(error)
    }
}

pub(crate) enum PlaybackMutation {
    Volume(f32),
    Shuffle(bool),
    Repeat(tg_player_domain::RepeatMode),
    Start(Vec<Track>, usize),
    Insert(Track, bool),
    Move(usize, usize),
    Remove(usize),
    Clear,
    Jump(tg_player_domain::QueueEntryId),
    Advance(bool),
    Previous,
    Stop,
    Current(Track),
}
impl PlaybackCore {
    /// Apply queue and playback preferences to the persistent read model.
    /// Domain Session owns occurrence IDs, ordering and navigation invariants.
    pub(crate) fn reduce_playback(
        &mut self,
        state: &mut State,
        command: PlaybackMutation,
    ) -> (Option<Track>, ChangeSet) {
        let session = !matches!(
            &command,
            PlaybackMutation::Volume(_) | PlaybackMutation::Repeat(_)
        );
        let settings = matches!(
            &command,
            PlaybackMutation::Volume(_)
                | PlaybackMutation::Shuffle(_)
                | PlaybackMutation::Repeat(_)
                | PlaybackMutation::Clear
        );
        let mut play = None;
        match command {
            PlaybackMutation::Volume(value) => {
                if !value.is_finite() {
                    return (None, ChangeSet::default());
                }
                state.volume = value.clamp(0., 100.);
                if state.volume > 0. {
                    state.unmuted_volume = state.volume;
                }
            }
            PlaybackMutation::Shuffle(value) => {
                state.shuffle = value;
                state.session.reorder(value);
            }
            PlaybackMutation::Repeat(value) => state.repeat = value,
            PlaybackMutation::Start(tracks, index) => {
                play = state
                    .session
                    .start(&tracks, index, state.shuffle)
                    .map(Into::into)
            }
            PlaybackMutation::Insert(track, next) => state.session.insert(track, next),
            PlaybackMutation::Move(from, to) => state.session.move_before(from, to),
            PlaybackMutation::Remove(index) => state.session.remove(index),
            PlaybackMutation::Clear => {
                state.session.clear();
                state.repeat = tg_player_domain::RepeatMode::Off;
            }
            PlaybackMutation::Jump(id) => {
                let index = state
                    .session
                    .remaining()
                    .iter()
                    .position(|t| t.entry_id == id);
                play = index
                    .and_then(|index| state.session.jump_to(index))
                    .map(Into::into);
            }
            PlaybackMutation::Advance(eof) => {
                play = state
                    .session
                    .advance(state.repeat, state.shuffle, eof)
                    .map(Into::into)
            }
            PlaybackMutation::Previous => play = state.session.previous().map(Into::into),
            PlaybackMutation::Stop => state.session.stop(),
            PlaybackMutation::Current(track) => {
                let id = track.entry_id;
                state.session.set_current(track.into(), id);
            }
        }
        let changes = ChangeSet {
            settings: settings.then(|| Settings::from(&*state)),
            session: session.then(|| state.session.clone()),
            ..Default::default()
        };
        (play, changes)
    }
}

#[cfg(test)]
mod mutation_tests {
    use super::*;
    use crate::core::{AppCore, CoreCommand};
    #[test]
    fn playback_preferences_persist_without_library_or_offline_snapshots() {
        let mut core = AppCore::new(State::default());
        core.handle(CoreCommand::Volume(42.));
        let delta = core.take_changes();
        assert_eq!(delta.settings.unwrap().volume, 42.);
        assert!(
            delta.session.is_none() && delta.offline.is_none() && delta.library_order.is_none()
        );
        assert!(
            delta.headers.is_empty() && delta.tracks.is_empty() && delta.track_order.is_empty()
        );
        core.handle(CoreCommand::Volume(f32::NAN));
        assert!(core.take_changes().is_empty());
        core.handle(CoreCommand::Start(
            vec![
                Track {
                    chat: 42,
                    message: 7,
                    ..Default::default()
                };
                3
            ],
            1,
        ));
        let delta = core.take_changes();
        let session = delta.session.unwrap();
        assert_eq!(session.snapshot().len(), 3);
        assert!(delta.settings.is_none() && delta.tracks.is_empty() && delta.headers.is_empty());
        core.handle(CoreCommand::Shuffle(true));
        let delta = core.take_changes();
        assert!(delta.settings.unwrap().shuffle);
        assert!(delta.session.is_some());
        assert!(delta.tracks.is_empty() && delta.headers.is_empty() && delta.offline.is_none());
    }
}
