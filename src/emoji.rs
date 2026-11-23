use crate::{
    adapters::emoji::{self, EmojiAsset},
    App, Event,
};
use eframe::egui::{
    self,
    text::{CCursor, LayoutJob},
    Color32, FontId, Rect, Vec2,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tg_player_application::{
    model::{Caption, EmojiStatus},
    tasks::{JobClass, Operation, RequestId},
};

const FRAME_BUDGET: usize = 64 * 1024 * 1024;

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
                    let seconds = animation.duration.as_secs_f64();
                    let step = seconds / animation.frames.len() as f64;
                    let position = ui.input(|input| input.time).rem_euclid(seconds);
                    ui.ctx()
                        .request_repaint_after(Duration::from_secs_f64(step - position % step));
                    ((position / step) as usize).min(animation.frames.len() - 1)
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
        if caption.emoji.is_empty() {
            ui.label(&caption.text);
            return;
        }
        let color = ui.visuals().text_color();
        let format = egui::TextFormat::simple(egui::TextStyle::Body.resolve(ui.style()), color);
        let mut hidden = format.clone();
        hidden.color = Color32::TRANSPARENT;
        let mut job = LayoutJob::default();
        job.wrap.max_width = ui.available_width();
        let mut end = 0;
        let mut placements = Vec::new();
        for entity in &caption.emoji {
            if let (Some(prefix), Some(fallback)) = (
                caption.text.get(end..entity.bytes.start),
                caption.text.get(entity.bytes.clone()),
            ) {
                job.append(prefix, 0., format.clone());
                placements.push((job.text.chars().count(), entity.id, fallback));
                job.append("⬛", 0., hidden.clone());
                end = entity.bytes.end;
            }
        }
        job.append(&caption.text[end..], 0., format);
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let (rect, response) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), &caption.text)
        });
        ui.painter().galley(rect.min, galley.clone(), color);
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
                    == tg_player_application::telegram::AuthorizationState::Ready
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
    use tg_player_application::model::CaptionEmoji;

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
