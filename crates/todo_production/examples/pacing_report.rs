//! Compare board pacing of old productions (as rendered) with the new schedule.
//! `cargo run -p todo_production --example pacing_report -- <production.json>...`
//! Writing time per element is the old renderer's, clamped into the new window.
use todo_production::pacing::{schedule, Pacing};
use todo_production::storyboard::{BoardPlan, Phrase, PlannedBoard, PlannedElement, Role};

#[derive(Default)]
struct Metrics {
    write_fraction: Vec<f64>,
    hold: Vec<f64>,
    first_stroke: Vec<f64>,
    ahead: Vec<f64>,
}

fn quantiles(name: &str, mut values: Vec<f64>) {
    if values.is_empty() {
        println!("  {name:<28} no samples");
        return;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |p: f64| values[((values.len() - 1) as f64 * p).round() as usize];
    println!(
        "  {name:<28} p10 {:>5.2}  p50 {:>5.2}  p90 {:>5.2}  (n={})",
        q(0.1),
        q(0.5),
        q(0.9),
        values.len()
    );
}

fn main() {
    let mut old = Metrics::default();
    let mut new = Metrics::default();
    let mut pacing = Pacing::default();
    if let Ok(floor) = std::env::var("FLOOR_HOLD") {
        pacing.floor_hold = floor.parse().unwrap();
    }
    for path in std::env::args().skip(1) {
        let doc: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let segments = doc["segments"].as_array().cloned().unwrap_or_default();
        let count = segments.len();
        for (index, segment) in segments.iter().enumerate() {
            let Some(voice) = segment.get("voice").filter(|v| v.is_object()) else {
                continue;
            };
            let phrases: Vec<Phrase> =
                serde_json::from_value(voice["phrases"].clone()).unwrap_or_default();
            if phrases.is_empty() {
                continue;
            }
            let boards = segment["boards"].as_array().cloned().unwrap_or_default();
            let mut natural = Vec::new();
            let mut plan = BoardPlan {
                title: String::new(),
                notes: String::new(),
                scenes: Vec::new(),
            };
            for board in &boards {
                let Some(result) = board["render"]["result"].as_object() else {
                    continue;
                };
                let rendered = result["elements"].as_array().unwrap();
                // Old metrics, from what was actually rendered.
                let (start, end) = (
                    result["startMs"].as_f64().unwrap() / 1e3,
                    result["endMs"].as_f64().unwrap() / 1e3,
                );
                let written: f64 = rendered
                    .iter()
                    .map(|e| e["durationMs"].as_f64().unwrap() / 1e3)
                    .sum();
                let finish = rendered
                    .iter()
                    .map(|e| {
                        (e["startMs"].as_f64().unwrap() + e["durationMs"].as_f64().unwrap()) / 1e3
                    })
                    .fold(0.0, f64::max);
                let first = rendered
                    .iter()
                    .map(|e| e["startMs"].as_f64().unwrap() / 1e3)
                    .fold(f64::MAX, f64::min);
                old.write_fraction.push(written / (end - start));
                old.hold.push(end - finish);
                old.first_stroke.push(first - start);
                natural.push(
                    rendered
                        .iter()
                        .map(|e| e["durationMs"].as_f64().unwrap() / 1e3)
                        .collect::<Vec<_>>(),
                );
                plan.scenes.push(PlannedBoard {
                    start: board["start"].as_u64().unwrap() as usize,
                    elements: board["elements"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|e| PlannedElement {
                            at: e["at"].as_u64().unwrap_or(0) as usize,
                            label: String::new(),
                            draw: String::new(),
                            role: Role::Hero,
                            weight: 1.0,
                        })
                        .collect(),
                });
            }
            if plan.scenes.is_empty() || plan.scenes.len() != boards.len() {
                continue;
            }
            let duration = voice["duration"].as_f64().unwrap();
            let tail = if index + 1 == count {
                doc["params"]["tail"].as_f64().unwrap_or(0.0)
            } else {
                0.0
            };
            let timed = schedule(&plan, &phrases, duration, tail, &pacing);
            for (k, board) in timed.iter().enumerate() {
                let leave =
                    timed.get(k + 1).map_or(board.end_ms, |next| next.start_ms) as f64 / 1e3;
                let start = board.start_ms as f64 / 1e3;
                let mut written = 0.0;
                let mut finish: f64 = 0.0;
                for (e, nat) in board.elements.iter().zip(&natural[k]) {
                    let window = (e.window_end_ms - e.at_ms) as f64 / 1e3;
                    let write = nat.min(window);
                    written += write;
                    finish = finish.max(e.at_ms as f64 / 1e3 + write);
                }
                new.write_fraction.push(written / (leave - start));
                new.hold.push(leave - finish);
                new.first_stroke
                    .push(board.elements[0].at_ms as f64 / 1e3 - start);
                for (e, planned) in board.elements.iter().zip(&plan.scenes[k].elements) {
                    let wanted =
                        phrases[planned.at.min(phrases.len() - 1)].start - pacing.anticipation;
                    new.ahead.push((wanted - e.at_ms as f64 / 1e3).max(0.0));
                }
            }
        }
    }
    for (name, m) in [("old (rendered)", old), ("new schedule", new)] {
        println!("{name}");
        quantiles("writing share of board", m.write_fraction);
        quantiles("stillness after writing, s", m.hold);
        quantiles("board shown → first stroke, s", m.first_stroke);
        if !m.ahead.is_empty() {
            quantiles("stroke ahead of its word, s", m.ahead);
        }
    }
}
