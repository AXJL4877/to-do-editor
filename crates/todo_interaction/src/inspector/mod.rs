//! Selected-clip properties: toolkit-neutral projection and input translation.
mod input;
mod presentation;
pub(crate) use presentation::project_properties;
pub use presentation::{
    ClipProperties, MotionOption, MotionProperties, MotionSlot, SourceProperties, StyleOption,
    TextProperties,
};
