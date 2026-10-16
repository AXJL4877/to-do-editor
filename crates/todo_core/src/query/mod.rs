//! Read-only domain projections and lookups.
pub mod find_clip;
pub mod linked;
mod move_targets;
pub mod placement;
pub mod project;
pub mod resolve_preview;

pub use find_clip::{find_clip, FoundClip};
pub use linked::linked_clip_ids;
pub use move_targets::can_move_clips;
pub use placement::can_place_media;
pub use resolve_preview::{
    clip_display_name, resolve_audio_clips, resolve_layers, resolve_preview, HitSource, PreviewHit,
    ResolvedAudioClip, MAX_PREVIEW_LAYERS, MAX_TEXT_LAYERS, MAX_VIDEO_LAYERS,
};
