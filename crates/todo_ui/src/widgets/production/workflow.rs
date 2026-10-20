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
const CARD_W: f32 = 220.0;
const CARD_H: f32 = 132.0;
const INSPECTOR_W: f32 = 340.0;
const PORT_HIT: f32 = 14.0;

#[derive(Clone)]
struct NodeClip {
    node_type: String,
    label: Option<String>,
    params_json: String,
}

#[derive(Clone)]
struct LinkDrag {
    from: String,
    out: String,
}

#[derive(Clone)]
struct EdgeHit {
    from: String,
    to: String,
    out: Option<String>,
    input: Option<String>,
}

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
    /// Right-click target: None = blank canvas.
    menu_node: Option<String>,
    menu_graph_pos: Option<Pos2>,
    menu_edge: Option<EdgeHit>,
    clipboard: Option<NodeClip>,
    linking: Option<LinkDrag>,
    link_cursor: Option<Pos2>,
    /// 本次运行的口播文案（空则用素材容器/脚本节点里的文本）。
    script: String,
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
            menu_node: None,
            menu_graph_pos: None,
            menu_edge: None,
            clipboard: None,
            linking: None,
            link_cursor: None,
            script: String::new(),
        }
    }
}

fn port_name(port: &str) -> &str {
    port.split(':').next().unwrap_or(port)
}

fn node_origin(node: &WorkflowNodeView, state: &CanvasState) -> Pos2 {
    state
        .positions
        .get(&node.id)
        .copied()
        .unwrap_or(Pos2::new(node.x, node.y))
}

/// Graph-space port centers for a node (outputs on the right, inputs on the left).
fn port_slots(node: &WorkflowNodeView, state: &CanvasState) -> Vec<(String, bool, Pos2)> {
    let origin = node_origin(node, state);
    let mut slots = Vec::new();
    let n_in = node.inputs.len().max(1);
    for (i, port) in node.inputs.iter().enumerate() {
        let t = (i + 1) as f32 / (n_in + 1) as f32;
        slots.push((
            port_name(port).to_owned(),
            false,
            Pos2::new(origin.x, origin.y + CARD_H * t),
        ));
    }
    let n_out = node.outputs.len().max(1);
    for (i, port) in node.outputs.iter().enumerate() {
        let t = (i + 1) as f32 / (n_out + 1) as f32;
        slots.push((
            port_name(port).to_owned(),
            true,
            Pos2::new(origin.x + CARD_W, origin.y + CARD_H * t),
        ));
    }
    slots
}

fn hit_port(
    graph_pos: Pos2,
    workflow: &WorkflowPanelView,
    state: &CanvasState,
    want_output: Option<bool>,
) -> Option<(String, String, bool)> {
    let radius = PORT_HIT / state.zoom;
    let mut best: Option<(f32, String, String, bool)> = None;
    for node in &workflow.nodes {
        for (name, is_out, pos) in port_slots(node, state) {
            if want_output.is_some_and(|w| w != is_out) {
                continue;
            }
            if !is_out && node.inputs.is_empty() {
                continue;
            }
            if is_out && node.outputs.is_empty() {
                continue;
            }
            let d = pos.distance(graph_pos);
            if d <= radius {
                if best.as_ref().is_none_or(|(bd, ..)| d < *bd) {
                    best = Some((d, node.id.clone(), name, is_out));
                }
            }
        }
    }
    best.map(|(_, id, name, is_out)| (id, name, is_out))
}

fn hit_edge(
    screen: Pos2,
    canvas: Rect,
    workflow: &WorkflowPanelView,
    state: &CanvasState,
) -> Option<EdgeHit> {
    let mut best: Option<(f32, EdgeHit)> = None;
    for edge in &workflow.edges {
        let Some(from) = workflow.nodes.iter().find(|n| n.id == edge.from) else {
            continue;
        };
        let Some(to) = workflow.nodes.iter().find(|n| n.id == edge.to) else {
            continue;
        };
        let start = edge_endpoint(from, edge.out.as_deref(), true, canvas, state);
        let end = edge_endpoint(to, edge.input.as_deref(), false, canvas, state);
        let d = bezier_distance(screen, start, end);
        if d < 8.0 {
            if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                best = Some((
                    d,
                    EdgeHit {
                        from: edge.from.clone(),
                        to: edge.to.clone(),
                        out: edge.out.clone(),
                        input: edge.input.clone(),
                    },
                ));
            }
        }
    }
    best.map(|(_, e)| e)
}

fn edge_endpoint(
    node: &WorkflowNodeView,
    port: Option<&str>,
    output: bool,
    canvas: Rect,
    state: &CanvasState,
) -> Pos2 {
    let slots = port_slots(node, state);
    let graph = slots
        .into_iter()
        .find(|(name, is_out, _)| *is_out == output && port.is_none_or(|p| name == p))
        .map(|(_, _, p)| p)
        .unwrap_or_else(|| {
            let o = node_origin(node, state);
            if output {
                Pos2::new(o.x + CARD_W, o.y + CARD_H * 0.5)
            } else {
                Pos2::new(o.x, o.y + CARD_H * 0.5)
            }
        });
    graph_to_screen(graph, canvas, state)
}

fn bezier_distance(p: Pos2, start: Pos2, end: Pos2) -> f32 {
    let dx = (end.x - start.x).abs().max(40.0) * 0.45;
    let c1 = Pos2::new(start.x + dx, start.y);
    let c2 = Pos2::new(end.x - dx, end.y);
    let mut best = f32::MAX;
    let mut prev = start;
    for i in 1..=24 {
        let t = i as f32 / 24.0;
        let u = 1.0 - t;
        let q = Pos2::new(
            u * u * u * start.x
                + 3.0 * u * u * t * c1.x
                + 3.0 * u * t * t * c2.x
                + t * t * t * end.x,
            u * u * u * start.y
                + 3.0 * u * u * t * c1.y
                + 3.0 * u * t * t * c2.y
                + t * t * t * end.y,
        );
        // Point-to-segment distance.
        let ab = q - prev;
        let t = ((p - prev).dot(ab) / ab.length_sq().max(1e-4)).clamp(0.0, 1.0);
        let proj = prev + ab * t;
        best = best.min(p.distance(proj));
        prev = q;
    }
    best
}

fn snapshot_positions(state: &CanvasState) -> BTreeMap<String, (f32, f32)> {
    state
        .positions
        .iter()
        .map(|(id, p)| (id.clone(), (p.x, p.y)))
        .collect()
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
                            let state_key = canvas_id(extension_id);
                            let mut header_state = ui
                                .ctx()
                                .data(|d| d.get_temp::<CanvasState>(state_key))
                                .unwrap_or_default();
                            let script_response = ui.add(
                                egui::TextEdit::singleline(&mut header_state.script)
                                    .desired_width(260.0)
                                    .hint_text("本次口播文案（可留空）"),
                            );
                            if script_response.changed() {
                                ui.ctx()
                                    .data_mut(|d| d.insert_temp(state_key, header_state.clone()));
                            }
                            if ui
                                .button(RichText::new("运行").size(12.5))
                                .on_hover_text("逐节点执行：本地节点立刻走，服务节点等结果回来")
                                .clicked()
                            {
                                action = ProductionAction::WorkflowRun {
                                    extension: extension_id.to_owned(),
                                    script: header_state.script.clone(),
                                };
                            }
                            if let Some(progress) = &workflow.running {
                                ui.label(
                                    RichText::new(progress).size(11.5).color(BRAND),
                                );
                                ui.ctx().request_repaint_after(
                                    std::time::Duration::from_millis(250),
                                );
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
                                RichText::new("端口拖拽连线 · 右键断线/加节点 · Ctrl+C/V")
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

                    let avail = ui.available_rect_before_wrap();
                    ui.allocate_rect(avail, Sense::hover());
                    let gap = 8.0_f32;
                    let show_inspector = state.selected.is_some();
                    let iw = if show_inspector { INSPECTOR_W } else { 0.0 };
                    let canvas = Rect::from_min_size(
                        avail.min,
                        Vec2::new(
                            (avail.width() - iw - if show_inspector { gap } else { 0.0 }).max(200.0),
                            avail.height(),
                        ),
                    );
                    let response = ui.interact(
                        canvas,
                        Id::new(("wf_canvas_hit", extension_id)),
                        Sense::click_and_drag(),
                    );
                    let before = state.selected.clone();
                    let canvas_action =
                        interact_canvas(ui, canvas, &response, workflow, extension_id, &mut state);
                    if canvas_action != ProductionAction::None {
                        action = canvas_action;
                    }
                    paint_canvas(ui, canvas, workflow, &state);
                    if state.selected != before {
                        ui.ctx().request_repaint();
                    }

                    if let Some(sel) = state.selected.clone() {
                        if let Some(node) = workflow.nodes.iter().find(|n| n.id == sel) {
                            let irect = Rect::from_min_max(
                                Pos2::new(canvas.max.x + gap, avail.min.y),
                                avail.max,
                            );
                            // Same-frame select: canvas still full-width; draw overlay strip.
                            let irect = if show_inspector {
                                irect
                            } else {
                                Rect::from_min_max(
                                    Pos2::new((avail.max.x - INSPECTOR_W).max(avail.min.x), avail.min.y),
                                    avail.max,
                                )
                            };
                            let mut child = ui.new_child(
                                egui::UiBuilder::new()
                                    .id_salt(("wf_inspector", extension_id))
                                    .max_rect(irect),
                            );
                            child.set_clip_rect(irect);
                            egui::Frame::NONE
                                .fill(PANEL_2)
                                .stroke(Stroke::new(1.0_f32, BORDER))
                                .inner_margin(12.0)
                                .corner_radius(CornerRadius::same(8))
                                .show(&mut child, |ui| {
                                    ui.set_min_size(irect.size() - Vec2::splat(4.0));
                                    let next =
                                        draw_inspector(ui, node, extension_id, &mut state);
                                    if next != ProductionAction::None {
                                        action = next;
                                    }
                                });
                        }
                    }

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
    extension_id: &str,
    state: &mut CanvasState,
) -> ProductionAction {
    let mut action = ProductionAction::None;
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
        state.linking = None;
        state.link_cursor = None;
        if let Some(pointer) = response.interact_pointer_pos() {
            let graph_pos = screen_to_graph(pointer, canvas, state);
            if let Some((id, name, true)) = hit_port(graph_pos, workflow, state, Some(true)) {
                state.linking = Some(LinkDrag {
                    from: id,
                    out: name,
                });
                state.link_cursor = Some(pointer);
                state.dragging = None;
            } else {
                state.dragging = hit(graph_pos, state);
            }
        }
    }

    let linking = state.linking.is_some();
    let on_node = state.dragging.is_some();
    if response.dragged() && !on_node && !linking {
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
    if linking {
        if let Some(pointer) = response.interact_pointer_pos() {
            state.link_cursor = Some(pointer);
        }
    }

    if response.drag_stopped() {
        let click = state
            .press_screen
            .zip(response.interact_pointer_pos())
            .is_some_and(|(a, b)| a.distance(b) < 5.0);
        if let Some(link) = state.linking.take() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let graph_pos = screen_to_graph(pointer, canvas, state);
                if let Some((to_id, in_name, false)) =
                    hit_port(graph_pos, workflow, state, Some(false))
                {
                    if to_id != link.from {
                        action = ProductionAction::WorkflowConnect {
                            extension: extension_id.to_owned(),
                            from: link.from,
                            to: to_id,
                            out: Some(link.out),
                            input: Some(in_name),
                            positions: snapshot_positions(state),
                        };
                    }
                }
            }
            state.link_cursor = None;
        } else if click {
            if let Some(pointer) = response.interact_pointer_pos() {
                let graph_pos = screen_to_graph(pointer, canvas, state);
                state.selected = hit(graph_pos, state);
            }
        }
        state.dragging = None;
        state.press_screen = None;
    }

    if response.clicked() && !response.dragged() && state.linking.is_none() {
        if let Some(pointer) = response.interact_pointer_pos() {
            let graph_pos = screen_to_graph(pointer, canvas, state);
            state.selected = hit(graph_pos, state);
        }
    }

    if response.secondary_clicked() {
        if let Some(pointer) = response.interact_pointer_pos() {
            let graph_pos = screen_to_graph(pointer, canvas, state);
            state.menu_edge = hit_edge(pointer, canvas, workflow, state);
            state.menu_node = if state.menu_edge.is_some() {
                None
            } else {
                hit(graph_pos, state)
            };
            state.menu_graph_pos = Some(graph_pos);
            if let Some(id) = state.menu_node.clone() {
                state.selected = Some(id);
            }
        }
    }

    response.context_menu(|ui| {
        let positions = snapshot_positions(state);
        let paste_at = state
            .menu_graph_pos
            .unwrap_or(Pos2::new(120.0, 120.0));
        if let Some(edge) = state.menu_edge.clone() {
            ui.label(RichText::new("连线").size(12.0).color(MUTED));
            if ui.button("断开连线").clicked() {
                action = ProductionAction::WorkflowDisconnect {
                    extension: extension_id.to_owned(),
                    from: edge.from,
                    to: edge.to,
                    out: edge.out,
                    input: edge.input,
                    positions,
                };
                ui.close_menu();
            }
        } else if let Some(node_id) = state.menu_node.clone() {
            let node = workflow.nodes.iter().find(|n| n.id == node_id);
            if ui.button("复制").clicked() {
                if let Some(n) = node {
                    state.clipboard = Some(NodeClip {
                        node_type: n.node_type.clone(),
                        label: Some(n.label.clone()),
                        params_json: n.params_json.clone(),
                    });
                }
                ui.close_menu();
            }
            if ui.button("粘贴副本").clicked() {
                action = ProductionAction::WorkflowDuplicateNode {
                    extension: extension_id.to_owned(),
                    node_id: node_id.clone(),
                    positions: positions.clone(),
                };
                ui.close_menu();
            }
            if ui.button("删除").clicked() {
                action = ProductionAction::WorkflowDeleteNode {
                    extension: extension_id.to_owned(),
                    node_id,
                    positions,
                };
                ui.close_menu();
            }
        } else {
            ui.label(RichText::new("添加节点").size(12.0).color(MUTED));
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for choice in &workflow.palette {
                        if ui.button(&choice.label).on_hover_text(&choice.type_name).clicked()
                        {
                            action = ProductionAction::WorkflowAddNode {
                                extension: extension_id.to_owned(),
                                node_type: choice.type_name.clone(),
                                x: paste_at.x,
                                y: paste_at.y,
                                positions: positions.clone(),
                            };
                            ui.close_menu();
                        }
                    }
                });
            if state.clipboard.is_some() {
                ui.separator();
                if ui.button("粘贴").clicked() {
                    if let Some(clip) = state.clipboard.clone() {
                        action = ProductionAction::WorkflowPasteNode {
                            extension: extension_id.to_owned(),
                            node_type: clip.node_type,
                            label: clip.label,
                            params_json: clip.params_json,
                            x: paste_at.x,
                            y: paste_at.y,
                            positions,
                        };
                    }
                    ui.close_menu();
                }
            }
        }
    });

    // Keyboard: Ctrl+C / Ctrl+V / Delete (skip when typing).
    if !ui.ctx().wants_keyboard_input() {
        let (copy, paste, delete) = ui.input(|i| {
            let cmd = i.modifiers.command || i.modifiers.ctrl;
            (
                cmd && i.key_pressed(egui::Key::C),
                cmd && i.key_pressed(egui::Key::V),
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
            )
        });
        if copy {
            if let Some(id) = state.selected.clone() {
                if let Some(n) = workflow.nodes.iter().find(|n| n.id == id) {
                    state.clipboard = Some(NodeClip {
                        node_type: n.node_type.clone(),
                        label: Some(n.label.clone()),
                        params_json: n.params_json.clone(),
                    });
                }
            }
        }
        if paste {
            if let Some(clip) = state.clipboard.clone() {
                let at = state
                    .menu_graph_pos
                    .or_else(|| {
                        state.selected.as_ref().and_then(|id| {
                            state.positions.get(id).map(|p| Pos2::new(p.x + 48.0, p.y + 48.0))
                        })
                    })
                    .unwrap_or(Pos2::new(160.0, 160.0));
                action = ProductionAction::WorkflowPasteNode {
                    extension: extension_id.to_owned(),
                    node_type: clip.node_type,
                    label: clip.label,
                    params_json: clip.params_json,
                    x: at.x,
                    y: at.y,
                    positions: snapshot_positions(state),
                };
            }
        }
        if delete {
            if let Some(id) = state.selected.clone() {
                action = ProductionAction::WorkflowDeleteNode {
                    extension: extension_id.to_owned(),
                    node_id: id,
                    positions: snapshot_positions(state),
                };
            }
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
    action
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

    let hover_port = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|p| canvas.contains(*p))
        .map(|p| screen_to_graph(p, canvas, state))
        .and_then(|g| hit_port(g, workflow, state, None));

    for edge in &workflow.edges {
        let Some(from) = workflow.nodes.iter().find(|n| n.id == edge.from) else {
            continue;
        };
        let Some(to) = workflow.nodes.iter().find(|n| n.id == edge.to) else {
            continue;
        };
        let start = edge_endpoint(from, edge.out.as_deref(), true, canvas, state);
        let end = edge_endpoint(to, edge.input.as_deref(), false, canvas, state);
        let hot = state.menu_edge.as_ref().is_some_and(|e| {
            e.from == edge.from
                && e.to == edge.to
                && e.out == edge.out
                && e.input == edge.input
        });
        draw_edge(&painter, start, end, if hot { BRAND } else { MUTED });
    }

    if let (Some(link), Some(cursor)) = (&state.linking, state.link_cursor) {
        if let Some(from) = workflow.nodes.iter().find(|n| n.id == link.from) {
            let start = edge_endpoint(from, Some(&link.out), true, canvas, state);
            draw_edge(&painter, start, cursor, BRAND);
        }
    }

    for node in &workflow.nodes {
        let origin = node_origin(node, state);
        let screen_min = graph_to_screen(origin, canvas, state);
        let screen_max = graph_to_screen(origin + Vec2::new(CARD_W, CARD_H), canvas, state);
        let rect = Rect::from_min_max(screen_min, screen_max);
        if rect.intersects(canvas) {
            let selected = state.selected.as_deref() == Some(node.id.as_str());
            paint_node(&painter, rect, node, state.zoom, selected);
            for (name, is_out, graph) in port_slots(node, state) {
                if (!is_out && node.inputs.is_empty()) || (is_out && node.outputs.is_empty()) {
                    continue;
                }
                let screen = graph_to_screen(graph, canvas, state);
                let hot = hover_port
                    .as_ref()
                    .is_some_and(|(id, n, o)| id == &node.id && n == &name && *o == is_out)
                    || state
                        .linking
                        .as_ref()
                        .is_some_and(|l| is_out && l.from == node.id && l.out == name);
                let r = (if hot { 6.0 } else { 4.5 }) * state.zoom;
                painter.circle_filled(screen, r, if hot { BRAND } else { MUTED });
                painter.circle_stroke(
                    screen,
                    r,
                    Stroke::new(1.0_f32, if hot { TEXT } else { BORDER }),
                );
                // Prefer short type badge; fall back to port name.
                let kind = port_kind_label(node, &name, is_out);
                let label = if kind.is_empty() {
                    name.clone()
                } else {
                    kind
                };
                let font = FontId::proportional((10.0 * state.zoom).clamp(8.0, 13.0));
                if is_out {
                    painter.text(
                        Pos2::new(screen.x - 6.0 * state.zoom, screen.y),
                        egui::Align2::RIGHT_CENTER,
                        label,
                        font,
                        if hot { TEXT } else { MUTED },
                    );
                } else {
                    painter.text(
                        Pos2::new(screen.x + 6.0 * state.zoom, screen.y),
                        egui::Align2::LEFT_CENTER,
                        label,
                        font,
                        if hot { TEXT } else { MUTED },
                    );
                }
            }
        }
    }
}

fn port_kind_label(node: &WorkflowNodeView, name: &str, is_out: bool) -> String {
    let list = if is_out { &node.outputs } else { &node.inputs };
    list.iter()
        .find(|p| port_name(p) == name)
        .and_then(|p| p.split_once(':').map(|(_, k)| k.to_owned()))
        .unwrap_or_default()
}

fn draw_inspector(
    ui: &mut Ui,
    node: &WorkflowNodeView,
    extension_id: &str,
    state: &mut CanvasState,
) -> ProductionAction {
    let mut action = ProductionAction::None;
    egui::ScrollArea::vertical()
        .id_salt(("wf_insp_scroll", &node.id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&node.label).size(16.0).color(TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("×").size(14.0).color(MUTED))
                                .frame(false),
                        )
                        .clicked()
                    {
                        state.selected = None;
                    }
                });
            });
            ui.label(RichText::new(&node.node_type).size(12.0).color(BRAND));
            if !node.summary.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(&node.summary).size(12.0).color(MUTED));
            }
            if node.status == ProductionNodeStatus::Failed {
                if let Some(detail) = &node.detail {
                    ui.add_space(6.0);
                    egui::Frame::NONE
                        .stroke(Stroke::new(1.0_f32, status_color(node.status)))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(8)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(detail)
                                    .size(11.0)
                                    .color(status_color(node.status)),
                            );
                        });
                }
            }

            ui.add_space(8.0);
            egui::CollapsingHeader::new(RichText::new("端口").size(12.5).color(MUTED))
                .id_salt(("wf_ports", &node.id))
                .default_open(false)
                .show(ui, |ui| {
                    if node.inputs.is_empty() && node.outputs.is_empty() {
                        ui.label(RichText::new("无端口").size(11.0).color(DIM));
                        return;
                    }
                    for port in &node.inputs {
                        port_row(ui, port, true);
                    }
                    for port in &node.outputs {
                        port_row(ui, port, false);
                    }
                });

            if node.output.is_some() || node.image.is_some() {
                ui.add_space(8.0);
                egui::CollapsingHeader::new(RichText::new("最近产物").size(12.5).color(MUTED))
                    .id_salt(("wf_art", &node.id))
                    .default_open(true)
                    .show(ui, |ui| {
                if let Some(path) = &node.image {
                    if let Some(texture) = preview_texture(ui.ctx(), path) {
                        let width = ui.available_width().min(300.0);
                        let size = texture.size_vec2();
                        let scale = (width / size.x).min(1.0);
                        ui.image((texture.id(), size * scale));
                    }
                    ui.label(RichText::new(path).size(10.0).color(DIM));
                }
                if let Some(output) = &node.output {
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .id_salt(("wf_out", &node.id))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(output)
                                    .size(10.5)
                                    .monospace()
                                    .color(MUTED),
                            );
                        });
                }
                    });
            }

            ui.add_space(10.0);
            ui.separator();
            ui.label(RichText::new("参数").strong().size(13.0).color(TEXT));
            ui.add_space(6.0);

            if node.fields.is_empty() && node.files.is_empty() {
                ui.label(
                    RichText::new("此节点暂无可调参数（端口见上）")
                        .size(11.0)
                        .color(DIM),
                );
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
                            ui.label(RichText::new(label).size(12.0).color(TEXT))
                                .on_hover_text(hint.as_str());
                            let current = edits.get(id).cloned().unwrap_or_else(|| value.clone());
                            let mut picked = current.clone();
                            egui::ComboBox::from_id_salt(("wf_sel", &node.id, id))
                                .selected_text(
                                    options
                                        .iter()
                                        .find(|(v, _)| *v == current)
                                        .map(|(_, l)| l.as_str())
                                        .unwrap_or(&current),
                                )
                                .width((ui.available_width() - 8.0).max(80.0))
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
                            ui.label(RichText::new(label).size(12.0).color(TEXT))
                                .on_hover_text(hint.as_str());
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
                            ui.add_space(8.0);
                        }
                        WorkflowParamFieldView::Text {
                            id,
                            label,
                            hint,
                            placeholder,
                            value,
                        } => {
                            ui.label(RichText::new(label).size(12.0).color(TEXT))
                                .on_hover_text(hint.as_str());
                            let mut text = edits.get(id).cloned().unwrap_or_else(|| value.clone());
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut text)
                                    .hint_text(placeholder.as_str())
                                    .desired_width((ui.available_width() - 8.0).max(80.0)),
                            );
                            if resp.changed() {
                                edits.insert(id.clone(), text);
                            }
                            ui.add_space(8.0);
                        }
                        WorkflowParamFieldView::FileKinds {
                            label,
                            hint,
                            accept,
                            ..
                        } => {
                            ui.label(RichText::new(label).size(12.0).color(TEXT))
                                .on_hover_text(hint.as_str());
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
                            ui.add_space(6.0);
                            if node.files.is_empty() {
                                ui.label(RichText::new("尚未添加文件").size(11.0).color(DIM));
                            } else {
                                for file in &node.files {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(format!(
                                                "[{}] {}",
                                                file.kind, file.name
                                            ))
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

            let has_scalar = node
                .fields
                .iter()
                .any(|f| !matches!(f, WorkflowParamFieldView::FileKinds { .. }));
            if has_scalar {
                ui.add_space(4.0);
                if ui
                    .add(egui::Button::new(RichText::new("保存参数").size(12.5)))
                    .on_hover_text("把当前参数写回 graph.json（也可用顶栏「保存工作流」）")
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

            if !node.docs.is_empty() {
                ui.add_space(6.0);
                egui::CollapsingHeader::new(RichText::new("说明").size(12.0).color(DIM))
                    .id_salt(("wf_docs", &node.id))
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.label(RichText::new(&node.docs).size(11.0).color(DIM));
                    });
            }
        });
    action
}

fn port_row(ui: &mut Ui, port: &str, input: bool) {
    let (name, kind) = port
        .split_once(':')
        .map(|(n, k)| (n, k))
        .unwrap_or((port, ""));
    let badge = if input { "输入" } else { "输出" };
    ui.horizontal(|ui| {
        ui.label(RichText::new(badge).size(10.0).color(BRAND));
        ui.label(RichText::new(name).strong().size(12.0).color(TEXT));
        if !kind.is_empty() {
            ui.label(RichText::new(kind).size(11.0).color(MUTED));
        }
    });
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
        // 卡片只放一行摘要，完整错误在检查器里看
        Some(d) if node.status != ProductionNodeStatus::Failed => {
            format!("{status_label} {d}")
        }
        Some(d) => {
            let short: String = d.chars().take(18).collect();
            format!("{status_label} {short}…")
        }
        None => status_label.into(),
    };
    painter.text(
        Pos2::new(rect.left() + pad, y),
        egui::Align2::LEFT_TOP,
        status_text,
        font(11.0),
        status_color(node.status),
    );
}

/// Decode a produced image once per path and keep it as an egui texture.
fn preview_texture(ctx: &egui::Context, path: &str) -> Option<egui::TextureHandle> {
    let key = egui::Id::new(("wf_preview", path));
    ctx.data_mut(|data| {
        if let Some(handle) = data.get_temp::<egui::TextureHandle>(key) {
            return Some(handle);
        }
        let image = image::open(path).ok()?.to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let handle = ctx.load_texture(path, color, egui::TextureOptions::LINEAR);
        data.insert_temp(key, handle.clone());
        Some(handle)
    })
}
