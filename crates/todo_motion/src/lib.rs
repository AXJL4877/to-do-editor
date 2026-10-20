//! Motion module: entrances, exits and transitions as reusable keyframed data.
//!
//! One format for every motion the editor plays. Built-in presets are data
//! (`builtin.json`, embedded at compile time). The user library
//! (`<repo>/motions/`) and extension libraries (`extensions/<id>/motions/`)
//! use the same JSON. A project copies a non-built-in preset in when it is
//! first used, so documents render the same after a library changes.
//!
//! This crate knows nothing about projects or clips: `todo_core` stores motion
//! references and calls [`MotionPreset::sample`] when resolving a frame.
mod library;
mod preset;

pub use library::{LibraryEntry, MotionLibrary, MotionSource, MOTIONS_DIR};
pub use preset::{
    builtin_motions, is_builtin_motion, valid_motion_id, Ease, MotionChannels, MotionCurve,
    MotionKey, MotionKind, MotionPreset, MotionSample, DISSOLVE, FADE_OUT, MAX_MOTION_SECS,
};
