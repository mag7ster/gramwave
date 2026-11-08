use crate::{demo, App};
use eframe::egui;
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
        if let Err(error) = self.shutdown_core() {
            eprintln!("shutdown: {error}");
            self.error = error;
        }
    }
}
pub fn run() -> eframe::Result {
    let waker = Waker::default();
    let show = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = show.clone();
    let wake = waker.clone();
    let slot = crate::adapters::control::slot(std::env::args().any(|s| s == "--demo"));
    let (control_tx, control_rx) = std::sync::mpsc::sync_channel(8);
    let _guard = match slot.claim("show", move |request| {
        if request == "show" {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
            wake.wake();
            Some("ok".into())
        } else {
            crate::adapters::control::bridge(request, &control_tx, || wake.wake())
        }
    }) {
        fastframe_instance::Claim::First(g) => g,
        _ => return Ok(()),
    };
    let mut app = App::new();
    demo::configure(&mut app);
    app.show_requested = show;
    app.control = Some(control_rx);
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
                        .with_inner_size(demo::window_size())
                        .with_min_inner_size([850., 550.]),
                    ..Default::default()
                },
                Box::new(move |cc| {
                    if let Some(scale) = demo::scale() {
                        cc.egui_ctx.set_zoom_factor(scale);
                    }
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
