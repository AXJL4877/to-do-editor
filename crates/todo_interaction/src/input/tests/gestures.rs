use super::*;

#[test]
fn track_add_input_inserts_below_the_audio_anchor_and_undo_restores_order() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    let before = editor.project().clone();
    editor.apply(Intent::InsertTrackBelow {
        track_id: "a1".into(),
    });
    let anchor = editor
        .project()
        .tracks
        .iter()
        .position(|t| t.id == "a1")
        .unwrap();
    let inserted = &editor.project().tracks[anchor + 1];
    assert_eq!(inserted.kind, TrackKind::Audio);
    assert_eq!(inserted.id, "a2");
    assert!(inserted.clips.is_empty());
    editor.apply(Intent::Undo);
    assert_eq!(editor.project(), &before);
}

#[test]
fn track_move_swaps_same_kind_neighbors_and_undo_restores_order() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::InsertTrackBelow {
        track_id: "v1".into(),
    });
    let before = editor
        .project()
        .tracks
        .iter()
        .map(|t| t.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(before[0], "v1");
    assert_eq!(before[1], "v2");
    assert!(editor.view().timeline.tracks[1].can_move_up);
    assert!(!editor.view().timeline.tracks[0].can_move_up);
    editor.apply(Intent::MoveTrack {
        track_id: "v2".into(),
        up: true,
    });
    assert_eq!(editor.project().tracks[0].id, "v2");
    assert_eq!(editor.project().tracks[1].id, "v1");
    editor.apply(Intent::Undo);
    assert_eq!(
        editor
            .project()
            .tracks
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        before.iter().map(String::as_str).collect::<Vec<_>>()
    );
}

#[test]
fn changing_inspector_selection_commits_the_previous_clip_field() {
    let project = todo_core::add_clip(
        with_clip(),
        "v1",
        todo_core::normalize_clip("second".into(), "m1".into(), 5.0, 2.0, 0.0, 1.0),
    );
    let mut editor = TestEditor::new(project, "demo");
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    editor.apply(Intent::BeginEdit);
    editor.apply(Intent::SetClipDuration {
        clip_id: "c1".into(),
        seconds: 2.0,
    });
    editor.apply(Intent::SelectClip {
        clip_id: "second".into(),
        additive: false,
    });
    editor.apply(Intent::BeginEdit);
    editor.apply(Intent::SetClipTransform {
        clip_id: "second".into(),
        transform: ClipTransform {
            x: 0.2,
            y: 0.0,
            scale: 1.0,
            ..ClipTransform::default()
        },
    });
    editor.apply(Intent::CommitEdit);
    assert_eq!(
        find_clip(editor.project(), "c1").unwrap().clip.duration,
        2.0
    );
    assert_eq!(editor.view().properties.unwrap().clip_id, "second");
    editor.apply(Intent::Undo);
    assert_eq!(
        find_clip(editor.project(), "c1").unwrap().clip.duration,
        2.0
    );
    editor.apply(Intent::Undo);
    assert_eq!(
        find_clip(editor.project(), "c1").unwrap().clip.duration,
        4.0
    );
}

#[test]
fn preview_drag_preserves_and_separates_an_active_numeric_edit() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::BeginEdit);
    editor.apply(Intent::SetClipDuration {
        clip_id: "c1".into(),
        seconds: 2.0,
    });
    editor.apply(Intent::PreviewPointer {
        kind: PreviewPointerKind::Down,
        x: 0.5,
        y: 0.5,
    });
    editor.apply(Intent::PreviewPointer {
        kind: PreviewPointerKind::Move,
        x: 0.7,
        y: 0.5,
    });
    // A field can report lost_focus after the native popup already captured the mouse.
    editor.apply(Intent::CommitEdit);
    editor.apply(Intent::PreviewPointer {
        kind: PreviewPointerKind::Move,
        x: 0.8,
        y: 0.5,
    });
    editor.apply(Intent::PreviewPointer {
        kind: PreviewPointerKind::Up,
        x: 0.8,
        y: 0.5,
    });
    assert_eq!(
        find_clip(editor.project(), "c1").unwrap().clip.duration,
        2.0
    );
    editor.apply(Intent::Undo);
    let clip = find_clip(editor.project(), "c1").unwrap().clip;
    assert_eq!(clip.duration, 2.0);
    assert_eq!(clip.transform, ClipTransform::default());
    editor.apply(Intent::Undo);
    assert_eq!(
        find_clip(editor.project(), "c1").unwrap().clip.duration,
        4.0
    );
}
use todo_application::AudioClockSample;
use todo_core::{find_clip, ClipTransform};

#[test]
fn move_targets_use_the_primary_selection_and_all_linked_locks() {
    let mut project = todo_core::add_track(with_clip(), TrackKind::Video);
    for clip in project.tracks.iter_mut().flat_map(|t| &mut t.clips) {
        clip.link_id = Some("pair".into());
    }
    let mut editor = TestEditor::new(project, "demo");
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    let target = || "v2".to_owned();
    assert!(
        editor
            .view()
            .timeline
            .move_targets
            .iter()
            .find(|t| t.track_id == target())
            .unwrap()
            .enabled
    );
    editor.apply(Intent::ToggleLock {
        track_id: "a1".into(),
    });
    assert!(
        !editor
            .view()
            .timeline
            .move_targets
            .iter()
            .find(|t| t.track_id == target())
            .unwrap()
            .enabled
    );
    editor.apply(Intent::ToggleLock {
        track_id: "a1".into(),
    });
    editor.apply(Intent::ToggleLinkedSelection);
    editor.apply(Intent::SelectClip {
        clip_id: "a1c".into(),
        additive: false,
    });
    assert!(
        !editor
            .view()
            .timeline
            .move_targets
            .iter()
            .find(|t| t.track_id == target())
            .unwrap()
            .enabled
    );
}

fn pointer(editor: &mut TestEditor, kind: PreviewPointerKind, x: f64, y: f64) -> TestUpdate {
    editor.apply(Intent::PreviewPointer { kind, x, y })
}

fn transform(editor: &TestEditor) -> ClipTransform {
    find_clip(editor.project(), "c1").unwrap().clip.transform
}

#[test]
fn preview_drag_updates_normalized_position_and_is_one_undoable_edit() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    pointer(&mut editor, PreviewPointerKind::Down, 0.5, 0.5);
    pointer(&mut editor, PreviewPointerKind::Move, 0.7, 0.4);
    let update = pointer(&mut editor, PreviewPointerKind::Move, 0.8, 0.3);
    assert!((transform(&editor).x - 0.3).abs() < 1e-9);
    assert!((transform(&editor).y + 0.2).abs() < 1e-9);
    assert!(update.commands.iter().any(|command| matches!(
        command,
        PreviewCommand::Layers(layers)
            if layers.len() == 1 && (layers[0].transform.x - 0.3).abs() < 1e-9
    )));
    pointer(&mut editor, PreviewPointerKind::Up, 0.8, 0.3);
    assert!(!editor.workspace.editor().unwrap().transaction_active());
    editor.apply(Intent::Undo);
    assert_eq!(transform(&editor), ClipTransform::default());
    assert!(!editor.view().can_undo);
    editor.apply(Intent::Redo);
    assert!((transform(&editor).x - 0.3).abs() < 1e-9);
}

#[test]
fn escape_cancels_preview_drag_and_ignores_later_pointer_events() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    let before = editor.project().clone();
    pointer(&mut editor, PreviewPointerKind::Down, 0.5, 0.5);
    pointer(&mut editor, PreviewPointerKind::Move, 0.8, 0.3);
    editor.apply(Intent::Key {
        code: KeyCode::Escape,
        typing: false,
    });
    pointer(&mut editor, PreviewPointerKind::Move, 0.9, 0.2);
    pointer(&mut editor, PreviewPointerKind::Up, 0.9, 0.2);
    assert_eq!(editor.project(), &before);
    assert!(!editor.view().can_undo);
    assert!(!editor.workspace.editor().unwrap().transaction_active());
}

#[test]
fn locked_preview_picture_rejects_drag_without_opening_a_transaction() {
    let mut project = with_clip();
    project.tracks[0].locked = true;
    let mut editor = TestEditor::new(project.clone(), "demo");
    pointer(&mut editor, PreviewPointerKind::Down, 0.5, 0.5);
    pointer(&mut editor, PreviewPointerKind::Move, 0.8, 0.3);
    pointer(&mut editor, PreviewPointerKind::Up, 0.8, 0.3);
    assert_eq!(editor.project(), &project);
    assert!(!editor.view().preview.editable);
    assert!(!editor.view().can_undo);
    assert!(!editor.workspace.editor().unwrap().transaction_active());
}

#[test]
fn preview_center_snap_obeys_toggle_and_rejects_nonfinite_viewport_dimensions() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::PreviewViewport {
        width: f64::INFINITY,
        height: f64::INFINITY,
    });
    pointer(&mut editor, PreviewPointerKind::Down, 0.5, 0.5);
    pointer(&mut editor, PreviewPointerKind::Up, 0.506, 0.51);
    assert_eq!(transform(&editor), ClipTransform::default());
    editor.apply(Intent::ToggleSnapping);
    pointer(&mut editor, PreviewPointerKind::Down, 0.5, 0.5);
    pointer(&mut editor, PreviewPointerKind::Up, 0.506, 0.51);
    assert!((transform(&editor).x - 0.006).abs() < 1e-9);
    assert!((transform(&editor).y - 0.01).abs() < 1e-9);
}

#[test]
fn preview_drag_stops_playback_before_changing_the_visible_picture() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::TogglePlay);
    assert!(editor.playing());
    let update = pointer(&mut editor, PreviewPointerKind::Down, 0.5, 0.5);
    assert!(!editor.playing());
    assert!(update
        .commands
        .iter()
        .any(|command| matches!(command, PreviewCommand::Transport { playing: false, .. })));
}

#[test]
fn both_trim_edges_snap_to_playhead_and_edit_linked_members_together() {
    for (local_x, edge_time, target_time, expected_start, expected_duration, expected_offset) in [
        (1.0, 0.0, 1.0, 1.0, 3.0, 1.0),
        (191.0, 4.0, 2.0, 0.0, 2.0, 0.0),
    ] {
        let mut project = with_clip();
        for clip in project.tracks.iter_mut().flat_map(|track| &mut track.clips) {
            clip.link_id = Some("pair".into());
        }
        let mut editor = TestEditor::new(project, "demo");
        editor.apply(Intent::Seek(target_time));
        editor.apply(Intent::ClipPointerDown {
            clip_id: "c1".into(),
            view_x: LABEL_W + edge_time * 48.0,
            scroll_left: 0.0,
            local_x,
            width_px: 192.0,
            alt: false,
        });
        editor.apply(Intent::PointerMove {
            view_x: LABEL_W + (target_time + 0.1) * 48.0,
            scroll_left: 0.0,
            track_index: None,
        });
        assert_eq!(editor.view().timeline.snap_target, Some(target_time));
        editor.apply(Intent::PointerUp);
        for clip in editor
            .project()
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
        {
            assert_eq!(clip.start, expected_start);
            assert_eq!(clip.duration, expected_duration);
            assert_eq!(clip.media().unwrap().offset, expected_offset);
        }
        editor.apply(Intent::Undo);
        assert_eq!(
            find_clip(editor.project(), "c1").unwrap().clip.duration,
            4.0
        );
        assert!(!editor.view().can_undo);
    }
}

#[test]
fn disabled_timeline_snap_keeps_frame_aligned_trim_position() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::Seek(2.0));
    editor.apply(Intent::ToggleSnapping);
    editor.apply(Intent::ClipPointerDown {
        clip_id: "c1".into(),
        view_x: LABEL_W + 192.0,
        scroll_left: 0.0,
        local_x: 191.0,
        width_px: 192.0,
        alt: false,
    });
    editor.apply(Intent::PointerMove {
        view_x: LABEL_W + 2.1 * 48.0,
        scroll_left: 0.0,
        track_index: None,
    });
    assert_eq!(editor.view().timeline.snap_target, None);
    editor.apply(Intent::PointerUp);
    assert_eq!(
        find_clip(editor.project(), "c1").unwrap().clip.duration,
        2.1
    );
}

#[test]
fn timeline_move_can_reassign_video_clips_onto_the_same_track() {
    // Spill / extra video lanes leave clips on different rows; dragging must
    // accept a compatible destination lane, not only horizontal time shifts.
    let mut project = create_project(None, None, Some(20.0), None, None);
    project.media.push(MediaItem {
        id: "m1".into(),
        name: "clip.mp4".into(),
        path: Some("data/a.mp4".into()),
        duration: 10.0,
        has_audio: Some(false),
        width: Some(16),
        height: Some(9),
        kind: None,
    });
    project = add_clip(
        project,
        "v1",
        normalize_clip("c1".into(), "m1".into(), 0.0, 2.0, 0.0, 1.0),
    );
    project = todo_core::add_track(project, TrackKind::Video);
    let v2 = project
        .tracks
        .iter()
        .find(|t| t.id == "v2")
        .unwrap()
        .id
        .clone();
    project = add_clip(
        project,
        &v2,
        normalize_clip("c2".into(), "m1".into(), 3.0, 2.0, 0.0, 1.0),
    );
    let v1_index = project
        .tracks
        .iter()
        .position(|t| t.id == "v1")
        .expect("v1");
    assert!(todo_core::can_move_clips(&project, &["c2".into()], "v1").is_ok());
    let mut editor = TestEditor::new(project, "demo");
    editor.apply(Intent::ToggleLinkedSelection);
    editor.apply(Intent::ClipPointerDown {
        clip_id: "c2".into(),
        view_x: LABEL_W + 3.0 * 48.0 + 20.0,
        scroll_left: 0.0,
        local_x: 20.0,
        width_px: 96.0,
        alt: false,
    });
    editor.apply(Intent::PointerMove {
        view_x: LABEL_W + 3.0 * 48.0 + 20.0,
        scroll_left: 0.0,
        track_index: Some(v1_index),
    });
    editor.apply(Intent::PointerUp);
    let found = find_clip(editor.project(), "c2").unwrap();
    assert_eq!(found.track_id, "v1");
    assert!((found.clip.start - 3.0).abs() < 1e-6);
    assert_eq!(
        editor
            .project()
            .tracks
            .iter()
            .find(|t| t.id == "v1")
            .unwrap()
            .clips
            .len(),
        2
    );
}

#[test]
fn timeline_move_snaps_trailing_edge_and_undo_restores_origin() {
    let mut project = with_clip();
    project.media[0].duration = 20.0;
    project.tracks[0].clips[0].duration = 2.0;
    project = add_clip(
        project,
        "v1",
        normalize_clip("target".into(), "m1".into(), 5.0, 2.0, 0.0, 1.0),
    );
    let mut editor = TestEditor::new(project, "demo");
    editor.apply(Intent::ClipPointerDown {
        clip_id: "c1".into(),
        view_x: LABEL_W + 40.0,
        scroll_left: 0.0,
        local_x: 40.0,
        width_px: 96.0,
        alt: false,
    });
    editor.apply(Intent::PointerMove {
        view_x: LABEL_W + 40.0 + 2.94 * 48.0,
        scroll_left: 0.0,
        track_index: None,
    });
    assert_eq!(editor.view().timeline.snap_target, Some(5.0));
    editor.apply(Intent::PointerUp);
    assert_eq!(find_clip(editor.project(), "c1").unwrap().clip.start, 3.0);
    editor.apply(Intent::Undo);
    assert_eq!(find_clip(editor.project(), "c1").unwrap().clip.start, 0.0);
}

#[test]
fn lane_razor_uses_the_same_playhead_snap_as_clip_razor() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::Seek(1.0));
    editor.apply(Intent::ToolClicked(TimelineTool::Razor));
    editor.apply(Intent::LanePointerDown {
        track_id: "v1".into(),
        view_x: LABEL_W + 1.1 * 48.0,
        scroll_left: 0.0,
    });
    assert_eq!(
        find_clip(editor.project(), "c1").unwrap().clip.duration,
        1.0
    );
    assert_eq!(editor.playhead(), 1.0);
}

#[test]
fn host_audio_clock_events_stay_in_application_and_drive_projected_playhead() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::TogglePlay);
    editor.command(EditorCommand::Frame { now: 0.0 });
    let epoch = editor.workspace.editor().unwrap().clock_epoch();
    editor.command(EditorCommand::Clock {
        epoch,
        now: 0.1,
        sample: Some(AudioClockSample {
            time: 2.0,
            playing: true,
            available: true,
        }),
    });
    editor.command(EditorCommand::Frame { now: 0.2 });
    assert!((editor.view().current_time - 2.1).abs() < 1e-9);
    assert!(editor.playing());
}
