//! Breathing rules of the board schedule, checked on every plan of the
//! storyboard fixture (real narration texts, model-like plans).
use serde_json::Value;
use todo_production::pacing::{schedule, Pacing, TimedBoard};
use todo_production::storyboard::{BoardPlan, Phrase};

fn cases() -> Vec<(BoardPlan, Vec<Phrase>, f64)> {
    let golden: Value =
        serde_json::from_slice(include_bytes!("fixtures/production_golden.json")).unwrap();
    golden["planCases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c.get("plan").is_some())
        .map(|c| {
            (
                serde_json::from_value(c["plan"].clone()).unwrap(),
                serde_json::from_value(c["phrases"].clone()).unwrap(),
                c["duration"].as_f64().unwrap(),
            )
        })
        .collect()
}

fn s(ms: i64) -> f64 {
    ms as f64 / 1000.0
}

fn check(
    plan: &BoardPlan,
    phrases: &[Phrase],
    duration: f64,
    tail: f64,
    p: &Pacing,
) -> Vec<TimedBoard> {
    let timed = schedule(plan, phrases, duration, tail, p);
    assert_eq!(timed.len(), plan.scenes.len());
    assert_eq!(timed[0].start_ms, 0);
    assert_eq!(timed[0].transition_ms, 0);
    assert_eq!(
        timed.last().unwrap().end_ms,
        ((duration + tail) * 1000.0).round() as i64
    );
    for (k, board) in timed.iter().enumerate() {
        let leave = timed.get(k + 1).map_or(board.end_ms, |next| next.start_ms);
        if let Some(next) = timed.get(k + 1) {
            // Cross-fade: the old board stays until the new one has faded in.
            assert_eq!(
                board.end_ms,
                next.start_ms + next.transition_ms,
                "board {k}"
            );
            assert!(next.start_ms >= board.start_ms + (p.transition * 1000.0) as i64 - 1);
            assert_eq!(next.transition_ms, (p.transition * 1000.0) as i64);
        }
        let length = s(leave - board.start_ms);
        let ready = if k == 0 {
            0.0
        } else {
            s(board.start_ms) + p.transition + p.settle
        };
        let floor = p.floor_hold.min(length * 0.2);
        let mut previous: Option<i64> = None;
        for e in &board.elements {
            assert!(e.at_ms >= board.start_ms, "stroke before its board");
            if s(leave) - ready > floor + p.min_write + 0.5 {
                assert!(
                    s(e.at_ms) + 1e-3 >= ready.min(s(leave) - floor - p.min_write),
                    "no settle: board {k}"
                );
            }
            assert!(e.window_end_ms >= e.at_ms + 300, "window too short");
            if let Some(prev) = previous {
                assert!(e.at_ms >= prev, "strokes out of order");
            }
            previous = Some(e.at_ms);
        }
        let last = board.elements.last().unwrap();
        // The board stays still at the end.
        assert!(
            s(leave - last.window_end_ms) + 2e-3 >= floor.min(s(leave - last.at_ms) - 0.3).max(0.0),
            "board {k}: hold"
        );
    }
    timed
}

#[test]
fn every_fixture_plan_breathes() {
    let cases = cases();
    assert!(cases.len() > 40);
    let pacing = Pacing::default();
    for (plan, phrases, duration) in &cases {
        check(plan, phrases, *duration, 0.0, &pacing);
        check(plan, phrases, *duration, 1.5, &pacing);
    }
}

#[test]
fn boards_change_inside_pauses() {
    let pacing = Pacing::default();
    for (plan, phrases, duration) in cases() {
        let timed = schedule(&plan, &phrases, duration, 0.0, &pacing);
        for (k, board) in plan.scenes.iter().enumerate().skip(1) {
            let word = phrases[board.start].start;
            let pause_start = phrases[board.start - 1].end;
            let appear = s(timed[k].start_ms);
            // Shown before the sentence is spoken, never before the previous phrase ends
            // unless the pause is shorter than the minimum lead.
            if appear > s(timed[k - 1].start_ms) + pacing.transition + 1e-3 {
                assert!(appear <= word - pacing.min_lead + 1e-3, "late board");
                assert!(
                    appear + 1e-3 >= pause_start.min(word - pacing.min_lead),
                    "board cuts into speech"
                );
            }
        }
    }
}
