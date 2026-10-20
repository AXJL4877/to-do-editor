//! Read-only domain projections and lookups.
pub mod find_clip;
pub mod linked;
mod move_targets;
pub mod placement;
pub mod project;
pub mod resolve_preview;
pub mod reveal;

pub use find_clip::{find_clip, FoundClip};
pub use linked::linked_clip_ids;
pub use move_targets::can_move_clips;
pub use placement::can_place_media;
pub use resolve_preview::{
    adjacent_predecessor, clip_display_name, export_layers, hit_transform, resolve_audio_clips,
    resolve_layers, resolve_preview, transition_is_active, visible_hits, HitSource, PreviewHit,
    ResolvedAudioClip, TransitionPhase, ADJACENT_EPSILON, MAX_DECODED_LAYERS, MAX_PREVIEW_LAYERS,
    MAX_TEXT_LAYERS,
};
pub use reveal::{reveal_frame, reveal_hand, HandLayer, RevealFrame};
