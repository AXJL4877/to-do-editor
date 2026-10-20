//! Fullscreen free-canvas workflow editor (`~` toggle).
use super::status_color;
use super::{selected_workflow, set_selected_workflow, ProductionAction};
use crate::theme::{BORDER, BRAND, DIM, MUTED, PANEL, PANEL_2, TEXT};
use egui::{CornerRadius, FontId, Id, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use std::collections::BTreeMap;
use todo_interaction::{
    ProductionNodeStatus, ProductionPanelView, WorkflowNodeView, WorkflowPanelView,
    WorkflowParamFieldView,
};

const OPEN_ID: &str = "todo_ui_workflow_panel_open";
const CARD_W: f32 = 200.0;
const CARD_H: f32 = 118.0;
const INSPECTOR_W: f32 = 300.0;

#[derive(Clone)]
struct CanvasState {
    pan: Vec2,
    zoom: f32,
    positions: BTreeMap<String, Pos2>,
    dragging: Option<String>,
    selected: Option<String>,
    /// Pointer down position (screen) to distinguish click vs drag.
    press_screen: Option<Pos2>,
    /// Session overrides of inspector values: node_id → field_id → text.
    edits: BTreeMap<String, BTreeMap<String, String>>,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
            positions: BTreeMap::new(),
            dragging: None,
            selected: None,
            press_screen: None,
            edits: BTreeMap::new(),
        }
    }
}

pub fn open(ctx: &egui::Context, extension_id: String) {
    set_selected_workflow(ctx, Some(extension_id.clone()));
    ctx.data_mut(|data| data.insert_temp(Id::new(OPEN_ID), Some(extension_id)));
}

pub fn close(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(Id::new(OPEN_ID), None::<String>));
}

pub fn open_id(ctx: &egui::Context) -> Option<String> {
    ctx.data(|data| data.get_temp::<Option<String>>(Id::new(OPEN_ID)))
        .flatten()
}

pub fn is_open(ctx: &egui::Context) -> bool {
    open_id(ctx).is_some()
}

pub fn is_blocking_input(ctx: &egui::Context) -> bool {
    is_open(ctx)
}

/// `~` toggles fullscreen; Esc closes. Skip `~` while a text field wants keys.
pub fn handle_hotkeys(ctx: &egui::Context, panel: &ProductionPanelView) {
    let (tilde, esc) = ctx.input(|input| {
        (
            input.key_pressed(egui::Key::Backtick),
            input.key_pressed(egui::Key::Escape),
        )
    });
    if esc && is_open(ctx) {
        close(ctx);
        return;
    }
    if tilde && (!ctx.wants_keyboard_input() || is_open(ctx)) {
        if is_open(ctx) {
            close(ctx);
        } else if let Some(id) = selected_workflow(ctx)
            .or_else(|| panel.workflows.first().map(|e| e.extension_id.clone()))
        {
            open(ctx, id);
        }
    }
}

pub fn draw_fullscreen(
    ctx: &egui::Context,
    extension_id: &str,
    title: &str,
    workflow: &WorkflowPanelView,
) -> ProductionAction {
    let mut action = ProductionAction::None;
    let screen = ctx.screen_rect();
    egui::Area::new(Id::new("todo_ui_workflow_fullscreen"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            ui.set_min_size(screen.size());
            ui.set_max_size(screen.size());
            egui::Frame::NONE
                .fill(PANEL)
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("工作流 · {title}"))
                                .strong()
                                .size(15.0)
                                .color(TEXT),
                        );
                        ui.label(RichText::new(extension_id).size(11.0).color(DIM));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(RichText::new("关闭").size(12.5))
                                .on_hover_text("Esc 或 ~")
                                .clicked()
                            {
                                close(ui.ctx());
                            }
                            if ui
                                .button(RichText::new("保存工作流").size(12.5))
                                .on_hover_text("写入 graph.json（含节点位置）")
                                .clicked()
                            {
                                let key = canvas_id(extension_id);
                                let state = ui
                                    .ctx()
                                    .data(|d| d.get_temp::<CanvasState>(key))
                                    .unwrap_or_default();
                                let positions = state
                                    .positions
                                    .iter()
                                    .map(|(id, p)| (id.clone(), (p.x, p.y)))
                                    .collect();
                                action = ProductionAction::WorkflowSave {
                                    extension: extension_id.to_owned(),
                                    positions,
                                    edits: state.edits.clone(),
                                };
                            }
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("复位").size(12.0)).frame(false),
                                )
                                .on_hover_text("重置平移、缩放与拖动位置")
                                .clicked()
                            {
                                let key = canvas_id(extension_id);
                                ui.ctx()
                                    .data_mut(|d| d.insert_temp(key, CanvasState::default()));
                            }
                            ui.label(
                                RichText::new("拖动画布 · 滚轮缩放 · 点击节点")
                                    .size(11.0)
                                    .color(DIM),
                            );
                        });
                    });
                    ui.add_space(8.0);

                    let key = canvas_id(extension_id);
                    let mut state = ui
                        .ctx()
                        .data(|d| d.get_temp::<CanvasState>(key))
                        .unwrap_or_default();
                    for node in &workflow.nodes {
                        state
                            .positions
                            .entry(node.id.clone())
                            .or_insert(Pos2::new(node.x, node.y));
                    }

                    let full = ui.available_size();
                    let show_inspector = state.selected.is_some();
                    let canvas_w = if show_inspector {
                        (full.x - INSPECTOR_W - 8.0).max(200.0)
                    } else {
                        full.x
                    };

                    ui.horizontal(|ui| {
                        let (canvas, response) = ui.allocate_exact_size(
                            egui::vec2(canvas_w, full.y),
                            Sense::click_and_drag(),
                        );
                        interact_canvas(ui, canvas, &response, workflow, &mut state);
                        paint_canvas(ui, canvas, workflow, &state);

                        if let Some(sel) = state.selected.clone() {
                            if let Some(node) = workflow.nodes.iter().find(|n| n.id == sel) {
                                ui.add_space(8.0);
                                egui::Frame::NONE
                                    .fill(PANEL_2)
                                    .stroke(Stroke::new(1.0_f32, BORDER))
                                    .inner_margin(12.0)
                                    .corner_radius(CornerRadius::same(8))
                                    .show(ui, |ui| {
                                        ui.set_width(INSPECTOR_W - 8.0);
                                        ui.set_min_height(full.y);
                                        let next =
                                            draw_inspector(ui, node, extension_id, &mut state);
                                        if next != ProductionAction::None {
                                            action = next;
                                        }
                                    });
                            }
                        }
                    });

                    ui.ctx().data_mut(|d| d.insert_temp(key, state));
                });
        });
    action
}

fn canvas_id(extension_id: &str) -> Id {
    Id::new(("todo_ui_workflow_canvas", extension_id))
}

fn interact_canvas(
    ui: &mut Ui,
    canvas: Rect,
    response: &egui::Response,
    workflow: &WorkflowPanelView,
    state: &mut CanvasState,
) {
    let hit = |graph_pos: Pos2, state: &CanvasState| -> Option<String> {
        for node in workflow.nodes.iter().rev() {
            let origin = state
                .positions
                .get(&node.id)
                .copied()
                .unwrap_or(Pos2::new(node.x, node.y));
            let rect = Rect::from_min_size(origin, Vec2::new(CARD_W, CARD_H));
            if rect.contains(graph_pos) {
                return Some(node.id.clone());
            }
        }
        None
    };

    if response.drag_started() {
        state.press_screen = response.interact_pointer_pos();
        if let Some(pointer) = response.interact_pointer_pos() {
            let graph_pos = screen_to_graph(pointer, canvas, state);
            state.dragging = hit(graph_pos, state);
        }
    }

    let on_node = state.dragging.is_some();
    if response.dragged() && !on_node {
        state.pan += response.drag_delta();
    }
    if let Some(id) = state.dragging.clone() {
        if response.dragged() {
            let delta = response.drag_delta() / state.zoom;
            if let Some(pos) = state.positions.get_mut(&id) {
                *pos += delta;
            }
        }
    }

    if response.drag_stopped() {
        let click = state
            .press_screen
            .zip(response.interact_pointer_pos())
            .is_some_and(|(a, b)| a.distance(b) < 5.0);
        if click {
            if let Some(pointer) = response.interact_pointer_pos() {
                let graph_pos = screen_to_graph(pointer, canvas, state);
                state.selected = hit(graph_pos, state);
            }
        } else if !on_node {
            // pan finished on empty — keep selection
        }
        state.dragging = None;
        state.press_screen = None;
    }

    if response.clicked() && !response.dragged() {
        if let Some(pointer) = response.interact_pointer_pos() {
            let graph_pos = screen_to_graph(pointer, canvas, state);
            state.selected = hit(graph_pos, state);
        }
    }

    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.1 {
            let next = (state.zoom * (1.0 + scroll * 0.0015)).clamp(0.35, 2.5);
            if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
                let before = screen_to_graph(pointer, canvas, state);
                state.zoom = next;
                let after = screen_to_graph(pointer, canvas, state);
                state.pan += (after - before) * state.zoom;
            } else {
                state.zoom = next;
            }
        }
    }
}

fn paint_canvas(ui: &mut Ui, canvas: Rect, workflow: &WorkflowPanelView, state: &CanvasState) {
    let painter = ui.painter_at(canvas);
    painter.rect_filled(canvas, CornerRadius::same(8), PANEL_2);
    painter.rect_stroke(
        canvas,
        CornerRadius::same(8),
        Stroke::new(1.0_f32, BORDER),
        egui::epaint::StrokeKind::Inside,
    );
    draw_grid(&painter, canvas, state);

    for edge in &workflow.edges {
        let Some(from) = workflow.nodes.iter().find(|n| n.id == edge.from) else {
            continue;
        };
        let Some(to) = workflow.nodes.iter().find(|n| n.id == edge.to) else {
            continue;
        };
        let a = state
            .positions
            .get(&from.id)
            .copied()
            .unwrap_or(Pos2::new(from.x, from.y));
        let b = state
            .positions
            .get(&to.id)
            .copied()
            .unwrap_or(Pos2::new(to.x, to.y));
        let start = graph_to_screen(Pos2::new(a.x + CARD_W, a.y + CARD_H * 0.5), canvas, state);
        let end = graph_to_screen(Pos2::new(b.x, b.y + CARD_H * 0.5), canvas, state);
        draw_edge(&painter, start, end, MUTED);
    }

    for node in &workflow.nodes {
        let origin = state
            .positions
            .get(&node.id)
            .copied()
            .unwrap_or(Pos2::new(node.x, node.y));
        let screen_min = graph_to_screen(origin, canvas, state);
        let screen_max = graph_to_screen(origin + Vec2::new(CARD_W, CARD_H), canvas, state);
        let rect = Rect::from_min_max(screen_min, screen_max);
        if rect.intersects(canvas) {
            let selected = state.selected.as_deref() == Some(node.id.as_str());
            paint_node(&painter, rect, node, state.zoom, selected);
        }
    }
}

fn draw_inspector(
    ui: &mut Ui,
    node: &WorkflowNodeView,
    extension_id: &str,
    state: &mut CanvasState,
) -> ProductionAction {
    let mut action = ProductionAction::None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(&node.label).size(16.0).color(TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(egui::Button::new(RichText::new("×").size(14.0).color(MUTED)).frame(false))
                .clicked()
            {
                state.selected = None;
            }
        });
    });
    ui.label(RichText::new(&node.node_type).size(12.0).color(BRAND));
    ui.add_space(4.0);
    let status = match node.status {
        ProductionNodeStatus::Queued => "排队",
        ProductionNodeStatus::Running => "进行中",
        ProductionNodeStatus::Waiting => "等待",
        ProductionNodeStatus::Complete => "完成",
        ProductionNodeStatus::Failed => "失败",
        ProductionNodeStatus::Attention => "注意",
    };
    ui.label(
        RichText::new(format!(
            "状态 · {status}{}",
            node.detail
                .as_ref()
                .map(|d| format!(" · {d}"))
                .unwrap_or_default()
        ))
        .size(12.0)
        .color(status_color(node.status)),
    );
    ui.add_space(8.0);
    ui.label(RichText::new(&node.summary).size(12.0).color(MUTED));
    if !node.docs.is_empty() {
        ui.add_space(4.0);
        ui.label(RichText::new(&node.docs).size(11.0).color(DIM));
    }

    ui.add_space(10.0);
    ui.separator();
    ui.label(RichText::new("参数").size(13.0).color(TEXT));
    ui.add_space(6.0);

    if node.fields.is_empty() && node.files.is_empty() {
        ui.label(RichText::new("此节点暂无可配置参数").size(11.0).color(DIM));
    } else {
        let edits = state.edits.entry(node.id.clone()).or_default();
        for field in &node.fields {
            match field {
                WorkflowParamFieldView::Select {
                    id,
                    label,
                    hint,
                    options,
                    value,
                } => {
                    ui.label(RichText::new(label).size(12.0).color(TEXT));
                    let current = edits.get(id).cloned().unwrap_or_else(|| value.clone());
                    let mut picked = current.clone();
                    egui::ComboBox::from_id_salt(("wf_sel", &node.id, id))
                        .selected_text(options.iter().find(|(v, _)| *v == current).map(|(_, l)| l.as_str()).unwrap_or(&current))
                        .width(ui.available_width() - 8.0)
                        .show_ui(ui, |ui| {
                            for (v, l) in options {
                                if ui.selectable_label(picked == *v, l).clicked() {
                                    picked = v.clone();
                                }
                            }
                        });
                    if picked != current {
                        edits.insert(id.clone(), picked);
                    }
                    ui.label(RichText::new(hint).size(10.0).color(DIM));
                    ui.add_space(8.0);
                }
                WorkflowParamFieldView::Number {
                    id,
                    label,
                    hint,
                    min,
                    max,
                    step,
                    unit,
                    value,
                } => {
                    ui.label(RichText::new(label).size(12.0).color(TEXT));
                    let mut num = edits
                        .get(id)
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(*value);
                    let mut drag = egui::DragValue::new(&mut num)
                        .range(*min..=*max)
                        .speed(*step);
                    if let Some(u) = unit {
                        drag = drag.suffix(format!(" {u}"));
                    }
                    if ui.add(drag).changed() {
                        edits.insert(id.clone(), format!("{num}"));
                    }
                    ui.label(RichText::new(hint).size(10.0).color(DIM));
                    ui.add_space(8.0);
                }
                WorkflowParamFieldView::Text {
                    id,
                    label,
                    hint,
                    placeholder,
                    value,
                } => {
                    ui.label(RichText::new(label).size(12.0).color(TEXT));
                    let mut text = edits.get(id).cloned().unwrap_or_else(|| value.clone());
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .hint_text(placeholder.as_str())
                            .desired_width(ui.available_width() - 8.0),
                    );
                    if resp.changed() {
                        edits.insert(id.clone(), text);
                    }
                    ui.label(RichText::new(hint).size(10.0).color(DIM));
                    ui.add_space(8.0);
                }
                WorkflowParamFieldView::FileKinds {
                    label,
                    hint,
                    accept,
                    ..
                } => {
                    ui.label(RichText::new(label).size(12.0).color(TEXT));
                    ui.add_space(4.0);
                    egui::Frame::NONE
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .inner_margin(10.0)
                        .corner_radius(CornerRadius::same(6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(format!("支持 {accept}"))
                                    .size(10.0)
                                    .color(DIM),
                            );
                            if ui
                                .button(RichText::new("添加文件…").size(12.0))
                                .clicked()
                            {
                                action = ProductionAction::WorkflowPickMedia {
                                    extension: extension_id.to_owned(),
                                    node_id: node.id.clone(),
                                };
                            }
                        });
                    ui.label(RichText::new(hint).size(10.0).color(DIM));
                    ui.add_space(6.0);
                    if node.files.is_empty() {
                        ui.label(RichText::new("尚未添加文件").size(11.0).color(DIM));
                    } else {
                        for file in &node.files {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("[{}] {}", file.kind, file.name))
                                        .size(11.0)
                                        .color(MUTED),
                                );
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("移除").size(11.0).color(DIM),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    action = ProductionAction::WorkflowRemoveMedia {
                                        extension: extension_id.to_owned(),
                                        node_id: node.id.clone(),
                                        path: file.path.clone(),
                                    };
                                }
                            });
                        }
                    }
                    ui.add_space(8.0);
                }
            }
        }
    }

    let has_scalar = node.fields.iter().any(|f| {
        !matches!(f, WorkflowParamFieldView::FileKinds { .. })
    });
    if has_scalar {
        ui.add_space(4.0);
        if ui
            .add(egui::Button::new(RichText::new("保存到 graph.json").size(12.5)))
            .on_hover_text("把当前参数写回扩展包的工作流图")
            .clicked()
        {
            let mut params = BTreeMap::new();
            for field in &node.fields {
                match field {
                    WorkflowParamFieldView::Select { id, value, .. }
                    | WorkflowParamFieldView::Text { id, value, .. } => {
                        let v = state
                            .edits
                            .get(&node.id)
                            .and_then(|m| m.get(id))
                            .cloned()
                            .unwrap_or_else(|| value.clone());
                        params.insert(id.clone(), v);
                    }
                    WorkflowParamFieldView::Number { id, value, .. } => {
                        let v = state
                            .edits
                            .get(&node.id)
                            .and_then(|m| m.get(id))
                            .cloned()
                            .unwrap_or_else(|| format!("{value}"));
                        params.insert(id.clone(), v);
                    }
                    WorkflowParamFieldView::FileKinds { .. } => {}
                }
            }
            state.edits.remove(&node.id);
            action = ProductionAction::WorkflowSaveParams {
                extension: extension_id.to_owned(),
                node_id: node.id.clone(),
                params,
            };
        }
    }

    ui.add_space(8.0);
    ui.separator();
    ui.label(RichText::new("输入").size(13.0).color(TEXT));
    if node.inputs.is_empty() {
        ui.label(RichText::new("（无）").size(11.0).color(DIM));
    } else {
        for port in &node.inputs {
            ui.label(RichText::new(format!("· {port}")).size(11.0).color(MUTED));
        }
    }
    ui.add_space(6.0);
    ui.label(RichText::new("输出").size(13.0).color(TEXT));
    if node.outputs.is_empty() {
        ui.label(RichText::new("（无）").size(11.0).color(DIM));
    } else {
        for port in &node.outputs {
            ui.label(RichText::new(format!("· {port}")).size(11.0).color(MUTED));
        }
    }
    action
}

fn screen_to_graph(screen: Pos2, canvas: Rect, state: &CanvasState) -> Pos2 {
    let local = screen - canvas.min - state.pan;
    Pos2::new(local.x / state.zoom, local.y / state.zoom)
}

fn graph_to_screen(graph: Pos2, canvas: Rect, state: &CanvasState) -> Pos2 {
    canvas.min + state.pan + Vec2::new(graph.x * state.zoom, graph.y * state.zoom)
}

fn draw_grid(painter: &egui::Painter, canvas: Rect, state: &CanvasState) {
    let step = 40.0_f32 * state.zoom;
    if step < 8.0 {
        return;
    }
    let origin = canvas.min + state.pan;
    let mut x = origin.x.rem_euclid(step) + canvas.left() - step;
    while x < canvas.right() {
        painter.line_segment(
            [Pos2::new(x, canvas.top()), Pos2::new(x, canvas.bottom())],
            Stroke::new(1.0_f32, BORDER.gamma_multiply(0.35)),
        );
        x += step;
    }
    let mut y = origin.y.rem_euclid(step) + canvas.top() - step;
    while y < canvas.bottom() {
        painter.line_segment(
            [Pos2::new(canvas.left(), y), Pos2::new(canvas.right(), y)],
            Stroke::new(1.0_f32, BORDER.gamma_multiply(0.35)),
        );
        y += step;
    }
}

fn draw_edge(painter: &egui::Painter, start: Pos2, end: Pos2, color: egui::Color32) {
    let dx = (end.x - start.x).abs().max(40.0) * 0.45;
    let c1 = Pos2::new(start.x + dx, start.y);
    let c2 = Pos2::new(end.x - dx, end.y);
    let mut prev = start;
    for i in 1..=16 {
        let t = i as f32 / 16.0;
        let u = 1.0 - t;
        let p = Pos2::new(
            u * u * u * start.x
                + 3.0 * u * u * t * c1.x
                + 3.0 * u * t * t * c2.x
                + t * t * t * end.x,
            u * u * u * start.y
                + 3.0 * u * u * t * c1.y
                + 3.0 * u * t * t * c2.y
                + t * t * t * end.y,
        );
        painter.line_segment([prev, p], Stroke::new(2.0_f32, color));
        prev = p;
    }
    let dir = (end - c2).normalized();
    let ortho = Vec2::new(-dir.y, dir.x) * 5.0;
    let tip = end;
    let back = end - dir * 10.0;
    painter.line_segment([tip, back + ortho], Stroke::new(2.0_f32, color));
    painter.line_segment([tip, back - ortho], Stroke::new(2.0_f32, color));
}

fn paint_node(
    painter: &egui::Painter,
    rect: Rect,
    node: &WorkflowNodeView,
    zoom: f32,
    selected: bool,
) {
    let color = if selected {
        BRAND
    } else {
        status_color(node.status)
    };
    painter.rect(
        rect,
        CornerRadius::same((8.0 * zoom).clamp(4.0, 10.0) as u8),
        PANEL,
        Stroke::new(
            if selected {
                (2.4 * zoom).clamp(1.5, 3.5)
            } else {
                (1.8 * zoom).clamp(1.0, 3.0)
            },
            color,
        ),
        egui::epaint::StrokeKind::Inside,
    );
    let pad = 10.0 * zoom;
    let mut y = rect.top() + pad;
    let font = |size: f32| FontId::proportional((size * zoom).clamp(9.0, 18.0));
    painter.text(
        Pos2::new(rect.left() + pad, y),
        egui::Align2::LEFT_TOP,
        &node.label,
        font(13.0),
        TEXT,
    );
    y += 18.0 * zoom;
    painter.text(
        Pos2::new(rect.left() + pad, y),
        egui::Align2::LEFT_TOP,
        &node.node_type,
        font(11.0),
        MUTED,
    );
    y += 16.0 * zoom;
    let status_label = match node.status {
        ProductionNodeStatus::Queued => "排队",
        ProductionNodeStatus::Running => "进行中",
        ProductionNodeStatus::Waiting => "等待",
        ProductionNodeStatus::Complete => "完成",
        ProductionNodeStatus::Failed => "失败",
        ProductionNodeStatus::Attention => "注意",
    };
    let status_text = match &node.detail {
        Some(d) => format!("{status_label} {d}"),
        None => status_label.into(),
    };
    painter.text(
        Pos2::new(rect.left() + pad, y),
        egui::Align2::LEFT_TOP,
        status_text,
        font(11.0),
        status_color(node.status),
    );
    y += 18.0 * zoom;
    if !node.inputs.is_empty() {
        painter.text(
            Pos2::new(rect.left() + pad, y),
            egui::Align2::LEFT_TOP,
            truncate(&format!("← {}", short_ports(&node.inputs)), 26),
            font(10.0),
            DIM,
        );
        y += 14.0 * zoom;
    }
    if !node.outputs.is_empty() {
        painter.text(
            Pos2::new(rect.left() + pad, y),
            egui::Align2::LEFT_TOP,
            truncate(&format!("→ {}", short_ports(&node.outputs)), 26),
            font(10.0),
            DIM,
        );
    }
    if !node.inputs.is_empty() {
        painter.circle_filled(
            Pos2::new(rect.left(), rect.top() + rect.height() * 0.35),
            4.0 * zoom,
            color,
        );
    }
    if !node.outputs.is_empty() {
        painter.circle_filled(
            Pos2::new(rect.right(), rect.top() + rect.height() * 0.55),
            4.0 * zoom,
            color,
        );
    }
}

fn short_ports(ports: &[String]) -> String {
    ports
        .iter()
        .map(|p| p.split(':').next().unwrap_or(p.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn truncate(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}
