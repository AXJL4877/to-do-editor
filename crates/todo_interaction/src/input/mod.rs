mod controller;
mod intents;
mod preview;
#[cfg(test)]
#[allow(unused_must_use)]
mod tests;
mod view;

pub use controller::EditorController;
pub use intents::{EditorFocus, Intent, KeyCode, PreviewPointerKind, TimelineTool};

use crate::presentation::{DropPreview, LibraryState};
use crate::timeline::TimelineController;
use crate::transform::{AlignGuide, TransformGesture};
use todo_core::{ClipTransform, TransformBounds};

#[derive(Debug, Clone)]
struct PreviewDrag {
    clip_id: String,
    gesture: TransformGesture,
    projection: crate::PreviewProjection,
    canvas_size: (f64, f64),
    /// Other visible layer bounds at gesture start (canvas space).
    peers: Vec<TransformBounds>,
    /// Active align guides in canvas space while dragging.
    guides: Vec<AlignGuide>,
}
