use todo_core::*;

fn media(id: &str, kind: MediaKind, duration: f64) -> MediaItem {
    MediaItem {
        id: id.into(),
        name: format!("{id}.bin"),
        path: Some(format!("data/p/{id}")),
        duration,
        has_audio: Some(kind == MediaKind::Audio),
        width: (kind != MediaKind::Audio).then_some(1024),
        height: (kind != MediaKind::Audio).then_some(1024),
        kind: Some(kind),
        alpha: None,
        reveal: None,
        folder_id: None,
    }
}

fn project() -> Project {
    let mut p = create_project(None, None, None, None, None);
    p.media.push(media("plate", MediaKind::Image, 5.0));
    p.media.push(media("e1", MediaKind::Image, 5.0));
    p.media.push(media("e2", MediaKind::Image, 5.0));
    p.media.push(media("voice", MediaKind::Audio, 8.0));
    p
}

fn origin(key: &str) -> ClipOrigin {
    ClipOrigin {
        production: "p1".into(),
        key: key.into(),
        rev: "r1".into(),
    }
}

fn place(
    kind: TrackKind,
    slot: &str,
    above: Option<&str>,
    media_id: &str,
    start: f64,
    key: &str,
) -> GeneratedEdit {
    GeneratedEdit::Place {
        kind,
        lane: LaneHint {
            slot: slot.into(),
            preferred: None,
            above: above.map(|s| Anchor::Slot(s.into())),
            below: None,
        },
        clip: GeneratedClip {
            start,
            duration: 4.0,
            source: GeneratedSource::Media {
                media_id: media_id.into(),
                offset: 0.0,
            },
            transform: Transform::default(),
            intro: Some(Intro::new("fade", 0.4)),
            outro: None,
            transition: None,
            audio: None,
            origin: origin(key),
        },
    }
}

fn track_of(p: &Project, key: &str) -> (usize, String) {
    p.tracks
        .iter()
        .enumerate()
        .find_map(|(i, t)| {
            t.clips
                .iter()
                .any(|c| c.origin.as_ref().is_some_and(|o| o.key == key))
                .then(|| (i, t.id.clone()))
        })
        .unwrap()
}

fn clip_of<'a>(p: &'a Project, key: &str) -> &'a Clip {
    p.tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find(|c| c.origin.as_ref().is_some_and(|o| o.key == key))
        .unwrap()
}

#[test]
fn one_batch_places_stacked_lanes_and_reuses_a_slot_track() {
    let edits = vec![
        place(TrackKind::Video, "visual:0", None, "plate", 0.0, "plate"),
        place(
            TrackKind::Video,
            "visual:1",
            Some("visual:0"),
            "e1",
            0.0,
            "b0/e0",
        ),
        place(
            TrackKind::Video,
            "visual:1",
            Some("visual:0"),
            "e2",
            4.0,
            "b1/e0",
        ),
        place(
            TrackKind::Video,
            "visual:2",
            Some("visual:1"),
            "e2",
            0.0,
            "b0/e1",
        ),
        place(TrackKind::Audio, "voice:0", None, "voice", 0.0, "voice"),
    ];
    let p = execute_edit(project(), EditCommand::ApplyGenerated(edits)).unwrap();
    let (plate, _) = track_of(&p, "plate");
    let (lane1, lane1_id) = track_of(&p, "b0/e0");
    let (lane2, _) = track_of(&p, "b0/e1");
    // Earlier tracks draw on top: lane 2 above lane 1 above the plate.
    assert!(lane2 < lane1 && lane1 < plate, "{lane2} {lane1} {plate}");
    assert_eq!(track_of(&p, "b1/e0").1, lane1_id);
    let element = clip_of(&p, "b0/e0");
    assert_eq!(element.intro.as_ref().unwrap().kind, "fade");
    assert_eq!(element.origin.as_ref().unwrap().production, "p1");
    assert_eq!(p.tracks[track_of(&p, "voice").0].kind, TrackKind::Audio);
}

#[test]
fn updates_keep_the_users_transform_and_reject_nothing_else() {
    let p = execute_edit(
        project(),
        EditCommand::ApplyGenerated(vec![place(
            TrackKind::Video,
            "visual:0",
            None,
            "e1",
            1.0,
            "k",
        )]),
    )
    .unwrap();
    let id = clip_of(&p, "k").id.clone();
    let moved = Transform {
        x: 0.2,
        ..Transform::default()
    };
    let p = execute_edit(
        p,
        EditCommand::SetTransform {
            clip_id: id.clone(),
            transform: moved,
        },
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let p = execute_edit(
        p,
        EditCommand::ApplyGenerated(vec![GeneratedEdit::Update {
            clip_id: id.clone(),
            start: 2.0,
            duration: 3.0,
            source: GeneratedSource::Media {
                media_id: "e2".into(),
                offset: 0.0,
            },
            transform: None,
            intro: None,
            outro: None,
            transition: None,
            audio: None,
            origin: ClipOrigin {
                rev: "r2".into(),
                ..origin("k")
            },
        }]),
    )
    .unwrap();
    let clip = clip_of(&p, "k");
    assert_eq!(clip.id, id);
    assert_eq!(clip.transform.x, 0.2);
    assert_eq!((clip.start, clip.duration), (2.0, 3.0));
    assert_eq!(clip.media_id(), Some("e2"));
    assert_eq!(clip.origin.as_ref().unwrap().rev, "r2");
}

#[test]
fn a_failing_step_rejects_the_whole_batch() {
    let edits = vec![
        place(TrackKind::Video, "visual:0", None, "e1", 0.0, "ok"),
        place(TrackKind::Video, "visual:1", None, "missing", 0.0, "bad"),
    ];
    assert!(execute_edit(project(), EditCommand::ApplyGenerated(edits)).is_err());
}

#[test]
fn removal_and_move_apply_before_new_clips() {
    let p = execute_edit(
        project(),
        EditCommand::ApplyGenerated(vec![
            place(TrackKind::Video, "visual:0", None, "e1", 0.0, "a"),
            place(TrackKind::Video, "visual:0", None, "e2", 4.0, "b"),
        ]),
    )
    .unwrap();
    let a = clip_of(&p, "a").id.clone();
    let b = clip_of(&p, "b").id.clone();
    // `c` takes a's old slot; a is removed and b moves later in the same batch.
    let p = execute_edit(
        p,
        EditCommand::ApplyGenerated(vec![
            place(TrackKind::Video, "visual:0", None, "e2", 0.0, "c"),
            GeneratedEdit::Remove { clip_id: a },
            GeneratedEdit::Move {
                clip_id: b.clone(),
                start: 5.0,
            },
        ]),
    )
    .unwrap();
    assert_eq!(clip_of(&p, "b").start, 5.0);
    assert_eq!(clip_of(&p, "c").start, 0.0);
    assert!(p
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .all(|c| c.origin.as_ref().is_none_or(|o| o.key != "a")));
}

#[test]
fn text_clips_bring_their_built_in_style() {
    let edit = GeneratedEdit::Place {
        kind: TrackKind::Text,
        lane: LaneHint {
            slot: "subtitle:0".into(),
            ..LaneHint::default()
        },
        clip: GeneratedClip {
            start: 0.5,
            duration: 1.5,
            source: GeneratedSource::Text(TextContent::new("你好", "clean-white-outline")),
            transform: Transform::default(),
            intro: None,
            outro: Some(MotionRef::new("fadeOut", 0.3)),
            transition: None,
            audio: None,
            origin: origin("sub/0"),
        },
    };
    let p = execute_edit(project(), EditCommand::ApplyGenerated(vec![edit])).unwrap();
    assert!(p.text_style("clean-white-outline").is_some());
    let sub = clip_of(&p, "sub/0");
    assert!(sub.is_text());
    // The outro fades the clip out over its last 0.3 s.
    let span = VisibleSpan::default();
    assert!(picture_transform(&p, sub, 1.35, span).opacity < 0.5);
    assert_eq!(picture_transform(&p, sub, 1.0, span).opacity, 1.0);
}

#[test]
fn a_taken_slot_lane_gets_a_new_lane_directly_above_it_not_at_the_bottom() {
    // plate on its slot, an element slot above it.
    let p = execute_edit(
        project(),
        EditCommand::ApplyGenerated(vec![
            place(TrackKind::Video, "visual:0", None, "plate", 0.0, "plate"),
            place(
                TrackKind::Video,
                "visual:1",
                Some("visual:0"),
                "e1",
                0.0,
                "a",
            ),
        ]),
    )
    .unwrap();
    let (element_index, element_track) = track_of(&p, "a");
    let (plate_index, _) = track_of(&p, "plate");
    assert!(element_index < plate_index);
    // The next element of the slot overlaps the first one by a hair.
    let mut next = place(TrackKind::Video, "visual:1", None, "e2", 3.9999, "b");
    if let GeneratedEdit::Place { lane, .. } = &mut next {
        lane.preferred = Some(element_track.clone());
    }
    let p = execute_edit(p, EditCommand::ApplyGenerated(vec![next])).unwrap();
    let (b_index, b_track) = track_of(&p, "b");
    let (plate_index, _) = track_of(&p, "plate");
    assert_ne!(b_track, element_track);
    assert!(
        b_index < plate_index,
        "the new lane must stay above the plate"
    );
    assert_eq!(
        b_index + 1,
        track_of(&p, "a").0,
        "directly above the slot's lane"
    );
}
