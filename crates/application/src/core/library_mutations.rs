use crate::{
    core::LibraryPatch,
    library::LibraryCore,
    model::{Playlist, State},
    persistence::{ChangeSet, PlaylistHeader, Settings, TrackChange},
};
use std::collections::{HashMap, HashSet};

pub(crate) enum LibraryMutation {
    MergeChat(Playlist),
    Page(Playlist, HashSet<i64>),
    Patch(LibraryPatch, HashSet<i64>),
    Filters(i64, [bool; 3]),
    Recover(i64),
    Delete(i64, Vec<i64>),
    TogglePin(i64),
}
impl LibraryCore {
    /// Mutate one library record and describe only the affected records.
    /// The persistent State remains the application's read model, not a second copy.
    pub(crate) fn reduce_library(
        &mut self,
        state: &mut State,
        command: LibraryMutation,
    ) -> ChangeSet {
        let chat = match &command {
            LibraryMutation::MergeChat(p) | LibraryMutation::Page(p, _) => p.id,
            LibraryMutation::Patch(p, _) => p.chat,
            LibraryMutation::Filters(id, _)
            | LibraryMutation::Recover(id)
            | LibraryMutation::Delete(id, _) => *id,
            LibraryMutation::TogglePin(id) => {
                state.toggle_chat_pin(*id);
                return ChangeSet {
                    settings: Some(Settings::from(&*state)),
                    ..Default::default()
                };
            }
        };
        let before = state.playlists.iter().find(|p| p.id == chat).cloned();
        match command {
            LibraryMutation::MergeChat(playlist) => {
                if let Some(old) = state.playlists.iter_mut().find(|p| p.id == chat) {
                    old.title = playlist.title;
                } else {
                    state.playlists.push(playlist);
                }
            }
            LibraryMutation::Page(playlist, protected) => {
                if let Some(old) = state.playlists.iter_mut().find(|p| p.id == chat) {
                    old.merge_tracks(playlist.tracks, &protected);
                    old.cursors = playlist.cursors;
                    old.done = playlist.done;
                    if !playlist.url.is_empty() {
                        old.url = playlist.url;
                    }
                } else {
                    state.playlists.push(playlist);
                }
            }
            LibraryMutation::Patch(patch, protected) => {
                if let Some(old) = state.playlists.iter_mut().find(|p| p.id == chat) {
                    patch.apply(old, &protected);
                }
            }
            LibraryMutation::Filters(_, filters) => {
                if let Some(old) = state.playlists.iter_mut().find(|p| p.id == chat) {
                    old.filters = filters;
                }
            }
            LibraryMutation::Recover(_) => {
                if let Some(old) = state.playlists.iter_mut().find(|p| p.id == chat) {
                    old.recover_empty_index();
                }
            }
            LibraryMutation::Delete(_, messages) => {
                let messages: HashSet<_> = messages.into_iter().collect();
                if let Some(old) = state.playlists.iter_mut().find(|p| p.id == chat) {
                    old.tracks.retain(|t| !messages.contains(&t.message));
                }
            }
            LibraryMutation::TogglePin(_) => unreachable!(),
        }
        let Some(after) = state.playlists.iter_mut().find(|p| p.id == chat) else {
            return ChangeSet::default();
        };
        after.ensure_entry_ids();
        let mut changes = playlist_changes(before.as_ref(), after);
        if before.is_none() {
            changes.library_order = Some(state.playlists.iter().map(|p| p.id).collect());
        }
        if !changes.is_empty() {
            self.changed();
        }
        changes
    }
}
fn playlist_changes(before: Option<&Playlist>, after: &Playlist) -> ChangeSet {
    let mut changes = ChangeSet::default();
    let header = PlaylistHeader::from(after);
    if before.is_none_or(|p| PlaylistHeader::from(p) != header) {
        changes.headers.push(header);
    }
    let previous: HashMap<_, _> = before
        .into_iter()
        .flat_map(|p| p.tracks.iter())
        .map(|t| (t.library_id, t))
        .collect();
    for track in &after.tracks {
        if previous.get(&track.library_id).copied() != Some(track) {
            changes.tracks.push(TrackChange {
                playlist: after.id,
                track: track.clone(),
            });
        }
    }
    let order: Vec<_> = after.tracks.iter().map(|t| t.library_id).collect();
    if before.is_none_or(|p| {
        p.tracks
            .iter()
            .map(|t| t.library_id)
            .ne(order.iter().copied())
    }) {
        changes.track_order.push((after.id, order));
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::{AppCore, CoreCommand},
        model::Track,
    };
    fn playlist(id: i64) -> Playlist {
        Playlist {
            id,
            title: "Original".into(),
            tracks: (1..=100)
                .map(|message| Track {
                    chat: id,
                    message,
                    title: format!("Track {message}"),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }
    #[test]
    fn one_track_edit_produces_no_library_or_track_order_rewrites() {
        let mut core = AppCore::new(State {
            playlists: vec![playlist(1), playlist(2)],
            ..Default::default()
        });
        let original_id = core.state.playlists[0].tracks[49].library_id;
        core.handle(CoreCommand::Patch(
            LibraryPatch {
                chat: 1,
                tracks: vec![Track {
                    chat: 1,
                    message: 50,
                    title: "Edited".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            HashSet::new(),
        ));
        let delta = core.take_changes();
        // A page merge sorts a previously unsorted fixture. A later edit must
        // update one stable occurrence without rewriting either ordering table.
        assert_eq!(delta.tracks.len(), 1);
        assert_eq!(delta.tracks[0].track.library_id, original_id);
        core.handle(CoreCommand::Patch(
            LibraryPatch {
                chat: 1,
                tracks: vec![Track {
                    chat: 1,
                    message: 50,
                    title: "Edited twice".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            HashSet::new(),
        ));
        let delta = core.take_changes();
        assert_eq!(delta.tracks.len(), 1);
        assert!(
            delta.library_order.is_none()
                && delta.track_order.is_empty()
                && delta.headers.is_empty()
        );
        assert!(delta.settings.is_none() && delta.session.is_none() && delta.offline.is_none());
        assert_eq!(core.state.playlists[1].title, "Original");
    }
    #[test]
    fn new_playlist_records_order_once_and_noop_recovery_does_not_invalidate_cache() {
        let mut core = AppCore::new(State::default());
        core.handle(CoreCommand::MergeChat(playlist(1)));
        assert_eq!(core.take_changes().library_order, Some(vec![1]));
        let version = core.library_version();
        core.handle(CoreCommand::Recover(1));
        assert!(core.take_changes().is_empty());
        assert_eq!(core.library_version(), version);
    }
}
