use serde::{Deserialize, Serialize};

/// Center offset in canvas width/height units, aspect-fit size, and opacity.
/// `scale` affects both dimensions; `scale_x` and `scale_y` adjust each axis.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Transform {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub opacity: f64,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            scale_x: 1.0,
            scale_y: 1.0,
            opacity: 1.0,
        }
    }
}

impl Transform {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn is_valid(&self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && [
                self.scale,
                self.scale_x,
                self.scale_y,
                self.scale * self.scale_x,
                self.scale * self.scale_y,
            ]
            .into_iter()
            .all(|scale| scale.is_finite() && scale > 0.0)
            && self.opacity.is_finite()
            && (0.0..=1.0).contains(&self.opacity)
    }
}
