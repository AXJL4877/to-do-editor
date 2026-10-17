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
    ai_settings_open, close_ai_settings, close_production_panel, close_project_settings,
    draw_ai_settings, draw_close_prompt, draw_editor, draw_export, draw_production_panel,
    draw_projects, export_open, production_panel_open, project_settings_open, set_ai_settings_open,
    set_ai_settings_open_ctx, set_export_open, AiSettingsAction, ExportAction, ExportPresetChoice,
    ExportView, MonitorHole, ProductionAction,
};

#[cfg(test)]
pub(crate) fn test_view(
    project: todo_core::Project,
    name: &str,
) -> todo_interaction::EditorViewModel {
    let model = todo_application::EditorSession::new(project, name);
    let mut controller = todo_interaction::EditorController::new();
    controller.sync_output_frame(model.project().width, model.project().height);
    controller.view(&model)
}
