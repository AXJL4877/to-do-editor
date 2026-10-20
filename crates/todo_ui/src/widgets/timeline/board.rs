use egui::{pos2, Color32, Id, Sense, Ui};
use todo_interaction::{EditorFocus, Intent, TimelineViewModel, LABEL_W};

use crate::peaks::PeaksCache;
use crate::theme::{AUDIO_LANE, BORDER, BRAND, COLORS, ROW_H, RULER_H, TEXT_LANE, VIDEO_LANE};
use crate::widgets::icons::{self, Icon};
use crate::widgets::library::MediaDrag;

const SCROLL_ID: &str = "todo_timeline_board";
const SCRUB_ID: &str = "todo_timeline_scrubbing";

pub fn draw(ui: &mut Ui, vm: &TimelineViewModel, peaks: &PeaksCache) -> Vec<Intent> {
    let mut intents = Vec::new();
    let scroll_id = ui.make_persistent_id(Id::new(SCROLL_ID));
    sync_zoom_scroll(ui, vm, scroll_id);
    let scroll_left = egui::scroll_area::State::load(ui.ctx(), scroll_id)
        .map(|state| f64::from(state.offset.x))
        .unwrap_or(vm.scroll_left);
    let viewport = ui.available_rect_before_wrap();
    let media_drag_key = ui.make_persistent_id("media_drag_active");
    let was_media_drag = ui
        .data(|data| data.get_temp::<bool>(media_drag_key))
        .unwrap_or(false);
    let has_media_drag = egui::DragAndDrop::payload::<MediaDrag>(ui.ctx()).is_some();

    // Capture Alt/Ctrl+wheel before ScrollArea consumes it, so zoom keeps the
    // time under the cursor instead of also panning the board.
    let zoom_mods = ui.input(|input| input.modifiers.alt || input.modifiers.ctrl);
    let zoom_pointer = ui
        .input(|input| input.pointer.hover_pos())
        .filter(|&pos| viewport.contains(pos));
    let mut zoom_delta_y = 0.0_f32;
    if zoom_mods && zoom_pointer.is_some() {
        zoom_delta_y = ui.input(|input| input.raw_scroll_delta.y);
        ui.ctx().input_mut(|input| {
            input.raw_scroll_delta = egui::Vec2::ZERO;
            input.smooth_scroll_delta = egui::Vec2::ZERO;
        });
    }

    let output = egui::ScrollArea::both()
        // id_salt hashes its argument; use the same salt as the State lookup.
        .id_salt(SCROLL_ID)
        .auto_shrink([false, false])
        .drag_to_scroll(false)
        .enable_scrolling(!(zoom_mods && zoom_pointer.is_some()))
        .show(ui, |ui| {
            paint(
                ui,
                vm,
                peaks,
                viewport.left(),
                scroll_left,
                viewport.height(),
            )
        });
    intents.extend(output.inner);
    let scroll_left = f64::from(output.state.offset.x);
    intents.push(Intent::Viewport {
        scroll_left,
        width: f64::from(viewport.width()),
    });

    let scrub_id = Id::new(SCRUB_ID);
    let mut scrubbing = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(scrub_id))
        .unwrap_or(false);
    // BoardPointerDown always scrubs (ruler, including razor). LanePointerDown
    // only scrubs in select mode; in razor it cuts and must not start scrubbing.
    // MarqueePointerDown / active marquee keep the pointer stream without seeking.
    if intents
        .iter()
        .any(|intent| matches!(intent, Intent::BoardPointerDown { .. }))
        || intents
            .iter()
            .any(|intent| matches!(intent, Intent::MarqueePointerDown { .. }))
        || (!vm.razor_mode
            && intents
                .iter()
                .any(|intent| matches!(intent, Intent::LanePointerDown { .. })))
    {
        scrubbing = true;
    }
    if vm.drag.is_some() {
        scrubbing = true;
    }

    let primary_down = ui.input(|input| input.pointer.primary_down());
    let release_pos = ui.input(|input| {
        input.events.iter().find_map(|event| match event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                ..
            } => Some(*pos),
            _ => None,
        })
    });
    let scroll_top = f64::from(output.state.offset.y);
    if scrubbing || vm.drag.is_some() {
        // Later movement or PointerGone in the same frame must not replace the
        // primary release position. Commit it before ending the drag.
        let pointer = release_pos.or_else(|| ui.input(|input| input.pointer.latest_pos()));
        if let Some(pointer) = pointer {
            let content_y = f64::from(pointer.y - viewport.top()) + scroll_top;
            intents.push(Intent::PointerMove {
                view_x: f64::from(pointer.x - viewport.left()),
                scroll_left,
                track_index: track_index_at(content_y, vm.tracks.len()),
            });
        }
        if release_pos.is_some() || !primary_down {
            intents.push(Intent::PointerUp);
            scrubbing = false;
        }
    }
    ui.ctx()
        .data_mut(|d| d.insert_temp(scrub_id, scrubbing && primary_down));

    // raw_scroll_delta is one notch per event; smooth_scroll_delta spans many
    // frames and made wheel zoom feel far too sensitive.
    if zoom_delta_y.abs() >= 1.0 {
        if let Some(pointer) = zoom_pointer {
            // egui scroll down is often negative; wheel_zoom treats positive as zoom-out.
            intents.push(Intent::Wheel {
                delta_y: -f64::from(zoom_delta_y.signum()),
                view_x: f64::from(pointer.x - viewport.left()),
                scroll_left,
                alt: true,
            });
        }
    }

    let drag_ended = ui
        .input(|input| input.pointer.primary_released() || input.key_pressed(egui::Key::Escape))
        || !egui::DragAndDrop::has_payload_of_type::<MediaDrag>(ui.ctx());
    if (was_media_drag || has_media_drag) && drag_ended {
        intents.push(Intent::MediaDragEnd);
    }
    ui.data_mut(|data| data.insert_temp(media_drag_key, has_media_drag && !drag_ended));
    intents
}

fn paint(
    ui: &mut Ui,
    vm: &TimelineViewModel,
    peaks: &PeaksCache,
    viewport_left: f32,
    scroll_left: f64,
    viewport_h: f32,
) -> Vec<Intent> {
    let mut intents = Vec::new();
    let label_w = LABEL_W as f32;
    let width = vm.board_width as f32;
    let row_h = ROW_H;
    let ruler_h = RULER_H;
    let height = (ruler_h + vm.tracks.len() as f32 * row_h).max(viewport_h);
    let (response, painter) = ui.allocate_painter(
        egui::vec2(width.max(label_w + 1.0), height),
        Sense::click_and_drag(),
    );
    let origin = response.rect.min;
    painter.rect_filled(response.rect, 0.0, COLORS.bg);
    painter.rect_stroke(
        response.rect,
        0.0,
        egui::Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    let playhead_screen_x = origin.x + vm.playhead_x as f32;
    let pointer_view_x = |pointer_x: f32| f64::from(pointer_x - viewport_left);
    // Use the actual release event: later motion/PointerGone can change latest_pos.
    let release_pos = ui.input(|input| {
        input.events.iter().find_map(|event| match event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                ..
            } => Some(*pos),
            _ => None,
        })
    });
    let drop_pointer = release_pos.or_else(|| ui.input(|input| input.pointer.latest_pos()));

    for tick in &vm.ticks {
        let x = origin.x + label_w + tick.x as f32;
        painter.line_segment(
            [
                pos2(x, origin.y + ruler_h - 7.0),
                pos2(x, origin.y + ruler_h),
            ],
            egui::Stroke::new(1.0_f32, BORDER),
        );
        painter.text(
            pos2(x + 3.0, origin.y + 2.0),
            egui::Align2::LEFT_TOP,
            &tick.label,
            egui::FontId::proportional(10.0),
            crate::theme::MUTED,
        );
    }

    let mut child_hit = false;
    for (index, track) in vm.tracks.iter().enumerate() {
        let y = origin.y + ruler_h + index as f32 * row_h;
        let label = egui::Rect::from_min_size(pos2(origin.x, y), egui::vec2(label_w, row_h));
        child_hit |= super::header::draw(
            ui,
            track,
            label,
            vm.active_track_id.as_deref() == Some(track.id.as_str()),
            &mut intents,
        );
        let lane = egui::Rect::from_min_size(
            pos2(origin.x + label_w, y),
            egui::vec2(vm.lane_width as f32, row_h),
        );
        let lane_color = match track.kind {
            todo_interaction::TrackKind::Text => TEXT_LANE,
            todo_interaction::TrackKind::Video => VIDEO_LANE,
            todo_interaction::TrackKind::Audio | todo_interaction::TrackKind::Sfx => AUDIO_LANE,
        };
        painter.rect_filled(lane, 0.0, lane_color);
        let workspace_x = origin.x + label_w + (vm.workspace_end * vm.px_per_sec) as f32;
        if workspace_x < lane.right() {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    pos2(workspace_x.max(lane.left()), lane.top()),
                    lane.right_bottom(),
                ),
                0.0,
                Color32::from_black_alpha(45),
            );
        }
        painter.line_segment(
            [lane.left_bottom(), lane.right_bottom()],
            egui::Stroke::new(1.0_f32, BORDER),
        );
        for tick in &vm.ticks {
            let x = origin.x + label_w + tick.x as f32;
            painter.line_segment(
                [pos2(x, y), pos2(x, y + row_h)],
                egui::Stroke::new(1.0_f32, Color32::from_white_alpha(9)),
            );
        }
        let lane_resp = ui.interact(
            lane,
            ui.id().with(&track.id).with("lane"),
            Sense::click_and_drag(),
        );
        if lane_resp.clicked() || lane_resp.drag_started() {
            if let Some(pointer) = lane_resp.interact_pointer_pos() {
                let on_clip = track.clips.iter().any(|clip| {
                    let rect = egui::Rect::from_min_size(
                        pos2(origin.x + label_w + clip.left as f32, y + 6.0),
                        egui::vec2(clip.width.max(2.0) as f32, row_h - 12.0),
                    );
                    rect.contains(pointer)
                });
                if !on_clip {
                    child_hit = true;
                    intents.push(Intent::Focus(EditorFocus::Timeline));
                    let mods = ui.input(|input| input.modifiers);
                    if !vm.razor_mode && (mods.ctrl || mods.command) {
                        intents.push(Intent::MarqueePointerDown {
                            view_x: pointer_view_x(pointer.x),
                            track_index: index,
                            scroll_left,
                            additive: mods.shift,
                        });
                    } else {
                        intents.push(Intent::LanePointerDown {
                            track_id: track.id.clone(),
                            view_x: pointer_view_x(pointer.x),
                            scroll_left,
                        });
                    }
                }
            }
        }
        for clip in &track.clips {
            let rect = egui::Rect::from_min_size(
                pos2(origin.x + label_w + clip.left as f32, y + 6.0),
                egui::vec2(clip.width.max(2.0) as f32, row_h - 12.0),
            );
            child_hit |= super::clip::draw(
                ui,
                vm,
                track,
                clip,
                super::clip::ClipGeometry {
                    rect,
                    viewport_left,
                    scroll_left,
                },
                peaks,
                &mut intents,
            );
        }

        // Extend only the left edge of each visible lane to the window boundary.
        // The row's clipped vertical bounds keep other panels/ruler out of this target.
        let visible_lane = lane.intersect(ui.clip_rect());
        let drop_target = drop_pointer.filter(|pointer| {
            ui.input(|input| input.screen_rect().contains(*pointer))
                && pointer.y >= visible_lane.top()
                && pointer.y < visible_lane.bottom()
                && pointer.x <= visible_lane.right()
                && visible_lane.is_positive()
        });
        if let Some(payload) = egui::DragAndDrop::payload::<MediaDrag>(ui.ctx()) {
            if let Some(pointer) = drop_target {
                let time = f64::from(pointer.x - origin.x - label_w) / vm.px_per_sec;
                intents.push(Intent::MediaDragHover {
                    media_id: payload.media_id.clone(),
                    track_id: track.id.clone(),
                    time,
                });
                if let Some(preview) = vm
                    .drop_preview
                    .as_ref()
                    .filter(|preview| preview.media_id == payload.media_id)
                {
                    ui.ctx().set_cursor_icon(if preview.allowed {
                        egui::CursorIcon::Copy
                    } else {
                        egui::CursorIcon::NotAllowed
                    });
                }
                if release_pos.is_some() {
                    if let Some(payload) = egui::DragAndDrop::take_payload::<MediaDrag>(ui.ctx()) {
                        intents.push(Intent::Focus(EditorFocus::Timeline));
                        intents.push(Intent::PlaceMedia {
                            media_id: payload.media_id.clone(),
                            track_id: Some(track.id.clone()),
                            time: Some(time),
                        });
                    }
                }
            }
        }
        if let Some(preview) = vm
            .drop_preview
            .as_ref()
            .filter(|preview| preview.track_id == track.id)
        {
            let ghost = egui::Rect::from_min_size(
                pos2(
                    origin.x + label_w + (preview.start * vm.px_per_sec) as f32,
                    y + 5.0,
                ),
                egui::vec2(
                    (preview.duration * vm.px_per_sec).max(6.0) as f32,
                    row_h - 10.0,
                ),
            );
            painter.rect_stroke(
                ghost,
                0.0,
                egui::Stroke::new(
                    2.0_f32,
                    if preview.allowed {
                        Color32::from_rgb(145, 214, 172)
                    } else {
                        Color32::from_rgb(210, 88, 88)
                    },
                ),
                egui::StrokeKind::Inside,
            );
        }
    }

    if vm.axis > vm.workspace_end {
        let end_x = origin.x + label_w + (vm.workspace_end * vm.px_per_sec) as f32;
        let boundary_color = Color32::from_rgb(218, 163, 76);
        painter.line_segment(
            [pos2(end_x, origin.y), pos2(end_x, origin.y + height)],
            egui::Stroke::new(1.5_f32, boundary_color),
        );
        painter.text(
            pos2(end_x + 4.0, origin.y + ruler_h + 2.0),
            egui::Align2::LEFT_TOP,
            format!(
                "工作区结束 {}",
                todo_interaction::fmt_clock(vm.workspace_end)
            ),
            egui::FontId::proportional(10.0),
            boundary_color,
        );
    }

    // Playhead / ruler: scrub. In razor mode the lower lanes cut; the ruler only seeks.
    let playhead_x = playhead_screen_x;
    let ruler = egui::Rect::from_min_size(
        pos2(origin.x + label_w, origin.y),
        egui::vec2((width - label_w).max(1.0), ruler_h),
    );
    let ph_hit = if vm.razor_mode {
        // Keep scrubbing on the ruler; lane clicks handle cut-at-playhead snap.
        egui::Rect::from_center_size(
            pos2(playhead_x, origin.y + ruler_h * 0.5),
            egui::vec2(10.0, ruler_h),
        )
        .intersect(ruler)
    } else {
        egui::Rect::from_center_size(
            pos2(playhead_x, origin.y + height * 0.5),
            egui::vec2(10.0, height),
        )
    };
    let ph_resp = ui.interact(ph_hit, ui.id().with("playhead"), Sense::click_and_drag());
    if ph_resp.clicked() || ph_resp.drag_started() {
        if let Some(pointer) = ph_resp.interact_pointer_pos() {
            child_hit = true;
            intents.push(Intent::Focus(EditorFocus::Timeline));
            intents.push(Intent::BoardPointerDown {
                view_x: f64::from(pointer.x - viewport_left),
                scroll_left,
            });
        }
    }

    let ruler_resp = ui.interact(ruler, ui.id().with("ruler"), Sense::click_and_drag());
    if !child_hit && (ruler_resp.clicked() || ruler_resp.drag_started()) {
        if let Some(pointer) = ruler_resp.interact_pointer_pos() {
            child_hit = true;
            intents.push(Intent::Focus(EditorFocus::Timeline));
            intents.push(Intent::BoardPointerDown {
                view_x: f64::from(pointer.x - viewport_left),
                scroll_left,
            });
        }
    }

    // Select tool: empty board scrubs; Ctrl(+Shift) starts a marquee.
    // Razor: never cut from the catch-all; only lanes / clips cut.
    if !child_hit && !vm.razor_mode && (response.clicked() || response.drag_started()) {
        if let Some(pointer) = response.interact_pointer_pos() {
            intents.push(Intent::Focus(EditorFocus::Timeline));
            let mods = ui.input(|input| input.modifiers);
            if mods.ctrl || mods.command {
                let content_y = f64::from(pointer.y - origin.y);
                intents.push(Intent::MarqueePointerDown {
                    view_x: f64::from(pointer.x - viewport_left),
                    track_index: track_index_at(content_y, vm.tracks.len()).unwrap_or(0),
                    scroll_left,
                    additive: mods.shift,
                });
            } else {
                intents.push(Intent::BoardPointerDown {
                    view_x: f64::from(pointer.x - viewport_left),
                    scroll_left,
                });
            }
        }
    }

    if let Some(marquee) = &vm.marquee {
        let top = origin.y + ruler_h + marquee.track_lo as f32 * row_h;
        let bottom = origin.y + ruler_h + (marquee.track_hi + 1) as f32 * row_h;
        let left = origin.x + label_w + marquee.x0.min(marquee.x1) as f32;
        let right = origin.x + label_w + marquee.x0.max(marquee.x1) as f32;
        let rect = egui::Rect::from_min_max(pos2(left, top), pos2(right, bottom));
        painter.rect_filled(
            rect,
            0.0,
            Color32::from_rgba_unmultiplied(105, 185, 224, 40),
        );
        painter.rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.0_f32, BRAND),
            egui::StrokeKind::Inside,
        );
    }

    if vm.razor_mode {
        // Cut guide only over lanes — ruler is for scrubbing.
        let lanes = egui::Rect::from_min_max(
            pos2(origin.x + label_w, origin.y + ruler_h),
            response.rect.max,
        );
        if let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) {
            if lanes.contains(pointer) {
                painter.line_segment(
                    [
                        pos2(pointer.x, origin.y + ruler_h),
                        pos2(pointer.x, origin.y + height),
                    ],
                    egui::Stroke::new(
                        1.0_f32,
                        Color32::from_rgba_unmultiplied(0xff, 0xb4, 0x5b, 160),
                    ),
                );
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                icons::paint(
                    &painter,
                    egui::Rect::from_center_size(
                        pos2(pointer.x + 12.0, origin.y + 10.0),
                        egui::vec2(16.0, 16.0),
                    ),
                    Icon::Razor,
                    COLORS.playhead,
                );
            }
        }
    }

    if let Some(target) = vm.snap_target.or_else(|| {
        vm.drop_preview
            .as_ref()
            .and_then(|preview| preview.snap_target)
    }) {
        let x = origin.x + label_w + (target * vm.px_per_sec) as f32;
        painter.line_segment(
            [pos2(x, origin.y + ruler_h), pos2(x, origin.y + height)],
            egui::Stroke::new(1.5_f32, BRAND),
        );
    }
    painter.line_segment(
        [
            pos2(playhead_x, origin.y),
            pos2(playhead_x, origin.y + height),
        ],
        egui::Stroke::new(1.0_f32, COLORS.playhead),
    );
    painter.add(egui::Shape::convex_polygon(
        vec![
            pos2(playhead_x - 5.0, origin.y),
            pos2(playhead_x + 5.0, origin.y),
            pos2(playhead_x + 5.0, origin.y + 7.0),
            pos2(playhead_x, origin.y + 12.0),
            pos2(playhead_x - 5.0, origin.y + 7.0),
        ],
        COLORS.playhead,
        egui::Stroke::NONE,
    ));
    intents
}

fn track_index_at(content_y: f64, track_count: usize) -> Option<usize> {
    if track_count == 0 {
        return None;
    }
    let rel = content_y - f64::from(RULER_H);
    if rel < 0.0 {
        return Some(0);
    }
    Some(((rel / f64::from(ROW_H)).floor() as usize).min(track_count - 1))
}

fn sync_zoom_scroll(ui: &Ui, vm: &TimelineViewModel, scroll_id: Id) {
    let key = scroll_id.with("px_per_sec");
    let previous = ui.data(|data| data.get_temp::<f64>(key));
    // Long sequences have a small density, but their anchored offset still
    // needs applying for every zoom step, regardless of absolute scale.
    let density_changed = previous.is_some_and(|px| px != vm.px_per_sec);
    let mut state = egui::scroll_area::State::load(ui.ctx(), scroll_id).unwrap_or_default();
    // After wheel zoom, force the ScrollArea offset to the cursor-anchored value
    // from the interaction layer — density change alone is the zoom signal.
    if density_changed {
        state.offset.x = vm.scroll_left.max(0.0) as f32;
        state.store(ui.ctx(), scroll_id);
    }
    ui.data_mut(|data| data.insert_temp(key, vm.px_per_sec));
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_core::Project;
    use todo_interaction::DragKind;
    mod zoom;

    #[test]
    fn overflow_marks_the_configured_workspace_end_instead_of_the_content_end() {
        let mut vm = crate::test_view(
            todo_core::create_project(None, None, Some(300.0), None, None),
            "test",
        )
        .timeline;
        vm.px_per_sec = 3.0;
        vm.axis = 345.0;
        vm.lane_width = 1035.0;
        vm.board_width = LABEL_W + 1035.0;
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    pos2(0.0, 0.0),
                    egui::vec2(1300.0, 400.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = draw(ui, &vm, &crate::PeaksCache::default());
                });
            },
        );
        let label = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().starts_with("工作区结束") => {
                    Some((shape.clip_rect, text.pos))
                }
                _ => None,
            })
            .expect("overflow editing must show where the configured workspace ends");
        // CentralPanel margin + labels + the fixed 300-second boundary + label padding.
        assert!((label.1.x - (8.0 + LABEL_W as f32 + 900.0 + 4.0)).abs() < 1.0);
        assert!(label.0.contains(label.1));
    }

    #[test]
    fn razor_clip_click_sends_the_raw_pointer_to_interaction() {
        let x = LABEL_W as f32 + 130.0;
        let intents = click_fixture_tool(x, Default::default(), true);
        assert!(intents.iter().any(|intent| matches!(intent, Intent::ClipPointerDown { view_x, .. } if (*view_x - f64::from(x - 8.0)).abs() < 0.01)), "{intents:?}");
    }

    fn media_drop(x: f32, after: Vec<egui::Event>, locked: bool) -> Vec<Intent> {
        media_drop_at(pos2(x, 60.0), after, locked, true, false)
    }

    fn media_drop_at(
        target: egui::Pos2,
        after: Vec<egui::Event>,
        locked: bool,
        video: bool,
        occupied: bool,
    ) -> Vec<Intent> {
        let mut vm = crate::test_view(
            Project {
                version: 1,
                fps: 30.0,
                playhead: 0.0,
                width: 1920,
                height: 1080,
                duration: Some(10.0),
                media_scale: Default::default(),
                media: vec![],
                tracks: vec![],
                text_styles: Vec::new(),
                motions: Vec::new(),
                folders: Vec::new(),
            },
            "test",
        )
        .timeline;
        vm.px_per_sec = 80.0;
        vm.drop_preview = Some(todo_interaction::DropPreview {
            media_id: "source".into(),
            track_id: "v1".into(),
            start: 0.0,
            duration: 2.0,
            allowed: !locked && video,
            snap_target: None,
        });
        vm.tracks = vec![todo_interaction::TrackView {
            id: "v1".into(),
            name: "Video".into(),
            kind: todo_interaction::TrackKind::Video,
            muted: false,
            locked,
            mute_control: false,
            can_move_up: false,
            can_move_down: false,
            clips: vec![],
        }];
        if occupied {
            vm.tracks[0].clips.push(todo_interaction::ClipView {
                id: "existing".into(),
                text: false,
                name: "Existing".into(),
                start: 0.0,
                duration: 2.0,
                offset: 0.0,
                rate: 1.0,
                left: 0.0,
                width: 160.0,
                selected: false,
                audio: false,
                media_path: None,
                has_audio: true,
                wave_left: 0.0,
                wave_width: 160.0,
                transform: Default::default(),
                transition: None,
            });
        }
        let ctx = egui::Context::default();
        let mut result = vec![];
        for stage in 0..3 {
            let pos = if stage == 0 {
                pos2(400.0, 60.0)
            } else {
                target
            };
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if stage != 1 {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: stage == 0,
                    modifiers: Default::default(),
                });
            }
            if stage == 2 {
                events.extend(after.clone());
            }
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        pos2(0.0, 0.0),
                        egui::vec2(800.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    if stage == 0 {
                        egui::DragAndDrop::set_payload(
                            ctx,
                            MediaDrag {
                                media_id: "source".into(),
                            },
                        );
                    }
                    egui::CentralPanel::default().show(ctx, |ui| {
                        result.extend(draw(ui, &vm, &crate::PeaksCache::default()));
                    });
                },
            );
            if stage == 1
                && target.x >= 0.0
                && target.x < LABEL_W as f32
                && target.y == 60.0
                && !locked
                && video
                && !occupied
            {
                assert_eq!(
                    output.platform_output.cursor_icon,
                    egui::CursorIcon::Copy,
                    "The UI must paint the interaction-provided allowed preview"
                );
            }
        }
        result
    }

    #[test]
    fn media_drop_before_start_sends_raw_negative_time_once() {
        for x in [60.0, 2.0] {
            let intents = media_drop(x, vec![], false);
            let drops: Vec<_> = intents
                .iter()
                .filter(|i| matches!(i, Intent::PlaceMedia { .. }))
                .collect();
            assert_eq!(drops.len(), 1, "{intents:?}");
            let expected = f64::from(x - 8.0 - LABEL_W as f32) / 80.0;
            assert!(
                matches!(drops[0], Intent::PlaceMedia { time: Some(t), track_id: Some(track), .. } if (*t - expected).abs() < 0.01 && track == "v1")
            );
            assert!(intents.iter().any(|intent| matches!(intent, Intent::MediaDragHover { track_id, time, .. } if track_id == "v1" && *time < 0.0)));
        }
    }

    #[test]
    fn media_release_position_survives_later_motion_or_pointer_gone() {
        for after in [
            vec![egui::Event::PointerMoved(pos2(700.0, 200.0))],
            vec![egui::Event::PointerGone],
        ] {
            let intents = media_drop(2.0, after, false);
            assert_eq!(
                intents
                    .iter()
                    .filter(
                        |i| matches!(i, Intent::PlaceMedia { time: Some(time), .. } if *time < 0.0)
                    )
                    .count(),
                1
            );
        }
    }

    #[test]
    fn locked_lane_still_reports_the_hit_for_domain_validation() {
        assert!(media_drop(2.0, vec![], true).iter().any(
            |i| matches!(i, Intent::PlaceMedia { track_id: Some(track), .. } if track == "v1")
        ));
        assert!(!media_drop(-2.0, vec![], false)
            .iter()
            .any(|i| matches!(i, Intent::PlaceMedia { .. })));
    }

    #[test]
    fn escape_cancels_media_before_release() {
        let events = vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }];
        assert!(!media_drop(2.0, events, false)
            .iter()
            .any(|i| matches!(i, Intent::PlaceMedia { .. })));
    }

    #[test]
    fn media_drop_rejects_ruler_and_other_rows() {
        for (pos, video) in [(pos2(2.0, 10.0), true), (pos2(2.0, 200.0), true)] {
            assert!(!media_drop_at(pos, vec![], false, video, false)
                .iter()
                .any(|i| matches!(i, Intent::PlaceMedia { .. })));
        }
    }

    #[test]
    fn occupied_or_incompatible_lane_reports_actual_track() {
        for (video, occupied) in [(true, true), (false, false)] {
            let intents = media_drop_at(pos2(2.0, 60.0), vec![], false, video, occupied);
            assert!(
                intents.iter().any(|i| matches!(
                    i,
                    Intent::PlaceMedia {
                        track_id: Some(track_id),
                        ..
                    } if track_id == "v1"
                )),
                "placement policy belongs to interaction: {intents:?}"
            );
        }
    }

    #[test]
    fn media_drop_inside_lane_preserves_positive_time() {
        let intents = media_drop(408.0, vec![], false);
        assert!(
            intents
                .iter()
                .any(|i| matches!(i, Intent::PlaceMedia { time: Some(t), .. }
            if (*t - (400.0 - LABEL_W) / 80.0).abs() < 0.01)),
            "{intents:?}"
        );
    }

    fn release_intents(after_release: Vec<egui::Event>) -> Vec<Intent> {
        let project = Project {
            version: 1,
            fps: 30.0,
            playhead: 0.0,
            width: 1920,
            height: 1080,
            duration: Some(10.0),
            media_scale: Default::default(),
            media: Vec::new(),
            tracks: Vec::new(),
            text_styles: Vec::new(),
            motions: Vec::new(),
            folders: Vec::new(),
        };
        let mut timeline = crate::test_view(project, "test").timeline;
        timeline.drag = Some(DragKind::Seek);
        let ctx = egui::Context::default();
        let pointer = pos2(300.0, 100.0);
        let mut events = vec![
            egui::Event::PointerMoved(pointer),
            egui::Event::PointerButton {
                pos: pointer,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ];
        events.extend(after_release);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                pos2(0.0, 0.0),
                egui::vec2(800.0, 400.0),
            )),
            events,
            ..Default::default()
        };
        let mut intents = Vec::new();
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                intents = draw(ui, &timeline, &crate::PeaksCache::default())
            });
        });
        intents
    }

    fn assert_release_position(intents: Vec<Intent>) {
        let up = intents
            .iter()
            .position(|intent| matches!(intent, Intent::PointerUp))
            .unwrap();
        // CentralPanel's default eight-pixel margin puts the release at 300 - 8.
        assert!(
            matches!(intents[up - 1], Intent::PointerMove { view_x, .. } if (view_x - 292.0).abs() < 0.01)
        );
    }

    #[test]
    fn release_position_is_sent_before_pointer_up() {
        assert_release_position(release_intents(Vec::new()));
    }

    #[test]
    fn motion_after_release_does_not_move_the_final_playhead() {
        assert_release_position(release_intents(vec![egui::Event::PointerMoved(pos2(
            600.0, 100.0,
        ))]));
    }

    #[test]
    fn leaving_after_release_preserves_the_final_playhead() {
        assert_release_position(release_intents(vec![egui::Event::PointerGone]));
    }

    fn click_fixture(x: f32, modifiers: egui::Modifiers) -> Vec<Intent> {
        click_fixture_tool(x, modifiers, false)
    }

    fn click_fixture_tool(x: f32, modifiers: egui::Modifiers, razor: bool) -> Vec<Intent> {
        let mut timeline = crate::test_view(
            Project {
                version: 1,
                fps: 30.0,
                playhead: 5.0,
                width: 1920,
                height: 1080,
                duration: Some(10.0),
                media_scale: Default::default(),
                media: Vec::new(),
                tracks: Vec::new(),
                text_styles: Vec::new(),
                motions: Vec::new(),
                folders: Vec::new(),
            },
            "test",
        )
        .timeline;
        timeline.razor_mode = razor;
        timeline.tracks.clear();
        timeline.tracks.push(todo_interaction::TrackView {
            id: "v1".into(),
            name: "Video 1".into(),
            kind: todo_interaction::TrackKind::Video,
            muted: false,
            locked: false,
            mute_control: false,
            can_move_up: false,
            can_move_down: false,
            clips: vec![todo_interaction::ClipView {
                id: "clip-1".into(),
                text: false,
                name: "Test video".into(),
                start: 1.0,
                duration: 2.0,
                offset: 0.0,
                rate: 1.0,
                left: 80.0,
                width: 160.0,
                selected: false,
                audio: false,
                media_path: None,
                has_audio: true,
                wave_left: 0.0,
                wave_width: 160.0,
                transform: Default::default(),
                transition: None,
            }],
        });
        let ctx = egui::Context::default();
        let pointer = pos2(x, 60.0);
        let mut intents = Vec::new();
        for pressed in [None, Some(true), Some(false)] {
            let mut events = vec![egui::Event::PointerMoved(pointer)];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                });
            }
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 400.0),
                    )),
                    modifiers,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        intents.extend(draw(ui, &timeline, &crate::PeaksCache::default()))
                    });
                },
            );
        }
        intents
    }

    #[test]
    fn primary_click_on_track_header_never_deletes_the_track() {
        let intents = click_fixture(LABEL_W as f32 - 10.0, egui::Modifiers::default());
        assert!(!intents
            .iter()
            .any(|intent| matches!(intent, Intent::DeleteTrack { .. })));
    }

    #[test]
    fn additive_clip_click_selects_without_starting_a_drag() {
        let intents = click_fixture(
            LABEL_W as f32 + 130.0,
            egui::Modifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert!(intents.iter().any(|intent| matches!(intent, Intent::SelectClip { clip_id, additive: true } if clip_id == "clip-1")), "{intents:?}");
        assert!(!intents
            .iter()
            .any(|intent| matches!(intent, Intent::ClipPointerDown { .. })));
    }

    #[test]
    fn ctrl_empty_lane_starts_marquee_instead_of_scrub() {
        let intents = click_fixture(
            LABEL_W as f32 + 400.0,
            egui::Modifiers {
                ctrl: true,
                ..Default::default()
            },
        );
        assert!(
            intents
                .iter()
                .any(|intent| matches!(intent, Intent::MarqueePointerDown { .. })),
            "{intents:?}"
        );
        assert!(!intents.iter().any(|intent| matches!(
            intent,
            Intent::LanePointerDown { .. } | Intent::BoardPointerDown { .. }
        )));
    }
}
