mod chrome;
mod controls;
mod icons;
mod inspector;
mod layout;
mod library;
mod monitor;
mod projects;
mod shortcuts;
mod timeline;
mod toolbar;

pub use layout::draw_editor;
pub use monitor::MonitorHole;
pub use projects::{draw_close_prompt, draw_projects};
