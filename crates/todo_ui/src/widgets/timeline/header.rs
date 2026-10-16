use egui::{Color32, Rect, Sense, Ui};
use todo_interaction::{EditorFocus, Intent, TrackView};

use crate::theme::{BORDER, BRAND, MUTED, PANEL_2, TEXT};
use crate::widgets::icons::{self, Icon};

pub(super) fn draw(
    ui: &mut Ui,
    track: &TrackView,
    rect: Rect,
    active: bool,
    intents: &mut Vec<Intent>,
) -> bool {
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        0.0,
        if active {
            Color32::from_rgb(0x1d, 0x3b, 0x52)
        } else {
            PANEL_2
        },
    );
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    let name_rect = Rect::from_min_max(
        rect.min + egui::vec2(34.0, 4.0),
        rect.max - egui::vec2(4.0, 22.0),
    );
    painter.with_clip_rect(name_rect).text(
        name_rect.left_center(),
        egui::Align2::LEFT_CENTER,
        &track.name,
        egui::FontId::proportional(11.0),
        if track.locked {
            MUTED
        } else if active {
            BRAND
        } else {
            TEXT
        },
    );
    let response = ui.interact(rect, ui.id().with(&track.id).with("header"), Sense::click());
    let mut hit = response.clicked() || response.secondary_clicked();
    if hit {
        intents.push(Intent::Focus(EditorFocus::Timeline));
    }
    response.context_menu(|ui| {
        ui.strong(format!("轨道：{}", track.name));
        if ui
            .button(if track.locked {
                "解锁轨道"
            } else {
                "锁定轨道"
            })
            .clicked()
        {
            intents.push(Intent::ToggleLock {
                track_id: track.id.clone(),
            });
            ui.close_menu();
        }
        let visibility = if track.mute_control {
            if track.muted {
                "取消静音"
            } else {
                "静音轨道"
            }
        } else if track.muted {
            "显示视频轨道"
        } else {
            "隐藏视频轨道"
        };
        if ui.button(visibility).clicked() {
            intents.push(Intent::ToggleMute {
                track_id: track.id.clone(),
            });
            ui.close_menu();
        }
        ui.separator();
        if ui
            .add_enabled(track.can_move_up, egui::Button::new("上移轨道"))
            .on_hover_text("与上方同类型轨道交换位置")
            .clicked()
        {
            intents.push(Intent::MoveTrack {
                track_id: track.id.clone(),
                up: true,
            });
            ui.close_menu();
        }
        if ui
            .add_enabled(track.can_move_down, egui::Button::new("下移轨道"))
            .on_hover_text("与下方同类型轨道交换位置")
            .clicked()
        {
            intents.push(Intent::MoveTrack {
                track_id: track.id.clone(),
                up: false,
            });
            ui.close_menu();
        }
        ui.separator();
        if ui
            .add_enabled(!track.locked, egui::Button::new("删除轨道及其片段"))
            .on_hover_text("删除该轨道及片段；媒体库中的素材保留")
            .clicked()
        {
            intents.push(Intent::DeleteTrack {
                track_id: track.id.clone(),
            });
            ui.close_menu();
        }
    });

    let add_rect = Rect::from_min_size(rect.min + egui::vec2(5.0, 13.0), egui::vec2(22.0, 22.0));
    if icons::chip(
        ui,
        ui.id().with(&track.id).with("insert_below"),
        add_rect,
        Icon::Plus,
        false,
        "在此轨道下方添加同类型轨道",
    )
    .clicked()
    {
        hit = true;
        intents.push(Intent::Focus(EditorFocus::Timeline));
        intents.push(Intent::InsertTrackBelow {
            track_id: track.id.clone(),
        });
    }
    let mute_icon = if track.mute_control {
        if track.muted {
            Icon::SpeakerOff
        } else {
            Icon::Speaker
        }
    } else if track.muted {
        Icon::EyeOff
    } else {
        Icon::Eye
    };
    let mute_tip = if track.mute_control {
        if track.muted {
            "取消静音"
        } else {
            "静音"
        }
    } else if track.muted {
        "显示轨道"
    } else {
        "隐藏轨道"
    };
    let lock_icon = if track.locked {
        Icon::Lock
    } else {
        Icon::Unlock
    };

    for (index, (icon, tooltip, lit, lock)) in [
        (mute_icon, mute_tip, track.muted, false),
        (
            lock_icon,
            if track.locked { "解锁" } else { "锁定" },
            track.locked,
            true,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let control = Rect::from_min_size(
            rect.min + egui::vec2(34.0 + index as f32 * 26.0, 26.0),
            egui::vec2(22.0, 17.0),
        );
        let response = icons::chip(
            ui,
            ui.id().with(&track.id).with(("control", index)),
            control,
            icon,
            lit,
            tooltip,
        );
        if response.clicked() {
            hit = true;
            intents.push(Intent::Focus(EditorFocus::Timeline));
            intents.push(if lock {
                Intent::ToggleLock {
                    track_id: track.id.clone(),
                }
            } else {
                Intent::ToggleMute {
                    track_id: track.id.clone(),
                }
            });
        }
    }
    hit
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_interaction::TrackKind;
    #[test]
    fn left_add_control_identifies_the_exact_track_without_editing_it() {
        for (id, kind) in [("v7", TrackKind::Video), ("a3", TrackKind::Audio)] {
            let ctx = egui::Context::default();
            let track = TrackView {
                id: id.into(),
                name: "Track".into(),
                kind,
                muted: false,
                locked: true,
                mute_control: kind != TrackKind::Video,
                can_move_up: false,
                can_move_down: false,
                clips: Vec::new(),
            };
            let pos = egui::pos2(24.0, 32.0);
            let mut intents = Vec::new();
            for stage in 0..3 {
                let mut events = vec![egui::Event::PointerMoved(pos)];
                if stage > 0 {
                    events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: stage == 1,
                        modifiers: Default::default(),
                    });
                }
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(300.0, 160.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            draw(
                                ui,
                                &track,
                                Rect::from_min_size(egui::pos2(8.0, 8.0), egui::vec2(140.0, 48.0)),
                                true,
                                &mut intents,
                            );
                        });
                    },
                );
            }
            assert_eq!(
                intents
                    .iter()
                    .filter(|i| matches!(i,Intent::InsertTrackBelow{track_id} if track_id==id))
                    .count(),
                1,
                "{intents:?}"
            );
            assert!(!intents.iter().any(|i| matches!(
                i,
                Intent::DeleteTrack { .. } | Intent::ToggleLock { .. } | Intent::ToggleMute { .. }
            )));
        }
    }
}
