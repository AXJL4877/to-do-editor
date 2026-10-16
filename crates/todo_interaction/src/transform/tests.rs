use super::hit_test::{hit_test, Handle, TransformHit};
use super::TransformGesture;
use todo_core::{BaseSize, Transform, TransformBounds};

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

fn at(bounds: TransformBounds, sx: f64, sy: f64) -> (f64, f64) {
    let (x, y) = bounds.center();
    (
        x + sx * bounds.width() / 2.0,
        y + sy * bounds.height() / 2.0,
    )
}

#[test]
fn all_handles_precede_the_body_and_use_eight_logical_pixels() {
    let canvas = (1000.0, 500.0);
    let bounds = Transform::default().bounds((300, 600), canvas).unwrap();
    for handle in [
        Handle::TopLeft,
        Handle::TopRight,
        Handle::BottomRight,
        Handle::BottomLeft,
        Handle::Left,
        Handle::Right,
        Handle::Top,
        Handle::Bottom,
    ] {
        let (sx, sy) = handle.axes();
        let point = at(bounds, sx, sy);
        assert_eq!(
            hit_test(bounds, point, canvas, true),
            Some(TransformHit::Resize(handle))
        );
    }
    assert_eq!(
        hit_test(bounds, (bounds.right + 7.9 / canvas.0, 0.5), canvas, true),
        Some(TransformHit::Resize(Handle::Right))
    );
    assert_eq!(
        hit_test(bounds, (bounds.right + 8.1 / canvas.0, 0.5), canvas, true),
        None
    );
    assert_eq!(
        hit_test(bounds, (bounds.right + 7.9 / canvas.0, 0.5), canvas, false),
        None
    );
    assert_eq!(
        hit_test(bounds, bounds.center(), canvas, true),
        Some(TransformHit::Move)
    );
    assert_eq!(hit_test(bounds, (0.1, 0.5), canvas, true), None);
}

#[test]
fn tiny_element_chooses_the_nearest_handle_when_hit_regions_overlap() {
    let canvas = (1000.0, 500.0);
    let bounds = TransformBounds {
        left: 0.5,
        top: 0.5,
        right: 0.504,
        bottom: 0.508,
    };
    assert_eq!(
        hit_test(bounds, (bounds.right, bounds.bottom), canvas, true),
        Some(TransformHit::Resize(Handle::BottomRight))
    );
    assert_eq!(
        hit_test(bounds, (bounds.right, bounds.center().1), canvas, true),
        Some(TransformHit::Resize(Handle::Right))
    );
    assert_eq!(
        hit_test(bounds, bounds.center(), canvas, true),
        Some(TransformHit::Resize(Handle::Left))
    );
}

#[test]
fn four_edges_resize_one_axis_and_keep_the_opposite_edge_fixed() {
    let source = 0.5;
    let canvas = (1000.0, 500.0);
    let original = Transform {
        x: 0.1,
        y: -0.2,
        scale: 0.8,
        scale_x: 1.3,
        scale_y: 0.6,
        opacity: 0.25,
    };
    let before = original.bounds_for_aspect(source, canvas).unwrap();
    for handle in [Handle::Left, Handle::Right, Handle::Top, Handle::Bottom] {
        let (sx, sy) = handle.axes();
        let start = at(before, sx, sy);
        let gesture = TransformGesture::new(
            original,
            BaseSize::fit(source, canvas).unwrap(),
            start,
            TransformHit::Resize(handle),
        );
        let moved = (
            start.0 + sx * before.width() / 2.0,
            start.1 + sy * before.height() / 2.0,
        );
        let transform = gesture.update(moved, canvas, &[], false).unwrap().transform;
        let after = transform.bounds_for_aspect(source, canvas).unwrap();
        let anchor = at(before, -sx, -sy);
        let new_anchor = at(after, -sx, -sy);
        close(new_anchor.0, anchor.0);
        close(new_anchor.1, anchor.1);
        close(transform.scale, original.scale);
        close(
            transform.scale_x,
            original.scale_x * if sx == 0.0 { 1.0 } else { 1.5 },
        );
        close(
            transform.scale_y,
            original.scale_y * if sy == 0.0 { 1.0 } else { 1.5 },
        );
        assert_eq!(transform.opacity, original.opacity);
    }
}

#[test]
fn four_corners_scale_proportionally_about_the_opposite_corner() {
    let source = 0.5;
    let canvas = (1000.0, 500.0);
    let original = Transform {
        x: 0.1,
        y: -0.2,
        scale: 0.8,
        scale_x: 1.3,
        scale_y: 0.6,
        opacity: 0.0,
    };
    let before = original.bounds_for_aspect(source, canvas).unwrap();
    for handle in [
        Handle::TopLeft,
        Handle::TopRight,
        Handle::BottomRight,
        Handle::BottomLeft,
    ] {
        let (sx, sy) = handle.axes();
        let start = at(before, sx, sy);
        let gesture = TransformGesture::new(
            original,
            BaseSize::fit(source, canvas).unwrap(),
            start,
            TransformHit::Resize(handle),
        );
        let moved = (
            start.0 + sx * before.width() / 2.0,
            start.1 + sy * before.height() / 2.0,
        );
        let transform = gesture.update(moved, canvas, &[], false).unwrap().transform;
        let after = transform.bounds_for_aspect(source, canvas).unwrap();
        let anchor = at(before, -sx, -sy);
        let new_anchor = at(after, -sx, -sy);
        close(new_anchor.0, anchor.0);
        close(new_anchor.1, anchor.1);
        close(
            after.width() / after.height(),
            before.width() / before.height(),
        );
        close(transform.scale, original.scale * 1.5);
        assert_eq!(transform.scale_x, original.scale_x);
        assert_eq!(transform.scale_y, original.scale_y);
        assert_eq!(transform.opacity, original.opacity);
    }
}

#[test]
fn corner_projection_measures_the_pointer_in_logical_canvas_pixels() {
    let canvas = (1000.0, 500.0);
    let original = Transform::default();
    let gesture = TransformGesture::new(
        original,
        BaseSize::fit(0.5, canvas).unwrap(),
        (0.625, 1.0),
        TransformHit::Resize(Handle::BottomRight),
    );
    let transform = gesture
        .update((0.875, 1.0), canvas, &[], false)
        .unwrap()
        .transform;
    // The diagonal is 250 by 500 pixels; moving 250 horizontally projects
    // 20% of that diagonal, independently of normalized canvas proportions.
    close(transform.scale, 1.2);
    let bounds = transform.bounds_for_aspect(0.5, canvas).unwrap();
    close(bounds.left, 0.375);
    close(bounds.top, 0.0);
}

#[test]
fn every_resize_direction_stops_at_one_logical_pixel_without_flipping() {
    let source = 0.5;
    let canvas = (1000.0, 500.0);
    let original = Transform::default();
    let before = original.bounds_for_aspect(source, canvas).unwrap();
    for handle in [
        Handle::TopLeft,
        Handle::TopRight,
        Handle::BottomRight,
        Handle::BottomLeft,
        Handle::Left,
        Handle::Right,
        Handle::Top,
        Handle::Bottom,
    ] {
        let (sx, sy) = handle.axes();
        let start = at(before, sx, sy);
        let gesture = TransformGesture::new(
            original,
            BaseSize::fit(source, canvas).unwrap(),
            start,
            TransformHit::Resize(handle),
        );
        let moved = (start.0 - sx * 2.0, start.1 - sy * 2.0);
        let transform = gesture.update(moved, canvas, &[], false).unwrap().transform;
        let after = transform.bounds_for_aspect(source, canvas).unwrap();
        assert!(after.width() * canvas.0 >= 1.0 - 1e-9);
        assert!(after.height() * canvas.1 >= 1.0 - 1e-9);
        let anchor = at(before, -sx, -sy);
        let new_anchor = at(after, -sx, -sy);
        close(new_anchor.0, anchor.0);
        close(new_anchor.1, anchor.1);
        assert!(transform.scale > 0.0 && transform.scale_x > 0.0 && transform.scale_y > 0.0);
    }
}

#[test]
fn moving_preserves_all_size_and_opacity_parameters() {
    let original = Transform {
        scale: 0.8,
        scale_x: 1.3,
        scale_y: 0.6,
        opacity: 0.25,
        ..Transform::default()
    };
    let gesture = TransformGesture::new(
        original,
        BaseSize::fit(0.5, (1000.0, 500.0)).unwrap(),
        (0.5, 0.5),
        TransformHit::Move,
    );
    let transform = gesture
        .update((1.5, -0.5), (1000.0, 500.0), &[], false)
        .unwrap()
        .transform;
    assert_eq!(
        transform,
        Transform {
            x: 1.0,
            y: -1.0,
            ..original
        }
    );
}

#[test]
fn move_snaps_element_center_to_peer_center_with_guide() {
    let canvas = (800.0, 450.0);
    let peer = Transform {
        x: 0.2,
        y: -0.1,
        ..Transform::default()
    }
    .bounds_for_aspect(16.0 / 9.0, canvas)
    .unwrap();
    let gesture = TransformGesture::new(
        Transform::default(),
        BaseSize::fit(16.0 / 9.0, canvas).unwrap(),
        (0.5, 0.5),
        TransformHit::Move,
    );
    // Nudge near the peer center.
    let near = (0.5 + 0.2 + 0.005, 0.5 - 0.1 - 0.004);
    let update = gesture.update(near, canvas, &[peer], true).unwrap();
    let after = update
        .transform
        .bounds_for_aspect(16.0 / 9.0, canvas)
        .unwrap();
    close(after.center().0, peer.center().0);
    close(after.center().1, peer.center().1);
    assert!(!update.guides.is_empty());
}
