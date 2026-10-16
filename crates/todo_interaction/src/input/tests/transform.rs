use super::*;
use todo_core::{find_clip, ClipTransform};

fn pointer(editor: &mut TestEditor, kind: PreviewPointerKind, x: f64, y: f64) {
    editor.apply(Intent::PreviewPointer { kind, x, y });
}

fn transform(editor: &TestEditor) -> ClipTransform {
    find_clip(editor.project(), "c1").unwrap().clip.transform
}

#[test]
fn preview_background_does_not_select_or_start_an_edit() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    pointer(&mut editor, PreviewPointerKind::Down, 1.2, 0.5);
    pointer(&mut editor, PreviewPointerKind::Up, 1.4, 0.5);
    assert_eq!(transform(&editor), ClipTransform::default());
    assert!(editor.view().timeline.selected_ids.is_empty());
    assert!(!editor.workspace.editor().unwrap().transaction_active());
    assert!(!editor.view().can_undo);
}

#[test]
fn selected_right_handle_resizes_about_the_opposite_edge() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    pointer(&mut editor, PreviewPointerKind::Down, 1.0, 0.5);
    pointer(&mut editor, PreviewPointerKind::Up, 1.2, 0.5);
    assert!((transform(&editor).x - 0.1).abs() < 1e-9);
    assert_eq!(transform(&editor).y, 0.0);
}

#[test]
fn selected_corner_keeps_proportions_and_the_opposite_corner() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    pointer(&mut editor, PreviewPointerKind::Down, 1.0, 1.0);
    pointer(&mut editor, PreviewPointerKind::Up, 1.5, 1.5);
    assert!((transform(&editor).scale - 1.5).abs() < 1e-9);
    assert!((transform(&editor).x - 0.25).abs() < 1e-9);
    assert!((transform(&editor).y - 0.25).abs() < 1e-9);
}

#[test]
fn only_the_selected_active_picture_projects_a_selection_box() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    assert!(editor.view().preview.selection.is_none());
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    assert!(editor.view().preview.selection.is_some());
    editor.apply(Intent::ClearSelection);
    assert!(editor.view().preview.selection.is_none());
    editor.apply(Intent::ToggleLinkedSelection);
    editor.apply(Intent::SelectClip {
        clip_id: "a1c".into(),
        additive: false,
    });
    assert!(editor.view().preview.selection.is_none());
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    editor.apply(Intent::Seek(4.0));
    assert!(editor.view().preview.clip_id.is_none());
    assert!(editor.view().preview.selection.is_none());
}

#[test]
fn fitted_picture_misses_ignore_letterbox_and_first_body_hit_selects_and_moves() {
    let mut project = with_clip();
    project.media[0].width = Some(9);
    project.media[0].height = Some(16);
    let mut editor = TestEditor::new(project, "demo");
    pointer(&mut editor, PreviewPointerKind::Down, 0.1, 0.5);
    pointer(&mut editor, PreviewPointerKind::Up, 0.2, 0.5);
    assert!(editor.view().timeline.selected_ids.is_empty());
    assert_eq!(transform(&editor), ClipTransform::default());
    pointer(&mut editor, PreviewPointerKind::Down, 0.5, 0.5);
    assert!(editor.view().preview.selection.is_some());
    pointer(&mut editor, PreviewPointerKind::Up, 1.5, -0.5);
    assert_eq!(transform(&editor).x, 1.0);
    assert_eq!(transform(&editor).y, -1.0);
}

#[test]
fn preview_selection_reprojects_after_viewport_changes_without_changing_the_project() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    let before = editor.project().clone();
    let fitted = editor.view().preview.selection.unwrap();
    assert_eq!(fitted.left, 0.0);
    assert_eq!(fitted.top, 0.0);
    editor.apply(Intent::PreviewViewport {
        width: 400.0,
        height: 400.0,
    });
    let square = editor
        .controller
        .preview_view(editor.workspace.editor().unwrap());
    let bounds = square.selection.unwrap();
    assert_eq!(bounds.left, 0.0);
    assert!((bounds.top - 0.21875).abs() < 1e-9);
    assert!((bounds.bottom - 0.78125).abs() < 1e-9);
    assert_eq!(editor.view().preview, square);
    assert_eq!(editor.project(), &before);
    assert!(!editor.view().can_undo);
}

#[test]
fn selected_locked_picture_retains_its_box_and_rejects_resize() {
    let mut project = with_clip();
    project.tracks[0].locked = true;
    let mut editor = TestEditor::new(project.clone(), "demo");
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    assert!(editor.view().preview.selection.is_some());
    assert!(!editor.view().preview.editable);
    pointer(&mut editor, PreviewPointerKind::Down, 1.0, 0.5);
    pointer(&mut editor, PreviewPointerKind::Up, 1.5, 0.5);
    assert_eq!(editor.project(), &project);
    assert!(!editor.workspace.editor().unwrap().transaction_active());
}

#[test]
fn lower_track_video_is_selectable_and_movable_beside_a_covering_picture() {
    // Covering still on v1; full-frame video on a lower track. Clicks outside the
    // still must still treat the video as a transformable element.
    let mut project = create_project(None, None, Some(10.0), None, None);
    project.media.push(MediaItem {
        id: "still".into(),
        name: "cover.png".into(),
        path: Some("data/cover.png".into()),
        duration: 5.0,
        has_audio: Some(false),
        width: Some(16),
        height: Some(9),
        kind: Some(todo_core::MediaKind::Image),
    });
    project.media.push(MediaItem {
        id: "movie".into(),
        name: "clip.mp4".into(),
        path: Some("data/clip.mp4".into()),
        duration: 10.0,
        has_audio: Some(false),
        width: Some(16),
        height: Some(9),
        kind: Some(todo_core::MediaKind::Video),
    });
    project = add_clip(project, "v1", {
        let mut clip = normalize_clip("cover".into(), "still".into(), 0.0, 5.0, 0.0, 1.0);
        clip.transform.scale = 0.5;
        clip
    });
    project = todo_core::add_track(project, TrackKind::Video);
    project = add_clip(
        project,
        "v2",
        normalize_clip("movie".into(), "movie".into(), 0.0, 5.0, 0.0, 1.0),
    );
    let mut editor = TestEditor::new(project, "demo");
    editor.apply(Intent::ToggleLinkedSelection);
    // Left edge of the canvas sits outside the half-scale cover, on the video.
    pointer(&mut editor, PreviewPointerKind::Down, 0.1, 0.5);
    assert!(
        editor
            .view()
            .timeline
            .selected_ids
            .contains(&"movie".into()),
        "lower video must become the active element: {:?}",
        editor.view().timeline.selected_ids
    );
    pointer(&mut editor, PreviewPointerKind::Move, 0.3, 0.5);
    pointer(&mut editor, PreviewPointerKind::Up, 0.3, 0.5);
    let moved = find_clip(editor.project(), "movie").unwrap().clip.transform;
    assert!(
        (moved.x - 0.2).abs() < 1e-6,
        "video element must move, got {moved:?}"
    );
    assert_eq!(
        find_clip(editor.project(), "cover")
            .unwrap()
            .clip
            .transform
            .scale,
        0.5
    );
}

#[test]
fn missing_source_metadata_declines_gizmo_and_pointer_edits() {
    let mut project = with_clip();
    project.media[0].height = None;
    let mut editor = TestEditor::new(project.clone(), "demo");
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    assert!(editor.view().preview.selection.is_none());
    pointer(&mut editor, PreviewPointerKind::Down, 0.5, 0.5);
    pointer(&mut editor, PreviewPointerKind::Up, 0.8, 0.5);
    assert_eq!(editor.project(), &project);
    assert!(!editor.workspace.editor().unwrap().transaction_active());
}

#[test]
fn transparent_picture_can_be_selected_resized_undone_and_redone() {
    let mut project = with_clip();
    project.tracks[0].clips[0].transform.opacity = 0.0;
    let mut editor = TestEditor::new(project, "demo");
    pointer(&mut editor, PreviewPointerKind::Down, 0.5, 0.5);
    pointer(&mut editor, PreviewPointerKind::Up, 0.5, 0.5);
    assert!(editor.view().preview.selection.is_some());
    pointer(&mut editor, PreviewPointerKind::Down, 1.0, 0.5);
    pointer(&mut editor, PreviewPointerKind::Move, 1.2, 0.8);
    pointer(&mut editor, PreviewPointerKind::Up, 1.5, 0.8);
    let resized = transform(&editor);
    assert_eq!(resized.opacity, 0.0);
    assert!((resized.scale_x - 1.5).abs() < 1e-9);
    assert_eq!(resized.scale_y, 1.0);
    assert_eq!(resized.y, 0.0);
    editor.apply(Intent::Undo);
    assert_eq!(
        transform(&editor),
        ClipTransform {
            opacity: 0.0,
            ..ClipTransform::default()
        }
    );
    assert!(!editor.view().can_undo);
    editor.apply(Intent::Redo);
    assert_eq!(transform(&editor), resized);
}

#[test]
fn resizing_uses_current_numeric_values_and_escape_preserves_the_numeric_edit() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    let numeric = ClipTransform {
        x: 0.1,
        y: 0.2,
        scale: 0.5,
        scale_x: 1.3,
        scale_y: 0.8,
        opacity: 0.4,
    };
    editor.apply(Intent::BeginEdit);
    editor.apply(Intent::SetClipTransform {
        clip_id: "c1".into(),
        transform: numeric,
    });
    let bounds = editor.view().preview.selection.unwrap();
    pointer(
        &mut editor,
        PreviewPointerKind::Down,
        bounds.right,
        bounds.bottom,
    );
    pointer(
        &mut editor,
        PreviewPointerKind::Move,
        bounds.right + bounds.width(),
        bounds.bottom + bounds.height(),
    );
    let resized = transform(&editor);
    assert!((resized.scale - 1.0).abs() < 1e-9);
    assert_eq!(resized.scale_x, numeric.scale_x);
    assert_eq!(resized.scale_y, numeric.scale_y);
    assert_eq!(resized.opacity, numeric.opacity);
    editor.apply(Intent::Key {
        code: KeyCode::Escape,
        typing: false,
    });
    pointer(&mut editor, PreviewPointerKind::Up, 1.5, 1.5);
    assert_eq!(transform(&editor), numeric);
    assert!(!editor.workspace.editor().unwrap().transaction_active());
    editor.apply(Intent::Undo);
    assert_eq!(transform(&editor), ClipTransform::default());
    assert!(!editor.view().can_undo);
    editor.apply(Intent::Redo);
    assert_eq!(transform(&editor), numeric);
}

#[test]
fn a_shrunken_unselected_picture_does_not_capture_a_click_outside_its_bounds() {
    let mut project = with_clip();
    project.tracks[0].clips[0].transform.scale = 0.5;
    let mut editor = TestEditor::new(project.clone(), "demo");
    pointer(&mut editor, PreviewPointerKind::Down, 0.1, 0.5);
    pointer(&mut editor, PreviewPointerKind::Up, 0.3, 0.5);
    assert!(editor.view().timeline.selected_ids.is_empty());
    assert_eq!(editor.project(), &project);
}

#[test]
fn invalid_pointer_up_finishes_the_gesture_at_its_last_valid_state() {
    for (x, y) in [(f64::NAN, 0.8), (f64::INFINITY, 0.8)] {
        let mut editor = TestEditor::new(with_clip(), "demo");
        pointer(&mut editor, PreviewPointerKind::Down, 0.5, 0.5);
        pointer(&mut editor, PreviewPointerKind::Move, 0.8, 0.6);
        let valid = transform(&editor);
        pointer(&mut editor, PreviewPointerKind::Up, x, y);
        assert!(!editor.workspace.editor().unwrap().transaction_active());
        assert_eq!(transform(&editor), valid);
        pointer(&mut editor, PreviewPointerKind::Move, 0.9, 0.7);
        assert_eq!(transform(&editor), valid);
        editor.apply(Intent::Undo);
        assert_eq!(transform(&editor), ClipTransform::default());
    }
}

#[test]
fn unrepresentable_resize_release_finishes_at_the_last_valid_state() {
    let mut project = with_clip();
    let original = ClipTransform {
        scale: 0.5,
        scale_x: 3.0,
        ..ClipTransform::default()
    };
    project.tracks[0].clips[0].transform = original;
    let mut editor = TestEditor::new(project, "demo");
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    let bounds = editor.view().preview.selection.unwrap();
    pointer(
        &mut editor,
        PreviewPointerKind::Down,
        bounds.right,
        bounds.center().1,
    );
    pointer(
        &mut editor,
        PreviewPointerKind::Move,
        bounds.right + 0.1,
        bounds.center().1,
    );
    let valid = transform(&editor);
    pointer(
        &mut editor,
        PreviewPointerKind::Up,
        f64::MAX,
        bounds.center().1,
    );
    assert!(!editor.workspace.editor().unwrap().transaction_active());
    assert_eq!(transform(&editor), valid);
    pointer(
        &mut editor,
        PreviewPointerKind::Move,
        bounds.right + 0.2,
        bounds.center().1,
    );
    assert_eq!(transform(&editor), valid);
    editor.apply(Intent::Undo);
    assert_eq!(transform(&editor), original);
}

fn anamorphic_editor() -> TestEditor {
    let mut project = with_clip();
    project.media[0].width = Some(720);
    project.media[0].height = Some(576);
    let mut editor = TestEditor::new(project, "demo");
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    editor
}

#[test]
fn decoded_source_aspect_overrides_raw_dimensions_without_editing_the_project() {
    let mut editor = anamorphic_editor();
    let before = editor.project().clone();
    let raw = editor.view().preview.selection.unwrap();
    assert!((raw.left - 0.1484375).abs() < 1e-9);
    let update = editor.apply(Intent::PreviewSourceAspect {
        path: "data/a.mp4".into(),
        aspect: 4.0 / 3.0,
    });
    let preview = editor.view().preview;
    let bounds = preview.selection.unwrap();
    assert!((bounds.left - 0.125).abs() < 1e-9);
    assert!((bounds.right - 0.875).abs() < 1e-9);
    assert_eq!(bounds.top, 0.0);
    assert_eq!(bounds.bottom, 1.0);
    assert_eq!(preview.aspect, Some((720, 576)));
    assert!(update.commands.is_empty());
    assert_eq!(editor.project(), &before);
    assert!(!editor.view().can_undo);
}

#[test]
fn decoded_aspect_drives_handle_hits_and_keeps_the_opposite_edge_fixed() {
    let mut editor = anamorphic_editor();
    editor.apply(Intent::PreviewSourceAspect {
        path: "data/a.mp4".into(),
        aspect: 4.0 / 3.0,
    });
    pointer(&mut editor, PreviewPointerKind::Down, 0.875, 0.5);
    pointer(&mut editor, PreviewPointerKind::Up, 1.025, 0.5);
    let resized = transform(&editor);
    assert!((resized.scale_x - 1.2).abs() < 1e-9);
    assert!((resized.x - 0.075).abs() < 1e-9);
    let bounds = editor.view().preview.selection.unwrap();
    assert!((bounds.left - 0.125).abs() < 1e-9);
    assert!((bounds.right - 1.025).abs() < 1e-9);
    editor.apply(Intent::Undo);
    assert_eq!(transform(&editor), ClipTransform::default());
}

#[test]
fn invalid_or_other_path_aspect_events_cannot_replace_the_current_decoded_aspect() {
    let mut editor = anamorphic_editor();
    editor.apply(Intent::PreviewSourceAspect {
        path: "data/a.mp4".into(),
        aspect: 4.0 / 3.0,
    });
    let expected = editor.view().preview.selection.unwrap();
    assert!((expected.left - 0.125).abs() < 1e-9);
    for (path, aspect) in [
        ("data/other.mp4", 9.0 / 16.0),
        ("data\\a.mp4", 1.0),
        ("data/a.mp4", f64::NAN),
        ("data/a.mp4", f64::INFINITY),
        ("data/a.mp4", 0.0),
        ("data/a.mp4", -1.0),
    ] {
        editor.apply(Intent::PreviewSourceAspect {
            path: path.into(),
            aspect,
        });
        assert_eq!(editor.view().preview.selection, Some(expected));
    }
    assert!(!editor.view().can_undo);
}

#[test]
fn decoded_aspect_can_enable_handles_when_raw_dimensions_are_missing() {
    let mut project = with_clip();
    project.media[0].kind = Some(todo_core::MediaKind::Video);
    project.media[0].width = None;
    project.media[0].height = None;
    let mut editor = TestEditor::new(project, "demo");
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    assert!(editor.view().preview.selection.is_none());
    editor.apply(Intent::PreviewSourceAspect {
        path: "data/a.mp4".into(),
        aspect: 4.0 / 3.0,
    });
    let bounds = editor.view().preview.selection.unwrap();
    assert!((bounds.left - 0.125).abs() < 1e-9);
    pointer(&mut editor, PreviewPointerKind::Down, bounds.right, 0.5);
    pointer(
        &mut editor,
        PreviewPointerKind::Up,
        bounds.right + 0.15,
        0.5,
    );
    assert!((transform(&editor).scale_x - 1.2).abs() < 1e-9);
}

#[test]
fn decoder_aspect_changes_during_capture_preserve_the_gesture_starting_basis() {
    let mut editor = anamorphic_editor();
    editor.apply(Intent::PreviewSourceAspect {
        path: "data/a.mp4".into(),
        aspect: 4.0 / 3.0,
    });
    pointer(&mut editor, PreviewPointerKind::Down, 0.875, 0.5);
    pointer(&mut editor, PreviewPointerKind::Move, 1.025, 0.5);
    let before = transform(&editor);
    editor.apply(Intent::PreviewSourceAspect {
        path: "data/a.mp4".into(),
        aspect: 1.0,
    });
    pointer(&mut editor, PreviewPointerKind::Up, 1.025, 0.5);
    assert_eq!(transform(&editor), before);
    assert!((transform(&editor).scale_x - 1.2).abs() < 1e-9);
    assert!(!editor.workspace.editor().unwrap().transaction_active());
}

#[test]
fn a_different_source_uses_raw_metadata_until_its_own_decoder_event_arrives() {
    let mut project = with_clip();
    project.media[0].width = Some(720);
    project.media[0].height = Some(576);
    let mut next = project.media[0].clone();
    next.id = "m2".into();
    next.path = Some("data/b.mp4".into());
    next.width = Some(16);
    next.height = Some(9);
    project.media.push(next);
    project = add_clip(
        project,
        "v1",
        normalize_clip("c2".into(), "m2".into(), 4.0, 4.0, 0.0, 1.0),
    );
    let mut editor = TestEditor::new(project, "demo");
    editor.apply(Intent::PreviewSourceAspect {
        path: "data/a.mp4".into(),
        aspect: 4.0 / 3.0,
    });
    editor.apply(Intent::Seek(4.0));
    editor.apply(Intent::SelectClip {
        clip_id: "c2".into(),
        additive: false,
    });
    let raw = editor.view().preview.selection.unwrap();
    assert_eq!(raw.left, 0.0);
    assert_eq!(raw.right, 1.0);
    editor.apply(Intent::PreviewSourceAspect {
        path: "data/a.mp4".into(),
        aspect: 9.0 / 16.0,
    });
    assert_eq!(editor.view().preview.selection, Some(raw));
    editor.apply(Intent::PreviewSourceAspect {
        path: "data/b.mp4".into(),
        aspect: 4.0 / 3.0,
    });
    let decoded = editor.view().preview.selection.unwrap();
    assert!((decoded.left - 0.125).abs() < 1e-9);
    assert!((decoded.right - 0.875).abs() < 1e-9);
    assert!(!editor.view().can_undo);
}
