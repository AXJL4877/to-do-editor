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

    /// Place media so one source pixel maps to one canvas pixel (PR-style default).
    /// Falls back to identity (aspect-fit fill) when sizes are unknown.
    pub fn for_pixel_placement(source: (u32, u32), canvas: (u32, u32)) -> Self {
        let Some(pixel) = super::BaseSize::pixel(source, canvas) else {
            return Self::default();
        };
        let Some(fit) = super::BaseSize::fit(
            f64::from(source.0) / f64::from(source.1),
            (f64::from(canvas.0), f64::from(canvas.1)),
        ) else {
            return Self::default();
        };
        if fit.width <= 0.0 || !fit.width.is_finite() {
            return Self::default();
        }
        let scale = pixel.width / fit.width;
        if !scale.is_finite() || scale <= 0.0 {
            return Self::default();
        }
        Self {
            scale,
            ..Self::default()
        }
    }

    /// Placement transform for known source pixels under the project media-scale mode.
    pub fn for_media_placement(
        source: (u32, u32),
        canvas: (u32, u32),
        mode: crate::MediaScaleMode,
    ) -> Self {
        match mode {
            crate::MediaScaleMode::Pixel => Self::for_pixel_placement(source, canvas),
            crate::MediaScaleMode::MatchFrame => Self::default(),
        }
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
