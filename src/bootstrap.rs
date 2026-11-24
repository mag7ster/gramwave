mod services;
use crate::{demo, App};
use eframe::egui;
use fastframe_shell::{Closed, Headless, Held, Resident, Shell, Waker};
pub(crate) use services::{connect, Services};
struct Window {
    app: Held<App>,
}
impl eframe::App for Window {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        eframe::App::logic(&mut *self.app, ctx, frame);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut *self.app, ui, frame);
        fastframe_titlebar::resize(ui);
    }
    #[cfg(target_os = "linux")]
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        if let Some(tray) = &self.app.tray {
            tray.set_window(None);
        }
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
        self.custom_emoji.release_textures();
        self.hidden = false;
        self.tray_focus = None;
        self.wayland = false;
    }
    fn headless_frame(&mut self, ctx: &egui::Context) -> Headless {
        let wait = self
            .background(ctx, false)
            .unwrap_or(std::time::Duration::MAX);
        if self.quitting {
            Headless::Quit
        } else if self
            .show_requested
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            Headless::Show
        } else {
            Headless::WaitFor(wait)
        }
    }
    fn shutdown(&mut self) {
        if let Err(error) = self.shutdown_core() {
            eprintln!("shutdown: {error}");
            *self.shutdown_error.lock().unwrap() = Some(error.clone());
            self.error = error;
        }
    }
}
pub fn run() -> eframe::Result {
    let waker = Waker::default();
    let show = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = show.clone();
    let wake = waker.clone();
    let slot = crate::adapters::control::slot(crate::launch::options().demo);
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
    let mut app = App::new(waker.clone())
        .map_err(|e| eframe::Error::AppCreation(Box::new(std::io::Error::other(e))))?;
    demo::configure(&mut app);
    app.show_requested = show;
    app.control = Some(control_rx);
    if !app.demo
        && app.state.view().api_id.parse::<i32>().is_ok()
        && app.state.view().api_hash.len() == 32
    {
        app.connect();
    }
    let shutdown_error = app.shutdown_error.clone();
    #[cfg(target_os = "linux")]
    let app_id = crate::adapters::window_activation::app_id(app.demo);
    fastframe_titlebar::set_enabled(true);
    Shell::new(app, &waker).run(|lease| {
        let viewport = egui::ViewportBuilder::default()
            .with_inner_size(crate::launch::options().window_size())
            .with_decorations(!fastframe_titlebar::enabled())
            .with_min_inner_size(crate::launch::MIN_WINDOW_SIZE);
        #[cfg(target_os = "linux")]
        let viewport = viewport.with_app_id(app_id);
        #[cfg(feature = "vulkan")]
        let wgpu_options = {
            use eframe::egui_wgpu;
            let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
            setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
            egui_wgpu::WgpuConfiguration {
                wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup),
                ..Default::default()
            }
        };
        eframe::run_native(
            "Gramwave",
            eframe::NativeOptions {
                viewport,
                #[cfg(feature = "vulkan")]
                renderer: eframe::Renderer::Wgpu,
                #[cfg(not(feature = "vulkan"))]
                renderer: eframe::Renderer::Glow,
                #[cfg(feature = "vulkan")]
                wgpu_options,
                ..Default::default()
            },
            Box::new(move |cc| {
                if let Some(scale) = crate::launch::options().scale() {
                    cc.egui_ctx.set_zoom_factor(scale);
                }
                cc.egui_ctx
                    .set_fonts(fastframe_fonts::FontSetup::default().definitions());
                egui_extras::install_image_loaders(&cc.egui_ctx);
                crate::emoji::install(&cc.egui_ctx, crate::launch::options().screenshot.is_some());
                crate::design::install(&cc.egui_ctx);
                let app = lease.take(&cc.egui_ctx);
                #[cfg(target_os = "linux")]
                let app = {
                    use crate::adapters::window_activation::WindowActivation;
                    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                    let mut app = app;
                    app.wayland = cc
                        .window_handle()
                        .is_ok_and(|handle| matches!(handle.as_raw(), RawWindowHandle::Wayland(_)));
                    let token = app.reopen_token.take();
                    if let Some(tray) = &app.tray {
                        let activation = cc
                            .winit_window()
                            .and_then(|window| WindowActivation::new(window.clone(), app_id));
                        if let (Some(token), Some(activation)) =
                            (token.as_deref(), activation.as_ref())
                        {
                            activation.activate(Some(token));
                        }
                        tray.set_window(activation);
                    }
                    app
                };
                Ok(Box::new(Window { app }))
            }),
        )
    })?;
    if let Some(error) = shutdown_error.lock().unwrap().take() {
        return Err(eframe::Error::AppCreation(Box::new(std::io::Error::other(
            error,
        ))));
    }
    Ok(())
}
