use todo_core::{
    create_folder, create_project, delete_folder, execute_edit, move_folder, rename_folder,
    validate_project, DomainError, EditCommand, MediaItem, MediaKind,
};

fn media(id: &str, folder_id: Option<&str>) -> MediaItem {
    MediaItem {
        id: id.into(),
        name: format!("{id}.mp4"),
        path: None,
        duration: 5.0,
        has_audio: Some(true),
        width: Some(64),
        height: Some(64),
        kind: Some(MediaKind::Video),
        folder_id: folder_id.map(str::to_owned),
    }
}

#[test]
fn legacy_json_without_folders_still_validates() {
    let json = r#"{
        "version": 1,
        "fps": 30.0,
        "playhead": 0.0,
        "media": [],
        "tracks": []
    }"#;
    let project: todo_core::Project = serde_json::from_str(json).unwrap();
    assert!(project.folders.is_empty());
    validate_project(&project).unwrap();
}

#[test]
fn create_move_delete_folder_promotes_children() {
    let mut project = create_project(None, None, None, None, None);
    project = create_folder(project, "a".into(), "A".into(), None).unwrap();
    project = create_folder(project, "b".into(), "B".into(), Some("a".into())).unwrap();
    project.media.push(media("m1", Some("b")));
    project = delete_folder(project, "b").unwrap();
    assert!(project.folders.iter().all(|folder| folder.id != "b"));
    assert_eq!(project.media[0].folder_id.as_deref(), Some("a"));
    project = delete_folder(project, "a").unwrap();
    assert!(project.folders.is_empty());
    assert_eq!(project.media[0].folder_id, None);
}

#[test]
fn moving_folder_into_descendant_is_rejected() {
    let mut project = create_project(None, None, None, None, None);
    project = create_folder(project, "a".into(), "A".into(), None).unwrap();
    project = create_folder(project, "b".into(), "B".into(), Some("a".into())).unwrap();
    let err = move_folder(project, "a", Some("b".into())).unwrap_err();
    assert!(matches!(err, DomainError::FolderCycle(_)));
}

#[test]
fn edit_commands_move_media_between_folders() {
    let mut project = create_project(None, None, None, None, None);
    project = create_folder(project, "shots".into(), "镜头".into(), None).unwrap();
    project.media.push(media("m1", None));
    project = execute_edit(
        project,
        EditCommand::MoveMedia {
            media_id: "m1".into(),
            folder_id: Some("shots".into()),
        },
    )
    .unwrap();
    assert_eq!(project.media[0].folder_id.as_deref(), Some("shots"));
    project = rename_folder(project, "shots", "分镜".into()).unwrap();
    assert_eq!(project.folders[0].name, "分镜");
}
