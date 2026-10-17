//! Talk to `native/build/bin/preview_host.exe` over stdin/stdout.
//!
//! Line protocol: path/seek/transport/audio/hwnd/size-screen/quit, plus
//! `aspect` and `layout` (the current host may ignore those two).
//!
//! # Stdin
//! - `hwnd <hex>`
//! - `path` or `path <media>`
//! - `path-token <generation>` or `path-token <generation> <media>`
//! - `seek <seconds>`
//! - `transform <x> <y> <scale>` (offsets in full canvas fractions)
//! - `transform <x> <y> <scale> <scale_x> <scale_y> <opacity>`
//! - `gizmo` or `gizmo <left> <top> <right> <bottom> <editable 0|1>`
//! - `guides` or `guides <axis> <pos>…` (`axis` 0=vertical / 1=horizontal, positions are window fractions)
//! - `viewport <left> <top> <width> <height> <show_outside 0|1>` (window fractions)
//! - `aspect <width> <height>`
//! - `layout <x> <y> <w> <h>`
//! - `size-screen <x> <y> <w> <h> <visible 0|1>`
//! - `audio-begin` / `audio-clip <start> <duration> <offset> <rate> <gain> <fade_in> <fade_out> <path>` / `audio-commit`
//! - `layers-begin` / `layer <kind> <media_time> <x> <y> <scale> <scale_x> <scale_y> <opacity> <path>` / `layers-commit`
//! - `transport <seconds> <playing 0|1>`
//! - `quit`
//!
//! # Stdout
//! - `audio-clock <seconds> <playing 0|1> <available 0|1>`
//! - `preview-bounds <x> <y> <w> <h> [<visible 0|1>]`
//! - `source-aspect <generation> <decoded display width / height>`
//! - `preview-pointer <down|move|up> <x> <y>` (window fractions, may exceed 0..1 during capture)
//! - `preview-navigation <wheel|pan-down|pan-move|pan-up> <delta> <x> <y>`

mod host;
mod protocol;
mod render;
mod viewport;

pub use host::{resolve_host_exe, PreviewHost};
pub use protocol::{
    AlignGuide, AudioClip, AudioClock, ClockSample, HostEvent, LayerKind, LayoutRect,
    PreviewBounds, PreviewLayer, PreviewPointerKind, ScreenSlot, TransformGizmo, VisualTransform,
    CLOCK_STALE, HOST_RELATIVE, MAX_ALIGN_GUIDES, MAX_AUDIO_CLIPS, MAX_PREVIEW_LAYERS,
};
pub use render::OfflineRenderer;
pub use viewport::{PreviewNavigation, PreviewNavigationKind, PreviewViewport};

#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    /// `TODO_PREVIEW_HOST` was missing, or no `preview_host.exe` was found.
    #[error("preview host not found: {0} (set TODO_PREVIEW_HOST to the exe)")]
    HostNotFound(std::path::PathBuf),
    /// The process has exited, or stdin is already closed.
    #[error("preview host is not running")]
    Closed,
    /// A command was rejected before it was written.
    #[error("{0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
#[cfg(test)]
#[path = "../../../tests/support/mod.rs"]
pub(crate) mod test_support;
