//! Persisted project data and constructors, independent of input and rendering.
pub mod clip;
pub mod folder;
pub mod media;
pub mod project;
pub mod text;
pub mod track;

pub use clip::{Clip, ClipSource, ClipTransform, MediaSource};
pub use folder::MediaFolder;
pub use media::{MediaItem, MediaKind};
pub use project::{MediaScaleMode, Project};
pub use text::{
    FontRef, TextAlign, TextAnimation, TextColor, TextContent, TextErode, TextFrame, TextIntro,
    TextLayer, TextLayerKind, TextLayout, TextPaint, TextStyle,
};
pub use track::{Track, TrackKind};
