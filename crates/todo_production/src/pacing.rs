//! When boards appear and elements are drawn, with room to breathe.
//!
//! The previous schedule hard-cut to a new board the instant its first stroke
//! began and let writing run until 150 ms before the next element, so boards
//! were often cut away right after they were finished (median 1.1 s, p10
//! 0.15 s of stillness across six real productions). Here:
//! - a board appears inside the voice's pause before its sentence and
//!   cross-fades in over `transition`;
//! - drawing starts a moment after the board settles, slightly ahead of the word;
//! - every board keeps a final hold of `max(min_hold, hold_ratio × length)`;
//!   writing windows shrink (faster strokes, never under `min_write`) to make room.
use crate::storyboard::{BoardPlan, Phrase};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pacing {
    /// Cross-fade between consecutive boards.
    pub transition: f64,
    /// Stillness between the end of a transition and the first stroke.
    pub settle: f64,
    /// Strokes start this much before the word is said.
    pub anticipation: f64,
    /// Minimum spacing between element starts on one board.
    pub min_gap: f64,
    /// Shortest writing window.
    pub min_write: f64,
    /// Final stillness of each board: at least this, or `hold_ratio` of its length.
    pub min_hold: f64,
    pub hold_ratio: f64,
    /// How far into the preceding pause a board may appear.
    pub max_lead: f64,
    pub min_lead: f64,
    /// Guaranteed stillness, even if late strokes must start ahead of their word.
    pub floor_hold: f64,
}

impl Default for Pacing {
    fn default() -> Self {
        Self {
            transition: 0.5,
            settle: 0.2,
            anticipation: 0.15,
            min_gap: 0.4,
            min_write: 0.8,
            min_hold: 1.2,
            hold_ratio: 0.25,
            max_lead: 0.9,
            min_lead: 0.25,
            floor_hold: 1.2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedElement {
    pub at_ms: i64,
    /// Drawing finishes by then.
    pub window_end_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedBoard {
    /// The board starts fading in.
    pub start_ms: i64,
    /// Fade-in length (0 for the first board).
    pub transition_ms: i64,
    /// The board is gone (the next board's fade-in has completed).
    pub end_ms: i64,
    pub elements: Vec<TimedElement>,
}

fn ms(seconds: f64) -> i64 {
    (seconds * 1000.0).round() as i64
}

/// Times in milliseconds from the segment's voice start; `tail` extends the
/// last board after the voice.
pub fn schedule(
    plan: &BoardPlan,
    phrases: &[Phrase],
    duration: f64,
    tail: f64,
    pacing: &Pacing,
) -> Vec<TimedBoard> {
    let n = plan.scenes.len();
    let end = duration + tail;
    let word = |i: usize| phrases[i.min(phrases.len() - 1)].start;
    // Appearance of each board, inside the pause before its first sentence.
    let mut appear: Vec<f64> = plan
        .scenes
        .iter()
        .enumerate()
        .map(|(k, board)| {
            if k == 0 || board.start == 0 {
                return 0.0;
            }
            let anchor = word(board.start);
            let pause = (anchor - phrases[board.start - 1].end).max(0.0);
            anchor - pause.clamp(pacing.min_lead, pacing.max_lead)
        })
        .collect();
    // Keep boards ordered and at least one transition apart.
    for k in 1..n {
        appear[k] = appear[k].max(appear[k - 1] + pacing.transition).min(end);
    }
    let mut out = Vec::with_capacity(n);
    for (k, board) in plan.scenes.iter().enumerate() {
        let transition = if k == 0 { 0.0 } else { pacing.transition };
        let start = appear[k];
        // The board is fully replaced once the next one has faded in.
        let leave = appear.get(k + 1).copied().unwrap_or(end);
        let gone = if k + 1 < n {
            leave + pacing.transition
        } else {
            end
        };
        let ready = if k == 0 {
            0.0
        } else {
            start + transition + pacing.settle
        };
        let length = (leave - start).max(0.0);
        let mut starts: Vec<f64> = Vec::with_capacity(board.elements.len());
        for (j, element) in board.elements.iter().enumerate() {
            // The very first stroke goes with the voice's first word.
            let anchor = if k == 0 && j == 0 {
                word(board.start)
            } else {
                word(element.at)
            };
            let mut t = (anchor - pacing.anticipation).max(ready).max(0.0);
            if let Some(previous) = starts.last() {
                t = t.max(previous + pacing.min_gap);
            }
            starts.push(t);
        }
        // Stillness at the end: wanted, and a floor that may pull late strokes slightly ahead of their word.
        let wanted_hold = pacing.min_hold.max(length * pacing.hold_ratio);
        let floor_hold = pacing.floor_hold.min(length * 0.2);
        let latest = leave - floor_hold - pacing.min_write;
        let count = starts.len();
        for (j, t) in starts.iter_mut().enumerate() {
            let cap = latest - (count - 1 - j) as f64 * pacing.min_gap;
            *t = t.min(cap).max(start);
        }
        let last_start = starts.last().copied().unwrap_or(start);
        let room = (leave - last_start).max(0.0);
        let hold = wanted_hold
            .min(room - pacing.min_write)
            .max(floor_hold.min(room));
        let deadline = leave - hold;
        let elements = starts
            .iter()
            .enumerate()
            .map(|(j, &t)| {
                let until = starts
                    .get(j + 1)
                    .map_or(deadline, |next| (next - 0.15).min(deadline));
                TimedElement {
                    at_ms: ms(t),
                    window_end_ms: ms(until.max(t + 0.3)),
                }
            })
            .collect();
        out.push(TimedBoard {
            start_ms: ms(start),
            transition_ms: ms(transition),
            end_ms: ms(gone),
            elements,
        });
    }
    out
}
