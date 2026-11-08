use crate::AppError;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    runtime::{Builder, Runtime},
    sync::{oneshot, Notify, Semaphore},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JobId(pub u64);
#[derive(Clone, Copy, Debug)]
pub enum JobClass {
    Playback,
    Interactive,
    Background,
    Offline,
    Scan,
}
#[derive(Clone, Debug)]
pub struct Operation {
    pub id: JobId,
    pub generation: u64,
    pub cancel: CancellationToken,
}
impl Operation {
    pub fn new(generation: u64) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            id: JobId(NEXT.fetch_add(1, Ordering::Relaxed)),
            generation,
            cancel: CancellationToken::new(),
        }
    }
    pub fn key(&self) -> RequestId {
        RequestId {
            job: self.id,
            generation: self.generation,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RequestId {
    pub job: JobId,
    pub generation: u64,
}
pub struct Job<T> {
    pub operation: Operation,
    result: Option<oneshot::Receiver<Result<T, AppError>>>,
}
impl<T> Job<T> {
    pub async fn result(mut self) -> Result<T, AppError> {
        self.result
            .take()
            .unwrap()
            .await
            .unwrap_or(Err(AppError::Panicked))
    }
}
impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        self.operation.cancel.cancel();
    }
}

// Replacing an unpolled playback request must deliver cancellation, rather than
// merely dropping its sender and making the caller infer a task panic.
struct Completion<T>(Option<oneshot::Sender<Result<T, AppError>>>);
impl<T> Completion<T> {
    fn finish(mut self, result: Result<T, AppError>) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(result);
        }
    }
}
impl<T> Drop for Completion<T> {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(Err(AppError::Cancelled));
        }
    }
}
struct PlaybackRequest {
    cancel: CancellationToken,
    run: Pin<Box<dyn Future<Output = ()> + Send>>,
}
#[derive(Default)]
struct PlaybackState {
    active: Option<CancellationToken>,
    pending: Option<PlaybackRequest>,
}
#[derive(Default)]
struct PlaybackMailbox {
    state: Mutex<PlaybackState>,
    wake: Notify,
}
impl PlaybackMailbox {
    fn replace(&self, request: PlaybackRequest) {
        let mut state = self.state.lock().unwrap();
        if let Some(active) = &state.active {
            active.cancel();
        }
        if let Some(previous) = state.pending.replace(request) {
            previous.cancel.cancel();
            drop(previous);
        }
        drop(state);
        self.wake.notify_one();
    }
    async fn drive(self: Arc<Self>, shutdown: CancellationToken) {
        loop {
            // Register before checking the mailbox: a submission between this
            // check and await leaves a notification permit, so no wake is lost.
            let notified = self.wake.notified();
            let request = {
                let mut state = self.state.lock().unwrap();
                let request = state.pending.take();
                state.active = request.as_ref().map(|r| r.cancel.clone());
                request
            };
            if let Some(request) = request {
                request.run.await;
                continue;
            }
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => break,
                _ = notified => {}
            }
        }
        let mut state = self.state.lock().unwrap();
        if let Some(active) = state.active.take() {
            active.cancel();
        }
        if let Some(pending) = state.pending.take() {
            pending.cancel.cancel();
            drop(pending);
        }
    }
}

async fn run_job<T, F>(
    cancel: CancellationToken,
    shutdown: CancellationToken,
    future: F,
) -> Result<T, AppError>
where
    T: Send + 'static,
    F: Future<Output = Result<T, AppError>> + Send + 'static,
{
    if cancel.is_cancelled() || shutdown.is_cancelled() {
        return Err(AppError::Cancelled);
    }
    let mut task = tokio::spawn(future);
    tokio::select! {
        biased;
        _ = async { tokio::select! { _ = cancel.cancelled() => {}, _ = shutdown.cancelled() => {} } } => {
            task.abort();
            // An already completed publication still needs its real outcome;
            // discarding it could leave its published file without a registry.
            match task.await {
                Ok(result) => result,
                Err(error) if error.is_panic() => Err(AppError::Panicked),
                Err(_) => Err(AppError::Cancelled),
            }
        },
        result = &mut task => result.unwrap_or(Err(AppError::Panicked)),
    }
}

/// One desktop runtime. Blocking native services keep their own owner threads.
pub struct Scheduler {
    runtime: Option<Runtime>,
    tasks: TaskTracker,
    cancel: CancellationToken,
    ready: [Arc<Semaphore>; 5],
    slots: [Arc<Semaphore>; 5],
    playback: Arc<PlaybackMailbox>,
}
impl Scheduler {
    pub fn new() -> Self {
        Self::with_capacity(64)
    }
    pub fn with_capacity(capacity: usize) -> Self {
        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("tg-async")
            .build()
            .expect("create runtime");
        let tasks = TaskTracker::new();
        let cancel = CancellationToken::new();
        let playback = Arc::new(PlaybackMailbox::default());
        tasks.spawn_on(playback.clone().drive(cancel.clone()), runtime.handle());
        Self {
            runtime: Some(runtime),
            tasks,
            cancel,
            ready: [1, 1, capacity, 2, 1].map(|n| Arc::new(Semaphore::new(n))),
            slots: [1, 1, 4, 2, 1].map(|n| Arc::new(Semaphore::new(n))),
            playback,
        }
    }
    pub fn handle(&self) -> tokio::runtime::Handle {
        self.runtime
            .as_ref()
            .expect("runtime closed")
            .handle()
            .clone()
    }
    pub fn submit<T, F>(
        &self,
        class: JobClass,
        generation: u64,
        future: F,
    ) -> Result<Job<T>, AppError>
    where
        T: Send + 'static,
        F: Future<Output = Result<T, AppError>> + Send + 'static,
    {
        self.submit_operation(class, Operation::new(generation), future)
    }
    pub fn submit_operation<T, F>(
        &self,
        class: JobClass,
        operation: Operation,
        future: F,
    ) -> Result<Job<T>, AppError>
    where
        T: Send + 'static,
        F: Future<Output = Result<T, AppError>> + Send + 'static,
    {
        if self.cancel.is_cancelled() || operation.cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let cancel = operation.cancel.clone();
        let shutdown = self.cancel.clone();
        let (sender, result) = oneshot::channel();
        let completion = Completion(Some(sender));
        if matches!(class, JobClass::Playback) {
            self.playback.replace(PlaybackRequest {
                cancel: cancel.clone(),
                run: Box::pin(async move {
                    completion.finish(run_job(cancel, shutdown, future).await);
                }),
            });
        } else {
            let ready = self.ready[class as usize]
                .clone()
                .try_acquire_owned()
                .map_err(|_| AppError::Overloaded("tasks"))?;
            let slots = self.slots[class as usize].clone();
            self.tasks.spawn_on(
                async move {
                    let outcome = tokio::select! {
                        biased;
                        _ = shutdown.cancelled() => Err(AppError::Cancelled),
                        _ = cancel.cancelled() => Err(AppError::Cancelled),
                        permit = slots.acquire_owned() => {
                            drop(ready);
                            match permit {
                                Ok(_permit) => run_job(cancel, shutdown, future).await,
                                Err(_) => Err(AppError::Cancelled),
                            }
                        }
                    };
                    completion.finish(outcome);
                },
                &self.handle(),
            );
        }
        Ok(Job {
            operation,
            result: Some(result),
        })
    }
    pub fn watch<T: Send + 'static>(
        &self,
        job: Job<T>,
        complete: impl FnOnce(Result<T, AppError>) + Send + 'static,
    ) {
        self.tasks
            .spawn_on(async move { complete(job.result().await) }, &self.handle());
    }
    pub fn shutdown(&mut self, timeout: Duration) -> Result<(), String> {
        self.cancel.cancel();
        self.tasks.close();
        let Some(runtime) = self.runtime.take() else {
            return Ok(());
        };
        let done =
            runtime.block_on(async { tokio::time::timeout(timeout, self.tasks.wait()).await });
        runtime.shutdown_timeout(Duration::ZERO);
        done.map_err(|_| "tasks: shutdown timed out".to_owned())
    }
}
impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for Scheduler {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::ZERO);
        }
    }
}
#[cfg(test)]
#[path = "tasks/tests.rs"]
mod tests;
