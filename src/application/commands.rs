use crate::App;
pub use tg_player_application::commands::AppCommand;
impl App {
    pub(crate) fn execute(&mut self, command: AppCommand) {
        if self.quitting {
            return;
        }
        #[cfg(test)]
        if let AppCommand::Seek(value) = &command {
            self.test_seeks.push(*value);
        }
        self.core_command(command);
    }
}
