mod controller;
pub mod drag_session;
mod hotkeys;
mod marquee;
pub mod snapping;
pub mod zoom;

pub use controller::{TimelineController, TimelineOutcome};
pub use drag_session::{clip_zone, ClipZone, DragGesture, DragKind, DragSession};
pub use hotkeys::tool_for_key;
pub use marquee::{clips_in_marquee, marquee_selection};
pub use zoom::{
    clamp_zoom, lane_px, lane_view_width, playhead_x, time_at, timeline_scale, wheel_zoom,
    TimelineScale, ZoomHit, EDGE_PX, LABEL_W, LANE_MIN_WIDTH, PX_PER_SEC_DEFAULT, PX_PER_SEC_MAX,
    PX_PER_SEC_MIN, ZOOM_DEFAULT, ZOOM_FACTOR, ZOOM_MAX, ZOOM_MIN,
};
