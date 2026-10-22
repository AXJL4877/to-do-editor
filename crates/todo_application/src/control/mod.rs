//! Transport-independent editing capabilities shared by AI, scripts and other adapters.
//! Call [`crate::Workspace::control`] on the workspace-owning thread and execute
//! the returned [`crate::AppUpdate`] through the normal runtime effect handler.

pub(crate) mod batch;
pub mod motions;
pub mod review;
pub(crate) mod state;
mod transform;
mod types;
pub mod view;
pub mod workflow;

pub use types::*;
pub use view::{
    project_view, CanvasView, ElementView, ProjectView, Rect, Size, TextView, TrackView,
    TransitionView,
};
pub use workflow::{
    commercial_run_state, commercial_run_state_with_placement, placement_failed_after_done,
    placement_failed_after_done_ex, placement_summary, snapshot_from,
    snapshot_from_with_placement,
    ui_state_name,
};

pub const CONTROL_API_VERSION: u32 = 1;
