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
        .find(|(text, _)| text == label)
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
    for label in ["工作流", "新建工作流", "海绵宝宝口播", "内置"] {
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
}
