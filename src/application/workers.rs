use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
type Job = Box<dyn FnOnce() + Send>;
/// Fixed concurrency and a bounded waiting queue. Rejection is explicit.
pub struct Workers {
    sender: Option<mpsc::SyncSender<Job>>,
    stopping: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
}
impl Workers {
    pub fn new(count: usize, capacity: usize) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Job>(capacity);
        let receiver = Arc::new(Mutex::new(receiver));
        let stopping = Arc::new(AtomicBool::new(false));
        let handles = (0..count)
            .map(|_| {
                let receiver = receiver.clone();
                let stopping = stopping.clone();
                std::thread::spawn(move || loop {
                    let next = receiver
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_millis(50));
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    match next {
                        Ok(job) => job(),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }
                })
            })
            .collect();
        Self {
            sender: Some(sender),
            stopping,
            handles,
        }
    }
    pub fn spawn(&self, job: impl FnOnce() + Send + 'static) -> Result<(), String> {
        if self.stopping.load(Ordering::Acquire) {
            return Err("workers: shutting down".into());
        }
        self.sender
            .as_ref()
            .ok_or("workers: closed")?
            .try_send(Box::new(job))
            .map_err(|e| format!("workers: {e}"))
    }
    pub fn shutdown(&mut self, timeout: Duration) -> Result<(), String> {
        self.stopping.store(true, Ordering::Release);
        self.sender.take();
        let deadline = Instant::now() + timeout;
        while self.handles.iter().any(|h| !h.is_finished()) {
            if Instant::now() >= deadline {
                return Err("workers: shutdown timed out".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut failed = false;
        for handle in self.handles.drain(..) {
            failed |= handle.join().is_err();
        }
        if failed {
            Err("workers: thread panicked".into())
        } else {
            Ok(())
        }
    }
}
impl Drop for Workers {
    fn drop(&mut self) {
        let _ = self.shutdown(Duration::from_secs(2));
    }
}
#[cfg(test)]
#[path = "workers/tests.rs"]
mod tests;
