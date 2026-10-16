//! Persisted visual transforms and pure geometry in normalized canvas coordinates.
mod geometry;
mod model;

pub use geometry::{BaseSize, TransformBounds};
pub use model::Transform;

/// Compatibility name for the transform persisted on a clip.
pub type ClipTransform = Transform;
