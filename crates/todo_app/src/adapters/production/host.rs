//! Executes production tasks: AI requests, extension runs, saves; reports
//! results back as `ProductionOutput`. It decides nothing about what to
//! generate — that is `todo_application::productions`.
use super::bridge::{materialize, service_file, ServiceBridge};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use todo_ai::{
    default_paths, AiConfig, AiEvent, AiService, ModelInfo, Secret, Secrets, ServiceSettings,
};
use todo_application::{
    LocalRunId, Pcm16, ProductionOutput, ProductionSetup, ProductionTask, Recipe, TaskState,
};
use todo_extensions::{
    Budget, Extension, PoolOptions, Registry, RunId, RunRequest, RuntimePaths, WorkerEvent,
    WorkerPool,
};
use todo_interaction::{
    ChoiceView, ExtensionCardView, ExtensionStatus, ParamInput, ParamValue, ParamView, ScriptView,
};
use todo_production::extension::manifest::{Group, Kind, Param, Service};
use todo_production::extension::protocol::Canvas;
use todo_production::extension::{narration_spec, HostDefaults};
use todo_production::production::PackRef;
use todo_production::segment::Limits;
use todo_production::{
    Capability, ChatMessage, ChatRole, Production, Provider, SplitMode, TaskKey, TaskRequest,
    TaskSpec,
};
use todo_storage::ProductionStore;

/// Voices offered next to free entry (any id, cloned voices `S_…`).
const VOICES: &[(&str, &str)] = &[
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
}

pub struct ProductionHost {
    root: PathBuf,
    ai: AiService,
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
    defaults: HostDefaults,
    keys: HashMap<Provider, bool>,
    connection_test: Option<TaskKey>,
    connection_test_started: Option<Instant>,
    model_probe: Option<u64>,
}

fn ai_settings(
    root: &Path,
) -> Result<(ServiceSettings, HostDefaults, HashMap<Provider, bool>), String> {
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
        chat_model: config.huniu.chat_model.clone(),
        image_model: config.huniu.image_model.clone(),
        voice_resource: config.volc.resource_id.clone(),
        sample_rate: 24_000,
    };
    Ok((
        ServiceSettings {
            huniu_base_url: config.huniu.base_url.clone(),
            volc_base_url: config.volc.base_url.clone(),
            huniu_strategy: config.huniu.strategy.clone(),
            keys,
            journal_dir,
            artifacts_root,
            proxy: config.proxy,
            limits: config.limits.clone(),
            retry: todo_ai::net::RetryPolicy::default(),
            breaker: todo_ai::net::BreakerPolicy::default(),
        },
        defaults,
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
        let (settings, defaults, keys) = ai_settings(&root)?;
        let (ai, _recovered) = AiService::start(settings)?;
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
                let result = todo_extensions::check_extension(&extension, &runtime).map(|_| ());
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
        Ok(Self {
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
            defaults,
            keys,
            connection_test: None,
            connection_test_started: None,
            model_probe: None,
        })
    }

    pub fn set_key(&mut self, provider: Provider, key: Option<Secret>) {
        self.keys.insert(provider, key.is_some());
        self.ai.set_key(provider, key);
    }

    /// Submit a fresh, uncached chat request used by the settings dialog.
    /// The nonce deliberately prevents a previous successful test from being
    /// answered from the durable production journal.
    pub fn test_connection(&mut self) -> Result<(), String> {
        if !self.keys.get(&Provider::Huniu).copied().unwrap_or(false) {
            return Err("虎妞还没有配置 API Key".into());
        }
        if self.connection_test.is_some() || self.model_probe.is_some() {
            return Err("连接测试已经在进行中".into());
        }
        self.model_probe = Some(self.ai.list_models(Provider::Huniu));
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let spec = TaskSpec {
            provider: Provider::Huniu,
            request: TaskRequest::Chat(todo_production::ChatRequest {
                model: self.defaults.chat_model.clone(),
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
            }),
        };
        let key = self.ai.submit(spec.clone());
        self.submissions.insert(
            key.clone(),
            Submission {
                spec,
                capability: Capability::Chat,
                workdirs: Vec::new(),
            },
        );
        self.connection_test = Some(key);
        self.connection_test_started = Some(Instant::now());
        Ok(())
    }

    /// New service addresses or proxy: restart the AI service. Its journal
    /// keeps every task; in-flight ones resume and report as usual.
    pub fn reconfigure(&mut self) -> Result<(), String> {
        let (settings, defaults, keys) = ai_settings(&self.root)?;
        let (ai, _) = AiService::start(settings)?;
        self.ai = ai;
        self.defaults = defaults.clone();
        self.keys = keys;
        self.bridge = ServiceBridge::new(defaults, &self.root, self.workers.handle());
        // Requests the application had in flight are submitted again; the
        // journal answers finished ones without a new request.
        let specs: Vec<TaskSpec> = self.submissions.values().map(|s| s.spec.clone()).collect();
        for spec in specs {
            self.ai.submit(spec);
        }
        Ok(())
    }

    fn extension(&self, id: &str) -> Result<Arc<Extension>, String> {
        self.registry
            .get(id)
            .ok_or_else(|| format!("找不到扩展 {id}"))
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
            if script.trim().is_empty() {
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
        let capability = spec.capability();
        let key = self.ai.submit(spec.clone());
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
            if submission.capability == Capability::Speech {
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

    pub fn poll(&mut self) -> Vec<HostReport> {
        let mut reports = Vec::new();
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
            reports.push(HostReport::Notice("虎妞连接测试超时（30 秒）".into()));
        }
        while let Ok((id, result)) = self.checks.try_recv() {
            self.check_results.insert(id, result);
        }
        while let Ok(result) = self.saved.try_recv() {
            if let Err(e) = result {
                reports.push(HostReport::Notice(format!("成片保存失败：{e}")));
            }
        }
        while let Some(event) = self.ai.poll() {
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
            if matches!(&event, AiEvent::Models { id, .. } if self.model_probe == Some(*id)) {
                if let AiEvent::Models {
                    provider, models, ..
                } = event
                {
                    self.model_probe = None;
                    if self.connection_test.is_none() {
                        self.connection_test_started = None;
                    }
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
                        reports.push(HostReport::Notice(format!(
                            "虎妞连接成功：{}",
                            self.defaults.chat_model
                        )));
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
                        reports.push(HostReport::Notice(format!(
                            "虎妞连接失败：{class}：{message}"
                        )));
                    }
                    AiEvent::Cancelled { key } => {
                        self.connection_test = None;
                        if self.model_probe.is_none() {
                            self.connection_test_started = None;
                        }
                        self.submissions.remove(&key);
                        reports.push(HostReport::Notice("连接测试已取消".into()));
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
        reports
    }

    fn finish_run(&mut self, run: LocalRunId, pool_run: RunId) {
        self.local_runs.remove(&run);
        self.pool_runs.remove(&pool_run);
        self.bridge.remove_run(pool_run);
    }

    pub fn extension_name(&self, id: &str) -> Option<String> {
        self.registry.get(id).map(|e| e.manifest.name.clone())
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

fn uuid_like() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    todo_production::tasks::fingerprint(&(nanos, std::process::id()))
}
