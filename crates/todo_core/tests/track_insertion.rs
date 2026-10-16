use todo_core::*;

fn track(id: &str, name: &str, kind: TrackKind) -> Track {
    Track {
        id: id.into(),
        name: name.into(),
        kind,
        muted: false,
        locked: false,
        clips: vec![],
    }
}

fn interleaved_project() -> Project {
    let mut project = create_project(None, None, None, None, None);
    project.tracks = vec![
        track("v1", "视频1", TrackKind::Video),
        track("a1", "对白", TrackKind::Audio),
        track("v2", "视频2", TrackKind::Video),
        track("sfx1", "音效", TrackKind::Sfx),
        track("a2", "音频2", TrackKind::Audio),
        track("v4", "视频4", TrackKind::Video),
    ];
    project
}

#[test]
fn inserts_immediately_below_the_anchor_in_interleaved_track_order() {
    for (anchor, expected_ids, kind, name) in [
        (
            "v1",
            vec!["v1", "v3", "a1", "v2", "sfx1", "a2", "v4"],
            TrackKind::Video,
            "视频3",
        ),
        (
            "a1",
            vec!["v1", "a1", "a3", "v2", "sfx1", "a2", "v4"],
            TrackKind::Audio,
            "音频3",
        ),
        (
            "v2",
            vec!["v1", "a1", "v2", "v3", "sfx1", "a2", "v4"],
            TrackKind::Video,
            "视频3",
        ),
        (
            "sfx1",
            vec!["v1", "a1", "v2", "sfx1", "sfx2", "a2", "v4"],
            TrackKind::Sfx,
            "音效2",
        ),
        (
            "v4",
            vec!["v1", "a1", "v2", "sfx1", "a2", "v4", "v3"],
            TrackKind::Video,
            "视频3",
        ),
    ] {
        let next = insert_track_below(interleaved_project(), anchor).unwrap();
        assert_eq!(
            next.tracks
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
            expected_ids,
            "anchor: {anchor}"
        );
        let anchor_index = next.tracks.iter().position(|t| t.id == anchor).unwrap();
        let inserted = &next.tracks[anchor_index + 1];
        assert_eq!(inserted.kind, kind);
        assert_eq!(inserted.name, name);
        assert!(inserted.clips.is_empty());
        assert!(!inserted.locked);
        assert!(!inserted.muted);
    }
}

#[test]
fn repeated_insertions_remain_immediately_below_the_same_anchor() {
    let mut next = create_project(None, None, None, None, None);
    for _ in 0..2 {
        next = execute_edit(
            next,
            EditCommand::InsertTrackBelow {
                track_id: "v1".into(),
            },
        )
        .unwrap();
    }
    assert_eq!(
        next.tracks
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        ["v1", "v3", "v2", "a1", "sfx1"]
    );
    assert_eq!(next.tracks[1].name, "视频3");
    assert_eq!(next.tracks[2].name, "视频2");
}

#[test]
fn a_missing_anchor_reports_the_track_reference_error() {
    let base = interleaved_project();
    assert_eq!(
        insert_track_below(base.clone(), "missing"),
        Err(DomainError::TrackNotFound("missing".into()))
    );
    assert_eq!(
        execute_edit(
            base,
            EditCommand::InsertTrackBelow {
                track_id: "missing".into(),
            },
        ),
        Err(DomainError::TrackNotFound("missing".into()))
    );
}

#[test]
fn insertion_below_a_locked_track_preserves_existing_content_and_flags() {
    let mut base = interleaved_project();
    base.media.push(MediaItem {
        id: "movie".into(),
        name: "movie.mp4".into(),
        path: Some("movie.mp4".into()),
        duration: 20.0,
        has_audio: Some(true),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Video),
    });
    base = place_media(base, "movie", Some("v1"), 3.0).unwrap();
    base.tracks[0].locked = true;
    base.tracks[0].muted = true;
    base.tracks[1].locked = true;
    base.tracks[2].muted = true;
    base.playhead = 5.0;
    let mut next = execute_edit(
        base.clone(),
        EditCommand::InsertTrackBelow {
            track_id: "v1".into(),
        },
    )
    .unwrap();
    assert_eq!(next.tracks[1].id, "v3");
    assert!(next.tracks[1].clips.is_empty());
    next.tracks.remove(1);
    assert_eq!(next, base);
}

#[test]
fn insertion_avoids_existing_generated_names_as_well_as_track_ids() {
    let mut base = create_project(None, None, None, None, None);
    base.tracks.push(track("custom", "视频2", TrackKind::Video));
    let next = insert_track_below(base, "v1").unwrap();
    assert_eq!(next.tracks[1].id, "v3");
    assert_eq!(next.tracks[1].name, "视频3");
}

#[test]
fn add_track_lands_in_kind_section_and_keeps_video_above_audio() {
    let next = execute_edit(
        interleaved_project(),
        EditCommand::AddTrack {
            kind: TrackKind::Audio,
        },
    )
    .unwrap();
    let kinds: Vec<_> = next.tracks.iter().map(|t| t.kind).collect();
    let first_audio = kinds.iter().position(|k| *k == TrackKind::Audio).unwrap();
    let last_video = kinds.iter().rposition(|k| *k == TrackKind::Video).unwrap();
    assert!(last_video < first_audio);
    assert_eq!(
        next.tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .last()
            .unwrap()
            .id,
        "a3"
    );
}
