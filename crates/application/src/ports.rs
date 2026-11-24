#[derive(Debug, PartialEq)]
pub enum PlaybackEvent {
    Loaded,
    Finished,
    Stopped,
    Failed(crate::AppError),
}
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerCommand {
    Load { path: String, start_ms: u64 },
    Pause(bool),
    Stop,
    Seek(f64),
    Volume(f32),
}
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerSnapshot {
    pub paused: bool,
    pub seeking: bool,
    pub position: f64,
    /// Playback time from the audio output clock, when available.
    pub audio_position: Option<f64>,
    /// Confirmed timeline position of the current load after initial playback or seeking has settled.
    pub checkpoint_position: Option<f64>,
    pub duration: f64,
    pub volume: f64,
}
pub trait PlayerBackend {
    fn command(&self, command: PlayerCommand) -> Result<(), crate::AppError>;
    fn shutdown(&mut self, _timeout: std::time::Duration) -> Result<(), crate::AppError> {
        Ok(())
    }
    fn drain(&self) -> Vec<PlaybackEvent>;
    fn snapshot(&self) -> PlayerSnapshot;
    fn is_idle(&self) -> bool {
        self.snapshot().duration <= 0.
    }
}
pub trait Repository {
    fn apply(&self, changes: crate::persistence::ChangeSet) -> Result<(), crate::AppError>;
    fn flush(&self, timeout: std::time::Duration) -> Result<(), crate::AppError>;
    fn commit_barrier(
        &self,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), crate::AppError>>, crate::AppError>;
    fn shutdown(&mut self, timeout: std::time::Duration) -> Result<(), crate::AppError>;
    fn error(&self) -> Option<crate::AppError>;
}

#[derive(Clone, Debug)]
pub struct OfflinePublication {
    pub track: tg_player_domain::TrackId,
    pub attempt: u64,
    pub source: String,
    pub size: u64,
}
pub trait OfflineStorage: Send + Sync {
    fn publish<'a>(
        &'a self,
        request: OfflinePublication,
        cancel: &'a tokio_util::sync::CancellationToken,
    ) -> crate::telegram::GatewayFuture<'a, String>;
    fn verify<'a>(&'a self, path: &'a str, size: u64) -> crate::telegram::GatewayFuture<'a, bool>;
    fn remove<'a>(&'a self, path: &'a str) -> crate::telegram::GatewayFuture<'a, ()>;
}
