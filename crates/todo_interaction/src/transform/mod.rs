//! Preview transform hit testing and transient gestures. All coordinates are
//! normalized to the current canvas; viewport dimensions are logical pixels.
mod align;
mod gesture;
mod hit_test;

pub(crate) use align::{AlignAxis, AlignGuide};
pub(crate) use gesture::TransformGesture;
pub(crate) use hit_test::hit_test;

#[cfg(test)]
mod tests;
