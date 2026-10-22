use super::*;
use todo_interaction::{ExtensionStatus, WorkflowEntryView};

fn entry(status: ExtensionStatus) -> WorkflowEntryView {
    WorkflowEntryView {
        extension_id: "studio.spongebob-koubo".into(),
        name: "海绵宝宝口播".into(),
        summary: "工作流扩展：口播一句一个镜头".into(),
        origin_user: false,
        status,
    }
}

fn texts(output: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            _ => None,
        })
        .collect()
}

fn frame(
    ctx: &egui::Context,
    view: &ProductionPanelView,
    events: Vec<egui::Event>,
) -> (ProductionAction, egui::FullOutput) {
    let mut action = ProductionAction::None;
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(420.0, 1000.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                action = draw_dock(ui, view, &mut crate::ThumbCache::default());
            });
        },
    );
    (action, output)
}

fn click(ctx: &egui::Context, view: &ProductionPanelView, at: egui::Pos2) -> ProductionAction {
    frame(ctx, view, vec![egui::Event::PointerMoved(at)]);
    frame(
        ctx,
        view,
        vec![egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }],
    );
    frame(
        ctx,
        view,
        vec![egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }],
    )
    .0
}

fn find(output: &egui::FullOutput, label: &str) -> egui::Rect {
    texts(output)
        .into_iter()
        .find(|(text, _)| text == label || text.contains(label))
        .unwrap_or_else(|| panic!("missing {label}: {:?}", texts(output)))
        .1
}

#[test]
fn the_dock_lists_workflows_and_can_create() {
    let ctx = egui::Context::default();
    let view = ProductionPanelView {
        workflows: vec![entry(ExtensionStatus::Ready)],
        project_open: true,
        ..Default::default()
    };
    frame(&ctx, &view, vec![]);
    let (_, output) = frame(&ctx, &view, vec![]);
    for label in ["工作流", "新建工作流", "海绵宝宝口播"] {
        find(&output, label);
    }
    let all: String = texts(&output).into_iter().map(|(t, _)| t).collect();
    assert!(!all.contains("studio."), "internal ids leak: {all}");

    let create = find(&output, "新建工作流").center();
    assert_eq!(click(&ctx, &view, create), ProductionAction::WorkflowCreate);
}

#[test]
fn clicking_a_workflow_opens_fullscreen() {
    let ctx = egui::Context::default();
    let view = ProductionPanelView {
        workflows: vec![entry(ExtensionStatus::Ready)],
        project_open: true,
        ..Default::default()
    };
    frame(&ctx, &view, vec![]);
    let (_, output) = frame(&ctx, &view, vec![]);
    let row = find(&output, "海绵宝宝口播").center();
    click(&ctx, &view, row);
    assert_eq!(
        workflow_open_id(&ctx).as_deref(),
        Some("studio.spongebob-koubo")
    );
    // Survives the next frame (must not be one-shot temp that evaporates).
    frame(&ctx, &view, vec![]);
    assert_eq!(
        workflow_open_id(&ctx).as_deref(),
        Some("studio.spongebob-koubo")
    );
}

#[test]
fn user_workflow_row_can_delete() {
    let ctx = egui::Context::default();
    let view = ProductionPanelView {
        workflows: vec![WorkflowEntryView {
            extension_id: "workflow-demo".into(),
            name: "演示流".into(),
            summary: String::new(),
            origin_user: true,
            status: ExtensionStatus::Ready,
        }],
        project_open: true,
        ..Default::default()
    };
    frame(&ctx, &view, vec![]);
    let (_, output) = frame(&ctx, &view, vec![]);
    let all: String = texts(&output).into_iter().map(|(t, _)| t).collect();
    assert!(all.contains("删除"), "user rows expose delete: {all}");
    let del = find(&output, "删除").center();
    assert_eq!(
        click(&ctx, &view, del),
        ProductionAction::WorkflowDelete {
            extension: "workflow-demo".into(),
        }
    );
}

#[test]
fn builtin_workflow_row_has_no_delete() {
    let ctx = egui::Context::default();
    let view = ProductionPanelView {
        workflows: vec![entry(ExtensionStatus::Ready)],
        project_open: true,
        ..Default::default()
    };
    frame(&ctx, &view, vec![]);
    let (_, output) = frame(&ctx, &view, vec![]);
    let all: String = texts(&output).into_iter().map(|(t, _)| t).collect();
    assert!(!all.contains("删除"), "builtin must not delete: {all}");
}

fn overlay_frame(ctx: &egui::Context, available: bool) -> egui::FullOutput {
    overlay_input_frame(ctx, available, Vec::new())
}

fn overlay_input_frame(ctx: &egui::Context, available: bool, events: Vec<egui::Event>) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            if available {
                workflow::draw_fullscreen(
                    ctx,
                    "test",
                    "Test workflow",
                    &Default::default(),
                    &mut crate::thumbs::ThumbCache::default(),
                );
            } else {
                workflow::draw_unavailable(ctx, "test", "Test workflow");
            }
        },
    )
}

#[test]
fn fullscreen_controls_and_terminal_stay_inside_the_viewport() {
    let ctx = egui::Context::default();
    workflow::open(&ctx, "test".into());
    overlay_frame(&ctx, true);
    let output = overlay_frame(&ctx, true);
    let screen = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
    assert!(screen.contains_rect(find(&output, "Test workflow")));
    assert!(screen.contains_rect(find(&output, "关闭")));
    let terminal = workflow::terminal_hole(&ctx).unwrap();
    assert!(terminal.is_positive() && screen.contains_rect(terminal), "{terminal:?}");
}

#[test]
fn unavailable_workflow_keeps_its_close_button_on_screen() {
    let ctx = egui::Context::default();
    overlay_frame(&ctx, false);
    let output = overlay_frame(&ctx, false);
    let screen = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
    assert!(screen.contains_rect(find(&output, "关闭")));
}

#[test]
fn fullscreen_background_does_not_intercept_the_close_button() {
    for available in [false, true] {
        let ctx = egui::Context::default();
        workflow::open(&ctx, "test".into());
        overlay_frame(&ctx, available);
        let output = overlay_frame(&ctx, available);
        let pos = find(&output, "关闭").center();
        overlay_input_frame(&ctx, available, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            overlay_input_frame(&ctx, available, vec![egui::Event::PointerButton {
                pos, button: egui::PointerButton::Primary, pressed,
                modifiers: Default::default(),
            }]);
        }
        assert!(!workflow::is_open(&ctx), "close button did not work: available={available}");
    }
}

#[test]
fn backtick_and_shift_backtick_toggle_workflow_and_escape_closes_it() {
    for shift in [false, true] {
        let ctx = egui::Context::default();
        let panel = ProductionPanelView {
            workflows: vec![entry(ExtensionStatus::Ready)],
            ..Default::default()
        };
        for (key, pressed, open) in [
            (egui::Key::Backtick, true, true),
            (egui::Key::Backtick, false, true),
            (egui::Key::Backtick, true, false),
            (egui::Key::Backtick, false, false),
            (egui::Key::Backtick, true, true),
            (egui::Key::Escape, true, false),
        ] {
            let _ = ctx.run(egui::RawInput {
                events: vec![egui::Event::Key {
                    key, physical_key: Some(key), pressed, repeat: false,
                    modifiers: egui::Modifiers { shift, ..Default::default() },
                }],
                ..Default::default()
            }, |ctx| workflow::handle_hotkeys(ctx, &panel));
            assert_eq!(workflow::is_open(&ctx), open, "{key:?}, shift={shift}");
        }
    }
}
