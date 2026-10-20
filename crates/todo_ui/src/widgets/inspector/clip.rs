use crate::widgets::controls::edit_response;
use egui::Ui;
use todo_interaction::{ClipProperties, Intent, SourceProperties};

use crate::theme::{MUTED, TEXT};

pub(super) fn draw(ui: &mut Ui, properties: &ClipProperties) -> Vec<Intent> {
    let mut intents = Vec::new();
    ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);
    ui.add(egui::Label::new(egui::RichText::new(&properties.name).strong().color(TEXT)).wrap())
        .on_hover_text(&properties.name);
    if properties.locked {
        ui.weak("轨道已锁定");
    }
    ui.separator();
    ui.add_enabled_ui(!properties.locked, |ui| {
        ui.push_id((&properties.clip_id, "clip_properties"), |ui| {
            egui::Grid::new("clip_timing")
                .num_columns(2)
                .spacing(egui::vec2(12.0, 10.0))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("时长").color(MUTED));
                    let mut duration = properties.duration;
                    let response = ui
                        .add(
                            egui::DragValue::new(&mut duration)
                                .speed(0.05)
                                .fixed_decimals(2)
                                .range(0.0..=86_400.0)
                                .suffix(" 秒"),
                        )
                        .on_hover_text("拖动或点击输入片段时长；源素材和相邻片段决定可用范围");
                    let change = response.changed().then(|| Intent::SetClipDuration {
                        clip_id: properties.clip_id.clone(),
                        seconds: duration,
                    });
                    edit_response(ui, &response, change, &mut intents);
                    ui.end_row();
                    let mut rows = vec![("起点", format!("{:.2} s", properties.start))];
                    if let SourceProperties::Media { offset, rate } = properties.source {
                        rows.push(("源入点", format!("{offset:.2} s")));
                        rows.push(("播放速率", format!("{rate:.2}×")));
                    }
                    for (label, value) in rows {
                        ui.label(egui::RichText::new(label).color(MUTED));
                        ui.label(egui::RichText::new(value).monospace().color(TEXT));
                        ui.end_row();
                    }
                });
            if let SourceProperties::Text(text) = &properties.source {
                ui.separator();
                intents.extend(super::text::draw(ui, &properties.clip_id, text));
            }
            if properties.visual {
                intents.extend(super::transform::draw(ui, properties));
            }
            if let Some(motion) = &properties.motion {
                ui.separator();
                intents.extend(super::motion::draw(ui, &properties.clip_id, motion));
            }
        });
    });
    intents
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_interaction::ClipTransform;

    fn properties(locked: bool) -> ClipProperties {
        ClipProperties {
            clip_id: "clip".into(),
            name: "Picture".into(),
            start: 1.0,
            duration: 4.0,
            source: SourceProperties::Media {
                offset: 0.0,
                rate: 1.0,
            },
            visual: true,
            locked,
            transform: ClipTransform {
                x: 0.25,
                y: -0.5,
                scale: 1.5,
                scale_x: 0.8,
                scale_y: 1.2,
                opacity: 0.45,
            },
            motion: None,
        }
    }

    fn frame(
        ctx: &egui::Context,
        properties: &ClipProperties,
        events: Vec<egui::Event>,
    ) -> (Vec<Intent>, egui::FullOutput) {
        let mut intents = Vec::new();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    intents = draw(ui, properties);
                });
            },
        );
        (intents, output)
    }

    fn text_position(output: &egui::FullOutput, text: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(shape) if shape.galley.text() == text => {
                    Some(shape.pos + shape.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing field {text}"))
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn reset_transform_emits_a_semantic_command_and_locked_clip_disables_it() {
        for locked in [false, true] {
            let ctx = egui::Context::default();
            let properties = properties(locked);
            let (_, output) = frame(&ctx, &properties, vec![]);
            let pos = text_position(&output, "重置画面");
            frame(
                &ctx,
                &properties,
                vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
            );
            let (intents, _) = frame(&ctx, &properties, vec![pointer(pos, false)]);
            assert_eq!(intents.iter().filter(|intent| matches!(intent, Intent::ResetTransform { clip_id } if clip_id == "clip")).count(), usize::from(!locked));
        }
    }

    #[test]
    fn duration_drag_is_one_edit_transaction() {
        let ctx = egui::Context::default();
        let properties = properties(false);
        let (_, output) = frame(&ctx, &properties, vec![]);
        let pos = text_position(&output, "4.00 秒");
        let mut intents = frame(
            &ctx,
            &properties,
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        )
        .0;
        let moved = pos + egui::vec2(40.0, 0.0);
        intents.extend(frame(&ctx, &properties, vec![egui::Event::PointerMoved(moved)]).0);
        intents.extend(frame(&ctx, &properties, vec![pointer(moved, false)]).0);
        assert_eq!(
            intents
                .iter()
                .filter(|intent| matches!(intent, Intent::BeginEdit))
                .count(),
            1,
            "{intents:?}"
        );
        assert_eq!(
            intents
                .iter()
                .filter(|intent| matches!(intent, Intent::CommitEdit))
                .count(),
            1,
            "{intents:?}"
        );
        assert!(intents.iter().any(|intent| matches!(intent, Intent::SetClipDuration { clip_id, seconds } if clip_id == "clip" && *seconds > 4.0)), "{intents:?}");
        assert!(
            matches!(intents.first(), Some(Intent::BeginEdit)),
            "{intents:?}"
        );
        assert!(
            matches!(intents.last(), Some(Intent::CommitEdit)),
            "{intents:?}"
        );
    }

    #[test]
    fn scale_drag_uses_percent_and_preserves_position() {
        let ctx = egui::Context::default();
        let properties = properties(false);
        let (_, output) = frame(&ctx, &properties, vec![]);
        let pos = text_position(&output, "150.0 %");
        frame(
            &ctx,
            &properties,
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        );
        let moved = pos + egui::vec2(40.0, 0.0);
        let (intents, _) = frame(&ctx, &properties, vec![egui::Event::PointerMoved(moved)]);
        assert!(intents.iter().any(|intent| matches!(intent, Intent::SetClipTransform { clip_id, transform } if clip_id == "clip" && transform.x == 0.25 && transform.y == -0.5 && transform.scale > 1.5 && transform.scale < 3.0)), "{intents:?}");
    }

    #[test]
    fn opacity_control_changes_only_opacity_in_one_transaction() {
        let ctx = egui::Context::default();
        let properties = properties(false);
        let (_, output) = frame(&ctx, &properties, vec![]);
        let pos = text_position(&output, "45.0 %");
        let mut intents = frame(
            &ctx,
            &properties,
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        )
        .0;
        let moved = pos + egui::vec2(40.0, 0.0);
        intents.extend(frame(&ctx, &properties, vec![egui::Event::PointerMoved(moved)]).0);
        intents.extend(frame(&ctx, &properties, vec![pointer(moved, false)]).0);
        assert!(intents.iter().any(|i|matches!(i,Intent::SetClipTransform{transform,..} if transform.opacity>0.45 && transform.opacity<=1.0 && transform.scale==1.5 && transform.scale_x==0.8 && transform.scale_y==1.2 && transform.x==0.25 && transform.y== -0.5)),"{intents:?}");
        assert_eq!(
            intents
                .iter()
                .filter(|i| matches!(i, Intent::BeginEdit))
                .count(),
            1
        );
        assert_eq!(
            intents
                .iter()
                .filter(|i| matches!(i, Intent::CommitEdit))
                .count(),
            1
        );
    }

    #[test]
    fn width_control_preserves_height_and_opacity() {
        let ctx = egui::Context::default();
        let properties = properties(false);
        let (_, output) = frame(&ctx, &properties, vec![]);
        let pos = text_position(&output, "80.0 %");
        frame(
            &ctx,
            &properties,
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        );
        let moved = pos + egui::vec2(40.0, 0.0);
        let (intents, _) = frame(&ctx, &properties, vec![egui::Event::PointerMoved(moved)]);
        assert!(intents.iter().any(|i|matches!(i,Intent::SetClipTransform{transform,..} if transform.scale_x>0.8 && transform.scale_y==1.2 && transform.opacity==0.45 && transform.scale==1.5)),"{intents:?}");
    }

    #[test]
    fn viewing_valid_extreme_transform_does_not_clamp_or_edit_the_clip() {
        let ctx = egui::Context::default();
        let mut properties = properties(false);
        properties.transform.scale = 0.0001;
        properties.transform.scale_x = 100_000.0;
        properties.transform.scale_y = 0.00001;
        assert!(properties.transform.is_valid());
        let (intents, _) = frame(&ctx, &properties, vec![]);
        assert!(
            intents.is_empty(),
            "opening properties must not alter stored values: {intents:?}"
        );
    }

    #[test]
    fn typed_duration_commits_when_the_field_loses_focus() {
        let ctx = egui::Context::default();
        let properties = properties(false);
        let (_, output) = frame(&ctx, &properties, vec![]);
        let pos = text_position(&output, "4.00 秒");
        frame(
            &ctx,
            &properties,
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        );
        frame(&ctx, &properties, vec![pointer(pos, false)]);
        let (mut intents, _) = frame(&ctx, &properties, vec![egui::Event::Text("7.5".into())]);
        assert!(intents.iter().any(|intent| matches!(intent, Intent::SetClipDuration { seconds, .. } if *seconds == 7.5)), "{intents:?}");
        assert!(
            !intents
                .iter()
                .any(|intent| matches!(intent, Intent::CommitEdit)),
            "text edit is still focused: {intents:?}"
        );
        let outside = egui::pos2(740.0, 550.0);
        intents.extend(
            frame(
                &ctx,
                &properties,
                vec![egui::Event::PointerMoved(outside), pointer(outside, true)],
            )
            .0,
        );
        intents.extend(frame(&ctx, &properties, vec![pointer(outside, false)]).0);
        assert_eq!(
            intents
                .iter()
                .filter(|intent| matches!(intent, Intent::BeginEdit))
                .count(),
            1,
            "{intents:?}"
        );
        assert_eq!(
            intents
                .iter()
                .filter(|intent| matches!(intent, Intent::CommitEdit))
                .count(),
            1,
            "{intents:?}"
        );
    }

    #[test]
    fn locked_clip_numeric_fields_do_not_begin_or_mutate_an_edit() {
        let ctx = egui::Context::default();
        let properties = properties(true);
        let (_, output) = frame(&ctx, &properties, vec![]);
        let pos = text_position(&output, "4.00 秒");
        let mut intents = frame(
            &ctx,
            &properties,
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        )
        .0;
        let moved = pos + egui::vec2(40.0, 0.0);
        intents.extend(frame(&ctx, &properties, vec![egui::Event::PointerMoved(moved)]).0);
        intents.extend(frame(&ctx, &properties, vec![pointer(moved, false)]).0);
        assert!(intents.is_empty(), "{intents:?}");
    }
}
