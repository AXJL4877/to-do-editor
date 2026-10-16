//! Application use cases. No UI toolkit, filesystem, network, or process I/O.

mod action;
mod clipboard;
pub mod control;
mod editor;
mod effect;
mod event;
mod history;
mod media_jobs;
mod playback;
mod projects;
mod text_rasters;

pub use action::{AppAction, CloseChoice, EditorCommand};
pub use editor::{
    AudioClockSample, EditorSession, EditorUpdate, MonitorPane, Picture, PreviewCommand,
    PreviewLayer, SourceMonitor, Status, CLOCK_FRESH_SECS, CLOCK_MISSING_SECS, CLOCK_POLL_SECS,
};
pub use effect::{Effect, MediaTask, ProjectTask, Ticket};
pub use event::{AppEvent, MediaOutput, ProjectOutput};
pub use media_jobs::{AssetRequest, PeakStrip, PeaksCache};
pub use projects::{AppUpdate, Route, Workspace};
pub use text_rasters::TextRaster;
pub use todo_core::{
    ClipTransform, CopiedClip, DomainError, EditCommand, MediaItem, Project, ResolvedAudioClip,
    TrackKind, TrimEdge,
};
