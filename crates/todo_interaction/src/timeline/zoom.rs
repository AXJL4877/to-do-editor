//! View-space timeline mapping: seconds <-> pixels, zoom, and scroll.
//! Workspace duration lives in `todo_core` (`workspace_duration`).
//!
//! Zoom is a **multiplier** on fit-to-width: at `zoom == 1` the workspace
//! axis exactly fills the visible lane; Alt+wheel only changes the multiplier
//! (never leaves empty panel to the right of the axis).

pub const LABEL_W: f64 = 120.0;
pub const EDGE_PX: f64 = 8.0;
/// Headless / pre-layout density when the board width is not known yet.
pub const PX_PER_SEC_DEFAULT: f64 = 48.0;
pub const LANE_MIN_WIDTH: f64 = 640.0;

/// Fit-to-width multiplier. `1` = workspace fills the lane; larger = zoom in.
pub const ZOOM_DEFAULT: f64 = 1.0;
pub const ZOOM_MIN: f64 = 1.0;
pub const ZOOM_MAX: f64 = 64.0;
/// Per wheel notch. Kept modest so Alt+wheel does not jump a full stop each tick.
pub const ZOOM_FACTOR: f64 = 1.06;

/// Legacy aliases — zoom is a multiplier now; absolute px/sec is derived.
pub const PX_PER_SEC_MIN: f64 = 8.0;
pub const PX_PER_SEC_MAX: f64 = 400.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineScale {
    pub zoom: f64,
    pub lane_width: f64,
    pub px_per_sec: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomHit {
    pub zoom: f64,
    pub scroll_left: f64,
}

#[inline]
pub fn clamp_zoom(zoom: f64) -> f64 {
    if !zoom.is_finite() {
        return ZOOM_DEFAULT;
    }
    zoom.clamp(ZOOM_MIN, ZOOM_MAX)
}

/// Visible lane width inside the board (excludes the track label column).
#[inline]
pub fn lane_view_width(board_view_w: f64) -> f64 {
    if !board_view_w.is_finite() {
        return 0.0;
    }
    (board_view_w - LABEL_W).max(0.0)
}

/// Resolve lane pixel width and px/sec so the workspace fills the panel at zoom 1.
///
/// When `board_view_w` is not yet known (≤ label), keeps a fixed
/// [`PX_PER_SEC_DEFAULT`] density so headless tests stay stable.
#[inline]
pub fn timeline_scale(axis_secs: f64, board_view_w: f64, zoom: f64) -> TimelineScale {
    let zoom = clamp_zoom(zoom);
    let axis = if axis_secs.is_finite() && axis_secs > f64::EPSILON {
        axis_secs
    } else {
        1.0
    };
    let lane_view = lane_view_width(board_view_w);
    if lane_view > 1.0 {
        let lane_width = lane_view * zoom;
        TimelineScale {
            zoom,
            lane_width,
            px_per_sec: lane_width / axis,
        }
    } else {
        let px_per_sec = PX_PER_SEC_DEFAULT * zoom;
        TimelineScale {
            zoom,
            lane_width: (axis * px_per_sec).max(LANE_MIN_WIDTH),
            px_per_sec,
        }
    }
}

/// Wheel zoom that keeps the time under the cursor fixed.
///
/// `view_x` is the pointer x minus the board's left edge. Returns `None` when
/// `alt` is false (UI passes true for Alt/Ctrl+wheel) or the step does not change zoom.
///
/// Sign convention (DOM / product): `delta_y > 0` = scroll down = zoom out;
/// `delta_y < 0` = scroll up = zoom in. The UI should normalize egui's scroll
/// delta to this convention before calling.
pub fn wheel_zoom(
    zoom: f64,
    delta_y: f64,
    view_x: f64,
    scroll_left: f64,
    alt: bool,
    axis_secs: f64,
    board_view_w: f64,
) -> Option<ZoomHit> {
    if !alt || !delta_y.is_finite() || !view_x.is_finite() || !scroll_left.is_finite() {
        return None;
    }
    let prev_zoom = clamp_zoom(zoom);
    let factor = if delta_y < 0.0 {
        ZOOM_FACTOR
    } else {
        1.0 / ZOOM_FACTOR
    };
    let next_zoom = clamp_zoom(prev_zoom * factor);
    if (next_zoom - prev_zoom).abs() < 1e-12 {
        return None;
    }
    let prev = timeline_scale(axis_secs, board_view_w, prev_zoom);
    let next = timeline_scale(axis_secs, board_view_w, next_zoom);
    let cursor_x = view_x + scroll_left;
    let time_under = ((cursor_x - LABEL_W) / prev.px_per_sec).max(0.0);
    let target_x = LABEL_W + time_under * next.px_per_sec;
    Some(ZoomHit {
        zoom: next_zoom,
        scroll_left: (target_x - view_x).max(0.0),
    })
}

#[inline]
pub fn time_at(view_x: f64, scroll_left: f64, px_per_sec: f64) -> f64 {
    if !view_x.is_finite() || !scroll_left.is_finite() || px_per_sec <= f64::EPSILON {
        return 0.0;
    }
    ((view_x - LABEL_W + scroll_left) / px_per_sec).max(0.0)
}

#[inline]
pub fn lane_px(axis_secs: f64, board_view_w: f64, zoom: f64) -> f64 {
    timeline_scale(axis_secs, board_view_w, zoom).lane_width
}

#[inline]
pub fn playhead_x(time: f64, px_per_sec: f64) -> f64 {
    LABEL_W + time.max(0.0) * px_per_sec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_one_fills_the_lane_view() {
        let scale = timeline_scale(56.0, LABEL_W + 800.0, 1.0);
        assert!((scale.lane_width - 800.0).abs() < 1e-9);
        assert!((scale.px_per_sec - 800.0 / 56.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_in_grows_lane_not_shrinks_below_view() {
        let scale = timeline_scale(60.0, LABEL_W + 1000.0, 2.0);
        assert!((scale.lane_width - 2000.0).abs() < 1e-9);
        let out = timeline_scale(60.0, LABEL_W + 1000.0, 0.5);
        assert!(
            (out.lane_width - 1000.0).abs() < 1e-9,
            "zoom cannot go below fit"
        );
    }

    #[test]
    fn zoom_keeps_time_under_cursor() {
        let view_w = LABEL_W + 960.0;
        let axis = 60.0;
        let hit = wheel_zoom(1.0, -1.0, 168.0, 0.0, true, axis, view_w).unwrap();
        assert!((hit.zoom - ZOOM_FACTOR).abs() < 1e-9);
        let before = time_at(168.0, 0.0, timeline_scale(axis, view_w, 1.0).px_per_sec);
        let after = time_at(
            168.0,
            hit.scroll_left,
            timeline_scale(axis, view_w, hit.zoom).px_per_sec,
        );
        assert!((before - after).abs() < 1e-9);
    }

    #[test]
    fn zoom_keeps_time_under_cursor_while_scrolled() {
        let view_w = LABEL_W + 800.0;
        let axis = 120.0;
        let view_x = 420.0;
        let scroll = 180.0;
        let before = time_at(view_x, scroll, timeline_scale(axis, view_w, 2.0).px_per_sec);
        let hit = wheel_zoom(2.0, -1.0, view_x, scroll, true, axis, view_w).unwrap();
        let after = time_at(
            view_x,
            hit.scroll_left,
            timeline_scale(axis, view_w, hit.zoom).px_per_sec,
        );
        assert!((before - after).abs() < 1e-9, "{before} → {after}");
        assert!(hit.scroll_left > scroll, "zoom-in should pan to hold the cursor");
    }

    #[test]
    fn zoom_ignores_wheel_without_alt_and_clamps() {
        let view_w = LABEL_W + 800.0;
        assert!(wheel_zoom(1.0, -1.0, 168.0, 0.0, false, 60.0, view_w).is_none());
        assert!(wheel_zoom(ZOOM_MAX, -1.0, 10.0, 0.0, true, 60.0, view_w).is_none());
        assert!(wheel_zoom(ZOOM_MIN, 1.0, 10.0, 0.0, true, 60.0, view_w).is_none());
    }
}
