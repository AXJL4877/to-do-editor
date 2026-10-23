pub mod agent;
pub mod ai_settings;
pub mod assets;
mod chrome;
mod controls;
pub mod export;
mod icons;
mod inspector;
mod layout;
mod library;
mod monitor;
pub mod production;
mod project_settings;
mod projects;
mod shortcuts;
mod timeline;
mod toolbar;

pub use agent::{
    close as close_agent, draw_dock as draw_agent_dock, draw_modal as draw_agent,
    is_open as agent_open, request_copy_token as request_copy_agent_token,
    set_open as set_agent_open, take_copy_token as take_copy_agent_token,
};
pub use agent::{AgentAction, AgentApproval, AgentMessage, AgentRole, AgentView};
pub use ai_settings::AiSettingsAction;
pub use assets::{
    dock as assets_dock, draw_dock as draw_assets_dock, left_tab, set_left_tab, AssetsAction,
    LeftTab,
};
pub use ai_settings::{
    close as close_ai_settings, draw_modal as draw_ai_settings, is_open as ai_settings_open,
    set_open as set_ai_settings_open, set_open_ctx as set_ai_settings_open_ctx,
};
pub use export::{
    draw_modal as draw_export, is_open as export_open, set_open as set_export_open, ExportAction,
    ExportPresetChoice, ExportView,
};
pub use layout::draw_editor;
pub use monitor::{overlays_over, MonitorHole};
pub use production::{
    close as close_production_panel, close_workflow, dock_rect as production_dock,
    draw_dock as draw_production_dock, draw_workflow_overlay, is_open as production_panel_open,
    open_workflow, set_side_tab as set_production_dock, toggle as toggle_production_panel,
    workflow_fullscreen_open, workflow_open_id, workflow_terminal_hole, ProductionAction, SideTab,
};
pub use project_settings::close as close_project_settings;
pub use project_settings::is_open as project_settings_open;
pub use projects::{draw_close_prompt, draw_projects};
