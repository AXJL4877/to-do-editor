//! Flow detail window: a mind-map style tree of the production pipeline.
use super::status_color;
use crate::theme::{BORDER, DIM, MUTED, PANEL, PANEL_2, TEXT};
use egui::{CornerRadius, FontId, Id, RichText, Sense, Stroke, Ui};
use todo_interaction::{FlowNodeView, ProductionNodeStatus, ProductionView};

const OPEN_ID: &str = "todo_ui_production_flow_open";

pub fn open(ctx: &egui::Context, production_id: String) {
    ctx.data_mut(|data| data.insert_temp(Id::new(OPEN_ID), Some(production_id)));
}

pub fn close(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(Id::new(OPEN_ID), None::<String>));
}

fn open_id(ctx: &egui::Context) -> Option<String> {
    ctx.data(|data| data.get_temp::<Option<String>>(Id::new(OPEN_ID)))
        .flatten()
}

/// Draw the flow window when a production has it open.
pub fn draw_window(ctx: &egui::Context, productions: &[ProductionView]) {
    let Some(id) = open_id(ctx) else {
        return;
    };
    let Some(production) = productions.iter().find(|p| p.id == id) else {
        close(ctx);
        return;
    };
    let mut open = true;
    egui::Window::new(format!("流程 · {}", production.title))
        .id(Id::new(("todo_ui_production_flow", &production.id)))
        .open(&mut open)
        .resizable(true)
        .default_size([520.0, 560.0])
        .min_width(360.0)
        .min_height(280.0)
        .frame(
            egui::Frame::NONE
                .fill(PANEL)
                .stroke(Stroke::new(1.0_f32, BORDER))
                .inner_margin(14.0)
                .corner_radius(CornerRadius::ZERO),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&production.style).size(12.0).color(MUTED));
                ui.label(RichText::new("·").size(12.0).color(DIM));
                ui.label(
                    RichText::new(&production.headline)
                        .size(12.0)
                        .color(status_color(production.status)),
                );
            });
            ui.add_space(8.0);
            ui.painter().line_segment(
                [
                    egui::pos2(ui.max_rect().left(), ui.cursor().top()),
                    egui::pos2(ui.max_rect().right(), ui.cursor().top()),
                ],
                Stroke::new(1.0_f32, BORDER),
            );
            ui.add_space(8.0);
            egui::ScrollArea::both()
                .id_salt(("production_flow_scroll", &production.id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    map(ui, &production.flow, 0, true);
                });
        });
    if !open {
        close(ctx);
    }
}

fn map(ui: &mut Ui, node: &FlowNodeView, depth: usize, is_last: bool) {
    let color = status_color(node.status);
    let row_h = if node.detail.is_some() { 44.0 } else { 32.0 };
    let indent = 28.0 * depth as f32;
    let (row, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(280.0), row_h),
        Sense::hover(),
    );
    let painter = ui.painter_at(row);
    // Guide lines from the parent column into this node.
    if depth > 0 {
        let trunk_x = row.left() + indent - 14.0;
        let mid_y = row.center().y;
        let top = row.top();
        let bottom = if is_last { mid_y } else { row.bottom() };
        painter.line_segment(
            [egui::pos2(trunk_x, top), egui::pos2(trunk_x, bottom)],
            Stroke::new(1.2_f32, BORDER),
        );
        painter.line_segment(
            [egui::pos2(trunk_x, mid_y), egui::pos2(trunk_x + 12.0, mid_y)],
            Stroke::new(1.2_f32, BORDER),
        );
    }
    let card = egui::Rect::from_min_size(
        egui::pos2(row.left() + indent, row.top() + 2.0),
        egui::vec2((row.width() - indent - 4.0).clamp(160.0, 420.0), row_h - 4.0),
    );
    let fill = if matches!(
        node.status,
        ProductionNodeStatus::Running | ProductionNodeStatus::Waiting
    ) {
        color.gamma_multiply(0.14)
    } else {
        PANEL_2
    };
    painter.rect(
        card,
        CornerRadius::ZERO,
        fill,
        Stroke::new(1.0_f32, color.gamma_multiply(0.55)),
        egui::StrokeKind::Inside,
    );
    // Status gem on the left of the card.
    let gem = egui::pos2(card.left() + 12.0, card.center().y);
    match node.status {
        ProductionNodeStatus::Complete => {
            painter.circle_filled(gem, 4.0, color);
        }
        ProductionNodeStatus::Failed => {
            painter.rect_filled(
                egui::Rect::from_center_size(gem, egui::vec2(4.0, 10.0)),
                0.0,
                color,
            );
        }
        ProductionNodeStatus::Running | ProductionNodeStatus::Waiting => {
            painter.circle_filled(gem, 4.0, color);
            painter.circle_stroke(gem, 7.0, Stroke::new(1.0_f32, color.gamma_multiply(0.7)));
        }
        _ => {
            painter.circle_stroke(gem, 4.0, Stroke::new(1.2_f32, color));
        }
    }
    let label_x = card.left() + 24.0;
    let label: String = node.label.chars().take(28).collect();
    if let Some(detail) = &node.detail {
        painter.text(
            egui::pos2(label_x, card.top() + 6.0),
            egui::Align2::LEFT_TOP,
            label,
            FontId::proportional(13.0),
            TEXT,
        );
        let detail: String = detail.chars().take(36).collect();
        painter.text(
            egui::pos2(label_x, card.top() + 24.0),
            egui::Align2::LEFT_TOP,
            detail,
            FontId::proportional(11.0),
            MUTED,
        );
    } else {
        painter.text(
            egui::pos2(label_x, card.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.0),
            TEXT,
        );
    }
    let tip = format!(
        "{}\n{}",
        node.label,
        match node.status {
            ProductionNodeStatus::Complete => "已完成",
            ProductionNodeStatus::Running => "进行中",
            ProductionNodeStatus::Waiting => "等待网络",
            ProductionNodeStatus::Attention => "需要处理",
            ProductionNodeStatus::Failed => "失败",
            ProductionNodeStatus::Queued => "排队 / 未开始",
        }
    );
    response.on_hover_text(tip);

    let count = node.children.len();
    for (index, child) in node.children.iter().enumerate() {
        map(ui, child, depth + 1, index + 1 == count);
    }
}
