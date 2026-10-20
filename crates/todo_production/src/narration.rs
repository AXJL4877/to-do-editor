//! The narration recipe's timeline: absolute board timing (with crossfades
//! inside voice pauses and across segments), bake requests for the extension
//! worker, and the items the host places (voice, subtitles, plate, baked
//! layers). Pure: the application decides when to run what.
//!
//! Keys use document ids, so editing one segment never renames another
//! segment's clips: `<segment>/voice`, `<segment>/sub/<n>`, `plate`,
//! `<board>/<element>` (and extra baked layers below it).
use crate::extension::items::{Intro, Item, ItemFade, ItemRole, ItemSource, ItemTransform};
use crate::extension::protocol::Canvas;
use crate::pacing::{schedule, TimedBoard};
use crate::production::{Production, Segment};
use crate::storyboard::{BoardPlan, PlannedBoard, PlannedElement};
use crate::tasks::fingerprint;
use crate::work::ProductionSpec;
use serde_json::{json, Value};

pub const BAKE_SPEC: &str = "todo-bake-board/1";
/// Element lanes per board; consecutive boards alternate banks so their
/// crossfade never puts two clips on one lane.
pub const LANES_PER_BANK: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NarrationLayout {
    /// Silence between segments; the next board crossfades in during it.
    pub segment_gap: f64,
}

impl Default for NarrationLayout {
    fn default() -> Self {
        Self { segment_gap: 0.4 }
    }
}

/// One element's writing window, seconds from the board start.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementTiming {
    pub at: f64,
    pub window_end: f64,
}

/// Absolute timing of a board, seconds from the production origin.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardTiming {
    pub segment: usize,
    pub board: usize,
    /// Global board number, for lane banks.
    pub ordinal: usize,
    pub start: f64,
    pub end: f64,
    pub fade_in: f64,
    pub fade_out: f64,
    pub elements: Vec<ElementTiming>,
}

/// Voice start of every segment.
pub fn segment_starts(doc: &Production, layout: &NarrationLayout) -> Vec<f64> {
    let mut starts = Vec::with_capacity(doc.segments.len());
    let mut cursor = 0.0;
    for segment in &doc.segments {
        starts.push(cursor);
        if let Some(voice) = &segment.voice {
            cursor += voice.timeline.duration + layout.segment_gap.max(0.0);
        }
    }
    starts
}

/// End of the last voice plus the tail.
pub fn total_duration(doc: &Production, spec: &ProductionSpec, layout: &NarrationLayout) -> f64 {
    let starts = segment_starts(doc, layout);
    doc.segments
        .iter()
        .zip(&starts)
        .filter_map(|(s, start)| s.voice.as_ref().map(|v| start + v.timeline.duration))
        .fold(0.0, f64::max)
        + spec.tail.max(0.0)
}

fn plan_of(segment: &Segment) -> BoardPlan {
    BoardPlan {
        title: String::new(),
        notes: String::new(),
        scenes: segment
            .boards
            .iter()
            .map(|board| PlannedBoard {
                start: board.start,
                elements: board
                    .elements
                    .iter()
                    .map(|e| PlannedElement {
                        at: e.at,
                        label: e.label.clone(),
                        draw: e.draw.clone(),
                        role: e.role,
                        weight: e.weight,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn timed(segment: &Segment, spec: &ProductionSpec, tail: f64) -> Option<Vec<TimedBoard>> {
    let voice = segment.voice.as_ref()?;
    let phrases = voice.timeline.phrases();
    if phrases.is_empty() || segment.boards.is_empty() {
        return None;
    }
    let boards = schedule(
        &plan_of(segment),
        &phrases,
        voice.timeline.duration,
        tail,
        &spec.pacing,
    );
    (boards.len() == segment.boards.len()).then_some(boards)
}

/// Every board of every voiced segment, in order. Within a segment the
/// pacing schedule decides; between segments the next segment's first board
/// crossfades in during the gap, so the picture never cuts hard.
pub fn board_timings(
    doc: &Production,
    spec: &ProductionSpec,
    layout: &NarrationLayout,
) -> Vec<BoardTiming> {
    let starts = segment_starts(doc, layout);
    let last = doc.segments.iter().rposition(|s| s.voice.is_some());
    let mut out: Vec<BoardTiming> = Vec::new();
    for (g, segment) in doc.segments.iter().enumerate() {
        // Non-final segments hold their last board through the gap.
        let tail = if Some(g) == last {
            spec.tail
        } else {
            layout.segment_gap
        };
        let Some(boards) = timed(segment, spec, tail.max(0.0)) else {
            continue;
        };
        let base = starts[g];
        for (b, t) in boards.iter().enumerate() {
            let mut start = base + t.start_ms as f64 / 1000.0;
            let mut fade_in = t.transition_ms as f64 / 1000.0;
            if b == 0 && !out.is_empty() {
                fade_in = spec.pacing.transition.min(layout.segment_gap).max(0.0);
                start = (base - fade_in).max(0.0);
            }
            let elements = t
                .elements
                .iter()
                .map(|e| ElementTiming {
                    at: (base + e.at_ms as f64 / 1000.0 - start).max(0.0),
                    window_end: (base + e.window_end_ms as f64 / 1000.0 - start).max(0.0),
                })
                .collect();
            if let Some(previous) = out.last_mut() {
                // The previous board stays until this one is fully in.
                previous.end = previous.end.max(start + fade_in).min(start + fade_in);
                previous.fade_out = fade_in;
            }
            out.push(BoardTiming {
                segment: g,
                board: b,
                ordinal: out.len(),
                start,
                end: base + t.end_ms as f64 / 1000.0,
                fade_in,
                fade_out: 0.0,
                elements,
            });
        }
    }
    out
}

fn lane(ordinal: usize, index: usize) -> u32 {
    1 + (ordinal as u32 % 2) * LANES_PER_BANK + index as u32
}

/// Worker parameters a bake depends on, from the production's resolved params.
fn bake_params(doc: &Production) -> Value {
    let pick = |id: &str| doc.params.get(id).cloned().unwrap_or(Value::Null);
    json!({
        "layout": pick("layout"),
        "tone": pick("tone"),
        "enter": pick("enter"),
        "stroke": pick("stroke"),
        "fill": pick("fill"),
        "hand": pick("hand"),
        "handSize": pick("handSize"),
        "drawSpeed": pick("drawSpeed"),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct BakeRequest {
    pub segment: String,
    pub board: String,
    pub fingerprint: String,
    pub input: Value,
}

/// A bake input for every board whose picked candidates all have images.
/// Times are relative to the board, so shifting a board never re-bakes it.
pub fn bake_requests(
    doc: &Production,
    spec: &ProductionSpec,
    layout: &NarrationLayout,
    canvas: &Canvas,
) -> Vec<BakeRequest> {
    let params = bake_params(doc);
    let mut out = Vec::new();
    for timing in board_timings(doc, spec, layout) {
        let segment = &doc.segments[timing.segment];
        let board = &segment.boards[timing.board];
        let mut elements = Vec::new();
        for (index, (element, window)) in board.elements.iter().zip(&timing.elements).enumerate() {
            let Some(file) = element.picked().and_then(|v| v.file.clone()) else {
                break;
            };
            elements.push(json!({
                "id": element.id,
                "key": format!("{}/{}", board.id, element.id),
                "role": element.role.as_str(),
                "label": element.label,
                "draw": element.draw,
                "weight": element.weight,
                "file": file,
                "lane": lane(timing.ordinal, index),
                "at": round_ms(window.at),
                "windowEnd": round_ms(window.window_end),
            }));
        }
        if elements.len() != board.elements.len() || elements.is_empty() {
            continue;
        }
        let input = json!({
            "spec": BAKE_SPEC,
            "board": board.id,
            "canvas": canvas,
            "duration": round_ms(timing.end - timing.start),
            "fadeIn": round_ms(timing.fade_in),
            "params": params,
            "elements": elements,
        });
        out.push(BakeRequest {
            segment: segment.id.clone(),
            board: board.id.clone(),
            fingerprint: fingerprint(&input),
            input,
        });
    }
    out
}

fn round_ms(seconds: f64) -> f64 {
    (seconds * 1000.0).round() / 1000.0
}

/// The plate operation's input; `None` when the recipe has no plate.
pub fn plate_request(doc: &Production, canvas: &Canvas) -> Option<(String, Value)> {
    if doc.params.get("plate").and_then(Value::as_str) == Some("none") {
        return None;
    }
    let input = json!({
        "key": "plate",
        "canvas": canvas,
        "tone": doc.params.get("tone").cloned().unwrap_or(Value::Null),
    });
    Some((fingerprint(&input), input))
}

/// Items a bake emitted, as stored in `Board.render.result.items`.
pub fn baked_items(result: &Value) -> Vec<Item> {
    result
        .get("items")
        .and_then(|v| serde_json::from_value::<Vec<Item>>(v.clone()).ok())
        .unwrap_or_default()
}

const SUBTITLE_STYLE: &str = "clean-white-outline";
const SUBTITLE_SIZE: f64 = 0.055;
const SUBTITLE_Y: f64 = 0.38;

/// Everything the host places for the production as it stands. Boards whose
/// bake is missing or stale for the current timing are left out until baked;
/// without a bake operation, picked images are placed as stills.
pub fn host_items(
    doc: &Production,
    spec: &ProductionSpec,
    layout: &NarrationLayout,
    bakes: bool,
) -> Vec<Item> {
    let starts = segment_starts(doc, layout);
    let mut out = Vec::new();
    let total = total_duration(doc, spec, layout);
    if let Some(plate) = &doc.plate {
        if let Some(file) = plate.result.get("file").and_then(Value::as_str) {
            if total > 0.0 {
                out.push(Item {
                    key: "plate".into(),
                    rev: plate.fingerprint.chars().take(16).collect(),
                    role: ItemRole::Visual,
                    lane: 0,
                    start: 0.0,
                    duration: total,
                    source: ItemSource::File {
                        file: file.into(),
                        r#in: 0.0,
                    },
                    transform: None,
                    intro: None,
                    outro: None,
                    transition: None,
                    link: None,
                    gain: None,
                    fade: None,
                });
            }
        }
    }
    for (g, segment) in doc.segments.iter().enumerate() {
        let Some(voice) = &segment.voice else {
            continue;
        };
        let base = starts[g];
        let voice_key = format!("{}/voice", segment.id);
        out.push(Item {
            key: voice_key.clone(),
            rev: voice.task.0.chars().take(16).collect(),
            role: ItemRole::Voice,
            lane: 0,
            start: base,
            duration: voice.timeline.duration.max(0.05),
            source: ItemSource::File {
                file: voice.audio.clone(),
                r#in: 0.0,
            },
            transform: None,
            intro: None,
            outro: None,
            transition: None,
            link: None,
            gain: None,
            fade: None,
        });
        if let Some(subtitles) = &segment.subtitles {
            for (index, cue) in subtitles.cues.iter().enumerate() {
                let mut content = todo_core::TextContent::new(cue.text.clone(), SUBTITLE_STYLE);
                content.layout.font_size = SUBTITLE_SIZE;
                out.push(Item {
                    key: format!("{}/sub/{index}", segment.id),
                    rev: fingerprint(&(&cue.text, cue.start, cue.end))[..16].into(),
                    role: ItemRole::Subtitle,
                    lane: 0,
                    start: base + cue.start,
                    duration: (cue.end - cue.start).max(0.08),
                    source: ItemSource::Text { content },
                    transform: Some(ItemTransform {
                        y: SUBTITLE_Y,
                        ..ItemTransform::default()
                    }),
                    intro: None,
                    outro: None,
                    transition: None,
                    link: Some(voice_key.clone()),
                    gain: None,
                    fade: None,
                });
            }
        }
    }
    // A stale bake keeps showing (re-timed) until its replacement arrives,
    // so a re-bake never blanks a board.
    for timing in board_timings(doc, spec, layout) {
        let segment = &doc.segments[timing.segment];
        let board = &segment.boards[timing.board];
        let voice_key = format!("{}/voice", segment.id);
        let mut items = Vec::new();
        if bakes {
            let Some(render) = &board.render else {
                continue;
            };
            for mut item in baked_items(&render.result) {
                let rel = item.start.max(0.0);
                item.start = timing.start + rel;
                item.duration = (timing.end - item.start).max(0.05);
                item.link = Some(voice_key.clone());
                if timing.fade_out > 0.0 && !item.role.is_audio() {
                    item.fade = Some(ItemFade {
                        r#in: 0.0,
                        out: timing.fade_out.min(item.duration * 0.5),
                    });
                }
                items.push(item);
            }
        } else {
            for (index, (element, window)) in
                board.elements.iter().zip(&timing.elements).enumerate()
            {
                let Some(variant) = element.picked() else {
                    continue;
                };
                let Some(file) = variant.file.clone() else {
                    continue;
                };
                let start = timing.start + window.at;
                let duration = (timing.end - start).max(0.05);
                items.push(Item {
                    key: format!("{}/{}", board.id, element.id),
                    rev: variant
                        .task
                        .as_ref()
                        .map_or_else(|| fingerprint(&file), |k| k.0.clone())[..16]
                        .into(),
                    role: ItemRole::Visual,
                    lane: lane(timing.ordinal, index),
                    start,
                    duration,
                    source: ItemSource::File { file, r#in: 0.0 },
                    transform: None,
                    intro: Some(Intro::new("fade", 0.4f64.min(duration))),
                    outro: None,
                    transition: None,
                    link: Some(voice_key.clone()),
                    gain: None,
                    fade: (timing.fade_out > 0.0).then(|| ItemFade {
                        r#in: 0.0,
                        out: timing.fade_out.min(duration * 0.5),
                    }),
                });
            }
        }
        out.extend(items);
    }
    out
}
