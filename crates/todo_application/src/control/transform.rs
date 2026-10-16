//! Strict command input, independent of the permissive persisted-project format.
use serde::{Deserialize, Deserializer};
use todo_core::Transform;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransformInput {
    x: f64,
    y: f64,
    scale: f64,
    scale_x: f64,
    scale_y: f64,
    opacity: f64,
}

pub(super) fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Transform, D::Error> {
    let input = TransformInput::deserialize(deserializer)?;
    Ok(Transform {
        x: input.x,
        y: input.y,
        scale: input.scale,
        scale_x: input.scale_x,
        scale_y: input.scale_y,
        opacity: input.opacity,
    })
}
