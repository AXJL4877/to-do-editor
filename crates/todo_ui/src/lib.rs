//! UI layer: draw view-models and emit intents.

mod format;
mod peaks;
mod theme;
mod thumbs;
mod widgets;

pub use peaks::{PeakStrip, PeaksCache};
pub use theme::{button, install as install_theme, Btn, BG, COLORS, MUTED, TEXT};
pub use thumbs::ThumbCache;
pub use widgets::{
    agent_open, ai_settings_open, assets_dock, draw_assets_dock, left_tab, set_left_tab,
    AssetsAction, LeftTab, close_agent, close_ai_settings, close_production_panel,
    close_project_settings, draw_agent, draw_ai_settings, draw_close_prompt, draw_editor,
    draw_export, draw_production_dock, draw_projects, draw_workflow_overlay, export_open,
    open_workflow, overlays_over, production_dock, production_panel_open, project_settings_open,
    set_agent_open, set_ai_settings_open, set_ai_settings_open_ctx, set_export_open,
    set_production_dock, toggle_production_panel, workflow_fullscreen_open, workflow_open_id,
    AgentAction,
    AgentApproval, AgentMessage, AgentRole, AgentView, AiSettingsAction, ExportAction,
    ExportPresetChoice, ExportView, MonitorHole, ProductionAction, SideTab,
};

#[cfg(test)]
pub(crate) fn test_view(
    project: todo_core::Project,
    name: &str,
) -> todo_interaction::EditorViewModel {
    let model = todo_application::EditorSession::new(project, name);
    let mut controller = todo_interaction::EditorController::new();
    controller.sync_output_frame(model.project().width, model.project().height);
    controller.view(&model, &[])
}
