use crate::{
    design,
    telegram::{CacheCategory, CacheFilter, CacheReport, CacheSelection, CacheStats},
    tr, App, Event,
};
use eframe::egui;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tg_player_application::{
    persistence::CachePolicy, playback::PlaybackState, tasks::JobClass,
    telegram::AuthorizationState, AppError,
};

#[derive(Clone, Copy)]
pub(crate) enum CacheJob {
    Inspect,
    Clear,
    Maintain(CachePolicy),
}

#[derive(Default)]
pub(crate) enum CachePlayback {
    #[default]
    Stream,
    Local,
    Offline,
}

struct PendingClear {
    remaining: CacheFilter,
    freed: Option<u64>,
    cancelled: bool,
}

const RETENTION_DAYS: [u32; 8] = [1, 3, 7, 14, 30, 90, 180, 365];
const SIZE_MIB: [u32; 9] = [64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384];

#[derive(Default)]
pub(crate) struct CacheState {
    stats: Option<CacheStats>,
    pub(crate) offline_sizes: HashMap<String, u64>,
    error: Option<String>,
    notice: Option<String>,
    job: Option<CacheJob>,
    pending: Option<PendingClear>,
    inspected: bool,
    chat: Option<i64>,
    selected: CacheSelection,
    maintain_at: Option<Instant>,
    custom_days: bool,
}

impl App {
    fn cache_ready(&self) -> bool {
        self.tg.is_some() && *self.state.auth().state() == AuthorizationState::Ready
    }

    fn cache_audio_available(&self) -> bool {
        let playback = self.state.playback().state();
        !playback.loading()
            && self.player.as_ref().is_none_or(|player| {
                player.is_idle()
                    || (matches!(playback, PlaybackState::Playing | PlaybackState::Paused)
                        && match self.cache_playback {
                            CachePlayback::Stream => false,
                            CachePlayback::Local => player.snapshot().duration > 0.,
                            CachePlayback::Offline => true,
                        })
            })
    }

    fn inspect_cache(&mut self) {
        if self.cache.job.is_some() || !self.cache_ready() {
            return;
        }
        let tg = self.tg.clone().unwrap();
        let connection = tg.connection;
        let job = CacheJob::Inspect;
        self.cache.job = Some(job);
        let offline_root = self.root.join("offline");
        let paths = self
            .state
            .offline()
            .copies
            .values()
            .map(|copy| copy.path.clone())
            .collect();
        self.async_event(
            JobClass::Background,
            connection,
            async move {
                let (result, offline_sizes) = tokio::join!(
                    tg.cache_stats(),
                    crate::adapters::offline::saved_sizes(offline_root, paths),
                );
                Event::Cache {
                    connection,
                    job,
                    result: result.map(|stats| CacheReport {
                        stats,
                        freed: None,
                        error: None,
                    }),
                    offline_sizes: Some(offline_sizes),
                }
            },
            move |error| Event::Cache {
                connection,
                job,
                result: Err(error),
                offline_sizes: None,
            },
        );
    }

    fn clean_cache(&mut self, job: CacheJob, filter: CacheFilter) -> bool {
        if self.cache.job.is_some() || !self.cache_ready() {
            return false;
        }
        let tg = self.tg.clone().unwrap();
        let Some(access) = tg.try_cache_write(filter, self.cache_audio_available()) else {
            return false;
        };
        let connection = tg.connection;
        let policy = match job {
            CacheJob::Maintain(policy) => Some(policy),
            _ => None,
        };
        if policy.is_some() && access.filter.categories != filter.categories {
            return false;
        }
        if matches!(job, CacheJob::Clear) {
            let pending = self.cache.pending.as_mut().unwrap();
            for (remaining, scheduled) in pending
                .remaining
                .categories
                .iter_mut()
                .zip(access.filter.categories)
            {
                *remaining &= !scheduled;
            }
        }
        self.cache.job = Some(job);
        // Releasing the cache must not wait for background slots used by asset readers.
        self.async_event(
            JobClass::Interactive,
            connection,
            async move {
                Event::Cache {
                    connection,
                    job,
                    result: tg.optimize_cache(access, policy).await,
                    offline_sizes: None,
                }
            },
            move |error| Event::Cache {
                connection,
                job,
                result: Err(error),
                offline_sizes: None,
            },
        );
        true
    }

    pub(crate) fn cache_finished(
        &mut self,
        connection: u64,
        job: CacheJob,
        result: Result<CacheReport, AppError>,
        offline_sizes: Option<HashMap<String, u64>>,
    ) {
        if self.tg.as_ref().map(|tg| tg.connection) != Some(connection) {
            return;
        }
        self.cache.job = None;
        if let Some(sizes) = offline_sizes {
            self.cache.offline_sizes = sizes;
        }
        if let CacheJob::Maintain(completed_policy) = job {
            let retry = if completed_policy != self.state.view().cache_policy {
                0
            } else if matches!(&result, Ok(CacheReport { error: None, .. })) {
                3600
            } else {
                300
            };
            self.cache.maintain_at = Some(Instant::now() + Duration::from_secs(retry));
        }
        let (freed, error) = match result {
            Ok(CacheReport {
                stats,
                freed,
                error,
            }) => {
                if !matches!(job, CacheJob::Inspect)
                    && self.cache.stats.as_ref().is_none_or(|before| {
                        let emoji = CacheCategory::Emoji as usize;
                        before.total.by_category[emoji] > stats.total.by_category[emoji]
                    })
                {
                    self.custom_emoji.invalidate();
                }
                self.cache.stats = Some(stats);
                (freed, error)
            }
            Err(error) => (None, Some(error)),
        };
        if matches!(job, CacheJob::Clear) {
            if let Some(mut pending) = self.cache.pending.take() {
                if let Some(freed) = freed {
                    pending.freed = Some(pending.freed.unwrap_or_default().saturating_add(freed));
                }
                let more = pending
                    .remaining
                    .categories
                    .into_iter()
                    .any(|selected| selected);
                if let Some(freed) = pending.freed.filter(|&bytes| bytes > 0) {
                    self.cache.notice = Some(
                        tr("Removed from cache · {size}").replace("{size}", &cache_size(freed)),
                    );
                } else if error.is_none() && !more && !pending.cancelled {
                    self.cache.notice = Some(if pending.freed.is_some() {
                        tr("No cache files were removed.")
                    } else {
                        tr("Cache cleanup finished.")
                    });
                }
                if error.is_none() && more {
                    self.cache.pending = Some(pending);
                } else if error.is_none() && !pending.cancelled {
                    self.cache.selected.fill(false);
                }
            }
        }
        if let Some(error) = error.filter(|_| !matches!(job, CacheJob::Maintain(_))) {
            self.cache.error = Some(error.to_string());
        }
    }

    pub(crate) fn maintain_cache(&mut self, now: Instant) -> Option<Instant> {
        if self.quitting || !self.cache_ready() {
            return None;
        }
        if self.cache.job.is_some() {
            return None;
        }
        if let Some(filter) = self.cache.pending.as_ref().map(|pending| pending.remaining) {
            return (!self.clean_cache(CacheJob::Clear, filter))
                .then_some(now + Duration::from_millis(250));
        }
        let deadline = *self
            .cache
            .maintain_at
            .get_or_insert(now + Duration::from_secs(30));
        if deadline > now {
            return Some(deadline);
        }
        if !self.settings_open
            && self.clean_cache(
                CacheJob::Maintain(self.state.view().cache_policy),
                CacheFilter {
                    chat: None,
                    categories: [true; CacheCategory::ALL.len()],
                },
            )
        {
            return None;
        }
        let retry = now + Duration::from_secs(30);
        self.cache.maintain_at = Some(retry);
        Some(retry)
    }

    pub(crate) fn cache_policy_changed(&mut self) {
        self.cache.maintain_at = Some(Instant::now());
    }

    pub(crate) fn cache_settings_closed(&mut self) {
        self.cache.inspected = false;
        self.cache.custom_days = false;
    }

    fn request_cache_clear(&mut self, filter: CacheFilter) {
        self.cache.notice = None;
        self.cache.error = None;
        self.cache.pending = Some(PendingClear {
            remaining: filter,
            freed: None,
            cancelled: false,
        });
        self.clean_cache(CacheJob::Clear, filter);
        self.wake.wake();
    }

    fn cancel_pending_clear(&mut self) {
        if matches!(self.cache.job, Some(CacheJob::Clear)) {
            let pending = self.cache.pending.as_mut().unwrap();
            pending.remaining.categories.fill(false);
            pending.cancelled = true;
        } else {
            self.cache.pending = None;
        }
    }

    pub(crate) fn cache_settings_ui(
        &mut self,
        ui: &mut egui::Ui,
        policy: &mut CachePolicy,
    ) -> bool {
        if self.demo && self.cache.stats.is_none() {
            self.cache.stats = Some(CacheStats::demo());
        }
        if !self.cache.inspected && self.cache_ready() {
            self.inspect_cache();
            self.cache.inspected = true;
        }
        let mut changed = false;
        design::settings_section(ui, tr("Automatic cache"), |ui| {
            ui.weak(tr(
                "Media can be downloaded again. Saved offline tracks are kept separately.",
            ));
            cache_settings_row(
                ui,
                |ui| ui.label(tr("Keep unused files")),
                |ui, _| {
                    let custom = self.cache.custom_days || !RETENTION_DAYS.contains(&policy.days);
                    egui::ComboBox::from_id_salt("cache-retention")
                        .selected_text(retention_label(policy.days))
                        .show_ui(ui, |ui| {
                            for days in RETENTION_DAYS {
                                if ui
                                    .selectable_label(
                                        !custom && policy.days == days,
                                        retention_label(days),
                                    )
                                    .clicked()
                                {
                                    changed |= policy.days != days;
                                    policy.days = days;
                                    self.cache.custom_days = false;
                                }
                            }
                            if ui
                                .selectable_label(custom, tr("Custom retention…"))
                                .clicked()
                            {
                                self.cache.custom_days = true;
                            }
                        });
                    if self.cache.custom_days || !RETENTION_DAYS.contains(&policy.days) {
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut policy.days)
                                    .range(1..=365)
                                    .update_while_editing(false)
                                    .suffix(format!(" {}", tr("days"))),
                            )
                            .on_hover_text(tr("Enter retention in days"))
                            .changed();
                    }
                },
            );
            cache_settings_row(
                ui,
                |ui| ui.label(tr("Target size")),
                |ui, label| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut policy.size_mib)
                                .range(64..=16384)
                                .update_while_editing(false)
                                .suffix(format!(" {}", tr("MiB"))),
                        )
                        .on_hover_text(tr("Enter target size in MiB"))
                        .changed();
                    let mut tick = size_tick(policy.size_mib);
                    let slider = ui
                        .scope(|ui| {
                            let colors = design::colors(ui);
                            let width = ui.available_width().clamp(40., 128.);
                            let spacing = ui.spacing_mut();
                            spacing.slider_width = width;
                            spacing.slider_rail_height = 4.;
                            let visuals = ui.visuals_mut();
                            visuals.widgets.inactive.bg_fill = colors.muted;
                            visuals.selection.bg_fill = colors.accent;
                            ui.add(
                                egui::Slider::new(&mut tick, 0..=SIZE_MIB.len() - 1)
                                    .step_by(1.)
                                    .show_value(false)
                                    .trailing_fill(true)
                                    .handle_shape(egui::style::HandleShape::Circle),
                            )
                        })
                        .inner
                        .labelled_by(label.id)
                        .on_hover_text(cache_size(u64::from(SIZE_MIB[tick]) * 1024 * 1024));
                    if slider.changed() || slider.is_pointer_button_down_on() {
                        changed |= policy.size_mib != SIZE_MIB[tick];
                        policy.size_mib = SIZE_MIB[tick];
                    }
                },
            );
            let ready = self.cache_ready();
            if self.demo {
                ui.weak(tr("Example cache usage"));
            } else if !ready {
                ui.weak(tr("Connect to Telegram to inspect the cache."));
                return;
            }
            let busy = self.cache.job.is_some();
            let usage_label = self.cache.stats.as_ref().map(|stats| {
                tr("On disk · {size} · {count} files")
                    .replace("{size}", &cache_size(stats.total.bytes))
                    .replace("{count}", &stats.total.count.to_string())
            });
            cache_settings_row(
                ui,
                |ui| {
                    if let Some(label) = usage_label {
                        ui.strong(label);
                    }
                },
                |ui, ()| {
                    if busy {
                        ui.weak(if matches!(self.cache.job, Some(CacheJob::Inspect)) {
                            tr("Measuring cache…")
                        } else {
                            tr("Clearing cache…")
                        });
                    } else if ready
                        && self.cache.pending.is_none()
                        && ui.button(tr("Refresh storage")).clicked()
                    {
                        self.cache.error = None;
                        self.inspect_cache();
                    }
                },
            );
            if let Some(error) = &self.cache.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            if let Some(notice) = &self.cache.notice {
                ui.label(notice);
            }
            if let Some(pending) = &self.cache.pending {
                let waiting = CacheCategory::ALL
                    .into_iter()
                    .zip(pending.remaining.categories)
                    .filter(|(_, selected)| *selected)
                    .map(|(category, _)| category_label(category))
                    .collect::<Vec<_>>();
                if !waiting.is_empty() {
                    ui.weak(
                        tr("Waiting for files in use · {categories}")
                            .replace("{categories}", &waiting.join(", ")),
                    );
                    if ui.button(tr("Cancel pending cleanup")).clicked() {
                        self.cancel_pending_clear();
                    }
                }
            }
            let Some(stats) = &self.cache.stats else {
                return;
            };
            let title = |id: i64| {
                if let Some(playlist) = self.state.view().playlists.iter().find(|p| p.id == id) {
                    playlist.title.clone()
                } else {
                    tr("Chat {id}").replace("{id}", &id.to_string())
                }
            };
            if self
                .cache
                .chat
                .is_some_and(|id| stats.chats.iter().all(|chat| chat.id != id))
            {
                self.cache.chat = None;
            }
            let clear = ui
                .add_enabled_ui(!busy && self.cache.pending.is_none(), |ui| {
                    let previous_chat = self.cache.chat;
                    egui::ComboBox::from_id_salt("cache-scope")
                        .width(ui.available_width())
                        .selected_text(self.cache.chat.map_or_else(|| tr("All chats"), title))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.cache.chat, None, tr("All chats"));
                            for chat in stats.chats.iter().filter(|chat| chat.id != 0) {
                                ui.selectable_value(
                                    &mut self.cache.chat,
                                    Some(chat.id),
                                    format!(
                                        "{} · {}",
                                        title(chat.id),
                                        cache_size(chat.usage.bytes)
                                    ),
                                );
                            }
                        });
                    if previous_chat != self.cache.chat {
                        self.cache.selected.fill(false);
                    }
                    let usage = self
                        .cache
                        .chat
                        .and_then(|id| stats.chats.iter().find(|chat| chat.id == id))
                        .map_or(&stats.total, |chat| &chat.usage);
                    let separator = if ui.available_width() < 220. {
                        "\n"
                    } else {
                        " · "
                    };
                    for (index, category) in CacheCategory::ALL.into_iter().enumerate() {
                        let bytes = usage.by_category[index];
                        if bytes == 0 {
                            self.cache.selected[index] = false;
                            continue;
                        }
                        ui.checkbox(
                            &mut self.cache.selected[index],
                            format!(
                                "{}{separator}{}",
                                category_label(category),
                                cache_size(bytes)
                            ),
                        );
                    }
                    if self.cache.chat.is_none() {
                        if let Some(other) = stats.chats.iter().find(|chat| chat.id == 0) {
                            ui.weak(
                                tr("Other chats and unassigned files · {size}")
                                    .replace("{size}", &cache_size(other.usage.bytes)),
                            );
                        }
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                usage.by_category.iter().any(|&size| size > 0),
                                egui::Button::new(tr("Select all categories")),
                            )
                            .clicked()
                        {
                            self.cache.selected = usage.by_category.map(|size| size > 0);
                        }
                        ui.add_enabled(
                            ready && self.cache.selected.into_iter().any(|selected| selected),
                            egui::Button::new(tr("Clear selected cache")),
                        )
                        .clicked()
                    })
                    .inner
                })
                .inner;
            if clear {
                let request = CacheFilter {
                    chat: self.cache.chat,
                    categories: self.cache.selected,
                };
                self.request_cache_clear(request);
            }
        });
        changed
    }
}

fn cache_settings_row<L>(
    ui: &mut egui::Ui,
    left: impl FnOnce(&mut egui::Ui) -> L,
    right: impl FnOnce(&mut egui::Ui, L),
) {
    let row = |ui: &mut egui::Ui| {
        let label = left(ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            right(ui, label)
        });
    };
    if ui.available_width() >= 340. {
        ui.horizontal(row);
    } else {
        row(ui);
    }
}

fn retention_label(days: u32) -> String {
    if days == 1 {
        tr("1 day")
    } else {
        tr("{days} days").replace("{days}", &days.to_string())
    }
}

fn size_tick(size: u32) -> usize {
    SIZE_MIB
        .iter()
        .enumerate()
        .min_by_key(|(_, preset)| preset.abs_diff(size))
        .unwrap()
        .0
}

fn category_label(category: CacheCategory) -> String {
    match category {
        CacheCategory::Audio => tr("Audio and documents"),
        CacheCategory::Images => tr("Images and artwork"),
        CacheCategory::Emoji => tr("Emoji and stickers"),
        CacheCategory::Temporary => tr("Temporary files"),
        CacheCategory::Other => tr("Other media"),
    }
}

pub(crate) fn cache_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!(
            "{:.1} {}",
            bytes as f64 / (1024. * 1024. * 1024.),
            tr("GiB")
        )
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} {}", bytes as f64 / (1024. * 1024.), tr("MiB"))
    } else if bytes >= 1024 {
        format!("{:.1} {}", bytes as f64 / 1024., tr("KiB"))
    } else {
        format!("{bytes} {}", tr("B"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telegram::Telegram;
    use serde_json::{json, Value};
    use std::{cell::Cell, rc::Rc, sync::mpsc};
    use tg_player_application::{
        ports::{PlaybackEvent, PlayerBackend, PlayerCommand, PlayerSnapshot},
        workflow::AppEvent,
    };

    struct Player(Rc<Cell<bool>>);
    impl PlayerBackend for Player {
        fn command(&self, _: PlayerCommand) -> Result<(), AppError> {
            Ok(())
        }
        fn drain(&self) -> Vec<PlaybackEvent> {
            Vec::new()
        }
        fn snapshot(&self) -> PlayerSnapshot {
            PlayerSnapshot {
                duration: 120.,
                ..Default::default()
            }
        }
        fn is_idle(&self) -> bool {
            self.0.get()
        }
    }

    fn setup(fail_audio: bool) -> (App, Rc<Cell<bool>>, mpsc::Receiver<Value>) {
        let mut app = App::new_at(Default::default(), true);
        let (tx, requests) = mpsc::channel();
        let (mut sound, mut picture) = (4096_u64, 2048_u64);
        app.tg = Some(Telegram::mock_rpc(move |request| {
            match request["@type"].as_str() {
                Some("optimizeStorage") => {
                    tx.send(request.clone()).unwrap();
                    let types = request["file_types"].as_array().unwrap();
                    if types.iter().any(|ty| ty["@type"] == "fileTypeAudio") {
                        if fail_audio {
                            return json!({"@type":"error","code":500,"message":"Interrupted"});
                        }
                        sound = 0;
                    }
                    if types.iter().any(|ty| ty["@type"] == "fileTypePhoto") {
                        picture = 0;
                    }
                    json!({"@type":"storageStatistics"})
                }
                Some("getStorageStatistics") => {
                    json!({"@type":"storageStatistics","size":sound+picture,"count":2,
                    "by_chat":[{"chat_id":42,"size":sound+picture,"count":2,"by_file_type":[
                        {"file_type":{"@type":"fileTypeAudio"},"size":sound},
                        {"file_type":{"@type":"fileTypePhoto"},"size":picture}
                    ]}]})
                }
                _ => panic!("unexpected request: {request}"),
            }
        }));
        app.state
            .event(AppEvent::Authorization(AuthorizationState::Ready));
        let idle = Rc::new(Cell::new(false));
        app.player = Some(Box::new(Player(idle.clone())));
        (app, idle, requests)
    }

    fn finish_pass(app: &mut App) {
        assert!(matches!(app.cache.job, Some(CacheJob::Clear)));
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.cache.job.is_some() {
            app.process(&ctx);
            assert!(Instant::now() < deadline, "cache pass did not return");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    const SELECTION: CacheFilter = CacheFilter {
        chat: Some(42),
        categories: [true, true, false, false, false],
    };

    #[test]
    fn cleanup_runs_by_scope_and_continues_after_settings_close() {
        for fail_audio in [false, true] {
            let (mut app, idle, requests) = setup(fail_audio);
            app.request_cache_clear(SELECTION);
            finish_pass(&mut app);
            assert_eq!(
                requests.try_recv().unwrap()["file_types"][0]["@type"],
                "fileTypePhoto"
            );
            assert_eq!(app.cache.pending.as_ref().unwrap().freed, Some(2048));
            app.cache_settings_closed();
            assert!(app.maintain_cache(Instant::now()).is_some());
            assert!(requests.try_recv().is_err());
            idle.set(true);
            assert!(app.maintain_cache(Instant::now()).is_none());
            finish_pass(&mut app);
            let request = requests.try_recv().unwrap();
            assert_eq!(request["chat_ids"], json!([42]));
            assert_eq!(request["file_types"][0]["@type"], "fileTypeAudio");
            assert!(app.cache.pending.is_none());
            assert_eq!(app.cache.error.is_some(), fail_audio);
            assert_eq!(
                app.cache.stats.as_ref().unwrap().total.bytes,
                if fail_audio { 4096 } else { 0 }
            );
            assert_eq!(
                app.cache.notice.as_deref(),
                Some(if fail_audio {
                    "Removed from cache · 2.0 KiB"
                } else {
                    "Removed from cache · 6.0 KiB"
                })
            );
        }
    }

    #[test]
    fn cancelling_waiting_audio_does_not_cancel_started_cleanup() {
        let (mut app, idle, requests) = setup(false);
        app.request_cache_clear(SELECTION);
        app.cancel_pending_clear();
        finish_pass(&mut app);
        assert_eq!(app.cache.stats.as_ref().unwrap().total.bytes, 4096);
        assert!(app.cache.pending.is_none());
        idle.set(true);
        app.maintain_cache(Instant::now());
        assert_eq!(requests.try_iter().count(), 1);
    }

    #[test]
    fn local_and_offline_playback_release_audio_cache_after_loading() {
        use tg_player_application::commands::AppCommand;
        let (mut app, _, _) = setup(false);
        let track = app.state.view().playlists[0].tracks[0].clone();
        let loading = app.state.command(AppCommand::StartSession(vec![track], 0));
        assert!(!app.cache_audio_available());
        app.run_effects(loading);
        assert!(!app.state.playback().state().loading());
        assert!(!app.cache_audio_available());
        app.cache_playback = CachePlayback::Local;
        assert!(app.cache_audio_available());
        app.state.command(AppCommand::Pause(true));
        assert!(app.cache_audio_available());
        app.cache_playback = CachePlayback::Stream;
        assert!(!app.cache_audio_available());
        app.cache_playback = CachePlayback::Offline;
        assert!(app.cache_audio_available());
    }

    #[test]
    fn custom_size_maps_to_nearest_tick_without_changing_the_policy() {
        let policy = CachePolicy {
            days: 11,
            size_mib: 1600,
        };
        assert_eq!(SIZE_MIB[size_tick(policy.size_mib)], 2048);
        let (mut app, _, _) = setup(false);
        app.tg = None;
        let mut edited = policy;
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(190., 480.),
                )),
                ..Default::default()
            },
            |ui| assert!(!app.cache_settings_ui(ui, &mut edited)),
        );
        output.textures_delta.clear();
        assert_eq!(policy, edited);
        for shape in output.shapes {
            if let egui::Shape::Text(text) = shape.shape {
                assert!(
                    text.visual_bounding_rect().right() <= 190.,
                    "{}: {:?}",
                    text.galley.text(),
                    text.visual_bounding_rect()
                );
            }
        }
    }

    #[test]
    fn retention_preset_and_typed_size_edit_settings() {
        let mut app = App::new_at(Default::default(), true);
        let ctx = egui::Context::default();
        design::style(&ctx, false);
        let mut policy = CachePolicy::default();
        let mut time = 0.;
        let mut frame = |events| {
            time += 0.1;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    time: Some(time),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(430., 600.),
                    )),
                    ..Default::default()
                },
                |ui| {
                    app.cache_settings_ui(ui, &mut policy);
                },
            );
            output.textures_delta.clear();
            (output, policy)
        };
        fn find(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
            output
                .shapes
                .iter()
                .find_map(|entry| match &entry.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.visual_bounding_rect().center())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| {
                    panic!(
                        "label {label} not found in {:?}",
                        output
                            .shapes
                            .iter()
                            .filter_map(|entry| match &entry.shape {
                                egui::Shape::Text(text) => Some(text.galley.text()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                    )
                })
        }
        let pointer = |pos, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let initial = frame(vec![]).0;
        let retention = find(&initial, "7 days");
        assert!(retention.x > 300.);
        assert!(find(&initial, "1024").x > 300.);
        assert!(initial.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(rail) if rail.fill == design::palette(false).muted
                && rail.rect.width() >= 100. && rail.rect.height() == 4.
        )));
        frame(pointer(retention, true));
        frame(pointer(retention, false));
        let menu = frame(vec![]).0;
        let preset = find(&menu, "30 days");
        frame(pointer(preset, true));
        let (_, policy) = frame(pointer(preset, false));
        assert_eq!(policy.days, 30);
        let (output, _) = frame(vec![]);
        let number = find(&output, "1024");
        frame(pointer(number, true));
        frame(pointer(number, false));
        frame(vec![]);
        let (edit, before_commit) = frame(vec![
            key(egui::Key::A, egui::Modifiers::COMMAND),
            egui::Event::Text("1537".into()),
        ]);
        assert_eq!(before_commit.size_mib, 1024);
        find(&edit, "1537");
        frame(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        let (_, policy) = frame(vec![]);
        assert_eq!(policy.size_mib, 1537);
    }
}
