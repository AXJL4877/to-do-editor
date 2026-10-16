use super::*;

fn pointer(editor: &mut TestEditor, kind: PreviewPointerKind, x: f64, y: f64) {
    editor.apply(Intent::PreviewPointer { kind, x, y });
}

#[test]
fn viewport_navigation_preserves_project_history_and_decoding() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.sync();
    let original = editor.project().clone();
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    assert!(editor.apply(Intent::PreviewZoom(0.5)).commands.is_empty());
    let bounds = editor.view().preview.selection.unwrap();
    assert_eq!(
        (bounds.left, bounds.top, bounds.right, bounds.bottom),
        (0.25, 0.25, 0.75, 0.75)
    );
    for (kind, x) in [
        (PreviewPointerKind::Down, 0.5),
        (PreviewPointerKind::Move, 0.6),
        (PreviewPointerKind::Up, 0.6),
    ] {
        assert!(editor
            .apply(Intent::PreviewPan { kind, x, y: 0.5 })
            .commands
            .is_empty());
    }
    assert!((editor.view().preview.selection.unwrap().left - 0.35).abs() < 1e-9);
    editor.apply(Intent::PreviewOutputOnly(true));
    assert!(editor.view().preview.selection.is_none());
    editor.apply(Intent::PreviewFit);
    assert_eq!(editor.project(), &original);
    assert!(!editor.view().dirty);
    assert!(!editor.view().can_undo);
    assert!(!editor.workspace.editor().unwrap().transaction_active());
}

#[test]
fn an_element_outside_the_output_frame_can_be_dragged_inside_and_undone() {
    let mut project = with_clip();
    project.tracks[0].clips[0].transform.x = -0.75;
    project.tracks[0].clips[0].transform.scale = 0.25;
    let mut editor = TestEditor::new(project.clone(), "demo");
    editor.apply(Intent::PreviewZoom(0.5));
    pointer(&mut editor, PreviewPointerKind::Down, 0.125, 0.5);
    pointer(&mut editor, PreviewPointerKind::Move, 0.5, 0.5);
    pointer(&mut editor, PreviewPointerKind::Up, 0.5, 0.5);
    let transform = editor.project().tracks[0].clips[0].transform;
    assert_eq!(transform.x, 0.0);
    assert_eq!(transform.scale, 0.25);
    editor.apply(Intent::Undo);
    assert_eq!(editor.project(), &project);
    assert!(!editor.view().can_undo);
}

#[test]
fn navigation_during_a_transform_does_not_change_its_coordinate_basis() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::PreviewZoom(0.5));
    pointer(&mut editor, PreviewPointerKind::Down, 0.5, 0.5);
    editor.apply(Intent::PreviewZoom(2.0));
    editor.apply(Intent::PreviewFit);
    editor.apply(Intent::PreviewOutputOnly(true));
    pointer(&mut editor, PreviewPointerKind::Up, 0.6, 0.5);
    assert!((editor.project().tracks[0].clips[0].transform.x - 0.2).abs() < 1e-9);
    assert_eq!(editor.view().preview.viewport.zoom(), 0.5);
    assert!(!editor.view().preview.viewport.output_only());
}

#[test]
fn late_pan_motion_after_escape_does_not_restart_navigation() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.apply(Intent::PreviewPan {
        kind: PreviewPointerKind::Down,
        x: 0.5,
        y: 0.5,
    });
    editor.apply(Intent::Key {
        code: KeyCode::Escape,
        typing: false,
    });
    let before = editor.view().preview;
    editor.apply(Intent::PreviewPan {
        kind: PreviewPointerKind::Move,
        x: 0.7,
        y: 0.5,
    });
    editor.apply(Intent::PreviewPan {
        kind: PreviewPointerKind::Up,
        x: 0.8,
        y: 0.5,
    });
    assert_eq!(editor.view().preview, before);
    assert!(!editor.controller.preview_gesture_active());
}

#[test]
fn canceling_a_close_prompt_does_not_leave_the_hidden_native_pan_active() {
    let mut editor = TestEditor::new(with_clip(), "demo");
    editor.command(EditorCommand::Edit(todo_core::EditCommand::SetTransform {
        clip_id: "c1".into(),
        transform: ClipTransform {
            x: 0.1,
            ..Default::default()
        },
    }));
    editor.apply(Intent::PreviewPan {
        kind: PreviewPointerKind::Down,
        x: 0.5,
        y: 0.5,
    });
    assert!(editor.controller.preview_gesture_active());
    editor.apply(Intent::RequestClose);
    assert!(editor.workspace.close_prompt());
    // Hiding the native popup finishes capture, but modal input is discarded.
    assert!(!editor.controller.preview_gesture_active());
    editor.apply(Intent::ConfirmClose(todo_application::CloseChoice::Cancel));
    pointer(&mut editor, PreviewPointerKind::Down, 0.5, 0.5);
    assert!(editor.workspace.editor().unwrap().transaction_active());
    pointer(&mut editor, PreviewPointerKind::Up, 0.5, 0.5);
}
