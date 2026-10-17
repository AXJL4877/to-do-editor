//! A whole production without network: fake model answers, synthetic audio.
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use todo_production::extension::items::Item;
use todo_production::pacing::{schedule, Pacing};
use todo_production::production::*;
use todo_production::segment::Limits;
use todo_production::storyboard::{BoardPlan, PlannedBoard, PlannedElement};
use todo_production::voice::{Pcm, Word};
use todo_production::work::*;
use todo_production::{TaskKey, TaskRequest};

fn golden() -> Value {
    serde_json::from_slice(include_bytes!("fixtures/production_golden.json")).unwrap()
}

fn spec() -> ProductionSpec {
    let g = golden();
    let assets = &g["pack"]["assets"];
    let mut vars = BTreeMap::new();
    vars.insert("palette".to_owned(), "white chalk only".to_owned());
    ProductionSpec {
        chat_model: "gpt-6-astra".into(),
        planner_system: g["pack"]["system"].as_str().unwrap().into(),
        max_elements: 4,
        board_sec: 10.0,
        variants: 1,
        asset: AssetSpec {
            template: assets["template"].as_str().unwrap().into(),
            roles: serde_json::from_value(assets["roles"].clone()).unwrap(),
            avoid: serde_json::from_value(assets["avoid"].clone()).unwrap(),
            vars,
            model: "gpt-image-2".into(),
            size: "1024x1024".into(),
            quality: Some("low".into()),
            transparent: false,
        },
        voice: VoiceSpec {
            speaker: "zh_female_mizai_uranus_bigtts".into(),
            resource_id: "seed-tts-2.0".into(),
            sample_rate: 16000,
        },
        subtitle_chars: Some(18),
        pacing: Pacing::default(),
        tail: 0.5,
    }
}

fn script(name: &str) -> String {
    golden()["segmentCases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap()["raw"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn doc(script: &str, split: SplitMode) -> Production {
    Production::new(
        "p_test",
        PackRef {
            id: "builtin.chalk-hand".into(),
            version: "4.0.0".into(),
        },
        script,
        BTreeMap::new(),
        split,
        Limits {
            target: 120,
            max: 200,
        },
    )
}

/// One word per character, 0.2 s each (punctuation 0.1 s), a 0.5 s pause
/// after sentence ends; the PCM is a tone while speaking.
fn fake_voice(text: &str, rate: u32) -> (Vec<Value>, Vec<Word>, Vec<i16>) {
    let mut t = 0.3;
    let mut words = Vec::new();
    let mut sentences = Vec::new();
    let mut current: Vec<Value> = Vec::new();
    let mut spans = Vec::new();
    for c in text.chars().filter(|c| !c.is_whitespace()) {
        let punct = "，。！？、；：,.!?".contains(c);
        let d = if punct { 0.1 } else { 0.2 };
        if !punct {
            spans.push((t, t + d));
        }
        current.push(json!({"word": c.to_string(), "startTime": t, "endTime": t + d}));
        words.push(Word {
            word: c.to_string(),
            start: t,
            end: t + d,
        });
        t += d;
        if "。！？".contains(c) {
            sentences.push(json!({ "words": std::mem::take(&mut current) }));
            t += 0.5;
        }
    }
    if !current.is_empty() {
        sentences.push(json!({ "words": current }));
    }
    let total = ((t + 0.4) * f64::from(rate)) as usize;
    let mut pcm = vec![0i16; total];
    for (a, b) in spans {
        let (from, to) = (
            (a * f64::from(rate)) as usize,
            (b * f64::from(rate)) as usize,
        );
        for (i, sample) in pcm.iter_mut().enumerate().take(to).skip(from) {
            *sample = ((i as f64 * 0.09).sin() * 8000.0) as i16;
        }
    }
    (sentences, words, pcm)
}

fn targets(work: &[Work]) -> Vec<&Target> {
    work.iter().map(|w| &w.target).collect()
}

fn voice_all(doc: &mut Production, spec: &ProductionSpec) {
    for w in plan_work(doc, spec) {
        if let Target::Voice { segment } = &w.target {
            let TaskRequest::Speech(speech) = &w.spec.request else {
                panic!()
            };
            let (raw, words, pcm) = fake_voice(&speech.text, 16000);
            let pcm = Pcm {
                samples: &pcm,
                rate: 16000,
            };
            apply_voice(
                doc,
                spec,
                segment,
                SpeechResult {
                    task: w.key(),
                    audio: format!("{}.wav", w.key()),
                    sentences_raw: &raw,
                    words: &words,
                    pcm,
                },
            )
            .unwrap();
        }
    }
}

fn storyboard_all(doc: &mut Production, spec: &ProductionSpec, draws: &[&str]) {
    for w in plan_work(doc, spec) {
        if let Target::Storyboard { segment } = &w.target {
            let g = doc.segments.iter().position(|s| &s.id == segment).unwrap();
            let phrases = doc.segments[g].voice.as_ref().unwrap().timeline.phrases();
            let last = phrases.len() - 1;
            let answer = json!({
                "notes": "两块",
                "scenes": [
                    {"start": 0, "elements": [
                        {"at": 0, "label": "主角", "draw": draws[0], "role": "hero"},
                        {"at": 1, "label": "银行", "draw": draws[1]}]},
                    {"start": last, "elements": [{"at": last, "label": "结论", "draw": draws[2], "role": "accent"}]},
                ]
            });
            let reply = format!("好的：\n```json\n{answer}\n```");
            apply_storyboard(doc, spec, segment, Ok(reply)).unwrap();
        }
    }
}

/// Generate every pending candidate; returns the number of distinct requests.
fn images_all(doc: &mut Production, spec: &ProductionSpec) -> usize {
    refresh(doc, spec);
    let work = plan_work(doc, spec);
    let unique: BTreeSet<TaskKey> = work.iter().map(Work::key).collect();
    for w in &work {
        if let Target::Image { element, variant } = &w.target {
            apply_image(
                doc,
                element,
                variant,
                w.key(),
                Ok(format!("{}.png", w.key())),
            )
            .unwrap();
        }
    }
    unique.len()
}

fn board_plan(segment: &Segment) -> BoardPlan {
    BoardPlan {
        title: String::new(),
        notes: String::new(),
        scenes: segment
            .boards
            .iter()
            .map(|b| PlannedBoard {
                start: b.start,
                elements: b
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

#[test]
fn a_production_runs_from_outline_to_candidates() {
    let spec = spec();
    let mut doc = doc(&script("复利口播"), SplitMode::Semantic);
    let work = plan_work(&doc, &spec);
    assert_eq!(
        targets(&work),
        [&Target::Outline { chunk: 0 }],
        "short scripts are one request"
    );
    let (units, _) = outline_units(&doc);
    let half = units.len() / 2;
    let answer = json!({
        "title": "复利的秘密",
        "cast": [{"name": "小明", "look": "a stick figure with a cap"}],
        "segments": [{"start": 0, "title": "开场"}, {"start": half, "title": "后半"}]
    });
    apply_outline(&mut doc, &[Ok(answer.to_string())]);
    assert_eq!(doc.outline.as_ref().unwrap().source, Source::Ai);
    assert_eq!(doc.title, "复利的秘密");
    assert!(doc.segments.len() >= 2);
    assert_eq!(doc.segments[0].title, "开场");
    assert_eq!(doc.cast.len(), 1);

    let work = plan_work(&doc, &spec);
    assert!(work
        .iter()
        .all(|w| matches!(w.target, Target::Voice { .. })));
    assert_eq!(work.len(), doc.segments.len());
    voice_all(&mut doc, &spec);
    assert!(doc
        .segments
        .iter()
        .all(|s| s.subtitles.as_ref().is_some_and(|t| !t.cues.is_empty())));

    let work = plan_work(&doc, &spec);
    assert!(work
        .iter()
        .all(|w| matches!(w.target, Target::Storyboard { .. })));
    storyboard_all(
        &mut doc,
        &spec,
        &["a stick figure with a cap", "a bank", "big chalk words"],
    );
    // Same descriptions in every segment: one request per distinct image.
    assert_eq!(images_all(&mut doc, &spec), 3);
    assert!(plan_work(&doc, &spec).is_empty(), "everything is done");

    for segment in &doc.segments {
        let voice = segment.voice.as_ref().unwrap();
        let timed = schedule(
            &board_plan(segment),
            &voice.timeline.phrases(),
            voice.timeline.duration,
            spec.tail,
            &spec.pacing,
        );
        assert_eq!(timed.len(), segment.boards.len());
    }

    // Editing one segment's text redoes only that segment.
    let first = doc.segments[0].id.clone();
    let text = format!("{}再补一句。", doc.segments[0].text);
    doc.apply_edits(
        &[Edit::SetSegmentText {
            segment: first.clone(),
            text,
        }],
        4,
    )
    .unwrap();
    let work = plan_work(&doc, &spec);
    assert_eq!(
        targets(&work),
        [&Target::Voice {
            segment: first.clone()
        }]
    );
    voice_all(&mut doc, &spec);
    let work = plan_work(&doc, &spec);
    assert_eq!(targets(&work), [&Target::Storyboard { segment: first }]);
    storyboard_all(
        &mut doc,
        &spec,
        &["a stick figure with a cap", "a bank", "big chalk words"],
    );
    assert_eq!(
        images_all(&mut doc, &spec),
        0,
        "unchanged descriptions keep their candidates"
    );
}

#[test]
fn user_storyboards_survive_while_the_voice_is_unchanged() {
    let spec = spec();
    let mut doc = doc(&script("复利口播"), SplitMode::Paragraph);
    apply_outline(&mut doc, &[]);
    voice_all(&mut doc, &spec);
    storyboard_all(&mut doc, &spec, &["a", "b", "c"]);
    let element = doc.segments[0].boards[0].elements[0].id.clone();
    let patch = ElementPatch {
        label: Some("改名".into()),
        ..ElementPatch::default()
    };
    doc.apply_edits(&[Edit::SetElement { element, patch }], 4)
        .unwrap();
    assert_eq!(doc.segments[0].plan.as_ref().unwrap().source, Source::User);
    assert!(plan_work(&doc, &spec)
        .iter()
        .all(|w| !matches!(w.target, Target::Storyboard { .. })));
}

#[test]
fn long_scripts_outline_in_chunks_and_fall_back_per_chunk() {
    let spec = spec();
    let paragraph = script("复利口播");
    let long: String = (0..20)
        .map(|i| format!("第{i}部分。{paragraph}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut doc = doc(&long, SplitMode::Semantic);
    let work = plan_work(&doc, &spec);
    assert!(work.len() >= 3, "{} chunks", work.len());
    let (_, chunks) = outline_units(&doc);
    let answers: Vec<Result<String, String>> = chunks
        .iter()
        .enumerate()
        .map(|(i, _)| {
            if i == 1 {
                Err("network gave up".into())
            } else {
                Ok(json!({"segments": [{"start": 0}]}).to_string())
            }
        })
        .collect();
    let before: String = doc.script.chars().filter(|c| !c.is_whitespace()).collect();
    apply_outline(&mut doc, &answers);
    let outline = doc.outline.as_ref().unwrap();
    assert_eq!(outline.source, Source::Fallback);
    assert!(outline.notes.contains("network gave up"));
    let after: String = doc
        .segments
        .iter()
        .flat_map(|s| s.text.chars())
        .filter(|c| !c.is_whitespace())
        .collect();
    assert_eq!(after, before, "segments are the script");
    assert!(doc
        .segments
        .iter()
        .all(|s| todo_production::text::char_count(&s.text) <= 200));
}

#[test]
fn edit_batches_are_all_or_nothing() {
    let mut doc = doc(&script("复利口播"), SplitMode::Paragraph);
    apply_outline(&mut doc, &[]);
    let before = doc.clone();
    let first = doc.segments[0].id.clone();
    let error = doc
        .apply_edits(
            &[
                Edit::SetTitle {
                    title: "新标题".into(),
                },
                Edit::RemoveSegment {
                    segment: "g999".into(),
                },
            ],
            4,
        )
        .unwrap_err();
    assert!(error.0.contains("g999"));
    assert_eq!(doc, before);
    doc.apply_edits(
        &[
            Edit::SetTitle {
                title: "新标题".into(),
            },
            Edit::SplitSegment {
                segment: first,
                at: 5,
            },
        ],
        4,
    )
    .unwrap();
    assert_eq!(doc.title, "新标题");
    assert!(doc.title_locked);
    assert_eq!(doc.segments.len(), before.segments.len() + 1);
    let edit: Edit =
        serde_json::from_value(json!({"op": "pickVariant", "element": "e1", "variant": "v2"}))
            .unwrap();
    assert_eq!(
        edit,
        Edit::PickVariant {
            element: "e1".into(),
            variant: "v2".into()
        }
    );
}

/// Two voiced segments with two boards each and images for every element.
fn ready_doc(spec: &ProductionSpec) -> Production {
    let paragraph = script("复利口播");
    let mut doc = doc(&format!("{paragraph}\n\n{paragraph}"), SplitMode::Paragraph);
    apply_outline(&mut doc, &[]);
    assert!(doc.segments.len() >= 2, "{}", doc.segments.len());
    voice_all(&mut doc, spec);
    storyboard_all(&mut doc, spec, &["a", "b", "c"]);
    images_all(&mut doc, spec);
    doc
}

#[test]
fn narration_boards_crossfade_and_never_share_a_lane() {
    use todo_production::narration::*;
    let spec = spec();
    let doc = ready_doc(&spec);
    let layout = NarrationLayout::default();
    let boards = board_timings(&doc, &spec, &layout);
    assert!(boards.len() >= 4);
    for pair in boards.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        // The outgoing board is fully visible until the next one is in.
        assert!((a.end - (b.start + b.fade_in)).abs() < 1e-9, "{a:?} {b:?}");
        assert_eq!(a.fade_out, b.fade_in);
        assert!(b.fade_in > 0.0, "no hard cuts, also across segments");
    }
    let canvas = todo_production::extension::protocol::Canvas {
        width: 1920,
        height: 1080,
        fps: 30.0,
    };
    let bakes = bake_requests(&doc, &spec, &layout, &canvas);
    assert_eq!(bakes.len(), boards.len());
    // Consecutive boards use different lane banks.
    let lanes = |input: &Value| -> Vec<u64> {
        input["elements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["lane"].as_u64().unwrap())
            .collect()
    };
    for pair in bakes.windows(2) {
        let (a, b) = (lanes(&pair[0].input), lanes(&pair[1].input));
        assert!(a.iter().all(|lane| !b.contains(lane)), "{a:?} {b:?}");
    }
    // Writing windows lie inside their board.
    for (bake, timing) in bakes.iter().zip(&boards) {
        for e in bake.input["elements"].as_array().unwrap() {
            let at = e["at"].as_f64().unwrap();
            assert!(at >= 0.0 && at <= timing.end - timing.start);
        }
    }
}

#[test]
fn editing_one_segment_keeps_every_other_key_and_bake() {
    use todo_production::narration::*;
    let spec = spec();
    let mut doc = ready_doc(&spec);
    let layout = NarrationLayout::default();
    let canvas = todo_production::extension::protocol::Canvas {
        width: 1920,
        height: 1080,
        fps: 30.0,
    };
    let before = bake_requests(&doc, &spec, &layout, &canvas);
    let first = doc.segments[0].id.clone();
    let last = doc.segments.last().unwrap().id.clone();
    let items_before = host_items(&doc, &spec, &layout, false);
    // Re-voice the last segment only.
    let text = doc.segments.last().unwrap().text.clone();
    doc.apply_edits(
        &[Edit::SetSegmentText {
            segment: last.clone(),
            text: format!("{text}再补一句。"),
        }],
        4,
    )
    .unwrap();
    voice_all(&mut doc, &spec);
    let after = bake_requests(&doc, &spec, &layout, &canvas);
    let first_boards = |list: &[BakeRequest]| -> Vec<String> {
        list.iter()
            .filter(|b| b.segment == first)
            .map(|b| b.fingerprint.clone())
            .collect()
    };
    assert_eq!(first_boards(&before), first_boards(&after));
    let items_after = host_items(&doc, &spec, &layout, false);
    let rev =
        |items: &[Item], key: &str| items.iter().find(|i| i.key == key).map(|i| i.rev.clone());
    let voice = format!("{first}/voice");
    assert!(rev(&items_before, &voice).is_some());
    assert_eq!(rev(&items_before, &voice), rev(&items_after, &voice));
    assert_ne!(
        rev(&items_before, &format!("{last}/voice")),
        rev(&items_after, &format!("{last}/voice"))
    );
    // Subtitles sit inside their segment's voice.
    let starts = segment_starts(&doc, &layout);
    for item in items_after
        .iter()
        .filter(|i| i.key.starts_with(&format!("{first}/sub/")))
    {
        assert!(item.start >= starts[0] && item.link.as_deref() == Some(voice.as_str()));
    }
}
