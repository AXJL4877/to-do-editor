use super::*;
use todo_core::Project;
use todo_interaction::Intent;

#[test]
fn selecting_and_clearing_a_clip_keeps_the_program_canvas_unchanged() {
    let ctx = egui::Context::default();
    let mut vm = crate::test_view(
        todo_core::create_project(None, None, None, None, None),
        "test",
    );
    let mut render_view = |properties| {
        vm.properties = properties;
        let mut hole = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440.0, 900.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    hole = draw_editor(
                        ui,
                        &vm,
                        &crate::PeaksCache::default(),
                        &mut crate::ThumbCache::default(),
                    )
                    .1;
                });
            },
        );
        let h = hole.unwrap();
        (h.x, h.y, h.w, h.h)
    };
    let empty = render_view(None);
    let selected = render_view(Some(todo_interaction::ClipProperties {
        clip_id: "c".into(),
        name: "2026-09-30_13-16-50.mp4".into(),
        start: 5.0,
        duration: 4.0,
        source: todo_interaction::SourceProperties::Media {
            offset: 0.0,
            rate: 1.0,
        },
        visual: true,
        locked: false,
        transform: Default::default(),
        motion: None,
    }));
    assert_eq!(
        empty, selected,
        "selection must only update the independent properties pane"
    );
    assert_eq!(empty, render_view(None));
}

fn render(ctx: &egui::Context, events: Vec<egui::Event>) -> MonitorHole {
    render_at(ctx, egui::vec2(1200.0, 800.0), events, false)
        .1
        .expect("native monitor slot")
}

fn render_at(
    ctx: &egui::Context,
    size: egui::Vec2,
    events: Vec<egui::Event>,
    minimized: bool,
) -> (Vec<Intent>, Option<MonitorHole>) {
    let vm = crate::test_view(
        Project {
            version: 1,
            fps: 30.0,
            playhead: 0.0,
            width: 1920,
            height: 1080,
            duration: None,
            media_scale: Default::default(),
            media: Vec::new(),
            tracks: Vec::new(),
            text_styles: Vec::new(),
            motions: Vec::new(),
            folders: Vec::new(),
        },
        "test",
    );
    let mut result = (Vec::new(), None);
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        events,
        ..Default::default()
    };
    input
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .minimized = Some(minimized);
    let _ = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            result = draw_editor(
                ui,
                &vm,
                &crate::PeaksCache::default(),
                &mut crate::ThumbCache::default(),
            );
        });
    });
    result
}

fn drag(ctx: &egui::Context, start: egui::Pos2, delta: egui::Vec2) -> MonitorHole {
    drag_at(ctx, egui::vec2(1200.0, 800.0), start, delta)
}

fn drag_at(
    ctx: &egui::Context,
    size: egui::Vec2,
    start: egui::Pos2,
    delta: egui::Vec2,
) -> MonitorHole {
    render_at(ctx, size, vec![egui::Event::PointerMoved(start)], false);
    render_at(
        ctx,
        size,
        vec![egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }],
        false,
    );
    render_at(
        ctx,
        size,
        vec![egui::Event::PointerMoved(start + delta)],
        false,
    );
    render_at(
        ctx,
        size,
        vec![egui::Event::PointerButton {
            pos: start + delta,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }],
        false,
    );
    // egui hit testing reads the previous pass; allow its widget rectangles to settle.
    render_at(ctx, size, Vec::new(), false)
        .1
        .expect("native monitor slot")
}

#[test]
fn dragging_library_divider_resizes_the_native_monitor_and_keeps_the_size() {
    let ctx = egui::Context::default();
    let before = render(&ctx, Vec::new());
    let divider = ctx
        .read_response(Id::new("todo_ui_library_width"))
        .expect("media library needs a draggable divider")
        .rect;
    let after = drag(&ctx, divider.center(), egui::vec2(120.0, 0.0));
    assert!((after.x - before.x - 120.0).abs() < 1.0);
    assert!((before.w - after.w - 120.0).abs() < 1.0);
    assert_eq!(after.h, before.h);
    let next = render(&ctx, Vec::new());
    assert_eq!(after.x, next.x);
    assert_eq!(after.w, next.w);
}

#[test]
fn inspector_divider_resizes_independently_and_remembers_its_width() {
    let ctx = egui::Context::default();
    let before = render(&ctx, vec![]);
    let divider = ctx.read_response(Id::new(INSPECTOR_ID)).unwrap().rect;
    let after = drag(&ctx, divider.center(), egui::vec2(-80.0, 0.0));
    assert!((before.w - after.w - 80.0).abs() < 1.0);
    assert_eq!(before.x, after.x);
    assert_eq!(before.h, after.h);
    let next = render(&ctx, vec![]);
    assert_eq!(after.w, next.w);
}

#[test]
fn play_control_below_program_canvas_emits_toggle_play() {
    let ctx = egui::Context::default();
    let mut vm = crate::test_view(
        todo_core::create_project(None, None, None, None, None),
        "test",
    );
    vm.can_play = true;
    let render_frame = |events| {
        let mut result = (Vec::new(), None);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    result = draw_editor(
                        ui,
                        &vm,
                        &crate::PeaksCache::default(),
                        &mut crate::ThumbCache::default(),
                    );
                });
            },
        );
        (result, output)
    };
    let ((_, hole), output) = render_frame(vec![]);
    let hole = hole.unwrap();
    let play = output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Rect(r)
                if r.fill == BRAND
                    && (r.rect.width() - 40.0).abs() < 0.1
                    && (r.rect.height() - 32.0).abs() < 0.1 =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .expect("program transport button");
    assert!(play.top() > hole.y + hole.h);
    let pos = play.center();
    let pointer = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    render_frame(vec![egui::Event::PointerMoved(pos), pointer(true)]);
    let ((intents, _), _) = render_frame(vec![pointer(false)]);
    assert_eq!(
        intents
            .iter()
            .filter(|i| matches!(i, Intent::TogglePlay))
            .count(),
        1
    );
}

#[test]
fn dragging_timeline_divider_resizes_the_native_monitor_and_obeys_limits() {
    let ctx = egui::Context::default();
    let before = render(&ctx, Vec::new());
    let divider = ctx
        .read_response(Id::new("todo_ui_monitor_height"))
        .expect("timeline needs a draggable divider")
        .rect;
    let after = drag(&ctx, divider.center(), egui::vec2(0.0, -80.0));
    assert!((before.h - after.h - 80.0).abs() < 1.0);
    assert_eq!(before.w, after.w);
    let divider = ctx
        .read_response(Id::new("todo_ui_monitor_height"))
        .unwrap()
        .rect;
    let smallest = drag(&ctx, divider.center(), egui::vec2(0.0, -2000.0));
    assert!(smallest.h >= 80.0, "preview must keep a usable canvas");
    assert!(
        smallest.h < after.h - 80.0,
        "second drag must reach the minimum"
    );
    assert!(smallest.y + smallest.h < 800.0 - 160.0);
}

#[test]
fn pane_sizes_survive_a_temporary_window_shrink() {
    let ctx = egui::Context::default();
    let vm = crate::test_view(
        todo_core::create_project(None, None, None, None, None),
        "test",
    );
    let hole_at = |size: egui::Vec2| {
        let mut hole = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    hole = draw_editor(
                        ui,
                        &vm,
                        &crate::PeaksCache::default(),
                        &mut crate::ThumbCache::default(),
                    )
                    .1;
                });
            },
        );
        hole.expect("native monitor slot")
    };
    let large = egui::vec2(1200.0, 800.0);
    let before = hole_at(large);
    let divider = ctx
        .read_response(Id::new(LIBRARY_ID))
        .expect("library divider")
        .rect;
    // Widen the library, then shrink the window (minimize-style), then restore.
    let _ = drag(&ctx, divider.center(), egui::vec2(100.0, 0.0));
    let widened = hole_at(large);
    assert!(widened.x > before.x + 80.0, "{widened:?} vs {before:?}");
    let _ = hole_at(egui::vec2(640.0, 400.0));
    let restored = hole_at(large);
    assert!(
        (restored.x - widened.x).abs() < 2.0,
        "preferred library width must return after the window grows again: {restored:?} vs {widened:?}"
    );
}

#[test]
fn all_resized_panes_survive_shrinking_and_restarting() {
    let ctx = egui::Context::default();
    render(&ctx, vec![]);
    for (id, delta) in [
        (LIBRARY_ID, egui::vec2(100.0, 0.0)),
        (INSPECTOR_ID, egui::vec2(-70.0, 0.0)),
        (HEIGHT_ID, egui::vec2(0.0, -110.0)),
    ] {
        let divider = ctx.read_response(Id::new(id)).unwrap().rect.center();
        drag(&ctx, divider, delta);
    }
    let preferred = render(&ctx, vec![]);
    for (size, minimized) in [(egui::vec2(640.0, 400.0), false), (egui::Vec2::ZERO, true)] {
        render_at(&ctx, size, vec![], minimized);
    }
    // Save while the panes are clamped, as eframe does on exit/autosave.
    let saved = ctx.memory(|memory| ron::to_string(memory).unwrap());
    let restarted = egui::Context::default();
    restarted.memory_mut(|memory| *memory = ron::from_str(&saved).unwrap());
    let restored = render(&restarted, vec![]);
    let bounds = |hole: MonitorHole| (hole.x, hole.y, hole.w, hole.h);
    assert_eq!(
        bounds(restored),
        bounds(preferred),
        "saved preferences must contain the chosen sizes"
    );
    assert_eq!(bounds(render(&ctx, vec![])), bounds(preferred));
}

#[test]
fn minimized_first_frame_does_not_seed_collapsed_pane_preferences() {
    let fresh = egui::Context::default();
    let expected = render(&fresh, vec![]);
    let ctx = egui::Context::default();
    render_at(&ctx, egui::Vec2::ZERO, vec![], true);
    let actual = render(&ctx, vec![]);
    assert_eq!(
        (actual.x, actual.y, actual.w, actual.h),
        (expected.x, expected.y, expected.w, expected.h)
    );
}

#[test]
fn resizing_a_clamped_pane_starts_from_its_visible_divider() {
    let small = egui::vec2(640.0, 500.0);
    for (id, delta) in [
        (LIBRARY_ID, egui::vec2(-24.0, 0.0)),
        (INSPECTOR_ID, egui::vec2(24.0, 0.0)),
        (HEIGHT_ID, egui::vec2(0.0, -24.0)),
    ] {
        let ctx = egui::Context::default();
        render(&ctx, vec![]);
        render_at(&ctx, small, vec![], false);
        render_at(&ctx, small, vec![], false);
        let before = ctx.read_response(Id::new(id)).unwrap().rect.center();
        drag_at(&ctx, small, before, delta);
        let after = ctx.read_response(Id::new(id)).unwrap().rect.center();
        assert!(
            (after - before - delta).length() < 1.0,
            "drag should follow the pointer after a window shrink ({id}): {before:?} -> {after:?}"
        );
    }
}

#[test]
fn settings_modal_blocks_background_shortcuts_even_when_escape_dismisses_it() {
    let ctx = egui::Context::default();
    render(&ctx, vec![]);
    ctx.data_mut(|data| data.insert_temp(Id::new("todo_ui_project_settings_open"), true));
    render(&ctx, vec![]);
    for key in [egui::Key::Space, egui::Key::Delete, egui::Key::Escape] {
        let (intents, _) = render_at(
            &ctx,
            egui::vec2(1200.0, 800.0),
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
            false,
        );
        assert!(
            intents.is_empty(),
            "settings key leaked into the editor: {intents:?}"
        );
    }
    assert!(
        !project_settings::is_open(&ctx),
        "Escape dismisses settings"
    );
}

#[test]
fn double_clicking_library_divider_restores_the_default_workspace_width() {
    let ctx = egui::Context::default();
    let before = render(&ctx, Vec::new());
    let divider = ctx
        .read_response(Id::new("todo_ui_library_width"))
        .unwrap()
        .rect;
    let resized = drag(&ctx, divider.center(), egui::vec2(130.0, 0.0));
    assert!(resized.x > before.x + 100.0);
    let pointer = ctx
        .read_response(Id::new("todo_ui_library_width"))
        .unwrap()
        .rect
        .center();
    render(&ctx, vec![egui::Event::PointerMoved(pointer)]);
    for _ in 0..2 {
        render(
            &ctx,
            vec![egui::Event::PointerButton {
                pos: pointer,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        render(
            &ctx,
            vec![egui::Event::PointerButton {
                pos: pointer,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
    }
    let restored = render(&ctx, Vec::new());
    assert!((restored.x - before.x).abs() < 1.0);
    assert!((restored.w - before.w).abs() < 1.0);
}

#[test]
fn library_leaves_a_native_monitor_beside_it_at_supported_window_sizes() {
    let vm = crate::test_view(
        Project {
            version: 1,
            fps: 30.0,
            playhead: 0.0,
            width: 1920,
            height: 1080,
            duration: None,
            media_scale: Default::default(),
            media: Vec::new(),
            tracks: Vec::new(),
            text_styles: Vec::new(),
            motions: Vec::new(),
            folders: Vec::new(),
        },
        "test",
    );
    for size in [egui::vec2(960.0, 640.0), egui::vec2(1440.0, 900.0)] {
        let ctx = egui::Context::default();
        ctx.data_mut(|data| data.insert_temp(Id::new(LIBRARY_ID), 10_000.0_f32));
        let mut hole = None;
        let mut intents = Vec::new();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    (intents, hole) = draw_editor(
                        ui,
                        &vm,
                        &crate::PeaksCache::default(),
                        &mut crate::ThumbCache::default(),
                    );
                });
            },
        );
        let hole = hole.expect("native monitor slot");
        assert!(
            hole.x >= 250.0,
            "media library must occupy the left of the monitor: {hole:?}"
        );
        assert!(hole.w >= 300.0 && hole.h >= MONITOR_MIN);
        assert!(hole.y + hole.h < size.y - 160.0);
        assert!(intents.iter().any(|intent| matches!(intent,
            Intent::Viewport { width, .. } if *width > f64::from(size.x - 40.0))));
        assert!(intents.iter().any(|intent| matches!(intent,
            Intent::PreviewViewport { width, height } if (*width - f64::from(hole.w)).abs() < 0.01 && (*height - f64::from(hole.h)).abs() < 0.01)));
    }
}

#[test]
fn selected_clip_inspector_stays_to_the_right_of_the_native_canvas() {
    let mut vm = crate::test_view(
        todo_core::create_project(None, None, None, None, None),
        "test",
    );
    vm.properties = Some(todo_interaction::ClipProperties {
        clip_id: "clip".into(),
        name: "Selected picture".into(),
        start: 0.0,
        duration: 4.0,
        source: todo_interaction::SourceProperties::Media {
            offset: 0.0,
            rate: 1.0,
        },
        visual: true,
        locked: false,
        transform: Default::default(),
        motion: None,
    });
    for size in [egui::vec2(960.0, 640.0), egui::vec2(1440.0, 900.0)] {
        let ctx = egui::Context::default();
        let mut hole = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    hole = draw_editor(
                        ui,
                        &vm,
                        &crate::PeaksCache::default(),
                        &mut crate::ThumbCache::default(),
                    )
                    .1;
                });
            },
        );
        let hole = hole.expect("selected clip retains native video canvas");
        assert!(hole.w > 290.0 && hole.h > 70.0, "{hole:?}");
        let name = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Selected picture" => {
                    Some(text.pos)
                }
                _ => None,
            })
            .expect("selected clip name in independent inspector");
        assert!(
            name.x > hole.x + hole.w,
            "properties must stay beside the native presenter: {name:?}, {hole:?}"
        );
        assert!(name.x < size.x - 12.0, "properties remain in the workspace");
    }
}

#[test]
fn small_window_inspector_scroll_reaches_transform_controls_without_resizing_preview() {
    let ctx = egui::Context::default();
    let mut vm = crate::test_view(
        todo_core::create_project(None, None, None, None, None),
        "test",
    );
    vm.properties = Some(todo_interaction::ClipProperties {
        clip_id: "c".into(),
        name: "Clip".into(),
        start: 0.0,
        duration: 4.0,
        source: todo_interaction::SourceProperties::Media {
            offset: 0.0,
            rate: 1.0,
        },
        visual: true,
        locked: false,
        transform: Default::default(),
        motion: None,
    });
    let render = |events| {
        let mut hole = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960.0, 640.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    hole = draw_editor(
                        ui,
                        &vm,
                        &crate::PeaksCache::default(),
                        &mut crate::ThumbCache::default(),
                    )
                    .1;
                });
            },
        );
        (hole.unwrap(), output)
    };
    let (before, _) = render(vec![]);
    let (_, mut output) = render(vec![
        egui::Event::PointerMoved(egui::pos2(870.0, 240.0)),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -600.0),
            modifiers: Default::default(),
        },
    ]);
    for _ in 0..12 {
        output = render(vec![]).1;
    }
    for label in ["不透明度", "重置画面"] {
        assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()==label && s.clip_rect.contains(t.pos))),"bottom control must be reachable: {label}");
    }
    let (after, _) = render(vec![]);
    assert_eq!(
        (before.x, before.y, before.w, before.h),
        (after.x, after.y, after.w, after.h)
    );
}
