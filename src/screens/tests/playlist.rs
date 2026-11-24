use super::*;

#[test]
fn long_metadata_fits_virtual_list_row_height() {
    let ctx = egui::Context::default();
    design::style(&ctx, false);
    let long = "A very long title or artist name ".repeat(40);
    let mut height = 0.;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        ui.set_width(600.);
        let row = egui::Frame::new().inner_margin(6.).show(ui, |ui| {
            ui.set_min_height(48.);
            ui.horizontal(|ui| {
                track_labels(ui, &long, &long, false);
            });
        });
        height = row.response.rect.height();
    });
    output.textures_delta.clear();
    assert!(
        (height - design::ROW_HEIGHT).abs() < 0.1,
        "Rendered row {height} differs from virtual row height 60"
    );
}

#[test]
fn playlist_heading_wraps_at_word_boundaries() {
    let mut h = Harness::new();
    h.size = egui::vec2(720., 620.);
    h.settle();
    let title = "Evening piano improvisations from the mountain studio";
    let mut state = h.app.state.view().clone();
    state.playlists[0].title = title.into();
    h.app.state = tg_player_application::AppCore::new(state);
    let mut output = h.ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, h.size)),
            time: Some(h.time + 0.02),
            ..Default::default()
        },
        |ui| h.app.render(ui),
    );
    output.textures_delta.clear();
    let heading = output
        .shapes
        .into_iter()
        .find_map(|shape| match shape.shape {
            egui::Shape::Text(text)
                if text.galley.text() == title && text.galley.rows.len() == 2 =>
            {
                Some(text.galley)
            }
            _ => None,
        })
        .expect("the playlist heading wraps to two lines");
    let lines: Vec<_> = heading
        .rows
        .iter()
        .map(|row| row.glyphs.iter().map(|glyph| glyph.chr).collect::<String>())
        .collect();
    assert_eq!(
        format!("{} {}", lines[0].trim_end(), lines[1].trim_start()),
        title
    );
}

#[test]
fn playlist_status_follows_the_last_visible_title_row() {
    let mut h = Harness::new();
    h.size = egui::vec2(720., 620.);
    h.settle();
    h.app.custom_emoji.set_demo();
    let wrapped = "Evening piano improvisations from the mountain studio";
    for (title, rows, elided) in [
        ("Quiet evening".into(), 1, false),
        (wrapped.into(), 2, false),
        (wrapped.repeat(4), 2, true),
    ] {
        let mut state = h.app.state.view().clone();
        state.playlists[0].title = title.clone();
        state.playlists[0].emoji_status = Some(tg_player_application::model::EmojiStatus {
            id: crate::adapters::emoji::DEMO_ID,
            expires_at: 0,
        });
        h.app.state = tg_player_application::AppCore::new(state);
        h.time += 0.1;
        let mut output = h.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, h.size)),
                time: Some(h.time),
                ..Default::default()
            },
            |ui| h.app.render(ui),
        );
        output.textures_delta.clear();
        let heading = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == title => Some(text),
                _ => None,
            })
            .unwrap();
        assert_eq!(heading.galley.rows.len(), rows);
        assert_eq!(heading.galley.elided, elided);
        let line = heading
            .galley
            .rows
            .last()
            .unwrap()
            .rect()
            .translate(heading.pos.to_vec2());
        let badge = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh) => {
                    let bounds = mesh.calc_bounds();
                    (bounds.size() == egui::Vec2::splat(24.)).then_some(bounds)
                }
                _ => None,
            })
            .expect("animated status texture");
        assert!((badge.left() - line.right() - 8.).abs() < 1.);
        assert!((badge.center().y - line.center().y).abs() < 1.);
        assert!(badge.right() <= h.size.x - 24.);
    }
}

#[test]
fn playlist_feedback_is_inline_and_elided_titles_have_no_tooltip() {
    let mut h = Harness::new();
    h.app.set_inline_notice("Link copied".into(), false);
    h.frame(vec![]);
    let row = h.app.test_rows[&0].0;
    let title = h.app.playlist_tracks(0)[0].title.clone();
    let pointer = egui::pos2(row.left() + 110., row.top() + 18.);
    h.frame(vec![egui::Event::PointerMoved(pointer)]);
    let mut output = h.ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, h.size)),
            time: Some(h.time + 1.),
            ..Default::default()
        },
        |ui| h.app.render(ui),
    );
    output.textures_delta.clear();
    let mut titles = 0;
    let mut feedback = None;
    for shape in output.shapes {
        if let egui::Shape::Text(text) = shape.shape {
            if text.galley.text() == title {
                titles += 1;
            } else if text.galley.text() == "Link copied" {
                feedback = Some(text.pos);
            }
        }
    }
    assert_eq!(titles, 1, "the title should only be drawn in its row");
    let feedback = feedback.expect("copy confirmation should remain visible");
    assert!(feedback.y < row.top() && feedback.x >= row.left());
}

#[test]
fn escape_clears_track_search() {
    let mut h = Harness::new();
    let id = egui::Id::new("track-search");
    let field = h.ctx.read_response(id).unwrap().rect;
    h.click(field.center());
    h.frame(vec![egui::Event::Text("Afterglow".into())]);
    assert_eq!(h.app.search, "Afterglow");
    h.key(egui::Key::Escape);
    assert!(h.app.search.is_empty());
}

#[test]
fn play_all_starts_the_playlist_when_the_search_has_no_results() {
    let mut h = Harness::new();
    h.size.y = 620.;
    h.settle();
    let count = h.app.playlist_tracks(0).len();
    h.app.search = "nothing matches this".into();
    h.frame(vec![]);
    assert!(h.app.playlist_tracks(0).is_empty());
    let play: egui::Rect = h
        .ctx
        .data(|d| d.get_temp(egui::Id::new("test-play-all")))
        .unwrap();
    h.click(play.center());
    let session = &h.app.state.view().session;
    assert_eq!(session.current().unwrap().message, 0);
    assert_eq!(session.snapshot().len(), count);
}

#[test]
fn playlist_pin_is_last_in_the_menu_after_the_sort_control() {
    let mut h = Harness::new();
    h.size.y = 620.;
    h.settle();
    let render = |h: &mut Harness| {
        h.time += 0.02;
        let mut output = h.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, h.size)),
                time: Some(h.time),
                ..Default::default()
            },
            |ui| h.app.render(ui),
        );
        output.textures_delta.clear();
        output
            .shapes
            .into_iter()
            .filter_map(|shape| match shape.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().to_owned(),
                    text.visual_bounding_rect().center(),
                )),
                _ => None,
            })
            .fold(
                std::collections::HashMap::new(),
                |mut positions, (label, center)| {
                    positions.entry(label).or_insert(center);
                    positions
                },
            )
    };
    let toolbar = render(&mut h);
    assert!(!toolbar.contains_key("Unpin chat"));
    let menu = toolbar["…"];
    assert!(toolbar["Newest first"].x < menu.x);
    h.click(menu);
    let actions = render(&mut h);
    let unpin = actions["Unpin chat"];
    assert!(unpin.y > actions["Refresh library"].y);
    h.click(unpin);
    assert!(!h.app.state.view().chat_pins.contains(&0));
    h.click(menu);
    assert!(render(&mut h).contains_key("Pin chat"));
}

#[test]
fn choosing_a_search_result_rebuilds_queue_for_mouse_keyboard_and_menu() {
    for activation in ["mouse", "keyboard", "menu"] {
        let mut h = Harness::new();
        h.size = egui::vec2(1240., 800.);
        h.frame(vec![]);
        h.time += 1.;
        h.click(h.app.test_rows[&1].0.center());
        assert_eq!(
            h.app
                .state
                .view()
                .session
                .current()
                .as_ref()
                .unwrap()
                .message,
            1
        );
        h.app.search = "Slow motion 05".into();
        h.app.test_rows.clear();
        h.frame(vec![]);
        h.time += 1.;
        h.frame(vec![]);
        assert_eq!(h.app.test_rows.len(), 1);
        let row = h.app.test_rows[&4].0;
        match activation {
            "keyboard" => {
                h.ctx
                    .memory_mut(|m| m.request_focus(h.app.test_row_ids[&4]));
                h.key(egui::Key::Enter);
            }
            "menu" => {
                h.click(h.app.test_rows[&4].1.center());
                h.frame(vec![]);
                h.click(h.app.test_play_buttons[&4].center());
            }
            _ => h.click(row.center()),
        }
        assert_eq!(
            h.app
                .state
                .view()
                .session
                .current()
                .as_ref()
                .unwrap()
                .message,
            4
        );
        h.app.execute(AppCommand::Previous);
        assert_eq!(
            h.app
                .state
                .view()
                .session
                .current()
                .as_ref()
                .unwrap()
                .message,
            3
        );
        h.app.execute(AppCommand::Next);
        assert_eq!(
            h.app
                .state
                .view()
                .session
                .current()
                .as_ref()
                .unwrap()
                .message,
            4
        );
        h.app.shutdown_core().unwrap();
    }
}

#[test]
fn right_click_opens_track_menu_without_starting_playback() {
    let mut h = Harness::new();
    let row = h.app.test_rows[&0].0;
    let p = row.center();
    for pressed in [true, false] {
        h.frame(vec![
            egui::Event::PointerMoved(p),
            egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
    }
    assert!(h.app.state.view().session.current().is_none());
    assert!(egui::Popup::is_any_open(&h.ctx));
    h.key(egui::Key::Escape);
}

#[test]
fn track_row_mouse_keyboard_and_menu_playback() {
    let mut h = Harness::new();
    let (row, menu) = h.app.test_rows[&0];
    assert!(
        row.right() <= 850. && row.bottom() < 454.,
        "row overflow: {row:?}"
    );
    assert_eq!(row.height(), design::ROW_HEIGHT);
    // Cover, title and blank row space all activate the same row.
    for x in [row.left() + 50., row.left() + 160., row.right() - 60.] {
        h.app.execute(AppCommand::Stop);
        h.time += 1.;
        h.click(egui::pos2(x, row.center().y));
        assert_eq!(
            h.app
                .state
                .view()
                .session
                .current()
                .as_ref()
                .map(|t| t.message),
            Some(0)
        );
        assert!(!h.app.state.playback().state().paused());
    }
    h.time += 1.;
    h.click(row.center());
    assert!(h.app.state.playback().state().paused());
    h.time += 1.;
    h.click(row.center());
    h.click(row.center());
    assert!(
        !h.app.state.playback().state().paused(),
        "double click must not pause after starting"
    );
    h.time += 1.;
    h.click(menu.center());
    assert!(
        !h.app.state.playback().state().paused(),
        "menu must not activate row"
    );
    h.key(egui::Key::Escape);
    h.time += 1.;
    h.click(row.center());
    assert!(h.app.state.playback().state().paused());
    h.key(egui::Key::Enter);
    assert!(!h.app.state.playback().state().paused());
    h.key(egui::Key::Space);
    assert!(
        h.app.state.playback().state().paused(),
        "Space on row toggles playback"
    );

    // Space is text while a search field owns focus.
    let search = egui::Id::new("track-search");
    h.ctx.memory_mut(|m| m.request_focus(search));
    h.frame(vec![]);
    let paused = h.app.state.playback().state().paused();
    h.key(egui::Key::Space);
    assert_eq!(h.app.state.playback().state().paused(), paused);
    let mut reached_row = false;
    for _ in 0..60 {
        h.key(egui::Key::Tab);
        let focused = h.ctx.memory(|m| m.focused());
        if h.app.test_row_ids.values().any(|&id| Some(id) == focused) {
            reached_row = true;
            break;
        }
    }
    assert!(reached_row, "Tab must reach track rows");
}

#[test]
fn warmed_playlist_covers_are_painted_after_scrolling_in_both_directions() {
    let mut h = Harness::new();
    egui_extras::install_image_loaders(&h.ctx);
    crate::demo::apply(&mut h.app, "artwork");

    let cover_at = |h: &Harness, frame: &egui::FullOutput, track: i64| {
        let row = h.app.test_rows[&track].0;
        frame.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect)
                if row.contains(rect.rect.center())
                    && rect.rect.width() >= TRACK_ARTWORK_SIZE - 1.
                    && rect.rect.width() <= TRACK_ARTWORK_SIZE + 1. =>
            {
                rect.brush.as_ref().map(|brush| brush.fill_texture_id)
            }
            _ => None,
        })
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let first = loop {
        let frame = h.frame(vec![]);
        if let Some(first) = cover_at(&h, &frame, 0) {
            // Give the offscreen images the same chance to complete as the first row.
            let all = h
                .ctx
                .loaders()
                .texture
                .lock()
                .iter()
                .any(|loader| loader.byte_size() >= 6 * 96 * 96 * 4);
            if all {
                break first;
            }
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    };

    let scroll_id = h
        .ctx
        .data(|data| data.get_temp(egui::Id::new("test-playlist-scroll")))
        .unwrap();
    let offset = |h: &mut Harness, y| {
        let mut state = egui::scroll_area::State::load(&h.ctx, scroll_id).unwrap();
        state.offset.y = y;
        state.store(&h.ctx, scroll_id);
        h.app.test_rows.clear();
        h.frame(vec![])
    };
    let down = offset(&mut h, 12. * design::ROW_HEIGHT);
    assert_eq!(cover_at(&h, &down, 12), Some(first));
    let up = offset(&mut h, 0.);
    assert_eq!(cover_at(&h, &up, 0), Some(first));
}

#[test]
fn removing_track_selected_and_all_downloads_preserves_cover_and_cache_schedule() {
    use tg_player_application::{telegram::AuthorizationState, workflow::AppEvent};

    for removal in ["track", "selected", "all"] {
        let mut h = Harness::new();
        crate::demo::apply(&mut h.app, "offline-storage");
        h.app.settings_open = false;
        crate::demo::apply(&mut h.app, "artwork");
        h.app.tg = Some(crate::adapters::telegram::Telegram::mock_rpc(|request| {
            panic!("unexpected Telegram request after unpinning audio: {request}")
        }));
        h.app
            .state
            .event(AppEvent::Authorization(AuthorizationState::Ready));
        egui_extras::install_image_loaders(&h.ctx);
        h.settle();
        let id = h.app.state.view().playlists[0].tracks[0].id();
        let path = h.app.artwork_path(1).unwrap().to_owned();
        let deadline = h.app.maintain_cache(std::time::Instant::now()).unwrap();
        let size = egui::Vec2::splat(TRACK_ARTWORK_SIZE);
        let wait_until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let texture = loop {
            match artwork_image(&path, TRACK_ARTWORK_SIZE).load_for_size(&h.ctx, size) {
                Ok(egui::load::TexturePoll::Ready { texture }) => break texture.id,
                _ => {
                    assert!(std::time::Instant::now() < wait_until, "cover did not load");
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            }
        };

        if removal == "track" {
            h.click(h.app.test_rows[&id.message].1.center());
            let frame = h.frame(vec![]);
            let remove = frame
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Remove offline download" => {
                        Some(text.visual_bounding_rect().center())
                    }
                    _ => None,
                })
                .expect("removal action in track menu");
            h.click(remove);
        } else {
            h.app.offline_confirm = Some(if removal == "selected" {
                vec![0, 2]
            } else {
                vec![0, 2, 4]
            });
            h.settle();
            h.key(egui::Key::Tab);
            h.key(egui::Key::Enter);
        }
        assert!(h.app.state.offline().path(id).is_none(), "{removal}");
        assert_eq!(
            h.app.maintain_cache(std::time::Instant::now()),
            Some(deadline)
        );
        h.app.process(&h.ctx);
        assert_eq!(h.app.artwork_path(1), Some(path.as_str()));
        assert_eq!(
            artwork_image(&path, TRACK_ARTWORK_SIZE)
                .load_for_size(&h.ctx, size)
                .unwrap()
                .texture_id(),
            Some(texture),
        );
    }
}
