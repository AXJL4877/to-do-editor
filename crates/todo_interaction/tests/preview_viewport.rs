use todo_interaction::PreviewViewport;

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

const HD: (f64, f64) = (1920.0, 1080.0);

#[test]
fn fit_keeps_a_fixed_output_aspect_when_the_panel_changes_shape() {
    let viewport = PreviewViewport::default();
    let wide = viewport.projection((640.0, 360.0), HD).unwrap();
    near(wide.left, 0.0);
    near(wide.width, 1.0);
    let square = viewport.projection((400.0, 400.0), HD).unwrap();
    near(square.top, 0.21875);
    near(square.height, 0.5625);
    near(square.width * 400.0 / (square.height * 400.0), 16.0 / 9.0);
}

#[test]
fn portrait_frame_fills_a_tall_panel() {
    let viewport = PreviewViewport::default();
    let portrait = (1080.0, 1920.0);
    let tall = viewport.projection((360.0, 640.0), portrait).unwrap();
    near(tall.top, 0.0);
    near(tall.height, 1.0);
    near(tall.width * 360.0 / (tall.height * 640.0), 9.0 / 16.0);
}

#[test]
fn zoom_retains_the_point_under_the_pointer_and_inverse_mapping_allows_outside() {
    let mut viewport = PreviewViewport::default();
    viewport.zoom_to(0.5, (0.25, 0.75), (640.0, 360.0), HD);
    let projection = viewport.projection((640.0, 360.0), HD).unwrap();
    near(projection.left, 0.125);
    near(projection.top, 0.375);
    let anchor = projection.to_canvas((0.25, 0.75));
    near(anchor.0, 0.25);
    near(anchor.1, 0.75);
    let outside = projection.to_canvas((0.0, 0.5));
    near(outside.0, -0.25);
    let roundtrip = projection.to_view(outside);
    near(roundtrip.0, 0.0);
    near(roundtrip.1, 0.5);
}

#[test]
fn pan_and_fit_are_independent_of_output_only_and_reject_invalid_values() {
    let mut viewport = PreviewViewport::default();
    viewport.zoom_to(0.5, (0.5, 0.5), (640.0, 360.0), HD);
    viewport.pan_by((0.1, -0.2));
    viewport.set_output_only(true);
    let projection = viewport.projection((640.0, 360.0), HD).unwrap();
    near(projection.left, 0.35);
    near(projection.top, 0.05);
    let before = viewport;
    for zoom in [f64::NAN, f64::INFINITY, -1.0, 0.0] {
        viewport.zoom_to(zoom, (0.5, 0.5), (640.0, 360.0), HD);
    }
    viewport.pan_by((f64::INFINITY, 0.0));
    assert_eq!(viewport, before);
    assert!(viewport.projection((0.0, 360.0), HD).is_none());
    assert!(viewport.projection((640.0, 360.0), (0.0, 9.0)).is_none());
    viewport.fit();
    near(viewport.zoom(), 1.0);
    assert!(viewport.output_only());
    near(viewport.projection((640.0, 360.0), HD).unwrap().left, 0.0);
}

#[test]
fn an_unrepresentable_projected_selection_is_omitted_without_clamping_the_element() {
    let mut viewport = PreviewViewport::default();
    viewport.zoom_to(4.0, (0.5, 0.5), (640.0, 360.0), HD);
    let projection = viewport.projection((640.0, 360.0), HD).unwrap();
    let bounds = todo_interaction::TransformBounds {
        left: f64::MAX / 2.0,
        right: f64::MAX,
        top: 0.0,
        bottom: 1.0,
    };
    assert!(projection.project_bounds(bounds).is_none());
    viewport.fit();
    viewport.zoom_to(1.5, (0.5, 0.5), (640.0, 360.0), HD);
    let bounds = todo_interaction::TransformBounds {
        left: -f64::MAX / 2.0,
        right: f64::MAX / 2.0,
        top: 0.0,
        bottom: 1.0,
    };
    // Individual corners fit in f64; their projected width does not.
    assert!(viewport
        .projection((640.0, 360.0), HD)
        .unwrap()
        .project_bounds(bounds)
        .is_none());
}
