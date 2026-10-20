//! Local JSONL bridge for editing agents.
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
use std::time::Duration;
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

#[derive(Debug)]
pub struct AgentBridge {
    port: u16,
    requests: Receiver<AgentRequest>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    endpoint: PathBuf,
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
        let endpoint = root.join("cache").join("agent-bridge.json");
        if let Some(parent) = endpoint.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建 endpoint 目录失败：{error}"))?;
        }
        let descriptor = json!({
            "apiVersion": API_VERSION,
            "transport": "tcp",
            "host": "127.0.0.1",
            "port": port,
            "protocol": "jsonl"
        });
        let _ = fs::remove_file(&endpoint);
        fs::write(&endpoint, serde_json::to_vec_pretty(&descriptor).unwrap())
            .map_err(|error| format!("写入 endpoint 文件失败：{error}"))?;

        let (request_tx, requests) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = thread::Builder::new()
            .name("todo-agent-bridge".into())
            .spawn(move || listener_loop(listener, request_tx, thread_stop))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            port,
            requests,
            stop,
            thread: Some(thread),
            endpoint,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn try_recv(&self) -> Option<AgentRequest> {
        self.requests.try_recv().ok()
    }

    pub fn endpoint_path(&self) -> &Path {
        &self.endpoint
    }
}

impl Drop for AgentBridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = fs::remove_file(&self.endpoint);
    }
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
    fn loopback_jsonl_request_reaches_the_runtime_queue() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/agent_bridge_tests")
            .join(format!("{}-{nonce}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let bridge = AgentBridge::start(&root).unwrap();
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
        drop(bridge);
        let _ = fs::remove_dir_all(root);
    }
}
