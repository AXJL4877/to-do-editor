//! Local JSONL bridge for editing agents.
//!
//! Discovery: short token `todo_<hex>` → `%LOCALAPPDATA%/TO_DO/bridges/<token>.json`
//! (plus `current.json`). Repo `cache/agent-bridge.json` is kept for smoke tests.
//!
//! The bridge keeps the workspace on the eframe thread. A loopback TCP listener
//! only parses requests and waits for the runtime to execute them, so an agent
//! never receives a second copy of project state that could drift from the UI.

use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use todo_application::control::ControlRequest;

const API_VERSION: u32 = 1;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug)]
pub enum AgentRequest {
    Control {
        request: ControlRequest,
        reply: Sender<Value>,
    },
    RenderPreview {
        request_id: String,
        session_id: u64,
        times: Vec<f64>,
        reply: Sender<Value>,
    },
}

/// Live project binding written into the token registry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BridgeBinding {
    pub session_id: Option<u64>,
    pub revision: Option<u64>,
    pub project_name: Option<String>,
    pub project_path: Option<String>,
}

#[derive(Debug)]
pub struct AgentBridge {
    port: u16,
    token: String,
    binding: BridgeBinding,
    requests: Receiver<AgentRequest>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// Repo-local discovery (backward compatible).
    endpoint: PathBuf,
    /// `%LOCALAPPDATA%/TO_DO/bridges/<token>.json`
    registry_file: PathBuf,
    current_file: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRequest {
    request_id: String,
    session_id: u64,
    #[serde(default)]
    expected_revision: Option<u64>,
    command: Value,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum BridgeCommand {
    RenderPreview {
        #[serde(default)]
        times: Vec<f64>,
    },
}

impl AgentBridge {
    pub fn start(root: &Path) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| format!("无法启动 Agent Bridge：{error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("无法配置 Agent Bridge：{error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("无法读取 Agent Bridge 地址：{error}"))?
            .port();

        let token = mint_token();
        let bridges = bridges_dir_for(root)?;
        let registry_file = bridges.join(format!("{token}.json"));
        let current_file = bridges.join("current.json");

        let endpoint = root.join("cache").join("agent-bridge.json");
        if let Some(parent) = endpoint.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建 endpoint 目录失败：{error}"))?;
        }

        let (request_tx, requests) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = thread::Builder::new()
            .name("todo-agent-bridge".into())
            .spawn(move || listener_loop(listener, request_tx, thread_stop))
            .map_err(|error| error.to_string())?;

        let bridge = Self {
            port,
            token,
            binding: BridgeBinding::default(),
            requests,
            stop,
            thread: Some(thread),
            endpoint,
            registry_file,
            current_file,
        };
        bridge.write_discovery()?;
        Ok(bridge)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Clipboard text: paste into Cursor / any agent to take over this editor.
    pub fn handoff_text(&self) -> String {
        handoff_instruction(&self.token, &self.binding)
    }

    pub fn try_recv(&self) -> Option<AgentRequest> {
        self.requests.try_recv().ok()
    }

    pub fn endpoint_path(&self) -> &Path {
        &self.endpoint
    }

    pub fn registry_path(&self) -> &Path {
        &self.registry_file
    }

    /// Refresh project identity on the token (no-op when unchanged).
    pub fn publish_binding(&mut self, binding: BridgeBinding) -> Result<(), String> {
        if self.binding == binding {
            return Ok(());
        }
        self.binding = binding;
        self.write_discovery()
    }

    fn write_discovery(&self) -> Result<(), String> {
        let issued = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let descriptor = json!({
            "apiVersion": API_VERSION,
            "token": self.token,
            "transport": "tcp",
            "host": "127.0.0.1",
            "port": self.port,
            "protocol": "jsonl",
            "sessionId": self.binding.session_id,
            "revision": self.binding.revision,
            "projectName": self.binding.project_name,
            "projectPath": self.binding.project_path,
            "issuedAt": issued,
        });
        let bytes = serde_json::to_vec_pretty(&descriptor)
            .map_err(|error| format!("序列化令牌失败：{error}"))?;
        fs::write(&self.registry_file, &bytes)
            .map_err(|error| format!("写入令牌登记失败：{error}"))?;
        fs::write(&self.current_file, &bytes)
            .map_err(|error| format!("写入 current 令牌失败：{error}"))?;
        let _ = fs::remove_file(&self.endpoint);
        fs::write(&self.endpoint, &bytes)
            .map_err(|error| format!("写入 endpoint 文件失败：{error}"))?;
        Ok(())
    }
}

impl Drop for AgentBridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = fs::remove_file(&self.endpoint);
        let _ = fs::remove_file(&self.registry_file);
        if let Ok(text) = fs::read_to_string(&self.current_file) {
            if text.contains(&self.token) {
                let _ = fs::remove_file(&self.current_file);
            }
        }
    }
}

/// `%LOCALAPPDATA%/TO_DO/bridges` (Windows) or `~/.local/share/TO_DO/bridges`.
/// Override with `TODO_BRIDGES_DIR` (tests / portable).
pub fn bridges_dir() -> Result<PathBuf, String> {
    if let Ok(raw) = std::env::var("TODO_BRIDGES_DIR") {
        let path = PathBuf::from(raw.trim());
        if !path.as_os_str().is_empty() {
            return Ok(path);
        }
    }
    let base = if let Ok(local) = std::env::var("LOCALAPPDATA") {
        PathBuf::from(local)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local").join("share")
    } else {
        return Err("无法定位 LOCALAPPDATA / HOME".into());
    };
    Ok(base.join("TO_DO").join("bridges"))
}

fn bridges_dir_for(root: &Path) -> Result<PathBuf, String> {
    let preferred = bridges_dir()?;
    match fs::create_dir_all(&preferred) {
        Ok(()) => Ok(preferred),
        Err(error) => {
            let fallback = root.join("cache").join("bridges");
            fs::create_dir_all(&fallback).map_err(|fallback_error| {
                format!("创建令牌目录失败：{error}；回退亦失败：{fallback_error}")
            })?;
            Ok(fallback)
        }
    }
}

/// Resolve a pasted `todo_*` token (or path / `current`) to discovery JSON.
pub fn resolve_token(token: &str) -> Result<Value, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("令牌为空".into());
    }
    let mut candidates = Vec::new();
    if let Ok(bridges) = bridges_dir() {
        candidates.push(bridges);
    }
    // Also try repo-local fallback used when LOCALAPPDATA is not writable.
    let root = std::env::var_os("TODO_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    candidates.push(root.join("cache").join("bridges"));

    let path = if token.eq_ignore_ascii_case("current") {
        candidates
            .iter()
            .map(|dir| dir.join("current.json"))
            .find(|p| p.is_file())
            .unwrap_or_else(|| {
                candidates
                    .first()
                    .cloned()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("current.json")
            })
    } else if token.starts_with("todo_") {
        candidates
            .iter()
            .map(|dir| dir.join(format!("{token}.json")))
            .find(|p| p.is_file())
            .unwrap_or_else(|| {
                candidates
                    .first()
                    .cloned()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(format!("{token}.json"))
            })
    } else {
        PathBuf::from(token)
    };
    let text = fs::read_to_string(&path).map_err(|error| {
        format!(
            "读不到令牌 {}：{error}\n请先启动 todo_app、打开工程，并复制工具栏「令牌」。",
            path.display()
        )
    })?;
    serde_json::from_str(&text).map_err(|error| format!("解析令牌失败：{error}"))
}

fn mint_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mix = nanos ^ ((std::process::id() as u128) << 32);
    format!("todo_{mix:x}")
}

/// Pasteable handoff for external agents (not a Cursor/todo-list id).
pub fn handoff_instruction(token: &str, binding: &BridgeBinding) -> String {
    let project = binding
        .project_name
        .as_deref()
        .unwrap_or("(未打开工程)");
    let session = binding
        .session_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| "-".into());
    format!(
        "TODO_BRIDGE {token}\n\
         用本机「TO DO」剪辑 App 的 Agent Bridge 接管当前工程（不是 Cursor 待办、不是 git todo）。\n\
         工程：{project}  sessionId={session}\n\
         硬规则：画面必须分层入轨（背景/元素/花字/配音各 clip）。禁止 import+place 整段合成视频或整图成片。\n\
         成片请 start_workflow（如 studio.spongebob-koubo）让扩展 emit 多条 todo-items，勿自烤成片再导入。\n\
         步骤：\n\
         1) 读 %LOCALAPPDATA%\\TO_DO\\bridges\\{token}.json（或 current.json）得到 host/port/sessionId\n\
         2) TCP 连 127.0.0.1:port，JSONL（docs/architecture/agent-bridge.md）\n\
         3) get_context → start_workflow 分层扩展 → 轮询入轨 → apply_edits / render_preview\n\
         4) cargo run -p todo_app --example bridge_smoke -- {token} get_context\n"
    )
}

fn listener_loop(listener: TcpListener, requests: Sender<AgentRequest>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                let requests = requests.clone();
                let _ = thread::Builder::new()
                    .name("todo-agent-client".into())
                    .spawn(move || serve_client(stream, requests));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
}

fn serve_client(stream: TcpStream, requests: Sender<AgentRequest>) {
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let reader = BufReader::new(cloned);
    let mut writer = stream;
    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => break,
        };
        let parsed = match parse_request(&line) {
            Ok(request) => request,
            Err(error) => {
                let response = error_response("", 0, None, "invalid_request", &error);
                let _ = write_response(&mut writer, &response);
                continue;
            }
        };
        let (reply_tx, reply_rx) = mpsc::channel();
        let request = match parsed {
            ParsedRequest::Control(request) => AgentRequest::Control {
                request,
                reply: reply_tx,
            },
            ParsedRequest::RenderPreview {
                request_id,
                session_id,
                times,
            } => AgentRequest::RenderPreview {
                request_id,
                session_id,
                times,
                reply: reply_tx,
            },
        };
        if requests.send(request).is_err() {
            break;
        }
        let response = reply_rx.recv_timeout(REQUEST_TIMEOUT).unwrap_or_else(|_| {
            error_response(
                "",
                0,
                None,
                "bridge_timeout",
                "Agent Bridge 没有在规定时间内完成请求",
            )
        });
        if write_response(&mut writer, &response).is_err() {
            break;
        }
    }
}

enum ParsedRequest {
    Control(ControlRequest),
    RenderPreview {
        request_id: String,
        session_id: u64,
        times: Vec<f64>,
    },
}

fn parse_request(line: &str) -> Result<ParsedRequest, String> {
    let raw: RawRequest = serde_json::from_str(line).map_err(|error| error.to_string())?;
    let command_type = raw
        .command
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "command.type is required".to_owned())?;
    if command_type == "render_preview" {
        let command: BridgeCommand =
            serde_json::from_value(raw.command).map_err(|error| error.to_string())?;
        let BridgeCommand::RenderPreview { times } = command;
        return Ok(ParsedRequest::RenderPreview {
            request_id: raw.request_id,
            session_id: raw.session_id,
            times,
        });
    }
    let command = serde_json::from_value(raw.command).map_err(|error| error.to_string())?;
    Ok(ParsedRequest::Control(ControlRequest {
        request_id: raw.request_id,
        session_id: raw.session_id,
        expected_revision: raw.expected_revision,
        command,
    }))
}

fn write_response(writer: &mut TcpStream, response: &Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *writer, response)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

pub fn ok_response(request_id: &str, session_id: u64, revision: u64, result: Value) -> Value {
    json!({
        "apiVersion": API_VERSION,
        "requestId": request_id,
        "sessionId": session_id,
        "revision": revision,
        "status": "ok",
        "result": result
    })
}

pub fn error_response(
    request_id: &str,
    session_id: u64,
    revision: Option<u64>,
    code: &str,
    message: &str,
) -> Value {
    let mut response = json!({
        "apiVersion": API_VERSION,
        "requestId": request_id,
        "sessionId": session_id,
        "status": "error",
        "error": {
            "code": code,
            "message": message
        }
    });
    if let Some(revision) = revision {
        response["revision"] = json!(revision);
    }
    response
}

pub fn frame_output_dir(root: &Path, request_id: &str) -> PathBuf {
    let safe: String = request_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .take(64)
        .collect();
    root.join("cache")
        .join("agent-frames")
        .join(if safe.is_empty() { "request" } else { &safe })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Write};
    use std::net::TcpStream;
    use std::time::Instant;
    use todo_application::control::ControlCommand;

    #[test]
    fn parses_control_and_render_requests() {
        let control =
            parse_request(r#"{"requestId":"ctx","sessionId":4,"command":{"type":"get_context"}}"#)
                .unwrap();
        assert!(matches!(
            control,
            ParsedRequest::Control(ControlRequest {
                command: ControlCommand::GetContext,
                ..
            })
        ));
        let render = parse_request(
            r#"{"requestId":"frame","sessionId":4,"command":{"type":"render_preview","times":[0,1.5]}}"#,
        )
        .unwrap();
        assert!(matches!(
            render,
            ParsedRequest::RenderPreview { times, .. } if times == vec![0.0, 1.5]
        ));
    }

    #[test]
    fn frame_output_path_drops_path_traversal_characters() {
        let path = frame_output_dir(Path::new("D:/repo"), "../review/one");
        assert_eq!(path, PathBuf::from("D:/repo/cache/agent-frames/reviewone"));
    }

    #[test]
    fn mint_token_has_prefix() {
        let token = mint_token();
        assert!(token.starts_with("todo_"), "{token}");
        assert!(token.len() > 8, "{token}");
    }

    #[test]
    fn handoff_text_is_bridge_not_todo_list() {
        let text = handoff_instruction(
            "todo_abc",
            &BridgeBinding {
                session_id: Some(3),
                revision: Some(1),
                project_name: Some("demo".into()),
                project_path: None,
            },
        );
        assert!(text.starts_with("TODO_BRIDGE todo_abc"));
        assert!(text.contains("Agent Bridge"));
        assert!(text.contains("不是 Cursor 待办"));
        assert!(text.contains("分层"));
        assert!(text.contains("禁止"));
    }

    #[test]
    fn loopback_jsonl_request_reaches_the_runtime_queue() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/agent_bridge_tests")
            .join(format!("{}-{nonce}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let bridges = root.join("bridges");
        fs::create_dir_all(&bridges).unwrap();
        std::env::set_var("TODO_BRIDGES_DIR", &bridges);
        let bridge = AgentBridge::start(&root).unwrap();
        assert!(bridge.token().starts_with("todo_"));
        assert!(bridge.registry_path().is_file());
        let discovery: Value =
            serde_json::from_str(&fs::read_to_string(bridge.registry_path()).unwrap()).unwrap();
        assert_eq!(discovery["token"], bridge.token());
        assert_eq!(discovery["port"], bridge.port());

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", bridge.port())) {
                Ok(stream) => break stream,
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("bridge did not accept a connection: {error}"),
            }
        };
        writeln!(
            stream,
            r#"{{"requestId":"ping","sessionId":1,"command":{{"type":"get_context"}}}}"#
        )
        .unwrap();
        let request = loop {
            if let Some(request) = bridge.try_recv() {
                break request;
            }
            assert!(Instant::now() < deadline, "bridge request was not queued");
            thread::sleep(Duration::from_millis(10));
        };
        let AgentRequest::Control { reply, .. } = request else {
            panic!("expected control request");
        };
        reply
            .send(ok_response("ping", 1, 0, json!({"type": "test"})))
            .unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap()["status"],
            "ok"
        );
        let registry = bridge.registry_path().to_path_buf();
        drop(bridge);
        assert!(!registry.is_file(), "registry cleaned on drop");
        let _ = fs::remove_dir_all(root);
    }
}
