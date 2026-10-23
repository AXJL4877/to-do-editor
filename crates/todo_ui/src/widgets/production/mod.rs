//! Right-hand dock: Agent chat (tool-calling).
//! Workflow graph UI modules are retained only for Bridge/runtime; they are not drawn.
#[allow(dead_code)]
mod card;
#[allow(dead_code)]
mod content;
#[allow(dead_code)]
mod flow;
#[allow(dead_code)]
mod form;
mod workflow;

use crate::theme::{BORDER, BRAND, DANGER, MUTED, TEXT, WARN};
use crate::widgets::agent::{self, AgentAction, AgentView};
use crate::ThumbCache;
use egui::{Color32, Id, Rect, RichText, Ui};
use std::collections::BTreeMap;
use todo_interaction::{
    ParamValue, ProductionEdit, ProductionNodeStatus, ProductionPanelView,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ProductionAction {
    None,
    OpenSettings,
    Start {
        extension: String,
        params: BTreeMap<String, ParamValue>,
        script: String,
    },
    /// `key: None` retries every failed composition of the production.
    Retry {
        production: String,
        key: Option<String>,
    },
    /// Stop generating and drop the card; placed clips stay.
    Remove {
        production: String,
    },
    /// Put back the generated clips the user deleted from the timeline.
    Restore {
        production: String,
    },
    Edit {
        production: String,
        edit: ProductionEdit,
    },
    Upload {
        production: String,
        element: String,
    },
    Thumbnail {
        path: String,
    },
    /// Open a local file with the OS default app (e.g. system video player).
    OpenExternal {
        path: String,
    },
    /// 运行整条工作流（空 script 用图里的文案）。
    /// `edits` 是检查器里还没保存的临时改动，运行按它生效。
    WorkflowRun {
        extension: String,
        script: String,
        edits: BTreeMap<String, BTreeMap<String, String>>,
    },
    /// 停止正在跑的工作流（取消在途请求，保留已完成的节点）。
    WorkflowStop {
        extension: String,
    },
    /// Scaffold a blank user workflow package.
    WorkflowCreate,
    /// Delete a user workflow package from disk.
    WorkflowDelete {
        extension: String,
    },
    /// Write the open graph (positions + inspector edits) to disk.
    WorkflowSave {
        extension: String,
        positions: BTreeMap<String, (f32, f32)>,
        edits: BTreeMap<String, BTreeMap<String, String>>,
    },
    /// Write inspector params into the extension graph.json.
    WorkflowSaveParams {
        extension: String,
        node_id: String,
        /// Field id → JSON-ish string (numbers as decimal, selects as value).
        params: BTreeMap<String, String>,
    },
    /// Open a file dialog and attach files to a media_bin node.
    WorkflowPickMedia {
        extension: String,
        node_id: String,
    },
    /// Canvas/toolbar import: pick files, then create media_bin or append to target.
    WorkflowImportMediaPick {
        extension: String,
        x: f32,
        y: f32,
        /// Drop onto this media_bin/relay when set; otherwise create a new media_bin.
        target_node: Option<String>,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// OS drop / already-picked paths onto the canvas.
    WorkflowImportMedia {
        extension: String,
        paths: Vec<std::path::PathBuf>,
        x: f32,
        y: f32,
        target_node: Option<String>,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// Drop one file entry from a media_bin node.
    WorkflowRemoveMedia {
        extension: String,
        node_id: String,
        path: String,
    },
    /// Clear disk cache for image_gen (etc.) so the next run regenerates.
    WorkflowClearNodeCache {
        extension: String,
        node_id: String,
    },
    /// Insert a catalog node onto the canvas.
    WorkflowAddNode {
        extension: String,
        node_type: String,
        x: f32,
        y: f32,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// Clone a node (no edges).
    WorkflowDuplicateNode {
        extension: String,
        node_id: String,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// Delete a node and its incident edges.
    WorkflowDeleteNode {
        extension: String,
        node_id: String,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// Paste a previously copied node.
    WorkflowPasteNode {
        extension: String,
        node_type: String,
        label: Option<String>,
        /// JSON object of params (`{}` if empty).
        params_json: String,
        x: f32,
        y: f32,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// Wire an output port to an input port.
    WorkflowConnect {
        extension: String,
        from: String,
        to: String,
        out: Option<String>,
        input: Option<String>,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// Remove one edge.
    WorkflowDisconnect {
        extension: String,
        from: String,
        to: String,
        out: Option<String>,
        input: Option<String>,
        positions: BTreeMap<String, (f32, f32)>,
    },
    /// Jump the editor playhead (subtitle preview on the program canvas).
    SeekPlayhead {
        time: f64,
    },
}

/// Which pane the right-hand column shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SideTab {
    #[default]
    Properties,
    Production,
}

const TAB_ID: &str = "todo_ui_side_tab";
const DOCK_ID: &str = "todo_ui_production_dock_rect";
const BADGE_ID: &str = "todo_ui_production_dock_badge";
const SELECTED_WF: &str = "todo_ui_workflow_selected";

pub fn side_tab(ctx: &egui::Context) -> SideTab {
    ctx.data(|data| data.get_temp::<SideTab>(Id::new(TAB_ID)))
        .unwrap_or_default()
}

pub fn set_side_tab(ctx: &egui::Context, tab: SideTab) {
    ctx.data_mut(|data| data.insert_temp(Id::new(TAB_ID), tab));
}

/// Toolbar entry: show the dock, or go back to properties if it is showing.
pub fn toggle(ctx: &egui::Context) {
    let next = match side_tab(ctx) {
        SideTab::Production => SideTab::Properties,
        SideTab::Properties => SideTab::Production,
    };
    set_side_tab(ctx, next);
}

pub fn is_open(ctx: &egui::Context) -> bool {
    side_tab(ctx) == SideTab::Production
}

pub fn close(ctx: &egui::Context) {
    set_side_tab(ctx, SideTab::Properties);
}

/// Where the layout reserved the dock this frame (`None`: not showing).
pub(crate) fn set_dock(ctx: &egui::Context, rect: Option<Rect>) {
    ctx.data_mut(|data| data.insert_temp(Id::new(DOCK_ID), rect));
}

pub fn dock_rect(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|data| data.get_temp::<Option<Rect>>(Id::new(DOCK_ID)))
        .flatten()
}

/// A dot on the tab while something runs or needs the user.
pub(crate) fn dock_badge(ctx: &egui::Context) -> Option<Color32> {
    ctx.data(|data| data.get_temp::<Option<Color32>>(Id::new(BADGE_ID)))
        .flatten()
}

pub(super) fn status_color(status: ProductionNodeStatus) -> Color32 {
    match status {
        ProductionNodeStatus::Queued => MUTED,
        ProductionNodeStatus::Running => BRAND,
        ProductionNodeStatus::Waiting | ProductionNodeStatus::Attention => WARN,
        ProductionNodeStatus::Complete => crate::theme::SUCCESS,
        ProductionNodeStatus::Failed => DANGER,
    }
}

#[allow(dead_code)]
pub(super) fn caption(ui: &mut Ui, step: Option<usize>, text: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if let Some(step) = step {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 9.0, BORDER);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                step.to_string(),
                egui::FontId::proportional(11.0),
                TEXT,
            );
        }
        ui.label(RichText::new(text).strong().size(13.0).color(TEXT));
    });
    ui.add_space(2.0);
}

#[allow(dead_code)]
pub fn selected_workflow(ctx: &egui::Context) -> Option<String> {
    ctx.data(|d| d.get_temp::<Option<String>>(Id::new(SELECTED_WF)))
        .flatten()
}

pub fn set_selected_workflow(ctx: &egui::Context, id: Option<String>) {
    ctx.data_mut(|d| d.insert_temp(Id::new(SELECTED_WF), id));
}

fn refresh_badge(ctx: &egui::Context, view: &ProductionPanelView) {
    let badge = if view
        .productions
        .iter()
        .any(|p| p.status == ProductionNodeStatus::Failed)
    {
        Some(DANGER)
    } else if view.productions.iter().any(|p| {
        matches!(
            p.status,
            ProductionNodeStatus::Running | ProductionNodeStatus::Waiting
        )
    }) {
        Some(BRAND)
    } else {
        None
    };
    ctx.data_mut(|data| data.insert_temp(Id::new(BADGE_ID), badge));
}

/// Draw the Agent chat dock into `ui` (its rect is [`dock_rect`]).
pub fn draw_dock(
    ui: &mut Ui,
    view: &ProductionPanelView,
    agent: &mut AgentView,
    thumbs: &mut ThumbCache,
) -> AgentAction {
    let _ = thumbs;
    refresh_badge(ui.ctx(), view);
    agent::draw_dock(ui, agent)
}

/// Workflow fullscreen UI retired — always closed, no `~` graph editor.
pub fn draw_workflow_overlay(
    ctx: &egui::Context,
    panel: &ProductionPanelView,
    graph: Option<&todo_interaction::WorkflowPanelView>,
    title: &str,
    thumbs: &mut ThumbCache,
) -> ProductionAction {
    let _ = (panel, graph, title, thumbs);
    if workflow::is_open(ctx) {
        workflow::close(ctx);
    }
    ProductionAction::None
}

pub use workflow::{
    close as close_workflow, is_blocking_input, is_open as workflow_fullscreen_open,
    open as open_workflow, open_id as workflow_open_id, terminal_hole as workflow_terminal_hole,
};

#[cfg(test)]
mod tests;
