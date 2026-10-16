//! Smart alignment for program-canvas transforms.
//!
//! Targets: composition frame edges/center, plus peer element edges/centers.
//! Threshold matches gizmo handle size so snap feels consistent with hit testing.

use super::hit_test::HANDLE_RADIUS_PX;
use todo_core::TransformBounds;

/// Composition frame in normalized canvas coordinates.
pub(crate) const FRAME: TransformBounds = TransformBounds {
    left: 0.0,
    top: 0.0,
    right: 1.0,
    bottom: 1.0,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignAxis {
    /// Constant X (vertical guide line).
    Vertical,
    /// Constant Y (horizontal guide line).
    Horizontal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignGuide {
    pub axis: AlignAxis,
    /// Position in normalized canvas coordinates.
    pub position: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlignSnap {
    pub delta: (f64, f64),
    pub guides: Vec<AlignGuide>,
}

impl Default for AlignSnap {
    fn default() -> Self {
        Self {
            delta: (0.0, 0.0),
            guides: Vec::new(),
        }
    }
}

fn edges_x(bounds: TransformBounds) -> [f64; 3] {
    [bounds.left, bounds.center().0, bounds.right]
}

fn edges_y(bounds: TransformBounds) -> [f64; 3] {
    [bounds.top, bounds.center().1, bounds.bottom]
}

fn collect_targets(peers: &[TransformBounds]) -> (Vec<f64>, Vec<f64>) {
    let mut xs = vec![FRAME.left, FRAME.center().0, FRAME.right];
    let mut ys = vec![FRAME.top, FRAME.center().1, FRAME.bottom];
    for peer in peers {
        xs.extend(edges_x(*peer));
        ys.extend(edges_y(*peer));
    }
    (xs, ys)
}

fn best_snap(moving: &[f64], targets: &[f64], threshold: f64) -> Option<(f64, f64)> {
    let mut best: Option<(f64, f64, f64)> = None; // abs_delta, delta, target
    for &edge in moving {
        if !edge.is_finite() {
            continue;
        }
        for &target in targets {
            if !target.is_finite() {
                continue;
            }
            let delta = target - edge;
            let abs = delta.abs();
            if abs <= threshold && best.is_none_or(|(best_abs, _, _)| abs < best_abs) {
                best = Some((abs, delta, target));
            }
        }
    }
    best.map(|(_, delta, target)| (delta, target))
}

/// Snap a free move so element edges/centers align to the frame or peers.
pub(crate) fn snap_move(
    moving: TransformBounds,
    peers: &[TransformBounds],
    canvas_px: (f64, f64),
    enabled: bool,
) -> AlignSnap {
    if !enabled || canvas_px.0 <= 0.0 || canvas_px.1 <= 0.0 {
        return AlignSnap::default();
    }
    let thresh_x = HANDLE_RADIUS_PX / canvas_px.0;
    let thresh_y = HANDLE_RADIUS_PX / canvas_px.1;
    let (targets_x, targets_y) = collect_targets(peers);
    let mut guides = Vec::new();
    let mut dx = 0.0;
    let mut dy = 0.0;
    if let Some((delta, target)) = best_snap(&edges_x(moving), &targets_x, thresh_x) {
        dx = delta;
        guides.push(AlignGuide {
            axis: AlignAxis::Vertical,
            position: target,
        });
    }
    if let Some((delta, target)) = best_snap(&edges_y(moving), &targets_y, thresh_y) {
        dy = delta;
        guides.push(AlignGuide {
            axis: AlignAxis::Horizontal,
            position: target,
        });
    }
    AlignSnap {
        delta: (dx, dy),
        guides,
    }
}

/// Snap only the edges that a resize handle is driving.
/// `sx`/`sy` are handle axes: -1 left/top, +1 right/bottom, 0 untouched.
pub(crate) fn snap_resize(
    moving: TransformBounds,
    peers: &[TransformBounds],
    canvas_px: (f64, f64),
    sx: f64,
    sy: f64,
    enabled: bool,
) -> AlignSnap {
    if !enabled || canvas_px.0 <= 0.0 || canvas_px.1 <= 0.0 {
        return AlignSnap::default();
    }
    let thresh_x = HANDLE_RADIUS_PX / canvas_px.0;
    let thresh_y = HANDLE_RADIUS_PX / canvas_px.1;
    let (targets_x, targets_y) = collect_targets(peers);
    let mut guides = Vec::new();
    let mut dx = 0.0;
    let mut dy = 0.0;
    if sx != 0.0 {
        let edge = if sx < 0.0 { moving.left } else { moving.right };
        if let Some((delta, target)) = best_snap(&[edge], &targets_x, thresh_x) {
            dx = delta;
            guides.push(AlignGuide {
                axis: AlignAxis::Vertical,
                position: target,
            });
        }
    }
    if sy != 0.0 {
        let edge = if sy < 0.0 { moving.top } else { moving.bottom };
        if let Some((delta, target)) = best_snap(&[edge], &targets_y, thresh_y) {
            dy = delta;
            guides.push(AlignGuide {
                axis: AlignAxis::Horizontal,
                position: target,
            });
        }
    }
    AlignSnap {
        delta: (dx, dy),
        guides,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_at(cx: f64, cy: f64, w: f64, h: f64) -> TransformBounds {
        TransformBounds {
            left: cx - w * 0.5,
            top: cy - h * 0.5,
            right: cx + w * 0.5,
            bottom: cy + h * 0.5,
        }
    }

    #[test]
    fn move_snaps_centers_to_frame_and_emits_guides() {
        // Threshold at 800×450 is 8px → 0.01× / ≈0.0178 y.
        let moving = box_at(0.505, 0.495, 0.2, 0.2);
        let snap = snap_move(moving, &[], (800.0, 450.0), true);
        assert!((snap.delta.0 + 0.005).abs() < 1e-12);
        assert!((snap.delta.1 - 0.005).abs() < 1e-12);
        assert!(snap
            .guides
            .iter()
            .any(|g| g.axis == AlignAxis::Vertical && (g.position - 0.5).abs() < 1e-12));
        assert!(snap
            .guides
            .iter()
            .any(|g| g.axis == AlignAxis::Horizontal && (g.position - 0.5).abs() < 1e-12));
    }

    #[test]
    fn move_snaps_left_edge_to_peer_right_edge() {
        // Offset Y so centers stay clear of the frame mid-line.
        let peer = box_at(0.25, 0.3, 0.2, 0.2); // right = 0.35
        let moving = box_at(0.455, 0.7, 0.2, 0.2); // left = 0.355
        let snap = snap_move(moving, &[peer], (800.0, 450.0), true);
        assert!((snap.delta.0 + 0.005).abs() < 1e-12);
        assert_eq!(snap.delta.1, 0.0);
        assert_eq!(
            snap.guides,
            vec![AlignGuide {
                axis: AlignAxis::Vertical,
                position: 0.35
            }]
        );
    }

    #[test]
    fn move_snaps_to_program_left_edge() {
        let moving = box_at(0.105, 0.5, 0.2, 0.2); // left = 0.005
        let snap = snap_move(moving, &[], (800.0, 450.0), true);
        assert!((snap.delta.0 + 0.005).abs() < 1e-12);
        assert!(snap
            .guides
            .iter()
            .any(|g| g.axis == AlignAxis::Vertical && g.position == 0.0));
    }

    #[test]
    fn disabled_snapping_is_a_no_op() {
        let moving = box_at(0.505, 0.495, 0.2, 0.2);
        assert_eq!(
            snap_move(moving, &[], (800.0, 450.0), false),
            AlignSnap::default()
        );
    }

    #[test]
    fn resize_snaps_only_the_driven_edge() {
        let moving = box_at(0.5, 0.5, 0.2, 0.2); // right=0.6
        let peer = box_at(0.8, 0.5, 0.2, 0.2); // left=0.7
                                               // Nudge right edge toward peer left.
        let mut nudged = moving;
        nudged.right = 0.705;
        let snap = snap_resize(nudged, &[peer], (800.0, 450.0), 1.0, 0.0, true);
        assert!((snap.delta.0 + 0.005).abs() < 1e-12);
        assert_eq!(snap.delta.1, 0.0);
        assert_eq!(snap.guides.len(), 1);
        assert_eq!(snap.guides[0].axis, AlignAxis::Vertical);
        assert!((snap.guides[0].position - 0.7).abs() < 1e-12);
    }
}
