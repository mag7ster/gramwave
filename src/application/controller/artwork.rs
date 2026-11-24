use super::{DesktopApp, PlaylistCacheKey, Track};
use crate::screens::{artwork_image, TRACK_ARTWORK_SIZE};
use eframe::egui;
use std::{ops::Range, sync::Arc, time::Instant};
use tg_player_application::library::LibraryCommand;

const NEARBY_ROWS: usize = 30;
const SCAN_BATCH: usize = 64;
const BACKGROUND_REQUESTS: usize = 2;
const WARMING_TEXTURES: usize = 4;

pub(super) struct PlaylistView {
    key: PlaylistCacheKey,
    tracks: Arc<Vec<Track>>,
    visible: Range<usize>,
}

impl PlaylistView {
    fn nearby(&self) -> impl Iterator<Item = usize> + '_ {
        (0..NEARBY_ROWS).flat_map(|distance| {
            [
                self.visible
                    .end
                    .checked_add(distance)
                    .filter(|&i| i < self.tracks.len()),
                self.visible.start.checked_sub(distance + 1),
            ]
            .into_iter()
            .flatten()
        })
    }
}

#[derive(Default)]
pub(super) struct Prefetch {
    scope: Option<(u64, PlaylistCacheKey)>,
    next: usize,
    warming: Vec<String>,
}

impl DesktopApp {
    pub(crate) fn note_playlist_view(
        &mut self,
        selected: usize,
        tracks: Arc<Vec<Track>>,
        visible: Range<usize>,
    ) {
        self.artwork_view = Some(PlaylistView {
            key: self.playlist_cache_key(selected, &self.search),
            tracks,
            visible,
        });
    }

    pub(super) fn request_playlist_artwork(
        &mut self,
        ctx: &egui::Context,
        view: Option<PlaylistView>,
        now: Instant,
    ) {
        let Some(view) = view else {
            self.artwork_prefetch.scope = None;
            return;
        };
        let scope = (self.state.library().connection(), view.key.clone());
        if self.artwork_prefetch.scope.as_ref() != Some(&scope) {
            self.artwork_prefetch.scope = Some(scope);
            self.artwork_prefetch.next = 0;
        }
        self.warm_nearby_textures(ctx, &view);

        if self.tg.is_none() || !self.state.library().connected() {
            return;
        }
        for i in view.nearby() {
            if self.state.library().media_requests_in_flight() >= BACKGROUND_REQUESTS {
                break;
            }
            self.prefetch_cover(&view.tracks[i], now);
        }

        let start = self.artwork_prefetch.next;
        let end = (start + SCAN_BATCH).min(view.tracks.len());
        for i in start..end {
            if self.prefetch_cover(&view.tracks[i], now) && self.artwork_prefetch.next == i {
                self.artwork_prefetch.next += 1;
            }
            if self.state.library().media_requests_in_flight() >= BACKGROUND_REQUESTS {
                break;
            }
        }
        if self.artwork_prefetch.next == end && end < view.tracks.len() {
            ctx.request_repaint();
        }
    }

    // A failed offscreen request can wait for the next visit. The existing
    // visible-row retry path still schedules its usual backoff.
    fn prefetch_cover(&mut self, track: &Track, now: Instant) -> bool {
        let library = self.state.library();
        let command = if let Some(media) = library.runtime_media(track.id()).filter(|m| m.file != 0)
        {
            let id = media.image_file;
            if id <= 0
                || library.artwork_path(id).is_some()
                || library.artwork_retry_at(id).is_some_and(|at| at > now)
            {
                return true;
            }
            LibraryCommand::RequestArtwork {
                id,
                now,
                force: false,
            }
        } else {
            if library
                .media_retry_at(track.id())
                .is_some_and(|at| at > now)
            {
                return true;
            }
            LibraryCommand::RequestMedia {
                track: track.metadata(),
                now,
            }
        };
        if library.media_requests_in_flight() < BACKGROUND_REQUESTS {
            self.library_command(command);
        }
        false
    }

    fn warm_nearby_textures(&mut self, ctx: &egui::Context, view: &PlaylistView) {
        let size = egui::Vec2::splat(TRACK_ARTWORK_SIZE);
        let pending = |path: &str| {
            artwork_image(path, TRACK_ARTWORK_SIZE)
                .load_for_size(ctx, size)
                .is_ok_and(|poll| poll.is_pending())
        };
        self.artwork_prefetch
            .warming
            .retain(|path| std::path::Path::new(path).is_file() && pending(path));
        let library = self.state.library();
        for i in view.nearby() {
            if self.artwork_prefetch.warming.len() >= WARMING_TEXTURES {
                break;
            }
            let Some(path) = library
                .runtime_media(view.tracks[i].id())
                .and_then(|media| library.artwork_path(media.image_file))
            else {
                continue;
            };
            if !self
                .artwork_prefetch
                .warming
                .iter()
                .any(|existing| existing == path)
                && std::path::Path::new(path).is_file()
                && pending(path)
            {
                self.artwork_prefetch.warming.push(path.to_owned());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::telegram::Telegram;
    use serde_json::json;
    use std::{thread, time::Duration};
    use tg_player_application::{
        model::{Playlist, State},
        AppCore,
    };

    #[test]
    fn prefetch_prioritizes_a_jump_and_finishes_the_open_playlist() {
        const TRACKS: i64 = 104;
        let mut app = DesktopApp::new_at(Default::default(), true);
        app.state = AppCore::new(State {
            playlists: vec![Playlist {
                id: 42,
                filters: [true; 3],
                done: [true; 3],
                tracks: (1..=TRACKS)
                    .map(|message| tg_player_application::model::Track {
                        chat: 42,
                        message,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        });
        let cover = tempfile::NamedTempFile::new().unwrap();
        let path = cover.path().to_string_lossy().into_owned();
        let (send, downloads) = std::sync::mpsc::channel();
        let tg = Telegram::mock_rpc(move |request| match request["@type"].as_str() {
            Some("getMessage") => {
                assert_eq!(request["chat_id"], 42);
                let message = request["message_id"].as_i64().unwrap();
                json!({"id":message,"content":{"@type":"messageAudio","audio":{
                    "audio":{"id":2000+message},"album_cover_thumbnail":{"file":{"id":1000+message}}
                }}})
            }
            Some("downloadFile") => {
                let id = request["file_id"].as_i64().unwrap();
                assert!((1001..=1000 + TRACKS).contains(&id));
                send.send(id).unwrap();
                json!({"@type":"file","id":id,"local":{"path":path,"is_downloading_completed":true}})
            }
            _ => panic!("Unexpected request {request}"),
        });
        app.library_command(LibraryCommand::Connected(tg.connection));
        app.tg = Some(tg);
        let ctx = egui::Context::default();
        let tracks = app.playlist_tracks(0);
        let tick = |app: &mut DesktopApp, visible: Range<usize>| {
            let now = Instant::now();
            app.process(&ctx);
            for track in &tracks[visible.clone()] {
                let view = app.media_track(track.metadata());
                app.image_wanted.push(view.image_file);
                if view.file == 0 {
                    app.track_wanted.push(view);
                }
            }
            app.note_playlist_view(0, tracks.clone(), visible);
            app.request_visible_media(&ctx);
            now
        };
        tick(&mut app, 0..1);
        let deadline = Instant::now() + Duration::from_secs(8);
        while app.artwork_path(1081).is_none() {
            assert!(tick(&mut app, 80..81) < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(
            app.artwork_path(1048).is_none(),
            "distant background row ran ahead of the jump"
        );
        while app.artwork_prefetch.next < TRACKS as usize {
            assert!(tick(&mut app, 80..81) < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        let mut ids: Vec<_> = downloads.try_iter().collect();
        ids.sort_unstable();
        assert_eq!(ids, (1001..=1000 + TRACKS).collect::<Vec<_>>());
        assert!(Arc::ptr_eq(&tracks, &app.playlist_tracks(0)));
    }
}
