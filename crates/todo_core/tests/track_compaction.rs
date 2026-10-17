use todo_core::*;

fn image(id: &str) -> MediaItem {
    MediaItem {
        id: id.into(),
        name: format!("{id}.png"),
        path: Some(format!("{id}.png")),
        duration: 20.0,
        has_audio: Some(false),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Image),
        folder_id: None,
        alpha: None,
        reveal: None,
    }
}

fn clip(id: &str, media_id: &str, start: f64, duration: f64) -> Clip {
    normalize_clip(id.into(), media_id.into(), start, duration, 0.0, 1.0)
}

fn cluttered_project() -> Project {
    let mut project = create_project(None, None, None, None, None);
    for id in ["m0", "m1", "m2", "m3", "m4"] {
        project.media.push(image(id));
    }
    project = add_track(project, TrackKind::Video);
    project = add_track(project, TrackKind::Video);
    project = add_track(project, TrackKind::Video);
    project = add_clip(project, "v1", clip("c0", "m0", 0.0, 2.0));
    project = add_clip(project, "v1", clip("c1", "m1", 4.0, 2.0));
    project = add_clip(project, "v2", clip("c2", "m2", 1.0, 2.0));
    project = add_clip(project, "v2", clip("c3", "m3", 6.0, 2.0));
    add_clip(project, "v3", clip("c4", "m4", 8.0, 2.0))
}

#[test]
fn compaction_reuses_non_overlapping_tracks_and_keeps_overlap_order() {
    let project = execute_edit(cluttered_project(), EditCommand::CompactTracks).unwrap();
    validate_project(&project).unwrap();

    let videos: Vec<_> = project
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Video)
        .collect();
    assert_eq!(
        videos.len(),
        2,
        "one of the generated lanes became reusable"
    );
    assert_eq!(videos[0].id, "v1");
    assert_eq!(videos[1].id, "v2");
    assert_eq!(
        videos[0]
            .clips
            .iter()
            .map(|clip| clip.id.as_str())
            .collect::<Vec<_>>(),
        ["c0", "c1", "c3", "c4"]
    );
    assert_eq!(videos[1].clips[0].id, "c2");
    assert!(
        videos[0].clips.iter().any(|clip| clip.id == "c0")
            && videos[1].clips.iter().any(|clip| clip.id == "c2"),
        "overlapping clips remain on separate layers"
    );
}

#[test]
fn compaction_does_not_cross_locked_tracks() {
    let mut project = cluttered_project();
    project.tracks[1].locked = true;
    let before_locked = project.tracks[1].clone();
    let next = execute_edit(project, EditCommand::CompactTracks).unwrap();
    assert_eq!(next.tracks.iter().filter(|t| t.locked).count(), 1);
    let locked = next.tracks.iter().find(|track| track.locked).unwrap();
    assert_eq!(locked, &before_locked);
}
