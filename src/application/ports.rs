use crate::domain::State;
#[derive(Debug, PartialEq)]
pub enum PlaybackEvent {
    Loaded,
    Finished,
    Stopped,
    Failed(String),
}
pub trait PlayerBackend {
    fn cmd(&self, args: &[&str]);
    fn drain(&self) -> Vec<PlaybackEvent>;
    fn number(&self, name: &str) -> f64;
}
pub trait Repository {
    fn save(&self, state: State) -> Result<(), String>;
    fn save_volume(&self, volume: f32, unmuted: f32) -> Result<(), String>;
    fn flush(&self) -> Result<(), String>;
    fn shutdown(&mut self) -> Result<(), String>;
    fn error(&self) -> Option<String>;
}
