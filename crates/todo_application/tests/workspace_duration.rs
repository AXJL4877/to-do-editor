use todo_application::{EditorCommand, EditorSession};
use todo_core::{create_project, MediaItem, MediaKind};

fn media(id: &str) -> MediaItem {
    MediaItem {
        id: id.into(),
        name: format!("{id}.mp4"),
        path: None,
        duration: 600.0,
        has_audio: Some(true),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Video),
        folder_id: None,
        alpha: None,
        reveal: None,
    }
}

#[test]
fn successive_long_imports_keep_workspace_fixed_full_clips_and_bounded_playhead() {
    let mut editor =
        EditorSession::new(create_project(None, None, Some(300.0), None, None), "test");
    for id in ["first", "second"] {
        let result = editor.apply(EditorCommand::Imported(media(id)));
        assert!(result.error.is_none());
        assert_eq!(editor.project().duration, Some(300.0));
        assert!(editor.playhead() <= 300.0);
    }
    let clips: Vec<_> = editor
        .project()
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .collect();
    assert_eq!(clips.len(), 4);
    assert!(clips.iter().all(|clip| clip.duration == 600.0));
    assert_eq!(clips.iter().filter(|clip| clip.start == 600.0).count(), 2);
    let _ = editor.apply(EditorCommand::Undo);
    assert_eq!(editor.project().duration, Some(300.0));
    assert_eq!(editor.project().media.len(), 1);
}
