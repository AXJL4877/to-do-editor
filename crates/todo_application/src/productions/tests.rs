//! A whole narration production through the Workspace with fake services:
//! voice first, then boards baked and placed, one save per batch, nothing
//! billed twice, user deletions respected.
use crate::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use todo_core::{create_project, MediaItem, MediaKind, Project};
use todo_workflow::workflow::protocol::Canvas;
use todo_production::pacing::Pacing;
use todo_production::production::{Edit, PackRef};
use todo_production::segment::Limits;
use todo_production::work::{AssetSpec, ProductionSpec, VoiceSpec};
use todo_production::{Production, SplitMode, TaskKey, TaskRequest};

const WORK: &str = "data/p/productions/x/work";

#[test]
fn workflow_bad_file_does_not_block_ready_sibling() {
    let mut ws = workspace();
    let run = ws.bind_canvas_workflow("test.workflow", "1.0.0", "", WORK.into(), setup().canvas).unwrap();
    let items = serde_json::from_value(json!([
        {"key":"bad", "rev":"1", "role":"visual", "lane":0, "start":0, "duration":1,
         "source":{"type":"file", "file":"a-bad.png"}},
        {"key":"good", "rev":"1", "role":"visual", "lane":0, "start":2, "duration":1,
         "source":{"type":"file", "file":"b-good.png"}}
    ])).unwrap();
    let update = ws.on_event(AppEvent::Production(ProductionOutput::RunItems { run, items }));
    let mut fake = Fake { fail_adopts: 1, ..Fake::default() };
    fake.pump(&mut ws, update);
    let placed = clips(ws.editor().unwrap().project());
    assert!(placed.iter().any(|(key, _)| key == "good"), "healthy sibling was blocked: {placed:?}");
    assert!(!placed.iter().any(|(key, _)| key == "bad"));
    let id = ws.production_ids().pop().unwrap();
    let update = ws.retry_production(&id, None);
    fake.pump(&mut ws, update);
    let placed = clips(ws.editor().unwrap().project());
    assert_eq!(placed.iter().filter(|(key, _)| key == "good").count(), 1);
    assert_eq!(placed.iter().filter(|(key, _)| key == "bad").count(), 1);
}

fn spec() -> ProductionSpec {
    ProductionSpec {
        planner_system: "plan boards".into(),
        max_elements: 4,
        board_sec: 10.0,
        variants: 1,
        asset: AssetSpec {
            template: "chalk {{role}} {{draw}}".into(),
            roles: BTreeMap::from([
                ("hero".into(), "hero".into()),
                ("support".into(), "support".into()),
                ("accent".into(), "accent".into()),
            ]),
            avoid: vec![],
            vars: BTreeMap::new(),
            size: "1024x1024".into(),
            quality: None,
            transparent: false,
        },
        voice: VoiceSpec {
            speaker: "mizai".into(),
            resource_id: "seed-tts-2.0".into(),
            sample_rate: 16000,
            speech_rate: None,
        },
        subtitle_chars: Some(12),
        pacing: Pacing::default(),
        tail: 0.5,
    }
}

fn setup() -> ProductionSetup {
    ProductionSetup {
        extension: "builtin.chalk-hand".into(),
        recipe: Recipe::Narration {
            spec: Box::new(spec()),
            bake: Some("bake_board".into()),
            plate: Some("plate".into()),
        },
        canvas: Canvas {
            width: 1920,
            height: 1080,
            fps: 30.0,
        },
        workdir: WORK.into(),
    }
}

fn doc() -> Production {
    Production::new(
        "x",
        PackRef {
            id: "builtin.chalk-hand".into(),
            version: "4.0.0".into(),
        },
        "存钱是第一步。复利让时间替你工作。\n\n投资要分散风险。不要把鸡蛋放在一个篮子里。",
        BTreeMap::from([
            ("tone".into(), json!("chalk")),
            ("plate".into(), json!("board")),
        ]),
        SplitMode::Paragraph,
        Limits {
            target: 12,
            max: 60,
        },
    )
}

/// One word per character (0.2 s), 0.5 s after sentence ends, a tone while speaking.
fn fake_voice(text: &str) -> (Value, Pcm16) {
    let rate = 16000;
    let mut t = 0.2;
    let mut words = Vec::new();
    let mut sentences = Vec::new();
    let mut current = Vec::new();
    let mut spans = Vec::new();
    for c in text.chars().filter(|c| !c.is_whitespace()) {
        let punct = "，。！？、；：".contains(c);
        let d = if punct { 0.1 } else { 0.2 };
        if !punct {
            spans.push((t, t + d));
        }
        words.push(json!({"word": c.to_string(), "start": t, "end": t + d}));
        current.push(json!({"word": c.to_string(), "startTime": t, "endTime": t + d}));
        t += d;
        if "。！？".contains(c) {
            sentences.push(json!({"words": std::mem::take(&mut current)}));
            t += 0.5;
        }
    }
    let mut samples = vec![0i16; ((t + 0.3) * rate as f64) as usize];
    for (a, b) in spans {
        for (i, sample) in samples
            .iter_mut()
            .enumerate()
            .take((b * rate as f64) as usize)
            .skip((a * rate as f64) as usize)
        {
            *sample = ((i as f64 * 0.09).sin() * 8000.0) as i16;
        }
    }
    (
        json!({"words": words, "sentences": sentences}),
        Pcm16 { samples, rate },
    )
}

#[derive(Default)]
struct Fake {
    submitted: Vec<TaskKey>,
    runs: Vec<(String, Value)>,
    saves: usize,
    production_saves: usize,
    /// Hold project saves on "disk" until released (a slow save).
    hold_saves: bool,
    held: Vec<AppEvent>,
    last_saved: Option<Project>,
    /// Pending answers, delivered one per pump step.
    queue: Vec<AppEvent>,
    /// Next adopt tickets fail this many times (then succeed).
    fail_adopts: usize,
}

impl Fake {
    fn answer(&mut self, effect: Effect) {
        match effect {
            Effect::Production(ProductionTask::Submit { spec, .. }) => {
                let key = spec.key();
                self.submitted.push(key.clone());
                let output = match &spec.request {
                    TaskRequest::Audio(speech) => {
                        let (output, pcm) = fake_voice(&speech.text);
                        ProductionOutput::TaskDone {
                            file: Some(format!("services/{}.wav", &key.0[..8])),
                            output: Some(output),
                            pcm: Some(pcm),
                            key,
                        }
                    }
                    TaskRequest::Language(_) => ProductionOutput::TaskDone {
                        output: Some(json!({"text": json!({
                            "notes": "",
                            "scenes": [
                                {"start": 0, "elements": [
                                    {"at": 0, "label": "存钱罐", "draw": "a piggy bank", "role": "hero"},
                                    {"at": 1, "label": "时钟", "draw": "a clock"}]},
                                {"start": 2, "elements": [{"at": 2, "label": "复利", "draw": "a rising curve", "role": "accent"}]}
                            ]}).to_string()})),
                        file: None,
                        pcm: None,
                        key,
                    },
                    TaskRequest::Image(_) => ProductionOutput::TaskDone {
                        file: Some(format!("services/{}.png", &key.0[..8])),
                        output: None,
                        pcm: None,
                        key,
                    },
                    TaskRequest::Video(_) | TaskRequest::Multimodal(_) => {
                        ProductionOutput::TaskDone {
                            output: None,
                            file: None,
                            pcm: None,
                            key,
                        }
                    }
                };
                self.queue.push(AppEvent::Production(output));
            }
            Effect::Production(ProductionTask::Run { run, op, input, .. }) => {
                self.runs.push((op.clone(), input.clone()));
                let items: Vec<Value> = if op == "plate" {
                    vec![
                        json!({"key": "plate", "rev": "p", "role": "visual", "lane": 0, "start": 0, "duration": 1,
                        "source": {"type": "file", "file": "out/plate.png"}}),
                    ]
                } else {
                    input["elements"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|e| json!({"key": e["key"], "rev": "r1", "role": "visual", "lane": e["lane"],
                            "start": e["at"], "duration": 1,
                            "source": {"type": "file", "file": format!("out/{}.reveal.png", e["id"].as_str().unwrap())}}))
                        .collect()
                };
                let items = serde_json::from_value(Value::Array(items)).unwrap();
                self.queue
                    .push(AppEvent::Production(ProductionOutput::RunItems {
                        run,
                        items,
                    }));
                self.queue
                    .push(AppEvent::Production(ProductionOutput::RunDone {
                        run,
                        result: json!({}),
                    }));
            }
            Effect::Media {
                ticket,
                task: MediaTask::Adopt { files, .. },
            } => {
                if self.fail_adopts > 0 {
                    self.fail_adopts -= 1;
                    self.queue.push(AppEvent::Media {
                        ticket,
                        result: Err("file missing".into()),
                    });
                    return;
                }
                let media = files
                    .iter()
                    .map(|f| {
                        let path = f.to_string_lossy().replace('\\', "/");
                        let audio = path.ends_with(".wav");
                        MediaItem {
                            id: format!("m:{path}"),
                            name: path.clone(),
                            path: Some(path),
                            duration: if audio { 60.0 } else { 5.0 },
                            has_audio: Some(audio),
                            width: (!audio).then_some(1024),
                            height: (!audio).then_some(1024),
                            kind: Some(if audio {
                                MediaKind::Audio
                            } else {
                                MediaKind::Image
                            }),
                            alpha: None,
                            reveal: None,
                            folder_id: None,
                        }
                    })
                    .collect();
                self.queue.push(AppEvent::Media {
                    ticket,
                    result: Ok(MediaOutput::Adopted(media)),
                });
            }
            Effect::Project {
                ticket,
                task: ProjectTask::Save { project, .. },
            } => {
                self.saves += 1;
                self.last_saved = Some(project.clone());
                let event = AppEvent::Project {
                    ticket,
                    result: Ok(ProjectOutput::Saved(project)),
                };
                if self.hold_saves {
                    self.held.push(event);
                } else {
                    self.queue.push(event);
                }
            }
            Effect::Production(ProductionTask::Save { .. }) => self.production_saves += 1,
            _ => {}
        }
    }

    fn pump(&mut self, ws: &mut Workspace, update: AppUpdate) {
        for effect in update.effects {
            self.answer(effect);
        }
        let mut steps = 0;
        while let Some(event) = (!self.queue.is_empty()).then(|| self.queue.remove(0)) {
            steps += 1;
            assert!(steps < 2000, "production does not settle");
            let update = ws.on_event(event);
            for effect in update.effects {
                self.answer(effect);
            }
        }
    }
}

fn clips(project: &Project) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(move |c| (t.id.clone(), c)))
        .filter_map(|(track, c)| c.origin.as_ref().map(|o| (o.key.clone(), track)))
        .collect();
    out.sort();
    out
}

fn workspace() -> Workspace {
    Workspace::with_project(create_project(None, None, None, None, None), "p")
}

#[test]
fn a_narration_runs_to_the_timeline_progressively() {
    let mut ws = workspace();
    let mut fake = Fake::default();
    let update = ws.add_production(doc(), setup());
    fake.pump(&mut ws, update);
    let project = ws.editor().unwrap().project().clone();
    let placed = clips(&project);
    let keys: Vec<&str> = placed.iter().map(|(k, _)| k.as_str()).collect();
    assert!(keys.contains(&"plate"), "{keys:?}");
    assert!(
        keys.iter().filter(|k| k.ends_with("/voice")).count() == 2,
        "{keys:?}"
    );
    assert!(keys.iter().any(|k| k.contains("/sub/")), "{keys:?}");
    // 2 segments × (2 + 1) elements.
    let elements = keys.iter().filter(|k| k.starts_with('b')).count();
    assert_eq!(elements, 6, "{keys:?}");
    // Every request went out once: 2 voices + 2 storyboards + 3 images (both
    // segments draw the same three elements, which is one request each).
    let mut unique = fake.submitted.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), fake.submitted.len());
    assert_eq!(fake.submitted.len(), 7);
    let boards: usize = ws
        .production("x")
        .unwrap()
        .segments
        .iter()
        .map(|s| s.boards.len())
        .sum();
    assert_eq!(
        fake.runs
            .iter()
            .filter(|(op, _)| op == "bake_board")
            .count(),
        boards
    );
    assert_eq!(fake.runs.iter().filter(|(op, _)| op == "plate").count(), 1);
    assert!(fake.saves >= 1 && fake.production_saves >= 1);
    // Generated files get readable library names, not content hashes.
    let names: Vec<&str> = project.media.iter().map(|m| m.name.as_str()).collect();
    assert!(names.contains(&"黑板底"), "{names:?}");
    assert!(names.contains(&"第 1 段配音"), "{names:?}");
    assert!(names.contains(&"画面 · 存钱罐"), "{names:?}");
    // Voice clips are audio, the plate is the bottom video lane.
    let status = ws.production_status();
    assert_eq!(status[0].done, status[0].total, "{:#?}", status[0].nodes);
    let kinds: Vec<StageKind> = status[0].stages.iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        [
            StageKind::Outline,
            StageKind::Voice,
            StageKind::Subtitles,
            StageKind::Storyboard,
            StageKind::Images,
            StageKind::Compose,
            StageKind::Timeline
        ]
    );
    assert!(
        status[0]
            .stages
            .iter()
            .all(|s| s.state == StatusState::Done && s.done == s.total && s.total > 0),
        "{:#?}",
        status[0].stages
    );
    let track_index = |key: &str| {
        let track = &placed.iter().find(|(k, _)| k == key).unwrap().1;
        project.tracks.iter().position(|t| &t.id == track).unwrap()
    };
    let element = placed
        .iter()
        .find(|(k, _)| k.starts_with('b'))
        .unwrap()
        .0
        .clone();
    let layout: Vec<(String, Vec<String>)> = project
        .tracks
        .iter()
        .map(|t| {
            (
                t.id.clone(),
                t.clips
                    .iter()
                    .filter_map(|c| c.origin.as_ref().map(|o| o.key.clone()))
                    .collect(),
            )
        })
        .collect();
    assert!(track_index(&element) < track_index("plate"), "{layout:#?}");
}

#[test]
fn deleting_a_generated_clip_sticks_and_edits_rebuild_only_their_part() {
    let mut ws = workspace();
    let mut fake = Fake::default();
    let update = ws.add_production(doc(), setup());
    fake.pump(&mut ws, update);
    let project = ws.editor().unwrap().project().clone();
    let plate = project
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find(|c| c.origin.as_ref().is_some_and(|o| o.key == "plate"))
        .unwrap()
        .id
        .clone();
    let update = ws.dispatch(AppAction::Editor(EditorCommand::Edit(
        todo_core::EditCommand::DeleteClips {
            clip_ids: vec![plate],
            ripple: false,
        },
    )));
    fake.pump(&mut ws, update);
    // Change one element's description: one new image, one new bake.
    let element = ws.production("x").unwrap().segments[0].boards[0].elements[0]
        .id
        .clone();
    let before_images = fake.submitted.len();
    let before_bakes = fake.runs.len();
    let update = ws.edit_production(
        "x",
        &[Edit::SetElement {
            element,
            patch: todo_production::production::ElementPatch {
                draw: Some("a golden piggy bank".into()),
                ..Default::default()
            },
        }],
    );
    fake.pump(&mut ws, update);
    assert_eq!(fake.submitted.len(), before_images + 1);
    assert_eq!(fake.runs.len(), before_bakes + 1);
    let keys = clips(ws.editor().unwrap().project());
    assert!(
        keys.iter().all(|(k, _)| k != "plate"),
        "a deleted clip is not put back"
    );
    assert_eq!(ws.production_status()[0].deleted, 1);
    // Until the user asks for it.
    let update = ws.restore_production("x");
    fake.pump(&mut ws, update);
    let keys = clips(ws.editor().unwrap().project());
    assert!(keys.iter().any(|(k, _)| k == "plate"), "{keys:?}");
    assert_eq!(ws.production_status()[0].deleted, 0);
}

#[test]
fn clips_placed_during_a_slow_save_are_saved_after_it() {
    let mut ws = workspace();
    let mut fake = Fake {
        hold_saves: true,
        ..Fake::default()
    };
    let update = ws.add_production(doc(), setup());
    fake.pump(&mut ws, update);
    // Release saves one at a time, as a slow disk would, until none is left.
    let mut rounds = 0;
    while !fake.held.is_empty() {
        rounds += 1;
        assert!(rounds < 100);
        let event = fake.held.remove(0);
        let update = ws.on_event(event);
        fake.pump(&mut ws, update);
    }
    let saved = fake.last_saved.clone().expect("saved");
    assert_eq!(
        clips(&saved),
        clips(ws.editor().unwrap().project()),
        "the last save holds every generated clip"
    );
    assert!(!ws.editor().unwrap().dirty());
}

#[test]
fn failed_requests_wait_for_the_user_and_retry_once() {
    let mut ws = workspace();
    let update = ws.add_production(doc(), setup());
    // Fail the first voice; answer nothing else.
    let key = update
        .effects
        .iter()
        .find_map(|e| match e {
            Effect::Production(ProductionTask::Submit { spec, .. }) => Some(spec.key()),
            _ => None,
        })
        .unwrap();
    let update = ws.on_event(AppEvent::Production(ProductionOutput::TaskFailed {
        key: key.clone(),
        class: "ambiguous".into(),
        message: "timeout".into(),
        possibly_billed: true,
    }));
    assert!(
        !update
            .effects
            .iter()
            .any(|e| matches!(e, Effect::Production(ProductionTask::Submit { spec, .. }) if spec.key() == key)),
        "possibly billed requests are not resubmitted"
    );
    let status = &ws.production_status()[0];
    assert!(status
        .failures
        .iter()
        .any(|f| f.key == key && f.possibly_billed && f.work == FailedWork::Voice { segment: 0 }));
    let voice = status
        .stages
        .iter()
        .find(|s| s.kind == StageKind::Voice)
        .unwrap();
    assert_eq!(voice.state, StatusState::Failed);
    let update = ws.retry_production("x", Some(key.clone()));
    assert!(update
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Production(ProductionTask::Retry(k)) if k == &key)));
}

#[test]
fn commit_edit_resumes_placement_paused_during_gesture() {
    let mut ws = workspace();
    let mut fake = Fake::default();
    let _ = ws.dispatch(AppAction::Editor(EditorCommand::BeginEdit));
    assert!(ws.editor().unwrap().transaction_active());
    let update = ws.add_production(doc(), setup());
    fake.pump(&mut ws, update);
    assert!(
        clips(ws.editor().unwrap().project()).is_empty(),
        "placement must wait out an open gesture"
    );
    let update = ws.dispatch(AppAction::Editor(EditorCommand::CommitEdit));
    fake.pump(&mut ws, update);
    let placed = clips(ws.editor().unwrap().project());
    assert!(
        !placed.is_empty(),
        "CommitEdit must resume placement: {placed:?}"
    );
}

#[test]
fn failed_adopt_can_retry_and_still_reach_timeline() {
    let mut ws = workspace();
    let mut fake = Fake {
        // Fail the first wave of adopts; later waves succeed after retry.
        fail_adopts: 1,
        ..Fake::default()
    };
    let update = ws.add_production(doc(), setup());
    fake.pump(&mut ws, update);
    let after_fail = ws
        .production_status()
        .into_iter()
        .find(|s| s.id == "x")
        .unwrap();
    assert!(
        after_fail
            .attention
            .iter()
            .any(|a| a.starts_with("入轨未成功")),
        "adopt failure must be readable on the session: {:?}",
        after_fail.attention
    );
    assert!(
        after_fail
            .stages
            .iter()
            .any(|s| s.kind == crate::productions::StageKind::Timeline
                && s.state == StatusState::Failed),
        "timeline must not stay Running after adopt fail: {:?}",
        after_fail.stages
    );
    // First adopt failed → nothing file-based on the timeline yet (or only
    // text if any). Retry clears unadoptable and re-queues registration.
    let update = ws.retry_production("x", None);
    fake.pump(&mut ws, update);
    let placed = clips(ws.editor().unwrap().project());
    assert!(
        placed.iter().any(|(k, _)| k == "plate"),
        "retry after adopt failure must place: {placed:?}"
    );
}

/// Offline main path: canvas workflow emit → adopt → Place on timeline.
#[test]
fn workflow_emit_places_on_timeline_offline() {
    let mut ws = workspace();
    let run = ws
        .bind_canvas_workflow("test.workflow", "1.0.0", "hello", WORK.into(), setup().canvas)
        .unwrap();
    let items = serde_json::from_value(json!([
        {"key":"visual/s1", "rev":"v1", "role":"visual", "lane":0, "start":0, "duration":2,
         "source":{"type":"file", "file":"out/s1.png"}}
    ]))
    .unwrap();
    let update = ws.on_event(AppEvent::Production(ProductionOutput::RunItems { run, items }));
    let mut fake = Fake::default();
    fake.pump(&mut ws, update);
    let placed = clips(ws.editor().unwrap().project());
    assert!(
        placed.iter().any(|(k, _)| k == "visual/s1"),
        "emit must place on timeline: {placed:?}"
    );
    let status = &ws.production_status()[0];
    assert!(
        status.attention.iter().all(|a| !a.starts_with("入轨未成功")),
        "clean place must not leave placement attention: {:?}",
        status.attention
    );
}

/// B7: ApplyGenerated rejection is observable via attention; retry clears it.
#[test]
fn placement_reject_surfaces_attention_and_clears_on_retry() {
    use crate::control::{ControlCommand, ControlRequest, ControlResult, ControlStatus};
    use todo_core::EditCommand;

    let mut ws = workspace();
    let mut fake = Fake::default();
    let run = ws
        .bind_canvas_workflow("test.workflow", "1.0.0", "", WORK.into(), setup().canvas)
        .unwrap();
    let pid = ws.production_ids().pop().unwrap();
    let items = serde_json::from_value(json!([
        {"key":"visual/s1", "rev":"v1", "role":"visual", "lane":0, "start":0, "duration":2,
         "source":{"type":"file", "file":"out/s1.png"}}
    ]))
    .unwrap();
    let update = ws.on_event(AppEvent::Production(ProductionOutput::RunItems { run, items }));
    fake.pump(&mut ws, update);
    assert!(
        clips(ws.editor().unwrap().project())
            .iter()
            .any(|(k, _)| k == "visual/s1")
    );
    // Lock every track so same-key Update fails ApplyGenerated.
    let track_ids: Vec<String> = ws
        .editor()
        .unwrap()
        .project()
        .tracks
        .iter()
        .map(|t| t.id.clone())
        .collect();
    for track_id in &track_ids {
        let update = ws.dispatch(AppAction::Editor(EditorCommand::Edit(
            EditCommand::ToggleLock {
                track_id: track_id.clone(),
            },
        )));
        fake.pump(&mut ws, update);
    }
    // Same canvas run id from bind (do not poke private Workspace.productions).
    let items = serde_json::from_value(json!([
        {"key":"visual/s1", "rev":"v2", "role":"visual", "lane":0, "start":0, "duration":2,
         "source":{"type":"file", "file":"out/s1b.png"}}
    ]))
    .unwrap();
    let update = ws.on_event(AppEvent::Production(ProductionOutput::RunItems { run, items }));
    fake.pump(&mut ws, update);
    let status = ws
        .production_status()
        .into_iter()
        .find(|s| s.id == pid)
        .unwrap();
    assert!(
        status
            .attention
            .iter()
            .any(|a| a.starts_with("入轨未成功")),
        "rejected placement must surface attention: {:?}",
        status.attention
    );
    let session_id = ws.control_context().unwrap().session_id;
    let outcome = ws.control(ControlRequest {
        request_id: "b7".into(),
        session_id,
        expected_revision: None,
        command: ControlCommand::GetProductionContext {
            production_id: Some(pid.clone()),
        },
    });
    let blocked = match outcome.response.outcome {
        ControlStatus::Ok {
            result: ControlResult::ProductionContext { context },
        } => context.productions.iter().any(|p| {
            p.placement.placement_blocked
                || p.attention.iter().any(|a| a.starts_with("入轨未成功"))
        }),
        other => panic!("unexpected {other:?}"),
    };
    assert!(blocked, "AI get_production_context must see placement pause");
    for track_id in track_ids {
        let update = ws.dispatch(AppAction::Editor(EditorCommand::Edit(
            EditCommand::ToggleLock { track_id },
        )));
        fake.pump(&mut ws, update);
    }
    let update = ws.retry_production(&pid, None);
    fake.pump(&mut ws, update);
    let after = ws
        .production_status()
        .into_iter()
        .find(|s| s.id == pid)
        .unwrap();
    assert!(
        after
            .attention
            .iter()
            .all(|a| !a.starts_with("入轨未成功")),
        "retry must clear placement attention: {:?}",
        after.attention
    );
}
