use super::scroll_anchor::ScrollAnchor;
use super::{
    art, chat_title_galley, clock, clock_width, full_menu_button, horizontal_slot,
    keyboard::{self, empty_label, focus_outline, ListNavigation, MenuItems},
    layout_fraction, list_scroll_area, search_field, selection, show_chat_title, track_list_frame,
    track_menu_button, track_more_button, WorkspacePane, TRACK_ARTWORK_SIZE, TRACK_ROW_PADDING,
    TRACK_SCROLL_GUTTER,
};
use crate::application::commands::AppCommand;
use crate::{design, tr, App};
use design::{button, Icon};
use eframe::egui;
use gramwave_domain::{PlaylistOrder, TrackId};

#[derive(Clone)]
struct PlaylistScroll {
    chat: i64,
    order: PlaylistOrder,
    library: u64,
    query: String,
    filters: [bool; 3],
    scrolled: bool,
    held_current: Option<(u64, TrackId)>,
    visible: ScrollAnchor<(u64, TrackId)>,
}

impl App {
    fn playlist_menu(&mut self, ui: &mut egui::Ui, selected: usize, chat_id: i64, url: &str) {
        if ui
            .add_enabled(!self.demo, egui::Button::new(tr("Open source chat")))
            .clicked()
        {
            self.open_source_chat(chat_id, url);
            ui.close();
        }
        if !url.is_empty() && ui.button(tr("Copy chat link")).clicked() {
            ui.ctx().copy_text(url.to_owned());
            ui.close();
        }
        if ui.button(tr("Refresh library")).clicked() {
            self.open_chat(selected);
            ui.close();
        }
    }

    fn sort_playlist(&mut self, ui: &mut egui::Ui) {
        let newest = tr("Newest first");
        let oldest = tr("Oldest first");
        egui::ComboBox::from_id_salt("order")
            .selected_text(match self.playlist_order {
                PlaylistOrder::NewestFirst => &newest,
                PlaylistOrder::OldestFirst => &oldest,
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.playlist_order, PlaylistOrder::NewestFirst, newest);
                ui.selectable_value(&mut self.playlist_order, PlaylistOrder::OldestFirst, oldest);
            });
    }

    fn playlist_pin_action(&mut self, ui: &mut egui::Ui, chat_id: i64) {
        let label = if self.state.view().chat_pins.contains(&chat_id) {
            tr("Unpin chat")
        } else {
            tr("Pin chat")
        };
        if ui.button(label).clicked() {
            self.toggle_chat_pin(chat_id);
            ui.close();
        }
    }

    fn playlist_heading(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        artwork: Option<egui::load::SizedTexture>,
        emoji: Option<gramwave_application::model::EmojiStatus>,
        full: f32,
        visible: f32,
    ) {
        let width = ui.available_width() - f32::from(TRACK_SCROLL_GUTTER) * full;
        let padding = 16. * full;
        let full_cover = ui.ctx().animate_value_with_time(
            egui::Id::new("playlist-cover-size"),
            if ui.available_width() - f32::from(TRACK_SCROLL_GUTTER) - 32. < 650. {
                72.
            } else {
                112.
            },
            0.15,
        );
        let cover_size = egui::lerp(32. ..=full_cover, full);
        let font_size = egui::lerp(18. ..=30., full);
        let text_x = padding + cover_size + ui.spacing().item_spacing.x + 16. * full;
        let text_width = (width - text_x - padding).max(0.);
        let galley = chat_title_galley(
            ui,
            title,
            emoji.is_some(),
            font_size,
            if full > 0. { 2 } else { 1 },
            text_width,
        );
        let first_line = galley.rows.first().map_or(0., |row| row.size.y);
        let title_height = egui::lerp(first_line..=galley.size().y, full);
        let eyebrow = 22. * full;
        let inner_height = cover_size.max(title_height + eyebrow);
        let height = inner_height + padding * 2.;
        // A short workspace still needs room for its search field and a track row.
        let room = if visible < 1. {
            (ui.available_height() - design::ROW_HEIGHT - 60.).max(0.)
        } else {
            f32::INFINITY
        };
        let clip_height = (height * visible).min(room);
        if clip_height <= 0. {
            return;
        }
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(width, clip_height), egui::Sense::hover());
        let clip = ui.clip_rect().intersect(rect);
        ui.painter().with_clip_rect(clip).rect_filled(
            rect,
            12. * full,
            design::colors(ui).panel.gamma_multiply(full * visible),
        );
        let mid_y = rect.top() + padding + inner_height / 2.;
        let mut cover = horizontal_slot(
            ui,
            "playlist-cover",
            egui::pos2(rect.left() + padding, mid_y),
            egui::Vec2::splat(cover_size),
        );
        cover.set_clip_rect(clip);
        cover.multiply_opacity(visible);
        art(&mut cover, title, artwork, cover_size);
        let text_origin = egui::pos2(rect.left() + text_x, mid_y - (title_height + eyebrow) / 2.);
        if full > 0. {
            ui.painter().with_clip_rect(clip).text(
                text_origin,
                egui::Align2::LEFT_TOP,
                tr("TELEGRAM PLAYLIST"),
                egui::FontId::proportional(12.),
                design::colors(ui).muted.gamma_multiply(full * visible),
            );
        }
        let title_rect = egui::Rect::from_min_size(
            text_origin + egui::vec2(0., eyebrow),
            egui::vec2(text_width, title_height),
        );
        let mut label = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("playlist-title")
                .max_rect(title_rect),
        );
        label.set_clip_rect(clip);
        label.multiply_opacity(visible);
        let mut text = label.new_child(egui::UiBuilder::new().max_rect(title_rect));
        text.set_clip_rect(clip.with_max_y(clip.bottom().min(title_rect.bottom())));
        let end = show_chat_title(&mut text, galley);
        self.chat_emoji(
            &mut label,
            emoji,
            24.,
            Some(end + egui::vec2(ui.spacing().item_spacing.x, 0.)),
        );
    }

    pub(super) fn playlist_screen(&mut self, ui: &mut egui::Ui, compact: bool, dense: bool) {
        if !self.demo && self.opened_chat.is_none() {
            if !compact {
                ui.add_space(60.);
            }
            let empty = empty_label(ui, egui::RichText::new(tr("Choose a chat")).heading());
            self.keyboard.empty(ui, WorkspacePane::Playlist, &empty);
            focus_outline(ui, &empty);
            ui.weak(tr("Music loads only when you open a chat."));
            return;
        }

        let Some(selected) = self
            .state
            .view()
            .playlists
            .iter()
            .position(|p| Some(p.id) == self.opened_chat.or(self.demo.then_some(0)))
        else {
            let empty = empty_label(ui, egui::RichText::new(tr("Choose a chat")).heading());
            self.keyboard.empty(ui, WorkspacePane::Playlist, &empty);
            focus_outline(ui, &empty);
            return;
        };
        let Some(p) = self.state.view().playlists.get(selected) else {
            ui.add_space(80.);
            ui.heading(tr("Your next favourite is waiting."));
            ui.weak(&self.status);
            return;
        };
        let chat_id = p.id;
        let previous_order = self.playlist_order;
        if !dense {
            self.image_wanted
                .push(self.state.library().playlist_artwork(p.id).file);
        }
        let title = p.title.clone();
        let artwork = self.chat_cover(ui.ctx(), chat_id);
        let url = p.url.clone();
        let can_play = p
            .tracks
            .iter()
            .any(|track| p.filters[usize::from(track.kind)]);

        let emoji_status = p.emoji_status;
        let before_filters = p.filters;
        let has_saved = self
            .state
            .offline()
            .copies
            .values()
            .any(|copy| !copy.cached && copy.track.chat == chat_id);
        let fully_saved = has_saved && self.state.offline().playlist_complete(p);
        let can_download = before_filters.into_iter().any(|enabled| enabled);
        let mut remove_downloads = false;
        let mut filters = before_filters;
        let filter_labels = [tr("Music"), tr("Audio files"), tr("Voice messages")];
        let mut play_all = false;
        let mut pin_all = false;
        let full_heading = layout_fraction(ui, "playlist-heading-full", !compact);
        let show_heading = layout_fraction(ui, "playlist-heading-visible", !dense);
        if show_heading > 0. {
            self.playlist_heading(
                ui,
                &title,
                artwork,
                emoji_status,
                full_heading,
                show_heading,
            );
        }
        if full_heading > 0. && !dense {
            let gap = 18. * full_heading;
            let area = ui.available_rect_before_wrap();
            let rect = area.with_min_y(area.top() + gap);
            let mut actions = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("playlist-actions")
                    .max_rect(rect),
            );
            actions.multiply_opacity(full_heading);
            if full_heading < 1. {
                actions.set_clip_rect(
                    ui.clip_rect()
                        .intersect(rect.with_max_y(rect.top() + 44. * full_heading)),
                );
            }
            let row = actions
                .horizontal_wrapped(|ui| {
                    let play = ui.add_enabled_ui(can_play, |ui| {
                        design::playback_button(ui, Icon::Play, "Play all")
                    });
                    play_all = play.inner;
                    #[cfg(test)]
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(egui::Id::new("test-play-all"), play.response.rect)
                    });
                    if !fully_saved {
                        pin_all = ui
                            .add_enabled_ui(can_download, |ui| {
                                button(ui, Icon::Download, "Save playlist offline")
                            })
                            .inner;
                    }
                    if has_saved {
                        remove_downloads = button(ui, Icon::Trash, "Remove downloads");
                    }
                    self.sort_playlist(ui);
                    ui.menu_button("…", |ui| {
                        self.playlist_menu(ui, selected, chat_id, &url);
                        self.playlist_pin_action(ui, chat_id);
                    });
                })
                .response
                .rect;
            ui.add_space(gap + (row.height() + ui.spacing().item_spacing.y) * full_heading);
        }
        let inline_filters = !compact && ui.available_width() >= 620.;
        let inline = layout_fraction(ui, "playlist-filters-inline", inline_filters);
        let gap = ui.spacing().item_spacing.x;
        let widths = if inline == 0. {
            [40.; 3]
        } else {
            filter_labels.each_ref().map(|label| {
                let text = egui::WidgetText::from(label.as_str()).into_galley(
                    ui,
                    Some(egui::TextWrapMode::Extend),
                    f32::INFINITY,
                    egui::TextStyle::Button,
                );
                (text.size().x + 2. * ui.spacing().button_padding.x + 2.).ceil()
            })
        };
        let inline_width = widths.iter().sum::<f32>() + 2. * gap;
        let (toolbar, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.), egui::Sense::hover());
        let menu_x = toolbar.right() - 40.;
        let filter_left = toolbar.right() - egui::lerp(40. ..=inline_width, inline);
        let filter_left = filter_left.max(toolbar.left() + 40. + gap);
        let mut search = horizontal_slot(
            ui,
            "track-toolbar-search",
            toolbar.left_center(),
            egui::vec2(filter_left - toolbar.left() - gap, 40.),
        );
        let _ = search_field(
            &mut search,
            &mut self.search,
            "track-search",
            tr("Search this playlist"),
            None,
        );

        if inline > 0. {
            let mut slot_left = toolbar.right() - inline_width;
            for (index, ((label, filter), width)) in filter_labels
                .iter()
                .zip(&mut filters)
                .zip(widths)
                .enumerate()
            {
                let center = egui::lerp((menu_x + 20.)..=(slot_left + width / 2.), inline);
                let visible_width = egui::lerp(16. ..=width, inline);
                let mut slot = horizontal_slot(
                    ui,
                    ("inline-filter", index),
                    egui::pos2(center - visible_width / 2., toolbar.center().y),
                    egui::vec2(visible_width, 36.),
                );
                slot.set_clip_rect(ui.clip_rect().intersect(toolbar.with_min_x(filter_left)));
                slot.multiply_opacity(inline);
                if !inline_filters {
                    slot.visuals_mut().disabled_alpha = 1.;
                    slot.disable();
                }
                if slot
                    .add_sized(
                        [visible_width, 36.],
                        egui::Button::selectable(*filter, label.as_str()).truncate(),
                    )
                    .clicked()
                {
                    *filter = !*filter;
                }
                slot_left += width + gap;
            }
        }
        if inline < 1. {
            let mut menu = horizontal_slot(
                ui,
                "playlist-filter-menu",
                egui::pos2(menu_x, toolbar.center().y),
                egui::Vec2::splat(40.),
            );
            menu.multiply_opacity(1. - inline);
            if inline_filters {
                menu.visuals_mut().disabled_alpha = 1.;
                menu.disable();
            }
            let ui = &mut menu;
            let label = if compact {
                tr("Playlist options")
            } else {
                tr("Filters")
            };
            let p = design::colors(ui);
            egui::containers::menu::MenuButton::from_button(
                egui::Button::image(Icon::More.image(p.muted, 20.).alt_text(&label))
                    .min_size(egui::vec2(40., 40.)),
            )
            .ui(ui, |ui| {
                if compact {
                    play_all = ui
                        .add_enabled(can_play, egui::Button::new(tr("Play all")))
                        .clicked();
                    if !fully_saved {
                        pin_all = ui
                            .add_enabled(
                                can_download,
                                egui::Button::new(tr("Save playlist offline")),
                            )
                            .clicked();
                    }
                    if has_saved {
                        remove_downloads = ui.button(tr("Remove downloads")).clicked();
                    }
                    if play_all || pin_all || remove_downloads {
                        ui.close();
                    }
                    self.playlist_menu(ui, selected, chat_id, &url);
                    ui.separator();
                    self.sort_playlist(ui);
                }
                for (filter, label) in filters.iter_mut().zip(&filter_labels) {
                    ui.checkbox(filter, label.as_str());
                }
                if compact {
                    ui.separator();
                    self.playlist_pin_action(ui, chat_id);
                }
            })
            .0
            .on_hover_text(if dense {
                format!("{title}\n{label}")
            } else {
                label
            });
        }
        ui.add_space(egui::lerp(2. ..=12., full_heading));
        if before_filters != filters {
            self.execute(AppCommand::Filters(chat_id, filters));
            self.open_chat(selected);
        }
        if previous_order != self.playlist_order {
            self.execute(AppCommand::CancelOldestStart(chat_id));
        }
        let tracks = self.playlist_tracks(selected);
        if play_all {
            self.play_playlist_from(selected, None);
        }
        if remove_downloads {
            self.offline_confirm = Some(vec![chat_id]);
        }
        if pin_all {
            self.execute(AppCommand::SavePlaylist(
                chat_id,
                self.state.view().playlists[selected].filters,
            ));
        }
        if let Some(notice) = (!self.settings_open)
            .then(|| self.inline_notice(ui.ctx(), false))
            .flatten()
        {
            ui.weak(notice);
        }
        let loading_id = ui.id().with("empty-playlist");
        let waiting = tracks.is_empty()
            && self.search.is_empty()
            && (self.state.library().indexing_chat() == Some(chat_id)
                || self.state.oldest_loading(chat_id))
            && self.state.view().playlists[selected].has_more();
        if !waiting {
            ui.data_mut(|data| data.remove::<(i64, f64)>(loading_id));
        }
        if tracks.is_empty() {
            if !compact {
                ui.add_space(24.);
            }
            let label = if !self.state.view().playlists[selected]
                .filters
                .iter()
                .any(|v| *v)
            {
                tr("All filters are off")
            } else if !self.search.is_empty() {
                tr("No matching tracks")
            } else if waiting {
                let time = ui.input(|input| input.time);
                let started = ui.data_mut(|data| {
                    let started = data.get_temp_mut_or(loading_id, (chat_id, time));
                    if started.0 != chat_id {
                        *started = (chat_id, time);
                    }
                    started.1
                });
                let remaining = 0.2 - (time - started);
                if remaining > 0. {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_secs_f64(remaining));
                    String::new()
                } else {
                    tr("Loading music…")
                }
            } else {
                tr("No music in this chat yet")
            };
            let empty = empty_label(ui, egui::RichText::new(label).heading());
            self.keyboard.empty(ui, WorkspacePane::Playlist, &empty);
            focus_outline(ui, &empty);
            if ui.button(tr("Refresh library")).clicked() {
                self.open_chat(selected);
            }
        }
        let order = self.playlist_order;
        let scope = selection::scope(chat_id, &self.search, filters, order);
        let key_at = |i: usize| {
            let track = &tracks[i];
            egui::Id::new((chat_id, track.library_id, track.id()))
        };
        self.selection
            .present_playlist(ui.ctx(), scope, &tracks, key_at);
        let mut action = None;
        let mut batch = None;
        let mut list_frame = track_list_frame().begin(ui);
        let list_ui = &mut list_frame.content_ui;
        let key = egui::Id::new("playlist-scroll-anchor");
        let library = self.state.library_version();
        let old: Option<PlaylistScroll> = list_ui.ctx().data(|data| data.get_temp(key));
        let old = old.filter(|old| old.chat == chat_id);
        let order_changed = old.as_ref().is_some_and(|old| old.order != order);
        let spacing = list_ui.spacing().item_spacing.y;
        let stride = design::ROW_HEIGHT + spacing;
        let scroll_salt = ("playlist-tracks", chat_id);
        // ScrollArea wraps the supplied salt before deriving its persistent ID.
        let area_id = list_ui.make_persistent_id(egui::IdSalt::new(scroll_salt));
        let state = egui::scroll_area::State::load(list_ui.ctx(), area_id).unwrap_or_default();
        let scrolled = old.as_ref().is_some_and(|old| old.scrolled);
        let current = self.state.view().session.current().map(|entry| entry.id());
        let mut held_current = old
            .as_ref()
            .filter(|old| old.query == self.search && old.filters == filters)
            .and_then(|old| old.held_current)
            .filter(|&(_, id)| Some(id) == current);
        let relayout = old.as_ref().is_some_and(|old| {
            order_changed
                || (old.library != library && old.query == self.search && old.filters == filters)
        });
        let mut area = list_scroll_area(list_ui);
        let start = if old.is_none() || relayout {
            let height = list_ui.available_height();
            let max = tracks.len() as f32 * stride - spacing - height;
            let find = |&(library_id, id): &(u64, TrackId)| {
                tracks
                    .iter()
                    .position(|track| track.library_id == library_id && track.id() == id)
            };
            let offset = old
                .as_ref()
                .and_then(|old| {
                    let current_offset = if order_changed {
                        let anchor = old.visible.interior_offset(stride, max, |key| {
                            if Some(key.1) == current {
                                find(key)
                            } else {
                                None
                            }
                        });
                        held_current = anchor.map(|(key, _)| *key);
                        anchor.map(|(_, offset)| offset)
                    } else {
                        let offset = held_current.and_then(|held| {
                            old.visible.offset(stride, max, |key| {
                                if *key == held {
                                    find(key)
                                } else {
                                    None
                                }
                            })
                        });
                        held_current = held_current.filter(|_| offset.is_some());
                        offset
                    };
                    current_offset.or_else(|| {
                        if !scrolled {
                            None
                        } else if order_changed {
                            old.visible.mirrored_offset(
                                stride,
                                max,
                                height,
                                design::ROW_HEIGHT,
                                find,
                            )
                        } else {
                            old.visible.offset(stride, max, find)
                        }
                    })
                })
                .unwrap_or_default();
            if old.is_none() || order_changed || (state.offset.y - offset).abs() > 0.01 {
                // Drop kinetic scrolling when relocating the viewport.
                egui::scroll_area::State::default().store(list_ui.ctx(), area_id);
                area = area.vertical_scroll_offset(offset);
            }
            offset
        } else {
            state.offset.y
        };
        let scroll = area.id_salt(scroll_salt).show_rows(
            list_ui,
            design::ROW_HEIGHT,
            tracks.len(),
            |ui, range| {
                let stride = design::ROW_HEIGHT + ui.spacing().item_spacing.y;
                let nav = ListNavigation::new(WorkspacePane::Playlist, tracks.len(), stride);
                let height = (tracks.len() as f32 * stride - ui.spacing().item_spacing.y).max(0.);
                let list = egui::Rect::from_min_size(
                    ui.max_rect().left_top() - egui::vec2(0., range.start as f32 * stride),
                    egui::vec2(ui.available_width(), height),
                );
                let (focus, _) =
                    self.selection
                        .pointer(ui, scope, tracks.len(), key_at, list, stride);
                if let Some(i) = focus {
                    ui.memory_mut(|memory| memory.request_focus(ui.id().with(key_at(i))));
                }
                if let Some(i) =
                    self.keyboard
                        .step(ui, nav, true, key_at, Some((&mut self.selection, scope)))
                {
                    let rect = egui::Rect::from_min_size(
                        ui.max_rect().left_top()
                            + egui::vec2(0., (i as f32 - range.start as f32) * stride),
                        egui::vec2(ui.available_width(), design::ROW_HEIGHT),
                    );
                    if !ui.is_rect_visible(rect) {
                        self.keyboard.reveal(ui, rect, nav, i);
                    }
                }
                self.note_playlist_view(selected, tracks.clone(), range.clone());
                for i in range {
                    let mut view = self.media_track(tracks[i].metadata());
                    view.playlist_number = tracks[i].playlist_number;
                    let t = &view;
                    self.image_wanted.push(t.image_file);
                    if t.file == 0 {
                        self.track_wanted.push(t.clone());
                    }
                    let active = current == Some(t.id());
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), design::ROW_HEIGHT),
                        egui::Sense::hover(),
                    );
                    // Register the row first; the later menu owns its independent hit target.
                    let response = ui.interact(
                        rect,
                        ui.id().with(key_at(i)),
                        if ui.is_rect_visible(rect) {
                            egui::Sense::click_and_drag()
                        } else {
                            egui::Sense::CLICK | egui::Sense::DRAG
                        },
                    );
                    let selected_row = self.selection.contains(scope, key_at(i));
                    self.keyboard
                        .row(ui, nav, i, key_at(i), false, (&response, rect));
                    let p = design::colors(ui);
                    let hover = ui.ctx().animate_bool_with_time(
                        response.id.with("hover"),
                        response.hovered(),
                        0.14,
                    );
                    let highlight = p
                        .track_fill(active, selected_row)
                        .or_else(|| response.has_focus().then_some(p.selected));
                    if highlight.is_some() || hover > 0. {
                        ui.painter().rect_filled(
                            rect,
                            8.,
                            highlight.unwrap_or_else(|| p.raised.gamma_multiply(hover)),
                        );
                    }
                    if active {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                rect.left_top() + egui::vec2(0., 12.),
                                egui::vec2(3., 36.),
                            ),
                            2.,
                            p.accent,
                        );
                    }
                    if selected_row {
                        selection::paint(ui, rect);
                    }
                    focus_outline(ui, &response);
                    let mut menu_clicked = false;
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .id_salt(key_at(i))
                            .max_rect(rect.shrink2(egui::vec2(TRACK_ROW_PADDING, 6.)))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| {
                            let indicator = if active {
                                Some(if self.state.playback().shows_loading() {
                                    "…"
                                } else if self.state.playback().state().paused() {
                                    "Ⅱ"
                                } else {
                                    "▶"
                                })
                            } else {
                                None
                            }
                            .map_or_else(|| t.playlist_number.to_string(), str::to_owned);
                            ui.add_sized(
                                [22., 36.],
                                egui::Label::new(egui::RichText::new(indicator).color(if active {
                                    p.accent
                                } else {
                                    p.muted
                                })),
                            );
                            let cover_left = ui.next_widget_position().x;
                            art(
                                ui,
                                &t.title,
                                self.track_cover(ui.ctx(), t),
                                TRACK_ARTWORK_SIZE,
                            );
                            let offline = if t.pinned.is_some() {
                                Some("✓")
                            } else if self
                                .state
                                .offline()
                                .pending
                                .values()
                                .any(|p| p.id() == t.id())
                            {
                                Some("↓")
                            } else if self
                                .state
                                .offline()
                                .errors()
                                .contains_key(&(t.chat, t.message))
                            {
                                Some("!")
                            } else {
                                None
                            };
                            if let Some(mark) = offline {
                                let pos = egui::pos2(cover_left + 40., rect.bottom() - 10.);
                                ui.painter().circle_filled(pos, 9., p.panel);
                                ui.painter().text(
                                    pos,
                                    egui::Align2::CENTER_CENTER,
                                    mark,
                                    egui::FontId::proportional(12.),
                                    p.accent,
                                );
                            }
                            let duration = if t.duration == 0 {
                                "—:—".into()
                            } else {
                                clock(t.duration as f64, false)
                            };
                            let duration_width = if t.duration >= 3600 {
                                clock_width(ui, duration.clone()).max(44.)
                            } else {
                                44.
                            };
                            track_labels(ui, &t.title, &t.artist, active, duration_width);
                            ui.add_sized(
                                [duration_width, 36.],
                                egui::Label::new(egui::RichText::new(duration).small()).extend(),
                            );
                            ui.scope(|ui| {
                                ui.visuals_mut().widgets.inactive.weak_bg_fill =
                                    egui::Color32::TRANSPARENT;
                                if !response.hovered()
                                    && !response.has_focus()
                                    && !ui.memory(|m| m.has_focus(ui.next_auto_id()))
                                    && !self.keyboard.targets(nav.pane, key_at(i))
                                {
                                    ui.visuals_mut().widgets.inactive.fg_stroke.color =
                                        egui::Color32::TRANSPARENT;
                                    ui.visuals_mut().override_text_color = None;
                                }
                                let menu_button = egui::Button::new("…")
                                    .frame_when_inactive(false)
                                    .min_size(egui::Vec2::splat(36.))
                                    .sense(if self.selection.active(scope) {
                                        egui::Sense::CLICK
                                    } else {
                                        keyboard::row_sense(ui, rect)
                                    });
                                let menu = ui.add(track_more_button(
                                    menu_button,
                                    p.raised,
                                    highlight.is_some(),
                                ));
                                self.keyboard
                                    .row(ui, nav, i, key_at(i), true, (&menu, rect));
                                focus_outline(ui, &menu);
                                self.selection.start(
                                    &response,
                                    &menu,
                                    scope,
                                    (i, key_at(i)),
                                    self.keyboard.source_key(nav.pane),
                                    None,
                                );
                                let contents = |ui: &mut egui::Ui, items: &mut MenuItems| {
                                    ui.visuals_mut().override_text_color = Some(p.text);
                                    ui.visuals_mut().widgets.inactive.fg_stroke.color = p.text;
                                    if selected_row && self.selection.is_group() {
                                        if let Some(action) = playlist_selection_menu(ui, items) {
                                            batch = Some((action, response.id));
                                        }
                                        return;
                                    }
                                    for (kind, label) in [
                                        (0, tr("Play")),
                                        (1, tr("Play next")),
                                        (2, tr("Add to queue")),
                                        (
                                            3,
                                            if t.pinned.is_some() {
                                                tr("Remove offline download")
                                            } else {
                                                tr("Save offline")
                                            },
                                        ),
                                        (7, tr("Open source message")),
                                        (4, tr("Copy message link")),
                                        (5, tr("Copy title")),
                                    ] {
                                        let button = if kind == 7 && self.demo {
                                            ui.add_enabled(
                                                false,
                                                full_menu_button(label, ui.available_width()),
                                            )
                                        } else {
                                            track_menu_button(ui, label)
                                        };
                                        items.push(ui, &button, None);
                                        #[cfg(test)]
                                        if kind == 0 {
                                            self.test_play_buttons.insert(t.message, button.rect);
                                        }
                                        if button.clicked() {
                                            action = Some((kind, i));
                                            ui.close();
                                        }
                                    }
                                    ui.spacing_mut().item_spacing.y = 3.;
                                    if offline.is_some() {
                                        ui.separator();
                                    }
                                    if t.pinned.is_some() {
                                        ui.label(tr("Saved offline"));
                                    } else if self
                                        .state
                                        .offline()
                                        .pending
                                        .values()
                                        .any(|p| p.id() == t.id())
                                    {
                                        ui.label(tr("Downloading…"));
                                    } else if let Some(error) =
                                        self.state.offline().errors().get(&(t.chat, t.message))
                                    {
                                        ui.label(tr("Download failed · Save offline to retry"));
                                        ui.label(error.to_string());
                                    }
                                    items.links(self.track_caption(ui, t.caption.as_ref()));
                                };
                                let (handled, popup, keep_click) = keyboard::show_track_menu(
                                    &response,
                                    &menu,
                                    selected_row,
                                    contents,
                                );
                                self.selection.remember_popup(popup, keep_click);
                                #[cfg(test)]
                                self.test_rows.insert(t.message, (rect, menu.rect));
                                #[cfg(test)]
                                self.test_row_ids.insert(t.message, response.id);
                                menu_clicked = handled;
                            });
                        },
                    );
                    // Keep the virtual row stride when the inset scope advances the cursor.
                    ui.advance_cursor_after_rect(rect);
                    if response.clicked() && !menu_clicked {
                        response.request_focus();
                    }
                    if !menu_clicked
                        && !self.selection.block_click()
                        && response.clicked()
                        && !response.double_clicked()
                    {
                        action = Some((6, i));
                    }
                }
            },
        );
        list_frame.end(ui);
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new("test-playlist-scroll"), scroll.id);
            data.insert_temp(scroll.id.with("test-viewport"), scroll.inner_rect);
        });
        design::middle_scroll(ui, scroll.id, scroll.inner_rect, scroll.content_size.y);
        let state = egui::scroll_area::State::load(ui.ctx(), scroll.id).unwrap_or(scroll.state);
        let max = (scroll.content_size.y - scroll.inner_rect.height()).max(0.);
        let moved = (state.offset.y - start.clamp(0., max)).abs() > 0.01;
        let visible = ScrollAnchor::capture(
            tracks.len(),
            stride,
            state.offset.y,
            scroll.inner_rect.height(),
            |i| (tracks[i].library_id, tracks[i].id()),
        );
        let query = self.search.clone();
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                key,
                PlaylistScroll {
                    chat: chat_id,
                    order,
                    library,
                    query,
                    filters,
                    scrolled: scrolled || moved,
                    held_current: if moved { None } else { held_current },
                    visible,
                },
            )
        });

        let playlist = &self.state.view().playlists[selected];
        if playlist.has_more() && !self.demo {
            let bottom = state.offset.y + scroll.inner_rect.height();
            if order == PlaylistOrder::OldestFirst {
                let frontier = (0..3)
                    .filter(|&kind| filters[kind] && !playlist.done[kind])
                    .map(|kind| playlist.oldest_cursors[kind])
                    .min()
                    .unwrap_or(i64::MAX);
                let known = tracks.partition_point(|track| track.message <= frontier);
                let near = bottom >= known as f32 * stride - 120.;
                let error = self.state.oldest_error(chat_id);
                if self.state.oldest_loading(chat_id) {
                    ui.weak(tr("Loading older history…"));
                } else if let Some(error) = error {
                    let message = error.to_string();
                    if ui.button(tr("Retry")).on_hover_text(message).clicked() {
                        self.execute(AppCommand::LoadOldest {
                            chat: chat_id,
                            retry: true,
                        });
                    }
                } else if ui.button(tr("Load more music")).clicked() || order_changed || near {
                    self.execute(AppCommand::LoadOldest {
                        chat: chat_id,
                        retry: false,
                    });
                }
            } else if self.state.library().indexing_chat().is_none() {
                let at_bottom = state.offset.y > 0. && bottom >= scroll.content_size.y - 120.;
                if ui.button(tr("Load more music")).clicked() || at_bottom {
                    self.load_chat(selected, false);
                }
            }
        }
        if let Some((batch, row)) = batch {
            ui.memory_mut(|memory| memory.request_focus(row));
            let mut selected_tracks: Vec<_> = tracks
                .iter()
                .enumerate()
                .filter(|(i, _)| self.selection.contains(scope, key_at(*i)))
                .map(|(_, track)| track.metadata())
                .collect();
            match batch {
                PlaylistSelectionAction::Insert { next, reverse } => {
                    if reverse {
                        selected_tracks.reverse();
                    }
                    self.execute(AppCommand::InsertTracks(selected_tracks, next));
                }
                PlaylistSelectionAction::SaveOffline => {
                    self.execute(AppCommand::SaveTracks(selected_tracks))
                }
            }
        }
        if let Some((kind, i)) = action {
            match kind {
                0 => self.play_playlist_from(selected, Some(tracks[i].library_id)),
                6 => self.execute(AppCommand::SelectPlaylistRow(i)),
                5 => ui.ctx().copy_text(tracks[i].title.clone()),
                1 => self.execute(AppCommand::Insert(tracks[i].metadata(), true)),
                2 => self.execute(AppCommand::Insert(tracks[i].metadata(), false)),
                3 => {
                    if let Some(path) = self.state.offline().path(tracks[i].id()) {
                        self.execute(AppCommand::RemoveOffline(tracks[i].id(), path.to_owned()));
                    } else {
                        self.execute(AppCommand::SaveTrack(tracks[i].metadata()));
                    }
                }
                7 => self.open_source_message(tracks[i].chat, tracks[i].message),
                _ => self.link(&tracks[i]),
            }
            self.persist();
        }
    }
}

enum PlaylistSelectionAction {
    Insert { next: bool, reverse: bool },
    SaveOffline,
}

fn playlist_selection_menu(
    ui: &mut egui::Ui,
    items: &mut MenuItems,
) -> Option<PlaylistSelectionAction> {
    let mut action = None;
    for (next, label, reverse_label) in [
        (true, tr("Play next"), tr("Play next in reverse order")),
        (
            false,
            tr("Add to queue"),
            tr("Add to queue in reverse order"),
        ),
    ] {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.;
            let highlight = ui.painter().add(egui::Shape::Noop);
            let reverse_width = 40.;
            let width = ui.available_width() - reverse_width;
            let forward = ui.add(full_menu_button(label, width).fill(egui::Color32::TRANSPARENT));
            let reverse = ui
                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = design::colors(ui).raised;
                    let button = ui.add(
                        egui::Button::new("⇅")
                            .min_size(egui::vec2(reverse_width, ui.spacing().interact_size.y)),
                    );
                    button.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            ui.is_enabled(),
                            &reverse_label,
                        )
                    });
                    button.on_hover_text(reverse_label)
                })
                .inner;
            if forward.hovered() || forward.is_pointer_button_down_on() {
                let fill = design::colors(ui).selected;
                ui.painter().set(
                    highlight,
                    egui::Shape::rect_filled(forward.rect.union(reverse.rect), 8., fill),
                );
                ui.painter().rect_stroke(
                    reverse.rect,
                    8.,
                    egui::Stroke::new(2., fill),
                    egui::StrokeKind::Inside,
                );
            }
            items.push(ui, &forward, Some(&reverse));
            if forward.clicked() || reverse.clicked() {
                action = Some(PlaylistSelectionAction::Insert {
                    next,
                    reverse: reverse.clicked(),
                });
                ui.close();
            }
        });
    }
    let save = track_menu_button(ui, tr("Save offline"));
    items.push(ui, &save, None);
    if save.clicked() {
        action = Some(PlaylistSelectionAction::SaveOffline);
        ui.close();
    }
    action
}

pub(super) fn track_labels(
    ui: &mut egui::Ui,
    title: &str,
    artist: &str,
    active: bool,
    duration_width: f32,
) -> egui::Response {
    ui.allocate_ui(
        egui::vec2(
            (ui.available_width() - duration_width - 36. - 2. * ui.spacing().item_spacing.x)
                .max(0.),
            40.,
        ),
        |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.;
            ui.vertical(|ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(title).color(if active {
                        design::colors(ui).accent
                    } else {
                        ui.visuals().text_color()
                    }))
                    .truncate()
                    .show_tooltip_when_elided(false),
                );
                ui.add(
                    egui::Label::new(egui::RichText::new(artist).weak())
                        .truncate()
                        .show_tooltip_when_elided(false),
                );
            });
        },
    )
    .response
}
