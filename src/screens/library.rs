use super::{
    art, change_progress, chat_title_galley,
    keyboard::{self, empty_label, focus_outline, ListNavigation},
    list_scroll_area, search_field, show_chat_title, WorkspacePane, TRACK_SCROLL_GUTTER,
};
use crate::application::commands::AppCommand;
use crate::{design, tr, App};
use design::Icon;
use eframe::egui;
use std::collections::HashMap;

impl App {
    pub(super) fn library_screen(&mut self, ui: &mut egui::Ui, docked: bool) {
        let trigger = search_field(
            ui,
            &mut self.chat_query,
            "chat-search",
            tr("Find a chat"),
            Some(&mut self.chat_search_active),
        );
        let duration = if self.chat_search_active { 0.23 } else { 0.18 };
        let progress = change_progress(ui, "chat-list-mode", self.chat_search_active, duration);
        if docked {
            egui::Frame::NONE
                .inner_margin(egui::Margin {
                    right: -TRACK_SCROLL_GUTTER,
                    ..egui::Margin::ZERO
                })
                .show(ui, |ui| self.library_chats(ui, progress, trigger.as_ref()));
        } else {
            self.library_chats(ui, progress, trigger.as_ref());
        }
        if let Some(trigger) = trigger {
            focus_outline(ui, &trigger);
        }
    }

    fn library_chats(
        &mut self,
        ui: &mut egui::Ui,
        progress: f32,
        trigger: Option<&egui::Response>,
    ) {
        const ART_SIZE: f32 = 40.;
        const ROW_MARGIN: f32 = 8.;
        const ROW_HEIGHT: f32 = ART_SIZE + ROW_MARGIN * 2.;
        const ROW_GAP: f32 = 3.;
        const STAGGER: f32 = 0.45;
        let query = self.chat_query.trim();
        let search = self.state.library().search();
        let mut indices = self.chat_search_indices(self.chat_search_active, query);
        if self.chat_search_active && !query.is_empty() && search.query == query {
            if search.loading() {
                ui.weak(tr("Searching chats…"));
            }
            if let Some(error) = &search.error {
                ui.label(error.to_string());
                if ui.button(tr("Retry search")).clicked() {
                    self.retry_chat_search();
                }
            }
        }
        if indices.is_empty() {
            let label = if self.chat_search_active {
                tr("No chats found")
            } else {
                tr("Pin chats here, or find one using search.")
            };
            if let Some(trigger) = trigger {
                self.keyboard.empty(ui, WorkspacePane::Library, trigger);
                ui.weak(label);
            } else {
                let response = empty_label(ui, label);
                self.keyboard.empty(ui, WorkspacePane::Library, &response);
                focus_outline(ui, &response);
            }
        }
        let mut recent = Vec::new();
        if self.chat_search_active && self.chat_query.trim().is_empty() {
            let state = self.state.view();
            if !state.recent_chats.is_empty() {
                let mut by_id: HashMap<_, _> = indices
                    .iter()
                    .map(|&i| (state.playlists[i].id, i))
                    .collect();
                recent.extend(state.recent_chats.iter().filter_map(|id| by_id.remove(id)));
                indices.retain(|&i| by_id.contains_key(&state.playlists[i].id));
            }
        }
        let recent_len = recent.len();
        recent.extend(indices);
        let mut open = None;
        let mut toggle = None;
        let mut recent_action = None;
        let chats_scroll = list_scroll_area(ui).id_salt("chat-list").show(ui, |ui| {
            if recent_len > 0 {
                ui.horizontal(|ui| {
                    ui.weak(tr("Recent"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = tr("Clear recent chats");
                        let icon = Icon::Trash
                            .image(design::colors(ui).muted, 14.)
                            .alt_text(&label);
                        let clear = ui
                            .add(
                                egui::Button::image(icon)
                                    .frame_when_inactive(false)
                                    .min_size(egui::vec2(28., 28.)),
                            )
                            .on_hover_text(label);
                        #[cfg(test)]
                        ui.data_mut(|data| {
                            data.insert_temp(egui::Id::new("test-clear-recent-chats"), clear.id)
                        });
                        if clear.clicked() {
                            recent_action = Some(AppCommand::ClearRecentChats);
                        }
                    });
                });
            }
            let clip = ui.clip_rect();
            let stride = ROW_HEIGHT + ROW_GAP + ui.spacing().item_spacing.y;
            let nav = ListNavigation::new(WorkspacePane::Library, recent.len(), stride);
            let key_at = |i: usize| egui::Id::new(self.state.view().playlists[recent[i]].id);
            self.keyboard.step(ui, nav, false, key_at, None);
            let first_visible =
                ((clip.top() - ui.next_widget_position().y) / stride).floor() as usize;
            let visible_slots = (clip.height() / stride).ceil().max(1.) as usize;
            for (row, i) in recent.into_iter().enumerate() {
                if recent_len > 0 && row == recent_len {
                    ui.separator();
                }
                let delay = STAGGER * row.saturating_sub(first_visible).min(visible_slots) as f32
                    / visible_slots as f32;
                let phase = ((progress - delay) / (1. - STAGGER)).clamp(0., 1.);
                let arrived = egui::emath::easing::cubic_out(phase);
                let origin = ui.next_widget_position()
                    + egui::vec2(ui.available_width() / 2., ROW_HEIGHT / 2.);
                let scale = egui::lerp(0.65..=1., arrived);
                let transform = egui::emath::TSTransform::new(
                    origin.to_vec2() * (1. - scale) + egui::vec2(0., -26. * (1. - arrived)),
                    scale,
                );
                let p = &self.state.view().playlists[i];
                let id = p.id;
                let pinned = self.state.view().chat_pins.contains(&id);
                let row_id = ui.id().with(("chat", id));
                let focused = ui.memory(|m| m.has_focus(row_id));
                let colors = design::colors(ui);
                let frame = egui::Frame::new()
                    .fill(if focused {
                        colors.raised
                    } else if self.opened_chat == Some(id) {
                        colors.selected
                    } else {
                        egui::Color32::TRANSPARENT
                    })
                    .corner_radius(8)
                    .inner_margin(ROW_MARGIN);
                let draw = |ui: &mut egui::Ui| {
                    frame
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                art(
                                    ui,
                                    &p.title,
                                    self.state.library().playlist_artwork(p.id).path.as_deref(),
                                    ART_SIZE,
                                );
                                chat_title(ui, &p.title, p.emoji_status.is_some(), 14., 1);
                                self.chat_emoji(ui, p.emoji_status, 18., None);
                            });
                        })
                        .response
                        .rect
                };
                let rect = if phase < 1. {
                    ui.with_visual_transform(transform, |ui| {
                        ui.set_clip_rect(transform.inverse() * clip);
                        ui.multiply_opacity((2. * phase).min(1.));
                        draw(ui)
                    })
                    .inner
                } else {
                    ui.scope(draw).inner
                };
                ui.add_space(ROW_GAP);
                if phase == 0. {
                    if self.keyboard.targets(nav.pane, egui::Id::new(id)) {
                        self.keyboard.reveal(ui, rect, nav, row);
                    }
                    continue;
                }
                let response = ui.interact(
                    transform * rect,
                    row_id,
                    keyboard::row_sense(ui, transform * rect),
                );
                if !ui.is_rect_visible(response.rect)
                    && self.keyboard.targets(nav.pane, egui::Id::new(id))
                {
                    self.keyboard.reveal(ui, rect, nav, row);
                }
                self.keyboard.row(
                    ui,
                    nav,
                    row,
                    egui::Id::new(id),
                    false,
                    (&response, response.rect),
                );
                if ui.is_rect_visible(response.rect) {
                    self.image_wanted
                        .push(self.state.library().playlist_artwork(p.id).file);
                }
                if response.clicked() {
                    open = Some(id);
                }
                response.context_menu(|ui| {
                    if ui
                        .button(if pinned {
                            tr("Unpin chat")
                        } else {
                            tr("Pin chat")
                        })
                        .clicked()
                    {
                        toggle = Some(id);
                        ui.close();
                    }
                    if row < recent_len && ui.button(tr("Remove from Recent")).clicked() {
                        recent_action = Some(AppCommand::RemoveRecentChat(id));
                        ui.close();
                    }
                });
            }
        });
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new("test-chat-scroll"),
                (chats_scroll.id, chats_scroll.inner_rect),
            )
        });
        design::middle_scroll(
            ui,
            chats_scroll.id,
            chats_scroll.inner_rect,
            chats_scroll.content_size.y,
        );
        if let Some(id) = toggle {
            self.toggle_chat_pin(id);
        }
        if let Some(command) = recent_action {
            self.execute(command);
        }
        if let Some(id) = open {
            self.execute(if self.chat_search_active {
                AppCommand::OpenSearchPlaylist(id)
            } else {
                AppCommand::OpenPlaylist(id)
            });
            self.pane = WorkspacePane::Playlist;
            ui.ctx().request_repaint();
            self.chat_search_active = false;
            self.chat_query.clear();
            ui.ctx()
                .memory_mut(|m| m.surrender_focus(egui::Id::new("chat-search")));
        }
    }
}

fn chat_title(
    ui: &mut egui::Ui,
    title: &str,
    emoji: bool,
    size: f32,
    max_rows: usize,
) -> egui::Pos2 {
    let galley = chat_title_galley(ui, title, emoji, size, max_rows, ui.available_width());
    show_chat_title(ui, galley)
}
