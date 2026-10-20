//! Which seconds of a cut are worth showing the user: one settled frame per
//! stretch where the picture does not change.
use crate::Project;
use todo_core::{clip_end, content_duration};

/// Shortest stretch worth a frame, seconds.
const MIN_STRETCH: f64 = 0.8;

/// Up to `limit` seconds, each 65% into a stretch between picture changes
/// (entrances have settled, the next cut has not come yet), spread evenly
/// over the cut when there are more stretches than frames.
pub fn review_times(project: &Project, limit: usize) -> Vec<f64> {
    if limit == 0 {
        return Vec::new();
    }
    let settled: Vec<f64> = stretches(project).iter().map(|s| settled(*s)).collect();
    if settled.len() <= limit {
        return settled;
    }
    (0..limit)
        .map(|i| {
            let index = (i as f64 * (settled.len() - 1) as f64 / (limit - 1).max(1) as f64).round()
                as usize;
            settled[index]
        })
        .collect()
}

fn settled((start, end): (f64, f64)) -> f64 {
    start + (end - start) * 0.65
}

/// Stretches between picture changes that are long enough to look at.
fn stretches(project: &Project) -> Vec<(f64, f64)> {
    let end = content_duration(project);
    let mut cuts: Vec<f64> = vec![0.0, end];
    for track in project
        .tracks
        .iter()
        .filter(|t| t.kind.is_visual() && !t.muted)
    {
        for clip in &track.clips {
            // Subtitles change every couple of seconds; they are not cuts.
            if clip
                .text()
                .is_some_and(|t| t.style_id.starts_with("subtitle"))
            {
                continue;
            }
            cuts.push(clip.start);
            cuts.push(clip_end(clip));
        }
    }
    cuts.retain(|t| t.is_finite() && (0.0..=end).contains(t));
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < 0.05);
    cuts.windows(2)
        .filter(|w| w[1] - w[0] >= MIN_STRETCH)
        .map(|w| (w[0], w[1]))
        .collect()
}
