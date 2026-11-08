use super::*;
#[test]
fn bounded_queue_never_blocks_caller_and_shutdown_is_confirmed() {
    let mut workers = Workers::new(1, 1);
    let (started, ready) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    workers
        .spawn(move || {
            started.send(()).unwrap();
            blocked.recv().unwrap();
        })
        .unwrap();
    ready.recv_timeout(Duration::from_secs(1)).unwrap();
    workers
        .spawn(|| panic!("queued job ran after cancellation"))
        .unwrap();
    assert!(workers.spawn(|| {}).is_err());
    assert!(workers
        .shutdown(Duration::ZERO)
        .unwrap_err()
        .contains("timed out"));
    assert!(workers.spawn(|| {}).is_err());
    release.send(()).unwrap();
    workers.shutdown(Duration::from_secs(1)).unwrap();
}
