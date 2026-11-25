use crate::{
    adapters::emoji::{self, EmojiAsset},
    App, Event,
};
use eframe::egui::{
    self,
    text::{CCursor, LayoutJob},
    Color32, FontId, Rect, Vec2,
};
use gramwave_application::{
    model::{Caption, CaptionLink, EmojiStatus},
    tasks::{JobClass, Operation, RequestId},
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const FRAME_BUDGET: usize = 64 * 1024 * 1024;
const ANIMATION_FPS: f64 = 12.;

pub(crate) fn install(ctx: &egui::Context, screenshot: bool) {
    fastframe_emoji::EmojiSetup::default()
        .system(!screenshot)
        .bundled(include_bytes!("../assets/fonts/NotoColorEmoji.ttf"))
        .synchronous(screenshot)
        .install();
    if !screenshot {
        std::thread::spawn(fastframe_emoji::warm_up);
    }
    ctx.add_plugin(fastframe_emoji::EmojiPlugin::default());
}

struct Loaded {
    asset: EmojiAsset,
    used: Instant,
    texture: Option<(usize, egui::TextureHandle)>,
}

#[derive(Default)]
struct Visible {
    connection: Option<u64>,
    generation: u64,
    pending: Option<(Operation, Vec<i64>)>,
    requested: Vec<i64>,
    prev_visible: HashSet<i64>,
    loaded: HashMap<i64, Loaded>,
    retry: HashMap<i64, Instant>,
}

#[derive(Default)]
pub(crate) struct CustomEmoji(RefCell<Visible>);

impl CustomEmoji {
    pub(crate) fn set_demo(&self) {
        self.0.borrow_mut().loaded.insert(
            emoji::DEMO_ID,
            Loaded {
                asset: emoji::demo_asset(),
                used: Instant::now(),
                texture: None,
            },
        );
    }

    pub(crate) fn set_connection(&self, connection: Option<u64>) {
        if self.0.borrow().connection != connection {
            self.invalidate();
            self.0.borrow_mut().connection = connection;
        }
    }

    pub(crate) fn release_textures(&self) {
        for loaded in self.0.borrow_mut().loaded.values_mut() {
            loaded.texture = None;
        }
    }

    pub(crate) fn invalidate(&self) {
        let mut state = self.0.borrow_mut();
        if let Some((operation, _)) = state.pending.take() {
            operation.cancel.cancel();
        }
        state.generation = state.generation.wrapping_add(1);
        state.loaded.clear();
        state.retry.clear();
    }

    pub(crate) fn complete(&self, request: RequestId, assets: Vec<(i64, EmojiAsset)>) {
        let mut state = self.0.borrow_mut();
        if state
            .pending
            .as_ref()
            .is_none_or(|(pending, _)| pending.key() != request)
        {
            return;
        }
        let Some((_, ids)) = state.pending.take() else {
            return;
        };
        let now = Instant::now();
        for id in ids {
            state.retry.insert(id, now + Duration::from_secs(30));
        }
        for (id, asset) in assets {
            state.retry.remove(&id);
            state.loaded.insert(
                id,
                Loaded {
                    asset,
                    used: now,
                    texture: None,
                },
            );
        }
        state.trim_frames();
    }

    fn paint(&self, ui: &egui::Ui, rect: Rect, id: i64, tint: Color32, fallback: &str) {
        if !ui.is_rect_visible(rect) {
            return;
        }
        let mut state = self.0.borrow_mut();
        state.requested.push(id);
        let mut alt = fallback;
        if let Some(loaded) = state.loaded.get_mut(&id) {
            loaded.used = Instant::now();
            if let Some(animation) = &loaded.asset.animation {
                let frame = if animation.frames.len() > 1 {
                    // Give short loops at least two ticks so they can show movement.
                    let seconds = animation.duration.as_secs_f64().max(2. / ANIMATION_FPS);
                    let time_in_ticks = ui.input(|input| input.time) * ANIMATION_FPS;
                    let tick = time_in_ticks.floor();
                    let position = (tick / ANIMATION_FPS).rem_euclid(seconds);
                    let until_next_tick = (1. - time_in_ticks.fract()) / ANIMATION_FPS;
                    crate::design::repaint_after(ui, Duration::from_secs_f64(until_next_tick));
                    let frame = (position / seconds * animation.frames.len() as f64) as usize;
                    frame.min(animation.frames.len() - 1)
                } else {
                    0
                };
                let (current, texture) = loaded.texture.get_or_insert_with(|| {
                    (
                        frame,
                        ui.ctx().load_texture(
                            format!("custom-emoji-{id}"),
                            animation.frames[frame].clone(),
                            egui::TextureOptions::LINEAR,
                        ),
                    )
                });
                if *current != frame {
                    texture.set(
                        animation.frames[frame].clone(),
                        egui::TextureOptions::LINEAR,
                    );
                    *current = frame;
                }
                ui.painter().image(
                    texture.id(),
                    rect,
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    if loaded.asset.adaptive {
                        tint
                    } else {
                        Color32::WHITE
                    },
                );
                return;
            }
            if !loaded.asset.fallback.is_empty() {
                alt = &loaded.asset.fallback;
            }
        }
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            alt,
            FontId::proportional(rect.height() * 0.8),
            tint,
        );
    }
}

impl Visible {
    fn trim_frames(&mut self) {
        self.loaded.retain(|id, value| {
            self.prev_visible.contains(id) || value.used.elapsed() < Duration::from_secs(60)
        });
        let bytes = |value: &Loaded| {
            value.asset.animation.as_ref().map_or(0, |animation| {
                animation
                    .frames
                    .iter()
                    .map(|frame| frame.pixels.len() * size_of::<Color32>())
                    .sum::<usize>()
            })
        };
        let mut total: usize = self.loaded.values().map(bytes).sum();
        while total > FRAME_BUDGET {
            if let Some((&id, loaded)) = self
                .loaded
                .iter()
                .filter(|(id, _)| !self.prev_visible.contains(id))
                .min_by_key(|(_, value)| value.used)
            {
                total -= bytes(loaded);
                self.loaded.remove(&id);
                continue;
            }
            let Some(loaded) = self
                .loaded
                .values_mut()
                .filter(|value| {
                    value
                        .asset
                        .animation
                        .as_ref()
                        .is_some_and(|animation| animation.frames.len() > 2)
                })
                .max_by_key(|value| bytes(value))
            else {
                break;
            };
            total -= bytes(loaded);
            if let Some(animation) = &mut loaded.asset.animation {
                let mut index = 0;
                animation.frames.retain(|_| {
                    index += 1;
                    index % 2 == 1
                });
            }
            loaded.texture = None;
            total += bytes(loaded);
        }
    }
}

fn caption_links(caption: &Caption) -> Vec<(CaptionLink, url::Url)> {
    let web_url = |value: &str| {
        let absolute = if value.contains("://") {
            value.to_owned()
        } else if !value.contains(':')
            && value
                .split('/')
                .next()
                .is_some_and(|host| host.contains('.'))
        {
            format!("https://{value}")
        } else {
            return None;
        };
        let url = url::Url::parse(&absolute).ok()?;
        (matches!(url.scheme(), "http" | "https" | "tg")
            && url.has_host()
            && !value.chars().any(char::is_whitespace))
        .then_some(url)
    };
    let mut links: Vec<_> = caption
        .links
        .iter()
        .filter_map(|link| {
            caption
                .text
                .get(link.bytes.clone())
                .filter(|label| !label.is_empty())?;
            Some((link.clone(), web_url(&link.url)?))
        })
        .collect();
    // Older records only stored visible text and custom emoji.
    for token in caption.text.split_whitespace() {
        let value = token
            .trim_start_matches(['(', '[', '{', '<', '"', '“', '«'])
            .trim_end_matches(['.', ',', '!', '?', ')', ']', '}', '>', '"', '”', '»']);
        let lower = value.to_ascii_lowercase();
        if !["https://", "http://", "www.", "tg://"]
            .iter()
            .any(|prefix| lower.starts_with(prefix))
        {
            continue;
        }
        let start = value.as_ptr() as usize - caption.text.as_ptr() as usize;
        let bytes = start..start + value.len();
        if caption
            .links
            .iter()
            .any(|link| link.bytes.start < bytes.end && link.bytes.end > bytes.start)
        {
            continue;
        }
        if let Some(url) = web_url(value) {
            links.push((
                CaptionLink {
                    bytes,
                    url: value.into(),
                },
                url,
            ));
        }
    }
    links.sort_by_key(|(link, _)| link.bytes.start);
    let mut end = 0;
    links.retain(|(link, _)| {
        if link.bytes.start < end {
            return false;
        }
        end = link.bytes.end;
        true
    });
    links
}

impl App {
    pub(crate) fn chat_emoji(
        &self,
        ui: &mut egui::Ui,
        status: Option<EmojiStatus>,
        size: f32,
        anchor: Option<egui::Pos2>,
    ) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let Some(status) =
            status.filter(|status| status.expires_at == 0 || status.expires_at > now)
        else {
            return;
        };
        let (rect, response) = if let Some(anchor) = anchor {
            let rect = Rect::from_min_size(anchor - egui::vec2(0., size / 2.), Vec2::splat(size));
            let response = ui.allocate_rect(rect, egui::Sense::hover());
            (response.rect, response)
        } else {
            ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover())
        };
        if ui.is_rect_visible(rect) && status.expires_at != 0 {
            ui.ctx()
                .request_repaint_after(Duration::from_secs(status.expires_at - now));
        }
        response.on_hover_text(crate::tr("Telegram emoji status"));
        self.custom_emoji
            .paint(ui, rect, status.id, crate::design::colors(ui).accent, "✦");
    }

    pub(crate) fn track_caption(&self, ui: &mut egui::Ui, caption: Option<&Caption>) {
        let Some(caption) = caption else {
            return;
        };
        ui.separator();
        ui.weak(crate::tr("Telegram caption"));
        let max_height = (ui.ctx().content_rect().height() / 3.).min(180.);
        egui::ScrollArea::vertical()
            .id_salt("telegram-caption")
            .max_height(max_height)
            .show(ui, |ui| self.caption_contents(ui, caption));
    }

    fn caption_contents(&self, ui: &mut egui::Ui, caption: &Caption) {
        ui.set_max_width(ui.available_width().min(280.));
        let links = caption_links(caption);
        if caption.emoji.is_empty() && links.is_empty() {
            ui.label(&caption.text);
            return;
        }
        let color = ui.visuals().text_color();
        let format = egui::TextFormat::simple(egui::TextStyle::Body.resolve(ui.style()), color);
        let mut link_format = format.clone();
        link_format.color = ui.visuals().hyperlink_color;
        link_format.underline = egui::Stroke::new(1., link_format.color);
        let mut hidden = format.clone();
        hidden.color = Color32::TRANSPARENT;
        let mut job = LayoutJob::default();
        job.wrap.max_width = ui.available_width();
        let mut placements = Vec::new();
        let mut bounds = vec![None::<std::ops::Range<usize>>; links.len()];
        let mut boundaries = vec![0, caption.text.len()];
        let emoji: Vec<_> = caption
            .emoji
            .iter()
            .filter(|entity| {
                entity.bytes.start < entity.bytes.end
                    && caption.text.get(entity.bytes.clone()).is_some()
            })
            .collect();
        for range in links
            .iter()
            .map(|(link, _)| &link.bytes)
            .chain(emoji.iter().map(|e| &e.bytes))
        {
            boundaries.extend([range.start, range.end]);
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut through = 0;
        let mut index = 0;
        for segment in boundaries.windows(2) {
            let start = segment[0];
            if start < through {
                continue;
            }
            let custom = emoji.iter().find(|entity| entity.bytes.start == start);
            through = custom.map_or(segment[1], |entity| entity.bytes.end);
            let link = links
                .iter()
                .position(|(link, _)| link.bytes.start < through && link.bytes.end > start);
            let part = &caption.text[start..through];
            let before = index;
            if let Some(entity) = custom {
                placements.push((index, entity.id, part));
                job.append("⬛", 0., hidden.clone());
                index += 1;
            } else {
                job.append(
                    part,
                    0.,
                    if link.is_some() {
                        link_format.clone()
                    } else {
                        format.clone()
                    },
                );
                index += part.chars().count();
            }
            if let Some(link) = link {
                bounds[link].get_or_insert(before..index).end = index;
            }
        }
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let (rect, response) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), &caption.text)
        });
        ui.painter().galley(rect.min, galley.clone(), color);
        let mut start = 0;
        let mut focusable = vec![true; links.len()];
        for (row_index, row) in galley.rows.iter().enumerate() {
            let end = start + row.glyphs.len();
            for (link_index, ((link, url), range)) in links.iter().zip(&bounds).enumerate() {
                let Some(range) = range else { continue };
                let from = range.start.max(start);
                let to = range.end.min(end);
                if from >= to {
                    continue;
                }
                let row_rect = row.rect().translate(rect.min.to_vec2());
                let area = row_rect
                    .with_min_x(row_rect.left() + row.x_offset((from - start).into()))
                    .with_max_x(row_rect.left() + row.x_offset((to - start).into()));
                let first = std::mem::take(&mut focusable[link_index]);
                let response = ui.interact(
                    area,
                    response.id.with((link_index, row_index)),
                    if first {
                        egui::Sense::click()
                    } else {
                        egui::Sense::CLICK
                    },
                );
                if first {
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Link,
                            ui.is_enabled(),
                            &caption.text[link.bytes.clone()],
                        )
                    });
                }
                if response.clicked() {
                    if url.scheme() == "tg" {
                        let uri = url.to_string();
                        self.async_event(JobClass::Interactive, 0, async move {
                            if crate::adapters::source_chat::open(&uri).await {
                                Event::Noop
                            } else {
                                Event::SourceChatError(crate::tr("Could not open Telegram. Check that a Telegram desktop app is installed."))
                            }
                        }, |_| Event::Noop);
                    } else {
                        ui.open_url(egui::OpenUrl::new_tab(url.as_str()));
                    }
                    ui.close();
                }
                response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(url.as_str());
            }
            start += row.char_count_including_newline().0;
        }

        for (index, id, fallback) in placements {
            let before = galley.pos_from_cursor(CCursor {
                index: index.into(),
                prefer_next_row: true,
            });
            let after = galley.pos_from_cursor(CCursor::new(index + 1));
            let size = before.height().min((after.left() - before.left()).abs());
            if size > 0. {
                let emoji_rect = Rect::from_center_size(
                    egui::pos2((before.left() + after.left()) * 0.5, before.center().y),
                    Vec2::splat(size),
                )
                .translate(rect.min.to_vec2());
                self.custom_emoji.paint(ui, emoji_rect, id, color, fallback);
            }
        }
    }

    pub(crate) fn emoji_telegram(&self) -> Option<&crate::telegram::Telegram> {
        self.tg.as_ref().filter(|_| {
            !self.quitting
                && *self.state.auth().state()
                    == gramwave_application::telegram::AuthorizationState::Ready
        })
    }

    pub(crate) fn request_custom_emoji(&self, ctx: &egui::Context) {
        let telegram = self.emoji_telegram();
        self.custom_emoji
            .set_connection(telegram.map(|tg| tg.connection));
        let mut state = self.custom_emoji.0.borrow_mut();
        let wanted = std::mem::take(&mut state.requested);
        state.prev_visible = wanted.iter().copied().collect();
        if self.settings_open {
            return;
        }
        let Some(telegram) = telegram else {
            return;
        };
        if state.pending.is_some() {
            return;
        }
        let now = Instant::now();
        state.retry.retain(|_, retry| *retry > now);
        let (mut next, mut ids) = (None::<Instant>, Vec::new());
        for id in wanted {
            if state.loaded.contains_key(&id) || ids.contains(&id) {
                continue;
            }
            if let Some(&retry) = state.retry.get(&id) {
                next = Some(next.map_or(retry, |next| next.min(retry)));
            } else {
                ids.push(id);
                if ids.len() == 8 {
                    break;
                }
            }
        }
        if let Some(next) = next {
            ctx.request_repaint_after(next.saturating_duration_since(now));
        }
        if ids.is_empty() {
            return;
        }
        let operation = Operation::new(state.generation);
        state.pending = Some((operation.clone(), ids.clone()));
        let request = operation.key();
        let cancel = operation.cancel.clone();
        let telegram = telegram.clone();
        self.operation_event(
            JobClass::Background,
            operation,
            async move { Event::Emoji(request, emoji::fetch(&telegram, &ids, &cancel).await) },
            move |_| Event::Emoji(request, Vec::new()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gramwave_application::model::CaptionEmoji;

    #[test]
    fn bundled_emoji_font_draws_joined_color_emoji() {
        let font = fastframe_emoji::EmojiSetup::default()
            .system(false)
            .bundled(include_bytes!("../assets/fonts/NotoColorEmoji.ttf"))
            .load();
        for cluster in ["👍🏽", "🇷🇺", "👩‍💻"] {
            let picture = font.render(cluster, 72).unwrap();
            assert!(picture
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] != 0 && pixel[0] != pixel[1]));
        }
    }

    #[test]
    fn caption_draws_the_custom_texture_and_submits_visible_id() {
        let app = App::new_at(Default::default(), true);
        app.custom_emoji.set_demo();
        let caption = Caption {
            text: "a✨b".into(),
            emoji: vec![CaptionEmoji {
                bytes: 1..4,
                id: emoji::DEMO_ID,
            }],
            links: vec![],
        };
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            app.track_caption(ui, Some(&caption))
        })
        .textures_delta
        .clear();
        let state = app.custom_emoji.0.borrow();
        assert_eq!(state.requested, vec![emoji::DEMO_ID]);
        assert!(state.loaded[&emoji::DEMO_ID].texture.is_some());
    }

    #[test]
    fn clicking_a_wrapped_caption_link_opens_its_destination() {
        let app = App::new_at(Default::default(), true);
        let text = "Read the complete programme and guest list";
        let caption = Caption {
            text: text.into(),
            emoji: vec![],
            links: vec![CaptionLink {
                bytes: 0..text.len(),
                url: "https://example.com/programme".into(),
            }],
        };
        let ctx = egui::Context::default();
        let frame = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(125.);
                    app.caption_contents(ui, &caption);
                },
            );
            output.textures_delta.clear();
            output
        };
        let first = frame(vec![]);
        let click = first
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(label) if label.galley.text() == text => {
                    Some(label.galley.rows[1].rect().center() + label.pos.to_vec2())
                }
                _ => None,
            })
            .unwrap();
        let pointer = |pressed| egui::Event::PointerButton {
            pos: click,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(click), pointer(true)]);
        let output = frame(vec![pointer(false)]);
        assert!(output
            .platform_output
            .commands
            .iter()
            .any(|command| matches!(command,
                egui::OutputCommand::OpenUrl(url) if url.url == "https://example.com/programme"
            )));
    }

    #[test]
    fn legacy_caption_recognizes_visible_web_urls_without_guessing_hidden_targets() {
        let caption: Caption =
            serde_json::from_str(r#"{"text":"(www.example.com). javascript:alert(1)","emoji":[]}"#)
                .unwrap();
        let links = caption_links(&caption);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].1.as_str(), "https://www.example.com/");
    }

    #[test]
    fn visible_emoji_with_different_durations_share_a_repaint_tick() {
        let cache = CustomEmoji::default();
        cache.set_demo();
        let mut second = emoji::demo_asset();
        let short = second.animation.as_mut().unwrap();
        short.frames.truncate(2);
        short.duration = Duration::from_secs_f64(1. / ANIMATION_FPS);
        let second_id = emoji::DEMO_ID + 1;
        cache.0.borrow_mut().loaded.insert(
            second_id,
            Loaded {
                asset: second,
                used: Instant::now(),
                texture: None,
            },
        );
        let ctx = egui::Context::default();
        let draw = |time| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    for id in [emoji::DEMO_ID, second_id] {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::splat(24.), egui::Sense::hover());
                        cache.paint(ui, rect, id, Color32::WHITE, "");
                    }
                },
            );
            let updates = output.textures_delta.set.len();
            output.textures_delta.clear();
            (
                updates,
                output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            )
        };
        draw(0.);
        draw(0.02);
        let (updates, delay) = draw(0.07);
        assert_eq!(updates, 0);
        assert!(delay.abs_diff(Duration::from_micros(13_333)) < Duration::from_micros(1));
        assert_eq!(draw(0.09).0, 2);
    }

    #[test]
    fn reopened_window_uploads_its_own_animated_status_texture() {
        use fastframe_shell::Resident;

        let mut app = App::new_at(Default::default(), true);
        app.custom_emoji.set_demo();
        let status = Some(EmojiStatus {
            id: emoji::DEMO_ID,
            expires_at: 0,
        });
        let draw = |app: &App, ctx: &egui::Context, time| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ui| app.chat_emoji(ui, status, 24., None),
            );
            let id = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh) => Some(mesh.texture_id),
                _ => None,
            });
            let upload = id.and_then(|id| output.textures_delta.set.remove(&id));
            output.textures_delta.clear();
            (id.unwrap(), upload.unwrap()[0].image.clone())
        };
        let original = egui::Context::default();
        let (original_id, first_frame) = draw(&app, &original, 0.);

        app.window_gone();
        let reopened = egui::Context::default();
        let cover = reopened.load_texture(
            "album cover",
            egui::ColorImage::filled([24, 24], Color32::RED),
            egui::TextureOptions::LINEAR,
        );
        assert_eq!(original_id, cover.id());
        let (reopened_id, reopened_frame) = draw(&app, &reopened, 0.);
        assert_ne!(reopened_id, cover.id());
        assert!(reopened_frame == first_frame);
        let (next_id, next_frame) = draw(&app, &reopened, 0.5);
        assert_eq!(next_id, reopened_id);
        assert!(next_frame != first_frame);
    }

    #[test]
    fn cache_invalidation_rejects_pending_emoji_results() {
        let cache = CustomEmoji::default();
        let operation = Operation::new(0);
        cache.0.borrow_mut().pending = Some((operation.clone(), vec![emoji::DEMO_ID]));
        cache.invalidate();
        assert!(operation.cancel.is_cancelled());
        cache.complete(operation.key(), vec![(emoji::DEMO_ID, emoji::demo_asset())]);
        assert!(cache.0.borrow().loaded.is_empty());
    }
}
