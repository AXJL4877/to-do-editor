//! Production domain: what to generate and how results reach the timeline.
//! Pure data and rules; no I/O, network, threads or UI.
//! Workflow package standards (manifest / graph / nodes) live in `todo_workflow::workflow`.
pub mod items;
pub mod narration;
pub mod pacing;
pub mod placement;
pub mod production;
pub mod segment;
pub mod storyboard;
pub mod subtitles;
pub mod tasks;
pub mod text;
pub mod voice;
pub mod work;

pub use production::{PackRef, Production, SplitMode, PRODUCTION_SPEC};
pub use segment::{CastMember, Limits, Outline, Unit};
pub use tasks::{
    AudioRequest, Capability, ChatMessage, ChatRole, ImageRequest, LanguageRequest,
    MultimodalRequest, Provider, TaskKey, TaskRequest, TaskSpec, VideoRequest, TASK_SPEC_VERSION,
};
