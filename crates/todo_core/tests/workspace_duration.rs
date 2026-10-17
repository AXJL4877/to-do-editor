use todo_core::*;

fn project() -> Project {
    let mut project = create_project(None, None, Some(300.0), None, None);
    project.media.push(MediaItem {
        id: "long".into(),
        name: "long.mp4".into(),
        path: None,
        duration: 900.0,
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

fn clip_at(mut project: Project, start: f64, duration: f64) -> Project {
    project = add_clip(
        project,
        "v1",
        normalize_clip("clip".into(), "long".into(), start, duration, 0.0, 1.0),
    );
    // Model existing projects where the user explicitly restored the setting.
    project.duration = Some(300.0);
    project
}

#[test]
fn placing_long_linked_media_preserves_300_seconds_and_the_complete_source() {
    let next = place_media(project(), "long", None, 225.0).unwrap();
    assert_eq!(workspace_duration(&next), 300.0);
    assert_eq!(project_duration(&next), 300.0);
    assert_eq!(next.media[0].duration, 900.0);
    let clips: Vec<_> = next.tracks.iter().flat_map(|track| &track.clips).collect();
    assert_eq!(clips.len(), 2);
    for clip in clips {
        assert_eq!(
            (clip.start, clip.duration, clip_end(clip)),
            (225.0, 900.0, 1125.0)
        );
    }
}

#[test]
fn direct_clip_addition_does_not_expand_configured_workspace() {
    let next = add_clip(
        project(),
        "v1",
        normalize_clip("clip".into(), "long".into(), 225.0, 90.0, 0.0, 1.0),
    );
    assert_eq!(next.duration, Some(300.0));
    assert_eq!(clip_end(&find_clip(&next, "clip").unwrap().clip), 315.0);
}

#[test]
fn moving_after_3m45s_can_cross_the_boundary_without_changing_settings() {
    let next = execute_edit(
        clip_at(project(), 225.0, 90.0),
        EditCommand::MoveClips {
            clip_ids: vec!["clip".into()],
            delta: 15.0,
            track_id: None,
        },
    )
    .unwrap();
    assert_eq!(next.duration, Some(300.0));
    let clip = find_clip(&next, "clip").unwrap().clip;
    assert_eq!(
        (clip.start, clip.duration, clip_end(&clip)),
        (240.0, 90.0, 330.0)
    );
}

#[test]
fn trimming_and_stretching_past_the_workspace_preserve_its_duration() {
    let base = clip_at(project(), 225.0, 60.0);
    for command in [
        EditCommand::TrimClips {
            primary_id: "clip".into(),
            edge: TrimEdge::Out,
            time: 330.0,
            linked: false,
        },
        EditCommand::StretchClips {
            primary_id: "clip".into(),
            edge: TrimEdge::Out,
            time: 330.0,
            linked: false,
        },
    ] {
        let next = execute_edit(base.clone(), command).unwrap();
        assert_eq!(next.duration, Some(300.0));
        assert_eq!(clip_end(&find_clip(&next, "clip").unwrap().clip), 330.0);
    }
}

#[test]
fn pasting_beyond_300_seconds_preserves_duration_and_full_copied_range() {
    let base = clip_at(project(), 10.0, 60.0);
    let copied = CopiedClip {
        track_id: "v1".into(),
        kind: TrackKind::Video,
        clip: find_clip(&base, "clip").unwrap().clip,
    };
    let next = paste_clips(base, &[copied], 310.0).unwrap();
    assert_eq!(next.duration, Some(300.0));
    let pasted = next
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .find(|clip| clip.id != "clip")
        .unwrap();
    assert_eq!(
        (pasted.start, pasted.duration, clip_end(pasted)),
        (310.0, 60.0, 370.0)
    );
}

#[test]
fn explicit_resize_changes_only_workspace_and_playhead_not_overflow_content() {
    let base = clip_at(project(), 225.0, 90.0);
    let original = find_clip(&base, "clip").unwrap().clip;
    let mut next = set_project_duration(base, 240.0).unwrap();
    assert_eq!(next.duration, Some(240.0));
    assert_eq!(find_clip(&next, "clip").unwrap().clip, original);
    next = set_project_duration(next, 400.0).unwrap();
    assert_eq!(next.duration, Some(400.0));
    assert_eq!(find_clip(&next, "clip").unwrap().clip, original);
}
