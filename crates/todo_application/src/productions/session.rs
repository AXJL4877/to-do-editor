use super::{
    LocalRunId, Pcm16, ProductionOutput, ProductionSetup, ProductionTask, Recipe, TaskState,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use todo_core::Project;
use todo_production::extension::items::Item;
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

/// What the production panel shows.
#[derive(Debug, Clone, PartialEq)]
pub struct ProductionStatus {
    pub id: String,
    pub title: String,
    pub extension: String,
    pub done: usize,
    pub total: usize,
    pub nodes: Vec<StatusNode>,
    pub attention: Vec<String>,
    /// Failed requests the user may retry: (key, message, possibly billed).
    pub failures: Vec<(TaskKey, String, bool)>,
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
                    todo_production::extension::items::ItemSource::File { file, .. } => {
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
                    record.result = Some(result);
                    record.error = None;
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
                    if record.error.take().is_some() {
                        record.fingerprint.clear();
                    }
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
                None if self.in_flight.contains_key(key) => (StatusState::Queued, None),
                None => (StatusState::Queued, None),
            }
        };
        match &self.setup.recipe {
            Recipe::Narration { spec, bake, .. } => {
                let bake_inputs: BTreeMap<String, String> =
                    narration::bake_requests(&self.doc, spec, &self.layout(), &self.setup.canvas)
                        .into_iter()
                        .map(|r| (r.board, r.fingerprint))
                        .collect();
                if self.doc.segments.is_empty() {
                    let state = if self.failed.is_empty() {
                        StatusState::Running
                    } else {
                        StatusState::Failed
                    };
                    count(state);
                    nodes.push(StatusNode {
                        key: "outline".into(),
                        label: "分段".into(),
                        state,
                        detail: None,
                        children: Vec::new(),
                    });
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
                            let picked = element.picked();
                            let (state, detail) = match picked {
                                Some(v) if v.file.is_some() => (StatusState::Done, None),
                                Some(v) if v.error.is_some() => {
                                    (StatusState::Failed, v.error.clone())
                                }
                                _ => {
                                    let key = work::image_spec(spec, element, 0).key();
                                    let (state, detail) = request_state(&key);
                                    (
                                        if state == StatusState::Failed {
                                            StatusState::Failed
                                        } else {
                                            state
                                        },
                                        detail,
                                    )
                                }
                            };
                            count(state);
                            elements.push(StatusNode {
                                key: element.id.clone(),
                                label: element.label.clone(),
                                state,
                                detail,
                                children: Vec::new(),
                            });
                        }
                        let baking = self
                            .runs
                            .values()
                            .find(|(p, _)| matches!(p, RunPurpose::Bake { board: id, .. } if *id == board.id));
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
                        }
                        children.push(StatusNode {
                            key: board.id.clone(),
                            label: format!("黑板 {}", b + 1),
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
                        state: if children.iter().all(|c| c.state == StatusState::Done) {
                            StatusState::Done
                        } else if children.iter().any(|c| c.state == StatusState::Failed) {
                            StatusState::Failed
                        } else {
                            StatusState::Running
                        },
                        detail: None,
                        children,
                    });
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
                nodes.push(StatusNode {
                    key: "run".into(),
                    label: format!("生成（已出 {} 条）", record.map_or(0, |r| r.items.len())),
                    state,
                    detail,
                    children: Vec::new(),
                });
            }
        }
        ProductionStatus {
            id: self.doc.id.clone(),
            title: if self.doc.title.is_empty() {
                "未命名成片".into()
            } else {
                self.doc.title.clone()
            },
            extension: self.setup.extension.clone(),
            done,
            total,
            nodes,
            attention: self.attention.clone(),
            failures: self
                .failed
                .iter()
                .map(|(k, f)| (k.clone(), f.message.clone(), f.possibly_billed))
                .collect(),
        }
    }
}
