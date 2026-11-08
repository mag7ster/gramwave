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
                self.tasks.watch(job, move |result| {
                    let event = match result {
                        Ok(event) => event,
                        Err(error) => failed(error),
                    };
                    let _ = tx.send(event);
                });
            }
            Err(error) => {
                let _ = self.tx.send(failed(error));
            }
        }
    }
    pub(crate) fn pump_offline(&mut self) {
        if self.quitting {
            return;
        }
        let online = !self.demo
            && self.tg.is_some()
            && *self.state.auth().state()
                == tg_player_application::telegram::AuthorizationState::Ready;
        let effects = self.state.pump(std::time::Instant::now(), online);
        self.run_effects(effects);
    }
}
