use super::*;

mod playlist;
mod queue;
mod transport;
mod window;

struct Harness {
    app: App,
    ctx: egui::Context,
    time: f64,
    size: egui::Vec2,
}
impl Harness {
    fn new() -> Self {
        let root = std::path::PathBuf::new();
        let mut app = App::new_at(root, true);
        app.player = None;
        app.opened_chat = Some(0);
        let ctx = egui::Context::default();
        design::install(&ctx);
        ctx.set_fonts(fastframe_fonts::FontSetup::default().definitions());
        let mut h = Self {
            app,
            ctx,
            time: 0.,
            size: egui::vec2(850., 550.),
        };
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| self.app.render(ui));
        output.textures_delta.clear();
        output
    }
    fn settle(&mut self) {
        for _ in 0..10 {
            self.frame(vec![]);
        }
    }
    fn pointer(&mut self, p: egui::Pos2, pressed: bool) {
        self.frame(vec![
            egui::Event::PointerMoved(p),
            egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
    }
    fn click(&mut self, p: egui::Pos2) {
        self.pointer(p, true);
        self.pointer(p, false);
    }
    fn key(&mut self, key: egui::Key) {
        self.frame(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        self.frame(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
    }
}

fn queue_harness(count: i64) -> Harness {
    let mut h = Harness::new();
    h.size = egui::vec2(1240., 800.);
    h.app.set_queue_panel_open(true);
    h.app.execute(AppCommand::StartSession(
        (1..=count)
            .map(|message| tg_player_application::model::Track {
                message,
                title: format!("Track {message}"),
                artist: "Artist".into(),
                duration: 180,
                ..Default::default()
            })
            .collect(),
        0,
    ));
    h.settle();
    h
}
