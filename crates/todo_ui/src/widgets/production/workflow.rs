//! Fullscreen free-canvas workflow editor (`~` toggle).
use super::status_color;
use super::{selected_workflow, set_selected_workflow, ProductionAction};
use crate::theme::{
    button, with_input_radius, Btn, BORDER, BRAND, DIM, MUTED, PANEL, PANEL_2, TEXT,
};
use egui::{Color32, CornerRadius, FontId, Id, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use std::collections::BTreeMap;
use crate::thumbs::ThumbCache;
use todo_interaction::{
    ProductionNodeStatus, ProductionPanelView, WorkflowNodeView, WorkflowPanelView,
    WorkflowParamFieldView,
};

const OPEN_ID: &str = "todo_ui_workflow_panel_open";
// Keep nodes roomy enough for readable labels and a clear status row. At
// zoomed-out levels the card collapses to a status tile instead of shrinking
// text into an unreadable thumbnail.
const CARD_W: f32 = 152.0;
const TITLE_H: f32 = 30.0;
const PORT_GAP: f32 = 20.0;

fn card_h(node: &WorkflowNodeView) -> f32 {
    let ports = node.inputs.len().max(node.outputs.len()).max(1);
    (TITLE_H + 6.0 + ports as f32 * PORT_GAP).max(44.0)
}

fn card_size(node: &WorkflowNodeView) -> Vec2 {
    Vec2::new(CARD_W, card_h(node))
}
/// Bottom dock for the live run.log (egui text — no native cmd HWND).
const TERMINAL_H: f32 = 200.0;
const TERMINAL_HOLE_ID: &str = "todo_ui_workflow_terminal_hole";
const TERMINAL_BG: Color32 = Color32::from_rgb(12, 12, 14);
const TERMINAL_FG: Color32 = Color32::from_rgb(0xb8, 0xc0, 0xc8);
const INSPECTOR_W: f32 = 340.0;
const PORT_HIT: f32 = 14.0;
const CANVAS_TOOLBAR_H: f32 = 36.0;

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
    /// Toolbar「导入文件…」clicked; consumed next frame with extension_id.
    pending_import_pick: bool,
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
            pending_import_pick: false,
        }
    }
}

fn port_name(port: &str) -> &str {
    port.split(':').next().unwrap_or(port)
}

fn draw_add_node_menu(
    ui: &mut Ui,
    workflow: &WorkflowPanelView,
    extension_id: &str,
    paste_at: Pos2,
    positions: &BTreeMap<String, (f32, f32)>,
    action: &mut ProductionAction,
) {
    ui.label(RichText::new("添加节点").size(12.0).color(MUTED));
    ui.separator();

    if workflow.palette.is_empty() {
        ui.label(RichText::new("没有可添加的节点").size(12.0).color(DIM));
        return;
    }

    let groups: Vec<(String, Vec<usize>)> = {
        let mut out: Vec<(String, Vec<usize>)> = Vec::new();
        for (i, choice) in workflow.palette.iter().enumerate() {
            match out.last_mut() {
                Some((name, idxs)) if *name == choice.group => idxs.push(i),
                _ => out.push((choice.group.clone(), vec![i])),
            }
        }
        out
    };

    for (group, idxs) in &groups {
        ui.menu_button(RichText::new(group).size(13.5), |ui| {
            ui.set_min_width(168.0);
            let need_scroll = idxs.len() > 10;
            let mut pick = |ui: &mut Ui| {
                for &i in idxs {
                    let choice = &workflow.palette[i];
                    if ui
                        .add(
                            egui::Button::new(RichText::new(&choice.label).size(13.5))
                                .min_size(Vec2::new(160.0, 0.0)),
                        )
                        .on_hover_text(&choice.type_name)
                        .clicked()
                    {
                        *action = ProductionAction::WorkflowAddNode {
                            extension: extension_id.to_owned(),
                            node_type: choice.type_name.clone(),
                            x: paste_at.x,
                            y: paste_at.y,
                            positions: positions.clone(),
                        };
                        ui.close_menu();
                    }
                }
            };
            if need_scroll {
                egui::ScrollArea::vertical()
                    .max_height(280.0)
                    .show(ui, |ui| pick(ui));
            } else {
                pick(ui);
            }
        });
    }
}

fn status_label(status: ProductionNodeStatus) -> &'static str {
    match status {
        ProductionNodeStatus::Queued => "排队",
        ProductionNodeStatus::Running => "运行中",
        ProductionNodeStatus::Waiting => "等待",
        ProductionNodeStatus::Complete => "完成",
        ProductionNodeStatus::Failed => "失败",
        ProductionNodeStatus::Attention => "需处理",
    }
}

fn short_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let short: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
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
    let h = card_h(node);
    let body_top = TITLE_H;
    let body_h = (h - body_top).max(1.0);
    let mut slots = Vec::new();
    let n_in = node.inputs.len().max(1);
    for (i, port) in node.inputs.iter().enumerate() {
        let t = (i + 1) as f32 / (n_in + 1) as f32;
        slots.push((
            port_name(port).to_owned(),
            false,
            Pos2::new(origin.x, origin.y + body_top + body_h * t),
        ));
    }
    let n_out = node.outputs.len().max(1);
    for (i, port) in node.outputs.iter().enumerate() {
        let t = (i + 1) as f32 / (n_out + 1) as f32;
        slots.push((
            port_name(port).to_owned(),
            true,
            Pos2::new(origin.x + CARD_W, origin.y + body_top + body_h * t),
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
            let mid_y = o.y + card_h(node) * 0.5;
            if output {
                Pos2::new(o.x + CARD_W, mid_y)
            } else {
                Pos2::new(o.x, mid_y)
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
    set_terminal_hole(ctx, None);
    ctx.data_mut(|data| data.insert_temp(Id::new(OPEN_ID), None::<String>));
}

pub fn open_id(ctx: &egui::Context) -> Option<String> {
    ctx.data(|data| data.get_temp::<Option<String>>(Id::new(OPEN_ID)))
        .flatten()
}

pub fn is_open(ctx: &egui::Context) -> bool {
    open_id(ctx).is_some()
}

/// Layout rect of the bottom log panel (tests / host may still query it).
pub fn terminal_hole(ctx: &egui::Context) -> Option<egui::Rect> {
    ctx.data(|data| data.get_temp::<Option<egui::Rect>>(Id::new(TERMINAL_HOLE_ID)))
        .flatten()
}

fn set_terminal_hole(ctx: &egui::Context, hole: Option<egui::Rect>) {
    ctx.data_mut(|data| data.insert_temp(Id::new(TERMINAL_HOLE_ID), hole));
}

fn draw_terminal(ui: &mut Ui, hole: Rect, workflow: &WorkflowPanelView) {
    ui.painter()
        .rect_filled(hole, CornerRadius::ZERO, TERMINAL_BG);
    ui.painter().hline(
        hole.x_range(),
        hole.min.y,
        Stroke::new(1.0_f32, Color32::from_rgb(0x2a, 0x2e, 0x36)),
    );
    let inner = hole.shrink2(Vec2::new(10.0, 8.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    );
    child.set_clip_rect(inner);
    let text = workflow.log_tail.as_deref().unwrap_or(
        "工作流终端 · 运行后这里会实时显示节点日志",
    );
    egui::ScrollArea::vertical()
        .id_salt("wf_term_scroll")
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            ui.colored_label(
                TERMINAL_FG,
                RichText::new(text).font(FontId::monospace(12.5)),
            );
            if workflow.running.is_some() {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(120));
            }
        });
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
        set_terminal_hole(ctx, None);
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

/// Fullscreen shell when the package has no editable graph (still blocks the editor).
pub fn draw_unavailable(ctx: &egui::Context, extension_id: &str, title: &str) -> ProductionAction {
    let screen = ctx.screen_rect();
    egui::Area::new(Id::new(("todo_ui_workflow_unavailable", extension_id)))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .interactable(true)
        .sense(Sense::hover())
        .show(ctx, |ui| {
            ui.set_min_size(screen.size());
            ui.set_max_size(screen.size());
            const CHROME_H: f32 = 44.0;
            let body = Rect::from_min_max(
                Pos2::new(screen.min.x, screen.min.y + CHROME_H),
                screen.max,
            );
            ui.interact(body, ui.id().with("background"), Sense::click());
            let chrome = Rect::from_min_size(screen.min, Vec2::new(screen.width(), CHROME_H));
            super::super::chrome::wire_toolbar(ui, chrome);
            egui::Frame::NONE
                .fill(PANEL)
                .inner_margin(egui::Margin::same(20))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(title).strong().size(16.0).color(TEXT));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if button(ui, "关闭", Btn::Ghost, true).clicked() {
                                close(ui.ctx());
                            }
                        });
                    });
                    ui.add_space(48.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("无法打开此工作流").size(16.0).color(TEXT));
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("这个工作流暂时打不开画布。")
                                .size(13.0)
                                .color(MUTED),
                        );
                    });
                });
        });
    ProductionAction::None
}

pub fn draw_fullscreen(
    ctx: &egui::Context,
    extension_id: &str,
    title: &str,
    workflow: &WorkflowPanelView,
    thumbs: &mut ThumbCache,
) -> ProductionAction {
    let mut action = ProductionAction::None;
    let screen = ctx.screen_rect();
    egui::Area::new(Id::new("todo_ui_workflow_fullscreen"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .interactable(true)
        .sense(Sense::hover())
        .show(ctx, |ui| {
            // Own the full screen so editor panes underneath never receive clicks.
            ui.set_min_size(screen.size());
            ui.set_max_size(screen.size());
            // Keep the top chrome free for window drag; only block the body.
            const CHROME_H: f32 = 44.0;
            let body = Rect::from_min_max(
                Pos2::new(screen.min.x, screen.min.y + CHROME_H),
                screen.max,
            );
            ui.interact(body, ui.id().with("background"), Sense::click());
            let chrome = Rect::from_min_size(screen.min, Vec2::new(screen.width(), CHROME_H));
            super::super::chrome::wire_toolbar(ui, chrome);
            egui::Frame::NONE
                .fill(PANEL)
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    let state_key = canvas_id(extension_id);
                    let mut header_state = ui
                        .ctx()
                        .data(|d| d.get_temp::<CanvasState>(state_key))
                        .unwrap_or_default();
                    ui.horizontal(|ui| {
                        let title_galley = ui.fonts(|f| {
                            f.layout_no_wrap(
                                title.to_owned(),
                                egui::FontId::proportional(16.0),
                                TEXT,
                            )
                        });
                        let title_size = title_galley.size() + Vec2::new(8.0, 4.0);
                        let (title_rect, title_drag) = ui.allocate_exact_size(
                            title_size,
                            Sense::click_and_drag(),
                        );
                        ui.painter().galley(
                            title_rect.min + Vec2::new(0.0, 2.0),
                            title_galley,
                            TEXT,
                        );
                        if title_drag.drag_started_by(egui::PointerButton::Primary) {
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::StartDrag);
                        }
                        if let Some(progress) = &workflow.running {
                            ui.label(RichText::new(progress).size(12.5).color(BRAND));
                            ui.ctx()
                                .request_repaint_after(std::time::Duration::from_millis(250));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if button(ui, "关闭", Btn::Ghost, true).clicked() {
                                close(ui.ctx());
                            }
                            if button(ui, "保存", Btn::Normal, true).clicked() {
                                let positions = header_state
                                    .positions
                                    .iter()
                                    .map(|(id, p)| (id.clone(), (p.x, p.y)))
                                    .collect();
                                action = ProductionAction::WorkflowSave {
                                    extension: extension_id.to_owned(),
                                    positions,
                                    edits: header_state.edits.clone(),
                                };
                            }
                            if workflow.running.is_some()
                                && button(ui, "停止", Btn::Danger, true).clicked()
                            {
                                action = ProductionAction::WorkflowStop {
                                    extension: extension_id.to_owned(),
                                };
                            }
                            if button(ui, "复位", Btn::Ghost, true).clicked() {
                                ui.ctx()
                                    .data_mut(|d| d.insert_temp(state_key, CanvasState::default()));
                                header_state = CanvasState::default();
                            }
                        });
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let script_response = with_input_radius(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut header_state.script)
                                    .desired_width(280.0)
                                    .hint_text("口播文案（可选）")
                                    .margin(egui::Margin::symmetric(10, 6)),
                            )
                        });
                        if script_response.changed() {
                            ui.ctx()
                                .data_mut(|d| d.insert_temp(state_key, header_state.clone()));
                        }
                        if button(ui, "运行", Btn::Primary, true).clicked() {
                            action = ProductionAction::WorkflowRun {
                                extension: extension_id.to_owned(),
                                script: header_state.script.clone(),
                                edits: header_state.edits.clone(),
                            };
                        }
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
                    let term_h = TERMINAL_H.min(avail.height() * 0.38).max(120.0);
                    let work = Rect::from_min_max(
                        avail.min,
                        Pos2::new(avail.max.x, (avail.max.y - term_h - gap).max(avail.min.y + 120.0)),
                    );
                    let term_hole = Rect::from_min_max(
                        Pos2::new(avail.min.x, work.max.y + gap),
                        avail.max,
                    );
                    set_terminal_hole(ui.ctx(), Some(term_hole));
                    draw_terminal(ui, term_hole, workflow);
                    let show_inspector = state.selected.is_some();
                    let iw = if show_inspector { INSPECTOR_W } else { 0.0 };
                    let canvas = Rect::from_min_size(
                        work.min + Vec2::new(0.0, CANVAS_TOOLBAR_H),
                        Vec2::new(
                            (work.width() - iw - if show_inspector { gap } else { 0.0 }).max(200.0),
                            (work.height() - CANVAS_TOOLBAR_H).max(120.0),
                        ),
                    );
                    let toolbar_rect = Rect::from_min_max(
                        work.min,
                        Pos2::new(canvas.max.x, canvas.min.y),
                    );
                    draw_canvas_toolbar(ui, toolbar_rect, canvas, workflow, &mut state);
                    if state.pending_import_pick {
                        state.pending_import_pick = false;
                        let (x, y) = import_anchor_graph(&state, canvas, workflow);
                        let target = state.selected.clone().filter(|id| {
                            workflow.nodes.iter().any(|n| {
                                n.id == *id
                                    && (n.node_type == "media_bin" || n.node_type == "relay")
                            })
                        });
                        action = ProductionAction::WorkflowImportMediaPick {
                            extension: extension_id.to_owned(),
                            x,
                            y,
                            target_node: target,
                            positions: snapshot_positions(&state),
                        };
                    }
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
                    // OS file drop onto canvas → media_bin (or append to hit/selected container).
                    if let Some(drop) =
                        take_canvas_file_drop(ui, canvas, workflow, extension_id, &state)
                    {
                        action = drop;
                    }
                    if state.selected != before {
                        ui.ctx().request_repaint();
                    }

                    if let Some(sel) = state.selected.clone() {
                        if let Some(node) = workflow.nodes.iter().find(|n| n.id == sel) {
                            let irect = Rect::from_min_max(
                                Pos2::new(canvas.max.x + gap, work.min.y),
                                Pos2::new(work.max.x, work.max.y),
                            );
                            // Same-frame select: canvas still full-width; draw overlay strip.
                            let irect = if show_inspector {
                                irect
                            } else {
                                Rect::from_min_max(
                                    Pos2::new((work.max.x - INSPECTOR_W).max(work.min.x), work.min.y),
                                    work.max,
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
                                .corner_radius(CornerRadius::ZERO)
                                .show(&mut child, |ui| {
                                    ui.set_min_size(irect.size() - Vec2::splat(4.0));
                                    let next =
                                        draw_inspector(ui, node, extension_id, &mut state, thumbs);
                                    if next != ProductionAction::None {
                                        action = next;
                                    }
                                });
                        }
                    }

                    // After inspector so unsaved field edits are included. Works even while typing.
                    let want_save = ui.input(|i| {
                        let cmd = i.modifiers.command || i.modifiers.ctrl;
                        cmd && !i.modifiers.alt && i.key_pressed(egui::Key::S)
                    });
                    if want_save {
                        action = ProductionAction::WorkflowSave {
                            extension: extension_id.to_owned(),
                            positions: snapshot_positions(&state),
                            edits: state.edits.clone(),
                        };
                    }

                    ui.ctx().data_mut(|d| d.insert_temp(key, state));
                });
        });
    action
}

fn canvas_id(extension_id: &str) -> Id {
    Id::new(("todo_ui_workflow_canvas", extension_id))
}

fn draw_canvas_toolbar(
    ui: &mut Ui,
    rect: Rect,
    canvas: Rect,
    workflow: &WorkflowPanelView,
    state: &mut CanvasState,
) {
    ui.painter().rect_filled(rect, CornerRadius::ZERO, PANEL);
    ui.painter()
        .rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0_f32, BORDER), egui::StrokeKind::Inside);
    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("wf_canvas_toolbar")
            .max_rect(rect.shrink2(Vec2::new(10.0, 4.0))),
    );
    bar.set_clip_rect(rect);
    bar.horizontal(|ui| {
        ui.label(
            RichText::new(if state.linking.is_some() {
                "松开以连接"
            } else {
                "画布 · 可拖入 txt/png"
            })
            .size(12.0)
            .strong()
            .color(if state.linking.is_some() { BRAND } else { TEXT }),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(RichText::new("+").size(14.0).strong()).clicked() {
                state.zoom = (state.zoom * 1.15).clamp(0.35, 2.5);
            }
            ui.label(
                RichText::new(format!("{:.0}%", state.zoom * 100.0))
                    .size(12.0)
                    .color(TEXT),
            );
            if ui.button(RichText::new("−").size(14.0).strong()).clicked() {
                state.zoom = (state.zoom / 1.15).clamp(0.35, 2.5);
            }
            if ui.button(RichText::new("适配").size(12.0)).clicked() {
                fit_to_view(state, workflow, canvas);
            }
            if ui
                .button(RichText::new("导入文件…").size(12.0))
                .on_hover_text("导入 txt / png 等：新建素材容器，image 口可接生图 ref")
                .clicked()
            {
                state.pending_import_pick = true;
            }
        });
    });
}

fn import_anchor_graph(
    state: &CanvasState,
    canvas: Rect,
    workflow: &WorkflowPanelView,
) -> (f32, f32) {
    if let Some(id) = state.selected.as_ref() {
        if let Some(pos) = state.positions.get(id) {
            return (pos.x + CARD_W + 40.0, pos.y);
        }
        if let Some(node) = workflow.nodes.iter().find(|n| n.id == *id) {
            return (node.x + CARD_W + 40.0, node.y);
        }
    }
    let center = screen_to_graph(canvas.center(), canvas, state);
    (center.x - CARD_W * 0.5, center.y - 24.0)
}

fn take_canvas_file_drop(
    ui: &mut Ui,
    canvas: Rect,
    workflow: &WorkflowPanelView,
    extension_id: &str,
    state: &CanvasState,
) -> Option<ProductionAction> {
    let hovered = ui.input(|i| !i.raw.hovered_files.is_empty());
    if hovered {
        let pointer = ui.input(|i| i.pointer.hover_pos()).unwrap_or(canvas.center());
        if canvas.contains(pointer) {
            ui.painter().rect(
                canvas,
                CornerRadius::ZERO,
                Color32::from_rgba_unmultiplied(0x2a, 0x6a, 0x4a, 28),
                Stroke::new(2.0_f32, BRAND),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                canvas.center(),
                egui::Align2::CENTER_CENTER,
                "松开以导入到画布",
                FontId::proportional(16.0),
                BRAND,
            );
        }
    }
    let paths: Vec<std::path::PathBuf> = ui.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .filter_map(|f| f.path.clone())
            .filter(|p| p.is_file())
            .collect()
    });
    if paths.is_empty() {
        return None;
    }
    let pointer = ui
        .input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()))
        .unwrap_or(canvas.center());
    if !canvas.contains(pointer) {
        return None;
    }
    let graph_pos = screen_to_graph(pointer, canvas, state);
    let target = hit_media_container(graph_pos, workflow, state).or_else(|| {
        state.selected.clone().filter(|id| {
            workflow.nodes.iter().any(|n| {
                n.id == *id && (n.node_type == "media_bin" || n.node_type == "relay")
            })
        })
    });
    let (x, y) = if target.is_some() {
        (graph_pos.x, graph_pos.y)
    } else {
        (graph_pos.x - CARD_W * 0.5, graph_pos.y - 24.0)
    };
    Some(ProductionAction::WorkflowImportMedia {
        extension: extension_id.to_owned(),
        paths,
        x,
        y,
        target_node: target,
        positions: snapshot_positions(state),
    })
}

fn hit_media_container(
    graph_pos: Pos2,
    workflow: &WorkflowPanelView,
    state: &CanvasState,
) -> Option<String> {
    for node in workflow.nodes.iter().rev() {
        if node.node_type != "media_bin" && node.node_type != "relay" {
            continue;
        }
        let origin = state
            .positions
            .get(&node.id)
            .copied()
            .unwrap_or(Pos2::new(node.x, node.y));
        let rect = Rect::from_min_size(origin, card_size(node));
        if rect.contains(graph_pos) {
            return Some(node.id.clone());
        }
    }
    None
}

fn fit_to_view(state: &mut CanvasState, workflow: &WorkflowPanelView, canvas: Rect) {
    let Some(first) = workflow.nodes.first() else {
        state.pan = Vec2::ZERO;
        state.zoom = 1.0;
        return;
    };
    let mut min = node_origin(first, state);
    let mut max = min + card_size(first);
    for node in &workflow.nodes[1..] {
        let origin = node_origin(node, state);
        let size = card_size(node);
        min.x = min.x.min(origin.x);
        min.y = min.y.min(origin.y);
        max.x = max.x.max(origin.x + size.x);
        max.y = max.y.max(origin.y + size.y);
    }
    let size = (max - min).max(Vec2::splat(1.0));
    let zoom = ((canvas.width() - 64.0) / size.x)
        .min((canvas.height() - 64.0) / size.y)
        .clamp(0.35, 1.35);
    let center = min + size * 0.5;
    state.zoom = zoom;
    state.pan = (canvas.center() - canvas.min) - (center.to_vec2() * zoom);
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
            let rect = Rect::from_min_size(origin, card_size(node));
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
            draw_add_node_menu(
                ui,
                workflow,
                extension_id,
                paste_at,
                &positions,
                &mut action,
            );
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
    painter.rect_filled(canvas, CornerRadius::ZERO, PANEL_2);
    painter.rect_stroke(
        canvas,
        CornerRadius::ZERO,
        Stroke::new(1.0_f32, BORDER),
        egui::epaint::StrokeKind::Inside,
    );
    draw_grid(&painter, canvas, state);

    let hover_port = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|p| canvas.contains(*p))
        .map(|p| screen_to_graph(p, canvas, state))
        .and_then(|g| hit_port(g, workflow, state, None));
    let link_target = state
        .linking
        .as_ref()
        .and_then(|link| {
            ui.input(|i| i.pointer.hover_pos())
                .filter(|p| canvas.contains(*p))
                .map(|p| (link.from.as_str(), screen_to_graph(p, canvas, state)))
        })
        .and_then(|(from, graph)| {
            hit_port(graph, workflow, state, Some(false))
                .filter(|(id, _, _)| id != from)
        });

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
        let screen_max = graph_to_screen(origin + card_size(node), canvas, state);
        let rect = Rect::from_min_max(screen_min, screen_max);
        if rect.intersects(canvas) {
            let selected = state.selected.as_deref() == Some(node.id.as_str());
            paint_node(&painter, rect, node, state.zoom, selected);
            for (name, is_out, graph) in port_slots(node, state) {
                if (!is_out && node.inputs.is_empty()) || (is_out && node.outputs.is_empty()) {
                    continue;
                }
                let screen = graph_to_screen(graph, canvas, state);
                let target = link_target
                    .as_ref()
                    .is_some_and(|(id, n, o)| id == &node.id && n == &name && !*o);
                let hot = hover_port
                    .as_ref()
                    .is_some_and(|(id, n, o)| id == &node.id && n == &name && *o == is_out)
                    || state
                        .linking
                        .as_ref()
                        .is_some_and(|l| is_out && l.from == node.id && l.out == name);
                let r = if target {
                    7.0
                } else if hot {
                    6.0
                } else {
                    5.0
                };
                let port_color = if target {
                    crate::theme::SUCCESS
                } else if hot {
                    BRAND
                } else {
                    MUTED
                };
                painter.circle_filled(screen, r, port_color);
                painter.circle_stroke(
                    screen,
                    r,
                    Stroke::new(1.2_f32, if target || hot { TEXT } else { BORDER }),
                );
                // Port labels stay readable. At low zoom only the larger,
                // high-contrast circles remain; the inspector still exposes
                // the full port names when the node is selected.
                if state.zoom >= 0.74 {
                    let font = FontId::proportional(11.5);
                    if is_out {
                        painter.text(
                            Pos2::new(screen.x - 9.0, screen.y),
                            egui::Align2::RIGHT_CENTER,
                            short_text(&name, 12),
                            font,
                            if hot { TEXT } else { MUTED },
                        );
                    } else {
                        painter.text(
                            Pos2::new(screen.x + 9.0, screen.y),
                            egui::Align2::LEFT_CENTER,
                            short_text(&name, 12),
                            font,
                            if hot || target { TEXT } else { MUTED },
                        );
                    }
                }
            }
        }
    }
}

fn draw_inspector(
    ui: &mut Ui,
    node: &WorkflowNodeView,
    extension_id: &str,
    state: &mut CanvasState,
    thumbs: &mut ThumbCache,
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
            ui.add_space(4.0);
            ui.label(
                RichText::new(status_label(node.status))
                    .size(12.0)
                    .color(status_color(node.status)),
            );
            if node.node_type == "image_gen" {
                ui.add_space(6.0);
                if node.has_cache {
                    ui.label(
                        RichText::new("已缓存：上游与参数未变时下次运行直接复用")
                            .size(11.0)
                            .color(MUTED),
                    );
                    if button(ui, "清除缓存", Btn::Danger, true).clicked() {
                        action = ProductionAction::WorkflowClearNodeCache {
                            extension: extension_id.to_owned(),
                            node_id: node.id.clone(),
                        };
                    }
                } else {
                    ui.label(
                        RichText::new("尚无缓存：跑通一次生图后会写入，供下次复用")
                            .size(11.0)
                            .color(DIM),
                    );
                }
                ui.add_space(4.0);
            }
            if !node.summary.is_empty() {
                ui.label(RichText::new(&node.summary).size(12.0).color(MUTED));
            }
            if let Some(detail) = &node.detail {
                ui.add_space(6.0);
                egui::Frame::NONE
                    .stroke(Stroke::new(1.0_f32, status_color(node.status)))
                    .corner_radius(CornerRadius::ZERO)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new(detail)
                                .size(12.0)
                                .color(status_color(node.status)),
                        );
                    });
            }

            let has_media = node.files.iter().any(|f| {
                matches!(f.kind.as_str(), "image" | "video") && f.preview.is_some()
            });
            if node.output.is_some()
                || node.image.is_some()
                || !node.text_blocks.is_empty()
                || has_media
            {
                ui.add_space(8.0);
                egui::CollapsingHeader::new(RichText::new("结果").size(12.5).color(MUTED))
                    .id_salt(("wf_art", &node.id))
                    .default_open(true)
                    .show(ui, |ui| {
                        // Keep this tiny: each preview used to sync-decode a full
                        // 1024² still on the UI thread and freeze the app.
                        const MEDIA_PREVIEW_CAP: usize = 6;
                        let media_all: Vec<_> = node
                            .files
                            .iter()
                            .filter(|f| {
                                matches!(f.kind.as_str(), "image" | "video")
                                    && f.preview.is_some()
                            })
                            .collect();
                        let media_extra = media_all.len().saturating_sub(MEDIA_PREVIEW_CAP);
                        let media = &media_all[..media_all.len().min(MEDIA_PREVIEW_CAP)];
                        if !media.is_empty() {
                            egui::ScrollArea::vertical()
                                .max_height(220.0)
                                .id_salt(("wf_media", &node.id))
                                .show(ui, |ui| {
                                    // One Thumbnail request per frame — worker fills the rest.
                                    let mut requested = action != ProductionAction::None;
                                    for file in media {
                                        let Some(path) = file.preview.as_deref() else {
                                            continue;
                                        };
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(kind_badge(&file.kind))
                                                    .size(11.0)
                                                    .color(BRAND),
                                            );
                                            ui.label(
                                                RichText::new(&file.name).size(11.0).color(MUTED),
                                            );
                                        });
                                        let next = paint_media_preview(
                                            ui,
                                            path,
                                            &file.kind,
                                            thumbs,
                                            &mut action,
                                            &mut requested,
                                        );
                                        if next != ProductionAction::None {
                                            action = next;
                                        }
                                        ui.add_space(6.0);
                                    }
                                    if media_extra > 0 {
                                        ui.label(
                                            RichText::new(format!(
                                                "另有 {media_extra} 张未展开预览"
                                            ))
                                            .size(11.0)
                                            .color(DIM),
                                        );
                                    }
                                });
                        } else if let Some(path) = &node.image {
                            let mut requested = action != ProductionAction::None;
                            let next = paint_media_preview(
                                ui,
                                path,
                                "image",
                                thumbs,
                                &mut action,
                                &mut requested,
                            );
                            if next != ProductionAction::None {
                                action = next;
                            }
                        }
                        if !node.text_blocks.is_empty() {
                            egui::ScrollArea::vertical()
                                .max_height(220.0)
                                .id_salt(("wf_blocks", &node.id))
                                .show(ui, |ui| {
                                    for block in &node.text_blocks {
                                        egui::Frame::NONE
                                            .stroke(Stroke::new(1.0_f32, BORDER))
                                            .inner_margin(8.0)
                                            .corner_radius(CornerRadius::ZERO)
                                            .show(ui, |ui| {
                                                ui.set_width(ui.available_width());
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "#{} · {} 字",
                                                            block.index + 1,
                                                            block.chars
                                                        ))
                                                        .size(11.0)
                                                        .color(BRAND),
                                                    );
                                                    ui.label(
                                                        RichText::new(&block.id)
                                                            .size(10.0)
                                                            .color(DIM),
                                                    );
                                                });
                                                ui.add_space(4.0);
                                                ui.label(
                                                    RichText::new(&block.text)
                                                        .size(12.0)
                                                        .color(TEXT),
                                                );
                                            });
                                        ui.add_space(6.0);
                                    }
                                });
                        } else if let Some(text) = preview_plain_text(node.output.as_deref()) {
                            egui::ScrollArea::vertical()
                                .max_height(180.0)
                                .id_salt(("wf_text", &node.id))
                                .show(ui, |ui| {
                                    ui.label(RichText::new(text).size(12.0).color(TEXT));
                                });
                        } else if let Some(output) = &node.output {
                            if media.is_empty() && node.image.is_none() {
                                egui::ScrollArea::vertical()
                                    .max_height(180.0)
                                    .id_salt(("wf_out", &node.id))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new(output).size(12.0).color(MUTED),
                                        );
                                    });
                            }
                        }
                    });
            }

            ui.add_space(10.0);
            ui.separator();
            ui.label(RichText::new("参数").strong().size(13.0).color(TEXT));
            ui.add_space(6.0);

            if node.fields.is_empty() && node.files.is_empty() && node.text_blocks.is_empty() {
                ui.label(RichText::new("无可调参数").size(12.0).color(DIM));
            } else {
                let edits = state.edits.entry(node.id.clone()).or_default();
                let segment_mode = (node.node_type == "segment_script").then(|| {
                    edits
                        .get("mode")
                        .cloned()
                        .or_else(|| {
                            node.fields.iter().find_map(|f| match f {
                                WorkflowParamFieldView::Select { id, value, .. }
                                    if id == "mode" =>
                                {
                                    Some(value.clone())
                                }
                                _ => None,
                            })
                        })
                        .unwrap_or_else(|| "semantic".into())
                });
                for field in &node.fields {
                    if let Some(mode) = segment_mode.as_deref() {
                        let id = match field {
                            WorkflowParamFieldView::Select { id, .. }
                            | WorkflowParamFieldView::Number { id, .. }
                            | WorkflowParamFieldView::Text { id, .. }
                            | WorkflowParamFieldView::FileKinds { id, .. } => id.as_str(),
                        };
                        // 语意/段落/语句不显示「目标字数」；段落/语句不显示字数上限；
                        // 非语意不显示切点模型。按语句 = 一句一段，不受字数上限。
                        let hide = matches!(
                            (id, mode),
                            ("target", "semantic" | "paragraph" | "sentence")
                                | ("max", "paragraph" | "sentence")
                                | ("model", "length" | "paragraph" | "sentence")
                        );
                        if hide {
                            continue;
                        }
                    }
                    match field {
                        WorkflowParamFieldView::Select {
                            id,
                            label,
                            hint,
                            options,
                            value,
                        } => {
                            ui.label(RichText::new(label).size(12.0).color(TEXT));
                            let _ = hint;
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
                            ui.label(RichText::new(label).size(12.0).color(TEXT));
                            let _ = hint;
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
                            ui.label(RichText::new(label).size(12.0).color(TEXT));
                            let _ = hint;
                            // Canvas resolution is host-injected; show read-only.
                            if id == "canvas" {
                                let shown = if value.trim().is_empty() {
                                    placeholder.as_str()
                                } else {
                                    value.as_str()
                                };
                                ui.label(RichText::new(shown).size(12.0).color(MUTED));
                                ui.add_space(8.0);
                                continue;
                            }
                            let mut text = edits.get(id).cloned().unwrap_or_else(|| value.clone());
                            let resp = with_input_radius(ui, |ui| {
                                let mut edit = egui::TextEdit::singleline(&mut text)
                                    .desired_width((ui.available_width() - 8.0).max(80.0));
                                if !placeholder.is_empty() {
                                    edit = edit.hint_text(placeholder.as_str());
                                }
                                ui.add(edit)
                            });
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
                            ui.label(RichText::new(label).size(12.0).color(TEXT));
                            let _ = (hint, accept);
                            ui.add_space(4.0);
                            egui::Frame::NONE
                                .stroke(Stroke::new(1.0_f32, BORDER))
                                .inner_margin(10.0)
                                .corner_radius(CornerRadius::ZERO)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
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
                                ui.label(RichText::new("尚未添加文件").size(12.0).color(DIM));
                            } else {
                                const PARAM_FILE_CAP: usize = 12;
                                let mut requested = action != ProductionAction::None;
                                for file in node.files.iter().take(PARAM_FILE_CAP) {
                                    ui.horizontal(|ui| {
                                        if let Some(preview) = file.preview.as_deref() {
                                            let next = paint_media_thumb(
                                                ui,
                                                preview,
                                                &file.kind,
                                                56.0,
                                                thumbs,
                                                &mut action,
                                                &mut requested,
                                            );
                                            if next != ProductionAction::None {
                                                action = next;
                                            }
                                        } else {
                                            ui.label(
                                                RichText::new(kind_badge(&file.kind))
                                                    .size(11.0)
                                                    .color(BRAND),
                                            );
                                        }
                                        ui.vertical(|ui| {
                                            ui.label(
                                                RichText::new(&file.name).size(12.0).color(MUTED),
                                            );
                                            ui.label(
                                                RichText::new(&file.kind).size(10.0).color(DIM),
                                            );
                                        });
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
                                let extra = node.files.len().saturating_sub(PARAM_FILE_CAP);
                                if extra > 0 {
                                    ui.label(
                                        RichText::new(format!("另有 {extra} 个文件未列出"))
                                            .size(11.0)
                                            .color(DIM),
                                    );
                                }
                            }
                            ui.add_space(8.0);
                        }
                    }
                }
            }

            if node.node_type == "subtitles" {
                let cues = subtitle_cues_from_preview(node.output.as_deref());
                if !cues.is_empty() {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(
                        RichText::new("跳到字幕（监视器预览位置）")
                            .strong()
                            .size(13.0)
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new("先跑通本节点入轨后，点下方条目把播放头跳过去核对垂直位置。")
                            .size(11.0)
                            .color(DIM),
                    );
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .show(ui, |ui| {
                            for (index, cue) in cues.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    let stamp = format!("{:.1}s", cue.start);
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(format!("▶ {stamp}"))
                                                    .size(11.0)
                                                    .color(BRAND),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        action = ProductionAction::SeekPlayhead { time: cue.start };
                                    }
                                    let line: String = cue
                                        .text
                                        .chars()
                                        .take(18)
                                        .collect();
                                    ui.label(
                                        RichText::new(format!("{}. {line}", index + 1))
                                            .size(12.0)
                                            .color(MUTED),
                                    );
                                });
                            }
                        });
                    ui.add_space(6.0);
                }
            }

            let has_scalar = node
                .fields
                .iter()
                .any(|f| !matches!(f, WorkflowParamFieldView::FileKinds { .. }));
            if has_scalar {
                ui.add_space(4.0);
                if button(ui, "保存参数", Btn::Primary, true).clicked() {
                    let mut params = BTreeMap::new();
                    for field in &node.fields {
                        match field {
                            WorkflowParamFieldView::Select { id, value, .. } => {
                                let v = state
                                    .edits
                                    .get(&node.id)
                                    .and_then(|m| m.get(id))
                                    .cloned()
                                    .unwrap_or_else(|| value.clone());
                                params.insert(id.clone(), v);
                            }
                            WorkflowParamFieldView::Text { id, value, .. } => {
                                if id == "canvas" {
                                    continue;
                                }
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

        });
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
    let accent = if selected {
        BRAND
    } else {
        status_color(node.status)
    };
    let stroke = if selected { 1.8_f32 } else { 1.0 };
    painter.rect(
        rect,
        CornerRadius::ZERO,
        PANEL,
        Stroke::new(stroke, if selected { BRAND } else { BORDER }),
        egui::epaint::StrokeKind::Inside,
    );
    // Left status rail — state lives here; copy stays in the inspector.
    let rail = Rect::from_min_max(
        rect.left_top(),
        Pos2::new(rect.left() + (3.0 * zoom).clamp(2.0, 4.0), rect.bottom()),
    );
    painter.rect_filled(rail, CornerRadius::ZERO, accent);
    if zoom < 0.55 {
        return;
    }
    let title = short_text(&node.label, 14);
    painter.text(
        Pos2::new(rect.left() + 12.0, rect.top() + 8.0 * zoom.max(0.85)),
        egui::Align2::LEFT_TOP,
        title,
        FontId::proportional((12.5 * zoom.max(0.85)).clamp(11.0, 14.0)),
        TEXT,
    );
}

struct SubtitleCuePreview {
    text: String,
    start: f64,
}

fn subtitle_cues_from_preview(raw: Option<&str>) -> Vec<SubtitleCuePreview> {
    let Some(raw) = raw.filter(|s| !s.trim().is_empty()) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let Some(list) = value.get("cues").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|cue| {
            let text = cue.get("text")?.as_str()?.trim();
            if text.is_empty() {
                return None;
            }
            let start = cue.get("start").and_then(|v| v.as_f64()).unwrap_or(0.0);
            Some(SubtitleCuePreview {
                text: text.to_owned(),
                start: start.max(0.0),
            })
        })
        .collect()
}

fn kind_badge(kind: &str) -> &'static str {
    match kind {
        "image" => "图",
        "video" => "视频",
        "audio" => "音频",
        "text" => "文本",
        "json" => "JSON",
        _ => "文件",
    }
}

fn preview_plain_text(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let text = value.get("text").and_then(|v| v.as_str())?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(4000).collect())
    }
}

fn paint_media_preview(
    ui: &mut Ui,
    path: &str,
    kind: &str,
    thumbs: &mut ThumbCache,
    action: &mut ProductionAction,
    requested: &mut bool,
) -> ProductionAction {
    let mut next = ProductionAction::None;
    let width = ui.available_width().min(300.0);
    let height = if kind == "video" {
        (width * 9.0 / 16.0).clamp(80.0, 160.0)
    } else {
        (width * 0.55).clamp(72.0, 140.0)
    };
    let sense = if kind == "video" {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), sense);
    // Async thumbs only — never image::open on the UI thread (67×1024² freezes).
    if !thumbs.contains(path) && !*requested && *action == ProductionAction::None {
        next = ProductionAction::Thumbnail {
            path: path.to_owned(),
        };
        *requested = true;
    }
    if !thumbs.paint(ui, path, rect) {
        ui.painter().rect_filled(rect, 0.0, PANEL_2);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            if kind == "video" {
                "视频载入中"
            } else {
                "载入中"
            },
            FontId::proportional(12.0),
            DIM,
        );
    }
    if kind == "video" {
        ui.painter().text(
            rect.left_bottom() + Vec2::new(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            "点击播放",
            FontId::proportional(11.0),
            BRAND,
        );
        if response.clicked() {
            next = ProductionAction::OpenExternal {
                path: path.to_owned(),
            };
        }
        response.on_hover_text("用系统播放器打开");
        if ui
            .add(
                egui::Button::new(RichText::new("用系统播放器打开").size(12.0).color(BRAND))
                    .frame(false),
            )
            .clicked()
        {
            next = ProductionAction::OpenExternal {
                path: path.to_owned(),
            };
        }
    }
    next
}

fn paint_media_thumb(
    ui: &mut Ui,
    path: &str,
    kind: &str,
    side: f32,
    thumbs: &mut ThumbCache,
    action: &mut ProductionAction,
    requested: &mut bool,
) -> ProductionAction {
    let mut next = ProductionAction::None;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    if !thumbs.contains(path) && !*requested && *action == ProductionAction::None {
        next = ProductionAction::Thumbnail {
            path: path.to_owned(),
        };
        *requested = true;
    }
    if !thumbs.paint(ui, path, rect) {
        ui.painter().rect_filled(rect, 0.0, PANEL_2);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            kind_badge(kind),
            FontId::proportional(10.0),
            BRAND,
        );
    }
    next
}

#[cfg(test)]
mod regression_tests {
    use super::*;

    #[test]
    fn media_preview_requests_thumbnail_instead_of_sync_decode() {
        let mut thumbs = ThumbCache::default();
        let ctx = egui::Context::default();
        let mut action = ProductionAction::None;
        let mut requested = false;
        let mut next = ProductionAction::None;
        let raw = egui::RawInput::default();
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                next = paint_media_preview(
                    ui,
                    "C:/missing/still.png",
                    "image",
                    &mut thumbs,
                    &mut action,
                    &mut requested,
                );
            });
        });
        assert!(matches!(next, ProductionAction::Thumbnail { .. }), "{next:?}");
        assert!(requested);
    }
}
