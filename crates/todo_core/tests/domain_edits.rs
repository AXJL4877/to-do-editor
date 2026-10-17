use todo_core::*;

fn project() -> Project {
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
    project
}

fn put(project: Project, id: &str, start: f64, duration: f64) -> Project {
    add_clip(
        project,
        "v1",
        normalize_clip(id.into(), "movie".into(), start, duration, 0.0, 1.0),
    )
}

#[test]
fn public_add_rejects_overlap_and_silent_video_on_audio_track() {
    let mut project = put(project(), "first", 0.0, 4.0);
    assert_eq!(put(project.clone(), "overlap", 2.0, 4.0), project);
    project.media[0].has_audio = Some(false);
    let audio = normalize_clip("silent".into(), "movie".into(), 5.0, 2.0, 0.0, 1.0);
    assert_eq!(add_clip(project.clone(), "a1", audio), project);
}

#[test]
fn public_move_spills_overlaps_and_rejects_visual_to_audio_reassignment() {
    let project = put(put(project(), "first", 0.0, 4.0), "block", 5.0, 4.0);
    let moved = move_clip(project.clone(), "first", 3.0, None);
    assert_eq!(find_clip(&moved, "first").unwrap().track_id, "v2");
    assert_eq!(find_clip(&moved, "first").unwrap().clip.start, 3.0);
    assert_eq!(
        move_clip(project.clone(), "first", 0.0, Some("a1")),
        project
    );
}

#[test]
fn trim_uses_media_bounds_even_when_caller_omits_duration() {
    let project = put(project(), "first", 0.0, 4.0);
    let next = trim(project, "first", TrimEdge::Out, 30.0, None);
    assert_eq!(find_clip(&next, "first").unwrap().clip.duration, 20.0);
}

#[test]
fn trim_rejects_overlap_and_keeps_at_least_one_project_frame() {
    let project = put(put(project(), "first", 0.0, 4.0), "block", 5.0, 4.0);
    assert_eq!(
        trim(project.clone(), "first", TrimEdge::Out, 6.0, None),
        project
    );
    let short = trim(project, "first", TrimEdge::Out, 0.001, None);
    assert!((find_clip(&short, "first").unwrap().clip.duration - 1.0 / 30.0).abs() < 1e-9);
}

#[test]
fn still_images_can_extend_beyond_default_hold() {
    let mut project = project();
    project.media[0].kind = Some(MediaKind::Image);
    project.media[0].duration = 5.0;
    let project = put(project, "still", 0.0, 4.0);
    let next = trim(project, "still", TrimEdge::Out, 40.0, Some(5.0));
    assert_eq!(find_clip(&next, "still").unwrap().clip.duration, 40.0);
}

#[test]
fn legacy_json_defaults_transform_and_custom_transform_survives_all_edits() {
    let legacy = r#"{"id":"clip","mediaId":"movie","start":1,"duration":4,"offset":1,"rate":1}"#;
    let mut clip: Clip = serde_json::from_str(legacy).unwrap();
    assert_eq!(clip.transform, ClipTransform::default());
    assert!(serde_json::to_value(&clip)
        .unwrap()
        .get("transform")
        .is_none());
    clip.transform = ClipTransform {
        x: 0.25,
        y: -0.4,
        scale: 1.5,
        scale_x: 0.6,
        scale_y: 1.4,
        opacity: 0.35,
    };
    let transform = clip.transform;
    let original = add_clip(project(), "v1", clip);
    let normalized = create_project(
        None,
        None,
        None,
        Some(original.media.clone()),
        Some(original.tracks.clone()),
    );
    assert_eq!(
        find_clip(&normalized, "clip").unwrap().clip.transform,
        transform
    );
    for edited in [
        move_clip(original.clone(), "clip", 2.0, None),
        trim(original.clone(), "clip", TrimEdge::In, 2.0, None),
        stretch(original.clone(), "clip", TrimEdge::Out, 8.0),
        razor(original.clone(), 3.0, None, || "right".into()),
    ] {
        for clip in edited.tracks.iter().flat_map(|t| &t.clips) {
            assert_eq!(clip.transform, transform);
        }
    }
    let back: Project = serde_json::from_value(serde_json::to_value(&original).unwrap()).unwrap();
    assert_eq!(back, original);
}

#[test]
fn command_reports_typed_lock_overlap_and_missing_reference_errors() {
    let mut base = put(put(project(), "first", 0.0, 4.0), "block", 5.0, 4.0);
    assert!(matches!(
        execute_edit(
            base.clone(),
            EditCommand::TrimClips {
                primary_id: "first".into(),
                edge: TrimEdge::Out,
                time: 6.0,
                linked: false,
            }
        ),
        Err(DomainError::Overlap(_))
    ));
    base.tracks[0].locked = true;
    assert!(matches!(
        execute_edit(
            base.clone(),
            EditCommand::TrimClips {
                primary_id: "first".into(),
                edge: TrimEdge::Out,
                time: 3.0,
                linked: false,
            }
        ),
        Err(DomainError::TrackLocked(_))
    ));
    assert!(matches!(
        execute_edit(
            base,
            EditCommand::SetTransform {
                clip_id: "missing".into(),
                transform: ClipTransform::default(),
            }
        ),
        Err(DomainError::ClipNotFound(_))
    ));
}

#[test]
fn transform_command_rejects_invalid_values_and_locked_tracks() {
    let base = put(project(), "first", 0.0, 4.0);
    for transform in [
        ClipTransform {
            x: f64::NAN,
            ..Default::default()
        },
        ClipTransform {
            y: f64::INFINITY,
            ..Default::default()
        },
        ClipTransform {
            scale: 0.0,
            ..Default::default()
        },
    ] {
        assert_eq!(
            execute_edit(
                base.clone(),
                EditCommand::SetTransform {
                    clip_id: "first".into(),
                    transform,
                }
            ),
            Err(DomainError::InvalidTransform)
        );
    }
    let transform = ClipTransform {
        x: 0.1,
        y: 0.2,
        scale: 0.75,
        scale_x: 1.25,
        scale_y: 2.0,
        opacity: 0.5,
    };
    let edited = execute_edit(
        base,
        EditCommand::SetTransform {
            clip_id: "first".into(),
            transform,
        },
    )
    .unwrap();
    assert_eq!(
        find_clip(&edited, "first").unwrap().clip.transform,
        transform
    );
    let locked = set_track_locked(edited, "v1", true);
    assert!(matches!(
        execute_edit(
            locked,
            EditCommand::SetTransform {
                clip_id: "first".into(),
                transform: ClipTransform::default(),
            }
        ),
        Err(DomainError::TrackLocked(_))
    ));
}

#[test]
fn linked_trim_and_stretch_change_both_members_or_reject_entire_group() {
    let base = place_media(project(), "movie", Some("v1"), 0.0).unwrap();
    let id = base.tracks[0].clips[0].id.clone();
    let next = trim_clips(base.clone(), &id, TrimEdge::Out, 10.0, true).unwrap();
    assert_eq!(next.tracks[0].clips[0].duration, 10.0);
    assert_eq!(next.tracks[1].clips[0].duration, 10.0);
    let stretched = stretch_clips(next, &id, TrimEdge::Out, 15.0, true).unwrap();
    assert_eq!(stretched.tracks[0].clips[0].duration, 15.0);
    assert_eq!(stretched.tracks[1].clips[0].duration, 15.0);
    let locked = set_track_locked(base, "a1", true);
    assert!(matches!(
        trim_clips(locked, &id, TrimEdge::Out, 10.0, true),
        Err(DomainError::TrackLocked(_))
    ));
}

#[test]
fn linked_edge_edits_reject_source_clamps_that_separate_the_group() {
    let mut base = place_media(project(), "movie", Some("v1"), 0.0).unwrap();
    let id = base.tracks[0].clips[0].id.clone();
    for track in &mut base.tracks {
        for clip in &mut track.clips {
            clip.duration = 4.0;
        }
    }
    base.tracks[1].clips[0].media_mut().unwrap().offset = 16.0;
    assert_eq!(
        trim_clips(base, &id, TrimEdge::Out, 8.0, true),
        Err(DomainError::LinkedEditMismatch)
    );
}

#[test]
fn paste_preserves_relative_timing_transform_links_and_fixed_workspace() {
    let mut base = place_media(project(), "movie", None, 2.0).unwrap();
    for track in &mut base.tracks {
        for clip in &mut track.clips {
            clip.duration = 4.0;
            clip.transform = ClipTransform {
                x: 0.3,
                y: -0.1,
                scale: 2.0,
                scale_x: 0.6,
                scale_y: 1.4,
                opacity: 0.35,
            };
        }
    }
    let copied: Vec<_> = base
        .tracks
        .iter()
        .flat_map(|track| {
            track.clips.iter().map(|clip| CopiedClip {
                track_id: track.id.clone(),
                kind: track.kind,
                clip: clip.clone(),
            })
        })
        .collect();
    let pasted = paste_clips(base.clone(), &copied, 59.0).unwrap();
    assert_eq!(workspace_duration(&pasted), 60.0);
    let picture = pasted.tracks[0]
        .clips
        .iter()
        .find(|clip| clip.start == 59.0)
        .unwrap();
    let sound = pasted.tracks[1]
        .clips
        .iter()
        .find(|clip| clip.start == 59.0)
        .unwrap();
    assert_ne!(picture.id, copied[0].clip.id);
    assert_ne!(picture.link_id, copied[0].clip.link_id);
    assert_eq!(picture.link_id, sound.link_id);
    assert_eq!(picture.transform, copied[0].clip.transform);
    assert_eq!(sound.transform, copied[1].clip.transform);
    assert!(matches!(
        paste_clips(base, &copied, 3.0),
        Err(DomainError::Overlap(_))
    ));
}

#[test]
fn validation_rejects_duplicate_ids_nonfinite_values_and_source_overrun() {
    let base = put(project(), "first", 0.0, 4.0);
    let mut duplicate = base.clone();
    let clip = duplicate.tracks[0].clips[0].clone();
    duplicate.tracks[1].clips.push(clip);
    assert!(matches!(
        validate_project(&duplicate),
        Err(DomainError::DuplicateId(_))
    ));
    let mut invalid = base.clone();
    invalid.tracks[0].clips[0].start = f64::INFINITY;
    assert_eq!(validate_project(&invalid), Err(DomainError::InvalidTime));
    let mut overrun = base;
    overrun.tracks[0].clips[0].media_mut().unwrap().offset = 19.0;
    assert!(matches!(
        validate_project(&overrun),
        Err(DomainError::SourceBounds(_))
    ));
}

#[test]
fn legacy_edge_edits_preserve_workspace_when_still_images_extend_past_the_end() {
    let mut base = project();
    base.media[0].kind = Some(MediaKind::Image);
    let base = put(base, "still", 0.0, 4.0);
    let next = trim(base.clone(), "still", TrimEdge::Out, 80.0, None);
    assert_eq!(workspace_duration(&next), 60.0);
    let next = stretch(base, "still", TrimEdge::Out, 80.0);
    // Stretch retains source span and clamps the rate to 0.1, hence a 40 second hold.
    assert_eq!(find_clip(&next, "still").unwrap().clip.duration, 40.0);
}

#[test]
fn paste_into_two_missing_tracks_keeps_the_original_lanes_distinct() {
    let mut base = project();
    base.tracks.retain(|t| t.kind == TrackKind::Audio);
    let clips = [
        CopiedClip {
            track_id: "v2".into(),
            kind: TrackKind::Video,
            clip: normalize_clip("one".into(), "movie".into(), 0.0, 4.0, 0.0, 1.0),
        },
        CopiedClip {
            track_id: "v1".into(),
            kind: TrackKind::Video,
            clip: normalize_clip("two".into(), "movie".into(), 0.0, 4.0, 0.0, 1.0),
        },
    ];
    let pasted = paste_clips(base, &clips, 2.0).unwrap();
    assert_eq!(
        pasted
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video && t.clips.len() == 1)
            .count(),
        2
    );
}

#[test]
fn placement_query_uses_spill_policy_without_mutating_the_project() {
    let base = put(project(), "occupied", 0.0, 4.0);
    assert!(can_place_media(&base, "movie", Some("v1"), 0.0).is_ok());
    assert_eq!(base.tracks.len(), 3);
    assert_eq!(base.tracks[0].clips.len(), 1);
    assert!(matches!(
        can_place_media(&base, "movie", Some("a1"), 0.0),
        Err(DomainError::IncompatibleTrack(_))
    ));
    let locked = set_track_locked(base, "v1", true);
    assert!(matches!(
        can_place_media(&locked, "movie", Some("v1"), 0.0),
        Err(DomainError::TrackLocked(_))
    ));
    assert!(matches!(
        can_place_media(&locked, "missing", None, 0.0),
        Err(DomainError::MediaNotFound(_))
    ));
}

#[test]
fn command_move_accepts_snapped_edges_despite_sub_nanosecond_rounding() {
    let base = put(put(project(), "moving", 0.1, 0.2), "next", 0.6, 1.0);
    let next = execute_edit(
        base,
        EditCommand::MoveClips {
            clip_ids: vec!["moving".into()],
            delta: 0.3,
            track_id: None,
        },
    )
    .unwrap();
    assert!((find_clip(&next, "moving").unwrap().clip.start - 0.4).abs() < 1e-9);
}

#[test]
fn split_reports_missing_clip_instead_of_an_empty_selection_error() {
    assert_eq!(
        execute_edit(
            project(),
            EditCommand::Split {
                clip_ids: vec!["missing".into()],
                time: 2.0,
            }
        ),
        Err(DomainError::ClipNotFound("missing".into()))
    );
}

#[test]
fn semantic_commands_reject_source_arithmetic_overflow_even_for_stills() {
    let mut base = put(project(), "still", 0.0, 4.0);
    base.media[0].kind = Some(MediaKind::Image);
    base.tracks[0].clips[0].duration = 1e308;
    base.tracks[0].clips[0].media_mut().unwrap().rate = 8.0;
    assert!(matches!(
        execute_edit(
            base,
            EditCommand::AddTrack {
                kind: TrackKind::Video
            }
        ),
        Err(DomainError::SourceBounds(_))
    ));
}
