use super::*;
use todo_application::{AppEvent, MediaOutput, MediaTask};
use todo_core::{find_clip, TransformBounds};

/// Add text at the playhead and deliver its raster with a known text block.
fn editor_with_text() -> (TestEditor, String) {
    let mut editor = TestEditor::new(with_clip(), "demo");
    let update = editor
        .controller
        .apply(&mut editor.workspace, Intent::AddText);
    let text_id = update.new_clips[0].clone();
    let ticket = update
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Media {
                ticket,
                task: MediaTask::TextRaster(_),
            } => Some(*ticket),
            _ => None,
        })
        .expect("text raster requested");
    let done = editor.workspace.on_event(AppEvent::Media {
        ticket,
        result: Ok(MediaOutput::TextRaster {
            path: "text.png".into(),
            block: TransformBounds {
                left: 0.3,
                top: 0.45,
                right: 0.7,
                bottom: 0.55,
            },
        }),
    });
    editor.controller.reconcile(&editor.workspace);
    assert!(done.effects.iter().any(|e| matches!(e, Effect::Preview(_))));
    (editor, text_id)
}

fn click(editor: &mut TestEditor, x: f64, y: f64) {
    editor.apply(Intent::PreviewPointer {
        kind: PreviewPointerKind::Down,
        x,
        y,
    });
    editor.apply(Intent::PreviewPointer {
        kind: PreviewPointerKind::Up,
        x,
        y,
    });
}

#[test]
fn add_text_selects_the_new_clip_on_a_text_lane() {
    let (editor, text_id) = editor_with_text();
    let found = find_clip(editor.project(), &text_id).unwrap();
    assert_eq!(found.track.kind, TrackKind::Text);
    assert_eq!(editor.view().timeline.selected_ids, vec![text_id.clone()]);
    let properties = editor.view().properties.unwrap();
    assert!(matches!(
        properties.source,
        crate::SourceProperties::Text(ref text) if text.content.text == "新文字"
    ));
}

#[test]
fn text_is_picked_by_its_block_and_video_stays_reachable_around_it() {
    let (mut editor, text_id) = editor_with_text();
    editor.apply(Intent::ClearSelection);
    click(&mut editor, 0.5, 0.5);
    assert_eq!(editor.view().timeline.selected_ids, vec![text_id]);
    editor.apply(Intent::ClearSelection);
    // Inside the full frame but outside the words: the video below is picked.
    click(&mut editor, 0.1, 0.2);
    assert_eq!(editor.view().timeline.selected_ids[0], "c1");
}

#[test]
fn text_selection_box_hugs_the_block_and_drags_move_the_text() {
    let (mut editor, text_id) = editor_with_text();
    let selection = editor.view().preview.selection.expect("text box");
    let projection = editor.view().preview.projection.unwrap();
    let expected = projection
        .project_bounds(TransformBounds {
            left: 0.3,
            top: 0.45,
            right: 0.7,
            bottom: 0.55,
        })
        .unwrap();
    for (got, want) in [
        (selection.left, expected.left),
        (selection.top, expected.top),
        (selection.right, expected.right),
        (selection.bottom, expected.bottom),
    ] {
        assert!((got - want).abs() < 1e-9, "{got} vs {want}");
    }
    editor.apply(Intent::PreviewPointer {
        kind: PreviewPointerKind::Down,
        x: 0.5,
        y: 0.5,
    });
    editor.apply(Intent::PreviewPointer {
        kind: PreviewPointerKind::Up,
        x: 0.6,
        y: 0.5,
    });
    let moved = find_clip(editor.project(), &text_id)
        .unwrap()
        .clip
        .transform;
    assert!(moved.x > 0.05, "text moved right: {}", moved.x);
    assert_eq!(moved.scale, 1.0);
}
