//! Motion fields of a picture or text clip: entrance, exit and the transition
//! from the previous clip. Picking a preset is one edit; dragging a duration is
//! one undo step.
use crate::theme::MUTED;
use crate::widgets::controls::edit_response;
use egui::Ui;
use todo_interaction::{Intent, MotionOption, MotionProperties, MotionRef, MotionSlot};

pub(super) fn draw(ui: &mut Ui, clip_id: &str, motion: &MotionProperties) -> Vec<Intent> {
    let mut intents = Vec::new();
    ui.label(egui::RichText::new("动效").color(MUTED));
    egui::Grid::new("clip_motion")
        .num_columns(3)
        .spacing(egui::vec2(8.0, 10.0))
        .show(ui, |ui| {
            let rows = [
                ("入场", MotionSlot::Intro, &motion.intro, &motion.intros, true),
                ("出场", MotionSlot::Outro, &motion.outro, &motion.outros, true),
                (
                    "转场",
                    MotionSlot::Transition,
                    &motion.transition,
                    &motion.transitions,
                    motion.can_transition || motion.transition.is_some(),
                ),
            ];
            for (label, slot, current, options, enabled) in rows {
                ui.label(egui::RichText::new(label).color(MUTED));
                ui.add_enabled_ui(enabled, |ui| {
                    row(ui, clip_id, slot, current.as_ref(), options, &mut intents)
                })
                .response
                .on_disabled_hover_text("转场需要同一轨道上紧挨着的前一个片段");
                ui.end_row();
            }
        });
    if motion.transition.is_some() && !motion.can_transition {
        ui.weak("前一个片段没有紧挨着，转场暂不生效");
    }
    intents
}

fn row(
    ui: &mut Ui,
    clip_id: &str,
    slot: MotionSlot,
    current: Option<&MotionRef>,
    options: &[MotionOption],
    intents: &mut Vec<Intent>,
) {
    let set = |motion: Option<MotionRef>| Intent::SetClipMotion {
        clip_id: clip_id.to_owned(),
        slot,
        motion,
    };
    let name = current.map_or("无", |motion| {
        options
            .iter()
            .find(|option| option.id == motion.kind)
            .map_or(motion.kind.as_str(), |option| option.name.as_str())
    });
    egui::ComboBox::from_id_salt(("motion", slot as u8))
        .selected_text(name)
        .width(110.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), "无").clicked() && current.is_some() {
                intents.push(set(None));
            }
            for option in options {
                let selected = current.is_some_and(|motion| motion.kind == option.id);
                if ui.selectable_label(selected, &option.name).clicked() && !selected {
                    // Keep the chosen length when swapping presets.
                    let duration = current.map_or(option.duration, |motion| motion.duration);
                    intents.push(set(Some(MotionRef::new(option.id.clone(), duration))));
                }
            }
        });
    match current {
        Some(motion) => {
            let mut seconds = motion.duration;
            let response = ui.add(
                egui::DragValue::new(&mut seconds)
                    .speed(0.01)
                    .fixed_decimals(2)
                    .range(0.02..=10.0)
                    .suffix(" 秒"),
            );
            let change = (response.changed() && seconds != motion.duration)
                .then(|| set(Some(MotionRef::new(motion.kind.clone(), seconds))));
            edit_response(ui, &response, change, intents);
        }
        None => {
            ui.label("");
        }
    }
}
