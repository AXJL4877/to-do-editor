mod chrome;
mod controls;
mod icons;
mod inspector;
mod layout;
mod library;
mod monitor;
mod project_settings;
mod projects;
mod shortcuts;
mod timeline;
mod toolbar;

pub use layout::draw_editor;
pub use monitor::MonitorHole;
pub use project_settings::close as close_project_settings;
pub use project_settings::is_open as project_settings_open;
pub use projects::{draw_close_prompt, draw_projects};
