use super::*;
use todo_core::{MediaFolder, MediaItem, Project};
use todo_interaction::ExplorerNode;

fn project() -> Project {
    Project {
        version: 1,
        fps: 30.0,
        playhead: 0.0,
        width: 1920,
        height: 1080,
        duration: None,
        media_scale: Default::default(),
        tracks: Vec::new(),
        text_styles: Vec::new(),
        motions: Vec::new(),
        media: vec![MediaItem {
            id: "jpg1".into(),
            name: "shot.jpg".into(),
            path: Some("data/shot.jpg".into()),
            duration: 5.0,
            has_audio: Some(false),
            width: Some(100),
            height: Some(100),
            kind: Some(todo_core::MediaKind::Image),
            folder_id: None,
            alpha: None,
            reveal: None,
        }],
        folders: vec![MediaFolder {
            id: "bin1".into(),
            name: "新建料箱".into(),
            parent_id: None,
        }],
    }
}

#[test]
fn explorer_lists_bins_and_media_in_one_tree() {
    let vm = crate::test_view(project(), "test");
    assert!(
        vm.library.explorer.len() >= 2,
        "expected folder + media at root, got {}",
        vm.library.explorer.len()
    );
    assert!(vm
        .library
        .explorer
        .iter()
        .any(|n| matches!(n, ExplorerNode::Folder { .. })));
    assert!(vm.library.explorer.iter().any(|n| matches!(
        n,
        ExplorerNode::Media { item, .. } if item.item.id == "jpg1"
    )));
}

#[test]
fn primary_click_selects_media_without_removing() {
    let vm = crate::test_view(project(), "test");
    let ctx = egui::Context::default();
    let mut pointer = egui::pos2(60.0, 80.0);
    let mut intents = Vec::new();
    for step in 0..2 {
        let events = match step {
            1 => vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
            _ => Vec::new(),
        };
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 480.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    intents = draw(ui, &vm, &mut ThumbCache::default());
                });
            },
        );
        if step == 0 {
            pointer = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "shot.jpg" => {
                        Some(text.pos + egui::vec2(6.0, 4.0))
                    }
                    _ => None,
                })
                .expect("jpg row");
        }
    }
    assert!(intents
        .iter()
        .any(|i| matches!(i, Intent::LibrarySelect(id) if id == "jpg1")));
    assert!(!intents
        .iter()
        .any(|i| matches!(i, Intent::RemoveMedia { .. })));
    assert!(!intents
        .iter()
        .any(|i| matches!(i, Intent::LibrarySelectFolder(None))));
}

#[test]
fn media_row_drag_carries_the_identifier() {
    let vm = crate::test_view(project(), "test");
    let ctx = egui::Context::default();
    let mut pointer = egui::pos2(40.0, 80.0);
    for step in 0..3 {
        let events = match step {
            1 => vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
            2 => vec![egui::Event::PointerMoved(pointer + egui::vec2(16.0, 0.0))],
            _ => Vec::new(),
        };
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 480.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    draw(ui, &vm, &mut ThumbCache::default());
                });
            },
        );
        if step == 0 {
            pointer = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "shot.jpg" => {
                        Some(text.pos + egui::vec2(4.0, 4.0))
                    }
                    _ => None,
                })
                .expect("media row");
        }
    }
    let payload = egui::DragAndDrop::payload::<MediaDrag>(&ctx).expect("drag");
    assert_eq!(payload.media_id, "jpg1");
}

#[test]
fn dragging_media_over_a_folder_shows_the_ghost_destination_row() {
    let vm = crate::test_view(project(), "test");
    let ctx = egui::Context::default();
    let first = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 480.0),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                draw(ui, &vm, &mut ThumbCache::default());
            });
        },
    );
    let folder_pos = first
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "新建料箱" => {
                Some(text.pos + egui::vec2(6.0, 4.0))
            }
            _ => None,
        })
        .expect("folder row");

    egui::DragAndDrop::set_payload(
        &ctx,
        MediaDrag {
            media_id: "jpg1".into(),
        },
    );
    let second = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 480.0),
            )),
            events: vec![egui::Event::PointerMoved(folder_pos)],
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                draw(ui, &vm, &mut ThumbCache::default());
            });
        },
    );
    egui::DragAndDrop::clear_payload(&ctx);

    let shot_rows = second.shapes.iter().filter(|shape| {
        matches!(
            &shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "shot.jpg"
        )
    });
    assert_eq!(shot_rows.count(), 2, "folder should show a ghost media row");
    assert!(second.shapes.iter().any(|shape| matches!(
        &shape.shape,
        egui::Shape::Text(text) if text.galley.text() == "预览"
    )));
}

#[test]
fn folder_arrow_selects_the_folder_and_toggles_its_children() {
    let vm = crate::test_view(project(), "test");
    let ctx = egui::Context::default();
    let mut pointer = egui::Pos2::ZERO;
    let mut intents = Vec::new();
    for step in 0..2 {
        let events = if step == 1 {
            vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ]
        } else {
            Vec::new()
        };
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 480.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    intents = draw(ui, &vm, &mut ThumbCache::default());
                });
            },
        );
        if step == 0 {
            pointer = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "新建料箱" => {
                        Some(text.pos + egui::vec2(-27.0, 5.0))
                    }
                    _ => None,
                })
                .expect("folder row");
        }
    }
    assert!(intents.iter().any(|intent| matches!(
        intent,
        Intent::LibrarySelectFolder(Some(id)) if id == "bin1"
    )));
    assert!(intents.iter().any(|intent| matches!(
        intent,
        Intent::LibraryToggleFolder(id) if id == "bin1"
    )));
    assert!(!intents
        .iter()
        .any(|intent| matches!(intent, Intent::LibrarySelectFolder(None))));
}

#[test]
fn clicking_empty_library_space_focuses_library_and_selects_root() {
    let vm = crate::test_view(project(), "test");
    let ctx = egui::Context::default();
    let mut intents = Vec::new();
    for step in 0..2 {
        let point = egui::pos2(100.0, 300.0);
        let events = if step == 1 {
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ]
        } else {
            Vec::new()
        };
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 480.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    intents = draw(ui, &vm, &mut ThumbCache::default());
                });
            },
        );
    }
    assert!(intents
        .iter()
        .any(|intent| matches!(intent, Intent::Focus(EditorFocus::Library))));
    assert!(intents
        .iter()
        .any(|intent| matches!(intent, Intent::LibrarySelectFolder(None))));
}
