//! Deterministic, account-free visual fixtures; only --demo activates them.
use crate::{tr, App, Playlist, RepeatMode, RetryAction, State};
use tg_player_application::model::Track;
pub fn state() -> State {
    State { playlists: (0..5).map(|i| Playlist {
        id: i, title: ["Midnight frequencies", "Saved Messages", "Jazz & coffee", "Electronic archive", "Music · Favorites"][i as usize].into(),
        personal: tg_player_application::model::PersonalChat {
            lists: [i != 3, i == 3],
            saved_messages: i == 1,
        },
        filters: [true,true,false], done: [true;3],
        tracks: (0..40).map(|j| Track {chat:i,message:j,
            title: if j == 0 {"Night Train — Music for the Long Road Home, Live at the Midnight Sessions".into()} else {format!("{} {:02}",["Afterglow","Pacific","Blue hour","Signals","Slow motion"][j as usize%5],j+1)},
            artist: if j == 0 {"The Midnight Station Orchestra featuring The Northbound Quartet".into()} else {["Nils Frahm","Bonobo","Ólafur Arnalds","Tycho"][j as usize%4].into()},
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
            state.queue_panel_open = true;
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
        "offline-storage" => {
            app.settings_open = true;
            let now = tg_player_application::offline::unix_seconds(std::time::SystemTime::now());
            let mut data = offline.data();
            for (chat, count) in [(0, 3), (2, 8), (4, 1)] {
                for track in state.playlists[chat].tracks.iter().take(count) {
                    data.copies
                        .push(tg_player_application::offline::OfflineCopy {
                            track: track.clone(),
                            path: format!("demo-{}-{}", track.chat, track.message),
                            cached: false,
                            size: 6 * 1024 * 1024,
                            last_used: now,
                            handoff_pending: false,
                            remote_id: None,
                        });
                }
            }
            offline = tg_player_application::offline::OfflineCore::new(data);
        }
        "resumed" => {
            state.session.start(&tracks, 1, false);
            state.session.set_position_ms(37_625);
            state.queue_panel_open = true;
        }
        "empty" => state.playlists[0].tracks.clear(),
        "filters" => state.playlists[0].filters = [false; 3],
        "search" => app.search = "no-matching-demo-track".into(),
        "recent-chats" => {
            state.recent_chats = vec![1, 3];
            app.chat_search_active = true;
            app.pane = crate::screens::WorkspacePane::Library;
        }
        "emojis" => {
            use tg_player_application::model::{Caption, CaptionEmoji, EmojiStatus};
            let id = crate::adapters::emoji::DEMO_ID;
            state.playlists[0].title = "Midnight frequencies 🌙".into();
            state.playlists[0].emoji_status = Some(EmojiStatus { id, expires_at: 0 });
            let track = &mut state.playlists[0].tracks[0];
            track.title = "On air 🌗 · Late night radio".into();
            track.artist = "DJ 👍🏽".into();
            track.caption = Some(Caption {
                text: "Live from the studio ✨ with headphones 🎧".into(),
                emoji: vec![CaptionEmoji { bytes: 21..24, id }],
                links: vec![],
            });
            app.custom_emoji.set_demo();
        }
        "queue" | "queue-history" => {
            state.session.start(&tracks, 3, false);
            state.session.jump_to(2);
            state.queue_panel_open = true;
            app.pane = crate::screens::WorkspacePane::Queue;
        }
        _ => {}
    }
    app.state = tg_player_application::AppCore::with_offline(state, offline.data());
    if scenario == "artwork" {
        artwork(app);
    }
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

fn artwork(app: &mut App) {
    use tg_player_application::{
        library::LibraryCommand,
        media::{MediaPlaylist, MediaTrack},
    };
    let Ok(directory) = tempfile::tempdir() else {
        return;
    };
    let playlist = &app.state.view().playlists[0];
    let mut covers = Vec::new();
    for (offset, (a, b)) in [
        ([22, 37, 76], [230, 97, 114]),
        ([35, 85, 74], [232, 192, 93]),
        ([91, 40, 90], [243, 158, 197]),
        ([44, 50, 86], [116, 182, 226]),
        ([97, 43, 38], [238, 182, 103]),
        ([28, 41, 37], [104, 185, 141]),
    ]
    .into_iter()
    .enumerate()
    {
        let path = directory.path().join(format!("cover-{offset}.png"));
        let image: image::RgbImage = image::ImageBuffer::from_fn(96, 96, |x, y| {
            let x = x as f32 / 96.;
            let y = y as f32 / 96.;
            let wave = (y * 30. + 3. * (x * 7. + offset as f32).sin()).sin().abs();
            let sun = ((x - 0.65).powi(2) + (y - 0.35).powi(2)) < 0.052;
            let mix = if sun { 0.9 } else { wave * 0.4 + x * 0.25 };
            image::Rgb(std::array::from_fn(|i| {
                (a[i] as f32 * (1. - mix) + b[i] as f32 * mix) as u8
            }))
        });
        if image.save(&path).is_ok() {
            covers.push(path.to_string_lossy().into_owned());
        }
    }
    if covers.is_empty() {
        return;
    }
    let tracks = playlist
        .tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let cover = i % covers.len();
            MediaTrack {
                file: 1000 + i as i64,
                image_file: cover as i64 + 1,
                image_path: Some(covers[cover].clone()),
                ..track.clone().into()
            }
        })
        .collect();
    app.library_command(LibraryCommand::MergeChat(MediaPlaylist {
        id: playlist.id,
        title: playlist.title.clone(),
        filters: playlist.filters,
        done: playlist.done,
        tracks,
        ..Default::default()
    }));
    app.demo_artwork = Some(directory);
}
