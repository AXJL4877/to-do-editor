use super::*;
use todo_core::{create_folder, create_project, MediaItem, MediaKind};

fn media(id: &str, folder_id: Option<&str>) -> MediaItem {
    MediaItem {
        id: id.into(),
        name: format!("{id}.mp4"),
        path: None,
        duration: 4.0,
        has_audio: Some(false),
        width: Some(8),
        height: Some(8),
        kind: Some(MediaKind::Video),
        alpha: None,
        reveal: None,
        folder_id: folder_id.map(str::to_owned),
    }
}

#[test]
fn library_view_filters_to_selected_folder_and_search_crosses_folders() {
    let mut project = create_project(None, None, None, None, None);
    project = create_folder(project, "a".into(), "Alpha".into(), None).unwrap();
    project = create_folder(project, "b".into(), "Beta".into(), Some("a".into())).unwrap();
    project.media.push(media("root-clip", None));
    project.media.push(media("nested", Some("b")));
    let mut controller = EditorController::new();
    let mut workspace = Workspace::with_project(project, "folders");
    let _ = controller.apply(
        &mut workspace,
        Intent::LibrarySelectFolder(Some("b".into())),
    );
    let view = controller.view(workspace.editor().unwrap(), &[]);
    assert_eq!(view.library.entries.len(), 1);
    match &view.library.entries[0] {
        crate::LibraryEntryView::Media(item) => assert_eq!(item.item.id, "nested"),
        _ => panic!("expected media"),
    }
    let _ = controller.apply(&mut workspace, Intent::LibraryQuery("root".into()));
    let view = controller.view(workspace.editor().unwrap(), &[]);
    assert!(view.library.searching);
    assert_eq!(view.library.items.len(), 1);
    assert_eq!(view.library.items[0].item.id, "root-clip");
}

#[test]
fn import_pressed_sets_choose_media_effect_for_current_folder() {
    let mut project = create_project(None, None, None, None, None);
    project = create_folder(project, "bin".into(), "Bin".into(), None).unwrap();
    let mut controller = EditorController::new();
    let mut workspace = Workspace::with_project(project, "import-folder");
    let _ = controller.apply(
        &mut workspace,
        Intent::LibrarySelectFolder(Some("bin".into())),
    );
    let update = controller.apply(&mut workspace, Intent::ImportPressed);
    assert!(update
        .effects
        .iter()
        .any(|effect| matches!(effect, Effect::ChooseMediaFiles)));
}

#[test]
fn shift_click_selects_a_contiguous_media_range() {
    let mut project = create_project(None, None, None, None, None);
    project.media.push(media("a", None));
    project.media.push(media("b", None));
    project.media.push(media("c", None));
    let mut controller = EditorController::new();
    let mut workspace = Workspace::with_project(project, "shift-range");
    let _ = controller.apply(
        &mut workspace,
        Intent::LibrarySelect {
            id: "a".into(),
            additive: false,
            range: false,
        },
    );
    let _ = controller.apply(
        &mut workspace,
        Intent::LibrarySelect {
            id: "c".into(),
            additive: false,
            range: true,
        },
    );
    let view = controller.view(workspace.editor().unwrap(), &[]);
    assert_eq!(view.library.selected_ids, vec!["a", "b", "c"]);
}

#[test]
fn ctrl_click_toggles_media_in_the_selection() {
    let mut project = create_project(None, None, None, None, None);
    project.media.push(media("a", None));
    project.media.push(media("b", None));
    let mut controller = EditorController::new();
    let mut workspace = Workspace::with_project(project, "ctrl-toggle");
    let _ = controller.apply(
        &mut workspace,
        Intent::LibrarySelect {
            id: "a".into(),
            additive: false,
            range: false,
        },
    );
    let _ = controller.apply(
        &mut workspace,
        Intent::LibrarySelect {
            id: "b".into(),
            additive: true,
            range: false,
        },
    );
    let view = controller.view(workspace.editor().unwrap(), &[]);
    assert_eq!(view.library.selected_ids, vec!["a", "b"]);
    let _ = controller.apply(
        &mut workspace,
        Intent::LibrarySelect {
            id: "a".into(),
            additive: true,
            range: false,
        },
    );
    assert_eq!(
        controller
            .view(workspace.editor().unwrap(), &[])
            .library
            .selected_ids,
        vec!["b"]
    );
}

#[test]
fn selecting_media_selects_its_folder_as_destination_without_highlighting_two_rows() {
    let mut project = create_project(None, None, None, None, None);
    project = create_folder(project, "bin".into(), "Bin".into(), None).unwrap();
    project.media.push(media("nested", Some("bin")));
    project.media.push(media("root", None));
    let mut controller = EditorController::new();
    let mut workspace = Workspace::with_project(project, "selection");
    let _ = controller.apply(
        &mut workspace,
        Intent::LibrarySelectFolder(Some("bin".into())),
    );
    let _ = controller.apply(&mut workspace, Intent::LibrarySelect { id: "nested".into(), additive: false, range: false });
    let view = controller.view(workspace.editor().unwrap(), &[]);
    assert_eq!(view.library.selected_id.as_deref(), Some("nested"));
    assert_eq!(view.library.selected_folder_id.as_deref(), Some("bin"));
    assert!(matches!(
        &view.library.explorer[0],
        crate::ExplorerNode::Folder {
            selected: false,
            expanded: true,
            ..
        }
    ));
    let _ = controller.apply(&mut workspace, Intent::LibrarySelect { id: "root".into(), additive: false, range: false });
    assert_eq!(
        controller
            .view(workspace.editor().unwrap(), &[])
            .library
            .selected_folder_id,
        None
    );
}

#[test]
fn creating_folder_selects_it_instead_of_previously_selected_media() {
    let mut project = create_project(None, None, None, None, None);
    project.media.push(media("root", None));
    let mut controller = EditorController::new();
    let mut workspace = Workspace::with_project(project, "new-folder");
    let _ = controller.apply(&mut workspace, Intent::LibrarySelect { id: "root".into(), additive: false, range: false });
    let _ = controller.apply(
        &mut workspace,
        Intent::LibraryCreateFolder { name: "Bin".into() },
    );
    let view = controller.view(workspace.editor().unwrap(), &[]);
    assert!(view.library.selected_id.is_none());
    assert!(view.library.selected_folder_id.is_some());
}

#[test]
fn library_focus_blocks_a_stale_timeline_delete_intent() {
    let mut editor = TestEditor::new(with_clip(), "delete-focus");
    editor.apply(Intent::SelectClip {
        clip_id: "c1".into(),
        additive: false,
    });
    editor.apply(Intent::LibrarySelect { id: "m1".into(), additive: false, range: false });
    let before = editor.project().clone();
    editor.apply(Intent::DeleteSelection { ripple: false });
    assert_eq!(editor.project(), &before);
}

#[test]
fn deleting_folder_promotes_children_and_preserves_source_paths_and_timeline() {
    let mut project = with_clip();
    project = create_folder(project, "bin".into(), "Bin".into(), None).unwrap();
    project = create_folder(project, "child".into(), "Child".into(), Some("bin".into())).unwrap();
    project.media[0].folder_id = Some("bin".into());
    let tracks = project.tracks.clone();
    let mut editor = TestEditor::new(project, "delete-bin");
    editor.apply(Intent::LibrarySelectFolder(Some("bin".into())));
    editor.apply(Intent::LibraryDeleteFolder { id: "bin".into() });
    assert_eq!(editor.project().folders.len(), 1);
    assert_eq!(editor.project().folders[0].id, "child");
    assert_eq!(editor.project().folders[0].parent_id, None);
    assert_eq!(editor.project().media[0].folder_id, None);
    assert_eq!(
        editor.project().media[0].path.as_deref(),
        Some("data/a.mp4")
    );
    assert_eq!(editor.project().tracks, tracks);
    assert_eq!(editor.view().library.selected_folder_id, None);
}

#[test]
fn non_cascading_library_delete_preserves_referenced_media_and_timeline() {
    let mut editor = TestEditor::new(with_clip(), "delete-used-media");
    editor.apply(Intent::LibrarySelect { id: "m1".into(), additive: false, range: false });
    let before = editor.project().clone();
    editor.apply(Intent::RemoveMedia {
        media_id: "m1".into(),
        cascade: false,
    });
    assert_eq!(editor.project(), &before);
}

#[test]
fn removing_unused_media_clears_the_deleted_row_selection() {
    let mut project = create_project(None, None, None, None, None);
    project.media.push(media("unused", None));
    let mut editor = TestEditor::new(project, "delete-media");
    editor.apply(Intent::LibrarySelect { id: "unused".into(), additive: false, range: false });
    editor.apply(Intent::RemoveMedia {
        media_id: "unused".into(),
        cascade: false,
    });
    assert!(editor.project().media.is_empty());
    assert!(editor.view().library.selected_id.is_none());
}
