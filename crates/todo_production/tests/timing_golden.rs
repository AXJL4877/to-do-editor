//! Parity with the previous Python timing and subtitle rules (fixture inputs
//! and outputs generated once from them; see the `generator` field).
use serde_json::Value;
use todo_production::subtitles::{cues, detect_silences, split_subtitles, PAUSE_GAP};
use todo_production::voice::*;

fn fixture() -> Value {
    serde_json::from_slice(include_bytes!("fixtures/timing_golden.json")).unwrap()
}

fn wav(name: &str) -> (Vec<i16>, u32) {
    let bytes = std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    read_wav(&bytes).unwrap()
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

#[test]
fn speech_text_is_sanitized_like_before() {
    for case in fixture()["sanitize"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let out = sanitize_for_speech(input);
        assert_eq!(out.spoken, case["output"]["spoken"], "{input:?}");
        assert_eq!(out.original, case["output"]["original"], "{input:?}");
        assert_eq!(out.changed, case["output"]["changed"], "{input:?}");
    }
}

fn check_timeline(name: &str, stage: &str, actual: &Timeline, expected: &Value) {
    assert!(
        close(
            actual.duration,
            expected["duration"].as_f64().unwrap(),
            1e-9
        ),
        "{name} {stage}: duration"
    );
    let chars = expected["chars"].as_array().unwrap();
    assert_eq!(actual.chars.len(), chars.len(), "{name} {stage}: chars");
    for (a, e) in actual.chars.iter().zip(chars) {
        assert_eq!(a.text, e["text"], "{name} {stage}");
        assert!(
            close(a.start, e["start"].as_f64().unwrap(), 1e-9),
            "{name} {stage}: char {} start {} vs {}",
            a.id,
            a.start,
            e["start"]
        );
        assert!(
            close(a.end, e["end"].as_f64().unwrap(), 1e-9),
            "{name} {stage}: char {} end {} vs {}",
            a.id,
            a.end,
            e["end"]
        );
    }
    let phrases = expected["phrases"].as_array().unwrap();
    assert_eq!(
        actual.phrases.len(),
        phrases.len(),
        "{name} {stage}: phrases"
    );
    for (a, e) in actual.phrases.iter().zip(phrases) {
        assert_eq!(a.text, e["text"]);
        assert_eq!(serde_json::to_value(&a.char_ids).unwrap(), e["char_ids"]);
        assert!(
            close(a.start, e["start"].as_f64().unwrap(), 1e-9),
            "{name} {stage}: phrase start"
        );
        assert!(
            close(a.end, e["end"].as_f64().unwrap(), 1e-9),
            "{name} {stage}: phrase end"
        );
    }
    let sentences = expected["sentences"].as_array().unwrap();
    assert_eq!(actual.sentences.len(), sentences.len());
    for (a, e) in actual.sentences.iter().zip(sentences) {
        assert_eq!(a.text, e["text"], "{name} {stage}");
        assert!(
            close(a.start, e["start"].as_f64().unwrap(), 1e-9),
            "{name} {stage}: sentence start"
        );
        assert!(
            close(a.end, e["end"].as_f64().unwrap(), 1e-9),
            "{name} {stage}: sentence end"
        );
    }
}

#[test]
fn timelines_retime_and_snap_like_before() {
    let golden = fixture();
    let cases = golden["timing"].as_array().unwrap();
    assert!(cases.len() >= 3);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let raw: Vec<Value> = serde_json::from_value(case["sentencesRaw"].clone()).unwrap();
        let words: Vec<Word> = serde_json::from_value(case["words"].clone()).unwrap();
        let mut timeline = build_timeline(case["text"].as_str().unwrap(), &raw, &words);
        check_timeline(name, "hierarchy", &timeline, &case["hierarchy"]);
        let Some(file) = case["wav"].as_str() else {
            continue;
        };
        let (samples, rate) = wav(file);
        let pcm = Pcm {
            samples: &samples,
            rate,
        };
        timeline.retime(pcm);
        check_timeline(name, "retimed", &timeline, &case["retimed"]);
        let moved = timeline.snap_sentence_edges(pcm);
        assert_eq!(
            moved as u64,
            case["snapMoved"].as_u64().unwrap(),
            "{name}: moved"
        );
        check_timeline(name, "snapped", &timeline, &case["snapped"]);
    }
}

#[test]
fn subtitle_lines_split_like_before() {
    for case in fixture()["split"].as_array().unwrap() {
        let text = case["text"].as_str().unwrap();
        let max = case["maxChars"].as_u64().unwrap() as usize;
        assert_eq!(
            serde_json::to_value(split_subtitles(text, max)).unwrap(),
            case["chunks"],
            "{text}"
        );
    }
}

#[test]
fn silences_match_ffmpeg_silencedetect() {
    for case in fixture()["align"].as_array().unwrap() {
        let file = if case["name"] == "probe" {
            "probe_voice.wav"
        } else {
            "synthetic_voice.wav"
        };
        let (samples, rate) = wav(file);
        let ours = detect_silences(&samples, rate, -30.0, PAUSE_GAP);
        let theirs: Vec<(f64, f64)> = serde_json::from_value(case["silences"].clone()).unwrap();
        assert_eq!(
            ours.len(),
            theirs.len(),
            "{}: {ours:?} vs {theirs:?}",
            case["name"]
        );
        for (a, b) in ours.iter().zip(&theirs) {
            // FFmpeg prints six significant digits.
            assert!(
                close(a.0, b.0, 1e-4) && close(a.1, b.1, 1e-4),
                "{a:?} vs {b:?}"
            );
        }
    }
}

#[test]
fn cues_are_timed_like_before() {
    for case in fixture()["align"].as_array().unwrap() {
        let chars: Vec<(String, f64, f64)> = serde_json::from_value(case["chars"].clone()).unwrap();
        let chars: Vec<(char, f64, f64)> = chars
            .into_iter()
            .map(|(t, a, b)| (t.chars().next().unwrap(), a, b))
            .collect();
        let silences: Vec<(f64, f64)> = serde_json::from_value(case["silences"].clone()).unwrap();
        let max = case["maxChars"].as_u64().unwrap() as usize;
        let actual = cues(
            case["text"].as_str().unwrap(),
            &chars,
            case["duration"].as_f64().unwrap(),
            &silences,
            max,
        );
        let expected: Vec<(f64, f64, String)> =
            serde_json::from_value(case["events"].clone()).unwrap();
        assert_eq!(actual.len(), expected.len(), "{}: {actual:?}", case["name"]);
        for (a, (s, e, t)) in actual.iter().zip(&expected) {
            assert_eq!(&a.text, t);
            assert!(
                close(a.start, *s, 1e-9) && close(a.end, *e, 1e-9),
                "{a:?} vs {s} {e}"
            );
        }
    }
}
