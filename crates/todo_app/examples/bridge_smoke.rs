//! 真机 Agent Bridge 冒烟：用令牌或发现文件连上，JSONL 发命令并打印回包。
//!
//! 前置：本机已打开 `todo_app` 且工程已打开；工具栏点「令牌」可复制 `todo_…`。
//!
//! ```text
//! cargo run -p todo_app --example bridge_smoke
//! cargo run -p todo_app --example bridge_smoke -- todo_abc123 get_context
//! cargo run -p todo_app --example bridge_smoke -- current list_extensions
//! cargo run -p todo_app --example bridge_smoke -- list_extensions
//! cargo run -p todo_app --example bridge_smoke -- start_dry studio.spongebob-koubo
//! ```
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;
use todo_app::adapters::agent_bridge;

fn connect(host: &str, port: u16) -> TcpStream {
    let mut last = None;
    for _ in 0..50 {
        match TcpStream::connect((host, port)) {
            Ok(s) => {
                let _ = s.set_read_timeout(Some(Duration::from_secs(30)));
                let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
                return s;
            }
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    eprintln!("连接 {host}:{port} 失败：{last:?}");
    std::process::exit(1);
}

fn roundtrip(host: &str, port: u16, body: &Value) -> Value {
    let mut last_err = String::new();
    for attempt in 1..=8 {
        match try_roundtrip(host, port, body) {
            Ok(v) => return v,
            Err(e) => {
                last_err = e;
                eprintln!("# retry {attempt}/8: {last_err}");
                std::thread::sleep(Duration::from_millis(80 * attempt as u64));
            }
        }
    }
    eprintln!("roundtrip 失败：{last_err}");
    std::process::exit(1);
}

fn try_roundtrip(host: &str, port: u16, body: &Value) -> Result<Value, String> {
    let mut stream = connect(host, port);
    writeln!(stream, "{body}").map_err(|e| format!("write: {e}"))?;
    let mut line = String::new();
    BufReader::new(&stream)
        .read_line(&mut line)
        .map_err(|e| format!("read: {e}"))?;
    if line.trim().is_empty() {
        return Err("empty reply".into());
    }
    serde_json::from_str(line.trim()).map_err(|e| format!("json: {e}; line={line}"))
}

fn summarize(label: &str, reply: &Value) {
    let status = reply["status"].as_str().unwrap_or("?");
    let sid = reply["sessionId"].as_u64().unwrap_or(0);
    let rev = reply["revision"].as_u64().unwrap_or(0);
    let ty = reply["result"]["type"].as_str().unwrap_or("-");
    println!("# {label} status={status} sessionId={sid} revision={rev} result.type={ty}");
    if status != "ok" {
        println!(
            "  error={} — {}",
            reply["error"]["code"], reply["error"]["message"]
        );
    }
    match ty {
        "extensions" => {
            if let Some(arr) = reply["result"]["extensions"].as_array() {
                for ext in arr {
                    println!(
                        "  ext id={} name={}",
                        ext["id"].as_str().unwrap_or("?"),
                        ext["name"].as_str().unwrap_or("")
                    );
                }
            }
        }
        "workflow_validation" => {
            let v = &reply["result"]["validation"];
            println!(
                "  dryRun ok={} ext={} msg={}",
                v["ok"], v["extensionId"], v["message"]
            );
            if let Some(ps) = v["problems"].as_array() {
                for p in ps {
                    println!("  problem: {p}");
                }
            }
        }
        "context" => {
            let c = &reply["result"]["context"];
            println!(
                "  project={} media={} tracks={} busy={}",
                c["projectName"],
                c["project"]["media"].as_array().map(|a| a.len()).unwrap_or(0),
                c["project"]["tracks"].as_array().map(|a| a.len()).unwrap_or(0),
                c["busy"]
            );
        }
        "production_context" => {
            println!(
                "  {}",
                serde_json::to_string(&reply["result"]).unwrap_or_default()
            );
        }
        _ => {}
    }
}

fn load_discovery(token_hint: Option<&str>) -> Value {
    if let Some(token) = token_hint {
        return agent_bridge::resolve_token(token).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2);
        });
    }
    if let Ok(token) = std::env::var("TODO_AGENT_TOKEN") {
        return agent_bridge::resolve_token(&token).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2);
        });
    }
    let root = std::env::var_os("TODO_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let endpoint = root.join("cache/agent-bridge.json");
    let text = match std::fs::read_to_string(&endpoint) {
        Ok(text) => text,
        Err(error) => {
            eprintln!(
                "读不到 {}：{error}\n请先启动 todo_app 并打开工程；或传入令牌 todo_… / current。",
                endpoint.display()
            );
            std::process::exit(2);
        }
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("解析 bridge 发现文件失败：{e}\n{text}");
        std::process::exit(1);
    })
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let token_hint = args
        .first()
        .filter(|a| a.starts_with("todo_") || a.eq_ignore_ascii_case("current"))
        .cloned();
    if token_hint.is_some() {
        args.remove(0);
    }
    let discovery = load_discovery(token_hint.as_deref());
    let command = args
        .first()
        .cloned()
        .unwrap_or_else(|| "get_context".into());
    let extra = args.get(1).cloned();

    if let Some(token) = discovery["token"].as_str() {
        println!(
            "# token={token} project={} sessionId={} revision={}",
            discovery["projectName"], discovery["sessionId"], discovery["revision"]
        );
    }
    let host = discovery["host"].as_str().unwrap_or("127.0.0.1").to_owned();
    let port = discovery["port"].as_u64().unwrap_or(0) as u16;
    if port == 0 {
        eprintln!("发现文件缺 port：{discovery}");
        std::process::exit(1);
    }

    // Bootstrap with sessionId 0. New binaries return ok; older ones may
    // reply session_conflict but still stamp the live sessionId/revision.
    let bootstrap = roundtrip(
        &host,
        port,
        &json!({
            "requestId": "smoke-bootstrap",
            "sessionId": 0,
            "command": { "type": "get_context" }
        }),
    );
    summarize("bootstrap get_context", &bootstrap);
    let session_id = bootstrap["sessionId"].as_u64().unwrap_or(0);
    let revision = bootstrap["revision"].as_u64();
    if session_id == 0 {
        eprintln!("bootstrap 未带回有效 sessionId：{bootstrap}");
        std::process::exit(1);
    }
    if bootstrap["status"] != "ok" {
        let code = bootstrap["error"]["code"].as_str().unwrap_or("");
        if code != "session_conflict" {
            eprintln!(
                "bootstrap 失败：{} — {}",
                bootstrap["error"]["code"], bootstrap["error"]["message"]
            );
            std::process::exit(1);
        }
        eprintln!("# note: 旧二进制 get_context 仍校验 session；用回包 sessionId={session_id}");
    }

    let (label, cmd, need_rev) = match command.as_str() {
        "get_context" => (
            "get_context".into(),
            json!({ "type": "get_context" }),
            false,
        ),
        "list_extensions" => (
            "list_extensions".into(),
            json!({ "type": "list_extensions" }),
            false,
        ),
        "get_production_context" => (
            "get_production_context".into(),
            json!({ "type": "get_production_context" }),
            false,
        ),
        "start_dry" => {
            let ext = extra.unwrap_or_else(|| "studio.spongebob-koubo".into());
            (
                format!("start_workflow dryRun {ext}"),
                json!({
                    "type": "start_workflow",
                    "extensionId": ext,
                    "dryRun": true
                }),
                true,
            )
        }
        other => {
            eprintln!(
                "unknown {other:?}; try get_context | list_extensions | get_production_context | start_dry [ext]"
            );
            std::process::exit(2);
        }
    };

    let mut req = json!({
        "requestId": format!("smoke-{command}"),
        "sessionId": session_id,
        "command": cmd
    });
    if need_rev {
        if let Some(rev) = revision {
            req["expectedRevision"] = json!(rev);
        }
    }

    let reply = roundtrip(&host, port, &req);
    summarize(&label, &reply);
    if reply["status"] == "ok" {
        std::process::exit(0);
    }
    eprintln!(
        "命令失败：{} — {}",
        reply["error"]["code"], reply["error"]["message"]
    );
    std::process::exit(1);
}
