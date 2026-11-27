use crate::identity::APP_ID;
use crate::tr;
use clap::Parser;
mod process;
pub(crate) use process::ProcessIdentity;
use std::{path::PathBuf, sync::OnceLock};

pub(crate) const MIN_WINDOW_SIZE: [f32; 2] = [220., 96.];

#[derive(Clone, Debug, Default, Parser)]
#[command(name = APP_ID, version, about)]
pub struct LaunchOptions {
    #[arg(long, help = tr("Use demo data without signing in. With --ctl, target the demo instance."))]
    pub demo: bool,
    #[arg(long, help = tr("Enable desktop media controls and the tray in demo mode"))]
    pub desktop_integrations: bool,
    #[arg(long, help = tr("Start the demo with the light theme"))]
    pub light: bool,
    #[arg(long, help = tr("Capture a screenshot of the window and exit after a few seconds"))]
    pub smoke_test: bool,
    #[arg(long, help = tr("Set up a demo scenario, for example playing, queue or settings"))]
    pub scenario: Option<String>,
    #[arg(long, help = tr("Preview a sign-in screen, for example qr, code or password"))]
    pub preview_auth: Option<String>,
    #[arg(long, help = tr("Set the demo window size, for example 1240x800"))]
    pub size: Option<String>,
    #[arg(long, help = tr("Set the demo UI scale between 0.5 and 3"))]
    pub scale: Option<f32>,
    #[arg(long, help = tr("Set the smoke-test screenshot path (default /tmp/gramwave-preview.png)"))]
    pub screenshot: Option<PathBuf>,
    #[arg(long, help = tr("Start without a window or tray. Sign in through the graphical app first."))]
    pub headless: bool,
    #[arg(long, help = tr("Start in the background and return once ready. Combine with --headless for no window."))]
    pub daemon: bool,
    #[arg(long, num_args = 0.., allow_hyphen_values = true, help = tr("Control a running player. Run --ctl --help for commands."))]
    pub ctl: Option<Vec<String>>,
}
fn parsed() -> &'static (Option<bool>, Result<LaunchOptions, clap::Error>) {
    static PARSED: OnceLock<(Option<bool>, Result<LaunchOptions, clap::Error>)> = OnceLock::new();
    PARSED.get_or_init(|| {
        let args: Vec<_> = std::env::args_os().collect();
        let control = args
            .iter()
            .enumerate()
            .skip(1)
            .take_while(|(_, arg)| *arg != "--")
            .find_map(|(index, arg)| {
                (arg == "--ctl" || arg.as_encoded_bytes().starts_with(b"--ctl=")).then_some(index)
            })
            .map(|index| {
                args[index + 1..]
                    .iter()
                    .take_while(|arg| *arg != "--")
                    .any(|arg| arg == "--json")
            });
        (control, LaunchOptions::try_parse_from(args))
    })
}
pub(crate) fn control_options(
) -> Option<(bool, Result<&'static LaunchOptions, &'static clap::Error>)> {
    let (json, result) = parsed();
    json.map(|json| (json, result.as_ref()))
}
pub fn options() -> &'static LaunchOptions {
    match &parsed().1 {
        Ok(options) => options,
        Err(error) => error.exit(),
    }
}
impl LaunchOptions {
    pub fn window_size(&self) -> [f32; 2] {
        if self.demo {
            if let Some((w, h)) = self.size.as_deref().and_then(|s| s.split_once('x')) {
                if let (Ok(w), Ok(h)) = (w.parse::<f32>(), h.parse::<f32>()) {
                    if w.is_finite() && h.is_finite() {
                        return [w.max(MIN_WINDOW_SIZE[0]), h.max(MIN_WINDOW_SIZE[1])];
                    }
                }
            }
        }
        [1240., 800.]
    }
    pub fn scale(&self) -> Option<f32> {
        self.scale
            .filter(|v| self.demo && v.is_finite() && (0.5..=3.).contains(v))
    }
}
