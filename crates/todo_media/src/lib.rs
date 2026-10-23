//! Local Rust media services. No server, network client or JavaScript runtime.
mod adopt;
mod cache;
mod encode;
mod error;
pub mod export;
mod files;
mod imports;
mod metadata;
mod proxies;
mod reveal;
mod service;
mod text;
mod thumbnails;
mod tools;
mod waveforms;

pub use encode::{encode_frames, EncodeInfo, EncodeQuality};
pub use error::MediaError;
pub use export::presets::ExportPreset;
pub use export::quality::{
    inspect as inspect_quality, write_report, QualityReport, QualityTargets,
};
pub use export::contact_sheet::stitch_contact_sheet;
pub use export::{export_frames, ExportRequest, ExportResult};
pub use metadata::{AlphaPacking, MediaMetadata};
pub use proxies::EditProxy;
pub use reveal::{compose_reveal, read_reveal, write_reveal, HandMeta, RevealMeta, REVEAL_SPEC};
pub use service::MediaService;
pub use text::{
    font_catalog, font_dirs, font_preview, packaged_font_dirs, style_preview, FontFile,
    FontRecord, TextRasterizer, FONT_MANIFEST,
};
pub use todo_text::RasterRequest;
pub use tools::MediaTools;
pub use waveforms::Waveform;

#[cfg(test)]
#[path = "../../../tests/support/mod.rs"]
pub(crate) mod test_support;

/// Pixel size of an image file, reading only its header.
pub fn image_size(path: &std::path::Path) -> Result<(u32, u32), MediaError> {
    image::image_dimensions(path)
        .map_err(|e| MediaError::Invalid(format!("{}: {e}", path.display())))
}
