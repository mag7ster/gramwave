use clap::Parser;
use std::{path::PathBuf, sync::OnceLock};

#[derive(Clone, Debug, Default, Parser)]
#[command(name = "tg-player", version, about)]
pub struct LaunchOptions {
    #[arg(long)]
    pub demo: bool,
    #[arg(long)]
    pub desktop_integrations: bool,
    #[arg(long)]
    pub light: bool,
    #[arg(long)]
    pub smoke_test: bool,
    #[arg(long)]
    pub scenario: Option<String>,
    #[arg(long)]
    pub preview_auth: Option<String>,
    #[arg(long)]
    pub size: Option<String>,
    #[arg(long)]
    pub scale: Option<f32>,
    #[arg(long)]
    pub screenshot: Option<PathBuf>,
    #[arg(long, num_args = 0.., allow_hyphen_values = true)]
    pub ctl: Option<Vec<String>>,
}
fn parsed() -> &'static (bool, Result<LaunchOptions, clap::Error>) {
    static PARSED: OnceLock<(bool, Result<LaunchOptions, clap::Error>)> = OnceLock::new();
    PARSED.get_or_init(|| {
        let args: Vec<_> = std::env::args_os().collect();
        let ctl = args.iter().any(|a| a == "--ctl");
        (ctl, LaunchOptions::try_parse_from(args))
    })
}
pub(crate) fn control_options() -> Option<Result<&'static LaunchOptions, String>> {
    let (control, result) = parsed();
    control.then(|| result.as_ref().map_err(|e| e.to_string()))
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
                        return [w.max(850.), h.max(550.)];
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
