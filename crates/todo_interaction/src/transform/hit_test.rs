use todo_core::TransformBounds;

pub(super) const HANDLE_RADIUS_PX: f64 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Handle {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
    Left,
    Right,
    Top,
    Bottom,
}

impl Handle {
    pub(super) fn axes(self) -> (f64, f64) {
        match self {
            Self::TopLeft => (-1.0, -1.0),
            Self::TopRight => (1.0, -1.0),
            Self::BottomRight => (1.0, 1.0),
            Self::BottomLeft => (-1.0, 1.0),
            Self::Left => (-1.0, 0.0),
            Self::Right => (1.0, 0.0),
            Self::Top => (0.0, -1.0),
            Self::Bottom => (0.0, 1.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransformHit {
    Move,
    Resize(Handle),
}

pub(crate) fn hit_test(
    bounds: TransformBounds,
    pointer: (f64, f64),
    canvas: (f64, f64),
    selected: bool,
) -> Option<TransformHit> {
    let (x, y) = pointer;
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    if selected {
        let (center_x, center_y) = bounds.center();
        let mut closest = None;
        // Nearest wins when small elements have overlapping hit regions.
        // Corners precede edge centers only for equal-distance ties.
        for (handle, hx, hy) in [
            (Handle::TopLeft, bounds.left, bounds.top),
            (Handle::TopRight, bounds.right, bounds.top),
            (Handle::BottomRight, bounds.right, bounds.bottom),
            (Handle::BottomLeft, bounds.left, bounds.bottom),
            (Handle::Left, bounds.left, center_y),
            (Handle::Right, bounds.right, center_y),
            (Handle::Top, center_x, bounds.top),
            (Handle::Bottom, center_x, bounds.bottom),
        ] {
            let dx = (x - hx) * canvas.0;
            let dy = (y - hy) * canvas.1;
            let distance = dx * dx + dy * dy;
            if dx.abs() <= HANDLE_RADIUS_PX
                && dy.abs() <= HANDLE_RADIUS_PX
                && closest.is_none_or(|(best, _)| distance < best)
            {
                closest = Some((distance, handle));
            }
        }
        if let Some((_, handle)) = closest {
            return Some(TransformHit::Resize(handle));
        }
    }
    (x >= bounds.left && x <= bounds.right && y >= bounds.top && y <= bounds.bottom)
        .then_some(TransformHit::Move)
}
