use crate::{design, tr, App};
use eframe::egui;
use gramwave_application::{commands::AppCommand, model::StartupPlaylist, persistence::Settings};

impl App {
    pub(crate) fn startup_settings_ui(&mut self, ui: &mut egui::Ui) {
        let current = self.state.view().startup_playlist;
        design::settings_section(ui, tr("On startup"), |ui| {
            ui.horizontal_wrapped(|ui| {
                for (mode, label) in [
                    (StartupPlaylist::LastOpened, tr("Last opened playlist")),
                    (StartupPlaylist::LastPlayed, tr("Last track's playlist")),
                ] {
                    if design::choice(ui, label, mode == current) && mode != current {
                        let mut settings = Settings::from(self.state.view());
                        settings.startup_playlist = mode;
                        self.execute(AppCommand::Settings(settings));
                    }
                }
            });
        });
    }
}
