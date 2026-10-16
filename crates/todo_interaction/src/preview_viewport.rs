//! Editor-only camera over the project program frame. No decoder state.

use todo_core::TransformBounds;

pub const PREVIEW_ZOOM_MIN: f64 = 0.1;
pub const PREVIEW_ZOOM_MAX: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreviewViewport {
    zoom: f64,
    pan: (f64, f64),
    output_only: bool,
}

impl Default for PreviewViewport {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: (0.0, 0.0),
            output_only: false,
        }
    }
}

/// Composition rectangle in normalized preview-window coordinates, unclipped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreviewProjection {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl PreviewProjection {
    pub fn to_canvas(self, point: (f64, f64)) -> (f64, f64) {
        (
            (point.0 - self.left) / self.width,
            (point.1 - self.top) / self.height,
        )
    }

    pub fn to_view(self, point: (f64, f64)) -> (f64, f64) {
        (
            self.left + point.0 * self.width,
            self.top + point.1 * self.height,
        )
    }

    pub fn canvas_size(self, window: (f64, f64)) -> (f64, f64) {
        (self.width * window.0, self.height * window.1)
    }

    pub fn project_bounds(self, bounds: TransformBounds) -> Option<TransformBounds> {
        let (left, top) = self.to_view((bounds.left, bounds.top));
        let (right, bottom) = self.to_view((bounds.right, bounds.bottom));
        [left, top, right, bottom, right - left, bottom - top]
            .iter()
            .all(|value| value.is_finite())
            .then_some(TransformBounds {
                left,
                top,
                right,
                bottom,
            })
            .filter(|bounds| bounds.left < bounds.right && bounds.top < bounds.bottom)
    }
}

impl PreviewViewport {
    pub fn zoom(self) -> f64 {
        self.zoom
    }
    pub fn output_only(self) -> bool {
        self.output_only
    }
    pub fn set_output_only(&mut self, value: bool) {
        self.output_only = value;
    }

    /// Fit the program frame (`frame` = project width×height in any proportional units)
    /// inside the preview window, then apply zoom/pan.
    pub fn projection(self, window: (f64, f64), frame: (f64, f64)) -> Option<PreviewProjection> {
        let (w, h) = window;
        let (fw, fh) = frame;
        if !w.is_finite()
            || !h.is_finite()
            || w <= 0.0
            || h <= 0.0
            || !fw.is_finite()
            || !fh.is_finite()
            || fw <= 0.0
            || fh <= 0.0
        {
            return None;
        }
        let unit = (w / fw).min(h / fh) * self.zoom;
        let width = unit * fw / w;
        let height = unit * fh / h;
        let left = (1.0 - width) * 0.5 + self.pan.0;
        let top = (1.0 - height) * 0.5 + self.pan.1;
        [left, top, width, height]
            .iter()
            .all(|n| n.is_finite())
            .then_some(PreviewProjection {
                left,
                top,
                width,
                height,
            })
            .filter(|p| p.width > 0.0 && p.height > 0.0)
    }

    pub fn zoom_to(
        &mut self,
        zoom: f64,
        anchor: (f64, f64),
        window: (f64, f64),
        frame: (f64, f64),
    ) {
        if !zoom.is_finite() || zoom <= 0.0 || !anchor.0.is_finite() || !anchor.1.is_finite() {
            return;
        }
        let Some(old) = self.projection(window, frame) else {
            return;
        };
        let point = old.to_canvas(anchor);
        let mut next = *self;
        next.zoom = zoom.clamp(PREVIEW_ZOOM_MIN, PREVIEW_ZOOM_MAX);
        let Some(projection) = next.projection(window, frame) else {
            return;
        };
        let moved = projection.to_view(point);
        next.pan_by((anchor.0 - moved.0, anchor.1 - moved.1));
        if next.projection(window, frame).is_some() {
            *self = next;
        }
    }

    pub fn pan_by(&mut self, delta: (f64, f64)) {
        let pan = (self.pan.0 + delta.0, self.pan.1 + delta.1);
        if pan.0.is_finite() && pan.1.is_finite() {
            self.pan = pan;
        }
    }

    pub fn fit(&mut self) {
        self.zoom = 1.0;
        self.pan = (0.0, 0.0);
    }
}
