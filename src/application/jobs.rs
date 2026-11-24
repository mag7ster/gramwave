use crate::{App, Event};
use std::sync::Arc;
use tg_player_application::tasks::JobClass;
impl App {
    pub(crate) fn async_event(
        &self,
        class: JobClass,
        generation: u64,
        future: impl std::future::Future<Output = Event> + Send + 'static,
        failed: impl Fn(tg_player_application::AppError) -> Event + Send + Sync + 'static,
    ) {
        self.operation_event(
            class,
            tg_player_application::tasks::Operation::new(generation),
            future,
            failed,
        );
    }
    pub(crate) fn operation_event(
        &self,
        class: JobClass,
        operation: tg_player_application::tasks::Operation,
        future: impl std::future::Future<Output = Event> + Send + 'static,
        failed: impl Fn(tg_player_application::AppError) -> Event + Send + Sync + 'static,
    ) {
        let failed = Arc::new(failed);
        match self
            .tasks
            .submit_operation(class, operation, async move { Ok(future.await) })
        {
            Ok(job) => {
                let tx = self.tx.clone();
                let wake = self.wake.clone();
                self.tasks.watch(job, move |result| {
                    let event = match result {
                        Ok(event) => event,
                        Err(error) => failed(error),
                    };
                    if tx.send(event).is_ok() {
                        wake.wake();
                    }
                });
            }
            Err(error) => {
                if self.tx.send(failed(error)).is_ok() {
                    self.wake.wake();
                }
            }
        }
    }
    pub(crate) fn pump_offline(&mut self, now: std::time::Instant) -> Option<std::time::Instant> {
        if self.quitting {
            return None;
        }
        let online = !self.demo
            && self.tg.is_some()
            && *self.state.auth().state()
                == tg_player_application::telegram::AuthorizationState::Ready;
        let handoff = online.then(|| self.finish_cache_handoff(now)).flatten();
        let effects = self.state.pump(now, online);
        self.run_effects(effects);
        handoff
            .into_iter()
            .chain(self.state.offline().retry_deadline(now, online))
            .min()
    }

    fn finish_cache_handoff(&mut self, now: std::time::Instant) -> Option<std::time::Instant> {
        use crate::telegram::{CacheCategory, CacheFilter};
        use std::time::Duration;
        if self.handoff_running {
            return None;
        }
        self.handoff_retry.retain(|path, _| {
            self.state
                .offline()
                .copies
                .values()
                .any(|copy| copy.path == *path && copy.handoff_pending)
        });
        let Some(copy) = self
            .state
            .offline()
            .copies
            .values()
            .find(|copy| {
                copy.handoff_pending
                    && self
                        .handoff_retry
                        .get(&copy.path)
                        .is_none_or(|at| *at <= now)
            })
            .cloned()
        else {
            return self.handoff_retry.values().copied().min();
        };
        let tg = self.tg.clone()?;
        let mut categories = [false; CacheCategory::ALL.len()];
        categories[CacheCategory::Audio as usize] = true;
        let Some(access) = tg.try_cache_write(
            CacheFilter {
                chat: None,
                categories,
            },
            self.cache_audio_available(),
        ) else {
            return Some(now + Duration::from_secs(1));
        };
        let db = self.db.as_ref()?;
        let Ok(barrier) = db.commit_barrier() else {
            return Some(now + Duration::from_secs(30));
        };
        self.handoff_running = true;
        let root = self.root.clone();
        let id = copy.track.id();
        let path = copy.path.clone();
        self.async_event(
            JobClass::Interactive,
            0,
            async move {
                let _access = access;
                let result = async {
                    barrier
                        .await
                        .map_err(|e| tg_player_application::AppError::Storage(e.to_string()))??;
                    crate::adapters::offline::release_temporary_copy(&tg, &copy, &root).await
                }
                .await;
                Event::CacheHandoff {
                    id,
                    path: copy.path,
                    result,
                }
            },
            move |error| Event::CacheHandoff {
                id,
                path: path.clone(),
                result: Err(error),
            },
        );
        None
    }
}
