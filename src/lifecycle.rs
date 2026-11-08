use crate::*;
use fastframe_shell::{Closed, Headless, Held, Resident, Shell, Waker};
struct Window {
    app: Held<App>,
}
impl eframe::App for Window {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.app.background(ctx, true);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut *self.app, ui, frame);
    }
}
impl Resident for App {
    fn closed(&self) -> Closed {
        if self.hidden && !self.quitting {
            Closed::Hide
        } else {
            Closed::Quit
        }
    }
    fn window_gone(&mut self) {
        self.hidden = false;
    }
    fn headless_frame(&mut self, ctx: &egui::Context) -> Headless {
        self.background(ctx, false);
        if self.quitting {
            Headless::Quit
        } else if self
            .show_requested
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            Headless::Show
        } else {
            Headless::Wait
        }
    }
    fn shutdown(&mut self) {
        self.shutdown_core();
    }
}
pub fn run() -> eframe::Result {
    let waker = Waker::default();
    let show = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = show.clone();
    let wake = waker.clone();
    let mut slot = fastframe_instance::Slot::new("org.tgplayer.App");
    if std::env::args().any(|s| s == "--demo") {
        slot = slot.scoped("demo");
    }
    let _guard = match slot.claim("show", move |_| {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
        wake.wake();
        Some("ok".into())
    }) {
        fastframe_instance::Claim::First(g) => g,
        _ => return Ok(()),
    };
    let mut app = App::new();
    app.show_requested = show;
    if !app.demo && app.state.api_id.parse::<i32>().is_ok() && app.state.api_hash.len() == 32 {
        app.connect();
    }
    Shell::new(app, &waker)
        .idle(fastframe_tray::idle)
        .run(|lease| {
            eframe::run_native(
                "TG Player",
                eframe::NativeOptions {
                    viewport: egui::ViewportBuilder::default()
                        .with_inner_size([1240., 800.])
                        .with_min_inner_size([850., 550.]),
                    ..Default::default()
                },
                Box::new(move |cc| {
                    cc.egui_ctx
                        .set_fonts(fastframe_fonts::FontSetup::default().definitions());
                    egui_extras::install_image_loaders(&cc.egui_ctx);
                    crate::design::install(&cc.egui_ctx);
                    Ok(Box::new(Window {
                        app: lease.take(&cc.egui_ctx),
                    }))
                }),
            )
        })
}
