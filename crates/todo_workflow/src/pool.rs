//! Reusable worker process pool for `todo-worker/1` extensions.
//!
//! The pool owns one manager thread. Child stdout/stderr are consumed by
//! `todo_process::Duplex`; the manager is the only place that mutates worker
//! state, which keeps run/call cancellation deterministic.
use crate::{worker_command, Budget, Extension, RuntimePaths};
use serde_json::Value;
use std::collections::{BTreeSet, VecDeque};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use todo_process::{Duplex, Job, Output};
use todo_production::items::{validate_items, Item};
use crate::workflow::manifest::Service;
use crate::workflow::protocol::{
    Canvas, HostMessage, RunError, RunErrorClass, ServiceCall, ServiceError, ServiceErrorClass,
    WorkerMessage, WORKER_SPEC,
};

pub type RunId = u64;

#[derive(Debug, Clone)]
pub struct RunRequest {
    pub extension: Arc<Extension>,
    pub op: String,
    pub input: Value,
    pub params: Value,
    pub canvas: Canvas,
    pub workdir: std::path::PathBuf,
    pub seed: u64,
    pub budget: Arc<Budget>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkerEvent {
    Progress {
        run: RunId,
        phase: String,
        fraction: f64,
    },
    Items {
        run: RunId,
        items: Vec<Item>,
    },
    Call {
        run: RunId,
        call: String,
        service: ServiceCall,
    },
    Done {
        run: RunId,
        result: Value,
    },
    Failed {
        run: RunId,
        error: RunError,
        stderr: String,
    },
}

#[derive(Debug, Clone)]
pub struct PoolOptions {
    pub runtime: RuntimePaths,
    pub hello_timeout: Duration,
    pub idle_timeout: Duration,
    pub cancel_grace: Duration,
    pub run_timeout: Option<Duration>,
}

impl Default for PoolOptions {
    fn default() -> Self {
        Self {
            runtime: RuntimePaths::new(std::env::current_dir().unwrap_or_default()),
            hello_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(120),
            cancel_grace: Duration::from_secs(5),
            run_timeout: None,
        }
    }
}

enum Command {
    Run(RunId, RunRequest),
    Reply {
        run: RunId,
        call: String,
        result: Result<Value, ServiceError>,
    },
    Cancel(RunId),
    Shutdown,
}

struct Inbox {
    worker: u64,
    output: Output,
}

struct Pending {
    id: RunId,
    request: RunRequest,
    retries: u8,
}

struct Active {
    id: RunId,
    request: RunRequest,
    retries: u8,
    emitted: BTreeSet<String>,
    calls: BTreeSet<String>,
    waiting_calls: BTreeSet<String>,
    started: Instant,
    cancel_deadline: Option<Instant>,
}

struct Process {
    id: u64,
    extension: Arc<Extension>,
    key: String,
    // Kept beside Duplex so the Job remains alive for the complete child
    // lifetime. Dropping it kills the child on Windows.
    _job: Job,
    duplex: Duplex,
    ready: bool,
    hello_deadline: Instant,
    last_activity: Instant,
    active: Option<Active>,
}

pub struct WorkerPool {
    commands: mpsc::Sender<Command>,
    next_run: std::sync::atomic::AtomicU64,
    thread: Option<JoinHandle<()>>,
}

/// Answers service calls from any thread (slow local services such as
/// `encode` reply from their own thread).
#[derive(Clone)]
pub struct PoolHandle {
    commands: mpsc::Sender<Command>,
}

impl PoolHandle {
    pub fn reply(&self, run: RunId, call: String, result: Result<Value, ServiceError>) {
        let _ = self.commands.send(Command::Reply { run, call, result });
    }
}

impl WorkerPool {
    pub fn handle(&self) -> PoolHandle {
        PoolHandle {
            commands: self.commands.clone(),
        }
    }

    pub fn start(options: PoolOptions, events: mpsc::Sender<WorkerEvent>) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (inbox_tx, inbox_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("todo-worker-pool".into())
            .spawn(move || Manager::new(options, events, inbox_tx, command_rx, inbox_rx).run())
            .expect("worker pool manager thread");
        Self {
            commands,
            next_run: std::sync::atomic::AtomicU64::new(0),
            thread: Some(thread),
        }
    }

    pub fn run(&self, request: RunRequest) -> RunId {
        let id = self
            .next_run
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .wrapping_add(1);
        let _ = self.commands.send(Command::Run(id, request));
        id
    }

    pub fn reply(&self, run: RunId, call: String, result: Result<Value, ServiceError>) {
        let _ = self.commands.send(Command::Reply { run, call, result });
    }

    pub fn cancel(&self, run: RunId) {
        let _ = self.commands.send(Command::Cancel(run));
    }

    pub fn shutdown(mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Manager {
    options: PoolOptions,
    events: mpsc::Sender<WorkerEvent>,
    inbox_tx: mpsc::Sender<Inbox>,
    inbox_rx: mpsc::Receiver<Inbox>,
    command_rx: mpsc::Receiver<Command>,
    pending: VecDeque<Pending>,
    processes: Vec<Process>,
    next_process: u64,
}

impl Manager {
    fn new(
        options: PoolOptions,
        events: mpsc::Sender<WorkerEvent>,
        inbox_tx: mpsc::Sender<Inbox>,
        command_rx: mpsc::Receiver<Command>,
        inbox_rx: mpsc::Receiver<Inbox>,
    ) -> Self {
        Self {
            options,
            events,
            inbox_tx,
            inbox_rx,
            command_rx,
            pending: VecDeque::new(),
            processes: Vec::new(),
            next_process: 0,
        }
    }

    fn run(mut self) {
        loop {
            while let Ok(command) = self.command_rx.try_recv() {
                if self.command(command) {
                    self.stop_all();
                    return;
                }
            }
            while let Ok(message) = self.inbox_rx.try_recv() {
                self.output(message);
            }
            self.expire();
            self.schedule();
            match self.command_rx.recv_timeout(Duration::from_millis(20)) {
                Ok(command) => {
                    if self.command(command) {
                        self.stop_all();
                        return;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.stop_all();
                    return;
                }
            }
        }
    }

    fn command(&mut self, command: Command) -> bool {
        match command {
            Command::Run(id, request) => self.pending.push_back(Pending {
                id,
                request,
                retries: 0,
            }),
            Command::Reply { run, call, result } => self.reply(run, &call, result),
            Command::Cancel(run) => self.cancel(run),
            Command::Shutdown => return true,
        }
        false
    }

    fn schedule(&mut self) {
        let mut index = 0;
        while index < self.pending.len() {
            let key = self.pending[index].request.extension.build_key();
            if let Some(worker_index) = self
                .processes
                .iter()
                .position(|worker| worker.ready && worker.active.is_none() && worker.key == key)
            {
                let pending = self.pending.remove(index).expect("pending index");
                self.start_run(worker_index, pending);
                continue;
            }
            let max = self.pending[index]
                .request
                .extension
                .manifest
                .worker
                .as_ref()
                .map_or(0, |worker| worker.max_processes.max(1)) as usize;
            let count = self
                .processes
                .iter()
                .filter(|worker| worker.key == key)
                .count();
            if count < max {
                let extension = self.pending[index].request.extension.clone();
                match self.spawn_process(extension) {
                    Ok(()) => {}
                    Err(error) => {
                        let pending = self.pending.remove(index).expect("pending index");
                        self.emit_failed(
                            pending.id,
                            RunError {
                                class: RunErrorClass::Resource,
                                message: error,
                            },
                            String::new(),
                        );
                    }
                }
                continue;
            }
            index += 1;
        }
    }

    fn spawn_process(&mut self, extension: Arc<Extension>) -> Result<(), String> {
        let worker = extension
            .manifest
            .worker
            .as_ref()
            .ok_or_else(|| "扩展没有 worker 配置".to_owned())?;
        let command =
            worker_command(&extension, &self.options.runtime).map_err(|e| e.to_string())?;
        let job = Job::new(Some(u64::from(worker.memory_mib) * 1024 * 1024))
            .map_err(|e| e.to_string())?;
        self.next_process = self.next_process.wrapping_add(1);
        let id = self.next_process;
        let sender = self.inbox_tx.clone();
        let duplex = Duplex::spawn_in(command, Some(&job), sender, move |output| Inbox {
            worker: id,
            output,
        })
        .map_err(|e| e.to_string())?;
        let now = Instant::now();
        let mut process = Process {
            id,
            key: extension.build_key(),
            extension,
            _job: job,
            duplex,
            ready: false,
            hello_deadline: now + self.options.hello_timeout,
            last_activity: now,
            active: None,
        };
        send(
            &mut process.duplex,
            HostMessage::Hello {
                spec: WORKER_SPEC.into(),
                host: env!("CARGO_PKG_VERSION").into(),
                package: process.extension.dir.to_string_lossy().into_owned(),
            },
        )?;
        self.processes.push(process);
        Ok(())
    }

    fn start_run(&mut self, index: usize, pending: Pending) {
        let Some(worker) = self.processes[index].extension.manifest.worker.as_ref() else {
            self.emit_failed(
                pending.id,
                RunError {
                    class: RunErrorClass::InvalidInput,
                    message: "扩展没有 worker 配置".into(),
                },
                String::new(),
            );
            return;
        };
        if !worker.ops.iter().any(|op| op == &pending.request.op) {
            self.emit_failed(
                pending.id,
                RunError {
                    class: RunErrorClass::InvalidInput,
                    message: format!("worker 不支持操作 {}", pending.request.op),
                },
                String::new(),
            );
            return;
        }
        let id = pending.id;
        let request = pending.request;
        let message = HostMessage::Run {
            id: id.to_string(),
            op: request.op.clone(),
            input: request.input.clone(),
            params: request.params.clone(),
            canvas: request.canvas.clone(),
            workdir: request.workdir.to_string_lossy().into_owned(),
            seed: request.seed,
        };
        let send_result = {
            let process = &mut self.processes[index];
            send(&mut process.duplex, message)
        };
        if let Err(error) = send_result {
            let stderr = self.processes[index].duplex.stderr_tail();
            self.emit_failed(
                id,
                RunError {
                    class: RunErrorClass::Resource,
                    message: error,
                },
                stderr,
            );
            return;
        }
        let process = &mut self.processes[index];
        process.last_activity = Instant::now();
        process.active = Some(Active {
            id,
            request,
            retries: pending.retries,
            emitted: BTreeSet::new(),
            calls: BTreeSet::new(),
            waiting_calls: BTreeSet::new(),
            started: Instant::now(),
            cancel_deadline: None,
        });
    }

    fn output(&mut self, inbox: Inbox) {
        let Some(index) = self
            .processes
            .iter()
            .position(|process| process.id == inbox.worker)
        else {
            return;
        };
        self.processes[index].last_activity = Instant::now();
        match inbox.output {
            Output::Line(line) => match serde_json::from_str::<WorkerMessage>(&line) {
                Ok(message) => self.message(index, message),
                Err(error) => self.fail_process(index, RunErrorClass::Resource, error.to_string()),
            },
            Output::Invalid(message) => self.fail_process(index, RunErrorClass::Resource, message),
            Output::Closed => {
                self.fail_process(index, RunErrorClass::Resource, "worker 已退出".into())
            }
        }
    }

    fn message(&mut self, index: usize, message: WorkerMessage) {
        match message {
            WorkerMessage::Ready { spec, ops, .. } => {
                let valid = spec == WORKER_SPEC
                    && self.processes[index]
                        .extension
                        .manifest
                        .worker
                        .as_ref()
                        .is_some_and(|worker| {
                            worker.ops.iter().all(|op| ops.iter().any(|got| got == op))
                        });
                if valid {
                    self.processes[index].ready = true;
                } else {
                    self.fail_process(index, RunErrorClass::Resource, "worker 握手不匹配".into());
                }
            }
            WorkerMessage::Progress {
                run,
                phase,
                fraction,
            } => {
                if let Some(active) = self.active(index, &run) {
                    let _ = self.events.send(WorkerEvent::Progress {
                        run: active.id,
                        phase,
                        fraction,
                    });
                }
            }
            WorkerMessage::Emit { run, items } => {
                let run_id = {
                    let Some(active) = self.active_mut(index, &run) else {
                        return;
                    };
                    if let Err(error) = validate_items(&items, &active.emitted, &|file| {
                        active.request.workdir.join(file).is_file()
                    }) {
                        self.fail_process(index, RunErrorClass::Internal, error);
                        return;
                    }
                    active
                        .emitted
                        .extend(items.iter().map(|item| item.key.clone()));
                    active.id
                };
                let _ = self.events.send(WorkerEvent::Items { run: run_id, items });
            }
            WorkerMessage::Call {
                id,
                run,
                service,
                request,
            } => self.call(index, &id, &run, service, request),
            WorkerMessage::Done { run, result } => {
                if let Some(active) = self.take_active(index, &run) {
                    let _ = self.events.send(WorkerEvent::Done {
                        run: active.id,
                        result,
                    });
                }
            }
            WorkerMessage::Failed { run, error } => {
                if error.class == RunErrorClass::Resource {
                    // A resource failure is retried in a fresh process, so the
                    // failed worker is removed before the request is queued.
                    self.fail_process(index, error.class, error.message);
                } else if let Some(active) = self.take_active(index, &run) {
                    self.emit_failed(active.id, error, self.processes[index].duplex.stderr_tail());
                }
            }
        }
    }

    fn call(&mut self, index: usize, call: &str, run: &str, service: Service, request: Value) {
        let duplicate = self
            .active_mut(index, run)
            .is_none_or(|active| !active.calls.insert(call.into()));
        if duplicate {
            self.fail_process(
                index,
                RunErrorClass::Internal,
                "call id 重复或 run 不存在".into(),
            );
            return;
        }
        if !self.processes[index]
            .extension
            .manifest
            .requires
            .services
            .contains(&service)
        {
            let _ = send_reply(
                &mut self.processes[index].duplex,
                call,
                Err(ServiceError {
                    class: ServiceErrorClass::Invalid,
                    message: format!("清单未声明服务 {service:?}"),
                }),
            );
            return;
        }
        let parsed = match ServiceCall::parse(service, &request) {
            Ok(parsed) => parsed,
            Err(error) => {
                let _ = send_reply(&mut self.processes[index].duplex, call, Err(error));
                return;
            }
        };
        let budget = self
            .active(index, run)
            .map(|active| active.request.budget.clone());
        let Some(budget) = budget else {
            return;
        };
        if let Err(error) = budget.charge(service, &request) {
            let _ = send_reply(&mut self.processes[index].duplex, call, Err(error));
            return;
        }
        let run_id = {
            let Some(active) = self.active_mut(index, run) else {
                return;
            };
            active.waiting_calls.insert(call.into());
            active.id
        };
        let _ = self.events.send(WorkerEvent::Call {
            run: run_id,
            call: call.into(),
            service: parsed,
        });
    }

    fn reply(&mut self, run: RunId, call: &str, result: Result<Value, ServiceError>) {
        for process in &mut self.processes {
            if process
                .active
                .as_ref()
                .is_some_and(|active| active.id == run)
            {
                if let Some(active) = process.active.as_mut() {
                    if !active.waiting_calls.remove(call) {
                        return;
                    }
                }
                let _ = send_reply(&mut process.duplex, call, result);
                return;
            }
        }
    }

    fn cancel(&mut self, run: RunId) {
        if let Some(index) = self.processes.iter().position(|process| {
            process
                .active
                .as_ref()
                .is_some_and(|active| active.id == run)
        }) {
            if let Some(active) = self.processes[index].active.as_mut() {
                active.cancel_deadline = Some(Instant::now() + self.options.cancel_grace);
            }
            let _ = send(
                &mut self.processes[index].duplex,
                HostMessage::Cancel {
                    id: run.to_string(),
                },
            );
            return;
        }
        if let Some(index) = self.pending.iter().position(|pending| pending.id == run) {
            self.pending.remove(index);
            self.emit_failed(
                run,
                RunError {
                    class: RunErrorClass::Cancelled,
                    message: "已取消".into(),
                },
                String::new(),
            );
        }
    }

    fn expire(&mut self) {
        let now = Instant::now();
        let mut remove = Vec::new();
        for index in 0..self.processes.len() {
            let process = &self.processes[index];
            if !process.ready && now >= process.hello_deadline {
                remove.push((index, RunErrorClass::Resource, "worker 握手超时".into()));
                continue;
            }
            if let Some(active) = process.active.as_ref() {
                if active
                    .cancel_deadline
                    .is_some_and(|deadline| now >= deadline)
                {
                    remove.push((
                        index,
                        RunErrorClass::Cancelled,
                        "取消超时，已终止 worker".into(),
                    ));
                } else if self.options.run_timeout.is_some_and(|timeout| {
                    now.duration_since(active.started) >= timeout && active.waiting_calls.is_empty()
                }) {
                    remove.push((index, RunErrorClass::Resource, "worker 运行超时".into()));
                }
            } else if process.ready
                && now.duration_since(process.last_activity) >= self.options.idle_timeout
            {
                remove.push((index, RunErrorClass::Resource, "worker 空闲回收".into()));
            }
        }
        for (index, class, message) in remove.into_iter().rev() {
            self.fail_process(index, class, message);
        }
    }

    fn fail_process(&mut self, index: usize, class: RunErrorClass, message: String) {
        if index >= self.processes.len() {
            return;
        }
        let mut process = self.processes.remove(index);
        if !process.ready {
            // The worker never started (missing runtime, crash on import,
            // no handshake). Queued runs for it are retried once in a fresh
            // process, then fail with the reason, instead of respawning forever.
            let stderr = process.duplex.stderr_tail();
            let key = process.key.clone();
            let mut failed = Vec::new();
            for pending in self.pending.iter_mut() {
                if pending.request.extension.build_key() == key {
                    if pending.retries >= 1 {
                        failed.push(pending.id);
                    } else {
                        pending.retries += 1;
                    }
                }
            }
            self.pending.retain(|p| !failed.contains(&p.id));
            for id in failed {
                self.emit_failed(
                    id,
                    RunError {
                        class: RunErrorClass::Resource,
                        message: format!("工作程序无法启动：{message}"),
                    },
                    stderr.clone(),
                );
            }
        }
        if let Some(active) = process.active.take() {
            self.finish_or_retry(active, RunError { class, message }, usize::MAX);
        }
    }

    fn finish_or_retry(&mut self, active: Active, error: RunError, _index: usize) {
        if error.class == RunErrorClass::Resource && active.retries < 1 {
            self.pending.push_back(Pending {
                id: active.id,
                request: active.request,
                retries: active.retries + 1,
            });
        } else {
            self.emit_failed(active.id, error, String::new());
        }
    }

    fn stop_all(&mut self) {
        for process in &mut self.processes {
            let _ = send(&mut process.duplex, HostMessage::Shutdown);
        }
        self.processes.clear();
        self.pending.clear();
    }

    fn active<'a>(&'a self, index: usize, run: &str) -> Option<&'a Active> {
        let id = run.parse::<RunId>().ok()?;
        self.processes[index]
            .active
            .as_ref()
            .filter(|active| active.id == id)
    }
    fn active_mut<'a>(&'a mut self, index: usize, run: &str) -> Option<&'a mut Active> {
        let id = run.parse::<RunId>().ok()?;
        self.processes[index]
            .active
            .as_mut()
            .filter(|active| active.id == id)
    }
    fn take_active(&mut self, index: usize, run: &str) -> Option<Active> {
        let id = run.parse::<RunId>().ok()?;
        if self.processes[index]
            .active
            .as_ref()
            .is_some_and(|active| active.id == id)
        {
            self.processes[index].active.take()
        } else {
            None
        }
    }
    fn emit_failed(&self, run: RunId, error: RunError, stderr: String) {
        let _ = self.events.send(WorkerEvent::Failed { run, error, stderr });
    }
}

fn send(duplex: &mut Duplex, message: HostMessage) -> Result<(), String> {
    duplex
        .send_line(&serde_json::to_string(&message).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())
}

fn send_reply(
    duplex: &mut Duplex,
    call: &str,
    result: Result<Value, ServiceError>,
) -> Result<(), String> {
    let message = match result {
        Ok(result) => HostMessage::Reply {
            to: call.into(),
            ok: true,
            result: Some(result),
            error: None,
        },
        Err(error) => HostMessage::Reply {
            to: call.into(),
            ok: false,
            result: None,
            error: Some(error),
        },
    };
    send(duplex, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn default_options_keep_the_documented_limits() {
        let options = PoolOptions::default();
        assert_eq!(options.hello_timeout, Duration::from_secs(30));
        assert_eq!(options.idle_timeout, Duration::from_secs(120));
        assert_eq!(options.cancel_grace, Duration::from_secs(5));
        let budget = Budget::new(&[(Service::Chat, 1)].into_iter().collect());
        budget.charge(Service::Chat, &json!({"x":1})).unwrap();
    }

    #[test]
    fn python_worker_handshakes_and_completes_a_run() {
        let python = python_path();
        let Some(python) = python else {
            eprintln!("skipped: Python is unavailable");
            return;
        };
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/worker-pool-tests")
            .join(format!("{}", unique()));
        let package = root.join("extension");
        let workdir = root.join("work");
        std::fs::create_dir_all(package.join("worker")).unwrap();
        std::fs::create_dir_all(&workdir).unwrap();
        let manifest = json!({
            "spec": "todo-extension/1",
            "id": "test.echo-worker",
            "name": "测试工作程序",
            "version": "1.0.0",
            "kind": "workflow",
            "requires": {"runtime": {"type": "python"}},
            "worker": {"entry": "worker/main.py", "ops": ["run"], "timeoutSec": 10, "memoryMiB": 1024}
        });
        std::fs::write(
            package.join("extension.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(
            package.join("worker/main.py"),
            r#"import json, sys
for line in sys.stdin:
    msg = json.loads(line)
    if msg["type"] == "hello":
        print(json.dumps({"type":"ready","spec":"todo-worker/1","ops":["run"],"version":"1"}), flush=True)
    elif msg["type"] == "run":
        print(json.dumps({"type":"progress","run":msg["id"],"phase":"完成","fraction":1.0}), flush=True)
        print(json.dumps({"type":"done","run":msg["id"],"result":{"ok":True}}), flush=True)
    elif msg["type"] == "shutdown":
        break
"#,
        )
        .unwrap();
        let extension = Arc::new(crate::registry::load(&package, crate::Origin::User).unwrap());
        let (events, receiver) = mpsc::channel();
        let options = PoolOptions {
            runtime: RuntimePaths {
                app_root: root.clone(),
                python_override: Some(python),
            },
            hello_timeout: Duration::from_secs(3),
            idle_timeout: Duration::from_secs(3),
            ..PoolOptions::default()
        };
        let pool = WorkerPool::start(options, events);
        let run = pool.run(RunRequest {
            extension,
            op: "run".into(),
            input: json!({"text":"hello"}),
            params: json!({}),
            canvas: Canvas {
                width: 320,
                height: 180,
                fps: 30.0,
            },
            workdir,
            seed: 7,
            budget: Arc::new(Budget::new(&Default::default())),
        });
        let mut done = false;
        for _ in 0..100 {
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(WorkerEvent::Done { run: id, result }) if id == run => {
                    assert_eq!(result["ok"], true);
                    done = true;
                    break;
                }
                Ok(WorkerEvent::Failed { run: id, error, .. }) if id == run => {
                    panic!("worker failed: {error:?}");
                }
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => panic!("event channel closed: {error}"),
            }
        }
        pool.shutdown();
        assert!(done, "worker did not finish");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn service_call_round_trip_is_replied_by_the_host_and_worker_is_reused() {
        let Some(python) = python_path() else {
            eprintln!("skipped: Python is unavailable");
            return;
        };
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/worker-pool-tests")
            .join(format!("call-{}", unique()));
        let package = root.join("extension");
        let workdir = root.join("work");
        std::fs::create_dir_all(package.join("worker")).unwrap();
        std::fs::create_dir_all(&workdir).unwrap();
        let manifest = json!({
            "spec": "todo-extension/1",
            "id": "test.call-worker",
            "name": "调用测试工作程序",
            "version": "1.0.0",
            "kind": "workflow",
            "requires": {"services": ["chat"], "runtime": {"type": "python"}},
            "worker": {"entry": "worker/main.py", "ops": ["run"], "timeoutSec": 10, "maxProcesses": 1},
            "budget": {"chat": 2}
        });
        std::fs::write(
            package.join("extension.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(
            package.join("worker/main.py"),
            r#"import json, sys
for line in sys.stdin:
    msg = json.loads(line)
    if msg["type"] == "hello":
        print(json.dumps({"type":"ready","spec":"todo-worker/1","ops":["run"],"version":"1"}), flush=True)
    elif msg["type"] == "run":
        call = "c-" + msg["id"]
        print(json.dumps({"type":"call","id":call,"run":msg["id"],"service":"chat","request":{"messages":[{"role":"user","content":"hello"}],"json":True}}), flush=True)
        for reply_line in sys.stdin:
            reply = json.loads(reply_line)
            if reply.get("type") == "reply" and reply.get("to") == call:
                print(json.dumps({"type":"done","run":msg["id"],"result":{"reply":reply.get("result")}}), flush=True)
                break
    elif msg["type"] == "shutdown":
        break
"#,
        )
        .unwrap();
        let extension = Arc::new(crate::registry::load(&package, crate::Origin::User).unwrap());
        let (events, receiver) = mpsc::channel();
        let pool = WorkerPool::start(
            PoolOptions {
                runtime: RuntimePaths {
                    app_root: root.clone(),
                    python_override: Some(python),
                },
                hello_timeout: Duration::from_secs(3),
                idle_timeout: Duration::from_secs(30),
                ..PoolOptions::default()
            },
            events,
        );
        let budget = Arc::new(Budget::new(&[(Service::Chat, 2)].into_iter().collect()));
        let request = |workdir: PathBuf| RunRequest {
            extension: extension.clone(),
            op: "run".into(),
            input: json!({}),
            params: json!({}),
            canvas: Canvas {
                width: 320,
                height: 180,
                fps: 30.0,
            },
            workdir,
            seed: 7,
            budget: budget.clone(),
        };
        let first = pool.run(request(workdir.clone()));
        let mut completed = BTreeSet::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while completed.is_empty() {
            assert!(Instant::now() < deadline, "call worker did not finish");
            let event = match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => event,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(error) => panic!("worker event channel closed: {error}"),
            };
            match event {
                WorkerEvent::Call { run, call, service } => {
                    assert_eq!(run, first);
                    assert!(matches!(service, ServiceCall::Chat(_)));
                    pool.reply(run, call, Ok(json!({"text":"world"})));
                }
                WorkerEvent::Done { run, result } => {
                    assert_eq!(result["reply"]["text"], "world");
                    completed.insert(run);
                }
                WorkerEvent::Failed { error, .. } => panic!("worker failed: {error:?}"),
                _ => {}
            }
        }
        // The same extension has maxProcesses=1; the second run must use the
        // already handshaken process and still receive a distinct call reply.
        let second = pool.run(request(workdir));
        while !completed.contains(&second) {
            assert!(Instant::now() < deadline + Duration::from_secs(10));
            let event = match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => event,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(error) => panic!("worker event channel closed: {error}"),
            };
            match event {
                WorkerEvent::Call { run, call, .. } => {
                    pool.reply(run, call, Ok(json!({"text":"again"})))
                }
                WorkerEvent::Done { run, result } => {
                    assert_eq!(result["reply"]["text"], "again");
                    completed.insert(run);
                }
                WorkerEvent::Failed { error, .. } => panic!("worker failed: {error:?}"),
                _ => {}
            }
        }
        pool.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cancelling_a_running_worker_terminates_it_with_cancelled() {
        let Some(python) = python_path() else {
            eprintln!("skipped: Python is unavailable");
            return;
        };
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/worker-pool-tests")
            .join(format!("cancel-{}", unique()));
        let package = root.join("extension");
        let workdir = root.join("work");
        std::fs::create_dir_all(package.join("worker")).unwrap();
        std::fs::create_dir_all(&workdir).unwrap();
        let manifest = json!({
            "spec": "todo-extension/1", "id": "test.cancel-worker", "name": "取消测试",
            "version": "1.0.0", "kind": "workflow", "requires": {"runtime": {"type": "python"}},
            "worker": {"entry": "worker/main.py", "ops": ["run"], "timeoutSec": 30}
        });
        std::fs::write(
            package.join("extension.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(
            package.join("worker/main.py"),
            r#"import json, sys, time
for line in sys.stdin:
    msg = json.loads(line)
    if msg["type"] == "hello":
        print(json.dumps({"type":"ready","spec":"todo-worker/1","ops":["run"],"version":"1"}), flush=True)
    elif msg["type"] == "run":
        print(json.dumps({"type":"progress","run":msg["id"],"phase":"长任务","fraction":0.1}), flush=True)
        time.sleep(30)
    elif msg["type"] == "shutdown":
        break
"#,
        )
        .unwrap();
        let extension = Arc::new(crate::registry::load(&package, crate::Origin::User).unwrap());
        let (events, receiver) = mpsc::channel();
        let pool = WorkerPool::start(
            PoolOptions {
                runtime: RuntimePaths {
                    app_root: root.clone(),
                    python_override: Some(python),
                },
                hello_timeout: Duration::from_secs(3),
                cancel_grace: Duration::from_millis(100),
                ..PoolOptions::default()
            },
            events,
        );
        let run = pool.run(RunRequest {
            extension,
            op: "run".into(),
            input: json!({}),
            params: json!({}),
            canvas: Canvas {
                width: 320,
                height: 180,
                fps: 30.0,
            },
            workdir,
            seed: 1,
            budget: Arc::new(Budget::new(&Default::default())),
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut cancelled = false;
        while !cancelled {
            assert!(
                Instant::now() < deadline,
                "cancelled worker did not terminate"
            );
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(WorkerEvent::Progress { run: id, .. }) if id == run => pool.cancel(run),
                Ok(WorkerEvent::Failed { run: id, error, .. }) if id == run => {
                    assert_eq!(error.class, RunErrorClass::Cancelled);
                    cancelled = true;
                }
                Ok(WorkerEvent::Done { run: id, .. }) if id == run => {
                    panic!("cancelled worker completed")
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => panic!("worker event channel closed: {error}"),
            }
        }
        pool.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn undeclared_service_is_rejected_without_emitting_a_host_call() {
        let Some(python) = python_path() else {
            eprintln!("skipped: Python is unavailable");
            return;
        };
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/worker-pool-tests")
            .join(format!("undeclared-{}", unique()));
        let package = root.join("extension");
        let workdir = root.join("work");
        std::fs::create_dir_all(package.join("worker")).unwrap();
        std::fs::create_dir_all(&workdir).unwrap();
        let manifest = json!({
            "spec": "todo-extension/1", "id": "test.undeclared-worker", "name": "未声明服务测试",
            "version": "1.0.0", "kind": "workflow",
            "requires": {"services": ["chat"], "runtime": {"type": "python"}},
            "worker": {"entry": "worker/main.py", "ops": ["run"], "timeoutSec": 10}
        });
        std::fs::write(
            package.join("extension.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(
            package.join("worker/main.py"),
            r#"import json, sys
for line in sys.stdin:
    msg = json.loads(line)
    if msg["type"] == "hello":
        print(json.dumps({"type":"ready","spec":"todo-worker/1","ops":["run"],"version":"1"}), flush=True)
    elif msg["type"] == "run":
        call = "bad-" + msg["id"]
        print(json.dumps({"type":"call","id":call,"run":msg["id"],"service":"image","request":{"prompt":"x","size":"64x64"}}), flush=True)
        for reply_line in sys.stdin:
            reply = json.loads(reply_line)
            if reply.get("type") == "reply" and reply.get("to") == call:
                print(json.dumps({"type":"done","run":msg["id"],"result":{"ok":reply.get("ok"),"error":reply.get("error")}}), flush=True)
                break
    elif msg["type"] == "shutdown":
        break
"#,
        )
        .unwrap();
        let extension = Arc::new(crate::registry::load(&package, crate::Origin::User).unwrap());
        let (events, receiver) = mpsc::channel();
        let pool = WorkerPool::start(
            PoolOptions {
                runtime: RuntimePaths {
                    app_root: root.clone(),
                    python_override: Some(python),
                },
                hello_timeout: Duration::from_secs(3),
                ..PoolOptions::default()
            },
            events,
        );
        let run = pool.run(RunRequest {
            extension,
            op: "run".into(),
            input: json!({}),
            params: json!({}),
            canvas: Canvas {
                width: 320,
                height: 180,
                fps: 30.0,
            },
            workdir,
            seed: 1,
            budget: Arc::new(Budget::new(&Default::default())),
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut done = false;
        while !done {
            assert!(
                Instant::now() < deadline,
                "undeclared service worker did not finish"
            );
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(WorkerEvent::Call { .. }) => panic!("undeclared service reached host"),
                Ok(WorkerEvent::Done { run: id, result }) if id == run => {
                    assert_eq!(result["ok"], false);
                    assert_eq!(result["error"]["class"], "invalid");
                    done = true;
                }
                Ok(WorkerEvent::Failed { error, .. }) => panic!("worker failed: {error:?}"),
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => panic!("worker event channel closed: {error}"),
            }
        }
        pool.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_worker_that_never_starts_fails_its_run_with_the_reason() {
        let Some(python) = python_path() else {
            eprintln!("skipped: Python is unavailable");
            return;
        };
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/worker-pool-tests")
            .join(format!("broken-{}", unique()));
        let package = root.join("extension");
        std::fs::create_dir_all(package.join("worker")).unwrap();
        std::fs::create_dir_all(root.join("work")).unwrap();
        let manifest = json!({
            "spec": "todo-extension/1", "id": "test.broken-worker", "name": "坏工作程序",
            "version": "1.0.0", "kind": "workflow", "requires": {"runtime": {"type": "python"}},
            "worker": {"entry": "worker/main.py", "ops": ["run"], "timeoutSec": 10}
        });
        std::fs::write(
            package.join("extension.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(
            package.join("worker/main.py"),
            "import missing_module_for_test\n",
        )
        .unwrap();
        let extension = Arc::new(crate::registry::load(&package, crate::Origin::User).unwrap());
        let (events, receiver) = mpsc::channel();
        let pool = WorkerPool::start(
            PoolOptions {
                runtime: RuntimePaths {
                    app_root: root.clone(),
                    python_override: Some(python),
                },
                hello_timeout: Duration::from_secs(5),
                ..PoolOptions::default()
            },
            events,
        );
        let run = pool.run(RunRequest {
            extension,
            op: "run".into(),
            input: json!({}),
            params: json!({}),
            canvas: Canvas {
                width: 320,
                height: 180,
                fps: 30.0,
            },
            workdir: root.join("work"),
            seed: 1,
            budget: Arc::new(Budget::new(&Default::default())),
        });
        let started = Instant::now();
        let failure = loop {
            match receiver.recv_timeout(Duration::from_secs(20)) {
                Ok(WorkerEvent::Failed {
                    run: id,
                    error,
                    stderr,
                }) if id == run => break (error, stderr),
                Ok(WorkerEvent::Done { .. }) => panic!("a broken worker cannot finish"),
                Ok(_) => {}
                Err(e) => panic!("no failure reported: {e}"),
            }
        };
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "fails promptly, after one retry"
        );
        assert!(failure.0.message.contains("无法启动"), "{:?}", failure.0);
        assert!(
            failure.1.contains("missing_module_for_test"),
            "stderr explains why: {}",
            failure.1
        );
        pool.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    fn python_path() -> Option<PathBuf> {
        let command = if cfg!(windows) { "where" } else { "which" };
        let output = std::process::Command::new(command)
            .arg("python")
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8(output.stdout)
            .ok()?
            .lines()
            .map(PathBuf::from)
            .find(|path| path.is_file())
    }

    fn unique() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
