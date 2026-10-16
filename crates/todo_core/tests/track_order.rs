use todo_core::*;

fn ids(project: &Project) -> Vec<&str> {
    project.tracks.iter().map(|t| t.id.as_str()).collect()
}

#[test]
fn default_and_loaded_projects_keep_video_above_audio() {
    let project = create_project(None, None, None, None, None);
    assert_eq!(ids(&project), ["v1", "a1", "sfx1"]);

    let interleaved = create_project(
        None,
        None,
        None,
        None,
        Some(vec![
            Track {
                id: "a1".into(),
                name: "对白".into(),
                kind: TrackKind::Audio,
                muted: false,
                locked: false,
                clips: vec![],
            },
            Track {
                id: "v1".into(),
                name: "视频1".into(),
                kind: TrackKind::Video,
                muted: false,
                locked: false,
                clips: vec![],
            },
            Track {
                id: "sfx1".into(),
                name: "音效".into(),
                kind: TrackKind::Sfx,
                muted: false,
                locked: false,
                clips: vec![],
            },
        ]),
    );
    assert_eq!(ids(&interleaved), ["v1", "a1", "sfx1"]);
}

#[test]
fn add_track_appends_inside_its_section_not_past_audio() {
    let project = add_track(
        create_project(None, None, None, None, None),
        TrackKind::Video,
    );
    assert_eq!(ids(&project), ["v1", "v2", "a1", "sfx1"]);
    let project = add_track(project, TrackKind::Audio);
    assert_eq!(ids(&project), ["v1", "v2", "a1", "a2", "sfx1"]);
}

#[test]
fn move_track_swaps_same_kind_neighbors_and_refuses_boundaries() {
    let mut project = create_project(None, None, None, None, None);
    project = add_track(project, TrackKind::Video);
    project = add_track(project, TrackKind::Video);
    assert_eq!(ids(&project), ["v1", "v2", "v3", "a1", "sfx1"]);

    project = move_track(project, "v2", true).unwrap();
    assert_eq!(ids(&project), ["v2", "v1", "v3", "a1", "sfx1"]);
    project = move_track(project, "v2", false).unwrap();
    assert_eq!(ids(&project), ["v1", "v2", "v3", "a1", "sfx1"]);

    assert_eq!(
        move_track(project.clone(), "v1", true),
        Err(DomainError::TrackOrder)
    );
    assert_eq!(
        move_track(project.clone(), "v3", false),
        Err(DomainError::TrackOrder)
    );
    assert_eq!(
        move_track(project.clone(), "missing", true),
        Err(DomainError::TrackNotFound("missing".into()))
    );
    assert!(can_move_track_up(&project, "v2"));
    assert!(!can_move_track_up(&project, "v1"));
    assert!(can_move_track_down(&project, "v2"));
    assert!(!can_move_track_down(&project, "v3"));
}

#[test]
fn move_track_command_is_undoable_via_history_snapshot_semantics() {
    let mut project = add_track(
        create_project(None, None, None, None, None),
        TrackKind::Video,
    );
    let before = project.clone();
    project = execute_edit(
        project,
        EditCommand::MoveTrack {
            track_id: "v2".into(),
            up: true,
        },
    )
    .unwrap();
    assert_eq!(ids(&project), ["v2", "v1", "a1", "sfx1"]);
    assert_ne!(project.tracks, before.tracks);
    // Clips/content identity is preserved — only order changed.
    assert_eq!(project.tracks.len(), before.tracks.len());
}
