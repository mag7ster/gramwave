use super::{
    art, clock, clock_width, horizontal_slot, layout_fraction, reveal_layout, PlayerLayout,
};
use crate::application::commands::AppCommand;
use crate::{design, tr, App, RepeatMode};
use design::{button, Icon};
use eframe::egui;
use std::time::Duration;

impl App {
    pub(super) fn animated_transport(
        &mut self,
        root: &mut egui::Ui,
        viewport: egui::Rect,
        rect: egui::Rect,
        player: PlayerLayout,
        dock: bool,
    ) {
        let seek_active = matches!(player, PlayerLayout::Mini | PlayerLayout::Full);
        let seek = layout_fraction(root, "transport-seek", seek_active);
        let clip = viewport.intersect(rect.with_max_y(rect.bottom().max(rect.top() + 84. * seek)));
        root.painter()
            .rect_filled(clip, 0., design::colors(root).background);
        if player.has_workspace() {
            let stroke = root.visuals().widgets.noninteractive.bg_stroke;
            root.painter()
                .hline(rect.x_range(), rect.top() + stroke.width / 2., stroke);
        }
        let mut ui = root.new_child(
            egui::UiBuilder::new()
                .id_salt("transport-controls")
                .max_rect(rect.shrink2(player.margin()))
                .sense(egui::Sense::CLICK)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        ui.set_clip_rect(clip);
        reveal_layout(&mut ui, "transport-layout", player.has_workspace());
        self.transport(&mut ui, player, dock, seek);
    }

    fn transport_buttons(
        &mut self,
        ui: &mut egui::Ui,
        center: egui::Pos2,
        reverse: f32,
        shuffle: f32,
        repeat: f32,
    ) {
        let gap = ui.spacing().item_spacing.x;
        let slot_width = 36. + gap;
        let half_width = 22. + slot_width;
        if reverse > 0. {
            let point = center - egui::vec2(half_width + (1. + shuffle) * slot_width, 0.);
            let mut slot = horizontal_slot(ui, "reverse-queue", point, egui::Vec2::splat(36.));
            slot.multiply_opacity(reverse);
            let session = &self.state.view().session;
            let enabled = !self.state.queue_reversing()
                && (session.timeline().len() > 1 || session.playlist_page_request().is_some());
            let active = session.playlist_order() == gramwave_domain::PlaylistOrder::OldestFirst;
            let hint = if active {
                tr("Reverse on · Turn off")
            } else {
                tr("Reverse off · Turn on")
            };
            let reverse = slot.add_enabled_ui(enabled, |ui| {
                design::toggle(ui, Icon::Reverse, active, "", hint)
            });
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("test-reverse-queue"), reverse.response.rect);
            });
            if reverse.inner {
                self.execute(AppCommand::ReverseQueue);
            }
        }
        if shuffle > 0. {
            let point = center - egui::vec2(half_width + slot_width, 0.);
            let mut button = horizontal_slot(ui, "shuffle", point, egui::Vec2::splat(36.));
            button.multiply_opacity(shuffle);
            if reverse > 0. {
                let left = point.x + slot_width * (1. - shuffle);
                button.set_clip_rect(
                    button
                        .clip_rect()
                        .with_min_x(left.max(button.clip_rect().left())),
                );
            }
            let shuffle_hint = if self.state.view().shuffle {
                tr("Shuffle on · Turn off")
            } else {
                tr("Shuffle off · Turn on")
            };
            if design::toggle(
                &mut button,
                Icon::Shuffle,
                self.state.view().shuffle,
                "",
                shuffle_hint,
            ) {
                self.execute(AppCommand::Shuffle(!self.state.view().shuffle));
            }
        }
        let mut playback = horizontal_slot(
            ui,
            "playback",
            center - egui::vec2(half_width, 0.),
            egui::vec2(2. * half_width, 44.),
        );
        if button(&mut playback, Icon::Previous, "Previous") {
            self.execute(AppCommand::Previous);
        }
        let waiting = self.state.queue_waiting();
        let loading = waiting || self.state.playback().shows_loading();
        let icon = if self.state.playback().state().paused()
            || self.state.view().session.current().is_none()
        {
            Icon::Play
        } else {
            Icon::Pause
        };
        if design::playback_control(
            &mut playback,
            (!loading).then_some(icon),
            if waiting {
                "Waiting for more tracks…"
            } else if loading {
                "Loading track…"
            } else {
                "Play / pause"
            },
        ) {
            self.execute(AppCommand::Pause(!self.state.playback().state().paused()));
        }
        if button(&mut playback, Icon::Next, "Next") {
            self.execute(AppCommand::Next);
        }
        if repeat > 0. {
            let point = center + egui::vec2(half_width + gap, 0.);
            let mut button = horizontal_slot(ui, "repeat", point, egui::Vec2::splat(36.));
            button.multiply_opacity(repeat);
            clip_slot_right(&mut button, point.x + slot_width * repeat);
            let hint = match self.state.view().repeat {
                RepeatMode::Off => tr("Repeat off · Repeat all"),
                RepeatMode::All => tr("Repeat all · Repeat one"),
                RepeatMode::One => tr("Repeat one · Turn off"),
            };
            if design::toggle(
                &mut button,
                Icon::Repeat,
                self.state.view().repeat != RepeatMode::Off,
                if self.state.view().repeat == RepeatMode::One {
                    "1"
                } else {
                    ""
                },
                hint,
            ) {
                self.execute(AppCommand::Repeat(self.state.view().repeat.next()));
            }
        }
    }

    fn queue_button(&mut self, ui: &mut egui::Ui) {
        let open = self.state.view().queue_panel_open;
        if design::toggle(
            ui,
            Icon::Queue,
            open,
            "",
            if open {
                tr("Queue open · Close queue")
            } else {
                tr("Queue closed · Open queue")
            },
        ) {
            self.set_queue_panel_open(!open);
            ui.ctx().request_repaint();
        }
    }

    fn mute_button(&mut self, ui: &mut egui::Ui) {
        let (icon, label) = if self.state.view().volume == 0. {
            (Icon::Muted, "Unmute")
        } else {
            (Icon::Volume, "Mute")
        };
        if button(ui, icon, label) {
            self.toggle_mute();
        }
    }

    fn volume_slider(&mut self, ui: &mut egui::Ui, width: f32) {
        let mut volume = self.state.view().volume as f64;
        let slider = design::slider(ui, &mut volume, 100., width, tr("Volume"), false);
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new("test-volume"), (slider.rect, slider.id));
        });
        if slider.changed() {
            self.execute(AppCommand::Volume(volume as f32));
        }
    }

    fn transport(&mut self, ui: &mut egui::Ui, player: PlayerLayout, dock: bool, seek: f32) {
        let area = ui.available_rect_before_wrap();
        let width = area.width();
        let gap = ui.spacing().item_spacing.x;
        let button_slot = 36. + gap;
        let basic_width = 44. + 2. * button_slot;
        // On shrink, hide the volume slider, reverse, shuffle, repeat, then mute.
        let [mute, repeat, shuffle, reverse] = [
            ("transport-mute", 1.),
            ("transport-repeat", 2.),
            ("transport-shuffle", 3.),
            ("transport-reverse", 4.),
        ]
        .map(|(id, slots)| {
            let fits = basic_width + slots * button_slot;
            let fraction = layout_fraction(ui, id, width >= fits + button_slot);
            // Leave room for the fade-out unless a larger resize takes the slot away.
            if width >= fits {
                fraction
            } else {
                0.
            }
        });
        let queue = layout_fraction(ui, "transport-queue", dock && player.has_workspace());
        let queue = if width >= 798. { queue } else { 0. };
        let queue_width = queue * button_slot;
        let left_width = (reverse + shuffle) * button_slot;
        let play_width = basic_width + left_width + repeat * button_slot;
        let available_slider = width - basic_width - 4. * button_slot - queue_width - gap;
        let slider = layout_fraction(ui, "transport-volume", available_slider >= 72.);
        let slider = if available_slider >= 28. { slider } else { 0. };
        let compact_slider = available_slider.clamp(28., 104.);
        let compact_buttons = play_width + queue_width + mute * button_slot;
        let compact_width = compact_buttons + (gap + compact_slider) * slider;
        let slider_footprint = if slider > 0. {
            let slider_width = egui::lerp(28. ..=compact_slider, slider);
            gap + (compact_slider + slider_width) / 2.
        } else {
            0.
        };
        let compact_left = ((width - compact_width - button_slot * slider) / 2.)
            .min(width - compact_buttons - slider_footprint)
            .max(0.);

        let seek_active = matches!(player, PlayerLayout::Mini | PlayerLayout::Full);
        let expanded = player == PlayerLayout::Full && width >= 798.;
        let mut wide = layout_fraction(ui, "transport-wide", expanded);
        let metadata_width = ((width - 48.) * 0.25).clamp(60., 300.);
        let wide_left = 24. + metadata_width + gap;
        let wide_right = 144. + 56. * queue;
        let wide_center = width - 48. - metadata_width - wide_right - 24.;
        if wide_center < basic_width + 2. * left_width.max(repeat * button_slot) {
            wide = 0.;
        }
        let center_left = wide * wide_left;
        let center_width = egui::lerp(width..=wide_center, wide);
        let play_x = area.left()
            + egui::lerp(
                (compact_left + left_width + basic_width / 2.)..=(wide_left + wide_center / 2.),
                wide,
            )
            .clamp(
                basic_width / 2.,
                (width - basic_width / 2.).max(basic_width / 2.),
            );
        let play_y = (area.top() + 22. + 8. * seek).min(ui.clip_rect().bottom() - 22.);
        let right_x = area.left()
            + egui::lerp(
                (compact_left + play_width + gap)..=(wide_left + wide_center + gap),
                wide,
            );
        let volume_x = right_x + queue_width;
        let volume_y = play_y + 12. * wide;

        let media = if seek_active || wide > 0. {
            self.state
                .view()
                .session
                .current()
                .map(|t| self.media_track(t.into()))
        } else {
            None
        };
        if let Some(track) = &media {
            self.image_wanted.push(track.image_file);
            if track.file == 0 {
                self.track_wanted.push(track.clone());
            }
        }
        if wide > 0. {
            let point = egui::pos2(area.left() + 24. - 14. * (1. - wide), play_y + 14.);
            let mut metadata =
                horizontal_slot(ui, "metadata", point, egui::vec2(metadata_width, 72.));
            metadata.multiply_opacity(wide);
            metadata.set_clip_rect(
                metadata
                    .clip_rect()
                    .with_max_x((play_x - basic_width / 2. - left_width - gap).max(area.left())),
            );
            metadata.horizontal(|ui| {
                let track = media.as_ref();
                art(
                    ui,
                    track.map(|t| t.title.as_str()).unwrap_or("TG"),
                    track.and_then(|t| self.track_cover(ui.ctx(), t)),
                    48.,
                );
                ui.vertical(|ui| {
                    ui.set_width((metadata_width - 48. - gap).max(0.));
                    let title = track
                        .map(|t| t.title.clone())
                        .unwrap_or_else(|| tr("Choose your soundtrack"));
                    let artist = track
                        .map(|t| t.artist.clone())
                        .unwrap_or_else(|| tr("Select a track to get started"));
                    ui.add(
                        egui::Label::new(egui::RichText::new(title).strong())
                            .truncate()
                            .show_tooltip_when_elided(false),
                    );
                    ui.add(
                        egui::Label::new(egui::RichText::new(artist).weak())
                            .truncate()
                            .show_tooltip_when_elided(false),
                    );
                });
            });
        }
        self.transport_buttons(ui, egui::pos2(play_x, play_y), reverse, shuffle, repeat);
        if queue > 0. {
            let point = egui::pos2(right_x, volume_y);
            let mut button = horizontal_slot(ui, "queue", point, egui::Vec2::splat(36.));
            button.multiply_opacity(queue);
            clip_slot_right(&mut button, volume_x);
            self.queue_button(&mut button);
        }
        if mute > 0. {
            let point = egui::pos2(volume_x, volume_y);
            let mut button = horizontal_slot(ui, "mute", point, egui::Vec2::splat(36.));
            button.multiply_opacity(mute);
            if slider > 0. {
                clip_slot_right(&mut button, volume_x + button_slot * mute);
            }
            self.mute_button(&mut button);
        }
        if slider > 0. {
            let full_width = egui::lerp(compact_slider..=(96. + 8. * queue), wide);
            let slider_width = egui::lerp(28. ..=full_width, slider);
            let point = egui::pos2(
                volume_x + button_slot * mute + (full_width - slider_width) / 2.,
                volume_y,
            );
            let mut volume = horizontal_slot(ui, "volume", point, egui::vec2(slider_width, 28.));
            volume.multiply_opacity(slider);
            self.volume_slider(&mut volume, slider_width);
        }
        if !seek_active {
            self.seek_preview = None;
        }
        if seek > 0. {
            let timeline_width = egui::lerp((center_width * 0.55).max(140.)..=center_width, seek);
            let point = egui::pos2(
                area.left() + center_left + (center_width - timeline_width) / 2.,
                play_y + egui::lerp(36. ..=40., seek),
            );
            let mut timeline =
                horizontal_slot(ui, "timeline", point, egui::vec2(timeline_width, 28.));
            timeline.multiply_opacity(seek);
            let ui = &mut timeline;
            let playback = self.state.playback().state();
            let snapshot = self
                .player
                .as_ref()
                .map(|player| player.snapshot())
                .unwrap_or_default();
            let current = self.state.view().session.current();
            let restored = self.restored_position(snapshot);
            let duration = if restored.is_some() {
                current.map_or(0., |track| track.duration as f64)
            } else if playback.loading() {
                0.
            } else if self.demo || snapshot.duration <= 0. {
                current.as_ref().map_or(0., |track| track.duration as f64)
            } else {
                snapshot.duration
            };
            let has_duration = duration.is_finite() && duration > 0. && current.is_some();
            let enabled = seek_active && restored.is_none() && has_duration;
            let position = if let Some(position) = restored {
                position
            } else if has_duration {
                let sampled = if snapshot.seeking {
                    snapshot.position
                } else {
                    snapshot.audio_position.unwrap_or(snapshot.position)
                };
                sampled.max(0.).min(duration)
            } else {
                0.
            };
            let mut preview = self.seek_preview.unwrap_or(position);
            let hours = duration >= 3600.;
            let duration_label = if has_duration {
                clock(duration, hours)
            } else {
                "—:—".into()
            };
            let digits = "8".repeat(duration_label.chars().count().saturating_sub(6).max(1));
            let clock_sample = if hours {
                format!("{digits}:88:88")
            } else {
                "88:88".into()
            };
            let left_width = clock_width(ui, format!("−{clock_sample}")).max(42.);
            let right_width = clock_width(ui, clock_sample).max(42.);
            ui.horizontal(|ui| {
                let can_toggle = seek_active && has_duration;
                let time = if self.show_remaining_time && has_duration {
                    format!("−{}", clock((duration - preview).max(0.).ceil(), hours))
                } else {
                    clock(preview, hours)
                };
                let response = ui.add_sized(
                    [left_width, 28.],
                    egui::Label::new(egui::RichText::new(time).small()).sense(if can_toggle {
                        egui::Sense::click()
                    } else {
                        egui::Sense::hover()
                    }),
                );
                if can_toggle {
                    response
                        .clone()
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(if self.show_remaining_time {
                            tr("Show elapsed time")
                        } else {
                            tr("Show remaining time")
                        });
                    if response.clicked() {
                        self.show_remaining_time = !self.show_remaining_time;
                        ui.request_repaint();
                    }
                }
                let response = ui
                    .add_enabled_ui(enabled, |ui| {
                        ui.push_id(("seek", self.state.playback().generation()), |ui| {
                            design::slider(
                                ui,
                                &mut preview,
                                duration.max(0.),
                                (timeline_width - left_width - right_width - 2. * gap).max(40.),
                                tr("Seek"),
                                true,
                            )
                        })
                        .inner
                    })
                    .inner;
                if enabled
                    && self.player.is_some()
                    && !playback.paused()
                    && !snapshot.seeking
                    && !response.is_pointer_button_down_on()
                    && ui.is_rect_visible(response.rect)
                {
                    let rail_pixels =
                        design::slider_rail(response.rect).width() * ui.ctx().pixels_per_point();
                    design::repaint_after(ui, seek_repaint_interval(duration, rail_pixels));
                }
                #[cfg(test)]
                {
                    self.test_seek = Some((response.rect, response.id));
                }
                if enabled && (response.dragged() || response.is_pointer_button_down_on()) {
                    self.seek_preview = Some(preview);
                }
                if enabled
                    && (response.drag_stopped()
                        || response.clicked()
                        || (response.changed() && !response.is_pointer_button_down_on()))
                {
                    self.execute(AppCommand::Seek(preview));
                }
                ui.add_sized(
                    [right_width, 28.],
                    egui::Label::new(egui::RichText::new(duration_label).small()),
                );
            });
        }
    }
}

fn clip_slot_right(ui: &mut egui::Ui, right: f32) {
    let clip = ui.clip_rect();
    ui.set_clip_rect(clip.with_max_x(clip.right().min(right)));
}

pub(super) fn seek_repaint_interval(duration: f64, rail_pixels: f32) -> Duration {
    let seconds_per_quarter_pixel = 0.25 * duration.max(1.) / f64::from(rail_pixels);
    Duration::from_secs_f64(seconds_per_quarter_pixel.clamp(1. / 60., 0.2))
}
