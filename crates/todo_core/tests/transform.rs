use todo_core::*;

#[test]
fn legacy_transform_fields_keep_their_effect_with_default_axes_and_opacity() {
    let transform: ClipTransform =
        serde_json::from_str(r#"{"x":0.25,"y":-0.4,"scale":1.5}"#).unwrap();
    let json = serde_json::to_value(transform).unwrap();
    assert_eq!(json["x"], 0.25);
    assert_eq!(json["y"], -0.4);
    assert_eq!(json["scale"], 1.5);
    assert_eq!(json["scaleX"], 1.0);
    assert_eq!(json["scaleY"], 1.0);
    assert_eq!(json["opacity"], 1.0);
}

#[test]
fn two_dimensional_transform_and_opacity_survive_json_roundtrip() {
    let expected = serde_json::json!({
        "x": -0.3, "y": 0.2, "scale": 0.75,
        "scaleX": 1.5, "scaleY": 0.5, "opacity": 0.4
    });
    let transform: ClipTransform = serde_json::from_value(expected.clone()).unwrap();
    assert!(transform.is_valid());
    assert_eq!(serde_json::to_value(transform).unwrap(), expected);
}

fn sample_project() -> Project {
    let mut project = create_project(None, None, None, None, None);
    project.media.push(MediaItem {
        id: "movie".into(),
        name: "movie.mp4".into(),
        path: Some("movie.mp4".into()),
        duration: 20.0,
        has_audio: Some(true),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Video),
        folder_id: None,
        alpha: None,
        reveal: None,
    });
    add_clip(
        project,
        "v1",
        normalize_clip("clip".into(), "movie".into(), 0.0, 4.0, 0.0, 1.0),
    )
}

#[test]
fn invalid_axis_scales_effective_scale_products_and_opacity_are_rejected() {
    let mut invalid = Vec::new();
    for value in [0.0, -0.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        invalid.extend([
            Transform {
                scale: value,
                ..Transform::default()
            },
            Transform {
                scale_x: value,
                ..Transform::default()
            },
            Transform {
                scale_y: value,
                ..Transform::default()
            },
        ]);
    }
    for value in [-0.1, 1.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        invalid.push(Transform {
            opacity: value,
            ..Transform::default()
        });
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        invalid.extend([
            Transform {
                x: value,
                ..Transform::default()
            },
            Transform {
                y: value,
                ..Transform::default()
            },
        ]);
    }
    invalid.extend([
        Transform {
            scale: f64::MAX,
            scale_x: 2.0,
            ..Transform::default()
        },
        Transform {
            scale: f64::MAX,
            scale_y: 2.0,
            ..Transform::default()
        },
        Transform {
            scale: f64::MIN_POSITIVE,
            scale_x: f64::MIN_POSITIVE,
            ..Transform::default()
        },
        Transform {
            scale: f64::MIN_POSITIVE,
            scale_y: f64::MIN_POSITIVE,
            ..Transform::default()
        },
    ]);
    let base = sample_project();
    for transform in invalid {
        assert!(!transform.is_valid(), "accepted {transform:?}");
        assert_eq!(
            execute_edit(
                base.clone(),
                EditCommand::SetTransform {
                    clip_id: "clip".into(),
                    transform,
                }
            ),
            Err(DomainError::InvalidTransform),
        );
        let mut invalid_document = base.clone();
        invalid_document.tracks[0].clips[0].transform = transform;
        assert_eq!(
            validate_project(&invalid_document),
            Err(DomainError::InvalidTransform)
        );
    }
}

#[test]
fn opacity_endpoints_and_large_finite_positions_are_valid() {
    for opacity in [0.0, 0.5, 1.0] {
        let transform = Transform {
            x: f64::MAX,
            y: -f64::MAX,
            opacity,
            ..Transform::default()
        };
        assert!(transform.is_valid());
        let edited = execute_edit(
            sample_project(),
            EditCommand::SetTransform {
                clip_id: "clip".into(),
                transform,
            },
        )
        .unwrap();
        assert_eq!(
            find_clip(&edited, "clip").unwrap().clip.transform,
            transform
        );
    }
}

#[test]
fn a_clip_only_omits_an_entirely_default_transform() {
    let legacy = serde_json::json!({
        "id": "clip", "mediaId": "movie", "start": 0,
        "duration": 4, "offset": 0, "rate": 1
    });
    let clip: Clip = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(clip.transform, Transform::default());
    assert!(serde_json::to_value(clip)
        .unwrap()
        .get("transform")
        .is_none());
    for field in ["scaleX", "scaleY", "opacity"] {
        let mut json = legacy.clone();
        json["transform"] = serde_json::json!({field: 0.5});
        let clip: Clip = serde_json::from_value(json).unwrap();
        let roundtrip = serde_json::to_value(clip).unwrap();
        assert_eq!(roundtrip["transform"][field], 0.5);
    }
}

fn assert_bounds(actual: TransformBounds, expected: [f64; 4]) {
    for (actual, expected) in [actual.left, actual.top, actual.right, actual.bottom]
        .into_iter()
        .zip(expected)
    {
        assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
    }
}

#[test]
fn bounds_aspect_fit_landscape_portrait_and_square_sources() {
    for (source, canvas, expected) in [
        ((100, 100), (200.0, 100.0), [0.25, 0.0, 0.75, 1.0]),
        ((200, 100), (100.0, 100.0), [0.0, 0.25, 1.0, 0.75]),
        ((100, 200), (200.0, 100.0), [0.375, 0.0, 0.625, 1.0]),
        ((1920, 1080), (1600.0, 900.0), [0.0, 0.0, 1.0, 1.0]),
    ] {
        assert_bounds(
            Transform::default().bounds(source, canvas).unwrap(),
            expected,
        );
    }
}

#[test]
fn bounds_use_decoded_display_aspect_for_non_square_pixels() {
    // PAL 720×576 with SAR 16:15 displays at 4:3, rather than the raw 5:4.
    let transform = Transform::default();
    assert_bounds(
        transform
            .bounds_for_aspect((720.0 / 576.0) * (16.0 / 15.0), (640.0, 360.0))
            .unwrap(),
        [0.125, 0.0, 0.875, 1.0],
    );
    assert_eq!(
        transform.bounds((1920, 1080), (640.0, 360.0)),
        transform.bounds_for_aspect(1920.0 / 1080.0, (640.0, 360.0)),
    );
    for aspect in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(transform
            .bounds_for_aspect(aspect, (640.0, 360.0))
            .is_none());
    }
}

#[test]
fn bounds_apply_center_offsets_uniform_and_independent_axis_scales() {
    let transform = Transform {
        x: 0.1,
        y: -0.2,
        scale: 2.0,
        scale_x: 0.5,
        scale_y: 0.25,
        opacity: 0.0,
    };
    let bounds = transform.bounds((1920, 1080), (1600.0, 900.0)).unwrap();
    assert_bounds(bounds, [0.1, 0.05, 1.1, 0.55]);
    assert!((bounds.width() - 1.0).abs() < 1e-12);
    assert!((bounds.height() - 0.5).abs() < 1e-12);
    let center = bounds.center();
    assert!((center.0 - 0.6).abs() < 1e-12);
    assert!((center.1 - 0.3).abs() < 1e-12);
    assert_eq!(
        bounds,
        transform.bounds((1920, 1080), (800.0, 450.0)).unwrap()
    );
}

#[test]
fn bounds_are_unclipped_when_the_element_is_outside_the_canvas() {
    let transform = Transform {
        x: -1.0,
        y: 1.0,
        ..Transform::default()
    };
    assert_bounds(
        transform.bounds((100, 100), (100.0, 100.0)).unwrap(),
        [-1.0, 1.0, 0.0, 2.0],
    );
}

#[test]
fn bounds_reject_invalid_dimensions_and_unrepresentable_geometry() {
    for source in [(0, 1080), (1920, 0), (0, 0)] {
        assert!(Transform::default()
            .bounds(source, (1600.0, 900.0))
            .is_none());
    }
    for value in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for canvas in [(value, 100.0), (100.0, value)] {
            assert!(Transform::default().bounds((100, 100), canvas).is_none());
        }
    }
    for transform in [
        Transform {
            scale: 0.0,
            ..Transform::default()
        },
        Transform {
            scale: f64::MIN_POSITIVE,
            ..Transform::default()
        },
        Transform {
            x: f64::MAX,
            ..Transform::default()
        },
        Transform {
            opacity: f64::NAN,
            ..Transform::default()
        },
    ] {
        assert!(
            transform.bounds((100, 100), (100.0, 100.0)).is_none(),
            "{transform:?}"
        );
    }
}

#[test]
fn bounds_handle_matching_extreme_canvas_sizes_without_intermediate_overflow() {
    for canvas in [(f64::MAX, f64::MAX), (f64::MIN_POSITIVE, f64::MIN_POSITIVE)] {
        assert_bounds(
            Transform::default()
                .bounds((u32::MAX, u32::MAX), canvas)
                .unwrap(),
            [0.0, 0.0, 1.0, 1.0],
        );
    }
}

#[test]
fn pixel_placement_maps_source_pixels_onto_the_canvas() {
    // 1080p on a 4K canvas should land at half scale (not aspect-fit fill).
    let transform = Transform::for_pixel_placement((1920, 1080), (3840, 2160));
    assert!((transform.scale - 0.5).abs() < 1e-12, "{transform:?}");
    assert_eq!(transform.x, 0.0);
    assert_eq!(transform.y, 0.0);
    // Matching canvas keeps identity.
    assert_eq!(
        Transform::for_pixel_placement((1920, 1080), (1920, 1080)).scale,
        1.0
    );
    // Unknown sizes fall back to aspect-fit fill.
    assert!(Transform::for_pixel_placement((0, 1080), (1920, 1080)).is_default());
}

#[test]
fn place_media_defaults_to_pixel_scale_when_dimensions_are_known() {
    let mut project = create_project(None, None, None, None, None);
    project.width = 3840;
    project.height = 2160;
    project.media.push(MediaItem {
        id: "m".into(),
        name: "A.mp4".into(),
        path: Some("A.mp4".into()),
        duration: 5.0,
        has_audio: Some(false),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Video),
        folder_id: None,
        alpha: None,
        reveal: None,
    });
    let next = place_media(project, "m", Some("v1"), 0.0).unwrap();
    let clip = &next.tracks.iter().find(|t| t.id == "v1").unwrap().clips[0];
    assert!(
        (clip.transform.scale - 0.5).abs() < 1e-12,
        "{:?}",
        clip.transform
    );
}

#[test]
fn match_frame_placement_fills_the_program_canvas() {
    let mut project = create_project(None, None, None, None, None);
    project.width = 3840;
    project.height = 2160;
    project.media_scale = MediaScaleMode::MatchFrame;
    project.media.push(MediaItem {
        id: "m".into(),
        name: "A.mp4".into(),
        path: Some("A.mp4".into()),
        duration: 5.0,
        has_audio: Some(false),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Video),
        folder_id: None,
        alpha: None,
        reveal: None,
    });
    let next = place_media(project, "m", Some("v1"), 0.0).unwrap();
    let clip = &next.tracks.iter().find(|t| t.id == "v1").unwrap().clips[0];
    assert!(clip.transform.is_default(), "{:?}", clip.transform);
}

#[test]
fn media_scale_mode_survives_json_and_edits() {
    let project = execute_edit(
        create_project(None, None, None, None, None),
        EditCommand::SetMediaScaleMode {
            mode: MediaScaleMode::MatchFrame,
        },
    )
    .unwrap();
    assert_eq!(project.media_scale, MediaScaleMode::MatchFrame);
    let json = serde_json::to_value(&project).unwrap();
    assert_eq!(json["mediaScale"], "matchFrame");
    let back: Project = serde_json::from_value(json).unwrap();
    assert_eq!(back.media_scale, MediaScaleMode::MatchFrame);
}
