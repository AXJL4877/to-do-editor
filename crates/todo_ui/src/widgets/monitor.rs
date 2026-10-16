use crate::theme::{BORDER, BRAND, DIM, MUTED, PANEL, TEXT};
use egui::{Ui, UiBuilder};
use todo_interaction::{
    EditorViewModel, Intent, MonitorPane, PREVIEW_ZOOM_MAX, PREVIEW_ZOOM_MIN,
};

#[derive(Debug, Clone, Copy)]
pub struct MonitorHole {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub fn draw(ui: &mut Ui, vm: &EditorViewModel) -> (Vec<Intent>, Option<MonitorHole>) {
    let mut intents = Vec::new();
    let rect = ui.available_rect_before_wrap();
    if rect.width() < 2.0 || rect.height() < 2.0 {
        return (intents, None);
    }
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, PANEL);
    let footer_h = 48.0;
    let canvas = egui::Rect::from_min_max(
        rect.min + egui::vec2(8.0, 32.0),
        rect.max - egui::vec2(8.0, footer_h),
    );
    if !canvas.is_positive() {
        return (intents, None);
    }
    painter.rect_filled(canvas, 0.0, egui::Color32::from_rgb(10, 11, 13));
    let program_active = vm.monitor_pane == MonitorPane::Program || !vm.source_available;
    let source_active = vm.monitor_pane == MonitorPane::Source && vm.source_available;
    let tab_y = rect.min.y + 9.0;
    let program_label = "节目";
    let source_label = "源";
    let program_w = 28.0_f32;
    let source_w = 20.0_f32;
    let program_pos = rect.min + egui::vec2(12.0, 9.0);
    let source_pos = rect.min + egui::vec2(12.0 + program_w + 12.0, 9.0);
    ui.painter().text(
        program_pos,
        egui::Align2::LEFT_TOP,
        program_label,
        egui::FontId::proportional(11.0),
        if program_active { TEXT } else { MUTED },
    );
    ui.painter().text(
        source_pos,
        egui::Align2::LEFT_TOP,
        source_label,
        egui::FontId::proportional(11.0),
        if source_active {
            TEXT
        } else if vm.source_available {
            MUTED
        } else {
            DIM
        },
    );
    if program_active {
        painter.line_segment(
            [
                egui::pos2(program_pos.x, rect.min.y + 29.0),
                egui::pos2(program_pos.x + program_w, rect.min.y + 29.0),
            ],
            egui::Stroke::new(2.0_f32, BRAND),
        );
    } else if source_active {
        painter.line_segment(
            [
                egui::pos2(source_pos.x, rect.min.y + 29.0),
                egui::pos2(source_pos.x + source_w, rect.min.y + 29.0),
            ],
            egui::Stroke::new(2.0_f32, BRAND),
        );
    }
    let program_hit = egui::Rect::from_min_size(
        egui::pos2(program_pos.x - 2.0, tab_y - 4.0),
        egui::vec2(program_w + 4.0, 22.0),
    );
    let source_hit = egui::Rect::from_min_size(
        egui::pos2(source_pos.x - 2.0, tab_y - 4.0),
        egui::vec2(source_w + 4.0, 22.0),
    );
    if ui
        .allocate_rect(program_hit, egui::Sense::click())
        .clicked()
    {
        intents.push(Intent::ShowProgramMonitor);
    }
    let source_response = ui.allocate_rect(
        source_hit,
        if vm.source_available {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    if source_response.clicked() && vm.source_available {
        intents.push(Intent::ShowSourceMonitor);
    }
    if !vm.source_available {
        source_response.on_hover_text("双击媒体库素材打开源监视器");
    }
    let header = egui::Rect::from_min_max(
        rect.min + egui::vec2(12.0 + program_w + 12.0 + source_w + 16.0, 3.0),
        egui::pos2(rect.right() - 8.0, rect.top() + 28.0),
    );
    let mut controls = ui.new_child(
        UiBuilder::new()
            .id_salt("preview_viewport_controls")
            .max_rect(header),
    );
    controls.set_clip_rect(header.intersect(ui.clip_rect()));
    controls.horizontal(|ui| {
        let zoom = vm.preview.viewport.zoom();
        if ui
            .add_enabled(zoom > PREVIEW_ZOOM_MIN, egui::Button::new("−").small())
            .on_hover_text("缩小视口")
            .clicked()
        {
            intents.push(Intent::PreviewZoom(zoom / 1.25));
        }
        if rect.width() > 320.0 {
            ui.label(format!("{:.0}%", zoom * 100.0))
                .on_hover_text("滚轮缩放 · 中键平移；100% 为适应窗口");
        }
        if ui
            .add_enabled(zoom < PREVIEW_ZOOM_MAX, egui::Button::new("+").small())
            .on_hover_text("放大视口")
            .clicked()
        {
            intents.push(Intent::PreviewZoom(zoom * 1.25));
        }
        if ui
            .small_button("适应")
            .on_hover_text("适应输出画框并居中（100%）")
            .clicked()
        {
            intents.push(Intent::PreviewFit);
        }
        let mut output_only = vm.preview.viewport.output_only();
        if ui
            .checkbox(&mut output_only, "仅输出")
            .on_hover_text("只查看输出画框内的画面")
            .changed()
        {
            intents.push(Intent::PreviewOutputOnly(output_only));
        }
    });
    if !vm.monitor_active {
        if let Some(projection) = vm.preview.viewport.projection(
            (f64::from(canvas.width()), f64::from(canvas.height())),
            (f64::from(vm.output_width), f64::from(vm.output_height)),
        ) {
            let output = egui::Rect::from_min_size(
                canvas.min
                    + egui::vec2(
                        projection.left as f32 * canvas.width(),
                        projection.top as f32 * canvas.height(),
                    ),
                egui::vec2(
                    projection.width as f32 * canvas.width(),
                    projection.height as f32 * canvas.height(),
                ),
            );
            let painter = painter.with_clip_rect(canvas);
            painter.rect_filled(output, 0.0, egui::Color32::BLACK);
            if !vm.preview.viewport.output_only() {
                painter.rect_stroke(
                    output,
                    0.0,
                    egui::Stroke::new(1.0_f32, BORDER),
                    egui::StrokeKind::Inside,
                );
            }
        }
        let center = canvas.center() - egui::vec2(0.0, 25.0);
        let icon = egui::Rect::from_center_size(center, egui::vec2(42.0, 30.0));
        painter.rect_stroke(
            icon,
            0.0,
            egui::Stroke::new(1.0_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        painter.add(egui::Shape::convex_polygon(
            vec![
                center + egui::vec2(-4.0, -6.0),
                center + egui::vec2(-4.0, 6.0),
                center + egui::vec2(6.0, 0.0),
            ],
            DIM,
            egui::Stroke::NONE,
        ));
        painter.text(
            center + egui::vec2(0.0, 38.0),
            egui::Align2::CENTER_CENTER,
            if vm.monitor_pane == MonitorPane::Source {
                "源监视器无画面"
            } else {
                "播放头处无画面"
            },
            egui::FontId::proportional(13.0),
            MUTED,
        );
        if canvas.height() > 160.0 {
            painter.text(
                center + egui::vec2(0.0, 61.0),
                egui::Align2::CENTER_CENTER,
                if vm.monitor_pane == MonitorPane::Source {
                    "源监视器无可用画面"
                } else {
                    "将素材加入时间线开始剪辑"
                },
                egui::FontId::proportional(11.0),
                DIM,
            );
        }
    }
    ui.allocate_rect(rect, egui::Sense::hover());
    let transport_y = canvas.bottom() + 24.0;
    painter.text(
        egui::pos2(rect.left() + 12.0, transport_y),
        egui::Align2::LEFT_CENTER,
        format!("{} / {}", vm.time_label, vm.duration_label),
        egui::FontId::monospace(11.0),
        BRAND,
    );
    painter.text(
        egui::pos2(rect.right() - 12.0, transport_y),
        egui::Align2::RIGHT_CENTER,
        format!("{:.0} FPS", vm.timeline.fps),
        egui::FontId::proportional(10.0),
        MUTED,
    );
    let transport_rect = egui::Rect::from_center_size(
        egui::pos2(canvas.center().x, transport_y),
        egui::vec2(40.0, 32.0),
    );
    let mut transport_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("monitor_transport")
            .max_rect(transport_rect),
    );
    if super::icons::play_toggle(&mut transport_ui, vm.playing, vm.can_play || vm.playing).clicked()
    {
        intents.push(Intent::TogglePlay);
    }
    (
        intents,
        Some(MonitorHole {
            x: canvas.min.x,
            y: canvas.min.y,
            w: canvas.width(),
            h: canvas.height(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>) -> (Vec<Intent>, egui::FullOutput) {
        let vm = crate::test_view(
            todo_core::create_project(None, None, None, None, None),
            "test",
        );
        let mut intents = Vec::new();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640.0, 400.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    intents = draw(ui, &vm).0;
                });
            },
        );
        (intents, output)
    }

    #[test]
    fn monitor_controls_emit_zoom_fit_and_output_mode_without_edit_transactions() {
        for label in ["+", "适应", "仅输出"] {
            let ctx = egui::Context::default();
            let (_, output) = frame(&ctx, vec![]);
            let pos = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing viewport control {label}"));
            let button = |pressed| egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: Default::default(),
            };
            frame(&ctx, vec![egui::Event::PointerMoved(pos), button(true)]);
            let (intents, _) = frame(&ctx, vec![button(false)]);
            assert!(
                intents.iter().any(|intent| match (label, intent) {
                    ("+", Intent::PreviewZoom(zoom)) => *zoom > 1.0,
                    ("适应", Intent::PreviewFit) | ("仅输出", Intent::PreviewOutputOnly(true)) =>
                        true,
                    _ => false,
                }),
                "{label}: {intents:?}"
            );
            assert!(!intents
                .iter()
                .any(|intent| matches!(intent, Intent::BeginEdit | Intent::CommitEdit)));
        }
    }
}
