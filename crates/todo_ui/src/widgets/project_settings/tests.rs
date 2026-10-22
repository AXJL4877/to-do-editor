use super::*;

fn frame(
    ctx: &egui::Context,
    vm: &EditorViewModel,
    events: Vec<egui::Event>,
) -> (Vec<Intent>, egui::FullOutput) {
    let mut intents = Vec::new();
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| intents = draw(ctx, vm),
    );
    (intents, output)
}

#[test]
fn settings_modal_lists_project_fields_and_can_be_dismissed() {
    let ctx = egui::Context::default();
    let vm = crate::test_view(
        todo_core::create_project(None, None, None, None, None),
        "test",
    );
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(OPEN_ID), true));
    frame(&ctx, &vm, vec![]);
    let (_, output) = frame(&ctx, &vm, vec![]);
    for label in [
        "工程设置",
        "分辨率",
        "帧率",
        "落轨缩放",
        "像素匹配",
        "工作区时长",
        "1080p",
        "完成",
    ] {
        assert!(
            output.shapes.iter().any(|shape| matches!(
                &shape.shape,
                egui::Shape::Text(text) if text.galley.text() == label
            )),
            "missing {label}"
        );
    }
    let done = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "完成" => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("完成");
    let pointer = |pressed| egui::Event::PointerButton {
        pos: done,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: Default::default(),
    };
    frame(
        &ctx,
        &vm,
        vec![egui::Event::PointerMoved(done), pointer(true)],
    );
    frame(&ctx, &vm, vec![pointer(false)]);
    assert!(!ctx
        .data(|data| data.get_temp::<bool>(egui::Id::new(OPEN_ID)))
        .unwrap_or(true));
}

#[test]
fn settings_modal_displays_rejected_project_setting_status() {
    let ctx = egui::Context::default();
    let mut vm = crate::test_view(
        todo_core::create_project(None, None, None, None, None),
        "test",
    );
    vm.status = Some("帧率更改失败：片段短于一帧".into());
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(OPEN_ID), true));
    frame(&ctx, &vm, vec![]);
    let (_, output) = frame(&ctx, &vm, vec![]);
    assert!(
        output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::Text(text) if text.galley.text() == vm.status.as_deref().unwrap()
        )),
        "setting validation feedback must stay visible inside the modal"
    );
}

#[test]
fn clicking_done_commits_a_live_typed_duration_edit() {
    let ctx = egui::Context::default();
    let vm = crate::test_view(
        todo_core::create_project(None, None, None, None, None),
        "test",
    );
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(OPEN_ID), true));
    frame(&ctx, &vm, vec![]);
    let (_, output) = frame(&ctx, &vm, vec![]);
    let text_pos = |predicate: &dyn Fn(&str) -> bool| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if predicate(text.galley.text()) => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("control text")
    };
    let duration = text_pos(&|text| text.ends_with(" 秒"));
    let done = text_pos(&|text| text == "完成");
    let pointer = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: Default::default(),
    };
    frame(
        &ctx,
        &vm,
        vec![egui::Event::PointerMoved(duration), pointer(duration, true)],
    );
    frame(&ctx, &vm, vec![pointer(duration, false)]);
    let (mut intents, _) = frame(&ctx, &vm, vec![egui::Event::Text("120".into())]);
    assert!(
        intents.iter().any(|intent| matches!(intent,
            Intent::SetProjectDuration { seconds } if *seconds == 120.0
        )),
        "typing changes the project immediately: {intents:?}"
    );
    assert!(!intents
        .iter()
        .any(|intent| matches!(intent, Intent::CommitEdit)));
    intents.extend(
        frame(
            &ctx,
            &vm,
            vec![egui::Event::PointerMoved(done), pointer(done, true)],
        )
        .0,
    );
    intents.extend(frame(&ctx, &vm, vec![pointer(done, false)]).0);
    assert!(!is_open(&ctx));
    assert_eq!(
        intents
            .iter()
            .filter(|intent| matches!(intent, Intent::BeginEdit))
            .count(),
        1
    );
    assert!(
        intents
            .iter()
            .any(|intent| matches!(intent, Intent::CommitEdit)),
        "Done must finish the edit for undo/save: {intents:?}"
    );
    assert!(
        intents.iter().any(|intent| matches!(intent, Intent::Save)),
        "Done must persist project duration/settings: {intents:?}"
    );
}
