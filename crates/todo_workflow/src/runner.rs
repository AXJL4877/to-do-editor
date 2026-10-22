//! Graph runner: executes a validated `todo-graph/1` with injected services.
//!
//! The runner owns sequencing and payload shaping only. Every side effect
//! (LLM, image, vision, speech, subtitles) goes through [`Services`], which the
//! host implements over its AI router. Nodes that are still design-stage
//! (`split_shots`, `picture_batch`, `python:`…) fail loudly instead of silently
//! producing nothing.
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use crate::workflow::graph::{Graph, GraphEdge, GraphNode};
use crate::workflow::nodes::{self, DataKind};
use crate::workflow::protocol::{ChatCall, ImageCall, SpeechCall, SubtitlesCall};
use todo_production::segment::{self, Limits as SegmentLimits, Unit};
use todo_production::tasks::ChatMessage;

/// Side effects the host provides. One call per node, no retries here.
pub trait Services {
    fn chat(&mut self, call: ChatCall) -> Result<Value, String>;
    fn image(&mut self, call: ImageCall) -> Result<Value, String>;
    fn vision(&mut self, prompt: String, images: Vec<PathBuf>) -> Result<Value, String>;
    fn speech(&mut self, call: SpeechCall) -> Result<Value, String>;
    fn subtitles(&mut self, call: SubtitlesCall) -> Result<Value, String>;

    /// Blocking driver entry: dispatch one node call. Async hosts ignore this
    /// and handle [`RequestedCall`] themselves.
    fn execute(&mut self, call: RequestedCall) -> Result<Value, String> {
        match call {
            RequestedCall::Chat(call) => self.chat(call),
            RequestedCall::Image(call) => self.image(call),
            RequestedCall::Vision { prompt, images } => self.vision(prompt, images),
            RequestedCall::Speech(call) => self.speech(call),
            RequestedCall::Subtitles(call) => self.subtitles(call),
        }
    }
}

/// What the run was started with.
#[derive(Debug, Clone, Default)]
pub struct RunInput {
    /// Text from the production panel's script box.
    pub script: String,
    /// Seconds into the timeline, stamped on the trigger pulse.
    pub at: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeRun {
    pub id: String,
    pub node_type: String,
    pub ok: bool,
    pub message: String,
    /// Output port names that produced a value.
    pub outputs: Vec<String>,
    /// First output payload as pretty JSON, truncated: the panel shows it.
    pub preview: Option<String>,
    /// Image this node produced (absolute or package-relative path).
    pub image: Option<String>,
    /// `segments` port payload when present (relay mid-store / 口播分段).
    pub segments: Option<Value>,
    /// `files` / `images` port payload for media preview on relay / media_bin.
    pub files: Option<Value>,
    /// Wall time this node took (including waiting for its call), in ms.
    pub duration_ms: u64,
    /// Characters in the first output's text (say, the script length).
    pub output_chars: Option<usize>,
    /// What this node did, step by step (the inspector's command window).
    pub logs: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunReport {
    pub nodes: Vec<NodeRun>,
    /// Payloads handed to the timeline by `emit` nodes.
    pub emitted: Vec<Value>,
    pub error: Option<String>,
}

impl RunReport {
    pub fn ok(&self) -> bool {
        self.error.is_none() && self.nodes.iter().all(|node| node.ok)
    }
}

fn node_has_edge(graph: &Graph, id: &str) -> bool {
    graph
        .edges
        .iter()
        .any(|edge| edge.from == id || edge.to == id)
}

fn node_missing_required(_graph: &Graph, node: &GraphNode, wired: &BTreeSet<(String, String)>) -> Vec<String> {
    let Some(spec) = nodes::lookup(&node.node_type) else {
        return Vec::new();
    };
    spec.inputs
        .iter()
        .filter(|port| port.required)
        .filter(|port| !wired.contains(&(node.id.clone(), port.name.to_owned())))
        .map(|port| format!("{}.{}", node.id, port.name))
        .collect()
}

/// Input payloads by port name, following edges into `node`.
fn gather_inputs(
    graph: &Graph,
    produced: &BTreeMap<(String, String), Value>,
    node: &GraphNode,
) -> BTreeMap<String, Value> {
    let mut inputs = BTreeMap::new();
    for edge in &graph.edges {
        if edge.to != node.id {
            continue;
        }
        let Some((out_port, in_port)) = ports_for(graph, edge) else {
            continue;
        };
        if let Some(value) = produced.get(&(edge.from.clone(), out_port)) {
            let value = coerce_wire_value(graph, edge, value.clone());
            inputs.insert(in_port, value);
        }
    }
    inputs
}

/// SpeechClips ↔ SpeechClip: keep count semantics on the source; adapt at the sink.
fn coerce_wire_value(graph: &Graph, edge: &GraphEdge, value: Value) -> Value {
    let Some(from) = graph.nodes.iter().find(|n| n.id == edge.from) else {
        return value;
    };
    let Some(to) = graph.nodes.iter().find(|n| n.id == edge.to) else {
        return value;
    };
    let Ok((out_port, in_port)) = nodes::resolve_edge_ports(
        &from.node_type,
        &to.node_type,
        edge.out.as_deref(),
        edge.input.as_deref(),
    ) else {
        return value;
    };
    match (out_port.kind, in_port.kind) {
        (DataKind::SpeechClips, DataKind::SpeechClip) => {
            if let Some(first) = value
                .get("clips")
                .and_then(Value::as_array)
                .and_then(|list| list.first())
            {
                first.clone()
            } else {
                value
            }
        }
        (DataKind::SpeechClip, DataKind::SpeechClips) => {
            if value.get("clips").is_some() {
                value
            } else {
                json!({ "clips": [value] })
            }
        }
        (DataKind::ImageAssets, DataKind::ImageAsset) => {
            if let Some(first) = value
                .get("images")
                .and_then(Value::as_array)
                .and_then(|list| list.first())
            {
                first.clone()
            } else {
                value
            }
        }
        (DataKind::ImageAsset, DataKind::ImageAssets) => {
            if value.get("images").is_some() {
                value
            } else {
                json!({ "images": [value] })
            }
        }
        _ => value,
    }
}

fn ports_for(graph: &Graph, edge: &GraphEdge) -> Option<(String, String)> {
    let from = graph.nodes.iter().find(|n| n.id == edge.from)?;
    let to = graph.nodes.iter().find(|n| n.id == edge.to)?;
    let (out, input) = nodes::resolve_edge_ports(
        &from.node_type,
        &to.node_type,
        edge.out.as_deref(),
        edge.input.as_deref(),
    )
    .ok()?;
    Some((out.name.to_owned(), input.name.to_owned()))
}

fn text_of(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return text.to_owned();
    }
    // A segments payload stands in for its text.
    if let Some(list) = value.get("segments").and_then(Value::as_array) {
        return list
            .iter()
            .filter_map(|segment| segment.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("");
    }
    String::new()
}

fn param_str(node: &GraphNode, key: &str) -> String {
    node.params
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn param_num(node: &GraphNode, key: &str, default: f64) -> f64 {
    node.params
        .get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}

fn segment_limits(node: &GraphNode) -> SegmentLimits {
    // The inspector exposes a practical minimum, while the runner also
    // supports tiny limits for tests and small utility workflows.
    let target = param_num(node, "target", 120.0).round().clamp(1.0, 1200.0) as usize;
    let max = param_num(node, "max", 200.0)
        .round()
        .clamp(target.max(1) as f64, 2000.0) as usize;
    SegmentLimits { target, max }
}

/// Required input ports that nothing is wired into, as `node.port` strings.
pub fn missing_inputs(graph: &Graph) -> Vec<String> {
    let mut wired: BTreeSet<(String, String)> = BTreeSet::new();
    for edge in &graph.edges {
        if let Some((_, input)) = ports_for(graph, edge) {
            wired.insert((edge.to.clone(), input));
        }
    }
    let mut missing = Vec::new();
    for node in &graph.nodes {
        // 画布上常留下未接线的孤立节点；它们不参与运行，也不应整图秒失败。
        if !node_has_edge(graph, &node.id) {
            continue;
        }
        missing.extend(node_missing_required(graph, node, &wired));
    }
    missing
}

/// One external call a node needs; the host executes it and hands the result
/// back through [`WorkflowJob::resume`].
#[derive(Debug, Clone, PartialEq)]
pub enum RequestedCall {
    Chat(ChatCall),
    Image(ImageCall),
    Vision { prompt: String, images: Vec<PathBuf> },
    Speech(SpeechCall),
    Subtitles(SubtitlesCall),
}

/// What a driver should do next.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// One or more service calls that just became ready (parallel branches).
    /// Host should dispatch every item, then `resume(node, result)` per node.
    Calls(Vec<(String, RequestedCall)>),
    /// All ready work is in flight; wait for at least one `resume`.
    Waiting,
    /// Nothing left; read `report()`.
    Finished,
}

struct PendingCall {
    resume: Resume,
    started: Instant,
    logs: Vec<String>,
}

/// How a node finishes once its call returns.
#[derive(Debug, Clone)]
enum Resume {
    /// Straight from the reply payload.
    Plain,
    /// Model picked cut points; sanitize them and fall back to the rules.
    Segment {
        text: String,
        mode: String,
        limits: SegmentLimits,
    },
    /// TTS is synthesizing one block at a time; `next` is the in-flight index.
    TtsBatch {
        blocks: Vec<(String, String)>,
        voice: String,
        rate: Option<i32>,
        done: Vec<Value>,
        next: usize,
        stream: bool,
        cursor: f64,
    },
    /// Edit each text block with the LLM; `next` is the in-flight index.
    EditTextBatch {
        blocks: Vec<(String, String)>,
        system: String,
        model: Option<String>,
        temperature: Option<f64>,
        mode: String,
        done: Vec<Value>,
        next: usize,
    },
    /// One-shot oral blocks → same-count image prompts.
    ImagePrompt {
        blocks: Vec<(String, String)>,
        mode: String,
    },
    ImageParallel {
        blocks: Vec<ImageGenBlock>,
        slots: Vec<Option<Value>>,
        remaining: usize,
        fingerprint: String,
    },
}

/// One image_gen slot: model prompt + oral script used to snap onto TTS.
#[derive(Debug, Clone)]
struct ImageGenBlock {
    id: String,
    /// Text sent to the image model (may be an LLM rewrite).
    prompt: String,
    /// Original spoken line for emit text-adsorption onto clip char timelines.
    script: String,
}

enum NodeOutcome {
    Done(BTreeMap<String, Value>),
    Call {
        call: RequestedCall,
        resume: Resume,
    },
    FanOut {
        calls: Vec<(String, RequestedCall)>,
        resume: Resume,
    },
}

/// Result of applying a service reply: either finish the node or park again.
enum AfterCall {
    Done {
        outputs: BTreeMap<String, Value>,
        emit: Option<Value>,
    },
    Call {
        call: RequestedCall,
        resume: Resume,
        emit: Option<Value>,
    },
}

/// Resumable execution of one graph. Independent branches run in parallel:
/// every node whose upstreams are done may start; multiple service calls can
/// be in flight at once.
pub struct WorkflowJob {
    graph: Graph,
    base_dir: PathBuf,
    /// Run workdir (AI artifacts). Cache hits copy package files here.
    workdir: Option<PathBuf>,
    input: RunInput,
    order: Vec<String>,
    done: BTreeSet<String>,
    /// Nodes that failed; skipped by ready_ids so sibling branches can continue.
    failed: BTreeSet<String>,
    produced: BTreeMap<(String, String), Value>,
    report: RunReport,
    pending: BTreeMap<String, PendingCall>,
    /// Continuations queued by resume (TTS / edit_text batches).
    /// A vec so parallel resumes in one host tick cannot overwrite each other.
    next_calls: Vec<(String, RequestedCall)>,
    emitted_from: usize,
    finished: bool,
    /// Scratch logs while a node is being stepped locally.
    logs: Vec<String>,
    node_started: Option<Instant>,
}

impl WorkflowJob {
    /// `graph` is cloned so a host can keep the job across frames.
    pub fn new(graph: &Graph, base_dir: &Path, input: RunInput) -> Self {
        let mut report = RunReport::default();
        let missing = missing_inputs(graph);
        let mut order = Vec::new();
        if !missing.is_empty() {
            report.error = Some(format!("还有必需输入没接线：{}", missing.join("、")));
        } else if let Some(topo) = graph.topo_order() {
            let mut wired: BTreeSet<(String, String)> = BTreeSet::new();
            for edge in &graph.edges {
                if let Some((_, input)) = ports_for(graph, edge) {
                    wired.insert((edge.to.clone(), input));
                }
            }
            // 跳过未接线的孤立节点（缺必需口）；已接线却缺口的已在 missing 里拦下。
            order = topo
                .into_iter()
                .filter(|id| {
                    let Some(node) = graph.nodes.iter().find(|n| n.id == *id) else {
                        return false;
                    };
                    if node_has_edge(graph, id) {
                        return true;
                    }
                    node_missing_required(graph, node, &wired).is_empty()
                })
                .map(str::to_owned)
                .collect();
        } else {
            report.error = Some("图中有环，无法执行".into());
        }
        let finished = report.error.is_some();
        Self {
            graph: graph.clone(),
            base_dir: base_dir.to_path_buf(),
            workdir: None,
            input,
            order,
            done: BTreeSet::new(),
            failed: BTreeSet::new(),
            produced: BTreeMap::new(),
            report,
            pending: BTreeMap::new(),
            next_calls: Vec::new(),
            emitted_from: 0,
            finished,
            logs: Vec::new(),
            node_started: None,
        }
    }

    /// Where AI service files land for this run (needed to stage image cache hits).
    pub fn set_workdir(&mut self, workdir: PathBuf) {
        self.workdir = Some(workdir);
    }

    /// Host-side failure (e.g. timeline item decode) that should mark the run
    /// unsuccessful without inventing a fake node.
    pub fn note_host_error(&mut self, message: String) {
        if self.report.error.is_none() {
            self.report.error = Some(message);
        }
    }

    pub fn report(&self) -> &RunReport {
        &self.report
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    #[cfg(test)]
    fn order_for_test(&self) -> &[String] {
        &self.order
    }

    /// `(done, total)`.
    pub fn progress(&self) -> (usize, usize) {
        (self.done.len().min(self.order.len()), self.order.len())
    }

    /// A node waiting for its call, if any (for UI status).
    pub fn waiting_on(&self) -> Option<&str> {
        self.pending.keys().next().map(String::as_str)
    }

    /// Every node currently waiting on a service call.
    pub fn waiting_nodes(&self) -> Vec<&str> {
        self.pending.keys().map(String::as_str).collect()
    }

    /// Payloads not yet handed to the caller.
    pub fn drain_emitted(&mut self, sink: &mut dyn FnMut(Value)) {
        while self.emitted_from < self.report.emitted.len() {
            let value = self.report.emitted[self.emitted_from].clone();
            self.emitted_from += 1;
            sink(value);
        }
    }

    fn input_required(&self, node_id: &str, input: &str) -> bool {
        let Some(node) = self.graph.nodes.iter().find(|n| n.id == node_id) else {
            return true;
        };
        nodes::lookup(&node.node_type)
            .and_then(|spec| spec.inputs.iter().find(|p| p.name == input))
            .map(|p| p.required)
            .unwrap_or(true)
    }

    /// Required wired inputs must succeed. Optional wired inputs (e.g.
    /// `emit.visuals`) do not block when a required edge exists — TTS can
    /// enter the timeline first; [`reopen_optional_consumers`] places visuals
    /// when the image branch finishes. All-optional nodes (relay) wait for
    /// every wired optional so they do not run empty at graph start.
    fn preds_done(&self, id: &str) -> bool {
        let mut has_required_edge = false;
        let mut required_ok = true;
        let mut optional_wired = 0usize;
        let mut optional_settled = 0usize;
        for edge in self.graph.edges.iter().filter(|e| e.to == id) {
            let Some((_, input)) = ports_for(&self.graph, edge) else {
                if !self.done.contains(&edge.from) {
                    return false;
                }
                has_required_edge = true;
                continue;
            };
            if self.input_required(id, &input) {
                has_required_edge = true;
                if self.failed.contains(&edge.from) || !self.done.contains(&edge.from) {
                    required_ok = false;
                }
            } else {
                optional_wired += 1;
                if self.done.contains(&edge.from) || self.failed.contains(&edge.from) {
                    optional_settled += 1;
                }
            }
        }
        if has_required_edge {
            return required_ok;
        }
        optional_wired == 0 || optional_settled == optional_wired
    }

    fn ready_ids(&self) -> Vec<String> {
        self.order
            .iter()
            .filter(|id| {
                !self.done.contains(*id)
                    && !self.failed.contains(*id)
                    && !self.pending.contains_key(*id)
                    && self.preds_done(id)
            })
            .cloned()
            .collect()
    }

    fn mark_done(&mut self, id: String) {
        self.done.insert(id.clone());
        self.reopen_optional_consumers(&id);
    }

    /// When an optional upstream finishes after the consumer already ran (voice-first emit),
    /// reopen the consumer so visuals can be placed without waiting initially.
    fn reopen_optional_consumers(&mut self, completed: &str) {
        let mut reopen = Vec::new();
        for edge in &self.graph.edges {
            if edge.from != completed {
                continue;
            }
            let Some((_, input)) = ports_for(&self.graph, edge) else {
                continue;
            };
            if self.input_required(&edge.to, &input) {
                continue;
            }
            if self.done.contains(&edge.to) && !self.failed.contains(&edge.to) {
                reopen.push(edge.to.clone());
            }
        }
        for id in reopen {
            self.done.remove(&id);
            self.produced.retain(|(nid, _), _| nid != &id);
            self.finished = false;
            if let Some(node) = self.report.nodes.iter_mut().find(|n| n.id == id) {
                node.logs.push(format!(
                    "· 上游 {completed} 就绪，补跑可选输入"
                ));
                node.message = "等待补跑".into();
            }
        }
    }

    fn has_remaining_work(&self) -> bool {
        !self.pending.is_empty()
            || !self.next_calls.is_empty()
            || self.ready_ids().iter().next().is_some()
    }

    /// Run every ready local node; start every ready service node in parallel.
    pub fn advance(&mut self) -> Step {
        self.advance_with_limit(None)
    }

    /// Same state machine, but yield after `limit` locally completed nodes so
    /// a large relay graph cannot monopolize the UI frame.
    pub fn advance_budgeted(&mut self, limit: usize) -> Step {
        self.advance_with_limit(Some(limit.max(1)))
    }

    fn advance_with_limit(&mut self, limit: Option<usize>) -> Step {
        if self.finished {
            return Step::Finished;
        }
        // Continuations from prior resumes, plus any newly ready service nodes.
        let mut new_calls = std::mem::take(&mut self.next_calls);
        let mut completed = 0usize;
        let mut budget_exhausted = false;
        loop {
            let ready = self.ready_ids();
            if ready.is_empty() {
                break;
            }
            let mut progressed_local = false;
            for id in ready {
                if self.done.contains(&id) || self.pending.contains_key(&id) {
                    continue;
                }
                let Some(node) = self.graph.nodes.iter().find(|n| n.id == id).cloned() else {
                    self.mark_done(id);
                    progressed_local = true;
                    continue;
                };
                let node_type = node.node_type.clone();
                self.node_started = Some(Instant::now());
                self.logs = vec![format!("▶ {}（{}）开始", node.id, node_type)];
                let inputs = gather_inputs(&self.graph, &self.produced, &node);
                let mut logs = std::mem::take(&mut self.logs);
                let outcome = step_node(
                    &node,
                    &node_type,
                    &inputs,
                    &self.base_dir,
                    self.workdir.as_deref(),
                    &self.input,
                    &mut self.report,
                    &mut logs,
                );
                self.logs = logs;
                match outcome {
                    Ok(NodeOutcome::Done(outputs)) => {
                        if let Err(message) = self.store_node(&node, &node_type, outputs) {
                            self.fail(&node.id, &node_type, message);
                            if self.finished {
                                return Step::Finished;
                            }
                            progressed_local = true;
                            continue;
                        }
                        self.mark_done(node.id.clone());
                        progressed_local = true;
                        completed += 1;
                    }
                    Ok(NodeOutcome::Call { call, resume }) => {
                        self.pending.insert(
                            node.id.clone(),
                            PendingCall {
                                resume,
                                started: self.node_started.take().unwrap_or_else(Instant::now),
                                logs: std::mem::take(&mut self.logs),
                            },
                        );
                        new_calls.push((node.id.clone(), call));
                    }
                    Ok(NodeOutcome::FanOut { calls, resume }) => {
                        self.pending.insert(
                            node.id.clone(),
                            PendingCall {
                                resume,
                                started: self.node_started.take().unwrap_or_else(Instant::now),
                                logs: std::mem::take(&mut self.logs),
                            },
                        );
                        new_calls.extend(calls);
                    }
                    Err(message) => {
                        self.fail(&node.id, &node_type, message);
                        if self.finished {
                            return Step::Finished;
                        }
                        progressed_local = true;
                        completed += 1;
                    }
                }
                if limit.is_some_and(|max| completed >= max) {
                    budget_exhausted = true;
                    break;
                }
            }
            if budget_exhausted {
                break;
            }
            if !progressed_local {
                break;
            }
        }
        if !new_calls.is_empty() {
            return Step::Calls(new_calls);
        }
        if budget_exhausted {
            return Step::Waiting;
        }
        if self.pending.is_empty() {
            self.finished = true;
            Step::Finished
        } else {
            Step::Waiting
        }
    }


    /// Result of a service call for `node_id` (or `node_id#slot` for parallel fan-out).
    pub fn resume(&mut self, call_key: &str, result: Result<Value, String>) {
        let (node_id, slot) = split_call_key(call_key);
        if let Some(slot) = slot {
            self.resume_image_slot(node_id, slot, result);
            return;
        }
        let Some(pending) = self.pending.remove(node_id) else {
            return;
        };
        let Some(node) = self.graph.nodes.iter().find(|n| n.id == node_id).cloned() else {
            return;
        };
        let node_type = node.node_type.clone();
        self.logs = pending.logs;
        self.node_started = Some(pending.started);
        let resume_kind = pending.resume;
        match &result {
            Ok(reply) => {
                let brief = serde_json::to_string(reply).unwrap_or_default();
                let head: String = brief.chars().take(160).collect();
                self.logs.push(format!(
                    "← 收到回复 {} 字：{head}",
                    brief.chars().count()
                ));
            }
            Err(message) => self.logs.push(format!("✗ 调用失败：{message}")),
        }
        let finished = match (result, resume_kind) {
            (Err(message), _) => Err(message),
            (Ok(reply), Resume::Plain) => finish_call(&node, &node_type, reply).map(|outputs| {
                let emit = if node_type == "subtitles" {
                    outputs.get("items").cloned().filter(|v| {
                        v.get("items")
                            .and_then(Value::as_array)
                            .is_some_and(|a| !a.is_empty())
                    })
                } else {
                    None
                };
                AfterCall::Done { outputs, emit }
            }),
            (Ok(reply), Resume::Segment { text, mode, limits }) => {
                segments_from_reply(&reply, &text, &mode, limits).map(|outputs| AfterCall::Done {
                    outputs,
                    emit: None,
                })
            }
            (Ok(reply), Resume::TtsBatch {
                blocks,
                voice,
                rate,
                done,
                next,
                stream,
                cursor,
            }) => resume_tts_batch(
                reply, blocks, voice, rate, done, next, stream, cursor, &mut self.logs,
            ),
            (Ok(reply), Resume::EditTextBatch {
                blocks,
                system,
                model,
                temperature,
                mode,
                done,
                next,
            }) => resume_edit_text_batch(
                reply,
                blocks,
                system,
                model,
                temperature,
                mode,
                done,
                next,
                &mut self.logs,
            ),
            (Ok(reply), Resume::ImagePrompt { blocks, mode }) => {
                prompts_from_reply(&reply, &blocks, &mode, &mut self.logs).map(|outputs| {
                    AfterCall::Done {
                        outputs,
                        emit: None,
                    }
                })
            }
            (Ok(_), Resume::ImageParallel { .. }) => {
                Err("生图并行槽位须用 node#index 回填".into())
            }
        };
        match finished {
            Ok(AfterCall::Done { outputs, emit }) => {
                if let Some(items) = emit {
                    self.report.emitted.push(items);
                }
                if let Err(message) = self.store_node(&node, &node_type, outputs) {
                    self.fail(&node.id, &node_type, message);
                } else {
                    self.mark_done(node.id.clone());
                }
            }
            Ok(AfterCall::Call {
                call,
                resume,
                emit,
            }) => {
                if let Some(items) = emit {
                    self.report.emitted.push(items);
                }
                let brief = match (&call, &resume) {
                    (
                        RequestedCall::Speech(s),
                        Resume::TtsBatch { next, blocks, .. },
                    ) => format!(
                        "继续配音 {}/{}（{} 字）",
                        next + 1,
                        blocks.len(),
                        s.text.chars().count()
                    ),
                    (
                        RequestedCall::Chat(_),
                        Resume::EditTextBatch { next, blocks, .. },
                    ) => format!("继续改写 {}/{}", next + 1, blocks.len()),
                    _ => "继续外部调用".into(),
                };
                self.logs.push(format!("→ {brief}"));
                self.pending.insert(
                    node.id.clone(),
                    PendingCall {
                        resume,
                        started: self.node_started.take().unwrap_or_else(Instant::now),
                        logs: std::mem::take(&mut self.logs),
                    },
                );
                self.next_calls.push((node.id.clone(), call));
            }
            Err(message) => self.fail(&node.id, &node_type, message),
        }
    }

    fn resume_image_slot(&mut self, node_id: &str, slot: usize, result: Result<Value, String>) {
        let Some(node) = self.graph.nodes.iter().find(|n| n.id == node_id).cloned() else {
            return;
        };
        let node_type = node.node_type.clone();
        let Some(pending) = self.pending.get_mut(node_id) else {
            return;
        };
        self.logs = std::mem::take(&mut pending.logs);
        match &result {
            Ok(reply) => {
                let path = reply
                    .get("file")
                    .or_else(|| reply.get("path"))
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                let wh = match (
                    reply.get("width").and_then(Value::as_u64),
                    reply.get("height").and_then(Value::as_u64),
                ) {
                    (Some(w), Some(h)) => format!(" {w}×{h}"),
                    _ => String::new(),
                };
                self.logs
                    .push(format!("← 生图槽 {slot} 完成{wh}：{path}"));
            }
            Err(message) => self.logs.push(format!("✗ 生图槽 {slot} 失败：{message}")),
        }
        let Resume::ImageParallel {
            blocks,
            slots,
            remaining,
            ..
        } = &mut pending.resume
        else {
            self.pending.remove(node_id);
            self.fail(node_id, &node_type, "生图并行状态丢失".into());
            return;
        };
        if slot >= slots.len() {
            self.pending.remove(node_id);
            self.fail(node_id, &node_type, format!("生图槽位越界：{slot}"));
            return;
        }
        if slots[slot].is_some() {
            pending.logs = std::mem::take(&mut self.logs);
            return;
        }
        let block = blocks[slot].clone();
        let asset = match result {
            Ok(reply) => match normalize_image_asset(reply, &block) {
                Ok(asset) => Some(asset),
                Err(message) => {
                    self.logs
                        .push(format!("⚠ 生图槽 {slot} 结果无效，跳过：{message}"));
                    None
                }
            },
            Err(message) => {
                self.logs
                    .push(format!("⚠ 生图槽 {slot} 失败，跳过：{message}"));
                None
            }
        };
        let total = slots.len();
        if let Some(asset) = asset {
            let head_src = if block.script.trim().is_empty() {
                block.prompt.as_str()
            } else {
                block.script.as_str()
            };
            let head: String = head_src.chars().take(16).collect();
            let finished = slots.iter().filter(|s| s.is_some()).count() + 1;
            self.logs.push(format!(
                "✓ 生图完成 {finished}/{total}「{head}{}」",
                if head_src.chars().count() > 16 { "…" } else { "" }
            ));
            slots[slot] = Some(asset);
        } else {
            // Marker so the slot is not retried; filtered out when collecting outputs.
            slots[slot] = Some(Value::Null);
        }
        *remaining = remaining.saturating_sub(1);
        let remaining_now = *remaining;
        let finished_now = total - remaining_now;
        // 预览/中转镜像较重：首张、每 8 张、收尾各刷一次，避免 67 路 O(n²) 卡死 UI。
        let publish_now =
            remaining_now == 0 || finished_now == 1 || finished_now % 8 == 0;
        pending.logs = std::mem::take(&mut self.logs);
        if publish_now {
            let done: Vec<Value> = slots
                .iter()
                .filter_map(|s| s.as_ref().filter(|v| !v.is_null()).cloned())
                .collect();
            if !done.is_empty() {
                self.publish_partial_images(&node, &node_type, &done, total);
            }
        }
        if remaining_now > 0 {
            return;
        }
        let pending = self.pending.remove(node_id).expect("pending");
        let Resume::ImageParallel {
            slots,
            fingerprint,
            ..
        } = pending.resume
        else {
            return;
        };
        self.logs = pending.logs;
        self.node_started = Some(pending.started);
        let done: Vec<Value> = slots
            .into_iter()
            .flatten()
            .filter(|v| !v.is_null())
            .collect();
        if done.is_empty() {
            self.fail(node_id, &node_type, "全部生图槽均失败".into());
            return;
        }
        let skipped = total.saturating_sub(done.len());
        if skipped > 0 {
            self.logs
                .push(format!("⚠ 跳过 {skipped} 个失败槽，继续用 {} 张图", done.len()));
        }
        if let Err(message) = crate::nodes::image_cache::save(
            &self.base_dir,
            self.workdir.as_deref(),
            node_id,
            &fingerprint,
            &done,
        ) {
            self.logs
                .push(format!("⚠ 生图缓存未写入：{message}"));
        } else {
            self.logs.push("✓ 生图结果已写入节点缓存".into());
        }
        match image_outputs(done) {
            Ok(outputs) => {
                if let Err(message) = self.store_node(&node, &node_type, outputs) {
                    self.fail(node_id, &node_type, message);
                } else {
                    self.mark_done(node.id);
                }
            }
            Err(message) => self.fail(node_id, &node_type, message),
        }
    }

    fn publish_partial_images(
        &mut self,
        node: &GraphNode,
        node_type: &str,
        done: &[Value],
        total: usize,
    ) {
        if done.is_empty() {
            return;
        }
        let images = json!({ "images": done });
        self.produced
            .insert((node.id.clone(), "images".into()), images.clone());
        if let Some(first) = done.first() {
            self.produced
                .insert((node.id.clone(), "image".into()), first.clone());
        }
        let image = done
            .first()
            .and_then(|asset| asset.get("path").and_then(Value::as_str))
            .map(str::to_owned);
        let preview = serde_json::to_string_pretty(&images)
            .unwrap_or_default()
            .chars()
            .take(6000)
            .collect::<String>();
        let message = format!("进行中 {}/{total}", done.len());
        let logs = if let Some(pending) = self.pending.get(&node.id) {
            let mut merged = pending.logs.clone();
            merged.extend(self.logs.iter().cloned());
            merged
        } else {
            self.logs.clone()
        };
        if let Some(existing) = self.report.nodes.iter_mut().find(|n| n.id == node.id) {
            existing.ok = true;
            existing.message = message.clone();
            existing.outputs = vec!["image".into(), "images".into()];
            existing.preview = Some(preview.clone());
            existing.image = image.clone();
            existing.files = Some(images.clone());
            existing.logs = logs.clone();
        } else {
            self.report.nodes.push(NodeRun {
                id: node.id.clone(),
                node_type: node_type.to_owned(),
                ok: true,
                message: message.clone(),
                outputs: vec!["image".into(), "images".into()],
                preview: Some(preview.clone()),
                image: image.clone(),
                segments: None,
                files: Some(images.clone()),
                duration_ms: 0,
                output_chars: None,
                logs,
            });
        }
        self.mirror_partial_images_to_relays(&node.id, &images, image.as_deref(), &message);
    }

    /// So a downstream relay can preview stills before image_gen finishes.
    fn mirror_partial_images_to_relays(
        &mut self,
        from: &str,
        images: &Value,
        image: Option<&str>,
        message: &str,
    ) {
        let targets: Vec<(String, String)> = self
            .graph
            .edges
            .iter()
            .filter(|edge| {
                edge.from == from
                    && matches!(
                        edge.input.as_deref(),
                        None | Some("in") | Some("image") | Some("images") | Some("files")
                    )
            })
            .filter_map(|edge| {
                let node = self.graph.nodes.iter().find(|n| n.id == edge.to)?;
                (node.node_type == "relay").then(|| (edge.to.clone(), node.node_type.clone()))
            })
            .collect();
        for (id, node_type) in targets {
            if self.done.contains(&id) {
                continue;
            }
            if let Some(existing) = self.report.nodes.iter_mut().find(|n| n.id == id) {
                existing.ok = true;
                existing.message = message.to_owned();
                existing.outputs = vec!["files".into(), "images".into()];
                existing.preview = Some(
                    serde_json::to_string_pretty(images)
                        .unwrap_or_default()
                        .chars()
                        .take(6000)
                        .collect(),
                );
                existing.image = image.map(str::to_owned);
                existing.files = Some(images.clone());
                continue;
            }
            self.report.nodes.push(NodeRun {
                id,
                node_type,
                ok: true,
                message: message.to_owned(),
                outputs: vec!["files".into(), "images".into()],
                preview: Some(
                    serde_json::to_string_pretty(images)
                        .unwrap_or_default()
                        .chars()
                        .take(6000)
                        .collect(),
                ),
                image: image.map(str::to_owned),
                segments: None,
                files: Some(images.clone()),
                duration_ms: 0,
                output_chars: None,
                logs: vec![format!("预览上游生图：{message}")],
            });
        }
    }

    fn store_node(
        &mut self,
        node: &GraphNode,
        node_type: &str,
        outputs: BTreeMap<String, Value>,
    ) -> Result<(), String> {
        let mut names = Vec::new();
        for (port, value) in outputs {
            if let Some(spec) = nodes::lookup(node_type) {
                if let Some(port_spec) = spec.outputs.iter().find(|p| p.name == port) {
                    if let Err(message) = nodes::validate_payload(port_spec.kind, &value) {
                        return Err(format!("输出 {port} 不合格：{message}"));
                    }
                }
            }
            names.push(port.clone());
            self.produced.insert((node.id.clone(), port), value);
        }
        // Prefer a meaningful port for the inspector (relay's `files` is often empty
        // while `json`/`text` carry upstream segments / script).
        let preview_value = pick_preview_value(&names, |port| {
            self.produced.get(&(node.id.clone(), port.to_owned()))
        });
        let output_chars = preview_value.and_then(|value| {
            value
                .get("text")
                .and_then(Value::as_str)
                .map(|text| text.chars().count())
                .or_else(|| {
                    value.get("segments").and_then(Value::as_array).map(|list| {
                        list.iter()
                            .filter_map(|segment| segment.get("text").and_then(Value::as_str))
                            .map(|text| text.chars().count())
                            .sum()
                    })
                })
        });
        let preview = preview_value.map(|value| {
            serde_json::to_string_pretty(value)
                .unwrap_or_default()
                .chars()
                .take(6000)
                .collect::<String>()
        });
        let image = names
            .iter()
            .filter_map(|port| self.produced.get(&(node.id.clone(), port.clone())))
            .find_map(|value| first_image_path(value));
        let segments = self
            .produced
            .get(&(node.id.clone(), "segments".into()))
            .cloned();
        let files = self
            .produced
            .get(&(node.id.clone(), "files".into()))
            .cloned()
            .or_else(|| {
                self.produced
                    .get(&(node.id.clone(), "images".into()))
                    .cloned()
            });
        let duration_ms = self.take_duration();
        self.logs.push(format!("✓ 完成，用时 {:.1}s", duration_ms as f64 / 1000.0));
        let logs = std::mem::take(&mut self.logs);
        let run = NodeRun {
            id: node.id.clone(),
            node_type: node_type.to_owned(),
            ok: true,
            message: String::new(),
            outputs: names,
            preview,
            image,
            segments,
            files,
            duration_ms,
            output_chars,
            logs,
        };
        if let Some(existing) = self.report.nodes.iter_mut().find(|n| n.id == node.id) {
            *existing = run;
        } else {
            self.report.nodes.push(run);
        }
        Ok(())
    }

    fn take_duration(&mut self) -> u64 {
        self.node_started
            .take()
            .map(|started| started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0)
    }

    /// 当前节点的日志（运行中就能看到进度）。
    pub fn current_logs(&self) -> &[String] {
        if let Some(id) = self.waiting_on() {
            if let Some(pending) = self.pending.get(id) {
                return &pending.logs;
            }
        }
        &self.logs
    }

    /// 当前节点已经跑了多久（面板显示"运行中 12.3s"）。
    pub fn current_elapsed(&self) -> Option<Duration> {
        self.waiting_on()
            .and_then(|id| self.pending.get(id))
            .map(|p| p.started.elapsed())
            .or_else(|| self.node_started.map(|started| started.elapsed()))
    }

    fn fail(&mut self, id: &str, node_type: &str, message: String) {
        let duration_ms = self.take_duration();
        self.logs.push(format!("✗ 失败：{message}"));
        let logs = std::mem::take(&mut self.logs);
        let run = NodeRun {
            id: id.to_owned(),
            node_type: node_type.to_owned(),
            ok: false,
            message: message.clone(),
            outputs: Vec::new(),
            preview: None,
            image: None,
            segments: None,
            files: None,
            duration_ms,
            output_chars: None,
            logs,
        };
        if let Some(existing) = self.report.nodes.iter_mut().find(|n| n.id == id) {
            *existing = run;
        } else {
            self.report.nodes.push(run);
        }
        if self.report.error.is_none() {
            self.report.error = Some(format!("{id}: {message}"));
        }
        self.failed.insert(id.to_owned());
        self.pending.remove(id);
        self.next_calls.retain(|(call_key, _)| {
            let (node_id, _) = split_call_key(call_key);
            node_id != id
        });
        // 不拖死其它并行支路（例如生图失败仍可让 TTS 入轨）。
        if !self.has_remaining_work() {
            self.finished = true;
        }
    }
}

/// Choose which output to show in the inspector.
fn first_image_path(value: &Value) -> Option<String> {
    if let Some(path) = value.get("path").and_then(Value::as_str) {
        if value.get("files").is_none() && value.get("segments").is_none() {
            return Some(path.to_owned());
        }
    }
    if let Some(list) = value.get("images").and_then(Value::as_array) {
        for item in list {
            if let Some(path) = item.get("path").and_then(Value::as_str) {
                return Some(path.to_owned());
            }
        }
    }
    if let Some(list) = value.get("files").and_then(Value::as_array) {
        for item in list {
            let kind = item.get("kind").and_then(Value::as_str).unwrap_or("");
            if kind == "image" || kind == "video" {
                if let Some(path) = item.get("path").and_then(Value::as_str) {
                    return Some(path.to_owned());
                }
            }
        }
    }
    None
}

fn pick_preview_value<'a>(
    names: &[String],
    get: impl Fn(&str) -> Option<&'a Value>,
) -> Option<&'a Value> {
    const PREFER: &[&str] = &[
        "cues", "json", "text", "segments", "clips", "reply", "script", "images", "image", "files",
        "items",
    ];
    for key in PREFER {
        if let Some(value) = get(key) {
            if *key == "files" {
                let empty = value
                    .get("files")
                    .and_then(Value::as_array)
                    .is_none_or(|list| list.is_empty());
                if empty {
                    continue;
                }
            }
            return Some(value);
        }
    }
    names.first().and_then(|port| get(port))
}

/// Output port a service call fills.
fn finish_call(
    node: &GraphNode,
    node_type: &str,
    reply: Value,
) -> Result<BTreeMap<String, Value>, String> {
    let mut out = BTreeMap::new();
    match node_type {
        "image_gen" => {
            let asset = normalize_image_asset(
                reply,
                &ImageGenBlock {
                    id: "0".into(),
                    prompt: String::new(),
                    script: String::new(),
                },
            )?;
            let images = json!({ "images": [asset.clone()] });
            nodes::validate_payload(DataKind::ImageAssets, &images)
                .map_err(|e| format!("生图输出不合格：{e}"))?;
            out.insert("image".into(), asset);
            out.insert("images".into(), images);
        }
        "llm_chat" | "multimodal" => {
            let text = reply
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{node_type} 回复缺少 text"))?
                .to_owned();
            out.insert("reply".into(), json!({ "text": text }));
        }
        "tts" => {
            let clip = normalize_tts_clip(reply, "0", "")?;
            for (port, value) in tts_outputs(vec![clip], false)? {
                out.insert(port, value);
            }
        }
        "subtitles" => {
            let max_chars = param_num(node, "maxChars", 18.0)
                .round()
                .clamp(6.0, 32.0) as usize;
            let cues = normalize_subtitle_reply(&reply, max_chars);
            let items = subtitle_items_from_cues(&cues, node)?;
            out.insert("cues".into(), cues);
            out.insert("items".into(), items);
        }
        other => return Err(format!("{other} 没有对应的外部调用")),
    }
    Ok(out)
}

/// Turn cue list into timeline text items using node style / size / y.
fn subtitle_items_from_cues(cues: &Value, node: &GraphNode) -> Result<Value, String> {
    let list = cues
        .get("cues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if list.is_empty() {
        return Ok(json!({ "items": [] }));
    }
    let style_id = {
        let raw = param_str(node, "styleId");
        if raw.trim().is_empty() {
            "subtitle-kuaile".to_owned()
        } else {
            raw
        }
    };
    let font_size = param_num(node, "fontSize", 0.055).clamp(0.03, 0.14);
    let y = param_num(node, "y", 0.36).clamp(-0.45, 0.45);
    let mut items = Vec::new();
    for (index, cue) in list.iter().enumerate() {
        let text = cue
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if text.is_empty() {
            continue;
        }
        let start = cue
            .get("start")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
            .unwrap_or(0.0)
            .max(0.0);
        let end = cue
            .get("end")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
            .unwrap_or(start + 0.8)
            .max(start + 0.05);
        items.push(json!({
            "key": format!("sub/{index}"),
            "rev": format!("v1-{}-{}", style_id, text.chars().count()),
            "role": "subtitle",
            "lane": 0,
            "start": start,
            "duration": end - start,
            "transform": {
                "x": 0.0,
                "y": y,
                "scale": 1.0,
                "scaleX": 1.0,
                "scaleY": 1.0,
                "opacity": 1.0
            },
            "source": {
                "type": "text",
                "content": {
                    "text": text,
                    "styleId": style_id,
                    "layout": {
                        "fontSize": font_size,
                        "letterSpacing": 0.0,
                        "lineHeight": 1.15,
                        "align": "center"
                    }
                }
            }
        }));
    }
    Ok(json!({ "items": items }))
}

fn normalize_subtitle_reply(reply: &Value, max_chars: usize) -> Value {
    let mut normalized = Vec::new();
    let Some(cues) = reply.get("cues").and_then(Value::as_array) else {
        return json!({ "cues": [] });
    };
    for cue in cues {
        let text = cue.get("text").and_then(Value::as_str).unwrap_or_default();
        let start = cue
            .get("start")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
            .unwrap_or(0.0)
            .max(0.0);
        let end = cue
            .get("end")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
            .unwrap_or(start + 0.2)
            .max(start + 0.02);
        let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
        if chars.is_empty() {
            continue;
        }
        let chunks: Vec<&[char]> = chars.chunks(max_chars.max(1)).collect();
        let span = (end - start).max(0.02);
        for (index, chunk) in chunks.iter().enumerate() {
            let a = start + span * index as f64 / chunks.len() as f64;
            let b = if index + 1 == chunks.len() {
                end
            } else {
                start + span * (index + 1) as f64 / chunks.len() as f64
            };
            normalized.push(json!({
                "text": chunk.iter().collect::<String>(),
                "start": a,
                "end": b.max(a + 0.02),
            }));
        }
    }
    normalized.sort_by(|a, b| {
        a.get("start")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .partial_cmp(&b.get("start").and_then(Value::as_f64).unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    // Do not allow an upstream service to create overlapping cues. A tiny
    // gap is preferable to a subtitle that covers the next spoken phrase.
    for index in 1..normalized.len() {
        let previous_end = normalized[index - 1]
            .get("end")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        if normalized[index]
            .get("start")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            < previous_end
        {
            normalized[index]["start"] = json!(previous_end);
            let current_end = normalized[index]
                .get("end")
                .and_then(Value::as_f64)
                .unwrap_or(previous_end + 0.02);
            normalized[index]["end"] = json!(current_end.max(previous_end + 0.02));
        }
    }
    json!({ "cues": normalized })
}

/// Execute the whole graph with blocking services (CLI, tests).
pub fn run(
    graph: &Graph,
    base_dir: &Path,
    services: &mut dyn Services,
    input: &RunInput,
    emit: &mut dyn FnMut(Value),
) -> RunReport {
    let mut job = WorkflowJob::new(graph, base_dir, input.clone());
    loop {
        match job.advance() {
            Step::Calls(batch) => {
                for (node, call) in batch {
                    let result = services.execute(call);
                    job.resume(&node, result);
                    job.drain_emitted(emit);
                }
            }
            Step::Waiting => break,
            Step::Finished => break,
        }
    }
    job.drain_emitted(emit);
    job.report
}

#[allow(clippy::too_many_arguments)]
fn step_node(
    node: &GraphNode,
    node_type: &str,
    inputs: &BTreeMap<String, Value>,
    base_dir: &Path,
    workdir: Option<&Path>,
    input: &RunInput,
    report: &mut RunReport,
    logs: &mut Vec<String>,
) -> Result<NodeOutcome, String> {
    let mut out = BTreeMap::new();
    match node_type {
        "trigger" => {
            out.insert("start".into(), json!({ "trigger": true, "at": input.at.unwrap_or(0.0) }));
        }
        "media_bin" => {
            out = crate::nodes::step_media_bin(node, base_dir, logs)?;
        }
        "relay" => {
            out = crate::nodes::step_relay(node, inputs, base_dir, logs)?;
        }
        "script" => {
            let mut text = text_of(inputs.get("text"));
            if text.is_empty() {
                text = param_str(node, "text");
            }
            if text.is_empty() {
                text = input.script.clone();
            }
            if text.trim().is_empty() {
                return Err("没有文案：接上游 text，或在参数里写 text".into());
            }
            out.insert("script".into(), json!({ "text": text }));
        }
        "segment_script" => {
            let text = text_of(inputs.get("text"));
            if text.trim().is_empty() {
                return Err("分段需要上游 script_text".into());
            }
            let mode = match param_str(node, "mode").as_str() {
                "length" => "length",
                "paragraph" => "paragraph",
                "sentence" => "sentence",
                _ => "semantic",
            }
            .to_owned();
            let limits = segment_limits(node);
            if mode == "semantic" {
                let units = segment::split_units(&text, limits.max);
                let head: String = text.chars().take(40).collect();
                let model = node
                    .params
                    .get("model")
                    .and_then(Value::as_str)
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or("跟随 AI 设置");
                logs.push(format!(
                    "→ 语义分段 [{model}]：{} 字 / {} 个语义单元，目标 {} 字（开头：{head}）",
                    text.chars().count(),
                    units.len(),
                    limits.target
                ));
                let messages =
                    segment::build_segment_messages(&units, limits, &param_str(node, "title"));
                return Ok(NodeOutcome::Call {
                    call: RequestedCall::Chat(ChatCall {
                        messages,
                        json: true,
                        temperature: Some(0.2),
                        model: node
                            .params
                            .get("model")
                            .and_then(Value::as_str)
                            .filter(|name| !name.trim().is_empty())
                            .map(str::to_owned),
                    }),
                    resume: Resume::Segment { text, mode, limits },
                });
            }
            return Ok(NodeOutcome::Done(BTreeMap::from([(
                "segments".to_owned(),
                segment_by_rules(&text, &mode, limits),
            )])));
        }
        "llm_chat" => {
            let mut prompt = text_of(inputs.get("prompt"));
            if prompt.trim().is_empty() {
                prompt = param_str(node, "prompt");
            }
            if prompt.trim().is_empty() {
                return Err("llm_chat 缺 prompt（接线或写参数 prompt）".into());
            }
            let head: String = prompt.chars().take(60).collect();
            let model = node
                .params
                .get("model")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .unwrap_or("跟随 AI 设置");
            logs.push(format!(
                "→ 对话请求 [{model}]：prompt {} 字（开头：{head}）",
                prompt.chars().count()
            ));
            let mut messages = Vec::new();
            let system = param_str(node, "system");
            if !system.trim().is_empty() {
                messages.push(ChatMessage {
                    role: todo_production::tasks::ChatRole::System,
                    content: system,
                });
            }
            messages.push(ChatMessage {
                role: todo_production::tasks::ChatRole::User,
                content: prompt,
            });
            let json = match node.params.get("json") {
                Some(Value::Bool(flag)) => bool::from(*flag),
                Some(Value::String(text)) => text == "json" || text == "true",
                _ => false,
            };
            return Ok(NodeOutcome::Call {
                call: RequestedCall::Chat(ChatCall {
                    messages,
                    json,
                    temperature: node.params.get("temperature").and_then(Value::as_f64),
                    model: node
                        .params
                        .get("model")
                        .and_then(Value::as_str)
                        .filter(|name| !name.trim().is_empty())
                        .map(str::to_owned),
                }),
                resume: Resume::Plain,
            });
        }
        "edit_text" => {
            let blocks = tts_blocks(inputs.get("text"))
                .map_err(|e| e.replace("tts", "编辑文本"))?;
            let system = {
                let s = param_str(node, "system");
                if s.trim().is_empty() {
                    "你是口播文案编辑。改写用户给出的这一段口播，保持原意与口语感，只输出改写后的正文，不要标题、编号或解释。".into()
                } else {
                    s
                }
            };
            let model = node
                .params
                .get("model")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .map(str::to_owned);
            let temperature = node.params.get("temperature").and_then(Value::as_f64);
            let mode = inputs
                .get("text")
                .and_then(|v| v.get("mode"))
                .and_then(Value::as_str)
                .filter(|m| matches!(*m, "semantic" | "length" | "paragraph" | "edit"))
                .unwrap_or("edit")
                .to_owned();
            let model_label = model.as_deref().unwrap_or("跟随 AI 设置");
            logs.push(format!(
                "→ 编辑文本：{} 段，模型 [{model_label}]",
                blocks.len()
            ));
            let first = &blocks[0].1;
            return Ok(NodeOutcome::Call {
                call: edit_text_call(&system, first, model.clone(), temperature),
                resume: Resume::EditTextBatch {
                    blocks,
                    system,
                    model,
                    temperature,
                    mode,
                    done: Vec::new(),
                    next: 0,
                },
            });
        }
        "image_prompt" => {
            let blocks = tts_blocks(inputs.get("text"))
                .map_err(|e| e.replace("tts", "生图提示词"))?;
            let system = {
                let s = param_str(node, "system");
                if s.trim().is_empty() {
                    nodes::IMAGE_PROMPT_DEFAULT_SYSTEM.to_owned()
                } else {
                    s
                }
            };
            let cast = param_str(node, "cast");
            let style = param_str(node, "style");
            let model = node
                .params
                .get("model")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .map(str::to_owned);
            let temperature = node
                .params
                .get("temperature")
                .and_then(Value::as_f64)
                .or(Some(0.3));
            let mode = inputs
                .get("text")
                .and_then(|v| v.get("mode"))
                .and_then(Value::as_str)
                .filter(|m| {
                    matches!(
                        *m,
                        "semantic" | "length" | "paragraph" | "sentence" | "edit" | "image_prompt"
                    )
                })
                .unwrap_or("image_prompt")
                .to_owned();
            let model_label = model.as_deref().unwrap_or("跟随 AI 设置");
            logs.push(format!(
                "→ 生图提示词：{} 块整篇一次，模型 [{model_label}]",
                blocks.len()
            ));
            return Ok(NodeOutcome::Call {
                call: image_prompt_call(&system, &blocks, &cast, &style, model, temperature),
                resume: Resume::ImagePrompt { blocks, mode },
            });
        }
        "image_gen" => {
            let blocks = match inputs.get("prompt") {
                Some(_) => image_gen_blocks(inputs.get("prompt"))
                    .map_err(|e| e.replace("tts", "生图"))?,
                None => {
                    let prompt = param_str(node, "prompt");
                    if prompt.trim().is_empty() {
                        return Err("image_gen 缺 prompt".into());
                    }
                    vec![ImageGenBlock {
                        id: "0".into(),
                        prompt: prompt.clone(),
                        script: prompt,
                    }]
                }
            };
            let style = param_str(node, "style");
            let mut references = Vec::new();
            if let Some(asset) = inputs.get("ref") {
                if let Some(path) = asset.get("path").and_then(Value::as_str) {
                    let path = path.trim();
                    if !path.is_empty() {
                        references.push(path.to_owned());
                    }
                }
            }
            let transparent = match node.params.get("transparent") {
                Some(Value::Bool(v)) => *v,
                Some(Value::String(s)) => matches!(s.as_str(), "true" | "1" | "yes"),
                _ => false,
            };
            let quality = node
                .params
                .get("quality")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && *s != "default")
                .map(str::to_owned);
            let size = node
                .params
                .get("size")
                .and_then(Value::as_str)
                .unwrap_or("1024x1024")
                .to_owned();
            let base_variant =
                node.params.get("variant").and_then(Value::as_u64).unwrap_or(0) as u32;
            // 「清除缓存」bumps epoch → new package fingerprint + new AI task key.
            let epoch = crate::nodes::image_cache::epoch(base_dir, &node.id);
            let variant = base_variant.wrapping_add(epoch.wrapping_mul(1_000_003));
            let model = node
                .params
                .get("model")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .map(str::to_owned);
            let fp_blocks: Vec<(String, String)> = blocks
                .iter()
                .map(|b| (b.id.clone(), b.prompt.clone()))
                .collect();
            let fp = crate::nodes::image_cache::image_gen_fingerprint(
                &fp_blocks,
                &style,
                &size,
                quality.as_deref(),
                transparent,
                base_variant,
                model.as_deref(),
                &references,
                epoch,
            );
            if let Some(cached) = crate::nodes::image_cache::try_load(
                base_dir,
                workdir.as_deref(),
                &node.id,
                &fp,
            ) {
                logs.push(format!(
                    "✓ 命中生图缓存（{} 张，配置未变）",
                    cached.len()
                ));
                return Ok(NodeOutcome::Done(image_outputs(cached)?));
            }
            let total_chars: usize = blocks.iter().map(|b| b.prompt.chars().count()).sum();
            if blocks.len() == 1 {
                logs.push(format!("→ 生图：{} 字", total_chars));
            } else {
                logs.push(format!(
                    "→ 生图：{} 段并行 / {} 字",
                    blocks.len(),
                    total_chars
                ));
            }
            if epoch > 0 {
                logs.push(format!("· 缓存世代 {epoch}（已清除过，将重新请求）"));
            }
            let n = blocks.len();
            let calls: Vec<(String, RequestedCall)> = blocks
                .iter()
                .enumerate()
                .map(|(index, block)| {
                    (
                        format!("{}#{index}", node.id),
                        image_call(
                            image_prompt(&block.prompt, &style),
                            &size,
                            quality.clone(),
                            transparent,
                            variant,
                            model.clone(),
                            references.clone(),
                        ),
                    )
                })
                .collect();
            return Ok(NodeOutcome::FanOut {
                calls,
                resume: Resume::ImageParallel {
                    blocks,
                    slots: vec![None; n],
                    remaining: n,
                    fingerprint: fp,
                },
            });
        }
        "multimodal" => {
            let prompt = text_of(inputs.get("prompt"));
            if prompt.trim().is_empty() {
                return Err("multimodal 缺 prompt".into());
            }
            let mut images = Vec::new();
            if let Some(bin) = inputs.get("images") {
                push_media(&mut images, bin, base_dir, MediaFilter::Image);
            }
            if let Some(one) = inputs.get("image") {
                push_media(&mut images, one, base_dir, MediaFilter::Image);
            }
            if images.is_empty() {
                return Err("multimodal 缺图片".into());
            }
            return Ok(NodeOutcome::Call {
                call: RequestedCall::Vision { prompt, images },
                resume: Resume::Plain,
            });
        }
        "tts" => {
            let blocks = tts_blocks(inputs.get("text"))?;
            let voice = param_str(node, "voice");
            let rate = node
                .params
                .get("rate")
                .and_then(Value::as_i64)
                .map(|v| v as i32);
            let stream = param_str(node, "handoff") == "stream";
            let total_chars: usize = blocks.iter().map(|(_, t)| t.chars().count()).sum();
            let mode = if stream {
                "流式（每段立即入轨）"
            } else {
                "阻塞（整批完成）"
            };
            if blocks.len() == 1 {
                logs.push(format!(
                    "→ 配音：{} 字，音色 {}，{}",
                    total_chars, voice, mode
                ));
            } else {
                logs.push(format!(
                    "→ 配音：{} 段 / {} 字，音色 {}，{}",
                    blocks.len(),
                    total_chars,
                    voice,
                    mode
                ));
            }
            let first_text = blocks[0].1.clone();
            return Ok(NodeOutcome::Call {
                call: RequestedCall::Speech(SpeechCall {
                    text: first_text,
                    voice: voice.clone(),
                    sample_rate: None,
                    rate,
                }),
                resume: Resume::TtsBatch {
                    blocks,
                    voice,
                    rate,
                    done: Vec::new(),
                    next: 0,
                    stream,
                    cursor: 0.0,
                },
            });
        }
        "subtitles" => {
            let script = text_of(inputs.get("script"));
            let speech = resolve_speech_audio_path(inputs.get("speech"), inputs.get("media"))
                .ok_or_else(|| {
                    "字幕缺声素材：请接 TTS.media，或 speech 带 file".to_string()
                })?;
            if script.trim().is_empty() {
                return Err("字幕缺 script 文案".into());
            }
            logs.push(format!("→ 字幕：按配音时间轴对齐 {} 字", script.chars().count()));
            return Ok(NodeOutcome::Call {
                call: RequestedCall::Subtitles(SubtitlesCall {
                    script,
                    speech,
                    max_chars: Some(
                        param_num(node, "maxChars", 18.0)
                            .round()
                            .clamp(6.0, 32.0) as usize,
                    ),
                }),
                resume: Resume::Plain,
            });
        }
        "emit" => {
            let clips = inputs.get("clips").cloned().or_else(|| {
                inputs.get("payload").and_then(|payload| {
                    if payload.get("clips").and_then(Value::as_array).is_some() {
                        Some(payload.clone())
                    } else if payload.get("file").and_then(Value::as_str).is_some()
                        && payload.get("duration").and_then(Value::as_f64).is_some()
                    {
                        Some(json!({ "clips": [payload.clone()] }))
                    } else {
                        None
                    }
                })
            });
            let voice = inputs.get("media");
            let visuals = inputs.get("visuals");
            let items = if let Some(clips) = clips {
                let matched = emit_on_clip_clock(&clips, voice, visuals)?;
                let voice_n = matched.voice;
                let visual_n = matched.visual;
                if visual_n > 0 {
                    logs.push(format!(
                        "→ 入轨：{voice_n} 段配音 + {visual_n} 路画面（按尺对齐）"
                    ));
                } else {
                    logs.push(format!("→ 入轨：按 {voice_n} 段配音顺序排轨"));
                }
                for note in matched.notes {
                    logs.push(note);
                }
                matched.items
            } else {
                let payload = inputs
                    .get("payload")
                    .cloned()
                    .ok_or_else(|| "入轨缺 payload 或 clips".to_string())?;
                if visuals.is_some() {
                    return Err(
                        "入轨接了 visuals，但缺少 clips：画面需要对齐配音时间轴".into(),
                    );
                }
                normalize_emit_payload(&payload)?
            };
            report.emitted.push(items.clone());
            out.insert("items".into(), items);
        }
        "emit_sequence" => {
            let payload = inputs
                .get("clips")
                .cloned()
                .ok_or_else(|| "顺序入轨缺 clips（时间尺）".to_string())?;
            let voice = inputs.get("media");
            let visuals = inputs.get("visuals");
            // Voice-first: place as soon as TTS clips+media are ready.
            // Reopen / streamed follow-up only pushes visuals (no duplicate voice).
            let redo = report.nodes.iter().any(|n| n.id == node.id && n.ok);
            let streamed = payload.get("streamed").and_then(Value::as_bool) == Some(true);
            if streamed && visuals.is_none() {
                logs.push("上游 TTS 已流式入轨，跳过重复排轨".into());
                out.insert("items".into(), json!({ "items": [] }));
            } else if (streamed || redo) && visuals.is_some() {
                let matched = emit_on_clip_clock(&payload, None, visuals)?;
                let visuals_only = visual_items_only(&matched.items);
                if matched.visual > 0 {
                    logs.push(format!(
                        "→ 顺序入轨：补 {} 路画面（配音已入轨）",
                        matched.visual
                    ));
                } else {
                    logs.push("→ 顺序入轨：配音已入轨，无可用画面".into());
                }
                for note in matched.notes {
                    logs.push(note);
                }
                let batch = json!({ "items": visuals_only });
                report.emitted.push(batch.clone());
                out.insert("items".into(), batch);
            } else {
                let matched = emit_on_clip_clock(&payload, voice, visuals)?;
                if matched.visual > 0 {
                    logs.push(format!(
                        "→ 顺序入轨：{} 段配音 + {} 路画面",
                        matched.voice, matched.visual
                    ));
                } else {
                    logs.push(format!("→ 顺序入轨：{} 段配音", matched.voice));
                }
                for note in matched.notes {
                    logs.push(note);
                }
                report.emitted.push(matched.items.clone());
                out.insert("items".into(), matched.items);
            }
        }
        "emit_visuals" => {
            // Clips are the sole clock; never place voice; never need wav.
            let payload = inputs
                .get("clips")
                .cloned()
                .ok_or_else(|| "画面入轨缺 clips（时间尺）".to_string())?;
            let visuals = inputs
                .get("visuals")
                .ok_or_else(|| "画面入轨缺 visuals".to_string())?;
            if payload
                .get("clips")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|c| c.get("file").is_some())
            {
                logs.push("⚠ clips 仍带 file；尺口不应传 wav，请接 TTS.clips".into());
            }
            let matched = emit_on_clip_clock(&payload, None, Some(visuals))?;
            let visuals_only = visual_items_only(&matched.items);
            if matched.visual > 0 {
                logs.push(format!(
                    "→ 画面入轨：{} 路按 clips 时间尺对齐",
                    matched.visual
                ));
            } else {
                logs.push("→ 画面入轨：无可用画面".into());
            }
            for note in matched.notes {
                logs.push(note);
            }
            let batch = json!({ "items": visuals_only });
            report.emitted.push(batch.clone());
            out.insert("items".into(), batch);
        }
        "label_shots" => {
            let shots = inputs
                .get("shots")
                .cloned()
                .ok_or_else(|| "label_shots 缺 shots".to_string())?;
            out.insert("plan".into(), label_shots(&shots));
        }
        "choose_motions" => {
            let plan = inputs
                .get("plan")
                .cloned()
                .ok_or_else(|| "choose_motions 缺 plan".to_string())?;
            out.insert("plan".into(), choose_motions(&plan, node));
        }
        "motion" => {
            let bound = bind_motion(inputs.get("elements"), node)?;
            let first = bound
                .get("images")
                .and_then(Value::as_array)
                .and_then(|list| list.first())
                .cloned()
                .ok_or_else(|| "动效没有产出元素".to_string())?;
            logs.push(format!(
                "→ 动效：{} 个元素",
                bound
                    .get("images")
                    .and_then(Value::as_array)
                    .map(|list| list.len())
                    .unwrap_or(0)
            ));
            out.insert("element".into(), first);
            out.insert("elements".into(), bound);
        }
        "ground_emit" => {
            let plan = inputs
                .get("plan")
                .cloned()
                .ok_or_else(|| "ground_emit 缺 plan".to_string())?;
            let items = normalize_emit_payload(&json!({ "items": ground_items(&plan) }))?;
            report.emitted.push(items.clone());
            out.insert("items".into(), items);
        }
        "picture_batch" => {
            let plan = inputs
                .get("plan")
                .cloned()
                .ok_or_else(|| "picture_batch 缺 plan".to_string())?;
            let items = normalize_emit_payload(&json!({ "items": picture_items(&plan) }))?;
            report.emitted.push(items.clone());
            out.insert("items".into(), items);
        }
        "split_shots" => {
            let speech = inputs
                .get("speech")
                .ok_or_else(|| "拆镜缺 speech".to_string())?;
            let cues = inputs.get("cues").cloned().unwrap_or_else(|| json!({ "cues": [] }));
            let segments = inputs.get("segments");
            out.insert(
                "shots".into(),
                split_shots_with_segments(speech, &cues, segments),
            );
        }
        "done" => {}
        other if other.starts_with("python:") => {
            return Err(format!("python 节点需宿主 worker 池，本轮未接入：{other}"));
        }
        other => {
            return Err(format!("节点尚未实现：{other}"));
        }
    }
    Ok(NodeOutcome::Done(out))
}

fn text_item(
    key: String,
    role: &str,
    text: &str,
    start: f64,
    end: f64,
) -> Value {
    json!({
        "key": key,
        "rev": format!("v1-{}", text.chars().count()),
        "role": role,
        "lane": 0,
        "start": start.max(0.0),
        "duration": (end - start).max(0.05),
        "source": {
            "type": "text",
            "content": {
                "text": text,
                "styleId": if role == "subtitle" { "subtitle" } else { "default" },
                "layout": {
                    "fontSize": if role == "subtitle" { 0.055 } else { 0.08 },
                    "letterSpacing": 0.0,
                    "lineHeight": 1.15,
                    "align": "center"
                }
            }
        }
    })
}

fn normalize_emit_one(value: &Value, index: usize) -> Option<Value> {
    if value
        .get("key")
        .and_then(Value::as_str)
        .is_some()
        && value.get("rev").is_some()
        && value.get("role").is_some()
        && value.get("source").is_some()
    {
        return Some(value.clone());
    }
    if let (Some(file), Some(duration)) = (
        value.get("file").and_then(Value::as_str),
        value.get("duration").and_then(Value::as_f64),
    ) {
        return Some(json!({
            "key": "voice",
            "rev": format!("v1-{file}"),
            "role": "voice",
            "lane": 0,
            "start": 0.0,
            "duration": duration.max(0.05),
            "source": { "type": "file", "file": file, "in": 0.0 }
        }));
    }
    if let Some(path) = value
        .get("path")
        .or_else(|| value.get("file"))
        .and_then(Value::as_str)
    {
        let start = value.get("start").and_then(Value::as_f64).unwrap_or(0.0);
        let end = value
            .get("end")
            .and_then(Value::as_f64)
            .unwrap_or(start + 1.0);
        let duration = (end - start).max(0.05);
        let mut item = json!({
            "key": format!("visual/{index}"),
            "rev": format!("v1-{path}"),
            "role": "visual",
            "lane": 0,
            "start": start.max(0.0),
            "duration": duration,
            "source": { "type": "file", "file": path, "in": 0.0 }
        });
        attach_item_motions(&mut item, value, duration);
        return Some(item);
    }
    if let Some(prompt) = value.get("prompt").and_then(Value::as_str) {
        let start = value.get("start").and_then(Value::as_f64).unwrap_or(0.0);
        let end = value
            .get("end")
            .and_then(Value::as_f64)
            .unwrap_or(start + 1.0);
        let duration = (end - start).max(0.05);
        let mut item = text_item(
            format!("visual/{index}"),
            "visual",
            prompt,
            start,
            end,
        );
        attach_item_motions(&mut item, value, duration);
        return Some(item);
    }
    let text = value.get("text").and_then(Value::as_str)?;
    let start = value.get("start").and_then(Value::as_f64).unwrap_or(0.0);
    let end = value
        .get("end")
        .and_then(Value::as_f64)
        .unwrap_or(start + 1.0);
    let role = value
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("subtitle");
    Some(text_item(
        format!("sub/{index}"),
        if matches!(role, "subtitle" | "text") {
            role
        } else {
            "subtitle"
        },
        text,
        start,
        end,
    ))
}

fn normalize_emit_payload(payload: &Value) -> Result<Value, String> {
    let values: Vec<Value> = if let Some(items) = payload
        .get("items")
        .and_then(Value::as_array)
    {
        items.clone()
    } else if let Some(images) = payload.get("images").and_then(Value::as_array) {
        images.clone()
    } else if let Some(cues) = payload.get("cues").and_then(Value::as_array) {
        cues.clone()
    } else if let Some(array) = payload.as_array() {
        array.clone()
    } else {
        vec![payload.clone()]
    };
    let items: Vec<Value> = values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| normalize_emit_one(value, index))
        .collect();
    if items.is_empty() {
        return Err("入轨没有可用条目：需要 file、text 或 todo-items/1".into());
    }
    Ok(json!({ "items": items }))
}

/// Collect TTS blocks: prefer `segments[]`, else one block from joined/plain text.
fn tts_blocks(source: Option<&Value>) -> Result<Vec<(String, String)>, String> {
    let Some(value) = source else {
        return Err("tts 缺文本".into());
    };
    if let Some(list) = value.get("segments").and_then(Value::as_array) {
        let mut blocks = Vec::new();
        for (index, segment) in list.iter().enumerate() {
            let text = segment
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if text.is_empty() {
                continue;
            }
            let id = match segment.get("id") {
                Some(Value::String(s)) if !s.is_empty() => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                _ => index.to_string(),
            };
            blocks.push((id, text.to_owned()));
        }
        if blocks.is_empty() {
            return Err("tts：segments 里没有可用文本".into());
        }
        return Ok(blocks);
    }
    let text = text_of(Some(value));
    if text.trim().is_empty() {
        return Err("tts 缺文本".into());
    }
    Ok(vec![("0".into(), text)])
}

/// Like [`tts_blocks`], but keeps oral `script` alongside rewritten `text` prompts.
fn image_gen_blocks(source: Option<&Value>) -> Result<Vec<ImageGenBlock>, String> {
    let Some(value) = source else {
        return Err("tts 缺文本".into());
    };
    if let Some(list) = value.get("segments").and_then(Value::as_array) {
        let mut blocks = Vec::new();
        for (index, segment) in list.iter().enumerate() {
            let prompt = segment
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if prompt.is_empty() {
                continue;
            }
            let id = match segment.get("id") {
                Some(Value::String(s)) if !s.is_empty() => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                _ => index.to_string(),
            };
            let script = segment
                .get("script")
                .or_else(|| segment.get("sourceText"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(prompt)
                .to_owned();
            blocks.push(ImageGenBlock {
                id,
                prompt: prompt.to_owned(),
                script,
            });
        }
        if blocks.is_empty() {
            return Err("tts：segments 里没有可用文本".into());
        }
        return Ok(blocks);
    }
    let text = text_of(Some(value));
    if text.trim().is_empty() {
        return Err("tts 缺文本".into());
    }
    Ok(vec![ImageGenBlock {
        id: "0".into(),
        prompt: text.clone(),
        script: text,
    }])
}

fn normalize_tts_clip(mut reply: Value, id: &str, fallback_text: &str) -> Result<Value, String> {
    let file = reply
        .get("file")
        .and_then(Value::as_str)
        .ok_or_else(|| "配音结果缺少 file".to_string())?
        .to_owned();
    let lower = file.to_ascii_lowercase();
    if !(lower.ends_with(".wav")
        || lower.ends_with(".mp3")
        || lower.ends_with(".m4a")
        || lower.ends_with(".ogg"))
    {
        return Err("配音 file 须为 wav/mp3/m4a/ogg".into());
    }
    let duration = reply
        .get("duration")
        .and_then(Value::as_f64)
        .ok_or_else(|| "配音结果缺少 duration".to_string())?;
    if !duration.is_finite() || duration < 0.0 {
        return Err("配音 duration 非法".into());
    }
    let timeline = reply
        .get_mut("timeline")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "配音结果缺少 timeline（逐字时间戳）".to_string())?;
    if timeline
        .get("chars")
        .and_then(Value::as_array)
        .is_none_or(|c| c.is_empty())
    {
        return Err("配音 timeline.chars 不能为空".into());
    }
    if timeline.get("text").and_then(Value::as_str).is_none_or(|t| t.is_empty())
        && !fallback_text.is_empty()
    {
        timeline.insert("text".into(), json!(fallback_text));
    }
    if timeline.get("duration").and_then(Value::as_f64).is_none() {
        timeline.insert("duration".into(), json!(duration));
    }
    let _fixed = sanitize_timeline_chars(timeline, duration)?;
    reply["id"] = json!(id);
    if let Err(e) = nodes::validate_payload(DataKind::SpeechClip, &reply) {
        rebuild_linear_timeline(&mut reply, duration, fallback_text)?;
        nodes::validate_payload(DataKind::SpeechClip, &reply)
            .map_err(|e2| format!("配音结果不合格：{e}；重建后仍失败：{e2}"))?;
    }
    Ok(reply)
}

fn sanitize_timeline_chars(
    timeline: &mut serde_json::Map<String, Value>,
    duration: f64,
) -> Result<usize, String> {
    let Some(chars) = timeline.get_mut("chars").and_then(Value::as_array_mut) else {
        return Err("配音 timeline.chars 须为数组".into());
    };
    let dur = duration.max(0.02);
    let mut prev_end = 0.0_f64;
    let mut fixed = 0usize;
    for (i, ch) in chars.iter_mut().enumerate() {
        let Some(obj) = ch.as_object_mut() else {
            return Err(format!("timeline.chars[{i}] 须为对象"));
        };
        let text = obj
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if text.is_empty() {
            return Err(format!("timeline.chars[{i}].text 不能为空"));
        }
        let raw_start = obj.get("start").and_then(Value::as_f64).filter(|v| v.is_finite());
        let raw_end = obj.get("end").and_then(Value::as_f64).filter(|v| v.is_finite());
        let mut start = raw_start.unwrap_or(prev_end).max(0.0);
        let mut end = raw_end.unwrap_or(start + 0.02);
        if start + 1e-4 < prev_end {
            start = prev_end;
            fixed += 1;
        }
        if end < start + 0.01 {
            end = (start + 0.02).min(dur);
            fixed += 1;
        }
        if end > dur {
            end = dur;
            fixed += 1;
        }
        if start > end {
            start = (end - 0.02).max(0.0).min(end);
            fixed += 1;
        }
        if start >= dur {
            start = (dur - 0.02).max(0.0);
            end = dur;
            fixed += 1;
        }
        obj.insert("start".into(), json!(start));
        obj.insert("end".into(), json!(end));
        prev_end = end.max(prev_end);
    }
    let tl_dur = timeline
        .get("duration")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if !tl_dur.is_finite() || (tl_dur - duration).abs() > 0.05 {
        timeline.insert("duration".into(), json!(duration));
    }
    Ok(fixed)
}

fn rebuild_linear_timeline(
    reply: &mut Value,
    duration: f64,
    fallback_text: &str,
) -> Result<(), String> {
    let text = reply
        .pointer("/timeline/text")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .unwrap_or(fallback_text)
        .to_owned();
    if text.is_empty() {
        return Err("无法重建时间轴：没有文本".into());
    }
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len().max(1) as f64;
    let dur = duration.max(0.02);
    let stamps: Vec<Value> = chars
        .into_iter()
        .enumerate()
        .map(|(i, ch)| {
            let start = dur * i as f64 / n;
            let end = dur * (i + 1) as f64 / n;
            json!({ "text": ch.to_string(), "start": start, "end": end })
        })
        .collect();
    reply["timeline"] = json!({
        "text": text,
        "duration": dur,
        "chars": stamps,
    });
    Ok(())
}

fn tts_outputs(done: Vec<Value>, streamed: bool) -> Result<BTreeMap<String, Value>, String> {
    if done.is_empty() {
        return Err("tts 没有生成任何音频".into());
    }
    // Split: clips = clock (no wav); media = voice files aligned by id.
    let mut rulers = Vec::with_capacity(done.len());
    let mut files = Vec::with_capacity(done.len());
    for (index, clip) in done.iter().enumerate() {
        let id = clip_id(clip, index);
        let file = clip
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("tts clips[{index}] 缺 file"))?;
        files.push(json!({
            "path": file,
            "kind": "audio",
            "id": id,
            "name": file.rsplit('/').next().unwrap_or(file),
        }));
        let mut ruler = clip.clone();
        if let Some(obj) = ruler.as_object_mut() {
            obj.remove("file");
        }
        rulers.push(ruler);
    }
    let mut clips = json!({ "clips": rulers });
    if streamed {
        clips["streamed"] = json!(true);
    }
    let check = json!({ "clips": clips["clips"].clone() });
    nodes::validate_payload(DataKind::SpeechClips, &check)
        .map_err(|e| format!("clips 不合格：{e}"))?;
    let media = json!({ "files": files });
    nodes::validate_payload(DataKind::MediaBin, &media)
        .map_err(|e| format!("media 不合格：{e}"))?;
    Ok(BTreeMap::from([
        ("clips".into(), clips),
        ("media".into(), media),
    ]))
}

/// Audio path for subtitle / legacy sinks: prefer embedded `file`, else MediaBin.
fn resolve_speech_audio_path(speech: Option<&Value>, media: Option<&Value>) -> Option<String> {
    if let Some(speech) = speech {
        if let Some(file) = speech.get("file").and_then(Value::as_str) {
            return Some(file.to_owned());
        }
        if let Some(file) = speech
            .get("clips")
            .and_then(Value::as_array)
            .and_then(|list| list.first())
            .and_then(|c| c.get("file"))
            .and_then(Value::as_str)
        {
            return Some(file.to_owned());
        }
    }
    media
        .and_then(|m| m.get("files"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|f| f.get("kind").and_then(Value::as_str).unwrap_or("audio") == "audio")
        .and_then(|f| f.get("path").and_then(Value::as_str))
        .map(str::to_owned)
}

fn voice_item_from_clip(clip: &Value, start: f64) -> Result<Value, String> {
    let file = clip
        .get("file")
        .and_then(Value::as_str)
        .ok_or_else(|| "clip 缺 file".to_string())?;
    let duration = clip
        .get("duration")
        .and_then(Value::as_f64)
        .ok_or_else(|| "clip 缺 duration".to_string())?
        .max(0.05);
    let id = clip
        .get("id")
        .and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "0".into());
    Ok(json!({
        "key": format!("voice/{id}"),
        "rev": format!("v1-{file}"),
        "role": "voice",
        "lane": 0,
        "start": start.max(0.0),
        "duration": duration,
        "source": { "type": "file", "file": file, "in": 0.0 }
    }))
}

fn resume_tts_batch(
    reply: Value,
    blocks: Vec<(String, String)>,
    voice: String,
    rate: Option<i32>,
    mut done: Vec<Value>,
    next: usize,
    stream: bool,
    mut cursor: f64,
    logs: &mut Vec<String>,
) -> Result<AfterCall, String> {
    let (id, text) = blocks
        .get(next)
        .ok_or_else(|| format!("tts 批次越界：next={next} / {}", blocks.len()))?
        .clone();
    let clip = normalize_tts_clip(reply, &id, &text)?;
    let duration = clip
        .get("duration")
        .and_then(Value::as_f64)
        .unwrap_or(0.05)
        .max(0.05);
    let head: String = text.chars().take(16).collect();
    logs.push(format!(
        "✓ 配音完成 {}/{}「{head}{}」{:.2}s",
        next + 1,
        blocks.len(),
        if text.chars().count() > 16 { "…" } else { "" },
        duration
    ));
    let mut emit = None;
    if stream {
        let item = voice_item_from_clip(&clip, cursor)?;
        logs.push(format!(
            "→ 流式入轨 @ {:.2}s（第 {} 段）",
            cursor,
            next + 1
        ));
        emit = Some(json!({ "items": [item] }));
        cursor += duration;
    }
    done.push(clip);
    let next = next + 1;
    if next < blocks.len() {
        let text = blocks[next].1.clone();
        return Ok(AfterCall::Call {
            call: RequestedCall::Speech(SpeechCall {
                text,
                voice: voice.clone(),
                sample_rate: None,
                rate,
            }),
            resume: Resume::TtsBatch {
                blocks,
                voice,
                rate,
                done,
                next,
                stream,
                cursor,
            },
            emit,
        });
    }
    tts_outputs(done, stream).map(|outputs| AfterCall::Done { outputs, emit })
}

fn edit_text_call(
    system: &str,
    block: &str,
    model: Option<String>,
    temperature: Option<f64>,
) -> RequestedCall {
    RequestedCall::Chat(ChatCall {
        messages: vec![
            ChatMessage {
                role: todo_production::tasks::ChatRole::System,
                content: system.to_owned(),
            },
            ChatMessage {
                role: todo_production::tasks::ChatRole::User,
                content: format!(
                    "请改写下面这一段口播文案，只输出改写后的正文：\n\n{block}"
                ),
            },
        ],
        json: false,
        temperature,
        model,
    })
}

fn image_prompt_call(
    system: &str,
    blocks: &[(String, String)],
    cast: &str,
    style: &str,
    model: Option<String>,
    temperature: Option<f64>,
) -> RequestedCall {
    let list: Vec<Value> = blocks
        .iter()
        .map(|(id, text)| json!({ "id": id, "text": text }))
        .collect();
    let blocks_json =
        serde_json::to_string_pretty(&list).unwrap_or_else(|_| "[]".into());
    let mut user = String::new();
    let cast = cast.trim();
    let style = style.trim();
    if !cast.is_empty() {
        user.push_str("补充设定（可选）：");
        user.push_str(cast);
        user.push('\n');
    }
    if !style.is_empty() {
        user.push_str("画风：");
        user.push_str(style);
        user.push('\n');
    }
    user.push_str("口播块（按顺序，共 ");
    user.push_str(&blocks.len().to_string());
    user.push_str(" 条）：\n");
    user.push_str(&blocks_json);
    user.push_str(
        "\n\n请输出 JSON：{\"prompts\":[{\"id\":\"…\",\"prompt\":\"…\"},…]}，\
条数与 id 必须与上表完全一致。",
    );
    RequestedCall::Chat(ChatCall {
        messages: vec![
            ChatMessage {
                role: todo_production::tasks::ChatRole::System,
                content: system.to_owned(),
            },
            ChatMessage {
                role: todo_production::tasks::ChatRole::User,
                content: user,
            },
        ],
        json: true,
        temperature,
        model,
    })
}

fn prompts_from_reply(
    reply: &Value,
    blocks: &[(String, String)],
    mode: &str,
    logs: &mut Vec<String>,
) -> Result<BTreeMap<String, Value>, String> {
    let raw = reply
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if raw.is_empty() {
        return Err("生图提示词：模型没有返回正文".into());
    }
    let parsed = parse_json_block(raw)
        .or_else(|| serde_json::from_str::<Value>(raw).ok())
        .ok_or_else(|| "生图提示词：回复不是合法 JSON".to_string())?;
    let items = extract_prompt_items(&parsed)
        .ok_or_else(|| "生图提示词：JSON 里找不到 prompts 列表".to_string())?;
    if items.len() != blocks.len() {
        return Err(format!(
            "生图提示词：期望 {} 条，模型返回 {} 条（必须锁死同数量）",
            blocks.len(),
            items.len()
        ));
    }
    let mut by_id: BTreeMap<String, String> = BTreeMap::new();
    for (index, item) in items.iter().enumerate() {
        let prompt = item
            .get("prompt")
            .or_else(|| item.get("text"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("生图提示词：第 {} 条缺少 prompt", index + 1))?;
        let id = item
            .get("id")
            .and_then(|v| match v {
                Value::String(s) if !s.is_empty() => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .unwrap_or_else(|| blocks[index].0.clone());
        by_id.insert(id, prompt.to_owned());
    }
    let mut done = Vec::with_capacity(blocks.len());
    for (id, oral) in blocks {
        let prompt = by_id
            .get(id)
            .cloned()
            .or_else(|| {
                // Fall back to positional order if the model dropped/changed ids
                // but kept count (already checked).
                let index = blocks.iter().position(|(bid, _)| bid == id)?;
                items
                    .get(index)
                    .and_then(|item| {
                        item.get("prompt")
                            .or_else(|| item.get("text"))
                            .and_then(Value::as_str)
                    })
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
            })
            .ok_or_else(|| format!("生图提示词：缺少 id={id} 的画面描述"))?;
        // `text` = model prompt for image_gen; `script` = oral source for emit snap.
        done.push(json!({
            "id": id,
            "text": prompt,
            "script": oral,
            "chars": prompt.chars().count(),
        }));
    }
    let head: String = done
        .first()
        .and_then(|s| s.get("text").and_then(Value::as_str))
        .unwrap_or("")
        .chars()
        .take(24)
        .collect();
    logs.push(format!(
        "✓ 生图提示词：{} 条（首条「{head}{}」）",
        done.len(),
        if done
            .first()
            .and_then(|s| s.get("text").and_then(Value::as_str))
            .map(|s| s.chars().count())
            .unwrap_or(0)
            > 24
        {
            "…"
        } else {
            ""
        }
    ));
    image_prompt_outputs(mode, done)
}

fn extract_prompt_items(parsed: &Value) -> Option<&Vec<Value>> {
    parsed
        .get("prompts")
        .and_then(Value::as_array)
        .or_else(|| parsed.get("segments").and_then(Value::as_array))
        .or_else(|| parsed.as_array())
}

fn image_prompt_outputs(
    mode: &str,
    done: Vec<Value>,
) -> Result<BTreeMap<String, Value>, String> {
    if done.is_empty() {
        return Err("生图提示词没有产出任何文本块".into());
    }
    let full: String = done
        .iter()
        .filter_map(|s| s.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let payload = json!({
        "mode": mode,
        "text": full,
        "segments": done,
    });
    nodes::validate_payload(DataKind::Segments, &payload)
        .map_err(|e| format!("生图提示词输出不合格：{e}"))?;
    Ok(BTreeMap::from([("prompt".into(), payload)]))
}

fn reply_text(reply: &Value) -> Result<String, String> {
    let text = reply
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "编辑文本：模型没有返回正文".to_string())?;
    Ok(text.to_owned())
}

fn edit_text_outputs(mode: &str, done: Vec<Value>) -> Result<BTreeMap<String, Value>, String> {
    if done.is_empty() {
        return Err("编辑文本没有产出任何文本块".into());
    }
    let full: String = done
        .iter()
        .filter_map(|s| s.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");
    let payload = json!({
        "mode": mode,
        "text": full,
        "segments": done,
    });
    nodes::validate_payload(DataKind::Segments, &payload)
        .map_err(|e| format!("编辑文本输出不合格：{e}"))?;
    Ok(BTreeMap::from([("text".into(), payload)]))
}

fn resume_edit_text_batch(
    reply: Value,
    blocks: Vec<(String, String)>,
    system: String,
    model: Option<String>,
    temperature: Option<f64>,
    mode: String,
    mut done: Vec<Value>,
    next: usize,
    logs: &mut Vec<String>,
) -> Result<AfterCall, String> {
    let (id, original) = blocks
        .get(next)
        .ok_or_else(|| format!("编辑文本批次越界：next={next} / {}", blocks.len()))?
        .clone();
    let rewritten = reply_text(&reply)?;
    let chars = rewritten.chars().count();
    let head: String = rewritten.chars().take(16).collect();
    logs.push(format!(
        "✓ 改写完成 {}/{}「{head}{}」{} 字（原 {} 字）",
        next + 1,
        blocks.len(),
        if rewritten.chars().count() > 16 { "…" } else { "" },
        chars,
        original.chars().count()
    ));
    done.push(json!({
        "id": id,
        "text": rewritten,
        "chars": chars,
    }));
    let next = next + 1;
    if next < blocks.len() {
        let text = blocks[next].1.clone();
        return Ok(AfterCall::Call {
            call: edit_text_call(&system, &text, model.clone(), temperature),
            resume: Resume::EditTextBatch {
                blocks,
                system,
                model,
                temperature,
                mode,
                done,
                next,
            },
            emit: None,
        });
    }
    edit_text_outputs(&mode, done).map(|outputs| AfterCall::Done {
        outputs,
        emit: None,
    })
}

fn image_prompt(block: &str, style: &str) -> String {
    let style = style.trim();
    if style.is_empty() {
        block.to_owned()
    } else {
        format!("{block}，{style}")
    }
}

fn image_call(
    prompt: String,
    size: &str,
    quality: Option<String>,
    transparent: bool,
    variant: u32,
    model: Option<String>,
    references: Vec<String>,
) -> RequestedCall {
    RequestedCall::Image(ImageCall {
        prompt,
        size: size.to_owned(),
        quality,
        transparent,
        variant,
        model,
        references,
    })
}

fn normalize_image_asset(reply: Value, block: &ImageGenBlock) -> Result<Value, String> {
    let path = reply
        .get("path")
        .and_then(Value::as_str)
        .or_else(|| reply.get("file").and_then(Value::as_str))
        .ok_or_else(|| "生图结果缺少文件路径".to_string())?
        .to_owned();
    if path.is_empty() || path.contains("..") || path.starts_with('/') || path.contains('\\') {
        return Err("生图 path 非法".into());
    }
    let mut asset = json!({ "path": path, "id": block.id });
    let script = block.script.trim();
    let prompt = block.prompt.trim();
    // `text` / `script` must be oral wording so emit can snap onto TTS chars.
    // `prompt` keeps the (possibly English) image-model rewrite.
    if !script.is_empty() {
        asset["text"] = json!(script);
        asset["script"] = json!(script);
    } else if !prompt.is_empty() {
        asset["text"] = json!(prompt);
    }
    if !prompt.is_empty() && prompt != script {
        asset["prompt"] = json!(prompt);
    }
    for key in ["mime", "width", "height"] {
        if let Some(value) = reply.get(key) {
            asset[key] = value.clone();
        }
    }
    nodes::validate_payload(DataKind::ImageAsset, &asset)
        .map_err(|e| format!("生图结果不合格：{e}"))?;
    Ok(asset)
}

fn image_outputs(done: Vec<Value>) -> Result<BTreeMap<String, Value>, String> {
    if done.is_empty() {
        return Err("生图没有产出任何图片".into());
    }
    let first = done[0].clone();
    let images = json!({ "images": done });
    nodes::validate_payload(DataKind::ImageAssets, &images)
        .map_err(|e| format!("生图输出不合格：{e}"))?;
    Ok(BTreeMap::from([
        ("image".into(), first),
        ("images".into(), images),
    ]))
}

fn split_call_key(call_key: &str) -> (&str, Option<usize>) {
    match call_key.rsplit_once('#') {
        Some((node, slot)) => match slot.parse::<usize>() {
            Ok(index) => (node, Some(index)),
            Err(_) => (call_key, None),
        },
        None => (call_key, None),
    }
}

struct EmitMatched {
    items: Value,
    voice: usize,
    visual: usize,
    notes: Vec<String>,
}

#[derive(Clone)]
struct VisualMedia {
    id: Option<String>,
    path: String,
    #[allow(dead_code)] // image vs video; reserved for placement rules
    kind: &'static str,
    /// Original prompt / caption used to adsorb onto clip char timestamps.
    text: Option<String>,
    intro: Option<Value>,
    outro: Option<Value>,
    transition: Option<Value>,
}

struct AbsChar {
    /// Normalized character (same rules as `normalized_match_text`).
    ch: char,
    start: f64,
    end: f64,
}

/// Legacy helper: clips that still embed `file` + optional visuals.
#[cfg(test)]
fn emit_clips_with_visuals(
    payload: &Value,
    visuals: Option<&Value>,
) -> Result<EmitMatched, String> {
    emit_on_clip_clock(payload, None, visuals)
}

/// Place materials on the clip clock.
/// - `clips`: ruler only (`duration` / `timeline`); embedded `file` is legacy fallback
/// - `voice`: MediaBin audio materials (preferred for wav)
/// - `visuals`: picture / video materials
fn emit_on_clip_clock(
    payload: &Value,
    voice: Option<&Value>,
    visuals: Option<&Value>,
) -> Result<EmitMatched, String> {
    let clips: Vec<Value> = if let Some(list) = payload.get("clips").and_then(Value::as_array) {
        list.clone()
    } else if payload.get("duration").and_then(Value::as_f64).is_some() {
        vec![payload.clone()]
    } else {
        return Err("入轨需要 speech_clips.clips（时间尺）".into());
    };
    if clips.is_empty() {
        return Err("入轨：clips 为空".into());
    }
    let voice_files = voice
        .map(collect_voice_media)
        .transpose()?
        .unwrap_or_default();
    let mut voice_used = vec![false; voice_files.len()];
    let picture = visuals
        .map(collect_visual_media)
        .transpose()?
        .unwrap_or_default();
    let axis = build_abs_char_axis(&clips);
    let mut notes = Vec::new();
    let mut cursor = 0.0_f64;
    let mut items = Vec::new();
    let mut clip_windows = Vec::new();
    let mut voice_n = 0usize;

    for (index, clip) in clips.iter().enumerate() {
        let duration = clip
            .get("duration")
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("clips[{index}] 缺 duration"))?
            .max(0.05);
        let id = clip_id(clip, index);
        let file = pick_voice_file(&voice_files, &mut voice_used, &id, index).or_else(|| {
            clip.get("file")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        if let Some(file) = file {
            items.push(json!({
                "key": format!("voice/{id}"),
                // Path in the rev so a new wav counts as regenerated even when
                // duration matches a previously deleted clip's tombstone.
                "rev": format!("v1-{file}"),
                "role": "voice",
                "lane": 0,
                "start": cursor,
                "duration": duration,
                "source": { "type": "file", "file": file, "in": 0.0 }
            }));
            voice_n += 1;
        } else if voice.is_some() {
            return Err(format!(
                "clips[{index}]（id={id}）在 media 中找不到对应声素材"
            ));
        }
        clip_windows.push((id, cursor, duration));
        cursor += duration;
    }

    let mut used = vec![false; picture.len()];
    let mut visual_n = 0usize;
    let mut snapped = 0usize;
    let mut search_from = 0usize;

    // Pass 1: adsorb by caption text onto the spoken char axis (left → right).
    for (i, vis) in picture.iter().enumerate() {
        let Some(text) = vis.text.as_deref().filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        if let Some((start, end, next_from)) = snap_text_to_axis(&axis, text, search_from) {
            let id = vis.id.clone().unwrap_or_else(|| format!("snap-{i}"));
            items.push(visual_item(vis, &id, start, (end - start).max(0.05)));
            used[i] = true;
            visual_n += 1;
            snapped += 1;
            search_from = next_from;
        }
    }

    // Pass 2: remaining visuals → whole-clip windows by id / index.
    for (index, (id, start, duration)) in clip_windows.iter().enumerate() {
        if let Some(vis) = pick_visual_for_clip(&picture, &mut used, id, index) {
            items.push(visual_item(vis, id, *start, *duration));
            visual_n += 1;
        }
    }

    let unused: usize = used.iter().filter(|u| !**u).count();
    if snapped > 0 {
        notes.push(format!("→ 文案吸附：{snapped} 路画面挂上逐字时间轴"));
    }
    if !picture.is_empty() && unused > 0 {
        notes.push(format!("⚠ 还有 {unused} 个画面素材未匹配到口播"));
    }
    Ok(EmitMatched {
        items: json!({ "items": items }),
        voice: voice_n,
        visual: visual_n,
        notes,
    })
}

struct VoiceMedia {
    id: Option<String>,
    path: String,
}

fn collect_voice_media(value: &Value) -> Result<Vec<VoiceMedia>, String> {
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| "声素材 media 须为 media_bin（含 files）".to_string())?;
    let mut out = Vec::new();
    for (i, file) in files.iter().enumerate() {
        let path = file
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("media.files[{i}] 缺 path"))?;
        let kind = file.get("kind").and_then(Value::as_str).unwrap_or("audio");
        if kind != "audio" {
            continue;
        }
        out.push(VoiceMedia {
            id: value_id(file),
            path: path.to_owned(),
        });
    }
    if out.is_empty() {
        return Err("media 中没有 kind=audio 的声素材".into());
    }
    Ok(out)
}

fn pick_voice_file(
    media: &[VoiceMedia],
    used: &mut [bool],
    clip_id: &str,
    index: usize,
) -> Option<String> {
    if let Some((i, v)) = media
        .iter()
        .enumerate()
        .find(|(i, v)| !used[*i] && v.id.as_deref() == Some(clip_id))
    {
        used[i] = true;
        return Some(v.path.clone());
    }
    if index < media.len() && !used[index] {
        used[index] = true;
        return Some(media[index].path.clone());
    }
    None
}

fn build_abs_char_axis(clips: &[Value]) -> Vec<AbsChar> {
    let mut axis = Vec::new();
    let mut cursor = 0.0_f64;
    for clip in clips {
        let duration = clip
            .get("duration")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .max(0.0);
        let chars = clip
            .pointer("/timeline/chars")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if chars.is_empty() {
            // Fall back: spread timeline.text evenly across the clip window.
            let text = clip
                .pointer("/timeline/text")
                .and_then(Value::as_str)
                .unwrap_or("");
            let norm: Vec<char> = normalized_match_text(text).chars().collect();
            let n = norm.len().max(1) as f64;
            for (i, ch) in norm.into_iter().enumerate() {
                let start = cursor + duration * i as f64 / n;
                let end = cursor + duration * (i as f64 + 1.0) / n;
                axis.push(AbsChar { ch, start, end });
            }
        } else {
            for ch in &chars {
                let raw = ch.get("text").and_then(Value::as_str).unwrap_or("");
                let start = ch.get("start").and_then(Value::as_f64).unwrap_or(0.0);
                let end = ch
                    .get("end")
                    .and_then(Value::as_f64)
                    .unwrap_or(start + 0.02);
                for c in normalized_match_text(raw).chars() {
                    axis.push(AbsChar {
                        ch: c,
                        start: cursor + start.max(0.0),
                        end: cursor + end.max(start),
                    });
                }
            }
        }
        cursor += duration;
    }
    axis
}

/// Find `needle` in the abs char axis starting at `from`; returns
/// (abs_start, abs_end, next_search_index).
fn snap_text_to_axis(axis: &[AbsChar], needle: &str, from: usize) -> Option<(f64, f64, usize)> {
    let needle: Vec<char> = normalized_match_text(needle).chars().collect();
    if needle.is_empty() || axis.is_empty() {
        return None;
    }
    let hay: Vec<char> = axis.iter().map(|c| c.ch).collect();
    if needle.len() > hay.len() {
        return None;
    }
    let end_limit = hay.len() - needle.len();
    let search = |from: usize| -> Option<(f64, f64, usize)> {
        for start in from..=end_limit {
            if hay[start..start + needle.len()] == needle[..] {
                let abs_start = axis[start].start;
                let abs_end = axis[start + needle.len() - 1].end;
                return Some((abs_start, abs_end, start + needle.len()));
            }
        }
        None
    };
    search(from.min(end_limit + 1)).or_else(|| if from > 0 { search(0) } else { None })
}

fn clip_id(clip: &Value, index: usize) -> String {
    clip.get("id")
        .and_then(|v| match v {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .unwrap_or_else(|| index.to_string())
}

fn visual_items_only(items: &Value) -> Vec<Value> {
    items
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("role").and_then(Value::as_str) == Some("visual"))
        .cloned()
        .collect()
}

fn visual_item(vis: &VisualMedia, id: &str, start: f64, duration: f64) -> Value {
    let duration = duration.max(0.05);
    // Do not attach unknown fields (e.g. `media`): Item uses deny_unknown_fields
    // and the host would reject the whole emit batch.
    let mut item = json!({
        "key": format!("visual/{id}"),
        "rev": format!("v1-{}", vis.path),
        "role": "visual",
        "lane": 1,
        "start": start.max(0.0),
        "duration": duration,
        "source": { "type": "file", "file": vis.path, "in": 0.0 },
    });
    let mut source = json!({ "path": vis.path });
    if let Some(m) = &vis.intro {
        source["intro"] = m.clone();
    }
    if let Some(m) = &vis.outro {
        source["outro"] = m.clone();
    }
    if let Some(m) = &vis.transition {
        source["transition"] = m.clone();
    }
    attach_item_motions(&mut item, &source, duration);
    item
}

fn pick_visual_for_clip<'a>(
    media: &'a [VisualMedia],
    used: &mut [bool],
    clip_id: &str,
    index: usize,
) -> Option<&'a VisualMedia> {
    if let Some((i, _)) = media
        .iter()
        .enumerate()
        .find(|(i, v)| !used[*i] && v.id.as_deref() == Some(clip_id))
    {
        used[i] = true;
        return Some(&media[i]);
    }
    if index < media.len() && !used[index] {
        used[index] = true;
        return Some(&media[index]);
    }
    None
}

fn value_id(value: &Value) -> Option<String> {
    value.get("id").and_then(|v| match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn media_kind_from_path(path: &str) -> Option<&'static str> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
        || lower.ends_with(".bmp")
    {
        Some("image")
    } else if lower.ends_with(".mp4")
        || lower.ends_with(".mov")
        || lower.ends_with(".webm")
        || lower.ends_with(".mkv")
        || lower.ends_with(".m4v")
    {
        Some("video")
    } else {
        None
    }
}

fn motion_field(value: &Value, key: &str) -> Option<Value> {
    let raw = value
        .get(key)
        .or_else(|| value.get("motion").and_then(|m| m.get(key)))?;
    if raw.is_null() {
        return None;
    }
    let kind = raw.get("kind").and_then(Value::as_str)?;
    if kind.is_empty() || !todo_motion::valid_motion_id(kind) {
        return None;
    }
    let duration = raw
        .get("duration")
        .and_then(Value::as_f64)
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or(0.35);
    Some(json!({ "kind": kind, "duration": duration }))
}

fn visual_caption(source: Option<&Value>) -> Option<String> {
    let value = source?;
    // Prefer oral script over image-model prompt (prompt often cannot match TTS chars).
    for key in ["script", "sourceText", "text", "caption", "label", "prompt"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_owned());
            }
        }
    }
    None
}

fn push_visual(
    out: &mut Vec<VisualMedia>,
    path: &str,
    id: Option<String>,
    kind_hint: Option<&str>,
    source: Option<&Value>,
) {
    let path = path.trim();
    if path.is_empty() || path.contains("..") || path.starts_with('/') || path.contains('\\') {
        return;
    }
    let kind = match kind_hint {
        Some("image") | Some("img") | Some("picture") => "image",
        Some("video") | Some("movie") => "video",
        _ => match media_kind_from_path(path) {
            Some(k) => k,
            None => return,
        },
    };
    let (intro, outro, transition) = match source {
        Some(v) => (
            motion_field(v, "intro"),
            motion_field(v, "outro"),
            motion_field(v, "transition"),
        ),
        None => (None, None, None),
    };
    out.push(VisualMedia {
        id,
        path: path.to_owned(),
        kind,
        text: visual_caption(source),
        intro,
        outro,
        transition,
    });
}

fn collect_visual_media(value: &Value) -> Result<Vec<VisualMedia>, String> {
    let mut out = Vec::new();
    if let Some(images) = value.get("images").and_then(Value::as_array) {
        for (i, item) in images.iter().enumerate() {
            let path = item
                .get("path")
                .or_else(|| item.get("file"))
                .and_then(Value::as_str)
                .ok_or_else(|| format!("visuals.images[{i}] 缺 path"))?;
            push_visual(
                &mut out,
                path,
                value_id(item).or_else(|| Some(i.to_string())),
                Some("image"),
                Some(item),
            );
        }
        return Ok(out);
    }
    if let Some(files) = value.get("files").and_then(Value::as_array) {
        for file in files {
            let path = file
                .get("path")
                .or_else(|| file.get("file"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let kind = file.get("kind").and_then(Value::as_str);
            let id = value_id(file).or_else(|| {
                file.get("name")
                    .and_then(Value::as_str)
                    .map(|s| s.to_owned())
            });
            push_visual(&mut out, path, id, kind, Some(file));
        }
        if out.is_empty() {
            return Err("visuals（media_bin）里没有可用的图片/视频".into());
        }
        return Ok(out);
    }
    if let Some(path) = value
        .get("path")
        .or_else(|| value.get("file"))
        .and_then(Value::as_str)
    {
        push_visual(
            &mut out,
            path,
            value_id(value),
            value.get("kind").and_then(Value::as_str),
            Some(value),
        );
        if out.is_empty() {
            return Err("visuals 单个素材不是图片或视频".into());
        }
        return Ok(out);
    }
    if let Some(list) = value.as_array() {
        for (i, item) in list.iter().enumerate() {
            if let Some(path) = item.as_str() {
                push_visual(&mut out, path, Some(i.to_string()), None, None);
            } else if let Some(path) = item
                .get("path")
                .or_else(|| item.get("file"))
                .and_then(Value::as_str)
            {
                push_visual(
                    &mut out,
                    path,
                    value_id(item).or_else(|| Some(i.to_string())),
                    item.get("kind").and_then(Value::as_str),
                    Some(item),
                );
            }
        }
        if out.is_empty() {
            return Err("visuals 数组里没有可用的图片/视频".into());
        }
        return Ok(out);
    }
    Err("visuals 须为 images / media_bin / 单张素材".into())
}

#[derive(Clone, Copy, PartialEq)]
enum MediaFilter {
    Image,
}

fn push_media(
    out: &mut Vec<PathBuf>,
    value: &Value,
    base_dir: &Path,
    filter: MediaFilter,
) {
    let Some(list) = value.get("files").and_then(Value::as_array) else {
        return;
    };
    for item in list {
        let Some(path) = item.get("path").and_then(Value::as_str) else {
            continue;
        };
        if filter == MediaFilter::Image {
            let kind = item.get("kind").and_then(Value::as_str).unwrap_or_default();
            if kind != "image" {
                continue;
            }
        }
        out.push(base_dir.join(path));
    }
}

/// Rule-based segmentation (`length` / `paragraph` / `sentence`).
fn segment_by_rules(text: &str, mode: &str, limits: SegmentLimits) -> Value {
    // sentence: one spoken sentence per segment — never re-split by char count.
    // length / paragraph: max is the clause / segment threshold.
    let clause_over = if mode == "sentence" {
        usize::MAX
    } else {
        limits.max
    };
    let units = segment::split_units(text, clause_over);
    let starts = if mode == "sentence" {
        (0..units.len()).collect::<Vec<_>>()
    } else {
        segment::heuristic_starts(&units, limits, mode == "paragraph")
    };
    let texts = segment::segment_texts(&units, &starts);
    let segments: Vec<Value> = texts
        .iter()
        .enumerate()
        .map(|(index, body)| {
            json!({
                "id": format!("s{}", index + 1),
                "index": index,
                "text": body,
                "chars": body.chars().count(),
            })
        })
        .collect();
    // Keep the source text alongside the trimmed segment views. Downstream
    // nodes should be able to consume the original narration without losing
    // paragraph breaks or spaces between segments.
    json!({ "mode": mode, "text": text, "segments": segments })
}

/// Model cut points → segments, falling back to the rules when the reply
/// cannot be sanitized (the workflow must not die on a bad segment answer).
fn segments_from_reply(
    reply: &Value,
    text: &str,
    mode: &str,
    limits: SegmentLimits,
) -> Result<BTreeMap<String, Value>, String> {
    let units: Vec<Unit> = segment::split_units(text, limits.max);
    if units.is_empty() {
        return Err("文案为空，无法分段".into());
    }
    let raw = match reply.get("json") {
        Some(value) => value.clone(),
        None => {
            let text = reply.get("text").and_then(Value::as_str).unwrap_or_default();
            parse_json_block(text).unwrap_or(Value::Null)
        }
    };
    let value = match segment::sanitize_outline(&raw, &units, limits) {
        Ok(outline) if !outline.starts.is_empty() => {
            let texts = segment::segment_texts(&units, &outline.starts);
            let segments: Vec<Value> = texts
                .iter()
                .enumerate()
                .map(|(index, body)| {
                    json!({
                        "id": format!("s{}", index + 1),
                        "index": index,
                        "text": body,
                        "chars": body.chars().count(),
                        "title": outline.titles.get(index).cloned().unwrap_or_default(),
                    })
                })
                .collect();
            json!({ "mode": mode, "text": text, "segments": segments })
        }
        _ => segment_by_rules(text, mode, limits),
    };
    Ok(BTreeMap::from([("segments".to_owned(), value)]))
}

/// Real segmentation for the blocking API: `length`/`paragraph` are pure rules,
/// `semantic` asks the model for cut points and falls back to the heuristic.
pub fn segment_script(
    node: &GraphNode,
    text: &str,
    services: &mut dyn Services,
) -> Result<Value, String> {
    let mode = match param_str(node, "mode").as_str() {
        "length" => "length",
        "paragraph" => "paragraph",
        "sentence" => "sentence",
        _ => "semantic",
    }
    .to_owned();
    let limits = segment_limits(node);
    if mode != "semantic" {
        return Ok(segment_by_rules(text, &mode, limits));
    }
    let units = segment::split_units(text, limits.max);
    if units.is_empty() {
        return Err("文案为空，无法分段".into());
    }
    let messages = segment::build_segment_messages(&units, limits, &param_str(node, "title"));
    let reply = services.chat(ChatCall {
        messages,
        json: true,
        temperature: Some(0.2),
        model: node
            .params
            .get("model")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .map(str::to_owned),
    })?;
    let outputs = segments_from_reply(&reply, text, &mode, limits)?;
    Ok(outputs["segments"].clone())
}

/// One line per shot: the shot text trimmed to a label (no model call, so the
/// node never parks more than once).
pub fn label_shots(shots: &Value) -> Value {
    let mut list = shots
        .get("shots")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for shot in &mut list {
        let text = shot.get("text").and_then(Value::as_str).unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        let label: String = text.trim().chars().take(12).collect();
        shot["label"] = json!(label);
    }
    json!({ "shots": list })
}

fn motion_ref(id: &str) -> Result<Option<Value>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Ok(None);
    }
    if !todo_motion::valid_motion_id(id) {
        return Err(format!("动效 id 不合法：{id}"));
    }
    let duration = todo_motion::builtin_motions()
        .iter()
        .find(|preset| preset.id == id)
        .map(|preset| preset.duration)
        .unwrap_or(0.35);
    Ok(Some(json!({ "kind": id, "duration": duration })))
}

fn bind_motion(source: Option<&Value>, node: &GraphNode) -> Result<Value, String> {
    let value = source.ok_or_else(|| "动效缺 elements".to_string())?;
    let list: Vec<Value> = if let Some(images) = value.get("images").and_then(Value::as_array) {
        images.clone()
    } else if value.get("path").and_then(Value::as_str).is_some() {
        vec![value.clone()]
    } else {
        return Err("动效需要 image_asset 或 image_assets".into());
    };
    if list.is_empty() {
        return Err("动效：elements 为空".into());
    }
    let intro = motion_ref(&param_str(node, "intro"))?;
    let outro = motion_ref(&param_str(node, "outro"))?;
    let transition = motion_ref(&param_str(node, "transition"))?;
    let mut images = Vec::with_capacity(list.len());
    for mut asset in list {
        nodes::validate_payload(DataKind::ImageAsset, &asset)
            .map_err(|e| format!("动效元素不合格：{e}"))?;
        match &intro {
            Some(m) => asset["intro"] = m.clone(),
            None => {
                if let Some(o) = asset.as_object_mut() {
                    o.remove("intro");
                }
            }
        }
        match &outro {
            Some(m) => asset["outro"] = m.clone(),
            None => {
                if let Some(o) = asset.as_object_mut() {
                    o.remove("outro");
                }
            }
        }
        match &transition {
            Some(m) => asset["transition"] = m.clone(),
            None => {
                if let Some(o) = asset.as_object_mut() {
                    o.remove("transition");
                }
            }
        }
        images.push(asset);
    }
    let payload = json!({ "images": images });
    nodes::validate_payload(DataKind::ImageAssets, &payload)
        .map_err(|e| format!("动效输出不合格：{e}"))?;
    Ok(payload)
}

fn attach_item_motions(item: &mut Value, source: &Value, item_duration: f64) {
    let nested = source.get("motion");
    for key in ["intro", "outro", "transition"] {
        let raw = source
            .get(key)
            .or_else(|| nested.and_then(|m| m.get(key)));
        let Some(raw) = raw else { continue };
        if raw.is_null() {
            continue;
        }
        let kind = raw.get("kind").and_then(Value::as_str).unwrap_or("");
        if kind.is_empty() || !todo_motion::valid_motion_id(kind) {
            continue;
        }
        let duration = raw
            .get("duration")
            .and_then(Value::as_f64)
            .filter(|d| d.is_finite() && *d > 0.0)
            .unwrap_or(0.35);
        let limit = if key == "transition" {
            10.0
        } else {
            item_duration
        };
        item[key] = json!({
            "kind": kind,
            "duration": duration.min(limit).max(0.05),
        });
    }
}

pub fn choose_motions(plan: &Value, node: &GraphNode) -> Value {
    const INTROS: [&str; 3] = ["pop", "rise", "stamp"];
    const OUTROS: [&str; 3] = ["fadeOut", "shrinkOut", "sinkOut"];
    let transition = param_str(node, "transition");
    let transition = if transition.is_empty() { "dissolve".to_owned() } else { transition };
    let mut value = plan.clone();
    if let Some(list) = value.get_mut("shots").and_then(Value::as_array_mut) {
        for (index, shot) in list.iter_mut().enumerate() {
            shot["motion"] = json!({
                "intro": { "kind": INTROS[index % INTROS.len()], "duration": 0.35 },
                "outro": { "kind": OUTROS[index % OUTROS.len()], "duration": 0.3 },
                "transition": if index == 0 {
                    Value::Null
                } else {
                    json!({ "kind": transition, "duration": 0.15 })
                },
            });
        }
    }
    value
}

/// Bottom layer: one plate item plus one subtitle item per shot.
pub fn ground_items(plan: &Value) -> Vec<Value> {
    let shots = plan.get("shots").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut items = vec![json!({
        "key": "ground/plate",
        "type": "plate",
        "layer": 0,
    })];
    for shot in &shots {
        items.push(json!({
            "key": format!("ground/sub/{}", shot.get("index").and_then(Value::as_u64).unwrap_or(0)),
            "type": "text",
            "role": "subtitle",
            "text": shot.get("text").cloned().unwrap_or(Value::Null),
            "start": shot.get("start").cloned().unwrap_or(json!(0.0)),
            "end": shot.get("end").cloned().unwrap_or(json!(0.0)),
        }));
    }
    items
}

/// One picture item per shot, carrying the label/prompt the host will render.
pub fn picture_items(plan: &Value) -> Vec<Value> {
    let shots = plan.get("shots").and_then(Value::as_array).cloned().unwrap_or_default();
    shots
        .iter()
        .map(|shot| {
            json!({
                "key": format!("shot/{}", shot.get("index").and_then(Value::as_u64).unwrap_or(0)),
                "type": "image",
                "role": "picture",
                "prompt": shot.get("label").cloned().unwrap_or_else(|| shot.get("text").cloned().unwrap_or(Value::Null)),
                "start": shot.get("start").cloned().unwrap_or(json!(0.0)),
                "end": shot.get("end").cloned().unwrap_or(json!(0.0)),
                "motion": shot.get("motion").cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
}

/// One shot per subtitle cue, merging cues shorter than 1.2 s into a neighbour.
pub fn split_shots(speech: &Value, cues: &Value) -> Value {
    split_shots_with_segments(speech, cues, None)
}

fn normalized_match_text(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .filter(|c| !matches!(c, '。' | '！' | '？' | '!' | '?' | '，' | '、' | ',' | '；' | ';'))
        .collect()
}

/// Build shots from stable narration segments first, then use subtitle cues as
/// the timing source. This keeps one visual beat aligned to one spoken idea
/// instead of creating a shot for every short subtitle line.
fn split_shots_with_segments(
    speech: &Value,
    cues: &Value,
    segments: Option<&Value>,
) -> Value {
    let duration = speech.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
    let list = cues
        .get("cues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(segment_list) = segments
        .and_then(|value| value.get("segments"))
        .and_then(Value::as_array)
        .filter(|list| !list.is_empty())
    {
        let mut shots = Vec::new();
        let mut cue_index = 0usize;
        for (segment_index, segment) in segment_list.iter().enumerate() {
            let text = segment
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let target = normalized_match_text(text);
            if target.is_empty() {
                continue;
            }
            let start_index = cue_index;
            let mut matched = String::new();
            while cue_index < list.len() {
                let cue_text = list[cue_index]
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                matched.push_str(&normalized_match_text(cue_text));
                cue_index += 1;
                if matched.len() >= target.len() {
                    break;
                }
            }
            let end_index = cue_index.saturating_sub(1);
            if start_index >= list.len() {
                break;
            }
            let start = list[start_index]
                .get("start")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let end = list
                .get(end_index)
                .and_then(|cue| cue.get("end"))
                .and_then(Value::as_f64)
                .unwrap_or(start + 1.0);
            shots.push(json!({
                "index": shots.len(),
                "segmentIndex": segment_index,
                "start": start.max(0.0),
                "end": end.max(start),
                "text": text,
                "title": segment.get("title").cloned().unwrap_or(Value::Null),
            }));
        }
        // Keep any trailing cue text visible instead of silently dropping it.
        if cue_index < list.len() {
            let start = list[cue_index]
                .get("start")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let end = list
                .last()
                .and_then(|cue| cue.get("end"))
                .and_then(Value::as_f64)
                .unwrap_or(start + 1.0);
            let text = list[cue_index..]
                .iter()
                .filter_map(|cue| cue.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("");
            shots.push(json!({
                "index": shots.len(),
                "segmentIndex": Value::Null,
                "start": start.max(0.0),
                "end": end.max(start),
                "text": text,
            }));
        }
        if !shots.is_empty() {
            return json!({ "shots": shots, "duration": duration, "source": "segments" });
        }
    }
    let mut shots: Vec<Value> = Vec::new();
    for cue in list {
        let start = cue.get("start").and_then(Value::as_f64).unwrap_or(0.0);
        let end = cue.get("end").and_then(Value::as_f64).unwrap_or(start + 1.0);
        let text = cue.get("text").and_then(Value::as_str).unwrap_or_default();
        match shots.last_mut() {
            Some(last) if end - start < 1.2 => {
                last["end"] = json!(end);
                let merged = format!(
                    "{} {}",
                    last["text"].as_str().unwrap_or_default(),
                    text
                );
                last["text"] = json!(merged.trim().to_owned());
            }
            _ => shots.push(json!({
                "index": shots.len(),
                "start": start,
                "end": end,
                "text": text,
            })),
        }
    }
    if shots.is_empty() && duration > 0.0 {
        shots.push(json!({ "index": 0, "start": 0.0, "end": duration, "text": "" }));
    }
    json!({ "shots": shots, "duration": duration, "source": "cues" })
}

/// The first complete JSON object in a model answer.
pub fn parse_json_block(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&text[start..=end]).ok()
}

/// Ports a node may read, for the UI/host to explain a run.
pub fn input_ports(node_type: &str) -> BTreeSet<&'static str> {
    nodes::lookup(node_type)
        .map(|spec| spec.inputs.iter().map(|port| port.name).collect())
        .unwrap_or_default()
}

/// Kinds a node emits, by port.
pub fn output_kinds(node_type: &str) -> Vec<(&'static str, DataKind)> {
    nodes::lookup(node_type)
        .map(|spec| spec.outputs.iter().map(|port| (port.name, port.kind)).collect())
        .unwrap_or_default()
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use crate::workflow::graph::{GraphEdge, GraphNode, GRAPH_SPEC};

    #[derive(Default)]
    struct Mock {
        chat_calls: RefCell<Vec<ChatCall>>,
        reply: Value,
        speech_calls: RefCell<usize>,
        image_calls: RefCell<Vec<ImageCall>>,
    }

    impl Services for Mock {
        fn chat(&mut self, call: ChatCall) -> Result<Value, String> {
            self.chat_calls.borrow_mut().push(call.clone());
            if !self.reply.is_null() {
                return Ok(self.reply.clone());
            }
            let user = call
                .messages
                .iter()
                .rev()
                .find(|m| m.role == todo_production::tasks::ChatRole::User)
                .map(|m| m.content.as_str())
                .unwrap_or("");
            let body = user.split("\n\n").last().unwrap_or(user).trim();
            Ok(json!({ "text": format!("改{body}") }))
        }
        fn image(&mut self, call: ImageCall) -> Result<Value, String> {
            self.image_calls.borrow_mut().push(call.clone());
            let n = self.image_calls.borrow().len();
            Ok(json!({ "path": format!("out/a{n}.png"), "mime": "image/png" }))
        }
        fn vision(&mut self, _prompt: String, images: Vec<PathBuf>) -> Result<Value, String> {
            Ok(json!({ "text": format!("{} 张图", images.len()) }))
        }
        fn speech(&mut self, call: SpeechCall) -> Result<Value, String> {
            *self.speech_calls.borrow_mut() += 1;
            let n = *self.speech_calls.borrow();
            let chars: Vec<Value> = call
                .text
                .chars()
                .enumerate()
                .map(|(index, ch)| {
                    json!({ "text": ch.to_string(), "start": index as f64 * 0.1,
                            "end": (index + 1) as f64 * 0.1 })
                })
                .collect();
            let duration = chars.len() as f64 * 0.1;
            Ok(json!({ "file": format!("out/a{n}.wav"), "duration": duration,
                       "timeline": { "text": call.text, "duration": duration, "chars": chars } }))
        }
        fn subtitles(&mut self, _call: SubtitlesCall) -> Result<Value, String> {
            Ok(json!({ "cues": [{ "text": "你好", "start": 0.0, "end": 1.0 }] }))
        }
    }

    #[test]
    fn large_relay_chain_yields_between_ui_frames_without_losing_outputs() {
        let nodes = (0..200).map(|i| node(&format!("hub{i}"), "relay", &[])).collect();
        let edges = (1..200).map(|i| edge(&format!("hub{}", i - 1), "files", &format!("hub{i}"), "files")).collect();
        let graph = graph(nodes, edges);
        let mut job = WorkflowJob::new(&graph, Path::new("."), RunInput::default());
        let mut frames = 0;
        loop {
            let before = job.progress().0;
            let step = job.advance_budgeted(8);
            assert!(job.progress().0 - before <= 8, "one UI frame ran the entire relay chain");
            frames += 1;
            if matches!(step, Step::Finished) { break; }
            assert!(matches!(step, Step::Waiting));
            assert!(frames <= 26);
        }
        assert_eq!(job.report().nodes.len(), 200);
        assert!(job.report().error.is_none());
        assert!(frames >= 25);
    }

    fn node(id: &str, node_type: &str, params: &[(&str, Value)]) -> GraphNode {
        GraphNode {
            id: id.into(),
            node_type: node_type.into(),
            label: None,
            params: params
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect(),
            x: None,
            y: None,
        }
    }

    fn edge(from: &str, out: &str, to: &str, input: &str) -> GraphEdge {
        GraphEdge {
            from: from.into(),
            to: to.into(),
            out: Some(out.into()),
            input: Some(input.into()),
        }
    }

    fn graph(nodes: Vec<GraphNode>, edges: Vec<GraphEdge>) -> Graph {
        Graph { spec: GRAPH_SPEC.into(), nodes, edges }
    }

    fn script_line() -> Vec<GraphNode> {
        vec![
            node("go", "trigger", &[]),
            node("txt", "script", &[("text", json!("第一句话。第二句话。第三句话。"))]),
            node("seg", "segment_script", &[("mode", json!("length")),
                                            ("target", json!(5.0)), ("max", json!(8.0))]),
            node("voice", "tts", &[("voice", json!("S_x"))]),
            node("subs", "subtitles", &[]),
            node("push", "emit", &[]),
            node("end", "done", &[]),
        ]
    }

    #[test]
    fn a_full_chain_runs_and_reconstructs_the_script() {
        let edges = vec![
            edge("txt", "script", "seg", "text"),
            edge("seg", "segments", "voice", "text"),
            edge("txt", "script", "subs", "script"),
            edge("voice", "clips", "subs", "speech"),
            edge("voice", "media", "subs", "media"),
            edge("subs", "cues", "push", "payload"),
            edge("push", "items", "end", "result"),
        ];
        let g = graph(script_line(), edges);
        assert!(g.validate(&BTreeSet::new()).is_empty(), "{:?}", g.validate(&BTreeSet::new()));
        let mut services = Mock::default();
        let mut emitted = Vec::new();
        let report = run(&g, Path::new("."), &mut services, &RunInput::default(),
                         &mut |value| emitted.push(value));
        assert!(report.ok(), "{report:?}");
        assert!(!emitted.is_empty(), "应至少入轨一次：{emitted:?}");
        let voice = report.nodes.iter().find(|n| n.id == "voice").unwrap();
        let clips = voice
            .preview
            .as_ref()
            .and_then(|p| serde_json::from_str::<Value>(p).ok())
            .and_then(|v| {
                v.get("clips")
                    .cloned()
                    .or_else(|| Some(v))
            });
        // Prefer counting from produced clips via speech_calls: one call per segment.
        let speech_n = *services.speech_calls.borrow();
        assert!(speech_n >= 2, "分段后应逐段配音，实际 {speech_n} 次；clips={clips:?}");
        let _ = clips;
    }

    #[test]
    fn length_segmentation_keeps_every_character() {
        let text = "第一句话。第二句话。第三句话。第四句话。";
        let mut services = Mock::default();
        let value = segment_script(
            &node("seg", "segment_script", &[("mode", json!("length")),
                                             ("target", json!(6.0)), ("max", json!(9.0))]),
            text,
            &mut services,
        )
        .unwrap();
        let segments = value["segments"].as_array().unwrap();
        assert!(segments.len() >= 2, "{value}");
        let rebuilt: String = segments
            .iter()
            .map(|s| s["text"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(rebuilt.trim(), text.trim());
        assert_eq!(value["mode"], "length");
    }

    #[test]
    fn sentence_mode_keeps_long_sentence_without_char_cap() {
        // 远超默认 max=200：按语句时仍应整句一段，不被逗号再拆。
        let long = format!(
            "{}，{}，{}。下一句。",
            "甲".repeat(80),
            "乙".repeat(80),
            "丙".repeat(80)
        );
        let mut services = Mock::default();
        let value = segment_script(
            &node(
                "seg",
                "segment_script",
                &[
                    ("mode", json!("sentence")),
                    ("target", json!(40.0)),
                    ("max", json!(50.0)),
                ],
            ),
            &long,
            &mut services,
        )
        .unwrap();
        let segments = value["segments"].as_array().unwrap();
        assert_eq!(segments.len(), 2, "{value}");
        assert!(
            segments[0]["text"].as_str().unwrap().chars().count() > 200,
            "长句不应被字数上限拆开：{}",
            segments[0]["text"]
        );
        assert!(segments[1]["text"].as_str().unwrap().contains("下一句"));
    }

    #[test]
    fn semantic_segmentation_uses_the_model_and_falls_back_on_junk() {
        let seg = node("seg", "segment_script", &[("mode", json!("semantic"))]);
        let text = "第一句话。第二句话。第三句话。";

        let mut good = Mock {
            reply: json!({ "text": r#"{"segments":[{"start":0},{"start":1}]}"# }),
            ..Mock::default()
        };
        let value = segment_script(&seg, text, &mut good).unwrap();
        assert_eq!(good.chat_calls.borrow().len(), 1);
        assert_eq!(value["mode"], "semantic");
        assert!(value["segments"].as_array().unwrap().len() >= 1);

        let mut junk = Mock { reply: json!({ "text": "抱歉我不会" }), ..Mock::default() };
        let value = segment_script(&seg, text, &mut junk).unwrap();
        assert!(value["segments"].as_array().unwrap().len() >= 1);
    }

    #[test]
    fn media_bin_feeds_text_and_files_into_the_chain() {
        // 沙箱里 %TEMP% 不可写，改用仓库 target/ 下的临时目录
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/wf-tests")
            .join(format!("case-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("script.txt"), "从文件来的文案。第二句。").unwrap();

        let nodes = vec![
            node("bin", "media_bin", &[("files", json!([{ "path": "script.txt", "kind": "text" }]))]),
            node("txt", "script", &[]),
            node("seg", "segment_script", &[("mode", json!("paragraph"))]),
        ];
        let edges = vec![
            edge("bin", "text", "txt", "text"),
            edge("txt", "script", "seg", "text"),
        ];
        let g = graph(nodes, edges);
        let mut services = Mock::default();
        let report = run(&g, &dir, &mut services, &RunInput::default(), &mut |_| {});
        assert!(report.ok(), "{report:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn media_bin_exposes_image_for_ref_port() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/wf-tests")
            .join(format!("media-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Minimal valid-enough PNG header bytes; runner only checks file exists.
        std::fs::write(dir.join("ref.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        std::fs::write(dir.join("note.txt"), "旁白").unwrap();

        let nodes = vec![
            node(
                "bin",
                "media_bin",
                &[(
                    "files",
                    json!([
                        { "path": "ref.png", "kind": "image", "name": "ref.png" },
                        { "path": "note.txt", "kind": "text", "name": "note.txt" }
                    ]),
                )],
            ),
            node("draw", "image_gen", &[("size", json!("1024x1024"))]),
        ];
        let edges = vec![
            edge("bin", "text", "draw", "prompt"),
            edge("bin", "image", "draw", "ref"),
        ];
        let g = graph(nodes, edges);
        let problems = g.validate_draft(&Default::default());
        assert!(
            problems.is_empty(),
            "media_bin.image → image_gen.ref should type-check: {problems:?}"
        );
        let mut services = Mock::default();
        let report = run(&g, &dir, &mut services, &RunInput::default(), &mut |_| {});
        assert!(report.ok(), "{report:?}");
        let calls = services.image_calls.borrow();
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(calls[0].references, vec!["ref.png".to_owned()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_studio_graph_is_runnable() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../extensions/user/studio.workflow-1792467074");
        let text = std::fs::read_to_string(root.join("graph.json")).expect("graph.json");
        let g = Graph::parse(&text, "graph").expect("parse");
        let missing = missing_inputs(&g);
        assert!(
            missing.is_empty(),
            "unexpected missing inputs: {missing:?}"
        );
        let mut job = WorkflowJob::new(&g, &root, RunInput::default());
        assert!(
            !job.finished(),
            "job should not finish before advance: {:?}",
            job.report().error
        );
        match job.advance() {
            Step::Finished => panic!(
                "advance finished immediately: err={:?} nodes={:?}",
                job.report().error,
                job.report()
                    .nodes
                    .iter()
                    .map(|n| (&n.id, &n.message, &n.logs))
                    .collect::<Vec<_>>()
            ),
            Step::Calls(batch) => assert!(!batch.is_empty()),
            Step::Waiting => panic!("waiting with no calls"),
        }
    }

    #[test]
    fn image_gen_reuses_package_cache_on_second_run() {
        let pkg = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/wf-tests")
            .join(format!("img-cache-{}", std::process::id()));
        let work = pkg.join("work");
        std::fs::create_dir_all(work.join("services")).unwrap();
        std::fs::write(work.join("services/a.png"), b"\x89PNG\r\n\x1a\n").unwrap();

        let blocks = vec![("0".into(), "一只螃蟹".into())];
        let fp = crate::nodes::image_cache::image_gen_fingerprint(
            &blocks,
            "",
            "1024x1024",
            None,
            false,
            0,
            None,
            &[],
            0,
        );
        let images = vec![json!({ "path": "services/a.png", "id": "0", "text": "一只螃蟹" })];
        crate::nodes::image_cache::save(&pkg, Some(&work), "draw", &fp, &images).unwrap();
        assert!(crate::nodes::image_cache::has_cache(&pkg, "draw"));

        let work2 = pkg.join("work2");
        std::fs::create_dir_all(&work2).unwrap();
        let hit = crate::nodes::image_cache::try_load(&pkg, Some(&work2), "draw", &fp).unwrap();
        assert_eq!(hit.len(), 1);
        let path = hit[0]["path"].as_str().unwrap();
        assert!(work2.join(path).is_file(), "{path}");

        crate::nodes::image_cache::clear(&pkg, "draw").unwrap();
        assert!(!crate::nodes::image_cache::has_cache(&pkg, "draw"));
        assert_eq!(crate::nodes::image_cache::epoch(&pkg, "draw"), 1);
        // Old fingerprint must miss; new epoch changes both package + AI keys.
        assert!(crate::nodes::image_cache::try_load(&pkg, Some(&work2), "draw", &fp).is_none());
        let fp_after = crate::nodes::image_cache::image_gen_fingerprint(
            &blocks,
            "",
            "1024x1024",
            None,
            false,
            0,
            None,
            &[],
            1,
        );
        assert_ne!(fp, fp_after);
        let _ = std::fs::remove_dir_all(&pkg);
    }

    #[test]
    fn orphan_required_node_does_not_block_run() {
        let nodes = vec![
            node("bin", "media_bin", &[(
                "files",
                json!([{ "path": "a.txt", "kind": "text" }]),
            )]),
            node("orphan", "emit_sequence", &[]),
            node("txt", "script", &[]),
        ];
        let edges = vec![edge("bin", "text", "txt", "text")];
        let g = graph(nodes, edges);
        assert!(missing_inputs(&g).is_empty(), "{:?}", missing_inputs(&g));
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/wf-tests")
            .join(format!("orphan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "你好。").unwrap();
        let job = WorkflowJob::new(&g, &dir, RunInput::default());
        assert!(!job.finished(), "{:?}", job.report().error);
        assert!(
            !job.order_for_test().iter().any(|id| id == "orphan"),
            "orphan should be skipped"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relay_merges_local_json_and_exposes_side_ports() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/wf-tests")
            .join(format!("relay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("meta.json"), r#"{"title":"ok"}"#).unwrap();
        std::fs::write(dir.join("note.txt"), "旁白一句").unwrap();
        let nodes = vec![node(
            "hub",
            "relay",
            &[(
                "files",
                json!([
                    { "path": "meta.json", "kind": "json" },
                    { "path": "note.txt", "kind": "text" }
                ]),
            )],
        )];
        let g = graph(nodes, vec![]);
        let report = run(
            &g,
            &dir,
            &mut Mock::default(),
            &RunInput::default(),
            &mut |_| {},
        );
        assert!(report.ok(), "{report:?}");
        let hub = report.nodes.iter().find(|n| n.id == "hub").unwrap();
        assert!(hub.ok);
        assert!(hub.outputs.iter().any(|p| p == "files"));
        assert!(hub.outputs.iter().any(|p| p == "text"));
        assert!(hub.outputs.iter().any(|p| p == "json"));
        let preview = hub.preview.clone().unwrap_or_default();
        assert!(preview.contains("title") || preview.contains("旁白"), "{preview}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relay_collects_images_and_videos_for_preview() {
        let nodes = vec![
            node("txt", "script", &[("text", json!("一只螃蟹"))]),
            node("draw", "image_gen", &[("size", json!("1024x1024"))]),
            node("hub", "relay", &[]),
        ];
        let edges = vec![
            edge("txt", "script", "draw", "prompt"),
            edge("draw", "images", "hub", "in"),
        ];
        let g = graph(nodes, edges);
        let mut services = Mock::default();
        let report = run(
            &g,
            Path::new("."),
            &mut services,
            &RunInput::default(),
            &mut |_| {},
        );
        assert!(report.ok(), "{report:?}");
        let hub = report.nodes.iter().find(|n| n.id == "hub").unwrap();
        assert!(hub.image.is_some(), "{hub:?}");
        let files = hub.files.as_ref().expect("files snapshot");
        let list = files["files"].as_array().expect("files.files");
        assert!(!list.is_empty());
        assert_eq!(list[0]["kind"], "image");
    }

    #[test]
    fn relay_previews_segments_wired_into_in() {
        let nodes = vec![
            node("txt", "script", &[("text", json!("第一段。第二段。"))]),
            node("seg", "segment_script", &[("mode", json!("paragraph"))]),
            node("hub", "relay", &[]),
        ];
        let edges = vec![
            edge("txt", "script", "seg", "text"),
            edge("seg", "segments", "hub", "in"),
        ];
        let g = graph(nodes, edges);
        let report = run(
            &g,
            Path::new("."),
            &mut Mock::default(),
            &RunInput::default(),
            &mut |_| {},
        );
        assert!(report.ok(), "{report:?}");
        let hub = report.nodes.iter().find(|n| n.id == "hub").unwrap();
        assert!(hub.outputs.iter().any(|p| p == "segments"));
        assert!(hub.outputs.iter().any(|p| p == "text"));
        assert!(hub.segments.is_some());
        let preview = hub.preview.clone().unwrap_or_default();
        assert!(
            preview.contains("第一段") || preview.contains("segments"),
            "expected text-block preview, got: {preview}"
        );
    }

    #[test]
    fn edit_text_rewrites_each_block_and_keeps_count() {
        let nodes = vec![
            node(
                "hub",
                "relay",
                &[(
                    "blocks",
                    json!([
                        {"id": "a", "text": "第一段原文"},
                        {"id": "b", "text": "第二段原文"},
                        {"id": "c", "text": "第三段原文"}
                    ]),
                )],
            ),
            node(
                "edit",
                "edit_text",
                &[
                    ("system", json!("只改写口语")),
                    ("model", json!("deepseek-flash")),
                ],
            ),
        ];
        let edges = vec![edge("hub", "text", "edit", "text")];
        let g = graph(nodes, edges);
        assert!(
            g.validate(&BTreeSet::new()).is_empty(),
            "{:?}",
            g.validate(&BTreeSet::new())
        );
        let mut services = Mock::default();
        let report = run(
            &g,
            Path::new("."),
            &mut services,
            &RunInput::default(),
            &mut |_| {},
        );
        assert!(report.ok(), "{report:?}");
        assert_eq!(services.chat_calls.borrow().len(), 3);
        let first = &services.chat_calls.borrow()[0];
        assert_eq!(first.model.as_deref(), Some("deepseek-flash"));
        assert!(
            first.messages.iter().any(|m| {
                m.role == todo_production::tasks::ChatRole::System && m.content.contains("只改写口语")
            }),
            "{first:?}"
        );
        let edit = report.nodes.iter().find(|n| n.id == "edit").unwrap();
        let preview = edit.preview.clone().unwrap_or_default();
        let value: Value = serde_json::from_str(&preview).unwrap_or(json!({}));
        let segs = value["segments"].as_array().unwrap();
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0]["id"], "a");
        assert_eq!(segs[0]["text"], "改第一段原文");
        assert_eq!(segs[2]["text"], "改第三段原文");
    }

    #[test]
    fn image_prompt_one_shot_keeps_count_and_resolves_ids() {
        let nodes = vec![
            node(
                "hub",
                "relay",
                &[(
                    "blocks",
                    json!([
                        {"id": "a", "text": "小明在追球"},
                        {"id": "b", "text": "他摔倒了，很沮丧"}
                    ]),
                )],
            ),
            node(
                "fx",
                "image_prompt",
                &[("model", json!("deepseek-flash"))],
            ),
        ];
        let edges = vec![edge("hub", "text", "fx", "text")];
        let g = graph(nodes, edges);
        assert!(
            g.validate(&BTreeSet::new()).is_empty(),
            "{:?}",
            g.validate(&BTreeSet::new())
        );
        let reply_body = json!({
            "prompts": [
                {"id": "a", "prompt": "小明在草地上追球，身体前倾"},
                {"id": "b", "prompt": "小明摔倒在地，表情沮丧"}
            ]
        });
        let mut services = Mock {
            reply: json!({ "text": reply_body.to_string() }),
            ..Mock::default()
        };
        let report = run(
            &g,
            Path::new("."),
            &mut services,
            &RunInput::default(),
            &mut |_| {},
        );
        assert!(report.ok(), "{report:?}");
        assert_eq!(services.chat_calls.borrow().len(), 1);
        let call = &services.chat_calls.borrow()[0];
        assert!(call.json);
        assert_eq!(call.model.as_deref(), Some("deepseek-flash"));
        assert!(
            call.messages.iter().any(|m| {
                m.role == todo_production::tasks::ChatRole::System
                    && m.content.contains("主语不明")
                    && !m.content.contains("海绵宝宝")
            }),
            "应使用通用默认系统提示：{call:?}"
        );
        let user = call
            .messages
            .iter()
            .find(|m| m.role == todo_production::tasks::ChatRole::User)
            .map(|m| m.content.as_str())
            .unwrap_or("");
        assert!(user.contains("小明在追球"), "{user}");
        assert!(
            !user.contains("补充设定") && !user.contains("画风："),
            "未填补充/画风时不应塞进用户消息：{user}"
        );
        let fx = report.nodes.iter().find(|n| n.id == "fx").unwrap();
        let preview = fx.preview.clone().unwrap_or_default();
        let value: Value = serde_json::from_str(&preview).unwrap_or(json!({}));
        let segs = value["segments"].as_array().unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0]["id"], "a");
        assert!(segs[0]["text"].as_str().unwrap().contains("小明"));
        assert_eq!(segs[0]["script"], "小明在追球");
        assert_eq!(segs[1]["id"], "b");
        assert!(
            segs[1]["text"].as_str().unwrap().contains("小明"),
            "第二句应补清主语：{}",
            segs[1]["text"]
        );
        assert!(segs[1]["text"].as_str().unwrap().contains("沮丧"));
        assert_eq!(segs[1]["script"], "他摔倒了，很沮丧");
    }

    #[test]
    fn image_prompt_rejects_mismatched_count() {
        let nodes = vec![
            node(
                "hub",
                "relay",
                &[(
                    "blocks",
                    json!([
                        {"id": "a", "text": "第一句"},
                        {"id": "b", "text": "第二句"}
                    ]),
                )],
            ),
            node("fx", "image_prompt", &[]),
        ];
        let edges = vec![edge("hub", "text", "fx", "text")];
        let g = graph(nodes, edges);
        let reply_body = json!({
            "prompts": [
                {"id": "a", "prompt": "只有一条"}
            ]
        });
        let mut services = Mock {
            reply: json!({ "text": reply_body.to_string() }),
            ..Mock::default()
        };
        let report = run(
            &g,
            Path::new("."),
            &mut services,
            &RunInput::default(),
            &mut |_| {},
        );
        assert!(!report.ok(), "{report:?}");
        let err = report.error.clone().unwrap_or_default();
        assert!(
            err.contains("期望 2")
                || report
                    .nodes
                    .iter()
                    .any(|n| n.id == "fx" && n.message.contains("期望 2")),
            "{report:?}"
        );
    }

    #[test]
    fn llm_chat_forwards_system_prompt_and_reply() {
        let nodes = vec![
            node("txt", "script", &[("text", json!("写一句开场白"))]),
            node("ask", "llm_chat", &[("system", json!("你是导演")), ("json", json!(true))]),
            node("out", "emit", &[]),
        ];
        let edges = vec![
            edge("txt", "script", "ask", "prompt"),
            edge("ask", "reply", "out", "payload"),
        ];
        let g = graph(nodes, edges);
        let mut services = Mock { reply: json!({ "text": "{}" }), ..Mock::default() };
        let mut emitted = Vec::new();
        let report = run(&g, Path::new("."), &mut services, &RunInput::default(),
                         &mut |v| emitted.push(v));
        assert!(report.ok(), "{report:?}");
        let chats = services.chat_calls.borrow();
        let calls: Vec<_> = chats.iter().filter(|c| c.json).collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].messages.len(), 2);
        assert!(calls[0].json);
        assert_eq!(emitted.len(), 1);
    }

    #[test]
    fn python_nodes_say_they_need_the_worker_pool() {
        let g = graph(vec![node("py", "python:bake", &[])], vec![]);
        let report = run(&g, Path::new("."), &mut Mock::default(), &RunInput::default(),
                         &mut |_| {});
        assert!(!report.ok());
        let error = report.error.clone().unwrap_or_default();
        assert!(error.contains("python"), "{report:?}");
    }

    #[test]
    fn unwired_required_inputs_are_reported_before_running() {
        // Isolated nodes are skipped; wire an optional edge so the node participates.
        let g = graph(
            vec![
                node("go", "trigger", &[]),
                node("ask", "llm_chat", &[]),
            ],
            vec![edge("go", "start", "ask", "start")],
        );
        assert_eq!(missing_inputs(&g), vec!["ask.prompt".to_owned()]);
        let report = run(&g, Path::new("."), &mut Mock::default(), &RunInput::default(),
                         &mut |_| {});
        assert!(report.error.unwrap().contains("没接线"));
    }

    #[test]
    fn missing_script_is_a_clear_error() {
        let g = graph(vec![node("txt", "script", &[])], vec![]);
        let report = run(&g, Path::new("."), &mut Mock::default(), &RunInput::default(),
                         &mut |_| {});
        assert!(report.error.unwrap().contains("没有文案"));
    }

    #[test]
    fn split_shots_merges_short_cues() {
        let speech = json!({ "duration": 6.0 });
        let cues = json!({ "cues": [
            { "text": "第一句", "start": 0.0, "end": 2.0 },
            { "text": "短", "start": 2.1, "end": 2.4 },
            { "text": "第三句", "start": 2.6, "end": 5.0 }
        ]});
        let shots = split_shots(&speech, &cues);
        let list = shots["shots"].as_array().unwrap();
        assert_eq!(list.len(), 2, "{shots}");
        assert_eq!(list[0]["end"], json!(2.4));
        assert!(list[0]["text"].as_str().unwrap().contains("第一句"));
    }

    #[test]
    fn split_shots_prefers_narration_segments_over_each_subtitle_line() {
        let speech = json!({ "duration": 8.0 });
        let cues = json!({ "cues": [
            { "text": "先说背景", "start": 0.0, "end": 1.2 },
            { "text": "再说原因", "start": 1.3, "end": 2.4 },
            { "text": "最后给方法", "start": 2.6, "end": 4.0 }
        ]});
        let segments = json!({ "mode": "length", "text": "先说背景再说原因最后给方法",
            "segments": [
                { "id": "s1", "text": "先说背景再说原因", "chars": 8 },
                { "id": "s2", "text": "最后给方法", "chars": 5 }
            ]
        });
        let shots = split_shots_with_segments(&speech, &cues, Some(&segments));
        let list = shots["shots"].as_array().unwrap();
        assert_eq!(list.len(), 2, "{shots}");
        assert_eq!(list[0]["segmentIndex"], 0);
        assert_eq!(list[0]["end"], json!(2.4));
        assert_eq!(list[1]["start"], json!(2.6));
    }

    #[test]
    fn subtitle_reply_is_split_and_sorted() {
        let reply = json!({ "cues": [
            { "text": "第一句第二句", "start": 1.0, "end": 3.0 },
            { "text": "开头", "start": 0.0, "end": 0.8 }
        ]});
        let normalized = normalize_subtitle_reply(&reply, 4);
        let cues = normalized["cues"].as_array().unwrap();
        assert_eq!(cues.len(), 3);
        assert_eq!(cues[0]["start"], json!(0.0));
        assert!(cues.iter().all(|cue| cue["text"].as_str().unwrap().chars().count() <= 4));
    }

    #[test]
    fn emit_adapts_service_outputs_to_timeline_items() {
        let payload = json!({
            "file": "out/voice.wav",
            "duration": 2.5,
            "timeline": { "chars": [{ "text": "你", "start": 0.0, "end": 0.2 }] }
        });
        let items = normalize_emit_payload(&payload).unwrap();
        assert_eq!(items["items"][0]["role"], "voice");
        assert_eq!(items["items"][0]["source"]["file"], "out/voice.wav");
        assert_eq!(items["items"][0]["duration"], json!(2.5));
    }

    #[test]
    fn image_node_returns_a_validated_asset() {
        let nodes = vec![
            node("txt", "script", &[("text", json!("一只红色螃蟹老板"))]),
            node("draw", "image_gen", &[("size", json!("1024x1024")), ("transparent", json!(true))]),
            node("out", "emit", &[]),
        ];
        let edges = vec![
            edge("txt", "script", "draw", "prompt"),
            edge("draw", "images", "out", "payload"),
        ];
        let g = graph(nodes, edges);
        let mut services = Mock::default();
        let mut emitted = Vec::new();
        let report = run(&g, Path::new("."), &mut services, &RunInput::default(),
                         &mut |v| emitted.push(v));
        assert!(report.ok(), "{report:?}");
        assert_eq!(emitted.len(), 1);
        assert_eq!(services.image_calls.borrow().len(), 1);
    }

    #[test]
    fn picture_batch_flattens_nested_shot_motion_onto_emit_items() {
        let plan = json!({
            "shots": [{
                "index": 0,
                "text": "开场",
                "label": "螃蟹上桌",
                "start": 0.0,
                "end": 2.0,
                "motion": {
                    "intro": { "kind": "rise", "duration": 0.4 },
                    "outro": { "kind": "fadeOut", "duration": 0.3 },
                    "transition": null
                }
            }]
        });
        let raw = picture_items(&plan);
        let items = normalize_emit_payload(&json!({ "items": raw })).unwrap();
        let item = &items["items"][0];
        assert_eq!(item["intro"]["kind"], "rise");
        assert_eq!(item["outro"]["kind"], "fadeOut");
        assert!(item.get("transition").is_none() || item["transition"].is_null());
    }

    #[test]
    fn motion_binds_intro_outro_onto_elements_for_emit() {
        let nodes = vec![
            node("txt", "script", &[("text", json!("一只螃蟹"))]),
            node("draw", "image_gen", &[("size", json!("1024x1024"))]),
            node(
                "fx",
                "motion",
                &[
                    ("intro", json!("pop")),
                    ("outro", json!("fadeOut")),
                    ("transition", json!("dissolve")),
                ],
            ),
            node("out", "emit", &[]),
        ];
        let edges = vec![
            edge("txt", "script", "draw", "prompt"),
            edge("draw", "images", "fx", "elements"),
            edge("fx", "elements", "out", "payload"),
        ];
        let g = graph(nodes, edges);
        assert!(
            g.validate(&BTreeSet::new()).is_empty(),
            "{:?}",
            g.validate(&BTreeSet::new())
        );
        let mut services = Mock::default();
        let mut emitted = Vec::new();
        let report = run(
            &g,
            Path::new("."),
            &mut services,
            &RunInput::default(),
            &mut |v| emitted.push(v),
        );
        assert!(report.ok(), "{report:?}");
        assert_eq!(emitted.len(), 1);
        let item = &emitted[0]["items"][0];
        assert_eq!(item["intro"]["kind"], "pop");
        assert_eq!(item["outro"]["kind"], "fadeOut");
        assert_eq!(item["transition"]["kind"], "dissolve");
        assert!(item["source"]["file"].as_str().unwrap().ends_with(".png"));
    }

    #[test]
    fn image_gen_draws_one_per_text_block() {
        let nodes = vec![
            node(
                "hub",
                "relay",
                &[(
                    "blocks",
                    json!([
                        {"id": "a", "text": "第一段画面"},
                        {"id": "b", "text": "第二段画面"},
                        {"id": "c", "text": "第三段画面"}
                    ]),
                )],
            ),
            node(
                "draw",
                "image_gen",
                &[
                    ("size", json!("1024x1024")),
                    ("style", json!("扁平插画")),
                ],
            ),
        ];
        let edges = vec![edge("hub", "text", "draw", "prompt")];
        let g = graph(nodes, edges);
        assert!(
            g.validate(&BTreeSet::new()).is_empty(),
            "{:?}",
            g.validate(&BTreeSet::new())
        );
        let mut services = Mock::default();
        let report = run(
            &g,
            Path::new("."),
            &mut services,
            &RunInput::default(),
            &mut |_| {},
        );
        assert!(report.ok(), "{report:?}");
        let calls = services.image_calls.borrow();
        assert_eq!(calls.len(), 3);
        assert!(calls[0].prompt.contains("第一段画面"));
        assert!(calls[0].prompt.contains("扁平插画"));
        assert!(calls[2].prompt.contains("第三段画面"));
        let draw = report.nodes.iter().find(|n| n.id == "draw").unwrap();
        assert!(draw.outputs.iter().any(|p| p == "images"));
        assert!(draw.outputs.iter().any(|p| p == "image"));
        let preview = draw.preview.clone().unwrap_or_default();
        let value: Value = serde_json::from_str(&preview).unwrap_or(json!({}));
        let list = value["images"].as_array().expect(&preview);
        assert_eq!(list.len(), 3);
        assert_eq!(list[0]["id"], "a");
        assert_eq!(list[0]["path"], "out/a1.png");
        assert_eq!(list[2]["path"], "out/a3.png");
    }

    #[test]
    fn image_gen_fans_out_in_parallel_and_mirrors_to_relay() {
        let blocks = json!([
            {"id": "a", "text": "第一段"},
            {"id": "b", "text": "第二段"},
            {"id": "c", "text": "第三段"}
        ]);
        let nodes = vec![
            node("src", "relay", &[("blocks", blocks)]),
            node("draw", "image_gen", &[("size", json!("1024x1024"))]),
            node("hub", "relay", &[]),
        ];
        let edges = vec![
            edge("src", "text", "draw", "prompt"),
            edge("draw", "images", "hub", "in"),
        ];
        let g = graph(nodes, edges);
        let mut job = WorkflowJob::new(&g, Path::new("."), RunInput::default());
        let Step::Calls(first) = job.advance() else {
            panic!("expected parallel image Calls");
        };
        assert_eq!(first.len(), 3, "{first:?}");
        let mut keys: Vec<_> = first.iter().map(|(k, _)| k.as_str()).collect();
        keys.sort();
        assert_eq!(keys, vec!["draw#0", "draw#1", "draw#2"]);

        job.resume(
            "draw#1",
            Ok(json!({ "path": "out/b.png", "mime": "image/png" })),
        );
        let hub = job.report().nodes.iter().find(|n| n.id == "hub").unwrap();
        assert!(hub.message.starts_with("进行中 1/3"), "{}", hub.message);
        let files = hub.files.as_ref().unwrap();
        let list = files["images"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["path"], "out/b.png");

        job.resume(
            "draw#0",
            Ok(json!({ "path": "out/a.png", "mime": "image/png" })),
        );
        job.resume(
            "draw#2",
            Ok(json!({ "path": "out/c.png", "mime": "image/png" })),
        );
        assert!(matches!(job.advance(), Step::Finished));
        assert!(job.report().ok(), "{:?}", job.report());
        let hub = job.report().nodes.iter().find(|n| n.id == "hub").unwrap();
        assert!(!hub.message.starts_with("进行中"), "{}", hub.message);
        let files = hub.files.as_ref().unwrap();
        assert_eq!(files["files"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn job_parks_on_a_service_call_and_continues() {
        let nodes = vec![
            node("txt", "script", &[("text", json!("第一句。第二句。"))]),
            node("voice", "tts", &[("voice", json!("S_x"))]),
            node("out", "emit", &[]),
        ];
        let edges = vec![
            edge("txt", "script", "voice", "text"),
            edge("voice", "clips", "out", "clips"),
            edge("voice", "media", "out", "media"),
        ];
        let g = graph(nodes, edges);
        let mut job = WorkflowJob::new(&g, Path::new("."), RunInput::default());
        let mut parks = 0;
        loop {
            match job.advance() {
                Step::Calls(batch) => {
                    for (node, _call) in batch {
                        parks += 1;
                        assert!(job.waiting_on().is_some());
                        job.resume(
                            &node,
                            Ok(json!({
                                "file": "out/a.wav",
                                "duration": 1.0,
                                "timeline": { "text": "第一句。第二句。", "duration": 1.0,
                                    "chars": [{ "text": "第", "start": 0.0, "end": 0.2 }] }
                            })),
                        );
                    }
                }
                Step::Waiting => panic!("刚 resume 完不应处于等待"),
                Step::Finished => break,
            }
        }
        assert_eq!(parks, 1, "单段文本 tts 只挂起一次");
        assert!(job.report().ok(), "{:?}", job.report());
        assert_eq!(job.progress(), (3, 3));
    }

    #[test]
    fn emit_expands_speech_clips_like_emit_sequence() {
        let nodes = vec![
            node(
                "seg",
                "relay",
                &[(
                    "blocks",
                    json!([
                        {"id": "a", "text": "第一段"},
                        {"id": "b", "text": "第二段"}
                    ]),
                )],
            ),
            node("voice", "tts", &[("voice", json!("S_x"))]),
            node("push", "emit", &[]),
        ];
        let edges = vec![
            edge("seg", "segments", "voice", "text"),
            edge("voice", "clips", "push", "clips"),
            edge("voice", "media", "push", "media"),
        ];
        let g = graph(nodes, edges);
        let mut services = Mock::default();
        let mut emitted = Vec::new();
        let report = run(
            &g,
            Path::new("."),
            &mut services,
            &RunInput::default(),
            &mut |value| emitted.push(value),
        );
        assert!(report.ok(), "{report:?}");
        assert_eq!(*services.speech_calls.borrow(), 2);
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0]["items"].as_array().unwrap().len(), 2);
        let voice = report.nodes.iter().find(|n| n.id == "voice").unwrap();
        let preview = voice.preview.as_deref().unwrap_or("");
        assert!(
            !preview.contains("\"file\""),
            "clips 尺口不应再带 file: {preview}"
        );
        assert!(
            preview.contains("media") || report.nodes.iter().any(|n| {
                n.id == "voice"
                    && n.outputs.iter().any(|p| p == "media")
            }),
            "tts 应产出 media 口: {:?}",
            voice.outputs
        );
        assert!(voice.outputs.iter().any(|p| p == "clips"));
        assert!(voice.outputs.iter().any(|p| p == "media"));
    }

    fn sample_clip(id: &str, file: &str, duration: f64) -> Value {
        let n = (duration / 0.1).round().max(1.0) as usize;
        let chars: Vec<Value> = (0..n)
            .map(|i| {
                json!({
                    "text": "字",
                    "start": i as f64 * 0.1,
                    "end": (i as f64 + 1.0) * 0.1
                })
            })
            .collect();
        json!({
            "id": id,
            "file": file,
            "duration": duration,
            "timeline": {
                "text": "字".repeat(n),
                "duration": duration,
                "chars": chars
            }
        })
    }

    #[test]
    fn emit_matches_visuals_to_clips_by_id_then_index() {
        let clips = json!({
            "clips": [
                sample_clip("s1", "out/a.wav", 1.0),
                sample_clip("s2", "out/b.wav", 2.0),
                sample_clip("s3", "out/c.wav", 0.5)
            ]
        });
        // 故意打乱顺序：靠 id 对齐
        let visuals = json!({
            "images": [
                {"id": "s2", "path": "out/p2.png"},
                {"id": "s1", "path": "out/p1.png"},
                {"id": "s3", "path": "out/p3.png"}
            ]
        });
        let matched = emit_clips_with_visuals(&clips, Some(&visuals)).unwrap();
        assert_eq!(matched.voice, 3);
        assert_eq!(matched.visual, 3);
        let items = matched.items["items"].as_array().unwrap();
        assert_eq!(items.len(), 6);
        let v1 = items.iter().find(|i| i["key"] == "visual/s1").unwrap();
        let v2 = items.iter().find(|i| i["key"] == "visual/s2").unwrap();
        let v3 = items.iter().find(|i| i["key"] == "visual/s3").unwrap();
        assert_eq!(v1["source"]["file"], "out/p1.png");
        assert_eq!(v1["start"], 0.0);
        assert_eq!(v1["duration"], 1.0);
        assert_eq!(v2["source"]["file"], "out/p2.png");
        assert_eq!(v2["start"], 1.0);
        assert_eq!(v2["duration"], 2.0);
        assert_eq!(v3["start"], 3.0);
        assert_eq!(v3["duration"], 0.5);
    }

    #[test]
    fn emit_matches_visuals_by_index_when_ids_missing() {
        let clips = json!({
            "clips": [
                sample_clip("a", "out/a.wav", 1.0),
                sample_clip("b", "out/b.wav", 1.5)
            ]
        });
        let visuals = json!({
            "files": [
                {"path": "shots/1.mp4", "kind": "video"},
                {"path": "shots/2.png", "kind": "image"}
            ]
        });
        let matched = emit_clips_with_visuals(&clips, Some(&visuals)).unwrap();
        assert_eq!(matched.visual, 2);
        let items = matched.items["items"].as_array().unwrap();
        let visuals: Vec<_> = items.iter().filter(|i| i["role"] == "visual").collect();
        assert_eq!(visuals[0]["source"]["file"], "shots/1.mp4");
        assert_eq!(visuals[0]["duration"], 1.0);
        assert_eq!(visuals[1]["source"]["file"], "shots/2.png");
        assert_eq!(visuals[1]["start"], 1.0);
        assert_eq!(visuals[1]["duration"], 1.5);
        for item in &visuals {
            serde_json::from_value::<todo_production::items::Item>((*item).clone())
                .unwrap_or_else(|e| panic!("visual item must decode for host placement: {e}"));
        }
    }

    #[test]
    fn emit_sequence_places_voice_before_late_visuals() {
        let g = graph(
            vec![
                node(
                    "hub",
                    "relay",
                    &[(
                        "blocks",
                        json!([
                            {"id": "a", "text": "甲"},
                            {"id": "b", "text": "乙"}
                        ]),
                    )],
                ),
                node("voice", "tts", &[("voice", json!("S_x"))]),
                node("draw", "image_gen", &[("size", json!("1024x1024"))]),
                node("mid", "relay", &[]),
                node("push", "emit_sequence", &[]),
            ],
            vec![
                edge("hub", "text", "voice", "text"),
                edge("hub", "text", "draw", "prompt"),
                edge("draw", "images", "mid", "in"),
                edge("voice", "clips", "push", "clips"),
                edge("voice", "media", "push", "media"),
                edge("mid", "images", "push", "visuals"),
            ],
        );
        let mut services = Mock::default();
        let mut emitted = Vec::new();
        let report = run(
            &g,
            Path::new("."),
            &mut services,
            &RunInput::default(),
            &mut |value| emitted.push(value),
        );
        assert!(report.ok(), "{report:?}");
        // Voice-first may yield 2 batches (voice, then visuals-only) or 1 if
        // both branches finished before emit ran.
        let voice_n: usize = emitted
            .iter()
            .flat_map(|b| b["items"].as_array().into_iter().flatten())
            .filter(|i| i["role"] == "voice")
            .count();
        let visual_n: usize = emitted
            .iter()
            .flat_map(|b| b["items"].as_array().into_iter().flatten())
            .filter(|i| i["role"] == "visual")
            .count();
        assert_eq!(voice_n, 2, "{emitted:?}");
        assert_eq!(visual_n, 2, "{emitted:?}");
        let voice_revs: Vec<&str> = emitted
            .iter()
            .flat_map(|b| b["items"].as_array().into_iter().flatten())
            .filter(|i| i["role"] == "voice")
            .filter_map(|i| i["rev"].as_str())
            .collect();
        assert!(
            voice_revs.iter().all(|r| r.contains("services/") || r.contains(".wav")),
            "voice rev must include file path so delete+re-run is not stuck: {voice_revs:?}"
        );
        assert!(
            emit_allows_voice_before_visuals(),
            "emit must not wait on optional visuals"
        );
    }

    fn emit_allows_voice_before_visuals() -> bool {
        // Contract: clips+media required, visuals optional → TTS can enter first.
        let spec = nodes::lookup("emit_sequence").unwrap();
        let clips = spec.inputs.iter().find(|p| p.name == "clips").unwrap();
        let media = spec.inputs.iter().find(|p| p.name == "media").unwrap();
        let visuals = spec.inputs.iter().find(|p| p.name == "visuals").unwrap();
        clips.required && media.required && !visuals.required
    }

    #[test]
    fn emit_visuals_uses_clips_clock_without_voice() {
        // Recommended graph: TTS → emit_sequence; TTS.clips + images → emit_visuals.
        let g = graph(
            vec![
                node(
                    "hub",
                    "relay",
                    &[(
                        "blocks",
                        json!([
                            {"id": "a", "text": "甲"},
                            {"id": "b", "text": "乙"}
                        ]),
                    )],
                ),
                node("voice", "tts", &[("voice", json!("S_x"))]),
                node("draw", "image_gen", &[("size", json!("1024x1024"))]),
                node("mid", "relay", &[]),
                node("push_voice", "emit_sequence", &[]),
                node("push_pic", "emit_visuals", &[]),
            ],
            vec![
                edge("hub", "text", "voice", "text"),
                edge("hub", "text", "draw", "prompt"),
                edge("draw", "images", "mid", "in"),
                edge("voice", "clips", "push_voice", "clips"),
                edge("voice", "media", "push_voice", "media"),
                edge("voice", "clips", "push_pic", "clips"),
                edge("mid", "images", "push_pic", "visuals"),
            ],
        );
        let mut services = Mock::default();
        let mut emitted = Vec::new();
        let report = run(
            &g,
            Path::new("."),
            &mut services,
            &RunInput::default(),
            &mut |value| emitted.push(value),
        );
        assert!(report.ok(), "{report:?}");
        let voice_n: usize = emitted
            .iter()
            .flat_map(|b| b["items"].as_array().into_iter().flatten())
            .filter(|i| i["role"] == "voice")
            .count();
        let visual_n: usize = emitted
            .iter()
            .flat_map(|b| b["items"].as_array().into_iter().flatten())
            .filter(|i| i["role"] == "visual")
            .count();
        assert_eq!(voice_n, 2, "voice only from emit_sequence: {emitted:?}");
        assert_eq!(visual_n, 2, "visuals only from emit_visuals: {emitted:?}");
        // First batch with voice must not wait for images.
        let first_voice_batch = emitted
            .iter()
            .position(|b| {
                b["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|i| i["role"] == "voice")
            })
            .expect("voice batch");
        let first_visual_batch = emitted
            .iter()
            .position(|b| {
                b["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|i| i["role"] == "visual")
            })
            .expect("visual batch");
        assert!(
            first_voice_batch <= first_visual_batch,
            "TTS 入轨应不晚于画面：{emitted:?}"
        );
        let pic = nodes::lookup("emit_visuals").unwrap();
        assert!(pic.inputs.iter().any(|p| p.name == "clips" && p.required));
        assert!(pic.inputs.iter().any(|p| p.name == "visuals" && p.required));
    }

    #[test]
    fn emit_node_wires_clips_and_visuals_ports() {
        let g = graph(
            vec![
                node(
                    "hub",
                    "relay",
                    &[(
                        "blocks",
                        json!([
                            {"id": "a", "text": "甲"},
                            {"id": "b", "text": "乙"}
                        ]),
                    )],
                ),
                node("voice", "tts", &[("voice", json!("S_x"))]),
                node("draw", "image_gen", &[("size", json!("1024x1024"))]),
                node("push", "emit", &[]),
            ],
            vec![
                edge("hub", "text", "voice", "text"),
                edge("hub", "text", "draw", "prompt"),
                edge("voice", "clips", "push", "clips"),
                edge("voice", "media", "push", "media"),
                edge("draw", "images", "push", "visuals"),
            ],
        );
        assert!(
            g.validate(&BTreeSet::new()).is_empty(),
            "{:?}",
            g.validate(&BTreeSet::new())
        );
        let mut services = Mock::default();
        let mut emitted = Vec::new();
        let report = run(
            &g,
            Path::new("."),
            &mut services,
            &RunInput::default(),
            &mut |value| emitted.push(value),
        );
        assert!(report.ok(), "{report:?}");
        assert_eq!(*services.speech_calls.borrow(), 2);
        assert_eq!(services.image_calls.borrow().len(), 2);
        assert_eq!(emitted.len(), 1);
        let items = emitted[0]["items"].as_array().unwrap();
        assert_eq!(items.len(), 4, "{items:?}");
        assert_eq!(items.iter().filter(|i| i["role"] == "voice").count(), 2);
        assert_eq!(items.iter().filter(|i| i["role"] == "visual").count(), 2);
        // 同源分段 id 对齐：画面 start 与对应配音一致
        let voice_a = items.iter().find(|i| i["key"] == "voice/a").unwrap();
        let visual_a = items.iter().find(|i| i["key"] == "visual/a").unwrap();
        assert_eq!(voice_a["start"], visual_a["start"]);
        assert_eq!(voice_a["duration"], visual_a["duration"]);
    }

    #[test]
    fn emit_partial_visuals_still_places_all_voice() {
        let clips = json!({
            "clips": [
                sample_clip("a", "out/a.wav", 1.0),
                sample_clip("b", "out/b.wav", 1.0),
                sample_clip("c", "out/c.wav", 1.0)
            ]
        });
        let visuals = json!({
            "images": [
                {"id": "a", "path": "out/a.png"}
            ]
        });
        let matched = emit_clips_with_visuals(&clips, Some(&visuals)).unwrap();
        assert_eq!(matched.voice, 3);
        assert_eq!(matched.visual, 1);
        let items = matched.items["items"].as_array().unwrap();
        assert_eq!(items.iter().filter(|i| i["role"] == "voice").count(), 3);
        assert_eq!(items.iter().filter(|i| i["role"] == "visual").count(), 1);
        assert!(items.iter().any(|i| i["key"] == "visual/a"));
    }

    fn clip_from_text(id: &str, file: &str, text: &str, duration: f64) -> Value {
        let chars: Vec<char> = text.chars().collect();
        let n = chars.len().max(1) as f64;
        let stamps: Vec<Value> = chars
            .iter()
            .enumerate()
            .map(|(i, ch)| {
                json!({
                    "text": ch.to_string(),
                    "start": duration * i as f64 / n,
                    "end": duration * (i as f64 + 1.0) / n
                })
            })
            .collect();
        json!({
            "id": id,
            "file": file,
            "duration": duration,
            "timeline": { "text": text, "duration": duration, "chars": stamps }
        })
    }

    #[test]
    fn emit_snaps_using_script_not_image_prompt() {
        // 二次分段 → 提示词改写后 text 对不上口播；必须靠 script 吸附。
        let clips = json!({
            "clips": [
                clip_from_text("coarse", "out/all.wav", "第一句话第二句话", 4.0)
            ]
        });
        let visuals = json!({
            "images": [
                {
                    "id": "fine1",
                    "path": "out/1.png",
                    "text": "cinematic shot of first sentence",
                    "prompt": "cinematic shot of first sentence",
                    "script": "第一句话"
                },
                {
                    "id": "fine2",
                    "path": "out/2.png",
                    "text": "melancholy second beat illustration",
                    "prompt": "melancholy second beat illustration",
                    "script": "第二句话"
                }
            ]
        });
        let matched = emit_clips_with_visuals(&clips, Some(&visuals)).unwrap();
        assert_eq!(matched.voice, 1);
        assert_eq!(matched.visual, 2, "{:?}", matched.notes);
        assert!(
            matched.notes.iter().any(|n| n.contains("文案吸附")),
            "{:?}",
            matched.notes
        );
        let items = matched.items["items"].as_array().unwrap();
        let v1 = items.iter().find(|i| i["key"] == "visual/fine1").unwrap();
        let v2 = items.iter().find(|i| i["key"] == "visual/fine2").unwrap();
        assert!(v1["start"].as_f64().unwrap() < v2["start"].as_f64().unwrap());
    }

    #[test]
    fn emit_snaps_fine_caption_onto_coarse_clip_chars() {
        // 粗分段一条口播；细分两句生图 —— 靠文案吸附，不靠同 id。
        let clips = json!({
            "clips": [
                clip_from_text("coarse", "out/all.wav", "第一句话第二句话", 4.0)
            ]
        });
        let visuals = json!({
            "images": [
                {"id": "fine1", "path": "out/1.png", "text": "第一句话"},
                {"id": "fine2", "path": "out/2.png", "text": "第二句话"}
            ]
        });
        let matched = emit_clips_with_visuals(&clips, Some(&visuals)).unwrap();
        assert_eq!(matched.voice, 1);
        assert_eq!(matched.visual, 2);
        assert!(
            matched.notes.iter().any(|n| n.contains("文案吸附")),
            "{:?}",
            matched.notes
        );
        let items = matched.items["items"].as_array().unwrap();
        let v1 = items.iter().find(|i| i["key"] == "visual/fine1").unwrap();
        let v2 = items.iter().find(|i| i["key"] == "visual/fine2").unwrap();
        assert!((v1["start"].as_f64().unwrap() - 0.0).abs() < 1e-6);
        assert!((v1["duration"].as_f64().unwrap() - 2.0).abs() < 1e-6);
        assert!((v2["start"].as_f64().unwrap() - 2.0).abs() < 1e-6);
        assert!((v2["duration"].as_f64().unwrap() - 2.0).abs() < 1e-6);
    }

    #[test]
    fn tts_batches_each_segment_then_emit_sequence() {
        let nodes = vec![
            node(
                "txt",
                "script",
                &[("text", json!("第一句话。第二句话。第三句话。"))],
            ),
            node(
                "seg",
                "segment_script",
                &[
                    ("mode", json!("length")),
                    ("target", json!(5.0)),
                    ("max", json!(8.0)),
                ],
            ),
            node("voice", "tts", &[("voice", json!("S_x"))]),
            node("push", "emit_sequence", &[]),
            node("end", "done", &[]),
        ];
        let edges = vec![
            edge("txt", "script", "seg", "text"),
            edge("seg", "segments", "voice", "text"),
            edge("voice", "clips", "push", "clips"),
            edge("voice", "media", "push", "media"),
            edge("push", "items", "end", "result"),
        ];
        let g = graph(nodes, edges);
        assert!(
            g.validate(&BTreeSet::new()).is_empty(),
            "{:?}",
            g.validate(&BTreeSet::new())
        );
        let mut services = Mock::default();
        let mut emitted = Vec::new();
        let report = run(
            &g,
            Path::new("."),
            &mut services,
            &RunInput::default(),
            &mut |value| emitted.push(value),
        );
        assert!(report.ok(), "{report:?}");
        let speech_n = *services.speech_calls.borrow();
        assert!(speech_n >= 2, "应逐段配音，实际 {speech_n}");
        assert_eq!(emitted.len(), 1);
        let items = emitted[0]["items"].as_array().unwrap();
        assert_eq!(items.len(), speech_n);
        let mut cursor = 0.0;
        for (i, item) in items.iter().enumerate() {
            assert_eq!(item["role"], "voice");
            assert_eq!(item["start"], json!(cursor));
            let dur = item["duration"].as_f64().unwrap();
            cursor += dur;
            assert!(
                item["source"]["file"]
                    .as_str()
                    .unwrap()
                    .ends_with(".wav"),
                "item {i} file"
            );
        }
    }

    #[test]
    fn relay_text_port_preserves_blocks_for_streaming_tts() {
        let g = graph(vec![
            node("relay", "relay", &[("blocks", json!([
                {"id": "a", "text": "第一段"}, {"id": "b", "text": "第二段"}
            ]))]),
            node("voice", "tts", &[("handoff", json!("stream"))]),
        ], vec![edge("relay", "text", "voice", "text")]);
        let mut services = Mock::default();
        let mut emitted = Vec::new();
        let report = run(&g, Path::new("."), &mut services, &RunInput::default(),
            &mut |value| emitted.push(value));
        assert!(report.ok(), "{report:?}");
        assert_eq!(*services.speech_calls.borrow(), 2);
        assert_eq!(emitted.len(), 2);
    }

    #[test]
    fn tts_sanitizes_backwards_char_timestamps() {
        let reply = json!({
            "file": "out/jitter.wav",
            "duration": 2.0,
            "timeline": {
                "text": "一二三四五",
                "duration": 2.0,
                "chars": [
                    {"text": "一", "start": 0.0, "end": 0.4},
                    {"text": "二", "start": 0.35, "end": 0.7},
                    {"text": "三", "start": 0.2, "end": 0.5},
                    {"text": "四", "start": 1.0, "end": 1.3},
                    {"text": "五", "start": 1.2, "end": 2.5}
                ]
            }
        });
        let clip = normalize_tts_clip(reply, "seg1", "一二三四五").expect("sanitize");
        let chars = clip["timeline"]["chars"].as_array().unwrap();
        let mut prev = 0.0;
        for ch in chars {
            let start = ch["start"].as_f64().unwrap();
            let end = ch["end"].as_f64().unwrap();
            assert!(start + 1e-6 >= prev, "start={start} prev={prev}");
            assert!(end >= start);
            assert!(end <= 2.0 + 1e-6);
            prev = end;
        }
        nodes::validate_payload(DataKind::SpeechClip, &clip).unwrap();
    }

    #[test]
    fn tts_stream_handoff_emits_each_clip_immediately() {
        let nodes = vec![
            node(
                "seg",
                "relay",
                &[(
                    "blocks",
                    json!([
                        {"id": "a", "text": "第一段"},
                        {"id": "b", "text": "第二段"},
                        {"id": "c", "text": "第三段"}
                    ]),
                )],
            ),
            node(
                "voice",
                "tts",
                &[("voice", json!("S_x")), ("handoff", json!("stream"))],
            ),
            node("push", "emit_sequence", &[]),
        ];
        let edges = vec![
            edge("seg", "segments", "voice", "text"),
            edge("voice", "clips", "push", "clips"),
            edge("voice", "media", "push", "media"),
        ];
        let g = graph(nodes, edges);
        let mut services = Mock::default();
        let mut emitted = Vec::new();
        let report = run(
            &g,
            Path::new("."),
            &mut services,
            &RunInput::default(),
            &mut |value| emitted.push(value),
        );
        assert!(report.ok(), "{report:?}");
        // Three progressive emits from TTS; emit_sequence skips because streamed.
        assert_eq!(emitted.len(), 3, "{emitted:?}");
        assert_eq!(*services.speech_calls.borrow(), 3);
        let mut cursor = 0.0;
        for (i, batch) in emitted.iter().enumerate() {
            let items = batch["items"].as_array().unwrap();
            assert_eq!(items.len(), 1, "stream emit {i}");
            assert_eq!(items[0]["start"], json!(cursor));
            cursor += items[0]["duration"].as_f64().unwrap();
        }
        let voice = report.nodes.iter().find(|n| n.id == "voice").unwrap();
        assert!(
            voice
                .preview
                .as_ref()
                .is_some_and(|p| p.contains("streamed")),
            "clips should mark streamed: {:?}",
            voice.preview
        );
    }

    #[test]
    fn tts_batch_job_re_parks_until_all_segments_done() {
        // Feed segments directly so the test does not depend on length heuristics.
        let nodes = vec![
            node(
                "seg",
                "relay",
                &[(
                    "blocks",
                    json!([
                        {"id": "a", "text": "第一段文案"},
                        {"id": "b", "text": "第二段文案"},
                        {"id": "c", "text": "第三段文案"}
                    ]),
                )],
            ),
            node("voice", "tts", &[("voice", json!("S_x"))]),
        ];
        let edges = vec![edge("seg", "segments", "voice", "text")];
        let g = graph(nodes, edges);
        let mut job = WorkflowJob::new(&g, Path::new("."), RunInput::default());
        let mut parks = 0;
        loop {
            match job.advance() {
                Step::Calls(batch) => {
                    for (node, call) in batch {
                        parks += 1;
                        let RequestedCall::Speech(speech) = call else {
                            panic!("expected speech call, got {call:?}");
                        };
                        let chars: Vec<Value> = speech
                            .text
                            .chars()
                            .enumerate()
                            .map(|(index, ch)| {
                                json!({
                                    "text": ch.to_string(),
                                    "start": index as f64 * 0.1,
                                    "end": (index + 1) as f64 * 0.1
                                })
                            })
                            .collect();
                        let duration = chars.len() as f64 * 0.1;
                        job.resume(
                            &node,
                            Ok(json!({
                                "file": format!("out/b{parks}.wav"),
                                "duration": duration,
                                "timeline": {
                                    "text": speech.text,
                                    "duration": duration,
                                    "chars": chars
                                }
                            })),
                        );
                    }
                }
                Step::Waiting => panic!("刚 resume 完不应 Waiting"),
                Step::Finished => break,
            }
        }
        assert_eq!(parks, 3, "三段应挂起三次，实际 {parks}");
        assert!(job.report().ok(), "{:?}", job.report());
        let voice = job.report().nodes.iter().find(|n| n.id == "voice").unwrap();
        assert!(voice.ok, "{voice:?}");
        assert!(
            voice.outputs.iter().any(|p| p == "clips"),
            "应输出 clips：{:?}",
            voice.outputs
        );
    }

    #[test]
    fn independent_branches_start_service_calls_together() {
        let nodes = vec![
            node("txt", "script", &[("text", json!("共用上游"))]),
            node("left", "llm_chat", &[("system", json!("左"))]),
            node("right", "llm_chat", &[("system", json!("右"))]),
        ];
        let edges = vec![
            edge("txt", "script", "left", "prompt"),
            edge("txt", "script", "right", "prompt"),
        ];
        let g = graph(nodes, edges);
        let mut job = WorkflowJob::new(&g, Path::new("."), RunInput::default());
        match job.advance() {
            Step::Calls(batch) => {
                assert_eq!(batch.len(), 2, "两分支应同时挂起：{batch:?}");
                let mut ids: Vec<_> = batch.iter().map(|(n, _)| n.as_str()).collect();
                ids.sort();
                assert_eq!(ids, vec!["left", "right"]);
                assert_eq!(job.waiting_nodes().len(), 2);
                for (node, _) in batch {
                    job.resume(&node, Ok(json!({ "text": format!("答{node}") })));
                }
            }
            other => panic!("expected parallel Calls, got {other:?}"),
        }
        assert!(matches!(job.advance(), Step::Finished));
        assert!(job.report().ok(), "{:?}", job.report());
        assert_eq!(job.progress(), (3, 3));
    }

    #[test]
    fn parallel_batch_continuations_are_kept_together() {
        // 同帧 resume 两路多段 TTS 时，续跑调用不能互相覆盖。
        let blocks = json!([
            {"id": "a", "text": "甲一段"},
            {"id": "b", "text": "甲二段"}
        ]);
        let nodes = vec![
            node("src", "relay", &[("blocks", blocks)]),
            node("left", "tts", &[("voice", json!("L"))]),
            node("right", "tts", &[("voice", json!("R"))]),
        ];
        let edges = vec![
            edge("src", "segments", "left", "text"),
            edge("src", "segments", "right", "text"),
        ];
        let g = graph(nodes, edges);
        let mut job = WorkflowJob::new(&g, Path::new("."), RunInput::default());
        let Step::Calls(first) = job.advance() else {
            panic!("expected first parallel Calls");
        };
        assert_eq!(first.len(), 2, "{first:?}");
        let speech_ok = |i: usize| {
            json!({
                "file": format!("out/{i}.wav"),
                "duration": 0.5,
                "timeline": {
                    "text": "x",
                    "duration": 0.5,
                    "chars": [{ "text": "x", "start": 0.0, "end": 0.5 }]
                }
            })
        };
        // 模拟 host 同 tick 结清两路第一段。
        for (node, _) in &first {
            job.resume(node, Ok(speech_ok(1)));
        }
        match job.advance() {
            Step::Calls(batch) => {
                assert_eq!(batch.len(), 2, "两路第二段应一起发出：{batch:?}");
                let mut ids: Vec<_> = batch.iter().map(|(n, _)| n.as_str()).collect();
                ids.sort();
                assert_eq!(ids, vec!["left", "right"]);
                for (node, _) in batch {
                    job.resume(&node, Ok(speech_ok(2)));
                }
            }
            other => panic!("expected both continuations, got {other:?}"),
        }
        assert!(matches!(job.advance(), Step::Finished));
        assert!(job.report().ok(), "{:?}", job.report());
    }

    #[test]
    fn long_script_is_not_truncated_and_nodes_time_themselves() {
        // 5200 字：脚本节点到分段节点必须原样传递
        let para = "蟹老板的餐厅最近生意冷清，顾客一进门就被隔壁的新店吸引走了。他决定推出限时优惠券。";
        let long_text: String = para.repeat(140);
        assert!(
            long_text.chars().count() > 5000,
            "chars = {}",
            long_text.chars().count()
        );
        let nodes = vec![
            node("txt", "script", &[("text", json!(long_text.clone()))]),
            node(
                "seg",
                "segment_script",
                &[("mode", json!("length")), ("target", json!(200.0)), ("max", json!(300.0))],
            ),
        ];
        let edges = vec![edge("txt", "script", "seg", "text")];
        let g = graph(nodes, edges);
        let mut services = Mock::default();
        let mut emitted = Vec::new();
        let report = run(&g, Path::new("."), &mut services, &RunInput::default(),
                         &mut |v| emitted.push(v));
        assert!(report.ok(), "{report:?}");
        let script = &report.nodes[0];
        assert_eq!(script.output_chars, Some(long_text.chars().count()));
        let body = report.nodes[1].preview.clone().unwrap();
        assert!(body.contains("segments"));
        // 用时字段有值（本地节点也要计时）
        assert!(report.nodes.iter().all(|n| n.duration_ms < 60_000));
    }
}
