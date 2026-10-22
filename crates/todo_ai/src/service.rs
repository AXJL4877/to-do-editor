//! The background executor. Owns a small tokio runtime on its own thread and
//! talks to the app through channels, like the other workers. Every state
//! change is journaled before the next network step, so a crash, a cancel or
//! a lost connection resumes from where it was instead of paying again.
use crate::artifacts::{ArtifactRef, ArtifactStore};
use crate::config::{Limits, ProxyMode};
use crate::journal::{Journal, Recovered, TaskRecord, TaskState};
use crate::net::http::Http;
use crate::net::{
    deadline, Attempts, Breaker, BreakerPolicy, Decision, ErrorClass, Permit, RetryPolicy,
};
use crate::providers::{huniu, volc, AttemptError, Outcome};
use crate::Secret;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use todo_production::{Capability, Provider, TaskKey, TaskRequest, TaskSpec};
use tokio::sync::{Notify, Semaphore};
use tokio::time::Instant;

/// A dedicated endpoint for the language capability (e.g. DeepSeek direct).
///
/// When present, chat **must** use [`Self::model`] on this origin — never the
/// huniu language model name (those families do not mix).
#[derive(Debug, Clone, PartialEq)]
pub struct LanguageEndpoint {
    pub base_url: String,
    pub key: Secret,
    /// Model id accepted by `base_url` (e.g. `deepseek-flash`).
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct ServiceSettings {
    pub huniu_base_url: String,
    /// Direct endpoint for language tasks; falls back to `huniu_base_url`.
    pub language: Option<LanguageEndpoint>,
    pub volc_base_url: String,
    /// Huniu channel routing (for example `XUE`).
    pub huniu_strategy: Option<String>,
    pub keys: HashMap<Provider, Secret>,
    /// Provider binding for each public capability. Callers never choose a
    /// provider on a task; this table is the router's private adapter map.
    pub bindings: HashMap<Capability, Provider>,
    /// Default model binding for each capability. Empty task models are
    /// resolved at execution time so the planner's task key remains stable.
    pub models: HashMap<Capability, String>,
    pub journal_dir: PathBuf,
    pub artifacts_root: PathBuf,
    pub proxy: ProxyMode,
    pub limits: Limits,
    pub retry: RetryPolicy,
    pub breaker: BreakerPolicy,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WaitReason {
    /// Network or gateway trouble; the provider's circuit is open.
    Network,
    /// Backing off before attempt `attempt`.
    Retrying {
        attempt: u32,
        after: Duration,
        message: String,
    },
    /// The provider needs a key or balance.
    Attention,
    /// Another program sharing the task journal is working on it.
    Elsewhere,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attention {
    MissingKey,
    Auth,
    Billing,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AiEvent {
    Generating {
        key: TaskKey,
    },
    Downloading {
        key: TaskKey,
    },
    Waiting {
        key: TaskKey,
        reason: WaitReason,
    },
    Done {
        key: TaskKey,
        artifact: Option<ArtifactRef>,
        output: Option<serde_json::Value>,
    },
    Failed {
        key: TaskKey,
        class: String,
        message: String,
        /// A billed request with an unknown outcome; retried only on request.
        possibly_billed: bool,
    },
    Cancelled {
        key: TaskKey,
    },
    Models {
        id: u64,
        provider: Provider,
        models: Vec<crate::models::ModelInfo>,
    },
    ModelsFailed {
        id: u64,
        provider: Provider,
        class: String,
        message: String,
    },
    NeedsAttention {
        provider: Provider,
        attention: Attention,
    },
}

enum Command {
    Submit(TaskSpec),
    Cancel(TaskKey),
    Retry(TaskKey),
    SetKey(Provider, Option<Secret>),
    ListModels {
        id: u64,
        provider: Provider,
        capability: todo_production::Capability,
    },
}

/// Handle owned by the app. Dropping it stops the runtime; in-flight sends are
/// settled by the journal's startup recovery next time.
pub struct AiService {
    commands: tokio::sync::mpsc::UnboundedSender<Command>,
    events: mpsc::Receiver<AiEvent>,
    store: ArtifactStore,
    thread: Option<std::thread::JoinHandle<()>>,
    stop: Arc<Notify>,
    next_probe: AtomicU64,
}

struct Shared {
    settings: ServiceSettings,
    journal: Mutex<Journal>,
    store: ArtifactStore,
    http: Http,
    keys: Mutex<HashMap<Provider, Secret>>,
    breakers: Mutex<HashMap<Provider, Breaker>>,
    resumed: Notify,
    slots: HashMap<Capability, Semaphore>,
    next_image_send: tokio::sync::Mutex<Instant>,
    events: mpsc::Sender<AiEvent>,
    origin: Instant,
    seed: Mutex<u64>,
}

pub fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

impl AiService {
    pub fn start(settings: ServiceSettings) -> Result<(Self, Vec<Recovered>), String> {
        let (journal, recovered) = Journal::open(
            &settings.journal_dir,
            settings.limits.image_paid_retries,
            unix_millis(),
        )
        .map_err(|e| e.to_string())?;
        let http = Http::new(settings.proxy)?;
        let store = ArtifactStore::new(&settings.artifacts_root);
        let (event_tx, events) = mpsc::channel();
        let (commands, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let stop = Arc::new(Notify::new());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let slots = HashMap::from([
            (
                Capability::Language,
                Semaphore::new(settings.limits.chat_inflight),
            ),
            (
                Capability::Image,
                Semaphore::new(settings.limits.image_inflight),
            ),
            (
                Capability::Audio,
                Semaphore::new(settings.limits.speech_inflight),
            ),
            (Capability::Video, Semaphore::new(1)),
            (
                Capability::Multimodal,
                Semaphore::new(settings.limits.chat_inflight),
            ),
        ]);
        let shared = Arc::new(Shared {
            keys: Mutex::new(settings.keys.clone()),
            breakers: Mutex::new(HashMap::new()),
            journal: Mutex::new(journal),
            store: store.clone(),
            http,
            resumed: Notify::new(),
            slots,
            next_image_send: tokio::sync::Mutex::new(Instant::now()),
            events: event_tx,
            origin: Instant::now(),
            seed: Mutex::new(unix_millis() | 1),
            settings,
        });
        let stopping = stop.clone();
        let thread = std::thread::Builder::new()
            .name("todo-ai".into())
            .spawn(move || runtime.block_on(run_loop(shared, command_rx, stopping)))
            .map_err(|e| e.to_string())?;
        Ok((
            Self {
                commands,
                events,
                store,
                thread: Some(thread),
                stop,
                next_probe: AtomicU64::new(1),
            },
            recovered,
        ))
    }

    /// Run `spec` unless its result is known: a finished task answers from the
    /// journal immediately and costs nothing.
    pub fn submit(&self, spec: TaskSpec) -> TaskKey {
        let key = spec.key();
        let _ = self.commands.send(Command::Submit(spec));
        key
    }
    pub fn cancel(&self, key: &TaskKey) {
        let _ = self.commands.send(Command::Cancel(key.clone()));
    }
    /// Try a failed or possibly-billed task again (the user's decision).
    pub fn retry(&self, key: &TaskKey) {
        let _ = self.commands.send(Command::Retry(key.clone()));
    }
    /// A key was added or changed; paused work of the provider continues.
    pub fn set_key(&self, provider: Provider, key: Option<Secret>) {
        let _ = self.commands.send(Command::SetKey(provider, key));
    }
    /// Ask a provider for its current model catalog without touching the task
    /// journal or submitting a generation request.
    pub fn list_models(&self, provider: Provider, capability: todo_production::Capability) -> u64 {
        let id = self.next_probe.fetch_add(1, Ordering::Relaxed);
        let _ = self.commands.send(Command::ListModels {
            id,
            provider,
            capability,
        });
        id
    }
    pub fn poll(&self) -> Option<AiEvent> {
        self.events.try_recv().ok()
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Option<AiEvent> {
        self.events.recv_timeout(timeout).ok()
    }
    pub fn artifact_path(&self, artifact: &ArtifactRef) -> PathBuf {
        self.store.path(artifact)
    }
}

impl Drop for AiService {
    fn drop(&mut self) {
        self.stop.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

async fn run_loop(
    shared: Arc<Shared>,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<Command>,
    stop: Arc<Notify>,
) {
    let mut running: HashMap<TaskKey, tokio::task::JoinHandle<()>> = HashMap::new();
    loop {
        let command = tokio::select! {
            _ = stop.notified() => break,
            command = commands.recv() => match command {
                Some(command) => command,
                None => break,
            },
        };
        running.retain(|_, handle| !handle.is_finished());
        match command {
            Command::Submit(spec) => {
                let key = spec.key();
                if running.contains_key(&key) {
                    continue;
                }
                let created = {
                    let mut journal = shared.journal.lock().unwrap();
                    // Another process may have answered it since we started.
                    journal.refresh(&key);
                    let created = journal.ensure(&spec, unix_millis()).map(|_| ());
                    // 上次失败（未扣费）的记录：这次提交真的重试一次；
                    // 绝不在 run_task 里原地重投，否则会绕过退避与放弃逻辑。
                    if created.is_ok()
                        && journal
                            .get(&key)
                            .is_some_and(|record| record.state == crate::journal::TaskState::Failed)
                    {
                        let _ = journal.retry(&key, unix_millis());
                    }
                    created
                };
                if let Err(error) = created {
                    let _ = shared.events.send(AiEvent::Failed {
                        key,
                        class: "storage".into(),
                        message: error.to_string(),
                        possibly_billed: false,
                    });
                    continue;
                }
                running.insert(key.clone(), tokio::spawn(run_task(shared.clone(), key)));
            }
            Command::Retry(key) => {
                if running.contains_key(&key) {
                    continue;
                }
                let retried = {
                    let mut journal = shared.journal.lock().unwrap();
                    journal.refresh(&key);
                    journal.retry(&key, unix_millis()).map(|_| ())
                };
                if retried.is_ok() {
                    running.insert(key.clone(), tokio::spawn(run_task(shared.clone(), key)));
                }
            }
            Command::Cancel(key) => {
                if let Some(handle) = running.remove(&key) {
                    handle.abort();
                    let _ = handle.await;
                    let budget = shared.settings.limits.image_paid_retries;
                    let _ = shared.journal.lock().unwrap().settle_interrupted(
                        &key,
                        budget,
                        unix_millis(),
                    );
                    let _ = shared.events.send(AiEvent::Cancelled { key });
                }
            }
            Command::SetKey(provider, key) => {
                let mut keys = shared.keys.lock().unwrap();
                match key {
                    Some(key) => {
                        keys.insert(provider, key);
                    }
                    None => {
                        keys.remove(&provider);
                    }
                }
                drop(keys);
                if let Some(breaker) = shared.breakers.lock().unwrap().get_mut(&provider) {
                    breaker.resume();
                }
                shared.resumed.notify_waiters();
            }
            Command::ListModels {
                id,
                provider,
                capability,
            } => {
                tokio::spawn(list_models(shared.clone(), id, provider, capability));
            }
        }
    }
    for (_, handle) in running {
        handle.abort();
    }
}

async fn list_models(
    shared: Arc<Shared>,
    id: u64,
    provider: Provider,
    capability: todo_production::Capability,
) {
    // 语言能力可能挂在独立端点上：列模型要问那个端点。
    let language_endpoint = (capability == todo_production::Capability::Language)
        .then(|| shared.settings.language.as_ref())
        .flatten()
        .map(|endpoint| (endpoint.base_url.clone(), endpoint.key.clone()));
    let result = match provider {
        Provider::Huniu => {
            let key = shared.keys.lock().unwrap().get(&provider).cloned();
            match (language_endpoint, key) {
                (Some((base_url, key)), _) => {
                    crate::providers::huniu::list_models(&shared.http, &base_url, &key).await
                }
                (None, Some(key)) => {
                    crate::providers::huniu::list_models(
                        &shared.http,
                        &shared.settings.huniu_base_url,
                        &key,
                    )
                    .await
                }
                (None, None) => Err(crate::providers::AttemptError::new(
                    ErrorClass::Auth,
                    "还没有配置 API Key",
                )),
            }
        }
        Provider::Volc => Err(crate::providers::AttemptError::new(
            ErrorClass::Invalid,
            "火山 TTS 没有可枚举的模型目录",
        )),
    };
    match result {
        Ok(models) => shared.emit(AiEvent::Models {
            id,
            provider,
            models,
        }),
        Err(error) => shared.emit(AiEvent::ModelsFailed {
            id,
            provider,
            class: class_name(&error.class).into(),
            message: error.message,
        }),
    }
}

impl Shared {
    fn provider_for(&self, capability: Capability) -> Provider {
        self.settings
            .bindings
            .get(&capability)
            .copied()
            .unwrap_or(match capability {
                Capability::Audio => Provider::Volc,
                Capability::Language
                | Capability::Image
                | Capability::Video
                | Capability::Multimodal => Provider::Huniu,
            })
    }

    fn model_for(&self, capability: Capability, requested: &str) -> String {
        if !requested.trim().is_empty() {
            return requested.to_owned();
        }
        self.settings
            .models
            .get(&capability)
            .cloned()
            .unwrap_or_default()
    }

    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn unit(&self) -> f64 {
        // xorshift64*; only spreads retries, no security use.
        let mut seed = self.seed.lock().unwrap();
        *seed ^= *seed >> 12;
        *seed ^= *seed << 25;
        *seed ^= *seed >> 27;
        (seed.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn emit(&self, event: AiEvent) {
        let _ = self.events.send(event);
    }

    fn record(&self, key: &TaskKey) -> Option<TaskRecord> {
        self.journal.lock().unwrap().get(key).cloned()
    }

    fn update(&self, key: &TaskKey, change: impl FnOnce(&mut TaskRecord)) -> Option<TaskRecord> {
        self.journal
            .lock()
            .unwrap()
            .update(key, unix_millis(), change)
            .ok()
            .cloned()
    }

    fn permit(&self, provider: Provider) -> Permit {
        let now = self.now();
        self.breakers
            .lock()
            .unwrap()
            .entry(provider)
            .or_insert_with(|| Breaker::new(self.settings.breaker.clone()))
            .permit(now)
    }

    /// A free GET of the provider's base address: any HTTP answer, even an
    /// error status, means the network and the gateway are reachable.
    async fn reachable(&self, provider: Provider) -> bool {
        let url = match provider {
            Provider::Huniu => self.settings.huniu_base_url.clone(),
            Provider::Volc => self.settings.volc_base_url.clone(),
        };
        let sent = self
            .http
            .send(crate::net::http::Request {
                method: reqwest::Method::GET,
                url,
                headers: Vec::new(),
                body: None,
                head_timeout: Duration::from_secs(15),
                paid: false,
            })
            .await;
        match sent {
            Ok(_) => true,
            Err(failure) => failure.status.is_some(),
        }
    }

    /// Any answer, even an error status, proves the network works.
    fn answered(&self, provider: Provider) {
        if let Some(breaker) = self.breakers.lock().unwrap().get_mut(&provider) {
            breaker.success();
        }
    }

    fn report(&self, provider: Provider, class: &ErrorClass) {
        let now = self.now();
        let mut breakers = self.breakers.lock().unwrap();
        let breaker = breakers
            .entry(provider)
            .or_insert_with(|| Breaker::new(self.settings.breaker.clone()));
        match class {
            ErrorClass::Transient | ErrorClass::Ambiguous => breaker.failure(now),
            ErrorClass::Auth | ErrorClass::Billing => breaker.pause(),
            _ => breaker.success(),
        }
    }

    /// Wait for a key change or for `fallback`, whichever comes first.
    async fn wait_for_resume(&self, fallback: Duration) {
        let _ = tokio::time::timeout(fallback, self.resumed.notified()).await;
    }

    /// Keep image submissions at least `1 / rate` seconds apart: the gateway
    /// queues bursts (76 at once: median 356 s; 2/s: median ~45 s).
    async fn pace_image(&self) {
        let spacing = Duration::from_secs_f64(1.0 / self.settings.limits.image_rate_per_second);
        let mut next = self.next_image_send.lock().await;
        let now = Instant::now();
        if *next > now {
            tokio::time::sleep_until(*next).await;
        }
        *next = Instant::now().max(*next) + spacing;
    }
}

async fn run_task(shared: Arc<Shared>, key: TaskKey) {
    let started = Instant::now();
    let mut waiting_for_network = false;
    // Held while this process works on the task: another process sharing
    // the journal waits for the result instead of paying for it again.
    let mut claim = None;
    let mut waiting_elsewhere = false;
    loop {
        let Some(record) = shared.record(&key) else {
            return;
        };
        match record.state {
            TaskState::Done => {
                if let Some(artifact) = &record.artifact {
                    if !shared.store.contains(artifact) {
                        // The cache was cleared: produce it again.
                        shared.update(&key, |r| {
                            r.state = if r.remote_url.is_some() {
                                TaskState::Downloading
                            } else {
                                TaskState::Pending
                            };
                            r.artifact = None;
                        });
                        continue;
                    }
                }
                shared.emit(AiEvent::Done {
                    key,
                    artifact: record.artifact,
                    output: record.output,
                });
                return;
            }
            // 终态失败只上报一次；重试发生在"新的提交"那一侧（见 Submit 分支）。
            // 在这里原地重投会绕过退避与放弃逻辑，变成无限快速重发。
            TaskState::Failed => {
                shared.emit(AiEvent::Failed {
                    key,
                    class: record.error_class.unwrap_or_default(),
                    message: record.error.unwrap_or_default(),
                    possibly_billed: false,
                });
                return;
            }
            TaskState::Ambiguous => {
                shared.emit(AiEvent::Failed {
                    key,
                    class: record.error_class.unwrap_or_default(),
                    message: record.error.unwrap_or_default(),
                    possibly_billed: true,
                });
                return;
            }
            TaskState::Submitted => {
                // Remote async jobs (video) are not supported by these providers yet.
                shared.update(&key, |r| {
                    r.state = TaskState::Failed;
                    r.error_class = Some("invalid".into());
                    r.error = Some("remote job polling is not supported".into());
                });
                continue;
            }
            TaskState::Pending | TaskState::Downloading | TaskState::Submitting => {}
        }
        if claim.is_none() {
            let got = shared.journal.lock().unwrap().claim(&key);
            match got {
                Some(got) => {
                    claim = Some(got);
                    // It may have been finished, or left mid-send, meanwhile.
                    let mut journal = shared.journal.lock().unwrap();
                    journal.refresh(&key);
                    if journal.get(&key).map(|r| r.state) == Some(TaskState::Submitting) {
                        let budget = shared.settings.limits.image_paid_retries;
                        let _ = journal.settle_interrupted(&key, budget, unix_millis());
                    }
                }
                None => {
                    if !waiting_elsewhere {
                        waiting_elsewhere = true;
                        shared.emit(AiEvent::Waiting {
                            key: key.clone(),
                            reason: WaitReason::Elsewhere,
                        });
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    shared.journal.lock().unwrap().refresh(&key);
                }
            }
            continue;
        }

        let provider = shared.provider_for(record.spec.capability());
        let Some(secret) = shared.keys.lock().unwrap().get(&provider).cloned() else {
            shared.emit(AiEvent::NeedsAttention {
                provider,
                attention: Attention::MissingKey,
            });
            shared.emit(AiEvent::Waiting {
                key: key.clone(),
                reason: WaitReason::Attention,
            });
            shared.wait_for_resume(Duration::from_secs(60)).await;
            continue;
        };
        match shared.permit(provider) {
            Permit::Go => waiting_for_network = false,
            // The circuit is open: find out whether the network is back with
            // a free request, not with this task (an image can take minutes
            // and would hold every other task of the provider until then).
            Permit::Probe => {
                if shared.reachable(provider).await {
                    shared.answered(provider);
                    waiting_for_network = false;
                } else {
                    shared.report(provider, &ErrorClass::Transient);
                    continue;
                }
            }
            Permit::Paused => {
                shared.emit(AiEvent::Waiting {
                    key: key.clone(),
                    reason: WaitReason::Attention,
                });
                shared.wait_for_resume(Duration::from_secs(60)).await;
                continue;
            }
            Permit::Wait(after) => {
                if !waiting_for_network {
                    waiting_for_network = true;
                    shared.emit(AiEvent::Waiting {
                        key: key.clone(),
                        reason: WaitReason::Network,
                    });
                }
                tokio::time::sleep(after).await;
                continue;
            }
        }

        let capability = record.spec.capability();
        // Free (chat/speech) sends can storm when a gateway flaps; hard-stop
        // before another billed-looking round-trip. Images keep their own hedge.
        if !capability.is_paid_per_attempt() && record.submits >= 12 {
            shared.update(&key, |r| {
                r.state = TaskState::Failed;
                r.error_class = Some("transient".into());
                r.error = Some(format!(
                    "同一请求已发送 {} 次仍未成功，已停止（请检查模型名与端点是否匹配）",
                    r.submits
                ));
            });
            continue;
        }
        let _slot = shared.slots[&capability]
            .acquire()
            .await
            .expect("semaphores are never closed");
        let result = attempt(&shared, &key, &record, &secret).await;
        drop(_slot);
        let error = match result {
            Ok(outcome) => {
                shared.answered(provider);
                shared.update(&key, |r| {
                    r.state = TaskState::Done;
                    r.artifact = outcome.artifact.clone();
                    r.output = outcome.output.clone();
                    r.failures = 0;
                    r.error = None;
                    r.error_class = None;
                });
                continue;
            }
            Err(error) => error,
        };
        shared.report(provider, &error.class);
        let Some(record) = shared.record(&key) else {
            return;
        };
        let attempts = Attempts {
            failures: record.failures + 1,
            paid_retries: record.paid_retries,
        };
        let decision = shared.settings.retry.decide(
            &error.class,
            attempts,
            started.elapsed(),
            deadline(capability),
            shared.unit(),
        );
        let class_name = class_name(&error.class);
        let ambiguous = error.class == ErrorClass::Ambiguous;
        let resume_state = |r: &TaskRecord| {
            if r.state == TaskState::Downloading {
                TaskState::Downloading
            } else {
                TaskState::Pending
            }
        };
        match decision {
            Decision::Retry { after } => {
                shared.update(&key, |r| {
                    r.failures += 1;
                    r.paid_retries += u32::from(ambiguous);
                    r.state = resume_state(r);
                    r.error_class = Some(class_name.into());
                    r.error = Some(error.message.clone());
                });
                shared.emit(AiEvent::Waiting {
                    key: key.clone(),
                    reason: WaitReason::Retrying {
                        attempt: attempts.failures + 1,
                        after,
                        message: error.message,
                    },
                });
                tokio::time::sleep(after).await;
            }
            Decision::PauseProvider => {
                shared.update(&key, |r| {
                    r.failures += 1;
                    r.state = resume_state(r);
                    r.error_class = Some(class_name.into());
                    r.error = Some(error.message.clone());
                });
                let attention = if error.class == ErrorClass::Billing {
                    Attention::Billing
                } else {
                    Attention::Auth
                };
                shared.emit(AiEvent::NeedsAttention {
                    provider,
                    attention,
                });
                shared.emit(AiEvent::Waiting {
                    key: key.clone(),
                    reason: WaitReason::Attention,
                });
                shared.wait_for_resume(Duration::from_secs(60)).await;
            }
            Decision::GiveUp { possibly_billed } => {
                shared.update(&key, |r| {
                    r.failures += 1;
                    r.state = if possibly_billed {
                        TaskState::Ambiguous
                    } else {
                        TaskState::Failed
                    };
                    r.error_class = Some(class_name.into());
                    r.error = Some(error.message.clone());
                });
            }
        }
    }
}

fn class_name(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::Transient => "transient",
        ErrorClass::RateLimited { .. } => "rate_limited",
        ErrorClass::Auth => "auth",
        ErrorClass::Billing => "billing",
        ErrorClass::Invalid => "invalid",
        ErrorClass::ContentPolicy => "content_policy",
        ErrorClass::Ambiguous => "ambiguous",
    }
}

async fn attempt(
    shared: &Shared,
    key: &TaskKey,
    record: &TaskRecord,
    secret: &Secret,
) -> Result<Outcome, AttemptError> {
    let settings = &shared.settings;
    let owner = key.as_str();
    let provider = shared.provider_for(record.spec.capability());
    let request = bind_model(shared, &record.spec.request);
    match (&request, record.state, provider) {
        (TaskRequest::Image(_), TaskState::Downloading, Provider::Huniu) => {
            let url = record.remote_url.clone().unwrap_or_default();
            shared.emit(AiEvent::Downloading { key: key.clone() });
            huniu::download_image(
                &shared.http,
                &settings.huniu_base_url,
                secret,
                &url,
                &shared.store,
                owner,
            )
            .await
        }
        (TaskRequest::Image(request), _, Provider::Huniu) => {
            match submit_image_hedged(shared, key, record, secret, request).await? {
                huniu::ImageResult::Inline(bytes) => {
                    huniu::store_inline_image(&shared.store, owner, &bytes)
                }
                huniu::ImageResult::Url(url) => {
                    // Generated and billed: from now on only the download repeats.
                    shared.update(key, |r| {
                        r.state = TaskState::Downloading;
                        r.remote_url = Some(url.clone());
                    });
                    shared.emit(AiEvent::Downloading { key: key.clone() });
                    huniu::download_image(
                        &shared.http,
                        &settings.huniu_base_url,
                        secret,
                        &url,
                        &shared.store,
                        owner,
                    )
                    .await
                }
            }
        }
        (TaskRequest::Language(request), _, Provider::Huniu) => {
            begin_send(shared, key);
            match &settings.language {
                Some(language) => {
                    // Bound model was forced in bind_model; hit this origin only.
                    huniu::chat(&shared.http, &language.base_url, &language.key, request).await
                }
                None => {
                    huniu::chat(&shared.http, &settings.huniu_base_url, secret, request).await
                }
            }
        }
        (TaskRequest::Audio(request), _, Provider::Volc) => {
            begin_send(shared, key);
            volc::synthesize(
                &shared.http,
                &settings.volc_base_url,
                secret,
                request,
                &shared.store,
                owner,
            )
            .await
        }
        (TaskRequest::Video(_), _, _) => Err(AttemptError::new(
            ErrorClass::Invalid,
            "视频能力已接入路由，但当前没有配置视频适配器",
        )),
        (TaskRequest::Multimodal(request), _, Provider::Huniu) => {
            // Vision stays on huniu — never the dedicated language endpoint
            // (DeepSeek chat origin ≠ multimodal catalog).
            begin_send(shared, key);
            crate::providers::vision::vision(
                &shared.http,
                &settings.huniu_base_url,
                secret,
                request,
            )
            .await
        }
        (_, _, _) => Err(AttemptError::new(
            ErrorClass::Invalid,
            "能力与供应商适配器不匹配",
        )),
    }
}

/// Send an image request; while no answer is back after
/// `limits.image_resubmit_after`, send it again without dropping the earlier
/// sends, up to the paid budget. The first image back wins and the others are
/// dropped. Every extra send is billed, so it counts as a paid retry.
async fn submit_image_hedged(
    shared: &Shared,
    key: &TaskKey,
    record: &TaskRecord,
    secret: &Secret,
    request: &todo_production::ImageRequest,
) -> Result<huniu::ImageResult, AttemptError> {
    let settings = &shared.settings;
    let extra = settings
        .limits
        .image_paid_retries
        .saturating_sub(record.paid_retries);
    let mut sends = tokio::task::JoinSet::new();
    let launch = |sends: &mut tokio::task::JoinSet<_>| {
        let (http, base_url, secret, request, strategy) = (
            shared.http.clone(),
            settings.huniu_base_url.clone(),
            secret.clone(),
            request.clone(),
            settings.huniu_strategy.clone(),
        );
        sends.spawn(async move {
            huniu::submit_image(&http, &base_url, &secret, &request, strategy.as_deref()).await
        });
    };
    shared.pace_image().await;
    begin_send(shared, key);
    launch(&mut sends);
    let mut resends = 0;
    let mut resend_at = Instant::now() + settings.limits.image_resubmit_after;
    let (mut ambiguous, mut last) = (None, None);
    loop {
        tokio::select! {
            joined = sends.join_next() => match joined {
                Some(Ok(Ok(result))) => return Ok(result),
                Some(Ok(Err(error))) if error.class == ErrorClass::Ambiguous => {
                    ambiguous = Some(error)
                }
                Some(Ok(Err(error))) => last = Some(error),
                Some(Err(join)) => {
                    ambiguous = Some(AttemptError::new(ErrorClass::Ambiguous, join.to_string()))
                }
                None => {}
            },
            _ = tokio::time::sleep_until(resend_at), if resends < extra => {
                shared.pace_image().await;
                shared.update(key, |r| r.paid_retries += 1);
                begin_send(shared, key);
                launch(&mut sends);
                resends += 1;
                resend_at = Instant::now() + settings.limits.image_resubmit_after;
                continue;
            }
        }
        if sends.is_empty() {
            // Nothing in flight: an unknown outcome outranks a plain failure.
            return Err(ambiguous
                .or(last)
                .unwrap_or_else(|| AttemptError::new(ErrorClass::Transient, "no image request")));
        }
    }
}

fn bind_model(shared: &Shared, request: &TaskRequest) -> TaskRequest {
    let capability = request.capability();
    let model = |current: &str| shared.model_for(capability, current);
    match request {
        TaskRequest::Language(value) => {
            let mut value = value.clone();
            if let Some(language) = &shared.settings.language {
                // Dedicated chat origin owns the model id. Overwrite any
                // empty / huniu / node leftover so the wire never mixes families.
                value.model = language.model.clone();
            } else if value.model.is_empty() {
                value.model = model(&value.model);
            }
            TaskRequest::Language(value)
        }
        TaskRequest::Image(value) => {
            let mut value = value.clone();
            if value.model.is_empty() {
                value.model = model(&value.model);
            }
            TaskRequest::Image(value)
        }
        TaskRequest::Video(value) => {
            let mut value = value.clone();
            if value.model.is_empty() {
                value.model = model(&value.model);
            }
            TaskRequest::Video(value)
        }
        TaskRequest::Multimodal(value) => {
            let mut value = value.clone();
            // Always the multimodal binding (huniu), never language.endpoint.model.
            if value.model.is_empty() {
                value.model = model(&value.model);
            }
            TaskRequest::Multimodal(value)
        }
        TaskRequest::Audio(value) => {
            let mut value = value.clone();
            if value.model.is_empty() {
                value.model = model(&value.model);
            }
            TaskRequest::Audio(value)
        }
    }
}

/// Journal the send before it happens: a crash from here on is an unknown outcome.
fn begin_send(shared: &Shared, key: &TaskKey) {
    shared.update(key, |r| {
        r.state = TaskState::Submitting;
        r.submits += 1;
    });
    shared.emit(AiEvent::Generating { key: key.clone() });
}

/// Where the service keeps its data under a repository/application root.
pub fn default_paths(root: &Path) -> (PathBuf, PathBuf) {
    (root.join("cache/ai/tasks"), root.join("cache/ai/blobs"))
}
