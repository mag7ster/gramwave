//! Deterministic, account-free visual fixtures; only --demo activates them.
use crate::{tr, App, Playlist, RepeatMode, RetryAction, State};
use tg_player_application::model::Track;
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
    let mut state = app.state.view().clone();
    state.light = app.options.light;
    app.state =
        tg_player_application::AppCore::with_offline(state, app.state.offline().data().clone());
    app.opened_chat = Some(0);
    if let Some(scenario) = app.options.scenario.clone() {
        apply(app, &scenario);
    }
}
pub fn apply(app: &mut App, scenario: &str) {
    let mut state = app.state.view().clone();
    let tracks = state.playlists[0].tracks.clone();
    let mut offline = tg_player_application::offline::OfflineCore::new(app.state.offline().data());
    match scenario {
        "playing" | "paused" | "loading" | "error" => {
            state.session.start(&tracks, 0, true);
            state.session.insert(tracks[4].clone(), true);
            state.session.insert(tracks[7].clone(), false);
            state.shuffle = true;
            state.repeat = RepeatMode::One;
            app.queue_open = true;
            offline.enqueue(tracks[1].clone(), true);
            offline.publish(tracks[3].clone(), "demo-offline".into());
            if scenario == "error" {
                app.error = tr("Unable to play track");
                app.error_retry = Some(RetryAction::Playback(tracks[0].clone()));
            }
            if scenario == "loading" {
                // Loading playback is represented by the application reducer.
            }
        }
        "settings" => app.settings_open = true,
        "empty" => state.playlists[0].tracks.clear(),
        "filters" => state.playlists[0].filters = [false; 3],
        "search" => app.search = "no-matching-demo-track".into(),
        "queue" => app.queue_open = true,
        _ => {}
    }
    app.state = tg_player_application::AppCore::with_offline(state, offline.data());
    if matches!(scenario, "playing" | "paused" | "loading" | "error") {
        use tg_player_application::{
            commands::AppCommand, ports::PlaybackEvent, workflow::AppEvent,
        };
        app.state.command(AppCommand::Play(tracks[0].clone()));
        if scenario != "loading" {
            app.state.event(AppEvent::Player(PlaybackEvent::Loaded));
        }
        app.state
            .command(AppCommand::Pause(matches!(scenario, "paused" | "error")));
    }
}
