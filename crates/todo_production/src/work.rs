//! What to generate next, and how results enter the document. Planning is a
//! pure read of the document: it lists every request whose inputs are ready
//! and whose result is missing or stale. The executor deduplicates by task
//! key, so asking again for a running or finished task costs nothing.
use crate::pacing::Pacing;
use crate::production::{
    Board, OutlineInfo, PlanState, Production, Source, SplitMode, Subtitles, Variant,
    VariantSource, Voice,
};
use crate::segment::{
    build_segment_messages, enforce_limits, heuristic_starts, sanitize_outline, segment_texts,
    split_units, CastMember, Unit,
};
use crate::storyboard::{
    asset_prompt, build_board_messages, fallback_plan, sanitize_plan, BoardPrompt, Phrase,
};
use crate::subtitles::timeline_cues;
use crate::tasks::{AudioRequest, ImageRequest, LanguageRequest, TaskKey, TaskRequest, TaskSpec};
use crate::text::js_slice;
use crate::voice::{sanitize_for_speech, speech_timeline, Pcm, Word};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Scripts longer than this are outlined in parallel chunks (cut at paragraph ends).
pub const OUTLINE_CHUNK_CHARS: usize = 2400;

/// The pack and parameters, resolved into everything planning needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionSpec {
    pub planner_system: String,
    pub max_elements: usize,
    pub board_sec: f64,
    /// Candidates per new element.
    pub variants: usize,
    pub asset: AssetSpec,
    pub voice: VoiceSpec,
    /// Characters per subtitle line; `None` disables subtitles.
    pub subtitle_chars: Option<usize>,
    pub pacing: Pacing,
    /// Seconds the last board stays after the voice.
    pub tail: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetSpec {
    pub template: String,
    /// Role → prompt fragment for `{{role}}`.
    pub roles: BTreeMap<String, String>,
    pub avoid: Vec<String>,
    /// Other `{{name}}` values (from select parameters).
    pub vars: BTreeMap<String, String>,
    pub size: String,
    pub quality: Option<String>,
    pub transparent: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSpec {
    pub speaker: String,
    pub resource_id: String,
    pub sample_rate: u32,
    /// Percent speed change, the narration recipe's `speech_rate`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech_rate: Option<i32>,
}

/// Where a result goes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Target {
    Outline { chunk: usize },
    Voice { segment: String },
    Storyboard { segment: String },
    Image { element: String, variant: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Work {
    pub target: Target,
    pub spec: TaskSpec,
}

impl Work {
    pub fn key(&self) -> TaskKey {
        self.spec.key()
    }
}

/// Sentence units the outline works on, and their chunk ranges.
pub fn outline_units(doc: &Production) -> (Vec<Unit>, Vec<std::ops::Range<usize>>) {
    let units = split_units(&doc.script, doc.limits.clause_over());
    let chunks = chunk_units(&units, OUTLINE_CHUNK_CHARS);
    (units, chunks)
}

/// Consecutive ranges of at most `max` characters, ending at paragraph ends when
/// one lies in the second half of the chunk.
pub fn chunk_units(units: &[Unit], max: usize) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < units.len() {
        let mut acc = 0;
        let mut end = start;
        while end < units.len() && (end == start || acc + units[end].chars <= max) {
            acc += units[end].chars;
            end += 1;
        }
        if end < units.len() {
            let mut seen = 0;
            let mut best = None;
            for (i, unit) in units[start..end].iter().enumerate() {
                seen += unit.chars;
                if (unit.para || unit.hard) && seen * 2 >= acc {
                    best = Some(start + i + 1);
                }
            }
            if let Some(best) = best {
                end = best;
            }
        }
        out.push(start..end);
        start = end;
    }
    out
}

fn chat(messages: Vec<crate::tasks::ChatMessage>) -> TaskSpec {
    TaskSpec {
        request: TaskRequest::Language(LanguageRequest {
            model: String::new(),
            messages,
            temperature: 0.3,
            json: true,
        }),
    }
}

pub fn voice_spec(spec: &ProductionSpec, text: &str) -> TaskSpec {
    TaskSpec {
        request: TaskRequest::Audio(AudioRequest {
            model: String::new(),
            text: sanitize_for_speech(text).spoken,
            speaker: spec.voice.speaker.clone(),
            resource_id: spec.voice.resource_id.clone(),
            sample_rate: spec.voice.sample_rate,
            speech_rate: spec.voice.speech_rate,
        }),
    }
}

fn phrases_of(voice: &Voice) -> Vec<Phrase> {
    voice.timeline.phrases()
}

fn max_boards(spec: &ProductionSpec, phrases: &[Phrase]) -> usize {
    let total = phrases.last().map_or(0.0, |p| p.end);
    ((total / (spec.board_sec * 0.6)).ceil().max(1.0) as usize).min(crate::storyboard::MAX_BOARDS)
}

/// UTF-16 tail / head of a neighbour's text (`slice(-40)` / `slice(0, 40)`).
fn neighbour(text: &str, tail: bool) -> String {
    if !tail {
        return js_slice(text, 40);
    }
    let reversed: String = text.chars().rev().collect();
    js_slice(&reversed, 40).chars().rev().collect()
}

/// What a segment's plan depends on. Neighbour text and the film title only
/// give the model context; changing them does not replan a segment.
pub fn storyboard_inputs(spec: &ProductionSpec, doc: &Production, g: usize) -> Option<String> {
    let voice = doc.segments[g].voice.as_ref()?;
    Some(crate::tasks::fingerprint(&serde_json::json!({
        "voice": voice.task,
        "boardSec": spec.board_sec,
        "maxElements": spec.max_elements,
        "system": spec.planner_system,
        "cast": doc.cast,
    })))
}

pub fn storyboard_spec(spec: &ProductionSpec, doc: &Production, g: usize) -> Option<TaskSpec> {
    let segment = &doc.segments[g];
    let phrases = phrases_of(segment.voice.as_ref()?);
    if phrases.is_empty() {
        return None;
    }
    let previous = g
        .checked_sub(1)
        .map(|p| neighbour(&doc.segments[p].text, true))
        .unwrap_or_default();
    let next = doc
        .segments
        .get(g + 1)
        .map(|n| neighbour(&n.text, false))
        .unwrap_or_default();
    let messages = build_board_messages(
        &spec.planner_system,
        spec.max_elements,
        &phrases,
        &BoardPrompt {
            board_sec: spec.board_sec,
            max_boards: max_boards(spec, &phrases),
            cast: &doc.cast,
            title: &doc.title,
            segment_title: &segment.title,
            previous: &previous,
            next: &next,
        },
    );
    Some(chat(messages))
}

/// The image request of an element's `ordinal`-th AI candidate. The first
/// candidate of the same description is the same request everywhere.
pub fn image_spec(
    spec: &ProductionSpec,
    element: &crate::production::Element,
    ordinal: u32,
) -> TaskSpec {
    let mut vars = spec.asset.vars.clone();
    vars.insert("draw".into(), element.draw.clone());
    vars.insert("label".into(), element.label.clone());
    vars.insert(
        "role".into(),
        spec.asset
            .roles
            .get(element.role.as_str())
            .cloned()
            .unwrap_or_default(),
    );
    TaskSpec {
        request: TaskRequest::Image(ImageRequest {
            model: String::new(),
            prompt: asset_prompt(&spec.asset.template, &vars, &spec.asset.avoid),
            size: spec.asset.size.clone(),
            quality: spec.asset.quality.clone(),
            transparent: spec.asset.transparent,
            variant: ordinal,
            references: Vec::new(),
            reference_digests: Vec::new(),
        }),
    }
}

fn ai_ordinal(variants: &[Variant], index: usize) -> u32 {
    variants[..index]
        .iter()
        .filter(|v| v.source == VariantSource::Ai)
        .count() as u32
}

/// The picked candidate can be used as it is.
fn usable(spec: &ProductionSpec, element: &crate::production::Element) -> bool {
    let Some(variant) = element.picked() else {
        return false;
    };
    if variant.file.is_none() {
        return false;
    }
    let index = element.pick.min(element.variants.len() - 1);
    variant.locked
        || variant.source == VariantSource::Upload
        || variant.task.as_ref()
            == Some(&image_spec(spec, element, ai_ordinal(&element.variants, index)).key())
}

/// Document changes planning relies on: a stale AI pick (description or
/// parameters changed) gets a fresh candidate; the old one stays selectable.
pub fn refresh(doc: &mut Production, spec: &ProductionSpec) {
    for g in 0..doc.segments.len() {
        for b in 0..doc.segments[g].boards.len() {
            for e in 0..doc.segments[g].boards[b].elements.len() {
                let element = &doc.segments[g].boards[b].elements[e];
                let picked_pending = element
                    .picked()
                    .is_some_and(|v| v.source == VariantSource::Ai && v.file.is_none());
                if !usable(spec, element) && !picked_pending {
                    let variant = doc.new_variant();
                    let element = &mut doc.segments[g].boards[b].elements[e];
                    element.variants.push(variant);
                    element.pick = element.variants.len() - 1;
                }
            }
        }
    }
}

/// Every request that can run now. Results of finished tasks come back from
/// the executor's journal without cost.
pub fn plan_work(doc: &Production, spec: &ProductionSpec) -> Vec<Work> {
    let mut out = Vec::new();
    if doc.segments.is_empty() {
        if doc.split == SplitMode::Semantic {
            let (units, chunks) = outline_units(doc);
            for (chunk, range) in chunks.iter().enumerate() {
                if units.len() <= 1 {
                    break;
                }
                let title = if doc.title_locked {
                    doc.title.as_str()
                } else {
                    ""
                };
                out.push(Work {
                    target: Target::Outline { chunk },
                    spec: chat(build_segment_messages(
                        &units[range.clone()],
                        doc.limits,
                        title,
                    )),
                });
            }
        }
        return out;
    }
    for (g, segment) in doc.segments.iter().enumerate() {
        let voice = voice_spec(spec, &segment.text);
        let voice_key = voice.key();
        let voice_ready = segment.voice.as_ref().is_some_and(|v| v.task == voice_key);
        if !voice_ready {
            out.push(Work {
                target: Target::Voice {
                    segment: segment.id.clone(),
                },
                spec: voice,
            });
            continue;
        }
        let storyboard = storyboard_spec(spec, doc, g);
        let inputs = storyboard_inputs(spec, doc, g);
        let plan_ready = match (&segment.plan, &storyboard) {
            (Some(plan), Some(_)) => {
                plan.voice == voice_key && (plan.source != Source::Ai || plan.inputs == inputs)
            }
            (_, None) => true,
            (None, _) => false,
        };
        if !plan_ready {
            if let Some(storyboard) = storyboard {
                out.push(Work {
                    target: Target::Storyboard {
                        segment: segment.id.clone(),
                    },
                    spec: storyboard,
                });
            }
            continue;
        }
        for board in &segment.boards {
            for element in &board.elements {
                for (index, variant) in element.variants.iter().enumerate() {
                    let pending = variant.source == VariantSource::Ai
                        && (variant.file.is_none()
                            || (index == element.pick
                                && !usable(spec, element)
                                && !variant.locked));
                    if pending {
                        out.push(Work {
                            target: Target::Image {
                                element: element.id.clone(),
                                variant: variant.id.clone(),
                            },
                            spec: image_spec(spec, element, ai_ordinal(&element.variants, index)),
                        });
                    }
                }
            }
        }
    }
    out
}

/// First `{` to last `}` of a model reply, parsed.
pub fn extract_json(text: &str) -> Result<serde_json::Value, String> {
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else {
        return Err("AI 没有返回 JSON".into());
    };
    if end <= start {
        return Err("AI 没有返回 JSON".into());
    }
    serde_json::from_str(&text[start..=end]).map_err(|e| format!("AI 返回的 JSON 不合法：{e}"))
}

/// Split the script once every chunk has an answer (`Err` = the request
/// failed): model cut points where valid, paragraph rules elsewhere.
pub fn apply_outline(doc: &mut Production, answers: &[Result<String, String>]) {
    let (units, chunks) = outline_units(doc);
    if units.is_empty() {
        return;
    }
    let local = |range: &std::ops::Range<usize>| -> Vec<usize> {
        heuristic_starts(
            &units[range.clone()],
            doc.limits,
            doc.split != SplitMode::Length,
        )
        .into_iter()
        .map(|s| s + range.start)
        .collect()
    };
    let mut starts: Vec<i64> = Vec::new();
    let mut titles: BTreeMap<usize, String> = BTreeMap::new();
    let mut cast: Vec<CastMember> = Vec::new();
    let mut title = String::new();
    let mut failures = Vec::new();
    let semantic = doc.split == SplitMode::Semantic && units.len() > 1;
    for (chunk, range) in chunks.iter().enumerate() {
        let outline = semantic
            .then(|| {
                answers
                    .get(chunk)
                    .cloned()
                    .unwrap_or_else(|| Err("缺少分段结果".into()))
            })
            .map(|answer| {
                answer
                    .and_then(|text| extract_json(&text))
                    .and_then(|raw| sanitize_outline(&raw, &units[range.clone()], doc.limits))
            });
        match outline {
            Some(Ok(outline)) => {
                for (s, t) in outline.starts.iter().zip(&outline.titles) {
                    starts.push((s + range.start) as i64);
                    titles.insert(s + range.start, t.clone());
                }
                if title.is_empty() {
                    title = outline.title;
                }
                for member in outline.cast {
                    if cast.len() < 6 && !cast.iter().any(|c| c.name == member.name) {
                        cast.push(member);
                    }
                }
            }
            Some(Err(error)) => {
                failures.push(error);
                starts.extend(local(range).into_iter().map(|s| s as i64));
            }
            None => starts.extend(local(range).into_iter().map(|s| s as i64)),
        }
    }
    let cuts = enforce_limits(&units, &starts, doc.limits);
    let texts = segment_texts(&units, &cuts);
    doc.segments = Vec::new();
    for (start, text) in cuts.iter().zip(texts) {
        let segment = doc.new_segment(text, titles.get(start).cloned().unwrap_or_default());
        doc.segments.push(segment);
    }
    doc.cast = cast;
    if !doc.title_locked && !title.is_empty() {
        doc.title = title;
    }
    doc.outline = Some(OutlineInfo {
        source: match (semantic, failures.is_empty()) {
            (true, true) => Source::Ai,
            (true, false) => Source::Fallback,
            (false, _) => Source::User,
        },
        notes: failures
            .first()
            .map(|e| format!("AI 分段失败的部分改按段落 + 字数切：{}", js_slice(e, 80)))
            .unwrap_or_default(),
    });
    doc.script = doc
        .segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
}

/// The voice of a segment: timeline from the TTS words, corrected against the
/// audio; subtitles are cut from it at the same time.
/// A finished speech task: its audio file, raw timestamps and samples.
pub struct SpeechResult<'a> {
    pub task: TaskKey,
    pub audio: String,
    pub sentences_raw: &'a [serde_json::Value],
    pub words: &'a [Word],
    pub pcm: Pcm<'a>,
}

pub fn apply_voice(
    doc: &mut Production,
    spec: &ProductionSpec,
    segment: &str,
    speech: SpeechResult,
) -> Result<(), String> {
    let SpeechResult {
        task,
        audio,
        sentences_raw,
        words,
        pcm,
    } = speech;
    let g = doc
        .segments
        .iter()
        .position(|s| s.id == segment)
        .ok_or("口播段已不存在")?;
    let spoken = sanitize_for_speech(&doc.segments[g].text).spoken;
    let timeline = speech_timeline(&spoken, sentences_raw, words, pcm);
    if timeline.chars.is_empty() {
        return Err("配音没有返回时间戳，无法对时".into());
    }
    let subtitles = spec.subtitle_chars.map(|max| Subtitles {
        fingerprint: format!("{task}:{max}"),
        cues: timeline_cues(&spoken, &timeline, pcm, max),
    });
    let segment = &mut doc.segments[g];
    segment.voice = Some(Voice {
        task,
        audio,
        timeline,
    });
    segment.subtitles = subtitles;
    Ok(())
}

/// A storyboard answer (or failure) for a segment. Elements whose description
/// did not change keep their candidates and choice.
pub fn apply_storyboard(
    doc: &mut Production,
    spec: &ProductionSpec,
    segment: &str,
    answer: Result<String, String>,
) -> Result<(), String> {
    let g = doc
        .segments
        .iter()
        .position(|s| s.id == segment)
        .ok_or("口播段已不存在")?;
    let voice = doc.segments[g].voice.as_ref().ok_or("口播段还没有配音")?;
    let voice_task = voice.task.clone();
    let phrases = phrases_of(voice);
    let cap = max_boards(spec, &phrases);
    let parsed = answer
        .and_then(|text| extract_json(&text))
        .and_then(|raw| sanitize_plan(&raw, spec.max_elements, &phrases, cap));
    let inputs = storyboard_inputs(spec, doc, g);
    let (plan, source, notes, inputs) = match parsed {
        Ok(plan) => {
            let notes = plan.notes.clone();
            (plan, Source::Ai, notes, inputs)
        }
        Err(error) => (
            fallback_plan(spec.max_elements, &phrases)?,
            Source::Fallback,
            format!("AI 拆分镜失败，按句各画一块：{}", js_slice(&error, 80)),
            None,
        ),
    };
    let mut previous: BTreeMap<String, crate::production::Element> = BTreeMap::new();
    for board in std::mem::take(&mut doc.segments[g].boards) {
        for element in board.elements {
            previous.entry(element.draw.clone()).or_insert(element);
        }
    }
    let mut boards = Vec::new();
    for scene in plan.scenes {
        let id = doc.next_id('b');
        let mut elements = Vec::new();
        for planned in scene.elements {
            let mut element = doc.new_element(
                planned.at,
                &planned.label,
                &planned.draw,
                planned.role,
                Some(planned.weight),
                spec.variants,
            );
            if let Some(old) = previous.remove(&element.draw) {
                element.variants = old.variants;
                element.pick = old.pick;
            }
            elements.push(element);
        }
        boards.push(Board {
            id,
            start: scene.start,
            elements,
            render: None,
        });
    }
    let segment = &mut doc.segments[g];
    segment.boards = boards;
    segment.plan = Some(PlanState {
        inputs,
        voice: voice_task,
        source,
        notes,
    });
    Ok(())
}

/// A generated candidate (or its final failure).
pub fn apply_image(
    doc: &mut Production,
    element: &str,
    variant: &str,
    task: TaskKey,
    result: Result<String, String>,
) -> Result<(), String> {
    for segment in &mut doc.segments {
        for board in &mut segment.boards {
            for e in &mut board.elements {
                if e.id != element {
                    continue;
                }
                let v = e
                    .variants
                    .iter_mut()
                    .find(|v| v.id == variant)
                    .ok_or("候选已不存在")?;
                match result {
                    Ok(file) => {
                        v.file = Some(file);
                        v.task = Some(task);
                        v.error = None;
                    }
                    Err(error) => v.error = Some(js_slice(&error, 160)),
                }
                board.render = None;
                return Ok(());
            }
        }
    }
    Err("元素已不存在".into())
}
