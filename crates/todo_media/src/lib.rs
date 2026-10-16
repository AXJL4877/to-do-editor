//! Local Rust media services. No server, network client or JavaScript runtime.
mod cache;
mod error;
mod files;
mod imports;
mod metadata;
mod service;
mod text;
mod thumbnails;
mod tools;
mod waveforms;

pub use error::MediaError;
pub use metadata::MediaMetadata;
pub use service::MediaService;
pub use text::{font_dirs, TextRasterizer};
pub use todo_text::RasterRequest;
pub use tools::MediaTools;
pub use waveforms::Waveform;

#[cfg(test)]
#[path = "../../../tests/support/mod.rs"]
pub(crate) mod test_support;
