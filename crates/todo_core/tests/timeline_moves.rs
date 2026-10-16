use todo_core::*;

fn put(
    mut project: Project,
    track: &str,
    id: &str,
    start: f64,
    duration: f64,
    link: Option<&str>,
) -> Project {
    let mut clip = normalize_clip(id.into(), "movie".into(), start, duration, 1.0, 1.0);
    clip.link_id = link.map(str::to_string);
    project = add_clip(project, track, clip);
    project
}

fn pairs() -> Project {
    let mut project = create_project(None, None, Some(20.0), None, None);
    project.media.push(MediaItem {
        id: "movie".into(),
        name: "movie.mp4".into(),
        path: None,
        duration: 30.0,
        has_audio: Some(true),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Video),
    });
    for (track, id, start, link) in [
        ("v1", "picture_a", 0.0, "a"),
        ("a1", "sound_a", 0.0, "a"),
        ("v1", "picture_b", 6.0, "b"),
        ("a1", "sound_b", 6.0, "b"),
    ] {
        project = put(project, track, id, start, 4.0, Some(link));
    }
    project
}

fn shift(
    project: Project,
    ids: &[&str],
    delta: f64,
    target: Option<&str>,
) -> Result<Project, DomainError> {
    execute_edit(
        project,
        EditCommand::MoveClips {
            clip_ids: ids.iter().map(|id| (*id).into()).collect(),
            delta,
            track_id: target.map(str::to_string),
        },
    )
}

#[test]
fn crossing_linked_videos_allocates_both_video_and_audio_lanes() {
    for target in [None, Some("v1")] {
        let base = pairs();
        let next = shift(base.clone(), &["picture_b", "sound_b"], -4.0, target).unwrap();
        for (id, track) in [("picture_b", "v2"), ("sound_b", "a2")] {
            let found = find_clip(&next, id).unwrap();
            assert_eq!(found.track_id, track);
            assert_eq!(
                (
                    found.clip.start,
                    found.clip.media().unwrap().offset,
                    found.clip.media().unwrap().rate
                ),
                (2.0, 1.0, 1.0)
            );
            assert_eq!(found.clip.link_id.as_deref(), Some("b"));
        }
        for id in ["picture_a", "sound_a"] {
            assert_eq!(
                find_clip(&next, id).unwrap().clip,
                find_clip(&base, id).unwrap().clip
            );
        }
        validate_project(&next).unwrap();
    }
}

#[test]
fn overlap_reuses_existing_unlocked_lanes_before_creating_any() {
    let mut base = pairs();
    for kind in [TrackKind::Video, TrackKind::Audio] {
        base = add_track(base, kind);
        base = add_track(base, kind);
    }
    for track in &mut base.tracks {
        if ["v2", "a2"].contains(&track.id.as_str()) {
            track.locked = true;
        }
    }
    let count = base.tracks.len();
    let next = shift(base, &["picture_b", "sound_b"], -4.0, None).unwrap();
    assert_eq!(next.tracks.len(), count);
    assert_eq!(find_clip(&next, "picture_b").unwrap().track_id, "v3");
    assert_eq!(find_clip(&next, "sound_b").unwrap().track_id, "a3");
}

#[test]
fn audio_collision_can_spill_without_moving_the_free_video_lane() {
    let base = pairs();
    let base = execute_edit(
        base,
        EditCommand::DeleteClips {
            clip_ids: vec!["picture_a".into()],
            ripple: false,
        },
    )
    .unwrap();
    let next = shift(base, &["picture_b", "sound_b"], -4.0, None).unwrap();
    assert_eq!(find_clip(&next, "picture_b").unwrap().track_id, "v1");
    assert_eq!(find_clip(&next, "sound_b").unwrap().track_id, "a2");
}

#[test]
fn moving_multiple_source_lanes_preserves_overlapping_members_and_primary_destination() {
    let mut base = pairs();
    base = add_track(base, TrackKind::Video);
    base = put(base, "v2", "top", 6.0, 4.0, None);
    let next = shift(base, &["top", "picture_b"], 5.0, Some("v1")).unwrap();
    assert_eq!(find_clip(&next, "top").unwrap().track_id, "v1");
    assert_eq!(find_clip(&next, "picture_b").unwrap().track_id, "v2");
    assert_eq!(find_clip(&next, "top").unwrap().clip.start, 11.0);
    assert_eq!(find_clip(&next, "picture_b").unwrap().clip.start, 11.0);
}

#[test]
fn exact_adjacency_keeps_original_lanes_and_never_creates_extra_tracks() {
    let base = pairs();
    let count = base.tracks.len();
    let next = shift(base, &["picture_b", "sound_b"], -2.0, None).unwrap();
    assert_eq!(next.tracks.len(), count);
    assert_eq!(find_clip(&next, "picture_b").unwrap().track_id, "v1");
    assert_eq!(find_clip(&next, "sound_b").unwrap().track_id, "a1");
    assert_eq!(find_clip(&next, "picture_b").unwrap().clip.start, 4.0);
}

#[test]
fn locked_members_or_explicit_locked_targets_still_reject_the_whole_move() {
    let mut base = pairs();
    base = add_track(base, TrackKind::Video);
    base.tracks
        .iter_mut()
        .find(|t| t.id == "v2")
        .unwrap()
        .locked = true;
    assert_eq!(
        shift(base.clone(), &["picture_b", "sound_b"], -4.0, Some("v2")),
        Err(DomainError::TrackLocked("v2".into()))
    );
    base.tracks
        .iter_mut()
        .find(|t| t.id == "a1")
        .unwrap()
        .locked = true;
    assert_eq!(
        shift(base, &["picture_b", "sound_b"], -4.0, None),
        Err(DomainError::TrackLocked("a1".into()))
    );
}
