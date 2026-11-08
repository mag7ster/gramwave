//! Deterministic, account-free visual fixtures; only --demo activates them.
use crate::*;
fn option(prefix: &str) -> Option<String> {
    std::env::args().find_map(|a| a.strip_prefix(prefix).map(str::to_owned))
}
pub fn state() -> State {
    State { playlists: (0..5).map(|i| Playlist {
        id: i, title: ["Midnight frequencies", "Saved Messages", "Jazz & coffee", "Electronic archive", "Музыка · Избранное"][i as usize].into(),
        filters: [true,true,false], done: [true;3],
        tracks: (0..40).map(|j| Track {chat:i,message:j,
            title: if j == 0 {"Ночной поезд — музыка для долгой дороги / A very long title that should gracefully truncate".into()} else {format!("{} {:02}",["Afterglow","Pacific","Blue hour","Signals","Slow motion"][j as usize%5],j+1)},
            artist: if j == 0 {"Композитор · Исполнитель с длинным именем / Live session".into()} else {["Nils Frahm","Bonobo","Ólafur Arnalds","Tycho"][j as usize%4].into()},
            duration: if j == 2 {0} else {180+j as u64*3}, ..Default::default() }).collect(),
        ..Default::default()
    }).collect(),
    chat_pins: vec![0, 2, 4], ..Default::default() }
}
pub fn configure(app: &mut App) {
    if !app.demo {
        return;
    }
    app.state.light = std::env::args().any(|a| a == "--light");
    app.opened_chat = Some(0);
    if let Some(scenario) = option("--scenario=") {
        apply(app, &scenario);
    }
}
pub fn apply(app: &mut App, scenario: &str) {
    let tracks = app.state.playlists[0].tracks.clone();
    match scenario {
        "playing" | "paused" | "loading" | "error" => {
            app.state.session.start(&tracks, 0, true);
            // Reproducible order, with visibly shuffled automatic continuation.
            app.state.session.remaining = tracks.iter().skip(1).rev().cloned().collect();
            app.state.queue = vec![tracks[4].clone(), tracks[7].clone()];
            app.state.shuffle = true;
            app.state.repeat = RepeatMode::One;
            app.queue_open = true;
            app.paused = scenario == "paused";
            app.loading = scenario == "loading";
            app.state.pending_pins.push(tracks[1].clone());
            app.state.playlists[0].tracks[3].pinned = Some("demo-offline".into());
            app.pin_errors.insert((0, 5), tr("Download interrupted"));
            if scenario == "error" {
                app.error = tr("Unable to play track");
                app.error_retry = Some(RetryAction::Playback(tracks[0].clone()));
                app.paused = true;
            }
            if scenario == "loading" {
                app.indexing_chat = Some(0);
            }
        }
        "settings" => app.settings_open = true,
        "empty" => app.state.playlists[0].tracks.clear(),
        "filters" => app.state.playlists[0].filters = [false; 3],
        "search" => app.search = "no-matching-demo-track".into(),
        "queue" => app.queue_open = true,
        _ => {}
    }
}
pub fn window_size() -> [f32; 2] {
    if std::env::args().any(|a| a == "--demo") {
        if let Some(size) = option("--size=") {
            if let Some((w, h)) = size.split_once('x') {
                if let (Ok(w), Ok(h)) = (w.parse::<f32>(), h.parse::<f32>()) {
                    return [w.max(850.), h.max(550.)];
                }
            }
        }
    }
    [1240., 800.]
}
pub fn scale() -> Option<f32> {
    std::env::args()
        .any(|a| a == "--demo")
        .then(|| {
            option("--scale=")
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| v.is_finite() && *v >= 0.5 && *v <= 3.)
        })
        .flatten()
}
pub fn preview_path() -> PathBuf {
    option("--screenshot=")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/tg-player-preview.png".into())
}
