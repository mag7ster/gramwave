use crate::App;
pub use gramwave_application::commands::AppCommand;
impl App {
    pub(crate) fn set_queue_panel_open(&mut self, open: bool) {
        let state = self.state.view();
        if state.queue_panel_open != open {
            let mut settings = gramwave_application::persistence::Settings::from(state);
            settings.queue_panel_open = open;
            self.execute(AppCommand::Settings(settings));
        }
    }

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
