//! Styled text rasterizer: [`TextContent`] + [`TextStyle`] → a transparent,
//! canvas-sized, premultiplied RGBA image.
//!
//! Pure computation apart from reading font files. The image covers the whole
//! output canvas with the text block centered, so a clip [`todo_core::Transform`]
//! places it exactly like any other full-frame picture. Whole-text opacity
//! ([`todo_core::TextFrame::opacity`]) is a compositing parameter and is not baked
//! in; only the per-character reveal changes the raster.
mod effects;
mod fonts;
mod layout;
mod raster;

pub use fonts::{family_name, FontLibrary};
pub use raster::{measure, render, TextBlock, TextImage};
use todo_core::{PerCharReveal, TextContent, TextStyle};

/// Largest canvas edge the rasterizer accepts, matching the project limit.
pub const MAX_CANVAS_EDGE: u32 = 8192;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TextError {
    #[error("font not found: {0}")]
    FontNotFound(String),
    #[error("font cannot be parsed: {0}")]
    InvalidFont(String),
    #[error("invalid canvas size {0}x{1}")]
    InvalidCanvas(u32, u32),
    #[error("png encoding failed: {0}")]
    Encode(String),
}

/// Everything that determines one raster.
#[derive(Debug, Clone, Copy)]
pub struct RasterRequest<'a> {
    pub content: &'a TextContent,
    pub style: &'a TextStyle,
    /// Output canvas size in pixels.
    pub canvas: (u32, u32),
    /// Per-character intro state; `None` draws every character fully.
    pub reveal: Option<PerCharReveal>,
}
