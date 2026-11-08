use super::*;
use std::sync::mpsc;
#[test]
fn bounded_queue_rejects_without_blocking_and_delivers_cancellation() {
    let mut scheduler = Scheduler::with_capacity(1);
    let mut running = Vec::new();
    for _ in 0..4 {
        let (started, ready) = mpsc::channel();
        let job = scheduler
            .submit(JobClass::Background, 0, async move {
                started.send(()).unwrap();
                std::future::pending::<Result<(), AppError>>().await
            })
            .unwrap();
        ready.recv_timeout(Duration::from_secs(1)).unwrap();
        running.push(job);
    }
    let queued = scheduler
        .submit(JobClass::Background, 1, async { Ok(()) })
        .unwrap();
    assert!(matches!(
        scheduler.submit(JobClass::Background, 2, async { Ok(()) }),
        Err(AppError::Overloaded(_))
    ));
    let interactive = scheduler
        .submit(JobClass::Interactive, 1, async { Ok(42) })
        .unwrap();
    assert_eq!(scheduler.handle().block_on(interactive.result()), Ok(42));
    queued.operation.cancel.cancel();
    assert_eq!(
        scheduler.handle().block_on(queued.result()),
        Err(AppError::Cancelled)
    );
    for job in running {
        job.operation.cancel.cancel();
        assert_eq!(
            scheduler.handle().block_on(job.result()),
            Err(AppError::Cancelled)
        );
    }
    scheduler.shutdown(Duration::from_secs(1)).unwrap();
    assert!(scheduler
        .submit(JobClass::Interactive, 0, async { Ok(()) })
        .is_err());
}

#[test]
fn classes_are_independent_cancellation_is_terminal_and_panics_are_reported() {
    let mut workers = Scheduler::new();
    let runtime = workers.handle();
    let gate = CancellationToken::new();
    let mut background = Vec::new();
    for _ in 0..4 {
        let gate = gate.clone();
        background.push(
            workers
                .submit(JobClass::Background, 1, async move {
                    gate.cancelled().await;
                    Ok(())
                })
                .unwrap(),
        );
    }
    let interactive = workers
        .submit(JobClass::Interactive, 2, async { Ok(42) })
        .unwrap();
    assert_eq!(runtime.block_on(interactive.result()), Ok(42));
    let cancelled = workers
        .submit(JobClass::Background, 3, async { Ok(()) })
        .unwrap();
    cancelled.operation.cancel.cancel();
    assert_eq!(
        runtime.block_on(cancelled.result()),
        Err(AppError::Cancelled)
    );
    let panic = workers
        .submit::<(), _>(JobClass::Interactive, 4, async { panic!("injected") })
        .unwrap();
    assert_eq!(runtime.block_on(panic.result()), Err(AppError::Panicked));
    gate.cancel();
    for job in background {
        runtime.block_on(job.result()).unwrap();
    }
    workers.shutdown(Duration::from_secs(1)).unwrap();
}

#[test]
fn playback_starts_while_search_and_auth_are_saturated() {
    let mut scheduler = Scheduler::new();
    let mut held = Vec::new();
    for class in [
        JobClass::Background,
        JobClass::Background,
        JobClass::Background,
        JobClass::Background,
        JobClass::Interactive,
    ] {
        let (started, ready) = mpsc::channel();
        held.push(
            scheduler
                .submit(class, 0, async move {
                    started.send(()).unwrap();
                    std::future::pending::<Result<(), AppError>>().await
                })
                .unwrap(),
        );
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
    }
    let (started, ready) = mpsc::channel();
    let playback = scheduler
        .submit(JobClass::Playback, 42, async move {
            started.send(()).unwrap();
            Ok(42)
        })
        .unwrap();
    ready
        .recv_timeout(Duration::from_secs(2))
        .expect("playback waited for unrelated work");
    assert_eq!(playback.operation.generation, 42);
    assert_eq!(scheduler.handle().block_on(playback.result()), Ok(42));
    drop(held);
    scheduler.shutdown(Duration::from_secs(2)).unwrap();
}

#[test]
fn rapid_playback_replacement_keeps_only_latest_pending_and_preserves_identity() {
    use std::sync::atomic::AtomicUsize;
    struct PendingGuard(Arc<AtomicUsize>);
    impl PendingGuard {
        fn new(live: &Arc<AtomicUsize>) -> Self {
            live.fetch_add(1, Ordering::SeqCst);
            Self(live.clone())
        }
    }
    impl Drop for PendingGuard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let mut scheduler = Scheduler::new();
    let live = Arc::new(AtomicUsize::new(0));
    let mut cancelled = Vec::new();
    for generation in 0..1000 {
        let guard = PendingGuard::new(&live);
        let operation = Operation::new(generation);
        let key = operation.key();
        let job = scheduler
            .submit_operation(JobClass::Playback, operation, async move {
                let _guard = guard;
                std::future::pending::<Result<(), AppError>>().await
            })
            .expect("replacement must never reject the newest selection");
        assert_eq!(job.operation.key(), key);
        assert!(
            live.load(Ordering::SeqCst) <= 2,
            "playback backlog grew beyond active + pending"
        );
        cancelled.push(job);
    }
    let operation = Operation::new(1000);
    let key = operation.key();
    let newest = scheduler
        .submit_operation(JobClass::Playback, operation, async { Ok(42) })
        .unwrap();
    assert_eq!(newest.operation.key(), key);
    assert_eq!(
        scheduler.handle().block_on(async {
            tokio::time::timeout(Duration::from_secs(2), newest.result())
                .await
                .unwrap()
        }),
        Ok(42)
    );
    for job in cancelled {
        assert_eq!(
            scheduler.handle().block_on(job.result()),
            Err(AppError::Cancelled)
        );
    }
    assert_eq!(live.load(Ordering::SeqCst), 0);
    scheduler.shutdown(Duration::from_secs(2)).unwrap();
}

#[test]
fn shutdown_delivers_active_and_pending_playback_terminals_and_watched_completion() {
    let mut scheduler = Scheduler::new();
    let (started, ready) = mpsc::channel();
    let first = scheduler
        .submit::<(), _>(JobClass::Playback, 1, async move {
            started.send(()).unwrap();
            std::future::pending().await
        })
        .unwrap();
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    let second = scheduler
        .submit::<(), _>(JobClass::Playback, 2, std::future::pending())
        .unwrap();
    let first_key = first.operation.key();
    let second_key = second.operation.key();
    let (completed, received) = mpsc::channel();
    let terminal = completed.clone();
    scheduler.watch(first, move |result| {
        terminal.send((first_key, result)).unwrap()
    });
    scheduler.watch(second, move |result| {
        completed.send((second_key, result)).unwrap()
    });
    scheduler.shutdown(Duration::from_secs(2)).unwrap();
    let results: std::collections::HashMap<_, _> = received.try_iter().collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results.get(&first_key), Some(&Err(AppError::Cancelled)));
    assert_eq!(results.get(&second_key), Some(&Err(AppError::Cancelled)));
}

#[test]
fn playback_panic_and_explicit_cancellation_do_not_stop_the_mailbox() {
    let mut scheduler = Scheduler::new();
    let panic = scheduler
        .submit::<(), _>(JobClass::Playback, 1, async {
            panic!("injected playback panic")
        })
        .unwrap();
    assert_eq!(
        scheduler.handle().block_on(panic.result()),
        Err(AppError::Panicked)
    );
    let pending = scheduler
        .submit::<(), _>(JobClass::Playback, 2, std::future::pending())
        .unwrap();
    pending.operation.cancel.cancel();
    assert_eq!(
        scheduler.handle().block_on(pending.result()),
        Err(AppError::Cancelled)
    );
    let next = scheduler
        .submit(JobClass::Playback, 3, async { Ok(42) })
        .unwrap();
    assert_eq!(scheduler.handle().block_on(next.result()), Ok(42));
    scheduler.shutdown(Duration::from_secs(2)).unwrap();
}
