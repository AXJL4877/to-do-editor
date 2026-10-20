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
use todo_production::extension::graph::{Graph, GraphEdge, GraphNode};
use todo_production::extension::nodes::{self, DataKind};
use todo_production::extension::protocol::{ChatCall, ImageCall, SpeechCall, SubtitlesCall};
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
            inputs.insert(in_port, value.clone());
        }
    }
    inputs
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
        let Some(spec) = nodes::lookup(&node.node_type) else {
            continue;
        };
        for port in spec.inputs.iter().filter(|port| port.required) {
            if !wired.contains(&(node.id.clone(), port.name.to_owned())) {
                missing.push(format!("{}.{}", node.id, port.name));
            }
        }
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
    /// Submit this call; call `resume` when it settles.
    Call(RequestedCall),
    /// Parked on a submitted call: wait for its result.
    Waiting,
    /// Nothing left; read `report()`.
    Finished,
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
}

enum NodeOutcome {
    Done(BTreeMap<String, Value>),
    Call {
        call: RequestedCall,
        resume: Resume,
    },
}

/// Resumable execution of one graph: advance until a node needs a call, park,
/// then resume with that call's result. Local nodes never park.
pub struct WorkflowJob {
    graph: Graph,
    base_dir: PathBuf,
    input: RunInput,
    order: Vec<String>,
    index: usize,
    produced: BTreeMap<(String, String), Value>,
    report: RunReport,
    pending: Option<(String, String, Resume)>,
    emitted_from: usize,
    finished: bool,
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
            order = topo.into_iter().map(str::to_owned).collect();
        } else {
            report.error = Some("图中有环，无法执行".into());
        }
        let finished = report.error.is_some();
        Self {
            graph: graph.clone(),
            base_dir: base_dir.to_path_buf(),
            input,
            order,
            index: 0,
            produced: BTreeMap::new(),
            report,
            pending: None,
            emitted_from: 0,
            finished,
        }
    }

    pub fn report(&self) -> &RunReport {
        &self.report
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    /// `(done, total)`.
    pub fn progress(&self) -> (usize, usize) {
        (self.index.min(self.order.len()), self.order.len())
    }

    /// Node waiting for its call, if any.
    pub fn waiting_on(&self) -> Option<&str> {
        self.pending.as_ref().map(|(node, _, _)| node.as_str())
    }

    /// Payloads not yet handed to the caller.
    pub fn drain_emitted(&mut self, sink: &mut dyn FnMut(Value)) {
        while self.emitted_from < self.report.emitted.len() {
            let value = self.report.emitted[self.emitted_from].clone();
            self.emitted_from += 1;
            sink(value);
        }
    }

    /// Run local nodes until one needs a service, or the graph ends.
    pub fn advance(&mut self) -> Step {
        while !self.finished {
            if self.pending.is_some() {
                return Step::Waiting;
            }
            let Some(id) = self.order.get(self.index).cloned() else {
                self.finished = true;
                break;
            };
            let Some(node) = self.graph.nodes.iter().find(|n| n.id == id).cloned() else {
                self.index += 1;
                continue;
            };
            let node_type = node.node_type.clone();
            let inputs = gather_inputs(&self.graph, &self.produced, &node);
            match step_node(&node, &node_type, &inputs, &self.base_dir, &self.input, &mut self.report)
            {
                Ok(NodeOutcome::Done(outputs)) => {
                    if let Err(message) = self.store_node(&node, &node_type, outputs) {
                        self.fail(&node.id, &node_type, message);
                        break;
                    }
                    self.index += 1;
                }
                Ok(NodeOutcome::Call { call, resume }) => {
                    self.pending = Some((node.id.clone(), call_port(&node_type).to_owned(), resume));
                    return Step::Call(call);
                }
                Err(message) => {
                    self.fail(&node.id, &node_type, message);
                    break;
                }
            }
        }
        Step::Finished
    }

    /// Result of the call returned by the last [`Self::advance`].
    pub fn resume(&mut self, result: Result<Value, String>) {
        let Some((id, _port, resume_kind)) = self.pending.take() else {
            return;
        };
        let Some(node) = self.graph.nodes.iter().find(|n| n.id == id).cloned() else {
            self.index += 1;
            return;
        };
        let node_type = node.node_type.clone();
        let finished = match (result, resume_kind) {
            (Err(message), _) => Err(message),
            (Ok(reply), Resume::Plain) => finish_call(&node, &node_type, reply),
            (Ok(reply), Resume::Segment { text, mode, limits }) => {
                segments_from_reply(&reply, &text, &mode, limits)
            }
        };
        match finished {
            Ok(outputs) => {
                if let Err(message) = self.store_node(&node, &node_type, outputs) {
                    self.fail(&node.id, &node_type, message);
                } else {
                    self.index += 1;
                }
            }
            Err(message) => self.fail(&node.id, &node_type, message),
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
        let first = names
            .first()
            .and_then(|port| self.produced.get(&(node.id.clone(), port.clone())));
        let preview = first.map(|value| {
            serde_json::to_string_pretty(value)
                .unwrap_or_default()
                .chars()
                .take(1200)
                .collect::<String>()
        });
        let image = first
            .and_then(|value| value.get("path"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        self.report.nodes.push(NodeRun {
            id: node.id.clone(),
            node_type: node_type.to_owned(),
            ok: true,
            message: String::new(),
            outputs: names,
            preview,
            image,
        });
        Ok(())
    }

    fn fail(&mut self, id: &str, node_type: &str, message: String) {
        self.report.nodes.push(NodeRun {
            id: id.to_owned(),
            node_type: node_type.to_owned(),
            ok: false,
            message: message.clone(),
            outputs: Vec::new(),
            preview: None,
            image: None,
        });
        self.report.error = Some(format!("{id}: {message}"));
        self.finished = true;
    }
}

/// Output port a service call fills.
fn call_port(node_type: &str) -> &'static str {
    match node_type {
        "image_gen" => "image",
        "tts" => "audio",
        "subtitles" => "cues",
        "segment_script" => "segments",
        _ => "reply",
    }
}

fn finish_call(
    node: &GraphNode,
    node_type: &str,
    reply: Value,
) -> Result<BTreeMap<String, Value>, String> {
    let _ = node;
    let mut out = BTreeMap::new();
    match node_type {
        "image_gen" => {
            let path = reply
                .get("path")
                .and_then(Value::as_str)
                .or_else(|| reply.get("file").and_then(Value::as_str))
                .ok_or_else(|| "生图结果缺少文件路径".to_string())?;
            let mut asset = json!({ "path": path });
            for key in ["mime", "width", "height"] {
                if let Some(value) = reply.get(key) {
                    asset[key] = value.clone();
                }
            }
            out.insert("image".into(), asset);
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
            out.insert("audio".into(), reply);
        }
        "subtitles" => {
            out.insert("cues".into(), reply);
        }
        other => return Err(format!("{other} 没有对应的外部调用")),
    }
    Ok(out)
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
            Step::Call(call) => {
                let result = services.execute(call);
                job.resume(result);
                job.drain_emitted(emit);
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
    input: &RunInput,
    report: &mut RunReport,
) -> Result<NodeOutcome, String> {
    let mut out = BTreeMap::new();
    match node_type {
        "trigger" => {
            out.insert("start".into(), json!({ "trigger": true, "at": input.at.unwrap_or(0.0) }));
        }
        "media_bin" => {
            let (files, text) = read_media_bin(node, base_dir)?;
            out.insert("files".into(), json!({ "files": files }));
            if !text.is_empty() {
                out.insert("text".into(), json!({ "text": text }));
            }
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
                _ => "semantic",
            }
            .to_owned();
            let limits = SegmentLimits {
                target: param_num(node, "target", 120.0) as usize,
                max: param_num(node, "max", 200.0) as usize,
            };
            if mode == "semantic" {
                let units = segment::split_units(&text, limits.max);
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
                Some(Value::Bool(flag)) => *flag,
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
        "image_gen" => {
            let mut prompt = text_of(inputs.get("prompt"));
            if prompt.trim().is_empty() {
                prompt = param_str(node, "prompt");
            }
            if prompt.trim().is_empty() {
                return Err("image_gen 缺 prompt".into());
            }
            let style = param_str(node, "style");
            if !style.trim().is_empty() {
                prompt = format!("{prompt}，{style}");
            }
            return Ok(NodeOutcome::Call {
                call: RequestedCall::Image(ImageCall {
                    prompt,
                    size: node
                        .params
                        .get("size")
                        .and_then(Value::as_str)
                        .unwrap_or("1024x1024")
                        .to_owned(),
                    quality: node
                        .params
                        .get("quality")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    transparent: node
                        .params
                        .get("transparent")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    variant: node.params.get("variant").and_then(Value::as_u64).unwrap_or(0) as u32,
                    model: node
                        .params
                        .get("model")
                        .and_then(Value::as_str)
                        .filter(|name| !name.trim().is_empty())
                        .map(str::to_owned),
                    references: Vec::new(),
                }),
                resume: Resume::Plain,
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
            let text = text_of(inputs.get("text"));
            if text.trim().is_empty() {
                return Err("tts 缺文本".into());
            }
            return Ok(NodeOutcome::Call {
                call: RequestedCall::Speech(SpeechCall {
                    text,
                    voice: param_str(node, "voice"),
                    sample_rate: None,
                    rate: node.params.get("rate").and_then(Value::as_i64).map(|v| v as i32),
                }),
                resume: Resume::Plain,
            });
        }
        "subtitles" => {
            let script = text_of(inputs.get("script"));
            let speech = inputs
                .get("speech")
                .and_then(|v| v.get("file"))
                .and_then(Value::as_str)
                .ok_or_else(|| "字幕缺 speech.file".to_string())?
                .to_owned();
            return Ok(NodeOutcome::Call {
                call: RequestedCall::Subtitles(SubtitlesCall {
                    script,
                    speech,
                    max_chars: node
                        .params
                        .get("maxChars")
                        .and_then(Value::as_u64)
                        .map(|v| v as usize),
                }),
                resume: Resume::Plain,
            });
        }
        "emit" => {
            let payload = inputs
                .get("payload")
                .cloned()
                .ok_or_else(|| "入轨缺 payload".to_string())?;
            let items = match &payload {
                Value::Array(_) => payload.clone(),
                Value::Object(map) if map.contains_key("items") => payload.clone(),
                other => json!({ "items": [other.clone()] }),
            };
            report.emitted.push(items.clone());
            out.insert("items".into(), items);
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
        "ground_emit" => {
            let plan = inputs
                .get("plan")
                .cloned()
                .ok_or_else(|| "ground_emit 缺 plan".to_string())?;
            let items = json!({ "items": ground_items(&plan) });
            report.emitted.push(items.clone());
            out.insert("items".into(), items);
        }
        "picture_batch" => {
            let plan = inputs
                .get("plan")
                .cloned()
                .ok_or_else(|| "picture_batch 缺 plan".to_string())?;
            let items = json!({ "items": picture_items(&plan) });
            report.emitted.push(items.clone());
            out.insert("items".into(), items);
        }
        "split_shots" => {
            let speech = inputs
                .get("speech")
                .ok_or_else(|| "拆镜缺 speech".to_string())?;
            let cues = inputs.get("cues").cloned().unwrap_or_else(|| json!({ "cues": [] }));
            out.insert("shots".into(), split_shots(speech, &cues));
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

/// `media_bin` params.files → (normalized file list, concatenated text files).
fn read_media_bin(node: &GraphNode, base_dir: &Path) -> Result<(Vec<Value>, String), String> {
    let mut files = Vec::new();
    let mut text = String::new();
    let Some(list) = node.params.get("files").and_then(Value::as_array) else {
        return Ok((files, text));
    };
    for item in list {
        let (path, kind) = match item {
            Value::String(path) => (path.clone(), String::new()),
            Value::Object(map) => (
                map.get("path").and_then(Value::as_str).unwrap_or_default().to_owned(),
                map.get("kind").and_then(Value::as_str).unwrap_or_default().to_owned(),
            ),
            _ => continue,
        };
        if path.is_empty() {
            continue;
        }
        let absolute = base_dir.join(&path);
        if !absolute.is_file() {
            return Err(format!("素材不存在：{path}"));
        }
        let kind = if kind.is_empty() { kind_of(&absolute) } else { kind };
        if kind == "text" {
            if let Ok(body) = std::fs::read_to_string(&absolute) {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(body.trim());
            }
        }
        files.push(json!({
            "path": path,
            "kind": kind,
            "name": absolute.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
        }));
    }
    Ok((files, text))
}

fn kind_of(path: &Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "txt" | "md" | "srt" | "json" => "text",
        "mp3" | "wav" | "m4a" | "aac" | "flac" => "audio",
        "mp4" | "mov" | "mkv" | "webm" => "video",
        "png" | "jpg" | "jpeg" | "webp" | "bmp" => "image",
        _ => "other",
    }
    .to_owned()
}

/// Rule-based segmentation (`length` / `paragraph`).
fn segment_by_rules(text: &str, mode: &str, limits: SegmentLimits) -> Value {
    let units = segment::split_units(text, limits.max);
    let starts = segment::heuristic_starts(&units, limits, mode == "paragraph");
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
    json!({ "mode": mode, "segments": segments })
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
            json!({ "mode": mode, "segments": segments })
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
        _ => "semantic",
    }
    .to_owned();
    let limits = SegmentLimits {
        target: param_num(node, "target", 120.0) as usize,
        max: param_num(node, "max", 200.0) as usize,
    };
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

/// Deterministic motion assignment from the built-in whitelist.
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
    let duration = speech.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
    let list = cues
        .get("cues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
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
    json!({ "shots": shots, "duration": duration })
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
    use todo_production::extension::graph::{GraphEdge, GraphNode, GRAPH_SPEC};

    #[derive(Default)]
    struct Mock {
        chat_calls: RefCell<Vec<ChatCall>>,
        reply: Value,
        speech_calls: RefCell<usize>,
    }

    impl Services for Mock {
        fn chat(&mut self, call: ChatCall) -> Result<Value, String> {
            self.chat_calls.borrow_mut().push(call);
            Ok(self.reply.clone())
        }
        fn image(&mut self, _call: ImageCall) -> Result<Value, String> {
            Ok(json!({ "path": "out/a.png", "mime": "image/png" }))
        }
        fn vision(&mut self, _prompt: String, images: Vec<PathBuf>) -> Result<Value, String> {
            Ok(json!({ "text": format!("{} 张图", images.len()) }))
        }
        fn speech(&mut self, call: SpeechCall) -> Result<Value, String> {
            *self.speech_calls.borrow_mut() += 1;
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
            Ok(json!({ "file": "out/a.wav", "duration": duration,
                       "timeline": { "text": call.text, "duration": duration, "chars": chars } }))
        }
        fn subtitles(&mut self, _call: SubtitlesCall) -> Result<Value, String> {
            Ok(json!({ "cues": [{ "text": "你好", "start": 0.0, "end": 1.0 }] }))
        }
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
            edge("voice", "audio", "subs", "speech"),
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
        assert_eq!(emitted.len(), 1);
        assert_eq!(*services.speech_calls.borrow(), 1);
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
        let g = graph(vec![node("ask", "llm_chat", &[])], vec![]);
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
    fn image_node_returns_a_validated_asset() {
        let nodes = vec![
            node("txt", "script", &[("text", json!("一只红色螃蟹老板"))]),
            node("draw", "image_gen", &[("size", json!("1024x1024")), ("transparent", json!(true))]),
            node("out", "emit", &[]),
        ];
        let edges = vec![
            edge("txt", "script", "draw", "prompt"),
            edge("draw", "image", "out", "payload"),
        ];
        let g = graph(nodes, edges);
        let mut services = Mock::default();
        let mut emitted = Vec::new();
        let report = run(&g, Path::new("."), &mut services, &RunInput::default(),
                         &mut |v| emitted.push(v));
        assert!(report.ok(), "{report:?}");
        assert_eq!(emitted.len(), 1);
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
            edge("voice", "audio", "out", "payload"),
        ];
        let g = graph(nodes, edges);
        let mut job = WorkflowJob::new(&g, Path::new("."), RunInput::default());
        let mut parks = 0;
        loop {
            match job.advance() {
                Step::Call(_call) => {
                    parks += 1;
                    assert!(job.waiting_on().is_some());
                    job.resume(Ok(json!({
                        "file": "out/a.wav",
                        "duration": 1.0,
                        "timeline": { "text": "第一句。第二句。", "duration": 1.0,
                            "chars": [{ "text": "第", "start": 0.0, "end": 0.2 }] }
                    })));
                }
                Step::Waiting => panic!("刚 resume 完不应处于等待"),
                Step::Finished => break,
            }
        }
        assert_eq!(parks, 1, "tts 应该只挂起一次");
        assert!(job.report().ok(), "{:?}", job.report());
        assert_eq!(job.progress(), (3, 3));
    }
}
