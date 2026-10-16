use todo_core::*;

fn project_with_video() -> Project {
    let mut p = create_project(None, None, None, None, None);
    p.media.push(MediaItem {
        id: "m".into(),
        name: "a.mp4".into(),
        path: Some("a.mp4".into()),
        duration: 10.0,
        has_audio: Some(false),
        width: Some(1280),
        height: Some(720),
        kind: Some(MediaKind::Video),
        folder_id: None,
});
    execute_edit(
        p,
        EditCommand::PlaceMedia {
            media_id: "m".into(),
            track_id: Some("v1".into()),
            time: 0.0,
        },
    )
    .unwrap()
}

fn add(p: Project, time: f64, intro: Option<TextIntro>) -> Project {
    let mut content = TextContent::new("钱越来越 不值钱了", DEFAULT_TEXT_STYLE_ID);
    content.animation.intro = intro;
    execute_edit(
        p,
        EditCommand::AddText {
            track_id: None,
            time,
            duration: 3.0,
            content,
        },
    )
    .unwrap()
}

fn text_clip_ids(p: &Project) -> Vec<String> {
    p.tracks
        .iter()
        .flat_map(|t| &t.clips)
        .filter(|c| c.is_text())
        .map(|c| c.id.clone())
        .collect()
}

#[test]
fn add_text_creates_top_text_lane_and_copies_style_once() {
    let p = add(project_with_video(), 1.0, None);
    assert_eq!(p.tracks[0].kind, TrackKind::Text);
    assert_eq!(p.text_styles.len(), 1);
    assert_eq!(p.text_styles[0].id, DEFAULT_TEXT_STYLE_ID);
    // A second overlapping text spills to a new text lane, still above video.
    let p = add(p, 2.0, None);
    assert_eq!(p.text_styles.len(), 1);
    let kinds: Vec<_> = p.tracks.iter().map(|t| t.kind).collect();
    assert_eq!(
        &kinds[..3],
        &[TrackKind::Text, TrackKind::Text, TrackKind::Video]
    );
}

#[test]
fn unknown_style_and_wrong_lane_are_rejected() {
    let p = project_with_video();
    let err = execute_edit(
        p.clone(),
        EditCommand::AddText {
            track_id: None,
            time: 0.0,
            duration: 1.0,
            content: TextContent::new("x", "nope"),
        },
    )
    .unwrap_err();
    assert_eq!(err, DomainError::TextStyleNotFound("nope".into()));
    let err = execute_edit(
        p,
        EditCommand::AddText {
            track_id: Some("v1".into()),
            time: 0.0,
            duration: 1.0,
            content: TextContent::new("x", DEFAULT_TEXT_STYLE_ID),
        },
    )
    .unwrap_err();
    assert_eq!(err, DomainError::IncompatibleTrack("v1".into()));
}

#[test]
fn text_trims_freely_but_cannot_stretch_or_leave_text_lanes() {
    let p = add(project_with_video(), 1.0, None);
    let id = text_clip_ids(&p).remove(0);
    let longer = execute_edit(
        p.clone(),
        EditCommand::TrimClips {
            primary_id: id.clone(),
            edge: TrimEdge::Out,
            time: 30.0,
            linked: false,
        },
    )
    .unwrap();
    assert!((find_clip(&longer, &id).unwrap().clip.duration - 29.0).abs() < 1e-9);
    let earlier = execute_edit(
        p.clone(),
        EditCommand::TrimClips {
            primary_id: id.clone(),
            edge: TrimEdge::In,
            time: 0.0,
            linked: false,
        },
    )
    .unwrap();
    let clip = find_clip(&earlier, &id).unwrap().clip;
    assert_eq!((clip.start, clip.duration), (0.0, 4.0));
    assert_eq!(
        execute_edit(
            p.clone(),
            EditCommand::StretchClips {
                primary_id: id.clone(),
                edge: TrimEdge::Out,
                time: 6.0,
                linked: false,
            },
        ),
        Err(DomainError::UnsupportedClip(id.clone()))
    );
    assert_eq!(
        execute_edit(
            p,
            EditCommand::MoveClips {
                clip_ids: vec![id],
                delta: 0.0,
                track_id: Some("v1".into()),
            },
        ),
        Err(DomainError::IncompatibleTrack("v1".into()))
    );
}

#[test]
fn split_keeps_intro_only_on_the_left_piece() {
    let intro = TextIntro::PerChar {
        duration: 1.0,
        char_duration: 0.2,
    };
    let p = add(project_with_video(), 0.0, Some(intro));
    let id = text_clip_ids(&p).remove(0);
    let p = execute_edit(
        p,
        EditCommand::Split {
            clip_ids: vec![id.clone()],
            time: 1.5,
        },
    )
    .unwrap();
    let ids = text_clip_ids(&p);
    assert_eq!(ids.len(), 2);
    let left = find_clip(&p, &id).unwrap().clip;
    let right = find_clip(&p, ids.iter().find(|i| **i != id).unwrap())
        .unwrap()
        .clip;
    assert_eq!(left.text().unwrap().animation.intro, Some(intro));
    assert_eq!(right.text().unwrap().animation.intro, None);
    assert_eq!(right.text().unwrap().text, left.text().unwrap().text);
    assert!((right.start - 1.5).abs() < 1e-9);
}

#[test]
fn set_text_replaces_content_and_rejects_media_clips() {
    let p = add(project_with_video(), 0.0, None);
    let id = text_clip_ids(&p).remove(0);
    let mut content = TextContent::new("新的", "clean-white-outline");
    content.layout.font_size = 0.2;
    let p = execute_edit(
        p,
        EditCommand::SetText {
            clip_id: id.clone(),
            content: content.clone(),
        },
    )
    .unwrap();
    assert_eq!(find_clip(&p, &id).unwrap().clip.text(), Some(&content));
    assert_eq!(p.text_styles.len(), 2);
    let video = p.tracks.iter().find(|t| t.id == "v1").unwrap().clips[0]
        .id
        .clone();
    assert_eq!(
        execute_edit(
            p,
            EditCommand::SetText {
                clip_id: video.clone(),
                content,
            },
        ),
        Err(DomainError::UnsupportedClip(video))
    );
}

#[test]
fn layers_put_text_above_video_with_animation_frame() {
    let p = add(
        project_with_video(),
        1.0,
        Some(TextIntro::Fade { duration: 1.0 }),
    );
    let layers = resolve_layers(&p, 1.25);
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].track_kind, TrackKind::Video);
    assert_eq!(layers[1].track_kind, TrackKind::Text);
    assert_eq!(layers[1].source_size, Some((1920, 1080)));
    match &layers[1].source {
        HitSource::Text { style, frame } => {
            assert_eq!(style.id, DEFAULT_TEXT_STYLE_ID);
            assert!((frame.opacity - 0.25).abs() < 1e-9);
        }
        other => panic!("expected text, got {other:?}"),
    }
    assert!(resolve_audio_clips(&p).is_empty());
}

#[test]
fn project_with_text_roundtrips_and_validates() {
    let p = add(project_with_video(), 0.0, None);
    let json = serde_json::to_string(&p).unwrap();
    let back: Project = serde_json::from_str(&json).unwrap();
    assert_eq!(back, p);
    validate_project(&back).unwrap();
    let mut missing_style = back.clone();
    missing_style.text_styles.clear();
    assert_eq!(
        validate_project(&missing_style),
        Err(DomainError::TextStyleNotFound(DEFAULT_TEXT_STYLE_ID.into()))
    );
}
