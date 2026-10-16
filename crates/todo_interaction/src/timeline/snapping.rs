//! Input-space snapping. Domain commands remain responsible for final validity.
use todo_core::{clip_end, find_clip, Project};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapHit {
    pub value: f64,
    pub target: Option<f64>,
}

fn frame(project: &Project, time: f64) -> f64 {
    let fps = if project.fps.is_finite() && project.fps > 0.0 {
        project.fps
    } else {
        30.0
    };
    (time * fps).round() / fps
}
fn targets(project: &Project, exclude: &[String], playhead: f64) -> Vec<f64> {
    std::iter::once(0.0)
        .chain(std::iter::once(todo_core::workspace_duration(project)))
        .chain(std::iter::once(playhead))
        .chain(
            project
                .tracks
                .iter()
                .flat_map(|t| &t.clips)
                .filter(|c| !exclude.contains(&c.id))
                .flat_map(|c| [c.start, clip_end(c)]),
        )
        .filter(|t| t.is_finite() && *t >= 0.0)
        .collect()
}

pub fn snap_position(
    project: &Project,
    time: f64,
    duration: f64,
    exclude: &[String],
    playhead: f64,
    px_per_sec: f64,
    enabled: bool,
) -> SnapHit {
    let time = time.max(0.0);
    let mut hit = SnapHit {
        value: frame(project, time),
        target: None,
    };
    if !enabled || !px_per_sec.is_finite() || px_per_sec <= 0.0 {
        return hit;
    }
    let mut distance = 8.0 / px_per_sec;
    for target in targets(project, exclude, playhead) {
        for edge in [0.0, duration.max(0.0)] {
            let candidate = target - edge;
            let delta = (candidate - time).abs();
            if candidate >= 0.0 && delta <= distance {
                distance = delta;
                hit = SnapHit {
                    value: candidate,
                    target: Some(target),
                };
            }
        }
    }
    hit
}

pub fn snap_group_move(
    project: &Project,
    ids: &[String],
    delta: f64,
    playhead: f64,
    px_per_sec: f64,
    enabled: bool,
) -> SnapHit {
    let Some(primary) = ids.first().and_then(|id| find_clip(project, id)) else {
        return SnapHit {
            value: delta,
            target: None,
        };
    };
    let members: Vec<_> = ids.iter().filter_map(|id| find_clip(project, id)).collect();
    let earliest = members
        .iter()
        .map(|found| found.clip.start)
        .fold(f64::INFINITY, f64::min);
    let delta = delta.max(-earliest);
    let mut best = SnapHit {
        value: (frame(project, primary.clip.start + delta) - primary.clip.start).max(-earliest),
        target: None,
    };
    if !enabled || !px_per_sec.is_finite() || px_per_sec <= 0.0 {
        return best;
    }
    let mut distance = 8.0 / px_per_sec;
    let targets = targets(project, ids, playhead);
    for found in members {
        for edge in [found.clip.start, clip_end(&found.clip)] {
            for &target in &targets {
                let candidate = target - edge;
                let correction = (candidate - delta).abs();
                if candidate >= -earliest && correction <= distance {
                    distance = correction;
                    best = SnapHit {
                        value: candidate,
                        target: Some(target),
                    };
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> Project {
        let mut p = todo_core::create_project(None, None, None, None, None);
        p.media.push(todo_core::MediaItem {
            id: "m".into(),
            name: "video.mp4".into(),
            path: None,
            duration: 30.0,
            has_audio: Some(false),
            width: Some(1),
            height: Some(1),
            kind: Some(todo_core::MediaKind::Video),
        });
        p = todo_core::add_clip(
            p,
            "v1",
            todo_core::normalize_clip("moving".into(), "m".into(), 0.0, 2.0, 0.0, 1.0),
        );
        todo_core::add_clip(
            p,
            "v1",
            todo_core::normalize_clip("target".into(), "m".into(), 5.0, 2.0, 0.0, 1.0),
        )
    }
    #[test]
    fn trailing_edge_can_snap_and_disable_preserves_only_frame_alignment() {
        let p = project();
        let ids = vec!["moving".to_owned()];
        let hit = snap_group_move(&p, &ids, 2.94, 20.0, 100.0, true);
        assert!((hit.value - 3.0).abs() < 1e-9);
        assert_eq!(hit.target, Some(5.0));
        let free = snap_group_move(&p, &ids, 2.94, 20.0, 100.0, false);
        assert!((free.value - 2.9333333333).abs() < 1e-6);
        assert_eq!(free.target, None);
    }
}
