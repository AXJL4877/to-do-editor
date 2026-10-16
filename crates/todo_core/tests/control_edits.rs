use todo_core::*;

fn library(audio: bool) -> Project {
    let mut project = create_project(Some(30.0), None, None, None, None);
    project.media.push(MediaItem {
        id: "m".into(),
        name: "A.mp4".into(),
        path: Some("A.mp4".into()),
        duration: 30.0,
        has_audio: Some(audio),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Video),
        folder_id: None,
});
    project
}

#[test]
fn source_range_placement_uses_requested_span_and_links_audio() {
    let next = execute_edit(
        library(true),
        EditCommand::PlaceMediaRange {
            media_id: "m".into(),
            track_id: Some("v1".into()),
            time: 10.0,
            source_in: 3.0,
            source_out: 8.0,
        },
    )
    .unwrap();
    let clips: Vec<_> = next.tracks.iter().flat_map(|t| &t.clips).collect();
    assert_eq!(clips.len(), 2);
    for clip in &clips {
        assert_eq!(
            (
                clip.start,
                clip.media().unwrap().offset,
                clip.duration,
                clip.media().unwrap().rate
            ),
            (10.0, 3.0, 5.0, 1.0)
        );
    }
    assert!(clips[0].link_id.is_some());
    assert_eq!(clips[0].link_id, clips[1].link_id);
    validate_project(&next).unwrap();
}

#[test]
fn range_placement_checks_overlap_using_the_requested_duration() {
    let project = place_media_range(library(false), "m", Some("v1"), 5.0, 0.0, 2.0).unwrap();
    let project = place_media_range(project, "m", Some("v1"), 0.0, 3.0, 8.0).unwrap();
    let video = project.tracks.iter().find(|t| t.id == "v1").unwrap();
    assert_eq!(video.clips.len(), 2);
    assert_eq!(
        project
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .count(),
        1
    );
}

#[test]
fn range_placement_rejects_invalid_or_out_of_bounds_source_times() {
    for (source_in, source_out) in [
        (-1.0, 5.0),
        (5.0, 5.0),
        (8.0, 3.0),
        (29.0, 31.0),
        (0.0, f64::INFINITY),
        (f64::NAN, 3.0),
        (0.0, 0.01),
    ] {
        assert!(
            place_media_range(library(false), "m", Some("v1"), 0.0, source_in, source_out).is_err()
        );
    }
}

#[test]
fn semantic_split_of_picture_also_splits_its_linked_audio() {
    let project = place_media(library(true), "m", Some("v1"), 0.0).unwrap();
    let id = project.tracks[0].clips[0].id.clone();
    let project = execute_edit(
        project,
        EditCommand::SplitLinked {
            clip_ids: vec![id],
            time: 12.0,
        },
    )
    .unwrap();
    let video = &project.tracks.iter().find(|t| t.id == "v1").unwrap().clips;
    let audio = &project.tracks.iter().find(|t| t.id == "a1").unwrap().clips;
    assert_eq!((video.len(), audio.len()), (2, 2));
    assert_eq!(video[0].link_id, audio[0].link_id);
    assert_eq!(video[1].link_id, audio[1].link_id);
    assert_ne!(video[0].link_id, video[1].link_id);
    assert_eq!(
        (audio[1].start, audio[1].media().unwrap().offset),
        (12.0, 12.0)
    );
}

#[test]
fn linked_split_rejects_a_locked_audio_track() {
    let mut project = place_media(library(true), "m", Some("v1"), 0.0).unwrap();
    let id = project.tracks[0].clips[0].id.clone();
    project
        .tracks
        .iter_mut()
        .find(|t| t.id == "a1")
        .unwrap()
        .locked = true;
    assert_eq!(
        execute_edit(
            project,
            EditCommand::SplitLinked {
                clip_ids: vec![id],
                time: 12.0
            }
        ),
        Err(DomainError::TrackLocked("a1".into()))
    );
}
