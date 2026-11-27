use super::{DesktopApp, Event};
use eframe::egui;
use gramwave_application::tasks::JobClass;
use gramwave_domain::TrackId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::{BufReader, Read},
    path::{Path, PathBuf},
    time::{Instant, SystemTime},
};

const TEXTURES: usize = 512;
const HINTS: usize = 4096;
const DECODES: usize = 2;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub(crate) enum CoverKey {
    Chat(i64),
    Track(TrackId),
}

impl CoverKey {
    fn chat(self) -> i64 {
        match self {
            Self::Chat(chat) => chat,
            Self::Track(track) => track.chat,
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
struct Fingerprint {
    size: u64,
    modified: SystemTime,
}

impl Fingerprint {
    fn of(metadata: &std::fs::Metadata) -> Option<Self> {
        metadata.is_file().then_some(Self {
            size: metadata.len(),
            modified: metadata.modified().ok()?,
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Hint {
    path: String,
    fingerprint: Fingerprint,
}

pub(crate) struct DecodedCover {
    pixels: egui::ColorImage,
    hint: Option<Hint>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum CoverMiss {
    Missing,
    Invalid,
    Interrupted,
}

struct Cover {
    decoded: DecodedCover,
    texture: Option<egui::TextureHandle>,
    used: Instant,
}

#[derive(Default)]
pub(crate) struct Covers {
    root: Option<PathBuf>,
    connection: Option<u64>,
    generation: u64,
    ready: HashMap<String, Cover>,
    pending: HashSet<String>,
    failed: HashMap<String, CoverMiss>,
    stale: HashSet<String>,
    shown: HashMap<CoverKey, String>,
    hints: HashMap<CoverKey, Hint>,
    order: Vec<CoverKey>,
    wanted: Vec<(CoverKey, String, Option<Hint>)>,
    dirtied: bool,
}

impl Covers {
    pub(super) fn begin_frame(&mut self) {
        self.wanted.clear();
    }

    pub(super) fn open(root: &Path, demo: bool) -> Self {
        let mut cache = Self::default();
        if !demo {
            let path = root.join("artwork-index.json");
            if let Ok(file) = std::fs::File::open(path) {
                let decoded: Result<Vec<(CoverKey, Hint)>, _> =
                    serde_json::from_reader(file.take(4 * 1024 * 1024));
                for (key, hint) in decoded.unwrap_or_default().into_iter().take(HINTS) {
                    cache.order.push(key);
                    cache.hints.insert(key, hint);
                }
            }
            cache.root = Some(root.to_owned());
        }
        cache
    }

    pub(crate) fn save(&self) {
        let Some(root) = self.root.as_ref().filter(|_| self.dirtied) else {
            return;
        };
        let entries: Vec<_> = self
            .order
            .iter()
            .filter_map(|key| self.hints.get(key).map(|hint| (key, hint)))
            .collect();
        if let Ok(mut file) = tempfile::NamedTempFile::new_in(root) {
            if serde_json::to_writer(file.as_file_mut(), &entries).is_ok() {
                let _ = file.persist(root.join("artwork-index.json"));
            }
        }
    }

    fn request(&mut self, key: CoverKey, path: Option<&str>, absent: bool) -> Option<&str> {
        if absent {
            self.shown.remove(&key);
            self.remove_hint(key);
            return None;
        }
        // Live paths take priority. The hint can still supply pixels while the live path decodes.
        let hint = self.hints.get(&key).cloned();
        if let Some(path) = path {
            self.wanted.push((key, path.to_owned(), None));
        }
        if let Some(hint) = &hint {
            if path != Some(&hint.path)
                && path.is_none_or(|live| !self.ready.contains_key(live))
                && !self.shown.contains_key(&key)
            {
                self.wanted
                    .push((key, hint.path.clone(), Some(hint.clone())));
            }
        }
        if let Some(path) = path.filter(|path| self.ready.contains_key(*path)) {
            return Some(self.bind(key, path));
        }
        if let Some(hint) = hint.filter(|hint| self.ready.contains_key(&hint.path)) {
            if !self.shown.contains_key(&key) {
                return Some(self.bind(key, &hint.path));
            }
        }
        self.shown.get(&key).map(String::as_str)
    }

    fn bind(&mut self, key: CoverKey, path: &str) -> &str {
        if let Some(hint) = self.ready[path]
            .decoded
            .hint
            .clone()
            .filter(|_| !self.stale.contains(path))
        {
            if self
                .hints
                .get(&key)
                .is_none_or(|old| old.path != hint.path || old.fingerprint != hint.fingerprint)
            {
                self.remove_hint(key);
                if self.order.len() == HINTS {
                    self.hints.remove(&self.order.remove(0));
                }
                self.order.push(key);
                self.hints.insert(key, hint);
                self.dirtied = true;
            }
        }
        self.shown
            .entry(key)
            .and_modify(|old| {
                if old != path {
                    *old = path.to_owned();
                }
            })
            .or_insert_with(|| path.to_owned())
    }

    fn remove_hint(&mut self, key: CoverKey) {
        if self.hints.remove(&key).is_some() {
            self.order.retain(|entry| *entry != key);
            self.dirtied = true;
        }
    }

    pub(crate) fn release_textures(&mut self) {
        for cover in self.ready.values_mut() {
            cover.texture = None;
        }
    }

    pub(super) fn new_connection(&mut self, connection: Option<u64>) {
        if self.connection != connection {
            self.connection = connection;
            self.generation = self.generation.wrapping_add(1);
            self.shown.clear();
            self.ready.clear();
            self.failed.clear();
            self.stale.clear();
            self.wanted.clear();
        }
    }

    pub(super) fn file_ready(&mut self, path: &str) {
        self.failed.remove(path);
    }

    pub(super) fn evicted(&mut self, path: &str) {
        self.failed.remove(path);
        if self.ready.contains_key(path) {
            self.stale.insert(path.to_owned());
        }
        self.hints.retain(|_, hint| hint.path != path);
        self.order.retain(|key| self.hints.contains_key(key));
        self.dirtied = true;
    }

    pub(crate) fn clear(&mut self, chat: Option<i64>) {
        self.generation = self.generation.wrapping_add(1);
        self.failed.clear();
        self.wanted.clear();
        let selected = |key: &CoverKey| chat.is_none_or(|chat| chat == key.chat());
        let removed: HashSet<_> = self
            .shown
            .iter()
            .filter(|(key, _)| selected(key))
            .map(|(_, path)| path.clone())
            .collect();
        self.shown.retain(|key, _| !selected(key));
        self.hints.retain(|key, _| !selected(key));
        self.order.retain(|key| self.hints.contains_key(key));
        self.ready.retain(|path, _| {
            !removed.contains(path) || self.shown.values().any(|shown| shown == path)
        });
        if chat.is_none() {
            self.ready.clear();
        }
        self.stale.retain(|path| self.ready.contains_key(path));
        self.dirtied = true;
        self.save();
    }

    pub(super) fn complete(
        &mut self,
        generation: u64,
        path: String,
        result: Result<DecodedCover, CoverMiss>,
    ) {
        self.pending.remove(&path);
        if generation != self.generation {
            return;
        }
        match result {
            Ok(decoded) => {
                self.stale.remove(&path);
                self.ready.insert(
                    path,
                    Cover {
                        decoded,
                        texture: None,
                        used: Instant::now(),
                    },
                );
            }
            Err(CoverMiss::Interrupted) => {}
            Err(error) => {
                self.evicted(&path);
                if self.failed.len() >= HINTS {
                    self.failed.clear();
                }
                self.failed.insert(path, error);
            }
        }
    }
}

impl DesktopApp {
    pub(crate) fn cover_texture(
        &self,
        ctx: &egui::Context,
        key: CoverKey,
        path: Option<&str>,
        absent: bool,
    ) -> Option<egui::load::SizedTexture> {
        let mut covers = self.covers.borrow_mut();
        let path = covers.request(key, path, absent)?.to_owned();
        let cover = covers.ready.get_mut(&path)?;
        cover.used = Instant::now();
        let texture = cover.texture.get_or_insert_with(|| {
            ctx.load_texture(
                "cover",
                cover.decoded.pixels.clone(),
                egui::TextureOptions::LINEAR,
            )
        });
        Some(egui::load::SizedTexture::from_handle(texture))
    }

    pub(crate) fn track_cover(
        &self,
        ctx: &egui::Context,
        track: &crate::Track,
    ) -> Option<egui::load::SizedTexture> {
        self.cover_texture(
            ctx,
            CoverKey::Track(track.id()),
            track.image_path.as_deref(),
            (track.file != 0 && track.image_file <= 0 && track.image_path.is_none())
                || (track.file == 0 && self.state.library().source_changed(track.id())),
        )
    }

    pub(crate) fn chat_cover(
        &self,
        ctx: &egui::Context,
        chat: i64,
    ) -> Option<egui::load::SizedTexture> {
        let art = self.state.library().playlist_artwork(chat);
        self.cover_texture(
            ctx,
            CoverKey::Chat(chat),
            art.path.as_deref(),
            self.state.library().knows_playlist_artwork(chat)
                && art.file <= 0
                && art.path.is_none(),
        )
    }

    pub(super) fn pump_cover_decodes(&self, ctx: &egui::Context) {
        let mut covers = self.covers.borrow_mut();
        let wanted = std::mem::take(&mut covers.wanted);
        let generation = covers.generation;
        let hot: HashSet<_> = wanted
            .iter()
            .take(128)
            .flat_map(|(key, path, _)| {
                std::iter::once(path.clone()).chain(covers.shown.get(key).cloned())
            })
            .collect();
        while covers.ready.len() > TEXTURES {
            let victim = covers
                .ready
                .iter()
                .filter(|(path, _)| !hot.contains(path.as_str()))
                .min_by_key(|(_, cover)| cover.used)
                .map(|(path, _)| path.clone());
            let Some(victim) = victim else { break };
            covers.ready.remove(&victim);
            covers.stale.remove(&victim);
            covers.shown.retain(|_, path| path != &victim);
        }
        let mut uploads = 0;
        for (key, path, hint) in wanted {
            if covers.ready.contains_key(&path) && !covers.stale.contains(&path) {
                if hint.is_none() || !covers.shown.contains_key(&key) {
                    covers.bind(key, &path);
                }
                let cover = covers.ready.get_mut(&path).unwrap();
                if cover.texture.is_none() && uploads < 16 {
                    cover.texture = Some(ctx.load_texture(
                        "cover",
                        cover.decoded.pixels.clone(),
                        egui::TextureOptions::LINEAR,
                    ));
                    uploads += 1;
                }
                continue;
            }
            if covers.pending.len() >= DECODES
                || covers.failed.contains_key(&path)
                || !covers.pending.insert(path.clone())
            {
                continue;
            }
            let root = covers.root.clone();
            let failed = path.clone();
            self.async_event(
                JobClass::Background,
                generation,
                async move {
                    let work_path = path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        decode(&work_path, root.as_deref(), hint.as_ref()).ok_or_else(|| {
                            if Path::new(&work_path).is_file() {
                                CoverMiss::Invalid
                            } else {
                                CoverMiss::Missing
                            }
                        })
                    })
                    .await
                    .unwrap_or(Err(CoverMiss::Interrupted));
                    Event::CoverDecoded(generation, path, result)
                },
                move |_| {
                    Event::CoverDecoded(generation, failed.clone(), Err(CoverMiss::Interrupted))
                },
            );
        }
    }

    pub(super) fn cover_decode_ready(&self, path: &str) -> bool {
        let covers = self.covers.borrow();
        covers.ready.contains_key(path) && !covers.stale.contains(path)
    }

    pub(super) fn cover_decode_failed(&self, path: &str) -> bool {
        self.covers.borrow().failed.contains_key(path)
    }

    pub(super) fn cover_needs_file(&self, path: &str) -> bool {
        self.covers.borrow().failed.get(path) == Some(&CoverMiss::Missing)
    }
}

fn decode(path: &str, root: Option<&Path>, hint: Option<&Hint>) -> Option<DecodedCover> {
    let local = |root: &Path| {
        let cache = root.join("cache").canonicalize().ok()?;
        let path = Path::new(path).canonicalize().ok()?;
        path.starts_with(cache).then_some(path)
    };
    let verified = root.and_then(local);
    if hint.is_some() && verified.is_none() {
        return None;
    }
    let file = std::fs::File::open(verified.as_deref().unwrap_or_else(|| Path::new(path))).ok()?;
    let fingerprint = Fingerprint::of(&file.metadata().ok()?)?;
    if fingerprint.size > 8 * 1024 * 1024
        || hint.is_some_and(|hint| fingerprint != hint.fingerprint)
    {
        return None;
    }
    let mut reader = image::ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    let image = if image.width().max(image.height()) > 160 {
        image.thumbnail(160, 160)
    } else {
        image
    };
    let pixels = image.into_rgba8();
    Some(DecodedCover {
        pixels: egui::ColorImage::from_rgba_unmultiplied(
            [pixels.width() as usize, pixels.height() as usize],
            pixels.as_raw(),
        ),
        hint: verified.map(|_| Hint {
            path: path.to_owned(),
            fingerprint,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_cover_hints_survive_restart_and_reject_changed_files() {
        let profile = tempfile::tempdir().unwrap();
        let cache = profile.path().join("cache");
        std::fs::create_dir(&cache).unwrap();
        let path = cache.join("telegram-photo");
        let image = image::RgbImage::from_pixel(4, 4, image::Rgb([12, 34, 56]));
        image
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();
        let path = path.to_str().unwrap();
        let key = CoverKey::Chat(42);
        let mut covers = Covers::open(profile.path(), false);
        let decoded = decode(path, Some(profile.path()), None).unwrap();
        covers.complete(0, path.to_owned(), Ok(decoded));
        assert_eq!(covers.request(key, Some(path), false), Some(path));
        covers.save();

        let mut reopened = Covers::open(profile.path(), false);
        assert!(reopened.request(key, None, false).is_none());
        let (_, path, hint) = reopened.wanted.pop().unwrap();
        let hint = hint.unwrap();
        let decoded = decode(&path, Some(profile.path()), Some(&hint)).unwrap();
        assert_eq!(decoded.pixels.size, [4, 4]);
        std::fs::write(&path, b"changed").unwrap();
        assert!(decode(&path, Some(profile.path()), Some(&hint)).is_none());
        reopened.clear(Some(42));
        assert!(Covers::open(profile.path(), false).hints.is_empty());
    }

    #[test]
    fn shown_cover_survives_eviction_until_its_replacement_is_ready() {
        let key = CoverKey::Chat(42);
        let mut covers = Covers::default();
        let decoded = || DecodedCover {
            pixels: egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
            hint: None,
        };
        covers.complete(0, "old".into(), Ok(decoded()));
        assert_eq!(covers.request(key, Some("old"), false), Some("old"));
        covers.evicted("old");
        assert_eq!(covers.request(key, Some("new"), false), Some("old"));
        covers.complete(0, "new".into(), Ok(decoded()));
        assert_eq!(covers.request(key, Some("new"), false), Some("new"));
        assert_eq!(covers.request(key, None, true), None);
    }
}
