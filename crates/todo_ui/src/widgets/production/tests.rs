use super::*;
use crate::widgets::agent::AgentView;

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
    agent: &mut AgentView,
    events: Vec<egui::Event>,
) -> (AgentAction, egui::FullOutput) {
    let mut action = AgentAction::None;
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
                action = draw_dock(ui, view, agent, &mut crate::ThumbCache::default());
            });
        },
    );
    (action, output)
}

fn find(output: &egui::FullOutput, label: &str) -> egui::Rect {
    texts(output)
        .into_iter()
        .find(|(text, _)| text == label || text.contains(label))
        .unwrap_or_else(|| panic!("missing {label}: {:?}", texts(output)))
        .1
}

#[test]
fn the_dock_is_an_agent_chat_not_a_workflow_list() {
    let ctx = egui::Context::default();
    let view = ProductionPanelView {
        project_open: true,
        ..Default::default()
    };
    let mut agent = AgentView::default();
    frame(&ctx, &view, &mut agent, vec![]);
    let (_, output) = frame(&ctx, &view, &mut agent, vec![]);
    find(&output, "Agent");
    find(&output, "发送");
    let all: String = texts(&output).into_iter().map(|(t, _)| t).collect();
    assert!(
        !all.contains("新建工作流"),
        "workflow list must be gone: {all}"
    );
    assert!(
        !all.contains("海绵宝宝口播"),
        "workflow packages must not appear in the dock: {all}"
    );
}

#[test]
fn workflow_overlay_never_opens() {
    let ctx = egui::Context::default();
    // Legacy open() only remembers selection; fullscreen graph is gone.
    open_workflow(&ctx, "studio.demo".into());
    assert!(
        !workflow_fullscreen_open(&ctx),
        "open_workflow must not open a graph UI anymore"
    );
    let view = ProductionPanelView::default();
    let action = draw_workflow_overlay(
        &ctx,
        &view,
        None,
        "demo",
        &mut crate::ThumbCache::default(),
    );
    assert_eq!(action, ProductionAction::None);
    assert!(!workflow_fullscreen_open(&ctx));
}
