mod board;
mod clip;
mod header;

use egui::Ui;
use todo_interaction::{EditorFocus, Intent, TimelineTool, TimelineViewModel};

use crate::peaks::PeaksCache;
use crate::theme::{button, Btn, BORDER, MUTED, PANEL, TEXT};
use crate::widgets::icons::{self, Icon};

pub fn draw(ui: &mut Ui, vm: &TimelineViewModel, peaks: &PeaksCache) -> Vec<Intent> {
    let mut intents = Vec::new();
    ui.painter().rect_filled(ui.max_rect(), 0.0, PANEL);
    ui.spacing_mut().item_spacing.x = 5.0;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("时间线")
                .strong()
                .size(11.0)
                .color(TEXT),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(format!(
                    "已选 {}   /   {:.0} fps",
                    vm.selected_ids.len(),
                    vm.fps
                ))
                .size(10.0)
                .color(MUTED),
            );
        });
    });
    ui.horizontal_wrapped(|ui| {
        ui.add_space(5.0);
        if icons::tool_button(
            ui,
            Icon::Select,
            vm.tool == TimelineTool::Select,
            "选择工具 (V)",
        )
        .clicked()
        {
            intents.push(Intent::ToolClicked(TimelineTool::Select));
        }
        if icons::tool_button(
            ui,
            Icon::Razor,
            vm.tool == TimelineTool::Razor,
            "剃刀工具 (C)",
        )
        .clicked()
        {
            intents.push(Intent::ToolClicked(TimelineTool::Razor));
        }
        ui.separator();
        if button(
            ui,
            "吸附",
            Btn::Tool {
                active: vm.snapping,
            },
            true,
        )
        .on_hover_text("吸附到帧、片段边缘与播放头")
        .clicked()
        {
            intents.push(Intent::ToggleSnapping);
        }
        if button(
            ui,
            "链接选择",
            Btn::Tool {
                active: vm.linked_selection,
            },
            true,
        )
        .on_hover_text(
            "一起选择已链接的片段（如同一素材的音视频）；关闭后可单独选择，不会解除已有链接",
        )
        .clicked()
        {
            intents.push(Intent::ToggleLinkedSelection);
        }
        if button(ui, "整理轨道", Btn::Normal, true)
            .on_hover_text("合并不重叠的片段，保持上下层关系并减少多余轨道")
            .clicked()
        {
            intents.push(Intent::CompactTracks);
        }
    });
    ui.add_space(3.0);
    let top = ui.cursor().top();
    ui.painter().line_segment(
        [
            egui::pos2(ui.min_rect().left(), top),
            egui::pos2(ui.max_rect().right(), top),
        ],
        egui::Stroke::new(1.0_f32, BORDER),
    );
    if !intents.is_empty() {
        intents.insert(0, Intent::Focus(EditorFocus::Timeline));
    }
    intents.extend(board::draw(ui, vm, peaks));
    intents
}
