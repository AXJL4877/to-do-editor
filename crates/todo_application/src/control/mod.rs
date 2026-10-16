//! Transport-independent editing capabilities shared by AI, scripts and other adapters.
//! Call [`crate::Workspace::control`] on the workspace-owning thread and execute
//! the returned [`crate::AppUpdate`] through the normal runtime effect handler.

pub(crate) mod batch;
pub(crate) mod state;
mod transform;
mod types;

pub use types::*;

pub const CONTROL_API_VERSION: u32 = 1;
