use super::{
    LocalRunId, Pcm16, ProductionOutput, ProductionSetup, ProductionTask, Recipe, TaskState,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use todo_core::Project;
use todo_production::items::Item;
use todo_production::narration::{self, NarrationLayout};
use todo_production::placement::{self, PlacementInput, PlacementPlan};
use todo_production::production::{Edit, Rendered, WorkflowRecord};
use todo_production::voice::{Pcm, Word};
use todo_production::work::{self, SpeechResult, Target};
use todo_production::{tasks::fingerprint, Production, SplitMode, TaskKey};

/// Most requests a production keeps in flight; the AI service queues and
/// rate-limits behind this, the cap only bounds memory and journal churn.
const MAX_IN_FLIGHT: usize = 64;

#[derive(Debug, Clone, PartialEq)]
struct Failure {
    class: String,
    message: String,
    possibly_billed: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum RunPurpose {
    Bake { board: String, fingerprint: String },
    Plate { fingerprint: String },
    Workflow { fingerprint: String },
}

#[derive(Debug, Clone, PartialEq, Default)]
struct RunBuffer {
    items: Vec<Item>,
    phase: String,
    fraction: f64,
}

/// One production of the open project.
#[derive(Debug, Clone)]
pub struct ProductionSession {
    doc: Production,
    setup: ProductionSetup,
    project: String,
    /// One request may serve several targets (identical content).
    in_flight: BTreeMap<TaskKey, Vec<Target>>,
    states: BTreeMap<TaskKey, TaskState>,
    failed: BTreeMap<TaskKey, Failure>,
    outline: BTreeMap<usize, Result<String, String>>,
    runs: BTreeMap<LocalRunId, (RunPurpose, RunBuffer)>,
    /// Bakes that failed for an input fingerprint: not retried automatically.
    run_failures: BTreeMap<String, String>,
    attention: Vec<String>,
    dirty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusState {
    Queued,
    Running,
    Waiting,
    Attention,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatusNode {
    pub key: String,
    pub label: String,
    pub state: StatusState,
    pub detail: Option<String>,
    pub children: Vec<StatusNode>,
}

/// The pipeline steps a production goes through, in order. Not every
/// recipe has every step (a workflow only has `Generate` and `Timeline`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StageKind {
    /// Split the script into segments.
    Outline,
    /// One voice-over per segment.
    Voice,
    /// Subtitle cues from the voice timing.
    Subtitles,
    /// Boards and elements planned per segment.
    Storyboard,
    /// One picture per element.
    Images,
    /// Elements turned into animated layers (bake, background plate).
    Compose,
    /// An extension's own run (workflow recipes).
    Generate,
    /// Results put on the timeline.
    Timeline,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StageStatus {
    pub kind: StageKind,
    pub done: usize,
    pub total: usize,
    pub state: StatusState,
    /// What a running extension reports it is doing, and how far (0..=1).
    pub phase: Option<(String, f64)>,
}

/// What a failed request was for, so the panel can name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailedWork {
    Outline,
    Voice { segment: usize },
    Storyboard { segment: usize },
    Image { label: String },
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FailureStatus {
    pub key: TaskKey,
    pub work: FailedWork,
    /// Error class from the service (`auth`, `billing`, `network`, …).
    pub class: String,
    pub message: String,
    pub possibly_billed: bool,
}

/// What the production panel shows.
#[derive(Debug, Clone, PartialEq)]
pub struct ProductionStatus {
    pub id: String,
    pub title: String,
    pub extension: String,
    /// The start of the script, for productions without a title yet.
    pub script: String,
    pub done: usize,
    pub total: usize,
    pub stages: Vec<StageStatus>,
    pub nodes: Vec<StatusNode>,
    pub attention: Vec<String>,
    /// Failed requests the user may retry.
    pub failures: Vec<FailureStatus>,
    /// Failed bakes / runs (retried together).
    pub run_failures: usize,
    /// Generated clips the user deleted from the timeline.
    pub deleted: usize,
}

/// Fold member states into one stage state: a failure needs the user,
/// then whatever is still moving, and done only when everything is.
fn fold_states(states: &[StatusState]) -> StatusState {
    let any = |s: StatusState| states.contains(&s);
    if any(StatusState::Failed) {
        StatusState::Failed
    } else if !states.is_empty() && states.iter().all(|s| *s == StatusState::Done) {
        StatusState::Done
    } else if any(StatusState::Waiting) {
        StatusState::Waiting
    } else if any(StatusState::Attention) {
        StatusState::Attention
    } else if any(StatusState::Running) {
        StatusState::Running
    } else {
        StatusState::Queued
    }
}

fn stage(kind: StageKind, states: &[StatusState]) -> StageStatus {
    StageStatus {
        kind,
        done: states.iter().filter(|s| **s == StatusState::Done).count(),
        total: states.len(),
        state: fold_states(states),
        phase: None,
    }
}

fn seed_of(id: &str) -> u64 {
    let hex = fingerprint(&id);
    u64::from_str_radix(&hex[..16], 16).unwrap_or(0)
}

impl ProductionSession {
    pub fn new(project: impl Into<String>, doc: Production, setup: ProductionSetup) -> Self {
        Self {
            doc,
            setup,
            project: project.into(),
            in_flight: BTreeMap::new(),
            states: BTreeMap::new(),
            failed: BTreeMap::new(),
            outline: BTreeMap::new(),
            runs: BTreeMap::new(),
            run_failures: BTreeMap::new(),
            attention: Vec::new(),
            dirty: false,
        }
    }

    pub fn id(&self) -> &str {
        &self.doc.id
    }

    pub fn doc(&self) -> &Production {
        &self.doc
    }

    pub fn setup(&self) -> &ProductionSetup {
        &self.setup
    }

    pub fn owns_key(&self, key: &TaskKey) -> bool {
        self.in_flight.contains_key(key)
            || self.failed.contains_key(key)
            || self.states.contains_key(key)
    }

    pub fn owns_run(&self, run: LocalRunId) -> bool {
        self.runs.contains_key(&run)
    }

    pub fn runs(&self) -> impl Iterator<Item = LocalRunId> + '_ {
        self.runs.keys().copied()
    }

    pub fn set_workdir(&mut self, workdir: String) {
        if !self.setup.workdir.is_empty() && self.setup.workdir != workdir {
            let current = self.setup.workdir.as_str();
            let keep_production = is_production_workdir(current) && is_cache_run_workdir(&workdir);
            if keep_production {
                // Resume must not point placement at a fresh cache/workflow-runs
                // tree — relative media paths under productions/.../work would break.
                let line = format!(
                    "续跑已复用原工作目录 {current}（忽略 cache run {workdir}）"
                );
                if !self.attention.contains(&line) {
                    self.attention.push(line);
                }
                self.dirty = true;
                return;
            }
            let line = format!(
                "工作目录已切换（{current} → {workdir}）。已入轨相对路径可能失效；续跑应复用原 production workdir。"
            );
            if !self.attention.contains(&line) {
                self.attention.push(line);
            }
        }
        self.setup.workdir = workdir;
        self.dirty = true;
    }

    pub fn set_canvas(&mut self, canvas: todo_workflow::workflow::protocol::Canvas) {
        self.setup.canvas = canvas;
        self.dirty = true;
    }

    pub fn set_workflow_script(&mut self, script: &str) {
        self.doc.script = script.to_owned();
        if let Recipe::Workflow { input, .. } = &mut self.setup.recipe {
            *input = serde_json::json!({ "script": script });
        }
        self.dirty = true;
    }

    /// Register a canvas-driven graph run (no worker pool). Items arrive via
    /// [`ProductionOutput::RunItems`] with this id.
    pub fn attach_canvas_run(
        &mut self,
        next_run: &mut LocalRunId,
    ) -> LocalRunId {
        self.runs
            .retain(|_, (purpose, _)| !matches!(purpose, RunPurpose::Workflow { .. }));
        let fp = self.workflow_fingerprint(match &self.setup.recipe {
            Recipe::Workflow { input, .. } => input,
            _ => &serde_json::Value::Null,
        });
        let kept = self
            .doc
            .workflow
            .take()
            .map(|r| r.items)
            .unwrap_or_default();
        self.doc.workflow = Some(WorkflowRecord {
            fingerprint: fp.clone(),
            items: kept,
            result: None,
            error: None,
        });
        *next_run += 1;
        let run = *next_run;
        self.runs.insert(
            run,
            (
                RunPurpose::Workflow {
                    fingerprint: fp,
                },
                RunBuffer::default(),
            ),
        );
        self.dirty = true;
        run
    }

    fn spec(&self) -> Option<&work::ProductionSpec> {
        match &self.setup.recipe {
            Recipe::Narration { spec, .. } => Some(spec),
            Recipe::Workflow { .. } => None,
        }
    }

    fn layout(&self) -> NarrationLayout {
        NarrationLayout::default()
    }

    /// Start whatever can run now; returns the tasks for the adapter.
    pub fn tick(&mut self, next_run: &mut LocalRunId) -> Vec<ProductionTask> {
        let mut tasks = match self.setup.recipe.clone() {
            Recipe::Narration { spec, bake, plate } => {
                let mut tasks = self.tick_requests(&spec);
                tasks.extend(self.tick_bakes(&spec, bake.as_deref(), plate.as_deref(), next_run));
                tasks
            }
            Recipe::Workflow { op, input } => self.tick_workflow(&op, &input, next_run),
        };
        if std::mem::take(&mut self.dirty) {
            self.doc.rev += 1;
            tasks.push(ProductionTask::Save {
                project: self.project.clone(),
                doc: Box::new(self.doc.clone()),
            });
        }
        tasks
    }

    fn tick_requests(&mut self, spec: &work::ProductionSpec) -> Vec<ProductionTask> {
        let mut tasks = Vec::new();
        if self.doc.segments.is_empty() {
            let outline: Vec<work::Work> = work::plan_work(&self.doc, spec);
            let answered = outline
                .iter()
                .enumerate()
                .all(|(chunk, _)| self.outline.contains_key(&chunk));
            if self.doc.split != SplitMode::Semantic || outline.is_empty() || answered {
                let answers: Vec<Result<String, String>> = (0..outline.len())
                    .map(|c| self.outline[&c].clone())
                    .collect();
                work::apply_outline(&mut self.doc, &answers);
                self.outline.clear();
                self.dirty = true;
            } else {
                for w in outline {
                    if let Target::Outline { chunk } = w.target {
                        if self.outline.contains_key(&chunk) {
                            continue;
                        }
                        if let Some(task) = self.submit(w.target.clone(), w.spec) {
                            tasks.push(task);
                        }
                    }
                }
                return tasks;
            }
        }
        let before = self.doc.clone();
        work::refresh(&mut self.doc, spec);
        if self.doc != before {
            self.dirty = true;
        }
        for w in work::plan_work(&self.doc, spec) {
            if self.in_flight.len() >= MAX_IN_FLIGHT {
                break;
            }
            if let Some(task) = self.submit(w.target.clone(), w.spec) {
                tasks.push(task);
            }
        }
        tasks
    }

    fn submit(
        &mut self,
        target: Target,
        spec: todo_production::TaskSpec,
    ) -> Option<ProductionTask> {
        let key = spec.key();
        if self.failed.contains_key(&key) {
            return None;
        }
        if let Some(targets) = self.in_flight.get_mut(&key) {
            if !targets.contains(&target) {
                targets.push(target);
            }
            return None;
        }
        self.in_flight.insert(key, vec![target]);
        Some(ProductionTask::Submit {
            workdir: self.setup.workdir.clone(),
            spec,
        })
    }

    fn running(&self, purpose: &RunPurpose) -> bool {
        self.runs.values().any(|(p, _)| p == purpose)
    }

    fn start_run(
        &mut self,
        purpose: RunPurpose,
        op: &str,
        input: Value,
        next_run: &mut LocalRunId,
    ) -> ProductionTask {
        *next_run += 1;
        let run = *next_run;
        self.runs.insert(run, (purpose, RunBuffer::default()));
        ProductionTask::Run {
            run,
            extension: self.setup.extension.clone(),
            op: op.into(),
            input,
            params: json!(self.doc.params),
            canvas: self.setup.canvas.clone(),
            workdir: self.setup.workdir.clone(),
            seed: seed_of(&self.doc.id),
        }
    }

    fn tick_bakes(
        &mut self,
        spec: &work::ProductionSpec,
        bake: Option<&str>,
        plate: Option<&str>,
        next_run: &mut LocalRunId,
    ) -> Vec<ProductionTask> {
        let mut tasks = Vec::new();
        if let (Some(op), Some((fp, input))) = (
            plate,
            narration::plate_request(&self.doc, &self.setup.canvas),
        ) {
            let done = self.doc.plate.as_ref().is_some_and(|p| p.fingerprint == fp);
            let purpose = RunPurpose::Plate {
                fingerprint: fp.clone(),
            };
            if !done && !self.running(&purpose) && !self.run_failures.contains_key(&fp) {
                tasks.push(self.start_run(purpose, op, input, next_run));
            }
        }
        let Some(op) = bake else { return tasks };
        for request in narration::bake_requests(&self.doc, spec, &self.layout(), &self.setup.canvas)
        {
            let current = self
                .doc
                .segments
                .iter()
                .flat_map(|s| &s.boards)
                .find(|b| b.id == request.board)
                .and_then(|b| b.render.as_ref())
                .is_some_and(|r| r.fingerprint == request.fingerprint);
            let purpose = RunPurpose::Bake {
                board: request.board.clone(),
                fingerprint: request.fingerprint.clone(),
            };
            if current
                || self.running(&purpose)
                || self.run_failures.contains_key(&request.fingerprint)
            {
                continue;
            }
            // A newer input supersedes a bake of the same board in flight.
            let stale: Vec<LocalRunId> = self
                .runs
                .iter()
                .filter(|(_, (p, _))| matches!(p, RunPurpose::Bake { board, .. } if *board == request.board))
                .map(|(run, _)| *run)
                .collect();
            for run in stale {
                self.runs.remove(&run);
                tasks.push(ProductionTask::CancelRun(run));
            }
            tasks.push(self.start_run(purpose, op, request.input, next_run));
        }
        tasks
    }

    fn workflow_fingerprint(&self, input: &Value) -> String {
        fingerprint(&json!({
            "extension": self.setup.extension,
            "input": input,
            "params": self.doc.params,
            "canvas": self.setup.canvas,
        }))
    }

    fn tick_workflow(
        &mut self,
        op: &str,
        input: &Value,
        next_run: &mut LocalRunId,
    ) -> Vec<ProductionTask> {
        let fp = self.workflow_fingerprint(input);
        let record = self.doc.workflow.as_ref();
        let finished = record
            .is_some_and(|r| r.fingerprint == fp && (r.result.is_some() || r.error.is_some()));
        let purpose = RunPurpose::Workflow {
            fingerprint: fp.clone(),
        };
        if finished || self.running(&purpose) {
            return Vec::new();
        }
        let mut tasks: Vec<ProductionTask> = self
            .runs
            .keys()
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .map(|run| {
                self.runs.remove(&run);
                ProductionTask::CancelRun(run)
            })
            .collect();
        // Keep the items of the previous run on the timeline until the new
        // run replaces them.
        let items = self
            .doc
            .workflow
            .take()
            .map(|r| r.items)
            .unwrap_or_default();
        self.doc.workflow = Some(WorkflowRecord {
            fingerprint: fp.clone(),
            items,
            result: None,
            error: None,
        });
        self.dirty = true;
        let mut run_input = input.clone();
        if let Some(object) = run_input.as_object_mut() {
            object.insert("title".into(), json!(self.doc.title));
        }
        tasks.push(self.start_run(purpose, op, run_input, next_run));
        tasks
    }

    /// Apply one adapter report. Returns true when the production changed.
    pub fn on_output(&mut self, output: ProductionOutput) -> bool {
        match output {
            ProductionOutput::TaskState { key, state } => {
                if !self.in_flight.contains_key(&key) {
                    return false;
                }
                self.states.insert(key, state);
                true
            }
            ProductionOutput::TaskDone {
                key,
                output,
                file,
                pcm,
            } => self.task_done(key, output, file, pcm),
            ProductionOutput::TaskFailed {
                key,
                class,
                message,
                possibly_billed,
            } => self.task_failed(key, class, message, possibly_billed),
            ProductionOutput::RunProgress {
                run,
                phase,
                fraction,
            } => {
                let Some((_, buffer)) = self.runs.get_mut(&run) else {
                    return false;
                };
                buffer.phase = phase;
                buffer.fraction = fraction;
                true
            }
            ProductionOutput::RunItems { run, items } => {
                let Some((purpose, buffer)) = self.runs.get_mut(&run) else {
                    return false;
                };
                if let RunPurpose::Workflow { .. } = purpose {
                    // Same-rev deletes stay deleted (production-pipeline §4).
                    // A new rev may place again; explicit "放回" uses
                    // restore_production. Do not clear tombstones on emit.
                    // Workflow items go on the timeline as they arrive.
                    if let Some(record) = self.doc.workflow.as_mut() {
                        for item in &items {
                            record.items.retain(|i| i.key != item.key);
                        }
                        record.items.extend(items.iter().cloned());
                        self.dirty = true;
                    }
                }
                buffer.items.extend(items);
                true
            }
            ProductionOutput::RunDone { run, result } => self.run_done(run, result),
            ProductionOutput::RunFailed { run, message, .. } => {
                let Some((purpose, _)) = self.runs.remove(&run) else {
                    return false;
                };
                match purpose {
                    RunPurpose::Bake { fingerprint, .. } | RunPurpose::Plate { fingerprint } => {
                        self.run_failures.insert(fingerprint, message);
                    }
                    RunPurpose::Workflow { fingerprint } => {
                        if let Some(record) = self
                            .doc
                            .workflow
                            .as_mut()
                            .filter(|r| r.fingerprint == fingerprint)
                        {
                            record.error = Some(message);
                            self.dirty = true;
                        }
                    }
                }
                true
            }
            ProductionOutput::Attention { provider, message } => {
                let line = format!("{provider}：{message}");
                if !self.attention.contains(&line) {
                    self.attention.push(line);
                }
                true
            }
        }
    }

    /// Every target a result is for: in flight, or (a result recovered from
    /// an earlier session) whatever the plan needs it for now.
    fn targets(&self, key: &TaskKey) -> Vec<Target> {
        let mut targets = self.in_flight.get(key).cloned().unwrap_or_default();
        if let Some(spec) = self.spec() {
            for w in work::plan_work(&self.doc, spec) {
                if &w.key() == key && !targets.contains(&w.target) {
                    targets.push(w.target);
                }
            }
        }
        targets
    }

    fn task_done(
        &mut self,
        key: TaskKey,
        output: Option<Value>,
        file: Option<String>,
        pcm: Option<Pcm16>,
    ) -> bool {
        let targets = self.targets(&key);
        let Some(spec) = self.spec().cloned() else {
            return false;
        };
        if targets.is_empty() {
            return false;
        }
        self.in_flight.remove(&key);
        self.states.remove(&key);
        let text = output
            .as_ref()
            .and_then(|o| o.get("text"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "AI 没有返回文字".to_string());
        for target in targets {
            let applied = match &target {
                Target::Outline { chunk } => {
                    self.outline.insert(*chunk, text.clone());
                    Ok(())
                }
                Target::Voice { segment } => match (&file, &pcm) {
                    (Some(file), Some(pcm)) => {
                        let output = output.clone().unwrap_or(Value::Null);
                        let sentences: Vec<Value> = output
                            .get("sentences")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default();
                        let words: Vec<Word> = output
                            .get("words")
                            .cloned()
                            .and_then(|w| serde_json::from_value(w).ok())
                            .unwrap_or_default();
                        work::apply_voice(
                            &mut self.doc,
                            &spec,
                            segment,
                            SpeechResult {
                                task: key.clone(),
                                audio: file.clone(),
                                sentences_raw: &sentences,
                                words: &words,
                                pcm: Pcm {
                                    samples: &pcm.samples,
                                    rate: pcm.rate,
                                },
                            },
                        )
                    }
                    _ => Err("配音没有返回音频".into()),
                },
                Target::Storyboard { segment } => {
                    work::apply_storyboard(&mut self.doc, &spec, segment, text.clone())
                }
                Target::Image { element, variant } => match &file {
                    Some(file) => work::apply_image(
                        &mut self.doc,
                        element,
                        variant,
                        key.clone(),
                        Ok(file.clone()),
                    ),
                    None => Err("生图没有返回图片".into()),
                },
            };
            if let Err(message) = applied {
                self.failed.insert(
                    key.clone(),
                    Failure {
                        class: "invalid".into(),
                        message,
                        possibly_billed: false,
                    },
                );
            }
        }
        self.dirty = true;
        true
    }

    fn task_failed(
        &mut self,
        key: TaskKey,
        class: String,
        message: String,
        possibly_billed: bool,
    ) -> bool {
        let targets = self.targets(&key);
        if targets.is_empty() {
            return false;
        }
        self.in_flight.remove(&key);
        self.states.remove(&key);
        if let Some(spec) = self.spec().cloned() {
            for target in &targets {
                match target {
                    // Outline and storyboard fall back to local rules.
                    Target::Outline { chunk } => {
                        self.outline.insert(*chunk, Err(message.clone()));
                    }
                    Target::Storyboard { segment } => {
                        let _ = work::apply_storyboard(
                            &mut self.doc,
                            &spec,
                            segment,
                            Err(message.clone()),
                        );
                    }
                    Target::Image { element, variant } => {
                        let _ = work::apply_image(
                            &mut self.doc,
                            element,
                            variant,
                            key.clone(),
                            Err(message.clone()),
                        );
                    }
                    Target::Voice { .. } => {}
                }
            }
        }
        self.failed.insert(
            key,
            Failure {
                class,
                message,
                possibly_billed,
            },
        );
        self.dirty = true;
        true
    }

    fn run_done(&mut self, run: LocalRunId, result: Value) -> bool {
        let Some((purpose, buffer)) = self.runs.remove(&run) else {
            return false;
        };
        match purpose {
            RunPurpose::Bake { board, fingerprint } => {
                let Some(target) = self
                    .doc
                    .segments
                    .iter_mut()
                    .flat_map(|s| &mut s.boards)
                    .find(|b| b.id == board)
                else {
                    return false;
                };
                target.render = Some(Rendered {
                    fingerprint,
                    result: json!({ "items": buffer.items, "worker": result }),
                });
            }
            RunPurpose::Plate { fingerprint } => {
                let file = buffer.items.iter().find_map(|i| match &i.source {
                    todo_production::items::ItemSource::File { file, .. } => {
                        Some(file.clone())
                    }
                    _ => None,
                });
                match file {
                    Some(file) => {
                        self.doc.plate = Some(Rendered {
                            fingerprint,
                            result: json!({ "file": file }),
                        })
                    }
                    None => {
                        self.run_failures
                            .insert(fingerprint, "底板没有产出图片".into());
                    }
                }
            }
            RunPurpose::Workflow { fingerprint } => {
                let emitted: BTreeSet<&str> = buffer.items.iter().map(|i| i.key.as_str()).collect();
                if let Some(record) = self
                    .doc
                    .workflow
                    .as_mut()
                    .filter(|r| r.fingerprint == fingerprint)
                {
                    record.items.retain(|i| emitted.contains(i.key.as_str()));
                    let ok = result
                        .get("ok")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    record.result = Some(result.clone());
                    if ok {
                        record.error = None;
                    } else {
                        // Keep failure visible: UI treats error as Failed even when
                        // result is also present.
                        record.error = Some(
                            result
                                .get("error")
                                .and_then(|v| v.as_str())
                                .unwrap_or("工作流失败")
                                .to_owned(),
                        );
                    }
                }
            }
        }
        self.dirty = true;
        true
    }

    /// The items the production has made so far, and whether the set is final.
    pub fn items(&self) -> (Vec<Item>, bool) {
        match &self.setup.recipe {
            Recipe::Narration { spec, bake, .. } => (
                narration::host_items(&self.doc, spec, &self.layout(), bake.is_some()),
                true,
            ),
            Recipe::Workflow { .. } => {
                let record = self.doc.workflow.as_ref();
                let complete = record.is_some_and(|r| r.result.is_some());
                (
                    record.map(|r| r.items.clone()).unwrap_or_default(),
                    complete,
                )
            }
        }
    }

    pub fn placement_plan(&self, project: &Project) -> PlacementPlan {
        let (items, complete) = self.items();
        placement::plan(&PlacementInput {
            project,
            production: &self.doc.id,
            placement: &self.doc.placement,
            items: &items,
            workdir: &self.setup.workdir,
            complete,
        })
    }

    /// Recover the state written by the old progressive placement path when a
    /// whole production was recorded as deleted before its media was saved.
    ///
    /// This is deliberately narrow: it only runs when there are no clips for
    /// this production, no item was ever recorded as placed, every current
    /// item is tombstoned at its current revision, and the persisted error is
    /// the media-registration failure. A real user deletion therefore keeps
    /// its tombstone and remains opt-in through `restore_production`.
    pub fn recover_stale_placement(&mut self, project: &Project) -> bool {
        if self.doc.placement.placed.is_empty()
            && self.doc.placement.error.as_deref()
                == Some("多个素材无法入轨（路径缺失或登记失败）")
        {
            let (items, _) = self.items();
            let all_current_tombstoned = !items.is_empty()
                && items.iter().all(|item| {
                    self.doc
                        .placement
                        .tombstones
                        .contains(&(item.key.clone(), item.rev.clone()))
                });
            let has_timeline_clip = project.tracks.iter().any(|track| {
                track.clips.iter().any(|clip| {
                    clip.origin
                        .as_ref()
                        .is_some_and(|origin| origin.production == self.doc.id)
                })
            });
            if all_current_tombstoned && !has_timeline_clip {
                self.doc.placement.tombstones.clear();
                self.doc.placement.error = None;
                self.attention
                    .retain(|line| !line.starts_with("入轨未成功："));
                self.dirty = true;
                return true;
            }
        }
        false
    }

    /// Record a plan that reached the timeline.
    pub fn placed(&mut self, plan: &PlacementPlan) {
        placement::record(&mut self.doc.placement, plan);
        self.dirty = true;
    }

    /// The user's production edits (one batch, all or nothing).
    pub fn edit(&mut self, edits: &[Edit]) -> Result<(), String> {
        let max = self.spec().map_or(4, |s| s.max_elements);
        self.doc
            .apply_edits(edits, max)
            .map_err(|e| e.to_string())?;
        self.dirty = true;
        Ok(())
    }

    /// Retry a failed request, or every failed bake and workflow run.
    pub fn retry(&mut self, key: Option<&TaskKey>) -> Vec<ProductionTask> {
        match key {
            Some(key) => {
                if self.failed.remove(key).is_none() {
                    return Vec::new();
                }
                // The journal re-runs it; the result arrives as a normal report.
                let targets = self.targets(key);
                if !targets.is_empty() {
                    self.in_flight.insert(key.clone(), targets);
                }
                vec![ProductionTask::Retry(key.clone())]
            }
            None => {
                self.run_failures.clear();
                if let Some(record) = self.doc.workflow.as_mut() {
                    // Clear the error so the host can start again, but keep
                    // fingerprint + items so resume stays the same logical run
                    // (placement / tombstones stay coherent — see onrail B6).
                    let _ = record.error.take();
                }
                Vec::new()
            }
        }
    }

    /// Stop everything this production runs (closing the project).
    pub fn stop(&mut self) -> Vec<ProductionTask> {
        let runs: Vec<LocalRunId> = self.runs.keys().copied().collect();
        self.runs.clear();
        runs.into_iter().map(ProductionTask::CancelRun).collect()
    }

    pub fn status(&self) -> ProductionStatus {
        let mut nodes = Vec::new();
        let mut stages = Vec::new();
        let mut done = 0;
        let mut total = 0;
        let mut count = |state: StatusState| {
            total += 1;
            if state == StatusState::Done {
                done += 1;
            }
        };
        let request_state = |key: &TaskKey| -> (StatusState, Option<String>) {
            if let Some(f) = self.failed.get(key) {
                let detail = if f.possibly_billed {
                    format!("{}（可能已扣费，确认后可重试）", f.message)
                } else {
                    f.message.clone()
                };
                return (StatusState::Failed, Some(detail));
            }
            match self.states.get(key) {
                Some(TaskState::Waiting(reason)) => (StatusState::Waiting, Some(reason.clone())),
                Some(TaskState::Downloading) => (StatusState::Running, Some("下载中".into())),
                Some(TaskState::Generating) => (StatusState::Running, None),
                None if self.in_flight.contains_key(key) => (StatusState::Running, None),
                None => (StatusState::Queued, None),
            }
        };
        match &self.setup.recipe {
            Recipe::Narration { spec, bake, plate } => {
                let bake_inputs: BTreeMap<String, String> =
                    narration::bake_requests(&self.doc, spec, &self.layout(), &self.setup.canvas)
                        .into_iter()
                        .map(|r| (r.board, r.fingerprint))
                        .collect();
                let mut outline_states = Vec::new();
                let mut voice_states = Vec::new();
                let mut subtitle_states = Vec::new();
                let mut story_states = Vec::new();
                let mut image_states = Vec::new();
                let mut compose_states = Vec::new();
                if self.doc.segments.is_empty() {
                    let state = if self.failed.is_empty() {
                        StatusState::Running
                    } else {
                        StatusState::Failed
                    };
                    count(state);
                    outline_states.push(state);
                    nodes.push(StatusNode {
                        key: "outline".into(),
                        label: "分段".into(),
                        state,
                        detail: None,
                        children: Vec::new(),
                    });
                } else {
                    outline_states.push(StatusState::Done);
                }
                for (g, segment) in self.doc.segments.iter().enumerate() {
                    let voice_key = work::voice_spec(spec, &segment.text).key();
                    let (state, detail) =
                        if segment.voice.as_ref().is_some_and(|v| v.task == voice_key) {
                            (StatusState::Done, None)
                        } else {
                            request_state(&voice_key)
                        };
                    count(state);
                    voice_states.push(state);
                    if spec.subtitle_chars.is_some() {
                        subtitle_states.push(if segment.subtitles.is_some() {
                            StatusState::Done
                        } else if state == StatusState::Done {
                            StatusState::Running
                        } else {
                            StatusState::Queued
                        });
                    }
                    story_states.push(if !segment.boards.is_empty() {
                        StatusState::Done
                    } else {
                        work::storyboard_spec(spec, &self.doc, g)
                            .map_or(StatusState::Queued, |s| request_state(&s.key()).0)
                    });
                    let mut children = vec![StatusNode {
                        key: format!("{}/voice", segment.id),
                        label: "配音".into(),
                        state,
                        detail,
                        children: Vec::new(),
                    }];
                    for (b, board) in segment.boards.iter().enumerate() {
                        let mut elements = Vec::new();
                        for element in &board.elements {
                            let (state, detail) = match element.picked() {
                                Some(v) if v.file.is_some() => (StatusState::Done, None),
                                Some(v) if v.error.is_some() => {
                                    (StatusState::Failed, v.error.clone())
                                }
                                _ => request_state(&work::image_spec(spec, element, 0).key()),
                            };
                            count(state);
                            image_states.push(state);
                            elements.push(StatusNode {
                                key: element.id.clone(),
                                label: element.label.clone(),
                                state,
                                detail,
                                children: Vec::new(),
                            });
                        }
                        let baking = self.runs.values().find(
                            |(p, _)| matches!(p, RunPurpose::Bake { board: id, .. } if *id == board.id),
                        );
                        let (state, detail) = if bake.is_none() {
                            (StatusState::Done, None)
                        } else if let Some((_, buffer)) = baking {
                            (
                                StatusState::Running,
                                Some(format!("{} {:.0}%", buffer.phase, buffer.fraction * 100.0)),
                            )
                        } else if board.render.is_some() {
                            (StatusState::Done, None)
                        } else if let Some(message) = bake_inputs
                            .get(&board.id)
                            .and_then(|fp| self.run_failures.get(fp))
                        {
                            (StatusState::Failed, Some(message.clone()))
                        } else {
                            (StatusState::Queued, None)
                        };
                        if bake.is_some() {
                            count(state);
                            compose_states.push(state);
                        }
                        children.push(StatusNode {
                            key: board.id.clone(),
                            label: format!("画面 {}", b + 1),
                            state,
                            detail,
                            children: elements,
                        });
                    }
                    nodes.push(StatusNode {
                        key: segment.id.clone(),
                        label: if segment.title.is_empty() {
                            format!("第 {} 段", g + 1)
                        } else {
                            segment.title.clone()
                        },
                        state: fold_states(&children.iter().map(|c| c.state).collect::<Vec<_>>()),
                        detail: None,
                        children,
                    });
                }
                if plate.is_some() {
                    if let Some((fingerprint, _)) =
                        narration::plate_request(&self.doc, &self.setup.canvas)
                    {
                        let running = self
                            .runs
                            .values()
                            .any(|(p, _)| matches!(p, RunPurpose::Plate { .. }));
                        compose_states.push(if self.doc.plate.is_some() {
                            StatusState::Done
                        } else if running {
                            StatusState::Running
                        } else if self.run_failures.contains_key(&fingerprint) {
                            StatusState::Failed
                        } else {
                            StatusState::Queued
                        });
                    }
                }
                stages.push(stage(StageKind::Outline, &outline_states));
                stages.push(stage(StageKind::Voice, &voice_states));
                if spec.subtitle_chars.is_some() {
                    stages.push(stage(StageKind::Subtitles, &subtitle_states));
                }
                stages.push(stage(StageKind::Storyboard, &story_states));
                stages.push(stage(StageKind::Images, &image_states));
                if bake.is_some() || plate.is_some() {
                    stages.push(stage(StageKind::Compose, &compose_states));
                }
            }
            Recipe::Workflow { .. } => {
                let record = self.doc.workflow.as_ref();
                let run = self.runs.values().next();
                let (state, detail) = match (record, run) {
                    (_, Some((_, buffer))) => (
                        StatusState::Running,
                        Some(format!("{} {:.0}%", buffer.phase, buffer.fraction * 100.0)),
                    ),
                    (Some(r), None) if r.error.is_some() => (StatusState::Failed, r.error.clone()),
                    (Some(r), None) if r.result.is_some() => (StatusState::Done, None),
                    _ => (StatusState::Queued, None),
                };
                count(state);
                let mut generate = stage(StageKind::Generate, &[state]);
                generate.phase = run.map(|(_, buffer)| (buffer.phase.clone(), buffer.fraction));
                stages.push(generate);
                nodes.push(StatusNode {
                    key: "run".into(),
                    label: format!("生成（已出 {} 条）", record.map_or(0, |r| r.items.len())),
                    state,
                    detail,
                    children: Vec::new(),
                });
            }
        }
        // Results reach the timeline as they are produced: a placed item or
        // one the user deleted is settled; the rest is on its way.
        let (items, _) = self.items();
        let placement = &self.doc.placement;
        let deleted_item = |i: &Item| {
            placement
                .tombstones
                .contains(&(i.key.clone(), i.rev.clone()))
        };
        let deleted = items.iter().filter(|i| deleted_item(i)).count();
        let placement_stuck = self
            .attention
            .iter()
            .any(|line| line.starts_with("入轨未成功"));
        let timeline: Vec<StatusState> = items
            .iter()
            .map(|i| {
                if placement.placed.get(&i.key) == Some(&i.rev) || deleted_item(i) {
                    StatusState::Done
                } else if placement_stuck {
                    // Adopt/ApplyGenerated failed: not "still placing".
                    StatusState::Failed
                } else {
                    StatusState::Running
                }
            })
            .collect();
        stages.push(stage(StageKind::Timeline, &timeline));
        ProductionStatus {
            id: self.doc.id.clone(),
            title: self.doc.title.clone(),
            extension: self.setup.extension.clone(),
            script: self.doc.script.chars().take(40).collect(),
            done,
            total,
            stages,
            nodes,
            attention: self.attention.clone(),
            failures: self
                .failed
                .iter()
                .map(|(k, f)| FailureStatus {
                    key: k.clone(),
                    work: self.failed_work(k),
                    class: f.class.clone(),
                    message: f.message.clone(),
                    possibly_billed: f.possibly_billed,
                })
                .collect(),
            run_failures: self.run_failures.len()
                + usize::from(
                    self.doc
                        .workflow
                        .as_ref()
                        .is_some_and(|r| r.error.is_some()),
                ),
            deleted,
        }
    }

    fn failed_work(&self, key: &TaskKey) -> FailedWork {
        let segment = |id: &str| self.doc.segments.iter().position(|s| s.id == id);
        match self.targets(key).into_iter().next() {
            Some(Target::Outline { .. }) => FailedWork::Outline,
            Some(Target::Voice { segment: id }) => {
                segment(&id).map_or(FailedWork::Other, |segment| FailedWork::Voice { segment })
            }
            Some(Target::Storyboard { segment: id }) => segment(&id)
                .map_or(FailedWork::Other, |segment| FailedWork::Storyboard {
                    segment,
                }),
            Some(Target::Image { element, .. }) => self
                .doc
                .segments
                .iter()
                .flat_map(|s| &s.boards)
                .flat_map(|b| &b.elements)
                .find(|e| e.id == element)
                .map_or(FailedWork::Other, |e| FailedWork::Image {
                    label: e.label.clone(),
                }),
            None => FailedWork::Other,
        }
    }

    /// A readable library name for a generated file this production placed:
    /// "第 1 段配音", "黑板底", "画面 · 存钱罐" — never the content hash.
    pub fn media_name(&self, path: &str) -> Option<String> {
        let (items, _) = self.items();
        let item = items.iter().find(|item| {
            matches!(&item.source, todo_production::items::ItemSource::File { file, .. }
                if placement::media_path(&self.setup.workdir, file) == path)
        })?;
        Some(self.key_name(&item.key, item.role))
    }

    fn key_name(&self, key: &str, role: todo_production::items::ItemRole) -> String {
        use todo_production::items::ItemRole;
        let segment = |id: &str| {
            self.doc
                .segments
                .iter()
                .position(|s| s.id == id)
                .map(|g| g + 1)
        };
        let element = |id: &str| {
            self.doc
                .segments
                .iter()
                .flat_map(|s| &s.boards)
                .flat_map(|b| &b.elements)
                .find(|e| e.id == id)
                .map(|e| e.label.clone())
        };
        let parts: Vec<&str> = key.split('/').collect();
        match parts.as_slice() {
            ["plate"] => return "黑板底".into(),
            [g, "voice"] => {
                if let Some(n) = segment(g) {
                    return format!("第 {n} 段配音");
                }
            }
            [_, e] => {
                if let Some(label) = element(e) {
                    return format!("画面 · {label}");
                }
            }
            [_, e, "dust"] => {
                if let Some(label) = element(e) {
                    return format!("粉笔灰 · {label}");
                }
            }
            _ => {}
        }
        // Workflow keys (`shot/3/prop`, `bgm/intro`, …): translate the words.
        let words: Vec<String> = parts
            .iter()
            .map(|part| {
                match *part {
                    "voice" => "配音",
                    "shot" => "镜头",
                    "bgm" => "背景音乐",
                    "wipe" => "转场",
                    "sfx" => "音效",
                    "sub" => "字幕",
                    "intro" => "开场",
                    "mid" => "中段",
                    "outro" => "结尾",
                    "hero" | "character" => "角色",
                    "prop" => "道具",
                    "bubble" => "气泡",
                    "intro_red" => "开场字",
                    "card" => "卡片",
                    "el" => "元素",
                    "line" => "升华字",
                    "bg" => "背景",
                    "title" => "片头",
                    other => other,
                }
                .to_owned()
            })
            .collect();
        let fallback = match role {
            ItemRole::Voice => "配音",
            ItemRole::Music => "背景音乐",
            ItemRole::Sfx => "音效",
            ItemRole::Visual => "画面",
            ItemRole::Text | ItemRole::Subtitle => "文字",
        };
        if words
            .iter()
            .all(|w| w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        {
            format!("{fallback} {}", words.join(" "))
        } else {
            words.join(" ")
        }
    }

    /// Put back every generated clip the user deleted from the timeline.
    pub fn restore_deleted(&mut self) {
        if !self.doc.placement.tombstones.is_empty() {
            self.doc.placement.tombstones.clear();
            self.dirty = true;
        }
    }

    /// Surface a placement / ApplyGenerated / adopt failure for UI + Bridge.
    /// Keeps a single「入轨未成功」card so 80+ file failures stay readable.
    pub fn note_placement_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        let short = if message.contains("素材登记失败") || message.contains("无法入轨") {
            "多个素材无法入轨（路径缺失或登记失败）".into()
        } else {
            message.chars().take(120).collect::<String>()
        };
        self.attention
            .retain(|line| !line.starts_with("入轨未成功："));
        self.attention.insert(
            0,
            format!("入轨未成功：{short}。可点「重试」或改时间轴锁定后重试。"),
        );
        self.doc.placement.error = Some(short);
        self.dirty = true;
    }

    pub fn clear_placement_errors(&mut self) {
        self.attention
            .retain(|line| !line.starts_with("入轨未成功："));
        if self.doc.placement.error.take().is_some() {
            self.dirty = true;
        }
    }

    /// Whether this production still needs the given repository-relative media path.
    pub fn needs_media_path(&self, path: &str) -> bool {
        let (items, _) = self.items();
        items.iter().any(|item| {
            matches!(
                &item.source,
                todo_production::items::ItemSource::File { file, .. }
                    if placement::media_path(&self.setup.workdir, file) == path
            )
        })
    }

    /// How many of this production's file items sit in the unadoptable set.
    pub fn unadoptable_count(&self, unadoptable: &BTreeSet<String>) -> usize {
        let (items, _) = self.items();
        items
            .iter()
            .filter_map(|item| match &item.source {
                todo_production::items::ItemSource::File { file, .. } => {
                    Some(placement::media_path(&self.setup.workdir, file))
                }
                _ => None,
            })
            .filter(|path| unadoptable.contains(path))
            .collect::<BTreeSet<_>>()
            .len()
    }
}

fn is_production_workdir(path: &str) -> bool {
    let n = path.replace('\\', "/");
    n.contains("/productions/") && (n.ends_with("/work") || n.contains("/work/") || n.ends_with("/work"))
}

fn is_cache_run_workdir(path: &str) -> bool {
    path.replace('\\', "/").contains("cache/workflow-runs")
}

#[cfg(test)]
mod resume_tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;
    use todo_core::create_project;
    use todo_production::items::{ItemRole, ItemSource};
    use todo_production::production::{PackRef, WorkflowRecord};
    use todo_production::segment::Limits;
    use todo_production::{Production, SplitMode};
    use todo_workflow::workflow::protocol::Canvas;

    fn workflow_session() -> ProductionSession {
        let doc = Production::new(
            "w",
            PackRef {
                id: "studio.workflow-test".into(),
                version: "1.0.0".into(),
            },
            "script",
            BTreeMap::new(),
            SplitMode::Paragraph,
            Limits {
                target: 12,
                max: 60,
            },
        );
        let mut session = ProductionSession::new(
            "p",
            doc,
            ProductionSetup {
                extension: "studio.workflow-test".into(),
                recipe: Recipe::Workflow {
                    op: "run".into(),
                    input: json!({ "script": "script" }),
                },
                canvas: Canvas {
                    width: 1920,
                    height: 1080,
                    fps: 30.0,
                },
                workdir: "data/p/productions/w/work".into(),
            },
        );
        session.doc.workflow = Some(WorkflowRecord {
            fingerprint: "fp-keep".into(),
            items: Vec::new(),
            result: None,
            error: Some("network down".into()),
        });
        session
    }

    #[test]
    fn whole_run_retry_keeps_workflow_fingerprint() {
        let mut session = workflow_session();
        let _ = session.retry(None);
        let record = session.doc.workflow.as_ref().unwrap();
        assert_eq!(record.fingerprint, "fp-keep");
        assert!(record.error.is_none());
    }

    #[test]
    fn set_workdir_change_surfaces_attention() {
        let mut session = workflow_session();
        session.set_workdir("data/p/productions/w/work-b".into());
        assert!(
            session
                .attention
                .iter()
                .any(|line| line.contains("工作目录已切换")),
            "{:?}",
            session.attention
        );
    }

    #[test]
    fn set_workdir_refuses_cache_run_over_production_work() {
        let mut session = workflow_session();
        let kept = session.setup.workdir.clone();
        session.set_workdir("cache/workflow-runs/studio-x-1".into());
        assert_eq!(session.setup.workdir, kept);
        assert!(
            session
                .attention
                .iter()
                .any(|line| line.contains("续跑已复用原工作目录")),
            "{:?}",
            session.attention
        );
    }

    #[test]
    fn stale_all_deleted_placement_is_recovered_without_timeline_clips() {
        let mut session = workflow_session();
        let item = Item {
            key: "visual/s1".into(),
            rev: "v1-services/a.png".into(),
            role: ItemRole::Visual,
            lane: 0,
            start: 0.0,
            duration: 1.0,
            source: ItemSource::File {
                file: "services/a.png".into(),
                r#in: 0.0,
            },
            transform: None,
            intro: None,
            outro: None,
            transition: None,
            link: None,
            gain: None,
            fade: None,
        };
        session.doc.workflow.as_mut().unwrap().items = vec![item.clone()];
        session.doc.workflow.as_mut().unwrap().result = Some(json!({"ok": true}));
        session.doc.placement.tombstones.insert((item.key, item.rev));
        session.doc.placement.error = Some("多个素材无法入轨（路径缺失或登记失败）".into());
        let project = create_project(Some(1920.0), Some(1080.0), Some(30.0), None, None);

        assert!(session.recover_stale_placement(&project));
        assert!(session.doc.placement.tombstones.is_empty());
        assert!(session.doc.placement.error.is_none());
    }
}
