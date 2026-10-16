use todo_core::TrimEdge;

use super::zoom::EDGE_PX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipZone {
    In,
    Out,
    Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragKind {
    Seek,
    Move,
    Trim,
    Stretch,
    Marquee,
}

/// In wins when the pointer is inside both edge grabs.
#[inline]
pub fn clip_zone(local_x: f64, width_px: f64) -> ClipZone {
    let edge = EDGE_PX.min(width_px.max(0.0) / 2.0);
    if local_x <= edge {
        ClipZone::In
    } else if local_x >= width_px - edge {
        ClipZone::Out
    } else {
        ClipZone::Body
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DragGesture {
    Seek,
    Move {
        clip_id: String,
        origin_start: f64,
        origin_view_x: f64,
    },
    Trim {
        clip_id: String,
        edge: TrimEdge,
        media_duration: Option<f64>,
    },
    Stretch {
        clip_id: String,
        edge: TrimEdge,
    },
    /// Rubber-band select across time 脳 track rows.
    Marquee {
        origin_time: f64,
        origin_track: usize,
        current_time: f64,
        current_track: usize,
        additive: bool,
        base_selection: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DragSession {
    gesture: DragGesture,
}

impl DragSession {
    pub fn seek() -> Self {
        Self {
            gesture: DragGesture::Seek,
        }
    }

    pub fn marquee(
        origin_time: f64,
        origin_track: usize,
        additive: bool,
        base_selection: Vec<String>,
    ) -> Self {
        Self {
            gesture: DragGesture::Marquee {
                origin_time,
                origin_track,
                current_time: origin_time,
                current_track: origin_track,
                additive,
                base_selection,
            },
        }
    }

    pub fn begin_clip(
        clip_id: String,
        origin_start: f64,
        origin_view_x: f64,
        local_x: f64,
        width_px: f64,
        alt: bool,
        media_duration: Option<f64>,
    ) -> Self {
        let gesture = match clip_zone(local_x, width_px) {
            ClipZone::Body => DragGesture::Move {
                clip_id,
                origin_start,
                origin_view_x,
            },
            zone => {
                let edge = if zone == ClipZone::In {
                    TrimEdge::In
                } else {
                    TrimEdge::Out
                };
                if alt {
                    DragGesture::Stretch { clip_id, edge }
                } else {
                    DragGesture::Trim {
                        clip_id,
                        edge,
                        media_duration,
                    }
                }
            }
        };
        Self { gesture }
    }

    pub fn gesture(&self) -> &DragGesture {
        &self.gesture
    }

    pub fn kind(&self) -> DragKind {
        match self.gesture {
            DragGesture::Seek => DragKind::Seek,
            DragGesture::Move { .. } => DragKind::Move,
            DragGesture::Trim { .. } => DragKind::Trim,
            DragGesture::Stretch { .. } => DragKind::Stretch,
            DragGesture::Marquee { .. } => DragKind::Marquee,
        }
    }

    /// Update marquee corner; returns the new gesture when this session is a marquee.
    pub fn with_marquee_at(&self, time: f64, track_index: usize) -> Option<Self> {
        let DragGesture::Marquee {
            origin_time,
            origin_track,
            additive,
            base_selection,
            ..
        } = &self.gesture
        else {
            return None;
        };
        Some(Self {
            gesture: DragGesture::Marquee {
                origin_time: *origin_time,
                origin_track: *origin_track,
                current_time: time,
                current_track: track_index,
                additive: *additive,
                base_selection: base_selection.clone(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_clip_prefers_in_edge() {
        assert_eq!(clip_zone(5.0, 10.0), ClipZone::In);
        assert_eq!(clip_zone(9.0, 10.0), ClipZone::Out);
        assert_eq!(clip_zone(40.0, 192.0), ClipZone::Body);
        assert_eq!(clip_zone(0.5, 4.0), ClipZone::In);
        assert_eq!(clip_zone(3.5, 4.0), ClipZone::Out);
    }

    #[test]
    fn alt_on_edge_stretches() {
        let trim = DragSession::begin_clip("c".into(), 0.0, 0.0, 190.0, 192.0, false, Some(4.0));
        assert!(matches!(
            trim.gesture(),
            DragGesture::Trim {
                edge: TrimEdge::Out,
                ..
            }
        ));
        let stretch = DragSession::begin_clip("c".into(), 0.0, 0.0, 1.0, 192.0, true, None);
        assert!(matches!(
            stretch.gesture(),
            DragGesture::Stretch {
                edge: TrimEdge::In,
                ..
            }
        ));
    }
}
