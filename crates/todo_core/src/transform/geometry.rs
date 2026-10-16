use super::Transform;

/// Unclipped bounds in normalized canvas coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformBounds {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl TransformBounds {
    pub fn width(&self) -> f64 {
        self.right - self.left
    }

    pub fn height(&self) -> f64 {
        self.bottom - self.top
    }

    pub fn center(&self) -> (f64, f64) {
        (
            self.left * 0.5 + self.right * 0.5,
            self.top * 0.5 + self.bottom * 0.5,
        )
    }
}

/// Element size at the identity transform, as canvas fractions. Every element is
/// centered on the canvas before its transform: media at its aspect-fit size,
/// text at its laid-out block size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BaseSize {
    pub width: f64,
    pub height: f64,
}

impl BaseSize {
    /// Aspect-fit box of a source with display aspect `source_aspect`.
    pub fn fit(source_aspect: f64, canvas: (f64, f64)) -> Option<Self> {
        if !source_aspect.is_finite()
            || source_aspect <= 0.0
            || ![canvas.0, canvas.1]
                .into_iter()
                .all(|size| size.is_finite() && size > 0.0)
        {
            return None;
        }
        // Ratios keep matching very large/small canvases representable without
        // multiplying source pixels by an overflowing or underflowing fit factor.
        let canvas_aspect = canvas.0 / canvas.1;
        Some(Self {
            width: (source_aspect / canvas_aspect).min(1.0),
            height: (canvas_aspect / source_aspect).min(1.0),
        })
    }

    fn valid(&self) -> bool {
        [self.width, self.height]
            .into_iter()
            .all(|size| size.is_finite() && size > 0.0)
    }
}

impl Transform {
    /// Aspect-fit the source to the canvas, then apply both scale axes and offset.
    /// Transparent elements still have bounds so they remain selectable.
    pub fn bounds(&self, source: (u32, u32), canvas: (f64, f64)) -> Option<TransformBounds> {
        if source.0 == 0 || source.1 == 0 {
            return None;
        }
        self.bounds_for_aspect(f64::from(source.0) / f64::from(source.1), canvas)
    }

    /// Use the decoded display aspect, including pixel aspect and visible aperture.
    /// The caller supplies metadata; this calculation has no decoder dependency.
    pub fn bounds_for_aspect(
        &self,
        source_aspect: f64,
        canvas: (f64, f64),
    ) -> Option<TransformBounds> {
        self.bounds_for_base(BaseSize::fit(source_aspect, canvas)?)
    }

    /// Scale a canvas-centered element of `base` size, then offset it.
    pub fn bounds_for_base(&self, base: BaseSize) -> Option<TransformBounds> {
        if !self.is_valid() || !base.valid() {
            return None;
        }
        let width = base.width * (self.scale * self.scale_x);
        let height = base.height * (self.scale * self.scale_y);
        let center_x = 0.5 + self.x;
        let center_y = 0.5 + self.y;
        let bounds = TransformBounds {
            left: center_x - width * 0.5,
            top: center_y - height * 0.5,
            right: center_x + width * 0.5,
            bottom: center_y + height * 0.5,
        };
        let finite = [bounds.left, bounds.top, bounds.right, bounds.bottom]
            .into_iter()
            .all(f64::is_finite);
        let nondegenerate = [bounds.width(), bounds.height()]
            .into_iter()
            .all(|size| size.is_finite() && size > 0.0);
        (finite && nondegenerate).then_some(bounds)
    }
}
