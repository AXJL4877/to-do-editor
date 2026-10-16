//! Transform parameter controls; emits semantic edits and never changes a project.
use crate::theme::{button, Btn, MUTED, TEXT};
use crate::widgets::controls::edit_response;
use egui::{RichText, Ui};
use todo_interaction::{ClipProperties, ClipTransform, Intent};

#[derive(Clone, Copy)]
enum PercentField {
    Scale,
    Width,
    Height,
    Opacity,
}
impl PercentField {
    fn set(self, transform: &mut ClipTransform, value: f64) {
        match self {
            Self::Scale => transform.scale = value,
            Self::Width => transform.scale_x = value,
            Self::Height => transform.scale_y = value,
            Self::Opacity => transform.opacity = value,
        }
    }
}

pub(super) fn draw(ui: &mut Ui, properties: &ClipProperties) -> Vec<Intent> {
    let mut intents = Vec::new();
    ui.add_space(12.0);
    ui.separator();
    ui.label(RichText::new("变换").strong().small().color(TEXT));
    ui.add_space(4.0);
    egui::Grid::new("clip_transform")
        .num_columns(2)
        .spacing(egui::vec2(12.0, 10.0))
        .show(ui, |ui| {
            for axis in ["X", "Y"] {
                ui.label(RichText::new(axis).color(MUTED));
                let mut value = if axis == "X" {
                    properties.transform.x
                } else {
                    properties.transform.y
                };
                let response = ui
                    .add(
                        egui::DragValue::new(&mut value)
                            .speed(0.005)
                            .fixed_decimals(3),
                    )
                    .on_hover_text("位置以画布宽高为单位：0 为居中，1 为一个画布宽度或高度");
                let mut transform = properties.transform;
                if axis == "X" {
                    transform.x = value
                } else {
                    transform.y = value
                }
                let change = response.changed().then(|| Intent::SetClipTransform {
                    clip_id: properties.clip_id.clone(),
                    transform,
                });
                edit_response(ui, &response, change, &mut intents);
                ui.end_row();
            }
            for (field, label, factor, tooltip) in [
                (
                    PercentField::Scale,
                    "缩放",
                    properties.transform.scale,
                    "同时缩放宽度和高度",
                ),
                (
                    PercentField::Width,
                    "宽度缩放",
                    properties.transform.scale_x,
                    "仅调整宽度；以统一缩放后的尺寸为基准",
                ),
                (
                    PercentField::Height,
                    "高度缩放",
                    properties.transform.scale_y,
                    "仅调整高度；以统一缩放后的尺寸为基准",
                ),
                (
                    PercentField::Opacity,
                    "不透明度",
                    properties.transform.opacity,
                    "0% 完全透明，100% 完全不透明",
                ),
            ] {
                ui.label(RichText::new(label).color(MUTED));
                let mut value = factor * 100.0;
                let range = if matches!(field, PercentField::Opacity) {
                    0.0..=100.0
                } else {
                    0.1..=10_000.0
                };
                let response = ui
                    .add(
                        egui::DragValue::new(&mut value)
                            .speed(0.5)
                            .fixed_decimals(1)
                            .range(range)
                            .clamp_existing_to_range(false)
                            .suffix(" %"),
                    )
                    .on_hover_text(tooltip);
                let mut transform = properties.transform;
                field.set(&mut transform, value / 100.0);
                let change = response.changed().then(|| Intent::SetClipTransform {
                    clip_id: properties.clip_id.clone(),
                    transform,
                });
                edit_response(ui, &response, change, &mut intents);
                ui.end_row();
            }
        });
    ui.add_space(12.0);
    if button(ui, "重置画面", Btn::Tool { active: false }, true)
        .on_hover_text("重置位置、宽高缩放和不透明度")
        .clicked()
    {
        intents.push(Intent::ResetTransform {
            clip_id: properties.clip_id.clone(),
        });
    }
    intents
}
