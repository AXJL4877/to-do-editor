//! Rule-based review of a cut: what can be checked from the document alone,
//! before (and cheaper than) a model looks at frames. Every finding names the
//! clips and the second it is about, so a fix can target exactly them.
use super::view::{project_view, ElementView, ProjectView};
use crate::Project;
use serde::{Deserialize, Serialize};
use todo_core::{content_duration, visible_hits};

/// Thresholds a style recipe sets for its look (`recipe.json` → `review`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Rubric {
    /// Inner frame pictures and text must stay in, canvas fractions.
    pub safe_area: [f64; 2],
    /// Where a subtitle's bottom edge may sit, canvas-height fractions.
    pub subtitle_baseline: [f64; 2],
    pub subtitle_max_chars: usize,
    /// Largest share of the smaller element two pictures may overlap.
    pub max_overlap: f64,
    /// Text style ids that are subtitles.
    pub subtitle_styles: Vec<String>,
    /// Longest the picture may stay unchanged, seconds (subtitles aside).
    pub max_hold: f64,
    /// Closed yes/no questions the reviewer answers for every checked
    /// picture; "yes" is the good answer.
    pub checks: Vec<Check>,
    /// Production keys of the pictures to check (`*` matches anything).
    pub checked: Vec<String>,
    /// Production key prefixes of the spoken lines shown with each picture.
    pub lines: Vec<String>,
    /// The production parameter that redraws pictures (`key:variant,…`).
    pub redraw: Option<String>,
}

/// One closed question of a style's checklist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub id: String,
    pub question: String,
}

impl Default for Rubric {
    fn default() -> Self {
        Self {
            safe_area: [0.02, 0.98],
            subtitle_baseline: [0.88, 0.96],
            subtitle_max_chars: 18,
            max_overlap: 0.3,
            subtitle_styles: vec!["subtitle-white-heavy".into(), "clean-white-outline".into()],
            max_hold: 10.0,
            checks: Vec::new(),
            checked: Vec::new(),
            lines: Vec::new(),
            redraw: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Must be fixed before the cut goes to the user.
    Block,
    Warn,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub code: String,
    pub severity: Severity,
    pub at: Option<f64>,
    pub clips: Vec<String>,
    pub message: String,
}

/// A picture covering most of the frame is a background or a card.
fn is_backdrop(element: &ElementView) -> bool {
    element.rect.is_some_and(|r| r.nw * r.nh >= 0.85)
}

fn is_subtitle(element: &ElementView, rubric: &Rubric) -> bool {
    element
        .text
        .as_ref()
        .is_some_and(|text| rubric.subtitle_styles.contains(&text.style_id))
}

fn overlap_share(a: &ElementView, b: &ElementView) -> f64 {
    let (Some(a), Some(b)) = (a.rect, b.rect) else {
        return 0.0;
    };
    let w = (a.nx + a.nw).min(b.nx + b.nw) - a.nx.max(b.nx);
    let h = (a.ny + a.nh).min(b.ny + b.nh) - a.ny.max(b.ny);
    if w <= 0.0 || h <= 0.0 {
        return 0.0;
    }
    let smaller = (a.nw * a.nh).min(b.nw * b.nh);
    if smaller <= 0.0 {
        0.0
    } else {
        w * h / smaller
    }
}

/// Seconds worth sampling: every element's settled middle, plus a regular
/// grid for frames with nothing on screen.
fn sample_times(project: &Project, step: f64) -> Vec<f64> {
    let end = content_duration(project);
    let mut times: Vec<f64> = project
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .map(|clip| clip.start + clip.duration * 0.5)
        .collect();
    let mut t = step * 0.5;
    while t < end {
        times.push(t);
        t += step;
    }
    times.sort_by(f64::total_cmp);
    times.dedup_by(|a, b| (*a - *b).abs() < 0.05);
    times
}

pub fn inspect(project: &Project, rubric: &Rubric) -> Vec<Finding> {
    let mut findings: Vec<Finding> = Vec::new();
    let mut push = |finding: Finding| {
        // One finding per code and clip set is enough to act on.
        if !findings
            .iter()
            .any(|f| f.code == finding.code && f.clips == finding.clips)
        {
            findings.push(finding);
        }
    };
    let [low, high] = rubric.safe_area;
    let mut empty_since: Option<f64> = None;
    // Where each element rests (its own transform, no motion applied).
    let resting: std::collections::HashMap<String, super::view::Rect> = project_view(project, None)
        .elements
        .into_iter()
        .filter_map(|e| e.rect.map(|rect| (e.clip_id, rect)))
        .collect();
    for time in sample_times(project, 0.5) {
        if visible_hits(project, time).is_empty() {
            empty_since.get_or_insert(time);
            continue;
        } else if let Some(since) = empty_since.take() {
            push(Finding {
                code: "empty_frame".into(),
                severity: Severity::Block,
                at: Some(since),
                clips: vec![],
                message: format!("{since:.1}s 起画面为空（没有任何可见图层）"),
            });
        }
        let mut view: ProjectView = project_view(project, Some(time));
        for element in &mut view.elements {
            if let Some(rect) = resting.get(&element.clip_id) {
                element.rect = Some(*rect);
            }
        }
        let shown: Vec<&ElementView> = view
            .elements
            .iter()
            .filter(|e| e.visible == Some(true) && e.rect.is_some())
            .collect();
        for element in &shown {
            let rect = element.rect.unwrap();
            let subtitle = is_subtitle(element, rubric);
            if !is_backdrop(element)
                && (rect.nx < low - 1e-6
                    || rect.ny < low - 1e-6
                    || rect.nx + rect.nw > high + 1e-6
                    || rect.ny + rect.nh > high + 1e-6)
            {
                push(Finding {
                    code: "out_of_frame".into(),
                    severity: if subtitle {
                        Severity::Block
                    } else {
                        Severity::Warn
                    },
                    at: Some(time),
                    clips: vec![element.clip_id.clone()],
                    message: format!(
                        "「{}」超出安全区（x {:.2}–{:.2}, y {:.2}–{:.2}）",
                        element.name,
                        rect.nx,
                        rect.nx + rect.nw,
                        rect.ny,
                        rect.ny + rect.nh
                    ),
                });
            }
            if subtitle {
                let chars = element.text.as_ref().map_or(0, |t| t.chars);
                if chars > rubric.subtitle_max_chars {
                    push(Finding {
                        code: "subtitle_long".into(),
                        severity: Severity::Warn,
                        at: Some(time),
                        clips: vec![element.clip_id.clone()],
                        message: format!(
                            "字幕「{}」{chars} 字，超过 {} 字",
                            element.name, rubric.subtitle_max_chars
                        ),
                    });
                }
                let bottom = rect.ny + rect.nh;
                let [lo, hi] = rubric.subtitle_baseline;
                if bottom < lo || bottom > hi {
                    push(Finding {
                        code: "subtitle_position".into(),
                        severity: Severity::Warn,
                        at: Some(time),
                        clips: vec![element.clip_id.clone()],
                        message: format!("字幕底边在 {bottom:.3}，应在 {lo:.2}–{hi:.2}"),
                    });
                }
            }
        }
        let pictures: Vec<&&ElementView> = shown
            .iter()
            .filter(|e| !is_backdrop(e) && !is_subtitle(e, rubric))
            .collect();
        for (i, a) in pictures.iter().enumerate() {
            for b in &pictures[i + 1..] {
                // Two pictures of one track only meet inside a transition.
                if a.track_id == b.track_id {
                    continue;
                }
                let share = overlap_share(a, b);
                if share > rubric.max_overlap {
                    let text = a.text.is_some() || b.text.is_some();
                    push(Finding {
                        code: if text { "text_covers" } else { "overlap" }.into(),
                        severity: Severity::Warn,
                        at: Some(time),
                        clips: vec![a.clip_id.clone(), b.clip_id.clone()],
                        message: format!(
                            "「{}」与「{}」重叠 {:.0}%",
                            a.name,
                            b.name,
                            share * 100.0
                        ),
                    });
                }
            }
        }
    }
    if let Some(since) = empty_since {
        push(Finding {
            code: "empty_frame".into(),
            severity: Severity::Block,
            at: Some(since),
            clips: vec![],
            message: format!("{since:.1}s 起画面为空（没有任何可见图层）"),
        });
    }
    // Rhythm: a picture held too long reads as a stalled video.
    let mut changes: Vec<f64> = project
        .tracks
        .iter()
        .filter(|t| t.kind.is_visual() && !t.muted)
        .flat_map(|t| &t.clips)
        .filter(|c| {
            c.text()
                .is_none_or(|text| !rubric.subtitle_styles.contains(&text.style_id))
        })
        .flat_map(|c| [c.start, c.start + c.duration])
        .collect();
    changes.push(0.0);
    changes.push(content_duration(project));
    changes.sort_by(f64::total_cmp);
    changes.dedup_by(|a, b| (*a - *b).abs() < 0.05);
    for pair in changes.windows(2) {
        if pair[1] - pair[0] > rubric.max_hold {
            push(Finding {
                code: "static_too_long".into(),
                severity: Severity::Warn,
                at: Some(pair[0]),
                clips: vec![],
                message: format!(
                    "{:.1}s–{:.1}s 画面 {:.1} 秒没有变化（上限 {:.1} 秒）",
                    pair[0],
                    pair[1],
                    pair[1] - pair[0],
                    rubric.max_hold
                ),
            });
        }
    }
    for element in project_view(project, None).elements {
        if element.transition.as_ref().is_some_and(|t| !t.active) {
            push(Finding {
                code: "transition_inactive".into(),
                severity: Severity::Warn,
                at: Some(element.start),
                clips: vec![element.clip_id.clone()],
                message: format!("「{}」的转场没有生效：前一个片段没有紧挨着", element.name),
            });
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_core::{add_clip, create_project, normalize_clip, MediaItem, MediaKind, Transform};

    fn picture(id: &str, w: u32, h: u32) -> MediaItem {
        MediaItem {
            id: id.into(),
            name: format!("{id}.png"),
            path: Some(format!("{id}.png")),
            duration: 5.0,
            has_audio: Some(false),
            width: Some(w),
            height: Some(h),
            kind: Some(MediaKind::Image),
            folder_id: None,
            alpha: None,
            reveal: None,
        }
    }

    #[test]
    fn finds_empty_frames_out_of_frame_and_overlap() {
        let mut p = create_project(None, None, None, None, None);
        p.media.push(picture("bg", 1920, 1080));
        p.media.push(picture("a", 1000, 1000));
        p = add_clip(
            p,
            "v1",
            normalize_clip("bg".into(), "bg".into(), 0.0, 4.0, 0.0, 1.0),
        );
        p = todo_core::add_track(p, todo_core::TrackKind::Video);
        p = todo_core::add_track(p, todo_core::TrackKind::Video);
        let lanes: Vec<String> = p
            .tracks
            .iter()
            .filter(|t| t.kind == todo_core::TrackKind::Video && t.id != "v1")
            .map(|t| t.id.clone())
            .collect();
        let mut left = normalize_clip("left".into(), "a".into(), 0.0, 4.0, 0.0, 1.0);
        left.transform = Transform {
            x: -0.45,
            scale: 0.5,
            ..Transform::default()
        };
        let mut right = normalize_clip("right".into(), "a".into(), 0.0, 4.0, 0.0, 1.0);
        right.transform = Transform {
            x: -0.30,
            scale: 0.5,
            ..Transform::default()
        };
        p = add_clip(p, &lanes[0], left);
        p = add_clip(p, &lanes[1], right);
        // Nothing after 4 s until a clip at 6 s.
        p = add_clip(
            p,
            "v1",
            normalize_clip("late".into(), "bg".into(), 6.0, 1.0, 0.0, 1.0),
        );
        let findings = inspect(&p, &Rubric::default());
        let codes: Vec<&str> = findings.iter().map(|f| f.code.as_str()).collect();
        assert!(codes.contains(&"empty_frame"), "{findings:#?}");
        assert!(codes.contains(&"out_of_frame"), "{findings:#?}");
        assert!(codes.contains(&"overlap"), "{findings:#?}");
        assert!(findings
            .iter()
            .filter(|f| f.code == "out_of_frame")
            .all(|f| f.clips == ["left"]));
    }

    #[test]
    fn the_two_sides_of_a_transition_are_not_an_overlap() {
        let mut p = create_project(None, None, None, None, None);
        p.media.push(picture("a", 1000, 1000));
        let mut first = normalize_clip("first".into(), "a".into(), 0.0, 2.0, 0.0, 1.0);
        first.transform.scale = 0.5;
        let mut second = normalize_clip("second".into(), "a".into(), 2.0, 2.0, 0.0, 1.0);
        second.transform.scale = 0.5;
        second.transition = Some(todo_core::MotionRef::new("crossfade", 1.0));
        p = add_clip(p, "v1", first);
        p = add_clip(p, "v1", second);
        let findings = inspect(&p, &Rubric::default());
        assert!(
            findings.iter().all(|f| f.code != "overlap"),
            "{findings:#?}"
        );
    }
}
