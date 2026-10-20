use std::fmt::Write;
use todo_core::ResolvedAudioClip;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoudnessMeasurement {
    pub input_i: f64,
    pub input_lra: f64,
    pub input_tp: f64,
    pub input_thresh: f64,
    pub target_offset: f64,
}

/// First pass of the two-pass loudness normalizer. FFmpeg prints the JSON
/// measurement on stderr; the export adapter parses it into
/// [`LoudnessMeasurement`] before calling [`loudnorm_apply`].
pub const fn loudnorm_measure() -> &'static str {
    "loudnorm=I=-16:TP=-1.5:LRA=11:print_format=json"
}

pub fn loudnorm_apply(measurement: LoudnessMeasurement) -> String {
    format!(
        "loudnorm=I=-16:TP=-1.0:LRA=11:measured_I={}:measured_LRA={}:measured_TP={}:measured_thresh={}:offset={}:linear=true:print_format=summary",
        measurement.input_i,
        measurement.input_lra,
        measurement.input_tp,
        measurement.input_thresh,
        measurement.target_offset
    )
}

/// Build the deterministic FFmpeg filter graph used by export. Inputs are
/// supplied separately as `-i` arguments in the same order as `clips`.
pub fn filter_graph(clips: &[ResolvedAudioClip]) -> String {
    if clips.is_empty() {
        return String::new();
    }
    let mut graph = String::new();
    for (index, clip) in clips.iter().enumerate() {
        let _ = write!(
            graph,
            "[{index}:a]atrim=start={}:duration={},asetpts=PTS-STARTPTS",
            clip.offset, clip.duration
        );
        // `atempo` only when the clip is actually retimed: besides costing
        // a resampler pass, it leaves timestamps that make a later `atrim`
        // cut the mix short (FFmpeg 9; see `mix_audio`).
        if (clip.rate - 1.0).abs() > 1e-9 {
            for factor in atempo_factors(clip.rate) {
                let _ = write!(graph, ",atempo={factor}");
            }
        }
        let _ = write!(graph, ",volume={}", clip.gain);
        if clip.fade_in > 0.0 {
            let _ = write!(graph, ",afade=t=in:st=0:d={}", clip.fade_in);
        }
        if clip.fade_out > 0.0 {
            let _ = write!(
                graph,
                ",afade=t=out:st={}:d={}",
                (clip.duration - clip.fade_out).max(0.0),
                clip.fade_out
            );
        }
        // Filter chains are separated by `;` (a newline is not a separator).
        let _ = write!(
            graph,
            ",adelay={}|{}[a{index}];",
            (clip.start * 1000.0).round(),
            (clip.start * 1000.0).round()
        );
    }
    let inputs = (0..clips.len())
        .map(|i| format!("[a{i}]"))
        .collect::<String>();
    let _ = write!(
        graph,
        "{inputs}amix=inputs={}:normalize=0[outa]",
        clips.len()
    );
    graph
}

fn atempo_factors(mut rate: f64) -> Vec<f64> {
    let mut factors = Vec::new();
    while rate > 2.0 {
        factors.push(2.0);
        rate /= 2.0;
    }
    while rate < 0.5 {
        factors.push(0.5);
        rate *= 2.0;
    }
    factors.push(rate);
    factors
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graph_contains_gain_fades_and_delay() {
        let graph = filter_graph(&[ResolvedAudioClip {
            id: "a".into(),
            path: "a.wav".into(),
            start: 1.0,
            duration: 2.0,
            offset: 0.2,
            rate: 1.0,
            gain: 0.3,
            fade_in: 0.2,
            fade_out: 0.4,
        }]);
        assert!(graph.contains("volume=0.3"));
        assert!(graph.contains("afade=t=in"));
        assert!(graph.contains("afade=t=out"));
        assert!(graph.contains("adelay=1000|1000"));
        assert!(graph.ends_with("amix=inputs=1:normalize=0[outa]"));
        assert!(!graph.contains('\n') && graph.contains("[a0];[a0]amix"));
    }

    #[test]
    fn loudnorm_has_two_pass_measurement_and_apply_forms() {
        assert!(loudnorm_measure().contains("print_format=json"));
        let filter = loudnorm_apply(LoudnessMeasurement {
            input_i: -20.0,
            input_lra: 4.0,
            input_tp: -3.0,
            input_thresh: -30.0,
            target_offset: 0.2,
        });
        assert!(filter.contains("measured_I=-20"));
        assert!(filter.contains("linear=true"));
    }
}
