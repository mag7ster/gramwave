use crate::model::Track;
use serde::{Deserialize, Serialize};
use tg_player_domain::TrackId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OfflineCopy {
    pub track: Track,
    pub path: String,
    #[serde(default)]
    pub cached: bool,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub last_used: u64,
    #[serde(default = "pending_handoff")]
    pub handoff_pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_id: Option<String>,
}

fn pending_handoff() -> bool {
    true
}

#[derive(Debug, PartialEq, Eq)]
pub struct PublishedFile {
    pub path: String,
    pub size: u64,
    pub remote_id: Option<String>,
}
impl From<&str> for PublishedFile {
    fn from(path: &str) -> Self {
        Self {
            path: path.into(),
            size: 0,
            remote_id: None,
        }
    }
}

pub fn unix_seconds(time: std::time::SystemTime) -> u64 {
    time.duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl OfflineCopy {
    pub fn fresh(&self, now: u64, days: u32) -> bool {
        self.last_used != 0 && now.saturating_sub(self.last_used) < u64::from(days) * 86_400
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DownloadPolicy {
    pub chat: i64,
    pub filters: [bool; 3],
    pub enabled: bool,
    pub cursors: [i64; 3],
    pub done: [bool; 3],
    #[serde(default)]
    pub head: [HeadScan; 3],
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OfflineData {
    pub copies: Vec<OfflineCopy>,
    pub removing: Vec<OfflineCopy>,
    pub policies: Vec<DownloadPolicy>,
    pub pending: Vec<Track>,
    pub excluded: Vec<TrackId>,
}
impl OfflineData {
    pub fn path(&self, id: TrackId) -> Option<&str> {
        self.copies
            .iter()
            .find(|c| !c.cached && c.track.id() == id)
            .map(|c| c.path.as_str())
    }
}

/// Independent progress for checking the newly added head of a playlist.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HeadScan {
    pub watermark: i64,
    pub cursor: i64,
    pub active: bool,
    pub newest: i64,
}
#[derive(Clone, Debug, Default)]
pub struct OfflineDelta {
    pub copies: std::collections::HashMap<TrackId, Option<OfflineCopy>>,
    pub pending: indexmap::IndexMap<TrackId, Option<Track>>,
    pub policies: indexmap::IndexMap<i64, Option<DownloadPolicy>>,
    pub excluded: std::collections::HashMap<TrackId, bool>,
    pub removing: std::collections::HashMap<String, Option<OfflineCopy>>,
}
impl OfflineDelta {
    pub fn is_empty(&self) -> bool {
        self.copies.is_empty()
            && self.pending.is_empty()
            && self.policies.is_empty()
            && self.excluded.is_empty()
            && self.removing.is_empty()
    }
    pub fn merge(&mut self, newer: Self) {
        self.copies.extend(newer.copies);
        for (id, value) in newer.pending {
            self.pending.shift_remove(&id);
            self.pending.insert(id, value);
        }
        self.policies.extend(newer.policies);
        self.excluded.extend(newer.excluded);
        self.removing.extend(newer.removing);
    }
}
impl From<OfflineData> for OfflineDelta {
    fn from(data: OfflineData) -> Self {
        Self {
            copies: data
                .copies
                .into_iter()
                .map(|c| (c.track.id(), Some(c)))
                .collect(),
            pending: data
                .pending
                .into_iter()
                .map(|t| (t.id(), Some(t)))
                .collect(),
            policies: data
                .policies
                .into_iter()
                .map(|p| (p.chat, Some(p)))
                .collect(),
            excluded: data.excluded.into_iter().map(|id| (id, true)).collect(),
            removing: data
                .removing
                .into_iter()
                .map(|c| (c.path.clone(), Some(c)))
                .collect(),
        }
    }
}
#[derive(Default)]
pub struct OfflineIndex {
    pub copies: indexmap::IndexMap<TrackId, OfflineCopy>,
    pub pending: indexmap::IndexMap<TrackId, Track>,
    pub policies: indexmap::IndexMap<i64, DownloadPolicy>,
    pub excluded: std::collections::HashSet<TrackId>,
    pub removing: indexmap::IndexMap<String, OfflineCopy>,
}
/// Indexed download state; dirty records are proportional to the actual mutation.
pub struct OfflineCore {
    pub(crate) data: OfflineIndex,
    pub(crate) work: crate::offline_work::OfflineWork,
    changes: OfflineDelta,
    version: u64,
}
impl std::ops::Deref for OfflineCore {
    type Target = OfflineIndex;
    fn deref(&self) -> &OfflineIndex {
        &self.data
    }
}
impl OfflineCore {
    pub fn new(data: OfflineData) -> Self {
        Self {
            data: OfflineIndex {
                copies: data.copies.into_iter().map(|c| (c.track.id(), c)).collect(),
                pending: data.pending.into_iter().map(|t| (t.id(), t)).collect(),
                policies: data.policies.into_iter().map(|p| (p.chat, p)).collect(),
                excluded: data.excluded.into_iter().collect(),
                removing: data
                    .removing
                    .into_iter()
                    .map(|c| (c.path.clone(), c))
                    .collect(),
            },
            work: Default::default(),
            changes: Default::default(),
            version: 0,
        }
    }
    fn changed(&mut self) {
        self.version = self.version.wrapping_add(1);
    }
    pub fn data(&self) -> OfflineData {
        OfflineData {
            copies: self.data.copies.values().cloned().collect(),
            pending: self.data.pending.values().cloned().collect(),
            policies: self.data.policies.values().cloned().collect(),
            excluded: self.data.excluded.iter().copied().collect(),
            removing: self.data.removing.values().cloned().collect(),
        }
    }
    pub fn path(&self, id: TrackId) -> Option<&str> {
        self.data
            .copies
            .get(&id)
            .filter(|c| !c.cached)
            .map(|c| c.path.as_str())
    }
    pub fn local_path(&self, id: TrackId, now: u64, days: u32) -> Option<&str> {
        self.data
            .copies
            .get(&id)
            .filter(|c| !c.cached || c.fresh(now, days))
            .map(|c| c.path.as_str())
    }
    pub fn playlist_complete(&self, playlist: &crate::model::Playlist) -> bool {
        let chat = playlist.id;
        let enabled = |track: &Track| playlist.filters[usize::from(track.kind)];
        self.data
            .copies
            .values()
            .any(|c| !c.cached && c.track.chat == chat && enabled(&c.track))
            && playlist.filters.iter().enumerate().all(|(kind, enabled)| {
                !enabled
                    || playlist.done[kind]
                    || self.data.policies.get(&chat).is_some_and(|p| {
                        p.enabled && p.filters[kind] && p.done[kind] && !p.head[kind].active
                    })
            })
            && !self
                .data
                .pending
                .values()
                .any(|t| t.chat == chat && enabled(t))
            && !self.data.excluded.iter().any(|id| {
                id.chat == chat
                    && playlist
                        .tracks
                        .iter()
                        .find(|t| t.id() == *id)
                        .is_none_or(enabled)
            })
            && playlist
                .tracks
                .iter()
                .filter(|t| enabled(t))
                .all(|t| self.path(t.id()).is_some())
    }
    pub(crate) fn mark_used(&mut self, id: TrackId, path: &str, now: u64) {
        if let Some(copy) = self.data.copies.get_mut(&id).filter(|c| c.path == path) {
            copy.last_used = now;
            self.changes.copies.insert(id, Some(copy.clone()));
        }
    }
    pub(crate) fn finish_handoff(&mut self, id: TrackId, path: &str) {
        if let Some(copy) = self.data.copies.get_mut(&id).filter(|c| c.path == path) {
            copy.handoff_pending = false;
            copy.remote_id = None;
            self.changes.copies.insert(id, Some(copy.clone()));
        }
    }
    pub(crate) fn prune_cache(&mut self, now: u64, policy: crate::persistence::CachePolicy) {
        let mut cached: Vec<_> = self.data.copies.values().filter(|c| c.cached).collect();
        cached.sort_by_key(|c| (c.last_used, c.track.chat, c.track.message));
        let mut size = cached.iter().map(|c| c.size).sum::<u64>();
        let limit = u64::from(policy.size_mib) * 1024 * 1024;
        let evict: Vec<_> = cached
            .into_iter()
            .filter_map(|c| {
                if !c.fresh(now, policy.days) || size > limit {
                    size = size.saturating_sub(c.size);
                    Some(c.track.id())
                } else {
                    None
                }
            })
            .collect();
        for id in evict {
            self.discard(id);
        }
    }
    pub(crate) fn clear_cache(&mut self, chat: Option<i64>) {
        let ids: Vec<_> = self
            .data
            .copies
            .values()
            .filter(|c| c.cached && chat.is_none_or(|chat| c.track.chat == chat))
            .map(|c| c.track.id())
            .collect();
        for id in ids {
            self.discard(id);
        }
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn take_changes(&mut self) -> Option<OfflineDelta> {
        (!self.changes.is_empty()).then(|| std::mem::take(&mut self.changes))
    }
    pub fn enqueue(&mut self, track: Track, explicit: bool) {
        let id = track.id();
        if explicit && self.data.excluded.remove(&id) {
            self.changes.excluded.insert(id, false);
        }
        if self.data.excluded.contains(&id) || self.data.pending.contains_key(&id) {
            return;
        }
        if let Some(copy) = self.data.copies.get_mut(&id) {
            if copy.cached {
                copy.cached = false;
                self.changes.copies.insert(id, Some(copy.clone()));
                self.changed();
            }
            return;
        }
        self.data.pending.insert(id, track.clone());
        self.changes.pending.shift_remove(&id);
        self.changes.pending.insert(id, Some(track));
        self.changed();
    }
    pub fn save_playlist(&mut self, chat: i64, filters: [bool; 3]) {
        let excluded: Vec<_> = self
            .data
            .excluded
            .iter()
            .filter(|id| id.chat == chat)
            .copied()
            .collect();
        for id in excluded {
            self.data.excluded.remove(&id);
            self.changes.excluded.insert(id, false);
        }
        let p = DownloadPolicy {
            chat,
            filters,
            enabled: true,
            cursors: [0; 3],
            done: filters.map(|f| !f),
            head: Default::default(),
        };
        self.data.policies.insert(chat, p.clone());
        self.changes.policies.insert(chat, Some(p));
        self.changed();
    }
    pub fn publish(&mut self, track: Track, path: String) {
        self.publish_file(
            track,
            PublishedFile {
                path,
                size: 0,
                remote_id: None,
            },
            0,
        );
    }
    pub(crate) fn publish_file(&mut self, track: Track, file: PublishedFile, now: u64) {
        let id = track.id();
        self.data.pending.shift_remove(&id);
        self.changes.pending.insert(id, None);
        let copy = OfflineCopy {
            track,
            path: file.path,
            cached: false,
            size: file.size,
            last_used: now,
            handoff_pending: file.size != 0,
            remote_id: file.remote_id,
        };
        self.data.copies.insert(id, copy.clone());
        self.changes.copies.insert(id, Some(copy));
        self.changed();
    }
    /// Stale publication cleanup is durable work, just like a user-requested removal.
    pub(crate) fn queue_removal(&mut self, path: String) {
        if self.data.removing.contains_key(&path)
            || self.data.copies.values().any(|copy| copy.path == path)
        {
            return;
        }
        let copy = OfflineCopy {
            track: Track::default(),
            path: path.clone(),
            cached: false,
            size: 0,
            last_used: 0,
            handoff_pending: false,
            remote_id: None,
        };
        self.data.removing.insert(path.clone(), copy.clone());
        self.changes.removing.insert(path, Some(copy));
        self.changed();
    }
    pub fn finish_removal(&mut self, path: &str) {
        self.data.removing.swap_remove(path);
        self.changes.removing.insert(path.into(), None);
        self.changed();
    }
    pub fn exclude(&mut self, id: TrackId) {
        self.source_changed(id);
        self.data.excluded.insert(id);
        self.changes.excluded.insert(id, true);
        self.changed();
    }
    pub(crate) fn unpin(&mut self, id: TrackId, now: u64, days: u32) {
        if let Some(copy) = self.data.copies.get_mut(&id).filter(|c| c.fresh(now, days)) {
            copy.cached = true;
            self.changes.copies.insert(id, Some(copy.clone()));
        } else {
            self.discard(id);
        }
        self.exclude(id);
    }
    pub fn remove(&mut self, id: TrackId) {
        self.discard(id);
        self.exclude(id);
    }
    pub(crate) fn discard(&mut self, id: TrackId) {
        if let Some(copy) = self.data.copies.swap_remove(&id) {
            self.data.removing.insert(copy.path.clone(), copy.clone());
            self.changes.removing.insert(copy.path.clone(), Some(copy));
            self.changes.copies.insert(id, None);
            self.changed();
        }
    }
    pub(crate) fn remove_playlists(&mut self, chats: &[i64], now: u64, days: u32) {
        let selected = |chat| chats.contains(&chat);
        if self
            .work
            .scan
            .as_ref()
            .is_some_and(|(_, chat, _)| selected(*chat))
        {
            self.cancel_scan();
            self.work.scan_retry = None;
        }
        let ids: Vec<_> = self
            .data
            .pending
            .keys()
            .copied()
            .filter(|id| selected(id.chat))
            .collect();
        for id in ids {
            self.cancel_attempt(id);
            self.source_changed(id);
        }
        self.work.retry.retain(|id, _| !selected(id.chat));
        self.work.errors.retain(|(chat, _), _| !selected(*chat));
        for &chat in chats {
            if self.data.policies.shift_remove(&chat).is_some() {
                self.changes.policies.insert(chat, None);
            }
        }
        let ids: Vec<_> = self
            .data
            .copies
            .values()
            .filter(|c| !c.cached && selected(c.track.chat))
            .map(|c| c.track.id())
            .collect();
        for id in ids {
            self.unpin(id, now, days);
        }
        self.changed();
    }
    pub fn cancel(&mut self) {
        for id in self.data.pending.keys() {
            self.changes.pending.insert(*id, None);
        }
        self.data.pending.clear();
        for p in self.data.policies.values_mut() {
            p.enabled = false;
            self.changes.policies.insert(p.chat, Some(p.clone()));
        }
        self.changed();
    }
    pub fn next_scan(&self) -> Option<(i64, usize, i64)> {
        for head in [true, false] {
            for p in self.data.policies.values().filter(|p| p.enabled) {
                for kind in 0..3 {
                    if p.filters[kind]
                        && if head {
                            p.head[kind].active
                        } else {
                            !p.done[kind]
                        }
                    {
                        return Some((
                            p.chat,
                            kind,
                            if head {
                                p.head[kind].cursor
                            } else {
                                p.cursors[kind]
                            },
                        ));
                    }
                }
            }
        }
        None
    }
    pub fn page(&mut self, chat: i64, kind: usize, next: i64, tracks: Vec<Track>) {
        let newest = tracks.iter().map(|t| t.message).max().unwrap_or(0);
        self.scan_page(
            chat,
            kind,
            crate::telegram::MediaPage {
                tracks: tracks.into_iter().map(Into::into).collect(),
                next,
                newest,
            },
        );
    }
    pub fn scan_page(&mut self, chat: i64, kind: usize, page: crate::telegram::MediaPage) {
        let crate::telegram::MediaPage {
            mut tracks,
            next,
            newest,
        } = page;
        let Some(policy) = self.data.policies.get_mut(&chat).filter(|p| p.enabled) else {
            return;
        };
        let head = &mut policy.head[kind];
        if head.active {
            head.newest = head.newest.max(newest);
            let reached = next == 0
                || (head.watermark > 0
                    && (next <= head.watermark
                        || tracks.iter().any(|t| t.message <= head.watermark)));
            tracks.retain(|t| t.message > head.watermark);
            head.cursor = next;
            if reached {
                head.active = false;
                head.cursor = 0;
                head.watermark = head.watermark.max(head.newest);
                head.newest = 0;
            }
        } else {
            policy.cursors[kind] = next;
            policy.done[kind] = next == 0;
            if head.watermark == 0 {
                head.watermark = newest;
            }
        }
        self.changes.policies.insert(chat, Some(policy.clone()));
        for track in tracks {
            self.enqueue(track.into(), false);
        }
        self.changed();
    }
    pub fn reconnect(&mut self) {
        for p in self.data.policies.values_mut().filter(|p| p.enabled) {
            for kind in 0..3 {
                if p.filters[kind] && !p.head[kind].active {
                    p.head[kind].active = true;
                    p.head[kind].cursor = 0;
                    p.head[kind].newest = 0;
                }
            }
            self.changes.policies.insert(p.chat, Some(p.clone()));
        }
        self.changed();
    }
    pub fn source_changed(&mut self, id: TrackId) {
        if self.data.pending.shift_remove(&id).is_some() {
            self.changes.pending.insert(id, None);
            self.changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exclusions_explicit_saving_restart_and_cancel_preserve_ready_copies() {
        let track = Track {
            chat: 42,
            message: 7,
            ..Default::default()
        };
        let mut core = OfflineCore::new(OfflineData::default());
        core.save_playlist(42, [true, false, false]);
        core.page(42, 0, 99, vec![track.clone(), track.clone()]);
        assert_eq!(core.pending.len(), 1);
        let data: OfflineData =
            serde_json::from_str(&serde_json::to_string(&core.data()).unwrap()).unwrap();
        let mut core = OfflineCore::new(data);
        assert_eq!(core.policies[&42].cursors[0], 99);
        core.publish(track.clone(), "/offline/ready".into());
        core.cancel();
        assert_eq!(core.path(track.id()), Some("/offline/ready"));
        assert!(!core.policies[&42].enabled);
        core.remove(track.id());
        core.enqueue(track.clone(), false);
        assert!(core.pending.is_empty());
        core.enqueue(track.clone(), true);
        assert_eq!(core.pending.len(), 1);
        core.remove(track.id());
        core.save_playlist(42, [true, true, false]);
        core.enqueue(track, false);
        assert_eq!(core.pending.len(), 1);
    }
}

#[cfg(test)]
mod scan_tests {
    use super::*;
    fn track(message: i64) -> Track {
        Track {
            chat: 42,
            message,
            ..Default::default()
        }
    }
    #[test]
    fn reconnect_keeps_history_and_resumes_interrupted_head_refresh() {
        let mut core = OfflineCore::new(Default::default());
        core.save_playlist(42, [true, true, false]);
        core.page(42, 0, 50, vec![track(100), track(70)]);
        core.page(42, 1, 30, vec![track(60)]);
        core.reconnect();
        assert_eq!(core.next_scan(), Some((42, 0, 0)));
        core.page(42, 0, 110, vec![track(120), track(115)]);
        assert_eq!(core.policies[&42].head[0].watermark, 100);
        let mut core = OfflineCore::new(core.data());
        core.reconnect();
        assert_eq!(core.next_scan(), Some((42, 0, 110)));
        core.page(42, 0, 90, vec![track(105), track(100)]);
        assert_eq!(core.policies[&42].head[0].watermark, 120);
        assert_eq!(core.policies[&42].cursors, [50, 30, 0]);
        core.page(42, 1, 0, vec![track(65), track(60)]);
        assert_eq!(core.next_scan(), Some((42, 0, 50)));
        assert_eq!(
            core.pending.values().map(|t| t.message).collect::<Vec<_>>(),
            vec![100, 70, 60, 120, 115, 105, 65]
        );
    }
    #[test]
    fn completing_one_of_ten_thousand_jobs_has_constant_delta() {
        let data = OfflineData {
            pending: (1..=10_000).map(track).collect(),
            ..Default::default()
        };
        let mut core = OfflineCore::new(data);
        core.publish(track(5000), "ready".into());
        let delta = core.take_changes().unwrap();
        assert_eq!(delta.pending.len(), 1);
        assert_eq!(delta.copies.len(), 1);
        assert!(
            delta.policies.is_empty() && delta.removing.is_empty() && delta.excluded.is_empty()
        );
        assert!(core.take_changes().is_none());
        assert_eq!(core.pending.values().next().unwrap().message, 1);
        assert_eq!(core.pending.values().last().unwrap().message, 10_000);
    }
    #[test]
    fn legacy_policy_without_watermark_initializes_once_without_resetting_history() {
        let policy: DownloadPolicy = serde_json::from_str(r#"{"chat":42,"filters":[true,false,false],"enabled":true,"cursors":[50,0,0],"done":[false,true,true]}"#).unwrap();
        let mut core = OfflineCore::new(OfflineData {
            policies: vec![policy],
            ..Default::default()
        });
        core.reconnect();
        core.page(42, 0, 80, vec![track(100)]);
        assert!(core.policies[&42].head[0].active);
        core.page(42, 0, 0, vec![track(60)]);
        assert_eq!(core.policies[&42].head[0].watermark, 100);
        core.reconnect();
        core.page(42, 0, 90, vec![track(105), track(100)]);
        assert!(!core.policies[&42].head[0].active);
        assert_eq!(core.next_scan(), Some((42, 0, 50)));
    }
}
