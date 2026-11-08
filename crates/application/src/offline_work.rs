use crate::{
    offline::OfflineCore,
    tasks::{Operation, RequestId},
    workflow::Effect,
    AppError,
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tg_player_domain::TrackId;
#[derive(Default)]
pub(crate) struct OfflineWork {
    pub generation: u64,
    pub active: HashMap<TrackId, Operation>,
    pub scan: Option<(Operation, i64, usize)>,
    pub removals: HashMap<String, Operation>,
    pub retry: HashMap<TrackId, Instant>,
    pub removal_retry: HashMap<String, Instant>,
    pub scan_retry: Option<Instant>,
    pub errors: HashMap<(i64, i64), AppError>,
}
impl OfflineCore {
    pub fn generation(&self) -> u64 {
        self.work.generation
    }
    pub fn errors(&self) -> &HashMap<(i64, i64), AppError> {
        &self.work.errors
    }
    pub fn retrying(&self, id: TrackId) -> bool {
        self.work.retry.contains_key(&id)
    }
    pub(crate) fn cancel_scan(&mut self) {
        if let Some((op, _, _)) = self.work.scan.take() {
            op.cancel.cancel();
        }
    }
    pub(crate) fn cancel_attempt(&mut self, id: TrackId) {
        if let Some(op) = self.work.active.remove(&id) {
            op.cancel.cancel();
        }
    }
    pub(crate) fn suspend(&mut self) {
        self.work.generation = self.work.generation.wrapping_add(1);
        self.cancel_scan();
        for (_, op) in self.work.active.drain() {
            op.cancel.cancel();
        }
        self.work.retry.clear();
        self.work.scan_retry = None;
    }
    pub(crate) fn cancel_work(&self) {
        for op in self.work.active.values().chain(self.work.removals.values()) {
            op.cancel.cancel();
        }
        if let Some((op, _, _)) = &self.work.scan {
            op.cancel.cancel();
        }
    }
    pub(crate) fn pump(&mut self, now: Instant, online: bool) -> Vec<Effect> {
        let mut effects = Vec::new();
        let removals: Vec<_> = self
            .data
            .removing
            .iter()
            .filter(|(path, _)| {
                !self.work.removals.contains_key(*path)
                    && self
                        .work
                        .removal_retry
                        .get(*path)
                        .is_none_or(|at| *at <= now)
            })
            .take(4usize.saturating_sub(self.work.removals.len()))
            .map(|(path, _)| path.clone())
            .collect();
        for path in removals {
            let op = Operation::new(self.work.generation);
            self.work.removals.insert(path.clone(), op.clone());
            effects.push(Effect::RemoveOffline {
                operation: op,
                path: path.clone(),
            });
        }
        if !online {
            return effects;
        }
        if self.work.scan.is_none() && self.work.scan_retry.is_none_or(|at| at <= now) {
            if let Some((chat, kind, cursor)) = self.next_scan() {
                let op = Operation::new(self.work.generation);
                self.work.scan = Some((op.clone(), chat, kind));
                effects.push(Effect::ScanOffline {
                    operation: op,
                    chat,
                    kind,
                    cursor,
                });
            }
        }
        let pending: Vec<_> = self
            .data
            .pending
            .iter()
            .filter(|(id, _)| {
                !self.work.active.contains_key(id)
                    && self.work.retry.get(id).is_none_or(|at| *at <= now)
            })
            .take(2usize.saturating_sub(self.work.active.len()))
            .map(|(id, track)| (*id, track.clone()))
            .collect();
        for (id, track) in pending {
            let op = Operation::new(self.work.generation);
            self.work.active.insert(id, op.clone());
            effects.push(Effect::DownloadOffline {
                operation: op,
                track: track.clone(),
            });
        }
        effects
    }
    pub(crate) fn downloaded(
        &mut self,
        request: RequestId,
        id: TrackId,
        result: Result<String, AppError>,
        now: Instant,
    ) -> Vec<Effect> {
        let current = self
            .work
            .active
            .get(&id)
            .is_some_and(|op| op.key() == request);
        if !current {
            if let Ok(path) = result {
                self.queue_removal(path);
            }
            return Vec::new();
        }
        self.work.active.remove(&id);
        match result {
            Ok(path) => {
                if self.data.excluded.contains(&id) {
                    self.queue_removal(path);
                    return Vec::new();
                }
                if let Some(track) = self.data.pending.get(&id).cloned() {
                    self.publish(track, path);
                    self.work.retry.remove(&id);
                    self.work.errors.remove(&(id.chat, id.message));
                    vec![Effect::Notice(crate::notice::Notice::OfflineSaved)]
                } else {
                    self.queue_removal(path);
                    Vec::new()
                }
            }
            Err(error) => {
                self.work.retry.insert(id, now + Duration::from_secs(30));
                self.work
                    .errors
                    .insert((id.chat, id.message), error.clone());
                vec![Effect::OfflineFailed(id, error)]
            }
        }
    }
}
