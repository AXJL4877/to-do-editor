//! Executes production tasks: AI requests, extension runs, saves; reports
//! results back as `ProductionOutput`. It decides nothing about what to
//! generate — that is `todo_application::productions`.
use super::bridge::{materialize, service_file, ServiceBridge};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use todo_ai::{LanguageEndpoint,
    default_paths, AiConfig, AiEvent, AiRouter, CapabilityBindings, ModelBinding, ModelInfo,
    Secret, Secrets, ServiceSettings,
};
use todo_application::{
    LocalRunId, Pcm16, ProductionOutput, ProductionSetup, ProductionTask, Recipe, TaskState,
};
use todo_core::Project;
use todo_workflow::{
    copy_package, create_blank_workflow, Budget, Extension, Origin, PoolOptions, Registry, RunId,
    RunRequest, RuntimePaths, WorkerEvent, WorkerPool,
};
use todo_interaction::{
    ChoiceView, ExtensionCardView, ExtensionStatus, ParamInput, ParamValue, ParamView, ScriptView,
};
use todo_workflow::workflow::manifest::{Group, Kind, Param, Service};
use todo_workflow::workflow::protocol::Canvas;
use todo_workflow::workflow::{narration_spec, HostDefaults};
use todo_production::production::PackRef;
use todo_production::segment::Limits;
use todo_production::{
    Capability, ChatMessage, ChatRole, Production, Provider, SplitMode, TaskKey, TaskRequest,
    TaskSpec,
};
use todo_storage::ProductionStore;
use super::workflow_cache::WorkflowCache;

/// Voices offered next to free entry (any id, cloned voices `S_…`).
const VOICES: &[(&str, &str)] = &[
    ("S_RksT6Z6g2", "克隆音色（口播）"),
    ("zh_female_mizai_uranus_bigtts", "咪仔（女）"),
    ("zh_male_m191_uranus_bigtts", "云舟（男）"),
    ("zh_female_vv_uranus_bigtts", "Vivi（女）"),
];

#[derive(Debug, Clone)]
struct Submission {
    spec: TaskSpec,
    capability: Capability,
    workdirs: Vec<PathBuf>,
}

pub enum HostReport {
    Output(ProductionOutput),
    Notice(String),
    Models {
        provider: Provider,
        models: Vec<ModelInfo>,
    },
    ModelsFailed {
        provider: Provider,
        class: String,
        message: String,
    },
    /// Result of the explicit chat connection test.
    Connection {
        provider: Provider,
        ok: bool,
        message: String,
    },
    /// The answer to a director request (language or multimodal).
    AgentReply {
        request_id: String,
        result: Result<String, String>,
    },
}

fn decode_workflow_items(value: serde_json::Value) -> Result<Vec<todo_production::items::Item>, String> {
    let payload = value
        .get("items")
        .cloned()
        .unwrap_or(value);
    serde_json::from_value(payload).map_err(|error| format!("渐进入轨条目无效：{error}"))
}

/// Plain words for a service error class (settings dialog).
pub fn class_reason(class: &str) -> &'static str {
    match class {
        "auth" => "Key 无效或已过期",
        "billing" => "账户余额不足",
        "rate_limited" => "请求太频繁，被限流",
        "invalid" => "服务不接受这个请求，模型名可能不对",
        "content_policy" => "请求被安全审核拦截",
        "transient" | "ambiguous" => "网络或服务暂时不可用",
        _ => "连接失败",
    }
}

/// The reason, plus the service's own words only when the class says nothing.
pub fn class_message(class: &str, message: &str) -> String {
    match class_reason(class) {
        "连接失败" if !message.trim().is_empty() => {
            let short: String = message.chars().take(80).collect();
            format!("连接失败：{short}")
        }
        reason => reason.to_owned(),
    }
}

/// 把检查器里的临时改动写进图（数字保持数字，其余按字符串）。
fn apply_workflow_edits(
    graph: &mut todo_workflow::workflow::graph::Graph,
    edits: &BTreeMap<String, BTreeMap<String, String>>,
) {
    for node in &mut graph.nodes {
        let Some(fields) = edits.get(&node.id) else {
            continue;
        };
        for (key, raw) in fields {
            let trimmed = raw.trim();
            let value = if let Ok(number) = trimmed.parse::<f64>() {
                serde_json::json!(number)
            } else if trimmed == "true" || trimmed == "false" {
                serde_json::json!(trimmed == "true")
            } else {
                serde_json::json!(raw)
            };
            node.params.insert(key.clone(), value);
        }
    }
}

/// A workflow mid-flight: the job plus in-flight service calls (parallel OK).
struct WorkflowLive {
    job: todo_workflow::runner::WorkflowJob,
    /// node_id → (call id, via AI router, started)
    calls: HashMap<String, (String, bool, Instant)>,
    run_id: RunId,
    workdir: PathBuf,
    /// Extension package root (graph base_dir); used to stage media_bin refs into workdir.
    extension_dir: PathBuf,
    /// 原始终端日志（真实文件，可被外部 tail）。
    log_path: PathBuf,
    /// 已写进日志文件的节点日志行数。
    logged: usize,
    started: Instant,
    /// 上一次往终端写"还在等"心跳的时间。
    last_beat: Instant,
    /// 每个 call_id 已发送的次数（并行槽各自计数，避免 67 路首发被当成同一次重试）。
    attempts: HashMap<String, u32>,
}

pub struct ProductionHost {
    root: PathBuf,
    ai: AiRouter,
    bridge: ServiceBridge,
    workers: WorkerPool,
    worker_events: mpsc::Receiver<WorkerEvent>,
    registry: Registry,
    store: ProductionStore,
    saver: mpsc::Sender<(String, Box<Production>)>,
    saved: mpsc::Receiver<Result<(), String>>,
    submissions: HashMap<TaskKey, Submission>,
    local_runs: HashMap<LocalRunId, RunId>,
    pool_runs: HashMap<RunId, LocalRunId>,
    budgets: HashMap<PathBuf, Arc<Budget>>,
    checks: mpsc::Receiver<(String, Result<(), String>)>,
    check_results: HashMap<String, Result<(), String>>,
    /// 最近一次工作流运行的逐节点结果，供面板显示状态与产物。
    workflow_runs: HashMap<String, todo_workflow::RunReport>,
    /// 正在跑的工作流：每帧推进一个节点，服务调用挂起等事件。
    workflow_live: HashMap<String, WorkflowLive>,
    /// 网关返回的模型目录（填一次 Key 后自动拉取，供所有下拉使用）。
    models: Vec<todo_ai::models::ModelInfo>,
    models_error: Option<String>,
    /// 多模态任务 key → 调用 id（视觉结果回填）。
    workflow_keys: HashMap<TaskKey, String>,
    workflow_results: HashMap<String, Result<serde_json::Value, String>>,
    /// Local host calls (timeline strip, …) finish off the UI thread.
    local_call_tx: mpsc::Sender<(String, Result<serde_json::Value, String>)>,
    local_call_rx: mpsc::Receiver<(String, Result<serde_json::Value, String>)>,
    /// `placement_ready` call ids waiting for adopt/Place to go idle.
    placement_waits: HashMap<String, Instant>,
    /// Items emitted by a graph between UI frames. They are queued so a
    /// synchronous `start_workflow` call cannot lose the first local emit.
    workflow_pending_items: Vec<ProductionOutput>,
    /// 每次运行的原始终端日志路径（运行结束后仍可打开）。
    workflow_logs: HashMap<String, PathBuf>,
    workflow_cache: WorkflowCache,
    workflow_seq: u64,
    /// AI Done/Failed 落盘较重：超出本帧配额的事件排到下一帧，避免 67 路同时拷贝卡死 UI。
    ai_event_backlog: VecDeque<AiEvent>,
    defaults: HostDefaults,
    keys: HashMap<Provider, bool>,
    connection_test: Option<TaskKey>,
    connection_test_started: Option<Instant>,
    model_probe: Option<u64>,
    agent_tasks: HashMap<TaskKey, String>,
}

fn ai_settings(
    root: &Path,
) -> Result<
    (
        ServiceSettings,
        HostDefaults,
        CapabilityBindings,
        HashMap<Provider, bool>,
    ),
    String,
> {
    let config = AiConfig::load(root).map_err(|e| e.to_string())?;
    let store = todo_ai::secrets::default_store();
    let secrets = Secrets {
        store: store.as_ref(),
        environment: &todo_ai::secrets::process_environment,
    };
    let mut keys = HashMap::new();
    let mut present = HashMap::new();
    for provider in [Provider::Huniu, Provider::Volc] {
        let key = secrets.resolve(provider).map_err(|e| e.to_string())?;
        present.insert(provider, key.is_some());
        if let Some((key, _)) = key {
            keys.insert(provider, key);
        }
    }
    let (journal_dir, artifacts_root) = default_paths(root);
    let defaults = HostDefaults {
        audio_model: config.volc.audio_model.clone(),
        speaker: config.volc.speaker.clone(),
        sample_rate: 24_000,
    };
    // Independent chat origin (DeepSeek etc.): model lives on the endpoint.
    // Without a key the origin is ignored so we never send huniu model names
    // to a foreign API or foreign model names to huniu by accident.
    let language = match &config.language_base_url {
        Some(base) => secrets
            .language()
            .map_err(|error| error.to_string())?
            .map(|(key, _)| LanguageEndpoint {
                base_url: base.clone(),
                key,
                model: config
                    .language_model
                    .clone()
                    .filter(|m| !m.trim().is_empty())
                    .unwrap_or_else(|| "deepseek-flash".into()),
            }),
        None => None,
    };
    let language_model = language
        .as_ref()
        .map(|endpoint| endpoint.model.clone())
        .unwrap_or_else(|| config.huniu.language_model.clone());
    let bindings = CapabilityBindings {
        models: HashMap::from([
            (
                Capability::Language,
                ModelBinding {
                    model: language_model.clone(),
                    provider: Provider::Huniu,
                },
            ),
            (
                Capability::Image,
                ModelBinding {
                    model: config.huniu.image_model.clone(),
                    provider: Provider::Huniu,
                },
            ),
            (
                Capability::Video,
                ModelBinding {
                    model: config.huniu.video_model.clone(),
                    provider: Provider::Huniu,
                },
            ),
            (
                Capability::Multimodal,
                ModelBinding {
                    model: config.huniu.multimodal_model.clone(),
                    provider: Provider::Huniu,
                },
            ),
            (
                Capability::Audio,
                ModelBinding {
                    model: config.volc.audio_model.clone(),
                    provider: Provider::Volc,
                },
            ),
        ]),
    };
    Ok((
        ServiceSettings {
            huniu_base_url: config.huniu.base_url.clone(),
            language,
            volc_base_url: config.volc.base_url.clone(),
            huniu_strategy: config.huniu.strategy.clone(),
            keys,
            bindings: HashMap::from([
                (Capability::Language, Provider::Huniu),
                (Capability::Image, Provider::Huniu),
                (Capability::Video, Provider::Huniu),
                (Capability::Multimodal, Provider::Huniu),
                (Capability::Audio, Provider::Volc),
            ]),
            models: HashMap::from([
                (Capability::Language, language_model),
                (Capability::Image, config.huniu.image_model.clone()),
                (Capability::Video, config.huniu.video_model.clone()),
                (
                    Capability::Multimodal,
                    config.huniu.multimodal_model.clone(),
                ),
                (Capability::Audio, config.volc.audio_model.clone()),
            ]),
            journal_dir,
            artifacts_root,
            proxy: config.proxy,
            limits: config.limits.clone(),
            retry: todo_ai::net::RetryPolicy::default(),
            breaker: todo_ai::net::BreakerPolicy::default(),
        },
        defaults,
        bindings,
        present,
    ))
}

fn group_label(group: Group) -> &'static str {
    match group {
        Group::Story => "分段与分镜",
        Group::Voice => "配音",
        Group::Scene => "画面",
        Group::Timing => "节奏",
        Group::Stroke => "笔触与进场",
        Group::Text => "文字",
        Group::Output => "输出",
        Group::Other => "其他",
    }
}

fn param_view(param: &Param, quick: bool) -> ParamView {
    let (id, label, group, hint, when, input, default) = match param {
        Param::Select {
            id,
            label,
            group,
            default,
            options,
            hint,
            when,
        } => (
            id,
            label,
            *group,
            hint,
            when,
            ParamInput::Select(
                options
                    .iter()
                    .map(|o| ChoiceView {
                        value: o.value.clone(),
                        label: o.label.clone(),
                        hint: o.hint.clone(),
                    })
                    .collect(),
            ),
            ParamValue::Text(default.clone()),
        ),
        Param::Number {
            id,
            label,
            group,
            default,
            min,
            max,
            step,
            unit,
            hint,
            when,
        } => (
            id,
            label,
            *group,
            hint,
            when,
            ParamInput::Number {
                min: *min,
                max: *max,
                step: *step,
                unit: unit.clone(),
            },
            ParamValue::Number(*default),
        ),
        Param::Text {
            id,
            label,
            group,
            default,
            max_length,
            placeholder,
            hint,
            when,
            ..
        } => (
            id,
            label,
            *group,
            hint,
            when,
            ParamInput::Text {
                max_length: *max_length,
                placeholder: placeholder.clone(),
            },
            ParamValue::Text(default.clone()),
        ),
        Param::Bool {
            id,
            label,
            group,
            default,
            hint,
            when,
        } => (
            id,
            label,
            *group,
            hint,
            when,
            ParamInput::Bool,
            ParamValue::Bool(*default),
        ),
        Param::Voice {
            id,
            label,
            group,
            default,
            hint,
            when,
        } => (
            id,
            label,
            *group,
            hint,
            when,
            ParamInput::Voice(
                VOICES
                    .iter()
                    .map(|(value, label)| ChoiceView {
                        value: (*value).into(),
                        label: (*label).into(),
                        hint: None,
                    })
                    .collect(),
            ),
            ParamValue::Text(default.clone()),
        ),
    };
    ParamView {
        id: id.clone(),
        label: label.clone(),
        hint: hint.clone(),
        group: group_label(group).into(),
        quick,
        when: when.clone().unwrap_or_default(),
        input,
        default,
    }
}

fn param_label(param: &todo_workflow::workflow::manifest::Param) -> &str {
    use todo_workflow::workflow::manifest::Param;
    match param {
        Param::Select { label, .. }
        | Param::Number { label, .. }
        | Param::Text { label, .. }
        | Param::Bool { label, .. }
        | Param::Voice { label, .. } => label,
    }
}

fn param_json(value: &ParamValue) -> serde_json::Value {
    match value {
        ParamValue::Text(t) => serde_json::json!(t),
        ParamValue::Number(n) => serde_json::json!(n),
        ParamValue::Bool(b) => serde_json::json!(b),
    }
}

impl ProductionHost {
    pub fn start(root: impl Into<PathBuf>) -> Result<Self, String> {
        let root = root.into();
        let (settings, defaults, bindings, keys) = ai_settings(&root)?;
        let (ai, _recovered) = AiRouter::start(settings, bindings)?;
        let (worker_tx, worker_events) = mpsc::channel();
        let workers = WorkerPool::start(
            PoolOptions {
                runtime: RuntimePaths::new(root.clone()),
                ..PoolOptions::default()
            },
            worker_tx,
        );
        let registry = Registry::scan(
            &root.join("extensions/builtin"),
            &root.join("extensions/user"),
        );
        let (check_tx, checks) = mpsc::channel();
        for extension in registry.list().cloned() {
            let tx = check_tx.clone();
            let runtime = RuntimePaths::new(root.clone());
            std::thread::spawn(move || {
                let result = todo_workflow::check_extension(&extension, &runtime).map(|_| ());
                let _ = tx.send((extension.manifest.id.clone(), result));
            });
        }
        let store = ProductionStore::new(root.clone());
        // Saves run in order on one thread; a newer document of the same
        // production supersedes a queued older one.
        let (saver, queue) = mpsc::channel::<(String, Box<Production>)>();
        let (saved_tx, saved) = mpsc::channel();
        let save_store = ProductionStore::new(root.clone());
        std::thread::Builder::new()
            .name("todo-production-saver".into())
            .spawn(move || {
                while let Ok(first) = queue.recv() {
                    let mut latest: BTreeMap<(String, String), Box<Production>> = BTreeMap::new();
                    latest.insert((first.0.clone(), first.1.id.clone()), first.1);
                    while let Ok((project, doc)) = queue.try_recv() {
                        latest.insert((project, doc.id.clone()), doc);
                    }
                    for ((project, _), doc) in latest {
                        let result = save_store
                            .save(&project, &doc)
                            .or_else(|_| save_store.create(&project, &doc))
                            .map_err(|e| e.to_string());
                        let _ = saved_tx.send(result);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let bridge = ServiceBridge::new(defaults.clone(), &root, workers.handle());
        let (local_call_tx, local_call_rx) = mpsc::channel();
        let built: Result<Self, String> = Ok(Self {
            root,
            ai,
            bridge,
            workers,
            worker_events,
            registry,
            store,
            saver,
            saved,
            submissions: HashMap::new(),
            local_runs: HashMap::new(),
            pool_runs: HashMap::new(),
            budgets: HashMap::new(),
            checks,
            check_results: HashMap::new(),
            workflow_runs: HashMap::new(),
            workflow_live: HashMap::new(),
            models: Vec::new(),
            models_error: None,
            workflow_keys: HashMap::new(),
            workflow_results: HashMap::new(),
            local_call_tx,
            local_call_rx,
            placement_waits: HashMap::new(),
            workflow_pending_items: Vec::new(),
            workflow_logs: HashMap::new(),
            workflow_cache: WorkflowCache::default(),
            workflow_seq: 0,
            ai_event_backlog: VecDeque::new(),
            defaults,
            keys,
            connection_test: None,
            connection_test_started: None,
            model_probe: None,
            agent_tasks: HashMap::new(),
        });
        let mut host = built?;
        host.fetch_models_if_ready();
        Ok(host)
    }

    pub fn set_key(&mut self, provider: Provider, key: Option<Secret>) {
        self.keys.insert(provider, key.is_some());
        self.ai.set_key(provider, key);
        self.models.clear();
        self.models_error = None;
        self.fetch_models_if_ready();
    }

    /// Submit a fresh, uncached chat request used by the settings dialog.
    /// The nonce deliberately prevents a previous successful test from being
    /// answered from the durable production journal.
    /// 填过 Key 就自动拉一次模型目录，用户不需要再手填模型名。
    fn fetch_models_if_ready(&mut self) {
        if self.models.is_empty() {
            let _ = self.list_models();
        }
    }

    /// Read the gateway's model catalogue (`GET /v1/models`, free).
    pub fn list_models(&mut self) -> Result<(), String> {
        let provider = self
            .ai
            .binding(Capability::Language)
            .map(|binding| binding.provider)
            .unwrap_or(Provider::Huniu);
        if !self.keys.get(&provider).copied().unwrap_or(false) {
            return Err("还没有配置语言能力的 API Key".into());
        }
        if self.model_probe.is_none() {
            self.model_probe = Some(self.ai.list_models(Capability::Language));
            self.connection_test_started
                .get_or_insert_with(Instant::now);
        }
        Ok(())
    }

    /// Send one tiny chat request with the configured model and key. It is a
    /// real (paid) call, so it only runs when the user asks for it.
    pub fn test_connection(&mut self) -> Result<(), String> {
        let provider = self
            .ai
            .binding(Capability::Language)
            .map(|binding| binding.provider)
            .unwrap_or(Provider::Huniu);
        if !self.keys.get(&provider).copied().unwrap_or(false) {
            return Err("还没有配置语言能力的 API Key".into());
        }
        if self.connection_test.is_some() {
            return Err("正在测试，请稍候".into());
        }
        self.list_models()?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let spec = self
            .ai
            .prepare(TaskRequest::Language(todo_production::LanguageRequest {
                model: String::new(),
                messages: vec![
                    ChatMessage {
                        role: ChatRole::System,
                        content: "你正在进行 API 连接测试。只回复 OK。".into(),
                    },
                    ChatMessage {
                        role: ChatRole::User,
                        content: format!("连接测试 nonce={nonce}，请回复 OK。"),
                    },
                ],
                temperature: 0.0,
                json: false,
            }));
        let key = self.ai.submit_spec(spec.clone());
        self.submissions.insert(
            key.clone(),
            Submission {
                spec,
                capability: Capability::Language,
                workdirs: Vec::new(),
            },
        );
        self.connection_test = Some(key);
        self.connection_test_started = Some(Instant::now());
        Ok(())
    }

    /// Model id bound for language / tool-agent chat.
    pub fn language_model(&self) -> Option<String> {
        self.ai
            .binding(Capability::Language)
            .map(|binding| binding.model.clone())
            .filter(|model| !model.trim().is_empty())
            .or_else(|| {
                self.ai
                    .binding(Capability::Language)
                    .map(|_| "deepseek-flash".into())
            })
    }

    pub fn agent_task(&mut self, request_id: String, request: TaskRequest) -> Result<(), String> {
        let capability = request.capability();
        let provider = self
            .ai
            .binding(capability)
            .map(|binding| binding.provider)
            .unwrap_or(Provider::Huniu);
        if !self.keys.get(&provider).copied().unwrap_or(false) {
            return Err("还没有配置这项能力的 API Key".into());
        }
        if self.agent_tasks.values().any(|id| id == &request_id) {
            return Err("这条请求已经在处理".into());
        }
        let key = self.ai.submit(request);
        self.agent_tasks.insert(key, request_id);
        Ok(())
    }

    /// Installed styles for the director: what each one is, its parameters
    /// and, when the package has a `recipe.json`, its review thresholds.
    pub fn styles(&self) -> Vec<todo_application::director::StyleInfo> {
        self.registry
            .list()
            .filter(|extension| {
                matches!(self.check_results.get(&extension.manifest.id), Some(Ok(())))
            })
            .map(|extension| {
                let manifest = &extension.manifest;
                let rubric = extension
                    .read("recipe.json")
                    .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                    .and_then(|recipe| {
                        let mut rubric: todo_application::control::review::Rubric =
                            serde_json::from_value(recipe.get("review")?.clone()).ok()?;
                        if let Some(style) = recipe
                            .pointer("/text/subtitle/style")
                            .and_then(serde_json::Value::as_str)
                        {
                            rubric.subtitle_styles = vec![style.to_owned()];
                        }
                        Some(rubric)
                    });
                let params = manifest
                    .params
                    .iter()
                    // The redraw parameter is the director's, not the model's.
                    .filter(|param| {
                        rubric.as_ref().and_then(|r| r.redraw.as_deref()) != Some(param.id())
                    })
                    .map(|param| match param {
                        todo_workflow::workflow::manifest::Param::Select {
                            id,
                            label,
                            options,
                            ..
                        } => format!(
                            "{id}（{label}，可选 {}）",
                            options
                                .iter()
                                .map(|o| o.value.as_str())
                                .collect::<Vec<_>>()
                                .join("/")
                        ),
                        other => format!("{}（{}）", other.id(), param_label(other)),
                    })
                    .collect();
                let assets = extension
                    .read("assets/library/library.json")
                    .and_then(|text| serde_json::from_str::<Vec<serde_json::Value>>(&text).ok())
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|entry| {
                        let text = |key: &str| entry.get(key)?.as_str().map(str::to_owned);
                        let path = extension.dir.join("assets/library").join(text("file")?);
                        (text("kind")? != "plate" && path.is_file()).then(|| {
                            todo_application::director::StyleAsset {
                                id: text("id").unwrap_or_default(),
                                name: text("name").unwrap_or_default(),
                                desc: text("desc").unwrap_or_default(),
                                path: path.to_string_lossy().into_owned(),
                            }
                        })
                    })
                    .collect();
                let draw_style = extension
                    .read("recipe.json")
                    .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                    .and_then(|recipe| {
                        recipe
                            .pointer("/card/style")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                    });
                todo_application::director::StyleInfo {
                    id: manifest.id.clone(),
                    name: manifest.name.clone(),
                    description: manifest.description.clone(),
                    params,
                    rubric,
                    assets,
                    draw_style,
                }
            })
            .collect()
    }

    /// New service addresses or proxy: restart the AI service. Its journal
    /// keeps every task; in-flight ones resume and report as usual.
    pub fn reconfigure(&mut self) -> Result<(), String> {
        let (settings, defaults, bindings, keys) = ai_settings(&self.root)?;
        let (ai, _) = AiRouter::start(settings, bindings)?;
        self.ai = ai;
        self.defaults = defaults.clone();
        self.keys = keys;
        self.bridge = ServiceBridge::new(defaults, &self.root, self.workers.handle());
        // Requests the application had in flight are submitted again; the
        // journal answers finished ones without a new request.
        let specs: Vec<TaskSpec> = self.submissions.values().map(|s| s.spec.clone()).collect();
        for spec in specs {
            self.ai.submit_spec(spec);
        }
        Ok(())
    }

    fn extension(&self, id: &str) -> Result<Arc<Extension>, String> {
        self.registry
            .get(id)
            .ok_or_else(|| format!("找不到扩展 {id}"))
    }

    pub fn production_workdir(&self, project: &str, id: &str) -> Result<String, String> {
        self.workdir(project, id)
    }

    fn workdir(&self, project: &str, id: &str) -> Result<String, String> {
        let dir = self.store.workdir(project, id).map_err(|e| e.to_string())?;
        // The store may hand back a canonical path; compare like with like.
        let canonical = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let (dir, root) = (canonical(&dir), canonical(&self.root));
        let relative = dir
            .strip_prefix(&root)
            .map_err(|_| "成片目录不在软件目录内".to_string())?;
        Ok(relative.to_string_lossy().replace('\\', "/"))
    }

    /// Absolute production workdir for UI-only asset requests such as
    /// candidate thumbnails. Stored variant paths remain relative to this
    /// directory; callers must not persist the returned absolute path.
    pub fn workdir_path(&self, project: &str, id: &str) -> Result<PathBuf, String> {
        self.store.workdir(project, id).map_err(|e| e.to_string())
    }

    /// Join a repository-relative path onto the host root.
    pub fn root_join(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// Copy a user-selected image into a production workdir and return the
    /// repository-relative path stored by `Edit::UploadVariant`.
    pub fn copy_variant(
        &self,
        project: &str,
        production: &str,
        source: &Path,
    ) -> Result<String, String> {
        if !source.is_file() {
            return Err("源文件不存在".into());
        }
        let source_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "源文件名无效".to_string())?;
        let safe: String = source_name
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ".-_".contains(ch) {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        let safe = if safe.is_empty() {
            "upload.png".into()
        } else {
            safe
        };
        let workdir = self
            .store
            .workdir(project, production)
            .map_err(|error| error.to_string())?;
        let upload_dir = workdir.join("uploads");
        std::fs::create_dir_all(&upload_dir).map_err(|error| error.to_string())?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let target = upload_dir.join(format!("{stamp}-{safe}"));
        let partial = target.with_extension("part");
        std::fs::copy(source, &partial).map_err(|error| error.to_string())?;
        std::fs::rename(&partial, &target).map_err(|error| error.to_string())?;
        let root = self
            .root
            .canonicalize()
            .unwrap_or_else(|_| self.root.clone());
        let canonical = target.canonicalize().map_err(|error| error.to_string())?;
        canonical
            .strip_prefix(root)
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .map_err(|_| "上传文件不在软件目录内".into())
    }

    /// What the application needs to run a production of this package.
    pub fn setup(
        &self,
        project: &str,
        doc: &Production,
        canvas: Canvas,
    ) -> Result<ProductionSetup, String> {
        let extension = self.extension(&doc.pack.id)?;
        let manifest = &extension.manifest;
        let recipe = match manifest.kind {
            Kind::Narration => {
                let (spec, _, _) = narration_spec(
                    manifest,
                    &doc.params,
                    &|p| extension.read(p),
                    &self.defaults,
                )?;
                let narration = manifest.narration.as_ref().ok_or("缺少 narration 配置")?;
                Recipe::Narration {
                    spec: Box::new(spec),
                    bake: narration.bake.clone(),
                    plate: narration.plate.clone(),
                }
            }
            Kind::Workflow => Recipe::Workflow {
                op: manifest
                    .worker
                    .as_ref()
                    .and_then(|w| w.ops.first().cloned())
                    .ok_or("扩展没有工作程序")?,
                input: serde_json::json!({ "script": doc.script }),
            },
        };
        Ok(ProductionSetup {
            extension: manifest.id.clone(),
            recipe,
            canvas,
            workdir: self.workdir(project, &doc.id)?,
        })
    }

    /// A new production document, stored, and its setup.
    pub fn create(
        &self,
        project: &str,
        extension_id: &str,
        values: &BTreeMap<String, ParamValue>,
        script: &str,
        canvas: Canvas,
    ) -> Result<(Production, ProductionSetup), String> {
        let extension = self.extension(extension_id)?;
        let manifest = &extension.manifest;
        let given: BTreeMap<String, serde_json::Value> = values
            .iter()
            .map(|(k, v)| (k.clone(), param_json(v)))
            .collect();
        let params = manifest.resolve_params(&given).map_err(|p| p.to_string())?;
        let (split, limits) = match manifest.kind {
            Kind::Narration => {
                let (_, split, limits) =
                    narration_spec(manifest, &params, &|p| extension.read(p), &self.defaults)?;
                (split, limits)
            }
            Kind::Workflow => (
                SplitMode::Paragraph,
                Limits {
                    target: 120,
                    max: 200,
                },
            ),
        };
        if let Some(input) = &manifest.input.script {
            if script.trim().is_empty() && !input.optional {
                return Err(format!("请填写{}", input.label));
            }
            if script.chars().count() > input.max_length {
                return Err(format!("{}超过 {} 字", input.label, input.max_length));
            }
        }
        let id = format!("p{}", &uuid_like()[..12]);
        let doc = Production::new(
            id,
            PackRef {
                id: manifest.id.clone(),
                version: manifest.version.clone(),
            },
            script,
            params,
            split,
            limits,
        );
        self.store
            .create(project, &doc)
            .map_err(|e| e.to_string())?;
        let setup = self.setup(project, &doc, canvas)?;
        Ok((doc, setup))
    }

    /// Every stored production of a project, ready to resume.
    pub fn open_all(
        &self,
        project: &str,
        canvas: &Canvas,
    ) -> (Vec<(Production, ProductionSetup)>, Vec<String>) {
        let mut out = Vec::new();
        let mut problems = Vec::new();
        let list = match self.store.list(project) {
            Ok(list) => list,
            Err(e) => return (out, vec![e.to_string()]),
        };
        for summary in list {
            match self
                .store
                .load(project, &summary.id)
                .map_err(|e| e.to_string())
                .and_then(|doc| self.setup(project, &doc, canvas.clone()).map(|s| (doc, s)))
            {
                Ok(pair) => out.push(pair),
                Err(e) => problems.push(format!("成片 {}：{e}", summary.id)),
            }
        }
        (out, problems)
    }

    pub fn execute(&mut self, task: ProductionTask) -> Option<String> {
        match task {
            ProductionTask::Submit { workdir, spec } => self.submit(&workdir, spec),
            ProductionTask::Cancel(key) => self.ai.cancel(&key),
            ProductionTask::Retry(key) => self.ai.retry(&key),
            ProductionTask::Run {
                run,
                extension,
                op,
                input,
                params,
                canvas,
                workdir,
                seed,
            } => {
                let extension = match self.extension(&extension) {
                    Ok(e) => e,
                    Err(e) => return Some(e),
                };
                let workdir = self.root.join(&workdir);
                let budget = self
                    .budgets
                    .entry(workdir.clone())
                    .or_insert_with(|| Arc::new(Budget::new(&extension.manifest.budget)))
                    .clone();
                let pool_run = self.workers.run(RunRequest {
                    extension,
                    op,
                    input,
                    params,
                    canvas,
                    workdir: workdir.clone(),
                    seed,
                    budget,
                });
                self.bridge.register_run(pool_run, workdir);
                self.local_runs.insert(run, pool_run);
                self.pool_runs.insert(pool_run, run);
            }
            ProductionTask::CancelRun(run) => {
                if let Some(pool_run) = self.local_runs.remove(&run) {
                    self.pool_runs.remove(&pool_run);
                    self.workers.cancel(pool_run);
                    for key in self.bridge.remove_run(pool_run) {
                        // Paid work nobody waits for any more is cancelled
                        // unless a production asked for it too.
                        if !self.submissions.contains_key(&key) {
                            self.ai.cancel(&key);
                        }
                    }
                }
            }
            ProductionTask::Save { project, doc } => {
                let _ = self.saver.send((project, doc));
            }
        }
        None
    }

    fn submit(&mut self, workdir: &str, spec: TaskSpec) {
        let spec = self.ai.prepare(spec.request);
        let capability = spec.capability();
        let key = self.ai.submit_spec(spec.clone());
        let dir = self.root.join(workdir);
        let entry = self.submissions.entry(key).or_insert(Submission {
            spec,
            capability,
            workdirs: Vec::new(),
        });
        if !entry.workdirs.contains(&dir) {
            entry.workdirs.push(dir);
        }
    }

    /// The file and samples of a finished task, in every submitting workdir.
    fn finished(
        &mut self,
        key: &TaskKey,
        artifact: Option<&todo_ai::ArtifactRef>,
        output: Option<serde_json::Value>,
    ) -> Result<ProductionOutput, String> {
        let submission = self.submissions.remove(key).ok_or("未知任务")?;
        let mut file = None;
        let mut pcm = None;
        if let Some(artifact) = artifact {
            let relative = service_file(key, &artifact.ext);
            let source = self.ai.artifact_path(artifact);
            for dir in &submission.workdirs {
                materialize(&source, &dir.join(&relative))?;
            }
            if submission.capability == Capability::Audio {
                let bytes = std::fs::read(&source).map_err(|e| e.to_string())?;
                let (samples, rate) =
                    todo_production::voice::read_wav(&bytes).ok_or("配音不是 WAV")?;
                pcm = Some(Pcm16 { samples, rate });
            }
            file = Some(relative);
        }
        Ok(ProductionOutput::TaskDone {
            key: key.clone(),
            output,
            file,
            pcm,
        })
    }

    /// `project` is the open editor project — required for `timeline_strip`.
    /// `placement_inflight` / `placement_block` come from the workspace after
    /// the previous frame applied emits (so the gate sees real timeline state).
    pub fn poll(
        &mut self,
        project: Option<&Project>,
        placement_inflight: bool,
        placement_block: Option<String>,
    ) -> Vec<HostReport> {
        let mut reports = Vec::new();
        // Emits drained this frame are applied only after poll returns — keep
        // the placement gate closed until the next frame sees real timeline state.
        let emits_this_frame = !self.workflow_pending_items.is_empty();
        reports.extend(
            self.workflow_pending_items
                .drain(..)
                .map(HostReport::Output),
        );
        while let Ok((call_id, result)) = self.local_call_rx.try_recv() {
            self.workflow_results.insert(call_id, result);
        }
        self.settle_placement_waits(
            placement_inflight || emits_this_frame,
            placement_block.as_deref(),
        );
        if self
            .connection_test_started
            .is_some_and(|started| started.elapsed() >= std::time::Duration::from_secs(30))
        {
            if let Some(key) = self.connection_test.take() {
                self.submissions.remove(&key);
                self.ai.cancel(&key);
            }
            self.model_probe = None;
            self.connection_test_started = None;
            reports.push(HostReport::Connection {
                provider: Provider::Huniu,
                ok: false,
                message: "30 秒内没有响应，请检查网络或服务地址".into(),
            });
        }
        while let Ok((id, result)) = self.checks.try_recv() {
            self.check_results.insert(id, result);
        }
        while let Ok(result) = self.saved.try_recv() {
            if let Err(e) = result {
                reports.push(HostReport::Notice(format!("成片保存失败：{e}")));
            }
        }
        // 每帧最多处理若干「重」AI 完成事件（拷文件 + 量尺寸），其余留到下一帧。
        const HEAVY_AI_PER_FRAME: usize = 6;
        let mut heavy_done = 0usize;
        loop {
            let event = match self.ai_event_backlog.pop_front() {
                Some(event) => event,
                None => match self.ai.poll() {
                    Some(event) => event,
                    None => break,
                },
            };
            let heavy = matches!(
                &event,
                AiEvent::Done { .. } | AiEvent::Failed { .. } | AiEvent::Cancelled { .. }
            );
            if heavy && heavy_done >= HEAVY_AI_PER_FRAME {
                self.ai_event_backlog.push_front(event);
                break;
            }
            if heavy {
                heavy_done += 1;
            }
            let event_key = match &event {
                AiEvent::Generating { key }
                | AiEvent::Downloading { key }
                | AiEvent::Waiting { key, .. }
                | AiEvent::Done { key, .. }
                | AiEvent::Failed { key, .. }
                | AiEvent::Cancelled { key } => Some(key),
                AiEvent::Models { .. }
                | AiEvent::ModelsFailed { .. }
                | AiEvent::NeedsAttention { .. } => None,
            };
            // 工作流调用的生命周期（生成中/等待网络/下载中）写进原始终端
            if let Some(key) = event_key.cloned() {
                if let Some(call_id) = self.bridge.call_id_for(&key) {
                    // 每次发送（含重试）在这里计数，避免刷屏也看得见重试次数
                    let mut attempt_note = None;
                    if matches!(&event, AiEvent::Generating { .. }) {
                        let prefix = self
                            .workflow_live
                            .iter()
                            .find(|(_, run)| call_id.starts_with(&format!("wf-{}-", run.run_id)))
                            .map(|(extension, _)| extension.clone());
                        if let Some(extension) = prefix {
                            if let Some(run) = self.workflow_live.get_mut(&extension) {
                                let attempts =
                                    run.attempts.entry(call_id.clone()).or_insert(0);
                                *attempts += 1;
                                let attempts = *attempts;
                                // 按 call_id：前 3 次逐条，之后每 5 次一条；超过 12 次才放弃该槽
                                if attempts > 12 {
                                    let message = format!(
                                        "模型连续 {attempts} 次请求失败（网络/网关不稳定），已放弃"
                                    );
                                    self.append_workflow_log(
                                        &call_id,
                                        &format!(
                                            "✗ 已连续发送 {attempts} 次仍未成功，放弃本次调用"
                                        ),
                                    );
                                    // Chat/image go through bridge.direct — not workflow_results.
                                    self.ai.cancel(&key);
                                    self.bridge.fail_direct(&call_id, message.clone());
                                    self.workflow_results
                                        .insert(call_id.clone(), Err(message));
                                    let _ = extension;
                                    continue;
                                }
                                if attempts <= 3 || attempts % 5 == 0 {
                                    attempt_note =
                                        Some(format!("· 第 {attempts} 次发送，等待模型响应"));
                                }
                            }
                        }
                    }
                    let waiting_note = match &event {
                        AiEvent::Waiting { reason, .. } => Some(match reason {
                            todo_ai::WaitReason::Network => "· 等待网络重试".to_owned(),
                            todo_ai::WaitReason::Retrying { attempt, after, message } =>
                                format!("· 请求失败：{message}；{:.1}s 后进行第 {attempt} 次尝试", after.as_secs_f32()),
                            todo_ai::WaitReason::Attention => "· 需要人工处理（额度/鉴权）".to_owned(),
                            todo_ai::WaitReason::Elsewhere => "· 另一个进程正在处理同一请求".to_owned(),
                        }),
                        _ => None,
                    };
                    let line = match &event {
                        AiEvent::Generating { .. } => attempt_note.as_deref(),
                        AiEvent::Downloading { .. } => Some("· 下载产物中"),
                        AiEvent::Waiting { .. } => waiting_note.as_deref(),
                        _ => None,
                    };
                    if let Some(line) = line {
                        self.append_workflow_log(&call_id, line);
                    }
                }
            }
            // 工作流里的多模态调用：先把结果交回 job，不再往下分发。
            if let Some(key) = event_key.cloned() {
                if let Some(call_id) = self.workflow_keys.remove(&key) {
                    let result = match &event {
                        AiEvent::Done { output, .. } => Ok(output
                            .as_ref()
                            .and_then(|value| value.get("text"))
                            .and_then(serde_json::Value::as_str)
                            .map(|text| serde_json::json!({ "text": text }))
                            .unwrap_or_else(|| serde_json::json!({ "text": "" }))),
                        AiEvent::Failed { message, .. } => Err(message.clone()),
                        AiEvent::Cancelled { .. } => Err("已取消".into()),
                        _ => continue,
                    };
                    self.workflow_results.insert(call_id, result);
                    continue;
                }
            }
            if matches!(&event, AiEvent::Models { id, .. } if self.model_probe == Some(*id)) {
                if let AiEvent::Models {
                    provider, models, ..
                } = event
                {
                    self.model_probe = None;
                    if self.connection_test.is_none() {
                        self.connection_test_started = None;
                    }
                    self.models = models.clone();
                    self.models_error = None;
                    reports.push(HostReport::Models { provider, models });
                }
                continue;
            }
            if matches!(&event, AiEvent::ModelsFailed { id, .. } if self.model_probe == Some(*id)) {
                if let AiEvent::ModelsFailed {
                    provider,
                    class,
                    message,
                    ..
                } = event
                {
                    self.model_probe = None;
                    if self.connection_test.is_none() {
                        self.connection_test_started = None;
                    }
                    self.models_error = Some(message.clone());
                    reports.push(HostReport::ModelsFailed {
                        provider,
                        class,
                        message,
                    });
                }
                continue;
            }
            if self
                .connection_test
                .as_ref()
                .is_some_and(|key| event_key == Some(key))
            {
                match event {
                    AiEvent::Done { key, .. } => {
                        self.connection_test = None;
                        if self.model_probe.is_none() {
                            self.connection_test_started = None;
                        }
                        self.submissions.remove(&key);
                        reports.push(HostReport::Connection {
                            provider: Provider::Huniu,
                            ok: true,
                            message: format!(
                                "连接正常，对话模型 {} 可用",
                                self.ai
                                    .binding(Capability::Language)
                                    .map(|binding| binding.model.as_str())
                                    .unwrap_or("未绑定")
                            ),
                        });
                    }
                    AiEvent::Failed {
                        key,
                        class,
                        message,
                        ..
                    } => {
                        self.connection_test = None;
                        if self.model_probe.is_none() {
                            self.connection_test_started = None;
                        }
                        self.submissions.remove(&key);
                        reports.push(HostReport::Connection {
                            provider: Provider::Huniu,
                            ok: false,
                            message: class_message(&class, &message),
                        });
                    }
                    AiEvent::Cancelled { key } => {
                        self.connection_test = None;
                        if self.model_probe.is_none() {
                            self.connection_test_started = None;
                        }
                        self.submissions.remove(&key);
                        reports.push(HostReport::Connection {
                            provider: Provider::Huniu,
                            ok: false,
                            message: "测试已取消".into(),
                        });
                    }
                    // Progress and retry states stay quiet; the final result
                    // below is the useful feedback for a settings dialog.
                    AiEvent::Generating { .. }
                    | AiEvent::Downloading { .. }
                    | AiEvent::Waiting { .. } => {}
                    AiEvent::Models { .. } | AiEvent::ModelsFailed { .. } => {}
                    AiEvent::NeedsAttention { .. } => unreachable!(),
                }
                continue;
            }
            if self.connection_test.is_some()
                && matches!(
                    &event,
                    AiEvent::NeedsAttention {
                        provider: Provider::Huniu,
                        ..
                    }
                )
            {
                let key = self.connection_test.take().unwrap();
                self.model_probe = None;
                self.connection_test_started = None;
                self.submissions.remove(&key);
                self.ai.cancel(&key);
                if let AiEvent::NeedsAttention { attention, .. } = event {
                    reports.push(HostReport::Notice(format!(
                        "虎妞连接失败：{}",
                        match attention {
                            todo_ai::Attention::MissingKey => "还没有配置 API Key",
                            todo_ai::Attention::Auth => "API Key 无效或已过期",
                            todo_ai::Attention::Billing => "账户余额不足",
                        }
                    )));
                }
                continue;
            }
            if let Some(request_id) = event_key.and_then(|key| self.agent_tasks.get(key).cloned()) {
                match event {
                    AiEvent::Done {
                        key,
                        output,
                        artifact,
                        ..
                    } => {
                        self.agent_tasks.remove(&key);
                        // Pictures answer with their file, text requests with text.
                        let text = match artifact {
                            Some(artifact) => self
                                .ai
                                .artifact_path(&artifact)
                                .to_string_lossy()
                                .into_owned(),
                            None => output
                                .and_then(|value| {
                                    value
                                        .get("text")
                                        .and_then(serde_json::Value::as_str)
                                        .map(str::to_owned)
                                })
                                .unwrap_or_default(),
                        };
                        reports.push(HostReport::AgentReply {
                            request_id,
                            result: Ok(text),
                        });
                    }
                    AiEvent::Failed {
                        key,
                        class,
                        message,
                        ..
                    } => {
                        self.agent_tasks.remove(&key);
                        reports.push(HostReport::AgentReply {
                            request_id,
                            result: Err(class_message(&class, &message)),
                        });
                    }
                    AiEvent::Cancelled { key } => {
                        self.agent_tasks.remove(&key);
                        reports.push(HostReport::AgentReply {
                            request_id,
                            result: Err("Agent 请求已取消".into()),
                        });
                    }
                    AiEvent::Generating { .. }
                    | AiEvent::Downloading { .. }
                    | AiEvent::Waiting { .. } => {}
                    AiEvent::Models { .. } | AiEvent::ModelsFailed { .. } => {}
                    AiEvent::NeedsAttention { attention, .. } => {
                        reports.push(HostReport::AgentReply {
                            request_id,
                            result: Err(format!("Agent 服务需要处理：{attention:?}")),
                        });
                    }
                }
                continue;
            }
            self.bridge.on_ai_event(&self.ai, &event);
            let output = match event {
                AiEvent::Generating { key } => Some(ProductionOutput::TaskState {
                    key,
                    state: TaskState::Generating,
                }),
                AiEvent::Downloading { key } => Some(ProductionOutput::TaskState {
                    key,
                    state: TaskState::Downloading,
                }),
                AiEvent::Waiting { key, reason } => Some(ProductionOutput::TaskState {
                    key,
                    state: TaskState::Waiting(match reason {
                        todo_ai::WaitReason::Network => "等待网络恢复".into(),
                        todo_ai::WaitReason::Retrying {
                            attempt,
                            after,
                            message,
                        } => {
                            format!("第 {attempt} 次重试，{} 秒后（{message}）", after.as_secs())
                        }
                        todo_ai::WaitReason::Attention => "等待处理 key 或余额".into(),
                        todo_ai::WaitReason::Elsewhere => "另一个程序正在生成，等它的结果".into(),
                    }),
                }),
                AiEvent::Done {
                    key,
                    artifact,
                    output,
                } => {
                    if !self.submissions.contains_key(&key) {
                        None
                    } else {
                        match self.finished(&key, artifact.as_ref(), output) {
                            Ok(output) => Some(output),
                            Err(message) => Some(ProductionOutput::TaskFailed {
                                key,
                                class: "storage".into(),
                                message,
                                possibly_billed: false,
                            }),
                        }
                    }
                }
                AiEvent::Failed {
                    key,
                    class,
                    message,
                    possibly_billed,
                } => {
                    self.submissions.remove(&key);
                    Some(ProductionOutput::TaskFailed {
                        key,
                        class,
                        message,
                        possibly_billed,
                    })
                }
                AiEvent::Cancelled { key } => {
                    self.submissions.remove(&key);
                    None
                }
                AiEvent::Models { .. } | AiEvent::ModelsFailed { .. } => None,
                AiEvent::NeedsAttention {
                    provider,
                    attention,
                } => Some(ProductionOutput::Attention {
                    provider: match provider {
                        Provider::Huniu => "虎妞".into(),
                        Provider::Volc => "火山".into(),
                    },
                    message: match attention {
                        todo_ai::Attention::MissingKey => "还没有填写 key，请到 AI 设置填写".into(),
                        todo_ai::Attention::Auth => "key 无效或已过期，请到 AI 设置更换".into(),
                        todo_ai::Attention::Billing => "余额不足，充值后会自动继续".into(),
                    },
                }),
            };
            if let Some(output) = output {
                reports.push(HostReport::Output(output));
            }
        }
        while let Ok(event) = self.worker_events.try_recv() {
            let pool_run = match &event {
                WorkerEvent::Progress { run, .. }
                | WorkerEvent::Items { run, .. }
                | WorkerEvent::Call { run, .. }
                | WorkerEvent::Done { run, .. }
                | WorkerEvent::Failed { run, .. } => *run,
            };
            if let WorkerEvent::Call { run, call, service } = event {
                self.bridge.call(&self.ai, run, call, service);
                continue;
            }
            let Some(&run) = self.pool_runs.get(&pool_run) else {
                continue;
            };
            let output = match event {
                WorkerEvent::Progress {
                    phase, fraction, ..
                } => ProductionOutput::RunProgress {
                    run,
                    phase,
                    fraction,
                },
                WorkerEvent::Items { items, .. } => ProductionOutput::RunItems { run, items },
                WorkerEvent::Done { result, .. } => {
                    self.finish_run(run, pool_run);
                    ProductionOutput::RunDone { run, result }
                }
                WorkerEvent::Failed { error, stderr, .. } => {
                    self.finish_run(run, pool_run);
                    let detail = stderr.lines().last().unwrap_or_default().to_owned();
                    ProductionOutput::RunFailed {
                        run,
                        class: format!("{:?}", error.class),
                        message: if detail.is_empty() {
                            error.message
                        } else {
                            format!("{}（{detail}）", error.message)
                        },
                    }
                }
                WorkerEvent::Call { .. } => continue,
            };
            reports.push(HostReport::Output(output));
        }
        self.pump_workflows(project);
        // Emits queued during this pump land next frame; do not release the
        // gate in the same poll or strip would race empty timeline.
        reports
    }

    /// Complete parked `placement_ready` calls once the timeline is quiet.
    fn settle_placement_waits(&mut self, inflight: bool, block: Option<&str>) {
        if self.placement_waits.is_empty() {
            return;
        }
        if let Some(message) = block {
            let ids: Vec<String> = self.placement_waits.keys().cloned().collect();
            for id in ids {
                self.placement_waits.remove(&id);
                self.workflow_results
                    .insert(id, Err(message.to_owned()));
            }
            return;
        }
        if inflight || !self.workflow_pending_items.is_empty() {
            // Still adopting / place pending / emits not yet handed to workspace.
            let timed_out: Vec<String> = self
                .placement_waits
                .iter()
                .filter(|(_, started)| started.elapsed().as_secs() > 600)
                .map(|(id, _)| id.clone())
                .collect();
            for id in timed_out {
                self.placement_waits.remove(&id);
                self.workflow_results.insert(
                    id,
                    Err("等待入轨超时（10 分钟）；请检查素材路径或点重试".into()),
                );
            }
            return;
        }
        let ids: Vec<String> = self.placement_waits.keys().cloned().collect();
        for id in ids {
            self.placement_waits.remove(&id);
            self.append_workflow_log(&id, "· 入轨已落地，放行");
            self.workflow_results.insert(
                id,
                Ok(serde_json::json!({ "trigger": true })),
            );
        }
    }

    fn finish_run(&mut self, run: LocalRunId, pool_run: RunId) {
        self.local_runs.remove(&run);
        self.pool_runs.remove(&pool_run);
        self.bridge.remove_run(pool_run);
    }

    pub fn extension_name(&self, id: &str) -> Option<String> {
        self.registry.get(id).map(|e| e.manifest.name.clone())
    }

    pub fn extension_version(&self, id: &str) -> Option<String> {
        self.registry.get(id).map(|e| e.manifest.version.clone())
    }

    pub fn registry_ref(&self) -> &Registry {
        &self.registry
    }

    pub fn workflow_is_live(&self, extension_id: &str) -> bool {
        self.workflow_live.contains_key(extension_id)
    }

    pub fn live_workflow_extension(&self) -> Option<String> {
        self.workflow_live.keys().next().cloned()
    }

    /// Summaries for Agent Bridge `list_extensions` (same registry as UI cards).
    pub fn extension_summaries(&self) -> Vec<todo_application::control::ExtensionSummary> {
        let mut out: Vec<_> = self
            .registry
            .list()
            .map(|extension| {
                let status = match self.check_results.get(&extension.manifest.id) {
                    None => "checking",
                    Some(Err(e)) if e.contains("Python") || e.contains("python") => {
                        "runtime_missing"
                    }
                    Some(Err(_)) => "failed",
                    Some(Ok(())) => "ready",
                };
                todo_application::control::ExtensionSummary {
                    id: extension.manifest.id.clone(),
                    name: extension.manifest.name.clone(),
                    version: extension.manifest.version.clone(),
                    origin: match extension.origin {
                        Origin::Builtin => "builtin",
                        Origin::User => "user",
                    }
                    .into(),
                    has_graph: extension.manifest.graph.is_some(),
                    description: extension.manifest.description.clone(),
                    status: status.into(),
                }
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Allocate a repository-relative workdir for a canvas workflow run.
    pub fn alloc_workflow_workdir(&self, extension_id: &str) -> Result<(PathBuf, String), String> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let abs = self
            .root
            .join("cache/workflow-runs")
            .join(format!(
                "{}-{stamp}",
                extension_id.replace(['.', '/', '\\'], "-")
            ));
        std::fs::create_dir_all(&abs).map_err(|e| e.to_string())?;
        let relative = abs
            .strip_prefix(&self.root)
            .map_err(|_| "工作目录不在软件目录内".to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        Ok((abs, relative))
    }

    /// Start a workflow and drive it from `poll` (no blocking, cancels cheaply).
    /// `run_id` must already be registered on a production session via
    /// `Workspace::bind_canvas_workflow`.
    pub fn start_workflow(
        &mut self,
        extension_id: &str,
        script: String,
        edits: &BTreeMap<String, BTreeMap<String, String>>,
        run_id: LocalRunId,
        workdir: PathBuf,
        project: Option<&Project>,
    ) -> Result<(), String> {
        let extension = self
            .registry
            .get(extension_id)
            .ok_or_else(|| format!("没有扩展 {extension_id}"))?;
        let mut workflow = extension
            .workflow()
            .map_err(|problems| {
                problems
                    .iter()
                    .map(|p| format!("{}: {}", p.path, p.message))
                    .collect::<Vec<_>>()
                    .join("; ")
            })?
            .ok_or_else(|| format!("{extension_id} 没有 graph.json"))?;
        apply_workflow_edits(&mut workflow.graph, edits);
        std::fs::create_dir_all(&workdir).ok();
        self.bridge.register_run(run_id as RunId, workdir.clone());
        let log_path = self.ensure_workflow_log(extension_id)?;
        {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                let _ = writeln!(file, "\n[  0.0s] ▶ 工作流开始");
                let _ = file.flush();
            }
        }
        let extension_dir = extension.dir.clone();
        let mut job = todo_workflow::runner::WorkflowJob::new(
            &workflow.graph,
            &extension_dir,
            todo_workflow::runner::RunInput {
                script,
                at: Some(0.0),
            },
        );
        job.set_workdir(workdir.clone());
        self.workflow_live.insert(
            extension_id.to_owned(),
            WorkflowLive {
                job,
                calls: HashMap::new(),
                run_id: run_id as RunId,
                workdir,
                extension_dir,
                log_path,
                logged: 0,
                started: Instant::now(),
                last_beat: Instant::now(),
                attempts: HashMap::new(),
            },
        );
        self.pump_workflows(project);
        self.flush_workflow_log(extension_id);
        // 秒结束时把原因抛给 UI（终端里也会有 ✗ 行）。
        if !self.workflow_live.contains_key(extension_id) {
            if let Some(err) = self
                .workflow_runs
                .get(extension_id)
                .and_then(|report| report.error.clone())
            {
                return Err(err);
            }
        }
        Ok(())
    }

    /// 往对应运行的原始终端里补一行（事件/重试/看门狗）。
    fn append_workflow_log(&mut self, call_id: &str, line: &str) {
        let Some(extension) = self
            .workflow_live
            .iter()
            .find(|(_, run)| call_id.starts_with(&format!("wf-{}-", run.run_id)))
            .map(|(extension, _)| extension.clone())
        else {
            return;
        };
        let Some(run) = self.workflow_live.get(&extension) else {
            return;
        };
        let elapsed = run.started.elapsed().as_secs_f32();
        let path = run.log_path.clone();
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "[{elapsed:>7.1}s] {line}");
            let _ = file.flush();
        }
    }

    /// 把新产生的节点日志追加到 run.log（真实文件，可被外部 tail）。
    fn flush_workflow_log(&mut self, extension: &str) {
        let Some(run) = self.workflow_live.get_mut(extension) else {
            return;
        };
        Self::flush_live_log(run);
    }

    fn flush_live_log(run: &mut WorkflowLive) {
        let logs: Vec<String> = run
            .job
            .report()
            .nodes
            .iter()
            .flat_map(|node| node.logs.iter().cloned())
            .chain(run.job.current_logs().iter().cloned())
            .collect();
        if logs.len() <= run.logged {
            return;
        }
        let elapsed = run.started.elapsed().as_secs_f32();
        let fresh = logs[run.logged..]
            .iter()
            .map(|line| format!("[{elapsed:>7.1}s] {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        run.logged = logs.len();
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&run.log_path)
        {
            let _ = writeln!(file, "{fresh}");
            let _ = file.flush();
        }
    }

    /// Path of the latest `run.log` for an extension (live or finished).
    pub fn workflow_log_path(&self, extension: &str) -> Option<PathBuf> {
        self.workflow_live
            .get(extension)
            .map(|run| run.log_path.clone())
            .or_else(|| self.workflow_logs.get(extension).cloned())
    }

    /// Ensure `run.log` exists so the embedded console can attach before/without a run.
    pub fn ensure_workflow_log(&mut self, extension: &str) -> Result<PathBuf, String> {
        if let Some(path) = self.workflow_log_path(extension) {
            if path.is_file() {
                return Ok(path);
            }
        }
        let dir = self
            .root
            .join("cache/workflow-runs")
            .join(format!("{}-idle", extension.replace(['.', '/', '\\'], "-")));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("run.log");
        if !path.is_file() {
            std::fs::write(
                &path,
                format!("# 工作流终端 · {extension}\n# 运行后这里会实时追加节点日志\n"),
            )
            .map_err(|e| e.to_string())?;
        }
        self.workflow_logs
            .insert(extension.to_owned(), path.clone());
        Ok(path)
    }

    /// 停止一条正在跑的工作流：取消在途请求，保留已完成节点。
    pub fn stop_workflow(&mut self, extension: &str) -> Result<(), String> {
        let Some(run) = self.workflow_live.remove(extension) else {
            return Err("没有正在运行的工作流".into());
        };
        let prefix = format!("wf-{}-", run.run_id);
        let mut keys = self.bridge.keys_for_calls(&prefix);
        keys.extend(
            self.workflow_keys
                .iter()
                .filter(|(_, call_id)| call_id.starts_with(&prefix))
                .map(|(key, _)| key.clone()),
        );
        for key in &keys {
            self.ai.cancel(key);
        }
        self.workflow_keys.retain(|_, call_id| !call_id.starts_with(&prefix));
        self.placement_waits
            .retain(|call_id, _| !call_id.starts_with(&prefix));
        let mut report = run.job.report().clone();
        for waiting in run.job.waiting_nodes() {
            report.nodes.push(todo_workflow::runner::NodeRun {
                id: waiting.to_owned(),
                node_type: String::new(),
                ok: false,
                message: "已停止".into(),
                outputs: Vec::new(),
                preview: None,
                image: None,
                segments: None,
                files: None,
                duration_ms: run
                    .job
                    .current_elapsed()
                    .map(|elapsed| elapsed.as_millis() as u64)
                    .unwrap_or(0),
                output_chars: None,
                logs: vec!["■ 用户停止".to_owned()],
            });
        }
        report.error = Some("已停止".into());
        self.workflow_pending_items
            .push(ProductionOutput::RunFailed {
                run: run.run_id as LocalRunId,
                class: "cancelled".into(),
                message: "已停止".into(),
            });
        self.workflow_runs.insert(extension.to_owned(), report);
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&run.log_path)
        {
            let _ = writeln!(
                file,
                "[{:>7.1}s] ■ 用户停止",
                run.started.elapsed().as_secs_f32()
            );
        }
        Ok(())
    }

    /// One frame of every running workflow: collect settled calls, advance until
    /// the next service call, submit it, stop.
    fn queue_workflow_emits(
        &mut self,
        run: LocalRunId,
        job: &mut todo_workflow::runner::WorkflowJob,
    ) {
        let mut emitted = Vec::new();
        job.drain_emitted(&mut |value| emitted.push(value));
        for value in emitted {
            match decode_workflow_items(value) {
                Ok(items) if !items.is_empty() => {
                    self.workflow_pending_items
                        .push(ProductionOutput::RunItems { run, items });
                }
                Ok(_) => {}
                Err(message) => {
                    // Placement failed: surface in the terminal AND fail the run
                    // so the UI cannot show「已结束」as success.
                    self.append_workflow_log(&format!("wf-{run}-"), &format!("✗ 入轨失败：{message}"));
                    job.note_host_error(format!("入轨失败：{message}"));
                }
            }
        }
    }

    fn pump_workflows(&mut self, project: Option<&Project>) {
        if self.workflow_live.is_empty() {
            return;
        }
        let mut live = std::mem::take(&mut self.workflow_live);
        let mut finished = Vec::new();
        for (extension, run) in live.iter_mut() {
            if run.started.elapsed().as_secs() > 1800 {
                let nodes: Vec<String> = run.job.waiting_nodes().iter().map(|s| (*s).to_owned()).collect();
                for node in nodes {
                    run.job.resume(&node, Err("工作流运行超时（30 分钟）".into()));
                }
                finished.push(extension.clone());
                continue;
            }
            // Settle completed calls — cap per frame so 67 路不会在一帧里全部 resume。
            const MAX_SETTLE_PER_FRAME: usize = 8;
            let inflight: Vec<(String, String, bool, Instant)> = run
                .calls
                .iter()
                .map(|(node, (call_id, via, started))| {
                    (node.clone(), call_id.clone(), *via, *started)
                })
                .collect();
            let mut settled_any = false;
            let mut settled_n = 0usize;
            let mut oldest_wait = 0u64;
            for (node, call_id, via_router, started) in inflight {
                if settled_n >= MAX_SETTLE_PER_FRAME {
                    break;
                }
                let result = if via_router {
                    self.workflow_results.remove(&call_id)
                } else {
                    self.bridge.take_direct(&call_id)
                };
                match result {
                    Some(result) => {
                        run.calls.remove(&node);
                        run.attempts.remove(&call_id);
                        run.job.resume(&node, result);
                        settled_any = true;
                        settled_n += 1;
                    }
                    None if started.elapsed().as_secs() > 300 => {
                        run.calls.remove(&node);
                        run.attempts.remove(&call_id);
                        run.job.resume(&node, Err("服务调用超时".into()));
                        settled_any = true;
                        settled_n += 1;
                    }
                    None => {
                        oldest_wait = oldest_wait.max(started.elapsed().as_secs());
                    }
                }
            }
            if settled_any {
                self.queue_workflow_emits(run.run_id, &mut run.job);
                Self::flush_live_log(run);
            } else if !run.calls.is_empty()
                && run.last_beat.elapsed().as_secs() >= 10
                && oldest_wait > 0
            {
                run.last_beat = Instant::now();
                let path = run.log_path.clone();
                use std::io::Write;
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let n = run.calls.len();
                    let _ = writeln!(
                        file,
                        "[{:>7.1}s] … {n} 路并行等待中（最长 {oldest_wait}s）",
                        run.started.elapsed().as_secs_f32()
                    );
                }
            }
            // Start every newly ready branch (possibly several Calls at once).
            loop {
                // Keep local relay/mapping chains cooperative with the UI.
                match run.job.advance_budgeted(8) {
                    todo_workflow::runner::Step::Waiting => {
                        self.queue_workflow_emits(run.run_id, &mut run.job);
                        break;
                    }
                    todo_workflow::runner::Step::Finished => {
                        self.queue_workflow_emits(run.run_id, &mut run.job);
                        finished.push(extension.clone());
                        break;
                    }
                    todo_workflow::runner::Step::Calls(batch) => {
                        self.queue_workflow_emits(run.run_id, &mut run.job);
                        for (node, call) in batch {
                            self.workflow_seq += 1;
                            let call_id =
                                format!("wf-{}-{}-{}", run.run_id, node, self.workflow_seq);
                            let via =
                                self.dispatch_workflow_call(run, call, &call_id, project);
                            run.calls.insert(node, (call_id, via, Instant::now()));
                        }
                        Self::flush_live_log(run);
                    }
                }
            }
        }
        for extension in &finished {
            if let Some(run) = live.get_mut(extension) {
                Self::flush_live_log(run);
                use std::io::Write;
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&run.log_path)
                {
                    let elapsed = run.started.elapsed().as_secs_f32();
                    if let Some(error) = run.job.report().error.as_deref() {
                        let _ = writeln!(file, "[{elapsed:>7.1}s] ✗ {error}");
                        let _ = writeln!(file, "[{elapsed:>7.1}s] ■ 工作流失败结束");
                    } else if run.job.report().nodes.iter().any(|n| !n.ok) {
                        let _ = writeln!(file, "[{elapsed:>7.1}s] ■ 工作流失败结束");
                    } else if run.job.report().nodes.is_empty() {
                        let _ = writeln!(
                            file,
                            "[{elapsed:>7.1}s] · 没有可执行节点（图为空，或只剩未接线的孤立节点）"
                        );
                        let _ = writeln!(file, "[{elapsed:>7.1}s] ■ 工作流结束");
                    } else {
                        let _ = writeln!(file, "[{elapsed:>7.1}s] ■ 工作流结束");
                    }
                    let _ = file.flush();
                }
            }
        }
        for extension in finished {
            if let Some(run) = live.remove(&extension) {
                let mut report = run.job.report().clone();
                // Node-level failures must make the run unsuccessful even if
                // report.error was never stamped (e.g. parallel branch fail).
                if report.error.is_none() {
                    if let Some(failed) = report.nodes.iter().find(|n| !n.ok) {
                        report.error = Some(format!("{}: {}", failed.id, failed.message));
                    }
                }
                for node in &mut report.nodes {
                    absolutize_node_media(node, &run.workdir);
                }
                let _ = self.persist_relay_blocks(&extension, &report);
                let ok = report.error.is_none();
                let result = serde_json::json!({
                    "ok": ok,
                    "error": report.error,
                });
                self.workflow_pending_items
                    .push(ProductionOutput::RunDone {
                        run: run.run_id as LocalRunId,
                        result,
                    });
                self.workflow_runs.insert(extension, report);
            }
        }
        self.workflow_live = live;
        let running: Vec<String> = self.workflow_live.keys().cloned().collect();
        for extension in running {
            self.flush_workflow_log(&extension);
        }
    }

    /// `true` when the call settles via `workflow_results` (AI router or local
    /// thread), `false` for the bridge.
    fn dispatch_workflow_call(
        &mut self,
        run: &WorkflowLive,
        call: todo_workflow::runner::RequestedCall,
        call_id: &str,
        project: Option<&Project>,
    ) -> bool {
        use todo_workflow::workflow::protocol::ServiceCall;
        use todo_workflow::runner::RequestedCall;
        match call {
            RequestedCall::Chat(call) => {
                let model = call
                    .model
                    .clone()
                    .or_else(|| {
                        self.ai
                            .binding(Capability::Language)
                            .map(|binding| binding.model.clone())
                    })
                    .unwrap_or_default();
                self.append_workflow_log(
                    call_id,
                    &format!(
                        "· 发往 {model}（json={}，{} 条消息）",
                        call.json,
                        call.messages.len()
                    ),
                );
                self.bridge
                    .call_async(&self.ai, run.run_id, call_id.to_owned(), ServiceCall::Chat(call));
                false
            }
            RequestedCall::Image(mut call) => {
                // media_bin / relay 存包内相对路径；桥接按 run workdir 解析，先把缺的参考图拷进来。
                for reference in &mut call.references {
                    stage_workflow_reference(run, reference);
                }
                let model = call
                    .model
                    .clone()
                    .or_else(|| self.ai.binding(Capability::Image).map(|b| b.model.clone()))
                    .unwrap_or_default();
                self.append_workflow_log(
                    call_id,
                    &format!("· 发往 {model}（尺寸 {}）", call.size),
                );
                self.bridge
                    .call_async(&self.ai, run.run_id, call_id.to_owned(), ServiceCall::Image(call));
                false
            }
            RequestedCall::Speech(call) => {
                let model = self
                    .ai
                    .binding(Capability::Audio)
                    .map(|b| b.model.clone())
                    .unwrap_or_default();
                self.append_workflow_log(
                    call_id,
                    &format!("· 发往 {model}（音色 {}，{} 字）", call.voice, call.text.chars().count()),
                );
                self.bridge
                    .call_async(&self.ai, run.run_id, call_id.to_owned(), ServiceCall::Speech(call));
                false
            }
            RequestedCall::Subtitles(call) => {
                self.append_workflow_log(
                    call_id,
                    &format!("· 本地字幕对齐（{} 字）", call.script.chars().count()),
                );
                self.bridge.call_async(
                    &self.ai,
                    run.run_id,
                    call_id.to_owned(),
                    ServiceCall::Subtitles(call),
                );
                false
            }
            RequestedCall::Vision { prompt, images } => {
                let model = self
                    .ai
                    .binding(Capability::Multimodal)
                    .map(|b| b.model.clone())
                    .unwrap_or_default();
                self.append_workflow_log(
                    call_id,
                    &format!("· 发往 {model}（{} 张图）", images.len()),
                );
                let media: Vec<String> = images
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect();
                let request = todo_production::tasks::MultimodalRequest {
                    model: String::new(),
                    prompt,
                    digests: todo_ai::providers::vision::media_digests(&media),
                    media,
                    json: false,
                };
                let key = self
                    .ai
                    .submit(todo_production::TaskRequest::Multimodal(request));
                self.workflow_keys.insert(key, call_id.to_owned());
                true
            }
            RequestedCall::WaitPlacement => {
                self.append_workflow_log(call_id, "· 等待入轨落地…");
                self.placement_waits
                    .insert(call_id.to_owned(), Instant::now());
                true
            }
            RequestedCall::TimelineStrip {
                every_frames,
                max_frames,
                thumb_height,
                range,
            } => {
                let Some(project) = project.cloned() else {
                    self.append_workflow_log(call_id, "· 时间轴截帧失败：没有打开的工程");
                    let _ = self.local_call_tx.send((
                        call_id.to_owned(),
                        Err("时间轴截帧需要先打开工程".into()),
                    ));
                    return true;
                };
                let range = crate::adapters::frame::StripRange::parse(&range);
                self.append_workflow_log(
                    call_id,
                    &format!(
                        "· 时间轴截帧：每 {every_frames} 帧 / 最多 {max_frames} 张（{:?}）",
                        range
                    ),
                );
                let root = self.root.clone();
                let workdir = run.workdir.clone();
                let out_dir = workdir.join("timeline_strip").join(call_id);
                let tx = self.local_call_tx.clone();
                let call_id = call_id.to_owned();
                std::thread::spawn(move || {
                    let rel = |abs: &std::path::Path| -> Result<String, String> {
                        abs.strip_prefix(&workdir)
                            .map(|p| p.to_string_lossy().replace('\\', "/"))
                            .map_err(|_| format!("截帧输出不在工作目录内：{}", abs.display()))
                    };
                    let result = crate::adapters::frame::render_timeline_strip(
                        &root,
                        &project,
                        &out_dir,
                        every_frames,
                        max_frames,
                        thumb_height,
                        range,
                    )
                    .and_then(|strip| {
                        let frames: Result<Vec<serde_json::Value>, String> = strip
                            .frames
                            .iter()
                            .map(|frame| {
                                Ok(serde_json::json!({
                                    "path": rel(&frame.png)?,
                                    "mime": "image/png",
                                    "time": frame.time,
                                }))
                            })
                            .collect();
                        Ok(serde_json::json!({
                            "path": rel(&strip.path)?,
                            "mime": "image/png",
                            "width": strip.width,
                            "height": strip.height,
                            "fps": strip.fps,
                            "everyFrames": strip.every_frames,
                            "frames": frames?,
                        }))
                    });
                    let _ = tx.send((call_id, result));
                });
                true
            }
        }
    }

    /// True while a canvas workflow is live or AI Done events are still draining.
    pub fn workflow_busy(&self) -> bool {
        !self.workflow_live.is_empty() || !self.ai_event_backlog.is_empty()
    }

    /// `(done, total, waiting node(s))` for a running workflow.
    pub fn workflow_progress(&self, extension_id: &str) -> Option<(usize, usize, Option<String>)> {
        let run = self.workflow_live.get(extension_id)?;
        let (done, total) = run.job.progress();
        let waiting = {
            let nodes = run.job.waiting_nodes();
            match nodes.as_slice() {
                [] => None,
                [one] => Some((*one).to_owned()),
                many => Some(format!("{} 路并行 · {}", many.len(), many.join("、"))),
            }
        };
        Some((done, total, waiting))
    }

    /// Run one installed workflow graph with the host's services.
    ///
    /// Blocking: each node waits for its AI task. Call it from a worker thread
    /// if the UI must stay responsive; the AI journal still makes it resumable.
    pub fn run_workflow(
        &mut self,
        extension_id: &str,
        script: String,
    ) -> Result<todo_workflow::RunReport, String> {
        let extension = self
            .registry
            .get(extension_id)
            .ok_or_else(|| format!("没有扩展 {extension_id}"))?;
        let workflow = extension
            .workflow()
            .map_err(|problems| {
                problems
                    .iter()
                    .map(|p| format!("{}: {}", p.path, p.message))
                    .collect::<Vec<_>>()
                    .join("; ")
            })?
            .ok_or_else(|| format!("{extension_id} 没有 graph.json"))?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let workdir = self
            .root
            .join("cache/workflow-runs")
            .join(format!("{}-{stamp}", extension_id.replace(['.', '/', '\\'], "-")));
        let run_id = stamp as todo_workflow::RunId;
        let run_dir = workdir.clone();
        let mut report = crate::adapters::workflow::run_workflow(
            &workflow,
            &extension.dir,
            workdir,
            &mut self.bridge,
            &self.ai,
            run_id,
            script,
        );
        for node in &mut report.nodes {
            absolutize_node_media(node, &run_dir);
        }
        self.workflow_runs
            .insert(extension_id.to_owned(), report.clone());
        Ok(report)
    }

    /// Declared workflow graph for the panel (None for narration / no graph).
    /// `canvas` is the open project's resolution (`宽×高 · fps`), shown on
    /// subtitle nodes; pass `None` when no project is open.
    pub fn workflow_panel(
        &self,
        extension_id: &str,
        canvas: Option<(u32, u32, f64)>,
    ) -> Option<todo_interaction::WorkflowPanelView> {
        let extension = self.registry.get(extension_id)?;
        let workflow = self.workflow_cache.get(&extension)?;
        let canvas_label = canvas.map(|(w, h, fps)| {
            if fps > 0.0 {
                format!("{w}×{h} · {fps:.0}fps")
            } else {
                format!("{w}×{h}")
            }
        });
        Some(todo_interaction::WorkflowPanelView {
            nodes: workflow
                .node_cards()
                .into_iter()
                .map(|n| {
                    let mut params = n.params.clone();
                    if n.node_type == "subtitles" {
                        if let Some(label) = canvas_label.as_ref() {
                            params.insert("canvas".into(), serde_json::json!(label));
                        }
                    }
                    let fields = workflow_inspector_fields(
                        &n.node_type,
                        &params,
                        &self.model_choices(&n.node_type),
                    );
                    let last_for_media = self
                        .workflow_live
                        .get(extension_id)
                        .and_then(|run| {
                            run.job
                                .report()
                                .nodes
                                .iter()
                                .find(|run| run.id == n.id)
                                .map(|node| (node, run.workdir.as_path()))
                        })
                        .or_else(|| {
                            self.workflow_runs
                                .get(extension_id)
                                .and_then(|report| {
                                    report
                                        .nodes
                                        .iter()
                                        .find(|run| run.id == n.id)
                                        .map(|node| (node, extension.dir.as_path()))
                                })
                        });
                    let files = relay_media_files(
                        &n.params,
                        &extension.dir,
                        last_for_media.map(|(node, dir)| (node, dir)),
                    );
                    let text_blocks = relay_text_blocks(
                        &n.params,
                        last_for_media.map(|(node, _)| node),
                    );
                    let params_json =
                        serde_json::to_string(&serde_json::Value::Object(params.clone()))
                            .unwrap_or_else(|_| "{}".into());
                    let live = self.workflow_live.get(extension_id);
                    let waiting = live.and_then(|run| run.job.waiting_on());
                    let last = match live {
                        Some(run) => run.job.report().nodes.iter().find(|run| run.id == n.id),
                        None => self
                            .workflow_runs
                            .get(extension_id)
                            .and_then(|report| report.nodes.iter().find(|run| run.id == n.id)),
                    };
                    let status = if waiting == Some(n.id.as_str())
                        || last.is_some_and(|run| run.ok && run.message.starts_with("进行中"))
                    {
                        todo_interaction::ProductionNodeStatus::Running
                    } else {
                        match last {
                            None => todo_interaction::ProductionNodeStatus::Queued,
                            Some(run) if run.ok => todo_interaction::ProductionNodeStatus::Complete,
                            Some(_) => todo_interaction::ProductionNodeStatus::Failed,
                        }
                    };
                    let elapsed = self
                        .workflow_live
                        .get(extension_id)
                        .and_then(|run| run.job.current_elapsed());
                    let detail = if waiting == Some(n.id.as_str()) {
                        Some(match elapsed {
                            Some(elapsed) => format!("运行中 {:.1}s", elapsed.as_secs_f32()),
                            None => "运行中".to_owned(),
                        })
                    } else if let Some(run) = last.filter(|run| run.ok && run.message.starts_with("进行中")) {
                        Some(run.message.clone())
                    } else {
                        last.map(|run| {
                            if run.ok {
                                format!("{} · {:.1}s", run.outputs.join(","), run.duration_ms as f64 / 1000.0)
                            } else {
                                run.message.clone()
                            }
                        })
                    };
                    let node_id = n.id.clone();
                    let has_cache = n.node_type == "image_gen"
                        && todo_workflow::nodes::image_node_has_cache(&extension.dir, &node_id);
                    todo_interaction::WorkflowNodeView {
                        id: node_id,
                        node_type: n.node_type,
                        label: n.label,
                        summary: n.summary,
                        docs: n.docs,
                        status,
                        detail,
                        image: last.and_then(|run| run.image.clone()),
                        output: last.and_then(|run| run.preview.clone()),
                        output_chars: last.and_then(|run| run.output_chars),
                        logs: match (waiting == Some(n.id.as_str()), last) {
                            (true, _) => live
                                .map(|run| run.job.current_logs().to_vec())
                                .unwrap_or_default(),
                            (false, Some(run)) => run.logs.clone(),
                            (false, None) => Vec::new(),
                        },
                        inputs: n
                            .inputs
                            .into_iter()
                            .map(|p| format!("{}:{}", p.name, p.kind))
                            .collect(),
                        outputs: n
                            .outputs
                            .into_iter()
                            .map(|p| format!("{}:{}", p.name, p.kind))
                            .collect(),
                        x: n.x,
                        y: n.y,
                        fields,
                        files,
                        text_blocks,
                        params_json,
                        has_cache,
                    }
                })
                .collect(),
            log_tail: self.workflow_logs.get(extension_id).and_then(|path| {
                let text = std::fs::read_to_string(path).ok()?;
                let lines: Vec<&str> = text.lines().collect();
                let start = lines.len().saturating_sub(200);
                Some(lines[start..].join("\n"))
            }),
            running: self.workflow_progress(extension_id).map(|(done, total, waiting)| {
                match waiting {
                    Some(node) => format!("{done}/{total} · 等待 {node}"),
                    None => format!("{done}/{total}"),
                }
            }),
            edges: workflow
                .edges()
                .iter()
                .map(|e| todo_interaction::WorkflowEdgeView {
                    from: e.from.clone(),
                    to: e.to.clone(),
                    out: e.out.clone(),
                    input: e.input.clone(),
                })
                .collect(),
            palette: {
                let mut choices: Vec<_> = todo_workflow::workflow::nodes::CATALOG
                    .iter()
                    .map(|s| todo_interaction::WorkflowTypeChoice {
                        type_name: s.type_name.to_owned(),
                        label: s.label.to_owned(),
                        group: s.group.label().to_owned(),
                        group_order: s.group.order(),
                    })
                    .collect();
                choices.sort_by(|a, b| {
                    a.group_order
                        .cmp(&b.group_order)
                        .then_with(|| a.label.cmp(&b.label))
                });
                choices
            },
        })
    }

    /// Packages that declare a workflow graph (side dock list).
    pub fn workflow_entries(&self) -> Vec<todo_interaction::WorkflowEntryView> {
        let mut entries: Vec<_> = self
            .registry
            .list()
            .filter(|e| e.manifest.graph.is_some())
            .map(|extension| {
                let status = match self.check_results.get(&extension.manifest.id) {
                    None => ExtensionStatus::Checking,
                    Some(Err(e)) if e.contains("Python") || e.contains("python") => {
                        ExtensionStatus::RuntimeMissing
                    }
                    Some(Err(_)) => ExtensionStatus::SelfCheckFailed,
                    Some(Ok(())) => ExtensionStatus::Ready,
                };
                let summary = extension.manifest.description.clone();
                let summary = {
                    let first = summary
                        .split(['。', '；', '\n'])
                        .next()
                        .unwrap_or_default()
                        .trim();
                    let mut out: String = first.chars().take(36).collect();
                    if first.chars().count() > 36 {
                        out.push('…');
                    }
                    out
                };
                todo_interaction::WorkflowEntryView {
                    extension_id: extension.manifest.id.clone(),
                    name: extension.manifest.name.clone(),
                    summary,
                    origin_user: extension.origin == Origin::User,
                    status,
                }
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    fn rescan_registry(&mut self) {
        self.workflow_cache.clear();
        self.registry = Registry::scan(
            &self.root.join("extensions/builtin"),
            &self.root.join("extensions/user"),
        );
    }

    /// Install or dry-validate a `.todoext` into `extensions/user` (Agent / UI 同源).
    pub fn import_extension(
        &mut self,
        path: &std::path::Path,
        dry_run: bool,
    ) -> Result<todo_application::control::ExtensionSummary, String> {
        let bytes = std::fs::read(path).map_err(|e| {
            format!(
                "读扩展包失败：{}（{}）。请确认路径指向 .todoext 文件。",
                path.display(),
                e
            )
        })?;
        if dry_run {
            let manifest = todo_workflow::validate_package(&bytes).map_err(|e| e.to_string())?;
            return Ok(todo_application::control::ExtensionSummary {
                id: manifest.id.clone(),
                name: manifest.name.clone(),
                version: manifest.version.clone(),
                origin: "user".into(),
                has_graph: manifest.graph.is_some(),
                description: manifest.description.clone(),
                status: "validated".into(),
            });
        }
        let user = self.root.join("extensions/user");
        let extension = todo_workflow::install(&bytes, &user).map_err(|e| e.to_string())?;
        self.rescan_registry();
        self.check_results
            .insert(extension.manifest.id.clone(), Ok(()));
        Ok(todo_application::control::ExtensionSummary {
            id: extension.manifest.id.clone(),
            name: extension.manifest.name.clone(),
            version: extension.manifest.version.clone(),
            origin: "user".into(),
            has_graph: extension.manifest.graph.is_some(),
            description: extension.manifest.description.clone(),
            status: "ready".into(),
        })
    }

    /// Create a blank user workflow; returns its extension id.
    pub fn create_workflow(&mut self, name: &str) -> Result<String, String> {
        let user = self.root.join("extensions/user");
        std::fs::create_dir_all(&user).map_err(|e| e.to_string())?;
        let id = create_blank_workflow(&user, name)?;
        self.rescan_registry();
        self.check_results.insert(id.clone(), Ok(()));
        Ok(id)
    }

    /// Delete a user workflow package. Built-in packages cannot be removed.
    pub fn delete_workflow(&mut self, extension_id: &str) -> Result<(), String> {
        let extension = self
            .registry
            .get(extension_id)
            .ok_or_else(|| format!("找不到工作流 {extension_id}"))?;
        if extension.origin != Origin::User {
            return Err("内置工作流不能删除".into());
        }
        let user_root = self.root.join("extensions/user");
        if !extension.dir.starts_with(&user_root) {
            return Err("拒绝删除非用户工作流目录".into());
        }
        let _ = self.stop_workflow(extension_id);
        self.workflow_live.remove(extension_id);
        std::fs::remove_dir_all(&extension.dir).map_err(|e| e.to_string())?;
        self.check_results.remove(extension_id);
        self.rescan_registry();
        Ok(())
    }

    /// Ensure the package is writable under extensions/user (copy builtin if needed).
    fn writable_extension(&mut self, extension_id: &str) -> Result<std::sync::Arc<Extension>, String> {
        let extension = self
            .registry
            .get(extension_id)
            .ok_or_else(|| format!("找不到扩展 {extension_id}"))?;
        if extension.origin == Origin::User {
            return Ok(extension);
        }
        let user_dir = self.root.join("extensions/user").join(extension_id);
        copy_package(&extension.dir, &user_dir)?;
        self.rescan_registry();
        self.registry
            .get(extension_id)
            .ok_or_else(|| format!("覆盖后仍找不到 {extension_id}"))
    }

    /// Persist inspector edits into the extension's graph.json (keeps files list).
    pub fn save_workflow_node_params(
        &mut self,
        extension_id: &str,
        node_id: &str,
        edits: &BTreeMap<String, String>,
    ) -> Result<(), String> {
        let extension = self.writable_extension(extension_id)?;
        let workflow = extension
            .workflow()
            .map_err(|ps| {
                ps.iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            })?
            .ok_or_else(|| "扩展没有工作流图".to_string())?;
        let node = workflow
            .nodes()
            .iter()
            .find(|n| n.id == node_id)
            .ok_or_else(|| format!("没有节点 {node_id}"))?;
        let mut params = node.params.clone();
        for (key, raw) in edits {
            if key == "files" {
                continue;
            }
            let value = if let Ok(n) = raw.parse::<i64>() {
                serde_json::json!(n)
            } else if let Ok(n) = raw.parse::<f64>() {
                serde_json::json!(n)
            } else {
                serde_json::json!(raw)
            };
            params.insert(key.clone(), value);
        }
        extension.save_node_params(node_id, params)
    }

    /// Save full graph: node positions + optional per-node field edits.
    pub fn save_workflow_graph(
        &mut self,
        extension_id: &str,
        positions: &BTreeMap<String, (f32, f32)>,
        edits: &BTreeMap<String, BTreeMap<String, String>>,
    ) -> Result<(), String> {
        let extension = self.writable_extension(extension_id)?;
        let workflow = extension
            .workflow()
            .map_err(|ps| {
                ps.iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            })?
            .ok_or_else(|| "扩展没有工作流图".to_string())?;
        let mut graph = workflow.graph.clone();
        for node in &mut graph.nodes {
            if let Some(&(x, y)) = positions.get(&node.id) {
                node.x = Some(x as f64);
                node.y = Some(y as f64);
            }
            if let Some(fields) = edits.get(&node.id) {
                for (key, raw) in fields {
                    if key == "files" {
                        continue;
                    }
                    let value = if let Ok(n) = raw.parse::<i64>() {
                        serde_json::json!(n)
                    } else if let Ok(n) = raw.parse::<f64>() {
                        serde_json::json!(n)
                    } else {
                        serde_json::json!(raw)
                    };
                    node.params.insert(key.clone(), value);
                }
            }
        }
        extension.save_graph(&graph)
    }

    fn load_writable_graph(
        &mut self,
        extension_id: &str,
        positions: &BTreeMap<String, (f32, f32)>,
    ) -> Result<(std::sync::Arc<Extension>, todo_workflow::workflow::graph::Graph), String> {
        use todo_workflow::workflow::graph::Graph;
        let extension = self.writable_extension(extension_id)?;
        let workflow = extension
            .workflow()
            .map_err(|ps| {
                ps.iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            })?
            .ok_or_else(|| "扩展没有工作流图".to_string())?;
        let mut graph: Graph = workflow.graph.clone();
        for node in &mut graph.nodes {
            if let Some(&(x, y)) = positions.get(&node.id) {
                node.x = Some(x as f64);
                node.y = Some(y as f64);
            }
        }
        Ok((extension, graph))
    }

    fn unique_node_id(graph: &todo_workflow::workflow::graph::Graph, base: &str) -> String {
        let safe: String = base
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let base = if safe.is_empty() { "node" } else { &safe };
        let existing: std::collections::BTreeSet<_> =
            graph.nodes.iter().map(|n| n.id.as_str()).collect();
        if !existing.contains(base) {
            return base.to_owned();
        }
        for i in 2..10_000 {
            let id = format!("{base}_{i}");
            if !existing.contains(id.as_str()) {
                return id;
            }
        }
        format!("{base}_{}", graph.nodes.len() + 1)
    }

    fn default_node_params(
        node_type: &str,
    ) -> std::collections::BTreeMap<String, serde_json::Value> {
        use todo_workflow::workflow::nodes::{self, ParamField};
        let mut params = std::collections::BTreeMap::new();
        let Some(spec) = nodes::lookup(node_type) else {
            return params;
        };
        for field in spec.fields {
            match field {
                ParamField::Select { id, default, .. } => {
                    params.insert((*id).into(), serde_json::json!(*default));
                }
                ParamField::Number { id, default, .. } => {
                    params.insert((*id).into(), serde_json::json!(*default));
                }
                ParamField::Text { id, default, .. } => {
                    if !default.is_empty() {
                        params.insert((*id).into(), serde_json::json!(*default));
                    }
                }
                ParamField::FileKinds { id, accept, .. } => {
                    params.insert((*id).into(), serde_json::json!(*accept));
                }
            }
        }
        params
    }

    /// Insert a catalog node at graph coordinates.
    pub fn add_workflow_node(
        &mut self,
        extension_id: &str,
        node_type: &str,
        x: f32,
        y: f32,
        positions: &BTreeMap<String, (f32, f32)>,
    ) -> Result<String, String> {
        use todo_workflow::workflow::graph::GraphNode;
        use todo_workflow::workflow::nodes;
        let spec = nodes::lookup(node_type).ok_or_else(|| format!("未知节点类型 {node_type}"))?;
        let (extension, mut graph) = self.load_writable_graph(extension_id, positions)?;
        let id = Self::unique_node_id(&graph, spec.type_name);
        graph.nodes.push(GraphNode {
            id: id.clone(),
            node_type: spec.type_name.to_owned(),
            label: Some(spec.label.to_owned()),
            params: Self::default_node_params(spec.type_name),
            x: Some(x as f64),
            y: Some(y as f64),
        });
        extension.save_graph(&graph)?;
        Ok(id)
    }

    /// Duplicate a node (params + label; no edges), offset on canvas.
    pub fn duplicate_workflow_node(
        &mut self,
        extension_id: &str,
        node_id: &str,
        positions: &BTreeMap<String, (f32, f32)>,
    ) -> Result<String, String> {
        use todo_workflow::workflow::graph::GraphNode;
        let (extension, mut graph) = self.load_writable_graph(extension_id, positions)?;
        let src = graph
            .nodes
            .iter()
            .find(|n| n.id == node_id)
            .ok_or_else(|| format!("没有节点 {node_id}"))?
            .clone();
        let id = Self::unique_node_id(&graph, &src.node_type);
        let (x, y) = positions
            .get(node_id)
            .copied()
            .map(|(x, y)| (x as f64, y as f64))
            .or_else(|| Some((src.x.unwrap_or(80.0), src.y.unwrap_or(80.0))))
            .unwrap();
        graph.nodes.push(GraphNode {
            id: id.clone(),
            node_type: src.node_type,
            label: src.label,
            params: src.params,
            x: Some(x + 48.0),
            y: Some(y + 48.0),
        });
        extension.save_graph(&graph)?;
        Ok(id)
    }

    /// Remove a node and any edges that touch it.
    pub fn delete_workflow_node(
        &mut self,
        extension_id: &str,
        node_id: &str,
        positions: &BTreeMap<String, (f32, f32)>,
    ) -> Result<(), String> {
        let (extension, mut graph) = self.load_writable_graph(extension_id, positions)?;
        if !graph.nodes.iter().any(|n| n.id == node_id) {
            return Err(format!("没有节点 {node_id}"));
        }
        graph.nodes.retain(|n| n.id != node_id);
        graph
            .edges
            .retain(|e| e.from != node_id && e.to != node_id);
        extension.save_graph(&graph)
    }

    /// Paste a copied node (type + JSON params) at a graph position.
    pub fn paste_workflow_node(
        &mut self,
        extension_id: &str,
        node_type: &str,
        label: Option<String>,
        params_json: &str,
        x: f32,
        y: f32,
        positions: &BTreeMap<String, (f32, f32)>,
    ) -> Result<String, String> {
        use todo_workflow::workflow::graph::GraphNode;
        use todo_workflow::workflow::nodes;
        let spec = nodes::lookup(node_type).ok_or_else(|| format!("未知节点类型 {node_type}"))?;
        let params: BTreeMap<String, serde_json::Value> = if params_json.is_empty() {
            Self::default_node_params(spec.type_name)
        } else {
            serde_json::from_str(params_json).map_err(|e| e.to_string())?
        };
        let (extension, mut graph) = self.load_writable_graph(extension_id, positions)?;
        let id = Self::unique_node_id(&graph, spec.type_name);
        graph.nodes.push(GraphNode {
            id: id.clone(),
            node_type: spec.type_name.to_owned(),
            label: label.or_else(|| Some(spec.label.to_owned())),
            params,
            x: Some(x as f64),
            y: Some(y as f64),
        });
        extension.save_graph(&graph)?;
        Ok(id)
    }

    /// Add an edge; port names optional when unambiguous / compatible.
    pub fn connect_workflow_edge(
        &mut self,
        extension_id: &str,
        from: &str,
        to: &str,
        out: Option<&str>,
        input: Option<&str>,
        positions: &BTreeMap<String, (f32, f32)>,
    ) -> Result<(), String> {
        use todo_workflow::workflow::graph::GraphEdge;
        use todo_workflow::workflow::nodes::resolve_edge_ports;
        if from == to {
            return Err("不能连接到自身".into());
        }
        let (extension, mut graph) = self.load_writable_graph(extension_id, positions)?;
        let from_type = graph
            .nodes
            .iter()
            .find(|n| n.id == from)
            .map(|n| n.node_type.clone())
            .ok_or_else(|| format!("没有源节点 {from}"))?;
        let to_type = graph
            .nodes
            .iter()
            .find(|n| n.id == to)
            .map(|n| n.node_type.clone())
            .ok_or_else(|| format!("没有目标节点 {to}"))?;
        let (out_port, in_port) =
            resolve_edge_ports(&from_type, &to_type, out, input).map_err(|e| e)?;
        graph.edges.retain(|e| {
            !(e.from == from
                && e.to == to
                && e.out.as_deref() == Some(out_port.name)
                && e.input.as_deref() == Some(in_port.name))
        });
        // One wire per target input port (replace previous source).
        graph.edges.retain(|e| {
            !(e.to == to && e.input.as_deref() == Some(in_port.name))
        });
        graph.edges.push(GraphEdge {
            from: from.to_owned(),
            to: to.to_owned(),
            out: Some(out_port.name.to_owned()),
            input: Some(in_port.name.to_owned()),
        });
        extension.save_graph(&graph)
    }

    pub fn disconnect_workflow_edge(
        &mut self,
        extension_id: &str,
        from: &str,
        to: &str,
        out: Option<&str>,
        input: Option<&str>,
        positions: &BTreeMap<String, (f32, f32)>,
    ) -> Result<(), String> {
        let (extension, mut graph) = self.load_writable_graph(extension_id, positions)?;
        let before = graph.edges.len();
        graph.edges.retain(|e| {
            if e.from != from || e.to != to {
                return true;
            }
            if let Some(o) = out {
                if e.out.as_deref() != Some(o) {
                    return true;
                }
            }
            if let Some(i) = input {
                if e.input.as_deref() != Some(i) {
                    return true;
                }
            }
            false
        });
        if graph.edges.len() == before {
            return Err("没有找到这条连线".into());
        }
        extension.save_graph(&graph)
    }

    /// After a run, write relay `segments` into `params.blocks` so the node
    /// keeps the text-block mid-store without needing another wire.
    fn persist_relay_blocks(
        &mut self,
        extension_id: &str,
        report: &todo_workflow::runner::RunReport,
    ) -> Result<(), String> {
        let mut updates: Vec<(String, serde_json::Value, String)> = Vec::new();
        for node in &report.nodes {
            if node.node_type != "relay" || !node.ok {
                continue;
            }
            let Some(segments) = &node.segments else {
                continue;
            };
            let Some(blocks) = segments.get("segments").cloned() else {
                continue;
            };
            if !blocks.as_array().is_some_and(|list| !list.is_empty()) {
                continue;
            }
            let mode = segments
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("semantic")
                .to_owned();
            updates.push((node.id.clone(), blocks, mode));
        }
        if updates.is_empty() {
            return Ok(());
        }
        let extension = self.writable_extension(extension_id)?;
        let mut graph = {
            let path = extension
                .manifest
                .graph
                .as_ref()
                .ok_or_else(|| "扩展没有 graph".to_string())?;
            let graph_path = extension.dir.join(path);
            let text = std::fs::read_to_string(&graph_path).map_err(|e| e.to_string())?;
            serde_json::from_str::<todo_workflow::workflow::graph::Graph>(&text)
                .map_err(|e| e.to_string())?
        };
        for (node_id, blocks, mode) in updates {
            let Some(node) = graph.nodes.iter_mut().find(|n| n.id == node_id) else {
                continue;
            };
            node.params.insert("blocks".into(), blocks);
            node.params
                .insert("mode".into(), serde_json::Value::String(mode));
        }
        extension.save_graph(&graph)
    }

    /// Pick-and-copy files into a media_bin node.
    pub fn add_workflow_media_files(
        &mut self,
        extension_id: &str,
        node_id: &str,
        sources: &[std::path::PathBuf],
    ) -> Result<usize, String> {
        let extension = self.writable_extension(extension_id)?;
        extension.add_media_bin_files(node_id, sources)
    }

    pub fn remove_workflow_media_file(
        &mut self,
        extension_id: &str,
        node_id: &str,
        rel_path: &str,
    ) -> Result<(), String> {
        let extension = self.writable_extension(extension_id)?;
        extension.remove_media_bin_file(node_id, rel_path)
    }

    /// Clear image_gen (etc.) node cache under the extension package.
    pub fn clear_workflow_node_cache(
        &mut self,
        extension_id: &str,
        node_id: &str,
    ) -> Result<(), String> {
        let extension = self.writable_extension(extension_id)?;
        todo_workflow::nodes::clear_image_cache(&extension.dir, node_id)
    }

    pub fn workflow_node_has_cache(&self, extension_id: &str, node_id: &str) -> bool {
        self.registry
            .get(extension_id)
            .is_some_and(|ext| todo_workflow::nodes::image_node_has_cache(&ext.dir, node_id))
    }

    /// Canvas import: drop onto an existing media_bin/relay, or create a media_bin at (x, y).
    pub fn import_workflow_media(
        &mut self,
        extension_id: &str,
        sources: &[std::path::PathBuf],
        x: f32,
        y: f32,
        target_node: Option<&str>,
        positions: &BTreeMap<String, (f32, f32)>,
    ) -> Result<(String, usize), String> {
        if sources.is_empty() {
            return Err("没有可导入的文件".into());
        }
        let node_id = if let Some(id) = target_node {
            let (_, graph) = self.load_writable_graph(extension_id, positions)?;
            let node = graph
                .nodes
                .iter()
                .find(|n| n.id == id)
                .ok_or_else(|| format!("没有节点 {id}"))?;
            if !matches!(node.node_type.as_str(), "media_bin" | "relay") {
                return Err("只能导入到素材容器或中转节点；空白处拖入会新建素材容器".into());
            }
            id.to_owned()
        } else {
            self.add_workflow_node(extension_id, "media_bin", x, y, positions)?
        };
        let n = self.add_workflow_media_files(extension_id, &node_id, sources)?;
        if n == 0 {
            return Err("没有成功导入任何文件".into());
        }
        Ok((node_id, n))
    }

    /// Extension cards for the production panel.
    pub fn cards(&self) -> Vec<ExtensionCardView> {
        let mut cards: Vec<ExtensionCardView> = self
            .registry
            .list()
            .map(|extension| {
                let manifest = &extension.manifest;
                let services = &manifest.requires.services;
                let needs = |provider: Provider| match provider {
                    Provider::Huniu => {
                        services.contains(&Service::Chat) || services.contains(&Service::Image)
                    }
                    Provider::Volc => services.contains(&Service::Speech),
                };
                let missing_key = [Provider::Huniu, Provider::Volc]
                    .into_iter()
                    .any(|p| needs(p) && !self.keys.get(&p).copied().unwrap_or(false));
                let (status, problem) = match self.check_results.get(&manifest.id) {
                    None => (ExtensionStatus::Checking, None),
                    Some(Err(e)) if e.contains("Python") || e.contains("python") => {
                        (ExtensionStatus::RuntimeMissing, Some(e.clone()))
                    }
                    Some(Err(e)) => (ExtensionStatus::SelfCheckFailed, Some(e.clone())),
                    Some(Ok(())) if missing_key => (
                        ExtensionStatus::MissingKey,
                        Some("需要在 AI 设置里填写服务 key".into()),
                    ),
                    Some(Ok(())) => (ExtensionStatus::Ready, None),
                };
                ExtensionCardView {
                    id: manifest.id.clone(),
                    name: manifest.name.clone(),
                    description: manifest.description.clone(),
                    status,
                    problem,
                    preview: manifest.previews.first().cloned(),
                    params: manifest
                        .params
                        .iter()
                        .map(|p| param_view(p, manifest.quick.iter().any(|q| q == p.id())))
                        .collect(),
                    script: manifest.input.script.as_ref().map(|s| ScriptView {
                        label: s.label.clone(),
                        max_length: s.max_length,
                        placeholder: s.placeholder.clone(),
                        optional: s.optional,
                    }),
                }
            })
            .collect();
        for rejected in &self.registry.rejected {
            cards.push(ExtensionCardView {
                id: rejected.dir.to_string_lossy().into_owned(),
                name: rejected
                    .dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                description: "扩展包不合法，无法使用".into(),
                status: ExtensionStatus::SelfCheckFailed,
                problem: Some(
                    rejected
                        .problems
                        .iter()
                        .map(|p| format!("{}：{}", p.path, p.message))
                        .collect::<Vec<_>>()
                        .join("；"),
                ),
                preview: None,
                params: Vec::new(),
                script: None,
            });
        }
        cards
    }
}

/// Copy a graph-relative media path from the extension package into the run workdir when missing.
fn stage_workflow_reference(run: &WorkflowLive, reference: &str) {
    let reference = reference.trim();
    if reference.is_empty()
        || !todo_workflow::workflow::manifest::valid_relative_path(reference)
    {
        return;
    }
    let dest = run.workdir.join(reference);
    if dest.is_file() {
        return;
    }
    let src = run.extension_dir.join(reference);
    if !src.is_file() {
        return;
    }
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::copy(&src, &dest);
}

fn absolutize_node_media(node: &mut todo_workflow::runner::NodeRun, base: &std::path::Path) {
    if let Some(image) = node.image.clone() {
        if !std::path::Path::new(&image).is_absolute() {
            node.image = Some(base.join(image).to_string_lossy().into_owned());
        }
    }
    if let Some(files) = node.files.as_mut() {
        for key in ["files", "images"] {
            let Some(list) = files.get_mut(key).and_then(|v| v.as_array_mut()) else {
                continue;
            };
            for item in list {
                let Some(path) = item.get("path").and_then(|v| v.as_str()) else {
                    continue;
                };
                if std::path::Path::new(path).is_absolute() {
                    continue;
                }
                item["path"] = serde_json::Value::String(
                    base.join(path).to_string_lossy().into_owned(),
                );
            }
        }
    }
}

fn relay_text_blocks(
    params: &serde_json::Map<String, serde_json::Value>,
    last: Option<&todo_workflow::runner::NodeRun>,
) -> Vec<todo_interaction::WorkflowTextBlockView> {
    let from_params = params.get("blocks").and_then(|v| v.as_array());
    let from_run = last
        .and_then(|run| run.segments.as_ref())
        .and_then(|seg| seg.get("segments"))
        .and_then(|v| v.as_array());
    let list = from_params.or(from_run);
    list.into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, item)| {
            let text = item.get("text")?.as_str()?.to_owned();
            if text.trim().is_empty() {
                return None;
            }
            let id = item
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("s{}", index + 1));
            let chars = item
                .get("chars")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or_else(|| text.chars().count());
            Some(todo_interaction::WorkflowTextBlockView {
                id,
                index: item
                    .get("index")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize)
                    .unwrap_or(index),
                text,
                chars,
            })
        })
        .collect()
}

fn relay_media_files(
    params: &serde_json::Map<String, serde_json::Value>,
    package_dir: &std::path::Path,
    last: Option<(&todo_workflow::runner::NodeRun, &std::path::Path)>,
) -> Vec<todo_interaction::WorkflowMediaFileView> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut push = |path: String, kind: String, name: String, base: &std::path::Path| {
        if path.is_empty() || !seen.insert(path.clone()) {
            return;
        }
        let abs = if std::path::Path::new(&path).is_absolute() {
            std::path::PathBuf::from(&path)
        } else {
            base.join(&path)
        };
        // Skip per-frame is_file() (dozens of images × every node freezes the UI).
        // Missing files fail softly when the thumb worker loads them.
        let preview = matches!(kind.as_str(), "image" | "video")
            .then(|| abs.to_string_lossy().into_owned());
        out.push(todo_interaction::WorkflowMediaFileView {
            path,
            kind,
            name,
            preview,
        });
    };
    if let Some(list) = params.get("files").and_then(|v| v.as_array()) {
        for f in list {
            let Some(path) = f.get("path").and_then(|v| v.as_str()) else {
                continue;
            };
            let kind = f
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("other")
                .to_owned();
            let name = f
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path))
                .to_owned();
            push(path.to_owned(), kind, name, package_dir);
        }
    }
    if let Some((node, base)) = last {
        if let Some(files) = node.files.as_ref() {
            if let Some(list) = files.get("files").and_then(|v| v.as_array()) {
                for f in list {
                    let Some(path) = f.get("path").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let kind = f
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("other")
                        .to_owned();
                    let name = f
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path))
                        .to_owned();
                    push(path.to_owned(), kind, name, base);
                }
            }
            if let Some(list) = files.get("images").and_then(|v| v.as_array()) {
                // 面板每帧重建：路径列表要短，检查器只异步缩略图预览前几张。
                for (i, f) in list.iter().take(16).enumerate() {
                    let Some(path) = f.get("path").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let name = path
                        .rsplit('/')
                        .next()
                        .unwrap_or(path)
                        .to_owned();
                    let kind = f
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("image")
                        .to_owned();
                    let _ = i;
                    push(path.to_owned(), kind, name, base);
                }
            }
        }
        if let Some(path) = node.image.as_deref() {
            let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned();
            let kind = if path.to_ascii_lowercase().ends_with(".mp4")
                || path.to_ascii_lowercase().ends_with(".mov")
                || path.to_ascii_lowercase().ends_with(".webm")
                || path.to_ascii_lowercase().ends_with(".mkv")
            {
                "video".to_owned()
            } else {
                "image".to_owned()
            };
            push(path.to_owned(), kind, name, base);
        }
    }
    out
}

impl ProductionHost {
    /// Models offered for a node's `model` field: the configured default plus
    /// the known-good names for the capability.
    fn model_choices(&self, node_type: &str) -> Vec<(String, String)> {
        let capability = match node_type {
            "image_gen" => Capability::Image,
            "multimodal" => Capability::Multimodal,
            "tts" => Capability::Audio,
            _ => Capability::Language,
        };
        let mut out: Vec<(String, String)> = vec![(String::new(), "跟随 AI 设置".to_owned())];
        if let Some(binding) = self.ai.binding(capability) {
            let current = binding.model.clone();
            out.push((current.clone(), format!("当前：{current}")));
        }
        let wanted = match capability {
            Capability::Language => todo_ai::models::ModelCapability::Language,
            Capability::Image => todo_ai::models::ModelCapability::Image,
            Capability::Video => todo_ai::models::ModelCapability::Video,
            Capability::Multimodal => todo_ai::models::ModelCapability::Multimodal,
            Capability::Audio => todo_ai::models::ModelCapability::Audio,
        };
        for info in self.models.iter().filter(|info| info.supports(wanted)) {
            if !out.iter().any(|(value, _)| value == &info.id) {
                out.push((info.id.clone(), info.id.clone()));
            }
        }
        let extra: &[&str] = match capability {
            Capability::Language => &["deepseek-flash", "deepseek-v4-pro", "claude-fable-5"],
            Capability::Image => &["gpt-image-2"],
            Capability::Multimodal => &["deepseek-flash", "gemini-3.8-flash"],
            Capability::Video => &["sora-2"],
            Capability::Audio => &[],
        };
        for model in extra {
            if !out.iter().any(|(value, _)| value == model) {
                out.push(((*model).to_owned(), (*model).to_owned()));
            }
        }
        out
    }
}

fn workflow_inspector_fields(
    node_type: &str,
    params: &serde_json::Map<String, serde_json::Value>,
    models: &[(String, String)],
) -> Vec<todo_interaction::WorkflowParamFieldView> {
    use todo_interaction::WorkflowParamFieldView;
    use todo_workflow::workflow::nodes::{self, ParamField};
    let Some(spec) = nodes::lookup(node_type) else {
        return Vec::new();
    };
    spec.fields
        .iter()
        .map(|field| match field {
            ParamField::Select {
                id,
                label,
                hint,
                options,
                default,
            } => {
                let value = params
                    .get(*id)
                    .and_then(|v| v.as_str())
                    .unwrap_or(default)
                    .to_owned();
                WorkflowParamFieldView::Select {
                    id: (*id).into(),
                    label: (*label).into(),
                    hint: (*hint).into(),
                    options: options
                        .iter()
                        .map(|(v, l)| ((*v).into(), (*l).into()))
                        .collect(),
                    value,
                }
            }
            ParamField::Number {
                id,
                label,
                hint,
                min,
                max,
                step,
                default,
                unit,
            } => {
                let value = params
                    .get(*id)
                    .and_then(|v| v.as_f64())
                    .unwrap_or(*default);
                WorkflowParamFieldView::Number {
                    id: (*id).into(),
                    label: (*label).into(),
                    hint: (*hint).into(),
                    min: *min,
                    max: *max,
                    step: *step,
                    unit: unit.map(|u| u.to_owned()),
                    value,
                }
            }
            ParamField::Text {
                id,
                label,
                hint,
                default,
                placeholder,
            } if *id == "model" => {
                let value = params
                    .get(*id)
                    .and_then(|v| v.as_str())
                    .unwrap_or(default)
                    .to_owned();
                let mut options = models.to_vec();
                if !value.is_empty() && !options.iter().any(|(v, _)| v == &value) {
                    options.push((value.clone(), value.clone()));
                }
                WorkflowParamFieldView::Select {
                    id: (*id).into(),
                    label: (*label).into(),
                    hint: "留空跟随 AI 设置；也可选具体模型".into(),
                    options,
                    value,
                }
            }
            ParamField::Text {
                id,
                label,
                hint,
                default,
                placeholder,
            } => {
                let value = params
                    .get(*id)
                    .and_then(|v| v.as_str())
                    .unwrap_or(default)
                    .to_owned();
                WorkflowParamFieldView::Text {
                    id: (*id).into(),
                    label: (*label).into(),
                    hint: (*hint).into(),
                    placeholder: (*placeholder).into(),
                    value,
                }
            }
            ParamField::FileKinds {
                id,
                label,
                hint,
                accept,
            } => {
                let accept = params
                    .get(*id)
                    .and_then(|v| v.as_str())
                    .unwrap_or(accept)
                    .to_owned();
                WorkflowParamFieldView::FileKinds {
                    id: (*id).into(),
                    label: (*label).into(),
                    hint: (*hint).into(),
                    accept,
                }
            }
        })
        .collect()
}

fn uuid_like() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    todo_production::tasks::fingerprint(&(nanos, std::process::id()))
}
