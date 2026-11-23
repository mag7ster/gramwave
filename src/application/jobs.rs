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
        let effects = self.state.pump(now, online);
        self.run_effects(effects);
        self.state.offline().retry_deadline(now, online)
    }
}
