use crate::{
    domain::{RepeatMode, Track},
    App,
};
/// Shared entry point for window controls, tray and media keys.
pub enum AppCommand {
    Next,
    Previous,
    Pause(bool),
    Volume(f32),
    Shuffle(bool),
    Repeat(RepeatMode),
    Seek(f64),
    Play(Track),
    OpenPlaylist(i64),
    SelectPlaylistRow(usize),
    PlayQueue(crate::domain::model::QueueEntryId),
}
impl App {
    pub(crate) fn execute(&mut self, command: AppCommand) {
        if self.quitting {
            return;
        }
        match command {
            AppCommand::Next => self.next(),
            AppCommand::Previous => self.previous(),
            AppCommand::Pause(value) => self.set_paused(value),
            AppCommand::Volume(value) => self.set_volume(value),
            AppCommand::Shuffle(value) => self.set_shuffle(value),
            AppCommand::Repeat(value) => self.set_repeat(value),
            AppCommand::Seek(value) => self.seek(value),
            AppCommand::Play(track) => self.play(track),
            AppCommand::OpenPlaylist(id) => {
                if let Some(index) = self.state.playlists.iter().position(|p| p.id == id) {
                    self.open_chat(index);
                }
            }
            AppCommand::SelectPlaylistRow(index) => self.select_playlist_row(index),
            AppCommand::PlayQueue(id) => self.play_queue_entry(id),
        }
    }
}
