use egui::{pos2, Color32, Rect, Sense, Stroke, Ui};
use todo_interaction::{ClipView, EditorFocus, Intent, TimelineViewModel, TrackView};

use crate::peaks::{PeakStrip, PeaksCache};
use crate::theme::{
    AUDIO_CLIP, AUDIO_TEXT, CLIP_TEXT, COLORS, MUTED, TEXT_CLIP, TEXT_CLIP_SELECTED,
};

pub(super) struct ClipGeometry {
    pub rect: Rect,
    pub viewport_left: f32,
    pub scroll_left: f64,
}

pub(super) fn draw(
    ui: &mut Ui,
    vm: &TimelineViewModel,
    track: &TrackView,
    clip: &ClipView,
    geometry: ClipGeometry,
    peaks: &PeaksCache,
    intents: &mut Vec<Intent>,
) -> bool {
    let ClipGeometry {
        rect,
        viewport_left,
        scroll_left,
    } = geometry;
    let response = ui.interact(rect, ui.id().with(&clip.id), Sense::click_and_drag());
    let fill = if track.locked {
        Color32::from_rgb(0x37, 0x3c, 0x42)
    } else if clip.selected {
        if clip.audio {
            Color32::from_rgb(0x3a, 0x92, 0x68)
        } else if clip.text {
            TEXT_CLIP_SELECTED
        } else {
            COLORS.clip_sel
        }
    } else if clip.audio {
        AUDIO_CLIP
    } else if clip.text {
        TEXT_CLIP
    } else {
        COLORS.clip
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    if clip.audio && clip.has_audio {
        if let Some(path) = clip.media_path.as_deref() {
            if let Some(strip) = peaks.get(path) {
                paint_waveform(ui, rect, clip, strip);
            }
        }
    }
    if clip.selected {
        ui.painter().rect_stroke(
            rect,
            0.0,
            Stroke::new(1.5_f32, Color32::from_rgb(0xbe, 0xe1, 0xff)),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(mark) = &clip.transition {
        paint_transition(ui, rect, mark);
    }
    ui.painter().with_clip_rect(rect.shrink(3.0)).text(
        rect.left_top() + egui::vec2(5.0, 3.0),
        egui::Align2::LEFT_TOP,
        &clip.name,
        egui::FontId::proportional(11.0),
        if track.locked {
            MUTED
        } else if clip.audio {
            AUDIO_TEXT
        } else {
            CLIP_TEXT
        },
    );
    if !track.locked && !vm.razor_mode {
        let handle_w = (todo_interaction::EDGE_PX as f32).min(rect.width() * 0.5);
        if clip.selected || response.hovered() || response.dragged() {
            for x in [rect.left(), rect.right() - handle_w] {
                let handle =
                    Rect::from_min_size(pos2(x, rect.top()), egui::vec2(handle_w, rect.height()));
                ui.painter()
                    .rect_filled(handle, 0.0, Color32::from_white_alpha(34));
                if handle_w >= 4.0 {
                    ui.painter().line_segment(
                        [
                            pos2(handle.center().x, handle.center().y - 5.0),
                            pos2(handle.center().x, handle.center().y + 5.0),
                        ],
                        Stroke::new(2.0_f32, Color32::from_white_alpha(200)),
                    );
                }
            }
        }
        if response.hovered() || response.dragged() {
            if let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) {
                let zone =
                    todo_interaction::clip_zone(f64::from(pointer.x - rect.left()), clip.width);
                ui.ctx()
                    .set_cursor_icon(if zone == todo_interaction::ClipZone::Body {
                        if response.dragged() {
                            egui::CursorIcon::Grabbing
                        } else {
                            egui::CursorIcon::Grab
                        }
                    } else {
                        egui::CursorIcon::ResizeHorizontal
                    });
            }
        }
    }
    let mut hit = false;
    if response.clicked() || response.drag_started() {
        if let Some(pointer) = response.interact_pointer_pos() {
            hit = true;
            let modifiers = ui.input(|input| input.modifiers);
            let additive = modifiers.ctrl || modifiers.command || modifiers.shift;
            intents.push(Intent::Focus(EditorFocus::Timeline));
            if vm.razor_mode {
                if !track.locked {
                    intents.push(Intent::ClipPointerDown {
                        clip_id: clip.id.clone(),
                        view_x: f64::from(pointer.x - viewport_left),
                        scroll_left,
                        local_x: f64::from(pointer.x - rect.min.x),
                        width_px: clip.width,
                        alt: modifiers.alt,
                    });
                }
            } else {
                if additive || !clip.selected {
                    intents.push(Intent::SelectClip {
                        clip_id: clip.id.clone(),
                        additive,
                    });
                }
                if !additive && !track.locked {
                    intents.push(Intent::ClipPointerDown {
                        clip_id: clip.id.clone(),
                        view_x: f64::from(pointer.x - viewport_left),
                        scroll_left,
                        local_x: f64::from(pointer.x - rect.min.x),
                        width_px: clip.width,
                        alt: modifiers.alt,
                    });
                }
            }
        }
    }
    if response.secondary_clicked() {
        hit = true;
        intents.push(Intent::Focus(EditorFocus::Timeline));
        if !clip.selected {
            intents.push(Intent::SelectClip {
                clip_id: clip.id.clone(),
                additive: false,
            });
        }
    }
    response.clone().on_hover_text(format!(
        "{} · {:.1}s{}",
        clip.name,
        clip.duration,
        if track.locked { " · 已锁定" } else { "" }
    ));
    response.context_menu(|ui| {
        if ui.button("复制").clicked() {
            intents.push(Intent::Copy);
            ui.close_menu();
        }
        if ui
            .add_enabled(vm.can_delete, egui::Button::new("删除"))
            .clicked()
        {
            intents.push(Intent::DeleteSelection { ripple: false });
            ui.close_menu();
        }
        if ui
            .add_enabled(vm.can_delete, egui::Button::new("波纹删除"))
            .clicked()
        {
            intents.push(Intent::DeleteSelection { ripple: true });
            ui.close_menu();
        }
        ui.menu_button("移动所选到轨道", |ui| {
            for target in &vm.move_targets {
                if ui
                    .add_enabled(target.enabled, egui::Button::new(&target.name))
                    .clicked()
                {
                    intents.push(Intent::MoveSelectionToTrack {
                        track_id: target.track_id.clone(),
                    });
                    ui.close_menu();
                }
            }
        });
    });
    hit
}

/// A bow tie across the cut: the transition window, centred on the clip's
/// left edge. Dimmed while the previous clip is not adjacent.
fn paint_transition(ui: &mut Ui, rect: Rect, mark: &todo_interaction::TransitionMark) {
    let half = (mark.half_px as f32).clamp(4.0, rect.height());
    let (x, top, bottom) = (rect.left(), rect.top() + 2.0, rect.bottom() - 2.0);
    let alpha = if mark.active { 150 } else { 60 };
    let fill = Color32::from_rgba_unmultiplied(0xff, 0xd3, 0x4d, alpha);
    let mid = (top + bottom) * 0.5;
    let painter = ui.painter().with_clip_rect(rect.expand2(egui::vec2(half, 0.0)));
    for points in [
        vec![pos2(x - half, top), pos2(x, mid), pos2(x - half, bottom)],
        vec![pos2(x + half, top), pos2(x, mid), pos2(x + half, bottom)],
    ] {
        painter.add(egui::Shape::convex_polygon(points, fill, Stroke::NONE));
    }
    painter.line_segment([pos2(x, top), pos2(x, bottom)], Stroke::new(1.0_f32, fill));
    let tip = Rect::from_center_size(pos2(x, mid), egui::vec2(half * 2.0, rect.height()));
    ui.interact(tip, ui.id().with(("transition", rect.left() as i32)), Sense::hover())
        .on_hover_text(if mark.active {
            format!("转场：{}", mark.name)
        } else {
            format!("转场：{}（前一个片段没有紧挨着，暂不生效）", mark.name)
        });
}

fn paint_waveform(ui: &mut Ui, rect: Rect, clip: &ClipView, strip: &PeakStrip) {
    if strip.peaks.is_empty()
        || strip.duration <= f64::EPSILON
        || clip.duration <= f64::EPSILON
        || clip.width <= f64::EPSILON
    {
        return;
    }
    let wave_left = clip.wave_left.clamp(0.0, clip.width.max(0.0));
    let wave_width = clip
        .wave_width
        .clamp(0.0, (clip.width - wave_left).max(0.0));
    if wave_width < 1.0 {
        return;
    }
    let x0 = rect.min.x + wave_left as f32;
    let mid_y = rect.center().y;
    let half = (rect.height() * 0.38).max(2.0);
    let stroke = Stroke::new(
        1.0_f32,
        Color32::from_rgba_unmultiplied(0xe8, 0xff, 0xf2, 180),
    );
    let painter = ui.painter().with_clip_rect(rect);
    let cols = wave_width.ceil().max(1.0) as i32;
    let n = strip.peaks.len() as f64;
    // Neural TTS often peaks well below full scale; normalize to the strip
    // so the clip still reads as a waveform (playback gain is unchanged).
    let peak_max = strip
        .peaks
        .iter()
        .copied()
        .fold(0.0_f64, f64::max)
        .max(1e-3);
    for col in 0..cols {
        let local_x = wave_left + (col as f64 + 0.5) * wave_width / cols as f64;
        let media_t = clip.offset + (local_x / clip.width) * clip.duration * clip.rate;
        let idx = ((media_t / strip.duration) * n).floor().clamp(0.0, n - 1.0) as usize;
        let amp = (strip.peaks[idx] / peak_max).clamp(0.0, 1.0) as f32;
        let h = amp * half;
        if h < 0.5 {
            continue;
        }
        let x = x0 + col as f32 + 0.5;
        painter.line_segment([pos2(x, mid_y - h), pos2(x, mid_y + h)], stroke);
    }
}
