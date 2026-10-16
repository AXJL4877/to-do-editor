//! Exercise the actual UI → interaction → UI path, including egui scroll state.
use super::*;
use todo_application::Workspace;
use todo_interaction::{time_at, EditorController};

struct Board {
    ctx: egui::Context,
    workspace: Workspace,
    controller: EditorController,
    frame: u32,
}

impl Board {
    fn new(duration: f64) -> Self {
        let mut board = Self {
            ctx: egui::Context::default(),
            workspace: Workspace::with_project(
                todo_core::create_project(None, None, Some(duration), None, None),
                "zoom",
            ),
            controller: EditorController::new(),
            frame: 0,
        };
        board.render(Vec::new(), Default::default());
        board.render(Vec::new(), Default::default());
        board
    }

    fn view(&self) -> TimelineViewModel {
        self.controller
            .view(self.workspace.editor().unwrap())
            .timeline
    }

    fn render(&mut self, events: Vec<egui::Event>, modifiers: egui::Modifiers) {
        self.frame += 1;
        let vm = self.view();
        let mut intents = Vec::new();
        let _ = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 400.0),
                )),
                time: Some(f64::from(self.frame) / 60.0),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    intents = draw(ui, &vm, &PeaksCache::default());
                });
            },
        );
        for intent in intents {
            let _ = self.controller.apply(&mut self.workspace, intent);
        }
    }

    fn wheel(&mut self, pointer: egui::Pos2, delta: egui::Vec2, modifiers: egui::Modifiers) {
        self.render(
            vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta,
                    modifiers,
                },
            ],
            modifiers,
        );
    }

    fn time_under(&self, pointer: egui::Pos2) -> f64 {
        let vm = self.view();
        // The CentralPanel adds an eight-point margin around the real board.
        time_at(f64::from(pointer.x - 8.0), vm.scroll_left, vm.px_per_sec)
    }
}

#[test]
fn wheel_zoom_keeps_the_mouse_time_when_the_next_frame_applies_real_scroll_state() {
    for modifiers in [
        egui::Modifiers {
            alt: true,
            ..Default::default()
        },
        egui::Modifiers {
            ctrl: true,
            ..Default::default()
        },
    ] {
        let mut board = Board::new(300.0);
        let pointer = egui::pos2(690.0, 90.0);
        for _ in 0..4 {
            let before = board.time_under(pointer);
            board.wheel(pointer, egui::vec2(0.0, 40.0), modifiers);
            board.render(Vec::new(), modifiers);
            let after = board.time_under(pointer);
            assert!(
                (before - after).abs() < 0.001,
                "pointer time moved: {before} → {after}"
            );
        }
        for _ in 0..3 {
            let before = board.time_under(pointer);
            board.wheel(pointer, egui::vec2(0.0, -40.0), modifiers);
            board.render(Vec::new(), modifiers);
            assert!((before - board.time_under(pointer)).abs() < 0.001);
        }
    }
}

#[test]
fn long_workspaces_apply_small_density_changes_while_holding_the_mouse_time() {
    let mut board = Board::new(86_400.0);
    let pointer = egui::pos2(610.0, 90.0);
    let modifiers = egui::Modifiers {
        alt: true,
        ..Default::default()
    };
    let before = board.time_under(pointer);
    board.wheel(pointer, egui::vec2(0.0, 40.0), modifiers);
    board.render(Vec::new(), modifiers);
    assert!(
        (before - board.time_under(pointer)).abs() < 0.05,
        "a small density change must still apply its anchored scroll offset"
    );
}

#[test]
fn ordinary_horizontal_scroll_then_zoom_preserves_the_scrolled_mouse_anchor() {
    let mut board = Board::new(300.0);
    let pointer = egui::pos2(540.0, 90.0);
    let alt = egui::Modifiers {
        alt: true,
        ..Default::default()
    };
    for _ in 0..14 {
        board.wheel(pointer, egui::vec2(0.0, 40.0), alt);
        board.render(Vec::new(), alt);
    }
    let previous = board.view().scroll_left;
    board.wheel(pointer, egui::vec2(-160.0, 0.0), Default::default());
    for _ in 0..30 {
        board.render(Vec::new(), Default::default());
    }
    assert!(
        board.view().scroll_left > previous + 100.0,
        "plain wheel must still scroll"
    );
    let before = board.time_under(pointer);
    board.wheel(pointer, egui::vec2(0.0, 40.0), alt);
    board.render(Vec::new(), alt);
    assert!((before - board.time_under(pointer)).abs() < 0.001);
}
