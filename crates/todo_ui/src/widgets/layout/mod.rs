//! Resizable workspace panes. Layout preferences belong to the UI, not the project.

use egui::{CursorIcon, Id, Rect, Sense, Ui, UiBuilder};
use todo_interaction::{EditorViewModel, Intent};

use super::{inspector, library, monitor, shortcuts, timeline, toolbar, MonitorHole};
use crate::format::band_heights;
use crate::peaks::PeaksCache;
use crate::theme::{BG, BORDER, BRAND, DIM, PANEL, TOOLBAR_H};
use crate::thumbs::ThumbCache;

const LIBRARY_ID: &str = "todo_ui_library_width";
const HEIGHT_ID: &str = "todo_ui_monitor_height";
const INSPECTOR_ID: &str = "todo_ui_inspector_width";
const SPLIT: f32 = 7.0;
pub(super) const MONITOR_MIN: f32 = 96.0;

pub fn draw_editor(
    ui: &mut Ui,
    vm: &EditorViewModel,
    peaks: &PeaksCache,
    thumbs: &mut ThumbCache,
) -> (Vec<Intent>, Option<MonitorHole>) {
    let mut intents = Vec::new();
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    ui.painter().rect_filled(full, 0.0, BG);

    let toolbar_h = TOOLBAR_H.min(full.height());
    let upper_top = full.top() + toolbar_h;
    let available_h = (full.height() - toolbar_h - SPLIT).max(0.0);
    // Keep one library row and the search/actions visible at the smallest split.
    let min_upper = (MONITOR_MIN + 80.0).min(available_h * 0.55);
    let max_upper = (available_h - 160.0_f32.min(available_h * 0.45)).max(min_upper);
    let default_h = band_heights(full.height()).1.clamp(min_upper, max_upper);
    let mut upper_h = remembered(ui, HEIGHT_ID, default_h).clamp(min_upper, max_upper);

    let available_w = (full.width() - SPLIT * 2.0).max(0.0);
    let min_inspector = 220.0_f32.min(available_w * 0.25);
    let max_inspector = 400.0_f32.min(available_w * 0.38).max(min_inspector);
    let default_inspector = 280.0_f32.clamp(min_inspector, max_inspector);
    let mut inspector_w =
        remembered(ui, INSPECTOR_ID, default_inspector).clamp(min_inspector, max_inspector);
    let inspector_split = Rect::from_min_size(
        egui::pos2(full.right() - inspector_w - SPLIT, upper_top),
        egui::vec2(SPLIT, upper_h),
    );
    let inspector_response = ui.interact(
        inspector_split,
        Id::new(INSPECTOR_ID),
        Sense::click_and_drag(),
    );
    if inspector_response.double_clicked() {
        inspector_w = default_inspector;
    } else if inspector_response.dragged() {
        inspector_w =
            (inspector_w - inspector_response.drag_delta().x).clamp(min_inspector, max_inspector);
    }
    let min_library = 180.0_f32.min(available_w * 0.22);
    let max_library =
        (available_w - inspector_w - 320.0_f32.min(available_w * 0.35)).max(min_library);
    let default_w = (full.width() * 0.25)
        .clamp(min_library, max_library)
        .min(340.0);
    let mut library_w = remembered(ui, LIBRARY_ID, default_w).clamp(min_library, max_library);

    let horizontal = Rect::from_min_size(
        egui::pos2(full.left(), upper_top + upper_h),
        egui::vec2(full.width(), SPLIT),
    );
    let height_response = ui.interact(horizontal, Id::new(HEIGHT_ID), Sense::click_and_drag());
    if height_response.double_clicked() {
        upper_h = default_h;
    } else if height_response.dragged() {
        upper_h = (upper_h + height_response.drag_delta().y).clamp(min_upper, max_upper);
    }
    let vertical = Rect::from_min_size(
        egui::pos2(full.left() + library_w, upper_top),
        egui::vec2(SPLIT, upper_h),
    );
    let width_response = ui.interact(vertical, Id::new(LIBRARY_ID), Sense::click_and_drag());
    if width_response.double_clicked() {
        library_w = default_w;
    } else if width_response.dragged() {
        library_w = (library_w + width_response.drag_delta().x).clamp(min_library, max_library);
    }
    ui.ctx().data_mut(|data| {
        data.insert_temp(Id::new(HEIGHT_ID), upper_h);
        data.insert_temp(Id::new(LIBRARY_ID), library_w);
        data.insert_temp(Id::new(INSPECTOR_ID), inspector_w);
    });

    let toolbar_rect = Rect::from_min_size(full.min, egui::vec2(full.width(), toolbar_h));
    intents.extend(toolbar::draw(
        &mut pane(ui, toolbar_rect, "editor_toolbar"),
        vm,
    ));
    let library_rect = Rect::from_min_size(
        egui::pos2(full.left(), upper_top),
        egui::vec2(library_w, upper_h),
    );
    intents.extend(library::draw(
        &mut pane(ui, library_rect, "editor_library"),
        vm,
        thumbs,
    ));
    let monitor_rect = Rect::from_min_max(
        egui::pos2(library_rect.right() + SPLIT, upper_top),
        egui::pos2(full.right() - inspector_w - SPLIT, upper_top + upper_h),
    );
    let (monitor_intents, hole) = monitor::draw(&mut pane(ui, monitor_rect, "editor_monitor"), vm);
    intents.extend(monitor_intents);
    let inspector_rect = Rect::from_min_max(
        egui::pos2(full.right() - inspector_w, upper_top),
        egui::pos2(full.right(), upper_top + upper_h),
    );
    intents.extend(inspector::draw(
        &mut pane(ui, inspector_rect, "editor_inspector"),
        vm.properties.as_ref(),
    ));
    if let Some(hole) = hole {
        intents.push(Intent::PreviewViewport {
            width: f64::from(hole.w),
            height: f64::from(hole.h),
        });
    }
    let timeline_rect = Rect::from_min_max(
        egui::pos2(full.left(), upper_top + upper_h + SPLIT),
        full.max,
    );
    intents.extend(timeline::draw(
        &mut pane(ui, timeline_rect, "editor_timeline"),
        &vm.timeline,
        peaks,
    ));
    paint_splitter(
        ui,
        Rect::from_min_size(
            egui::pos2(full.left(), upper_top + upper_h),
            egui::vec2(full.width(), SPLIT),
        ),
        height_response,
        false,
    );
    paint_splitter(
        ui,
        Rect::from_min_size(
            egui::pos2(full.left() + library_w, upper_top),
            egui::vec2(SPLIT, upper_h),
        ),
        width_response,
        true,
    );
    intents.extend(shortcuts::collect(ui.ctx(), vm));
    paint_splitter(
        ui,
        Rect::from_min_size(
            egui::pos2(inspector_rect.left() - SPLIT, upper_top),
            egui::vec2(SPLIT, upper_h),
        ),
        inspector_response,
        true,
    );
    (intents, hole)
}

fn remembered(ui: &Ui, key: &str, fallback: f32) -> f32 {
    ui.ctx()
        .data(|data| data.get_temp::<f32>(Id::new(key)))
        .filter(|value| value.is_finite())
        .unwrap_or(fallback)
}

fn pane(ui: &mut Ui, rect: Rect, key: &str) -> Ui {
    let mut child = ui.new_child(UiBuilder::new().id_salt(key).max_rect(rect));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    child
}

fn paint_splitter(ui: &Ui, rect: Rect, response: egui::Response, vertical: bool) {
    let active = response.hovered() || response.dragged();
    if active {
        ui.ctx().set_cursor_icon(if vertical {
            CursorIcon::ResizeHorizontal
        } else {
            CursorIcon::ResizeVertical
        });
    }
    ui.painter()
        .rect_filled(rect, 0.0, if active { PANEL } else { BG });
    let line = if vertical {
        [rect.center_top(), rect.center_bottom()]
    } else {
        [rect.left_center(), rect.right_center()]
    };
    ui.painter().line_segment(
        line,
        egui::Stroke::new(1.0_f32, if active { BRAND } else { BORDER }),
    );
    let grip = if vertical {
        egui::vec2(0.0, 12.0)
    } else {
        egui::vec2(12.0, 0.0)
    };
    ui.painter().line_segment(
        [rect.center() - grip, rect.center() + grip],
        egui::Stroke::new(3.0_f32, if active { BRAND } else { DIM }),
    );
    response.on_hover_text("拖动调整大小 · 双击恢复默认");
}

#[cfg(test)]
mod tests {
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
        }));
        assert_eq!(
            empty, selected,
            "selection must only update the independent properties pane"
        );
        assert_eq!(empty, render_view(None));
    }

    fn render(ctx: &egui::Context, events: Vec<egui::Event>) -> MonitorHole {
        let vm = crate::test_view(
            Project {
                version: 1,
                fps: 30.0,
                playhead: 0.0,
                width: 1920,
                height: 1080,
                duration: None,
                media: Vec::new(),
                tracks: Vec::new(),
                text_styles: Vec::new(),
            },
            "test",
        );
        let mut hole = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
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
        hole.expect("native monitor slot")
    }

    fn drag(ctx: &egui::Context, start: egui::Pos2, delta: egui::Vec2) -> MonitorHole {
        render(ctx, vec![egui::Event::PointerMoved(start)]);
        render(
            ctx,
            vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        render(ctx, vec![egui::Event::PointerMoved(start + delta)]);
        render(
            ctx,
            vec![egui::Event::PointerButton {
                pos: start + delta,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        // egui hit testing reads the previous pass; allow its widget rectangles to settle.
        render(ctx, Vec::new())
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
                media: Vec::new(),
                tracks: Vec::new(),
                text_styles: Vec::new(),
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
}
