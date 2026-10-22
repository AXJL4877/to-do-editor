//! Offline extension self-checks used before enabling a package.
use crate::{worker_command, Extension, RuntimePaths};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;
use todo_process::{Duplex, Output};
use todo_production::items::validate_items;
use crate::workflow::protocol::{Canvas, HostMessage, WorkerMessage, WORKER_SPEC};

#[derive(Debug, Clone, PartialEq)]
pub struct CheckReport {
    pub id: String,
    pub version: String,
    pub ops: Vec<String>,
    pub ready: bool,
    pub fixture_runs: usize,
}

/// Quick check for enabling a package: manifest, runtime and handshake.
pub fn check_extension(ext: &Extension, paths: &RuntimePaths) -> Result<CheckReport, String> {
    check(ext, paths, None)
}

/// Full self-check: also runs `tests/input.json` twice in `workdir` (a
/// scratch directory; the package itself is never written) and requires
/// identical, valid output.
pub fn check_extension_fixtures(
    ext: &Extension,
    paths: &RuntimePaths,
    workdir: &Path,
) -> Result<CheckReport, String> {
    check(ext, paths, Some(workdir))
}

fn check(
    ext: &Extension,
    paths: &RuntimePaths,
    fixtures: Option<&Path>,
) -> Result<CheckReport, String> {
    if !paths
        .app_root
        .join("extensions/sdk/python/todo_worker.py")
        .is_file()
    {
        return Err(format!(
            "SDK not found at {}",
            paths
                .app_root
                .join("extensions/sdk/python/todo_worker.py")
                .display()
        ));
    }
    let worker = ext.manifest.worker.as_ref().ok_or("扩展没有 worker")?;
    let command = worker_command(ext, paths).map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::channel();
    let mut duplex = Duplex::spawn(command, tx, |output| output).map_err(|e| e.to_string())?;
    let hello = HostMessage::Hello {
        spec: WORKER_SPEC.into(),
        host: env!("CARGO_PKG_VERSION").into(),
        package: ext.dir.to_string_lossy().into_owned(),
    };
    duplex
        .send_line(&serde_json::to_string(&hello).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let line = rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "工作程序握手超时".to_string())?;
    let Output::Line(line) = line else {
        return Err(format!(
            "握手输出无效：{line:?}；stderr={}",
            duplex.stderr_tail()
        ));
    };
    let ready: WorkerMessage =
        serde_json::from_str(&line).map_err(|e| format!("ready 无效：{e}"))?;
    let WorkerMessage::Ready { spec, ops, version } = ready else {
        return Err("工作程序第一条消息不是 ready".into());
    };
    if spec != WORKER_SPEC {
        return Err(format!("工作程序 spec 不匹配：{spec}"));
    }
    if worker
        .ops
        .iter()
        .any(|op| !ops.iter().any(|ready_op| ready_op == op))
    {
        return Err("工作程序缺少清单声明的 op".into());
    }
    let fixture_runs = match fixtures {
        Some(workdir) => offline_fixtures(ext, &mut duplex, &rx, &ops, workdir)?,
        None => 0,
    };
    duplex.kill();
    Ok(CheckReport {
        id: ext.manifest.id.clone(),
        version,
        ops,
        ready: true,
        fixture_runs,
    })
}

fn offline_fixtures(
    ext: &Extension,
    duplex: &mut Duplex,
    rx: &mpsc::Receiver<Output>,
    ops: &[String],
    workdir: &Path,
) -> Result<usize, String> {
    let input = ext.dir.join("tests/input.json");
    if !input.is_file() {
        return Ok(0);
    }
    let value: Value = serde_json::from_slice(&std::fs::read(&input).map_err(|e| e.to_string())?)
        .map_err(|e| format!("input.json 无效：{e}"))?;
    let _ = std::fs::remove_dir_all(workdir);
    std::fs::create_dir_all(workdir).map_err(|e| e.to_string())?;
    let workdir = workdir.canonicalize().map_err(|e| e.to_string())?;
    if let Ok(entries) = std::fs::read_dir(ext.dir.join("tests")) {
        for entry in entries.flatten() {
            let source = entry.path();
            if source.is_file()
                && source.file_name().and_then(|name| name.to_str()) != Some("input.json")
            {
                let target = workdir.join(source.file_name().unwrap_or_default());
                std::fs::copy(&source, target).map_err(|e| e.to_string())?;
            }
        }
    }
    let op = if ops.iter().any(|op| op == "run") {
        "run"
    } else {
        ops.first().map(String::as_str).unwrap_or("")
    };
    if op.is_empty() {
        return Ok(0);
    }
    let first = run_fixture(duplex, rx, op, "check-1", &value, &workdir)?;
    let second = run_fixture(duplex, rx, op, "check-2", &value, &workdir)?;
    if first != second {
        return Err("同一离线样例两次运行结果不一致".into());
    }
    Ok(1)
}

#[derive(Debug, PartialEq)]
struct FixtureOutput {
    items: Vec<todo_production::items::Item>,
    result: Value,
}

fn run_fixture(
    duplex: &mut Duplex,
    rx: &mpsc::Receiver<Output>,
    op: &str,
    id: &str,
    input: &Value,
    workdir: &Path,
) -> Result<FixtureOutput, String> {
    let run = HostMessage::Run {
        id: id.into(),
        op: op.into(),
        input: input.clone(),
        params: Value::Object(Default::default()),
        canvas: Canvas {
            width: 1920,
            height: 1080,
            fps: 30.0,
        },
        workdir: workdir.to_string_lossy().into_owned(),
        seed: 1,
    };
    duplex
        .send_line(&serde_json::to_string(&run).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut earlier = BTreeSet::new();
    let mut items = Vec::new();
    loop {
        match rx
            .recv_timeout(Duration::from_secs(120))
            .map_err(|_| "离线样例超时".to_string())?
        {
            Output::Line(line) => {
                let message: WorkerMessage =
                    serde_json::from_str(&line).map_err(|e| format!("样例输出无效：{e}"))?;
                match message {
                    WorkerMessage::Emit { items: emitted, .. } => {
                        validate_items(&emitted, &earlier, &|file| workdir.join(file).is_file())
                            .map_err(|e| format!("条目无效：{e}"))?;
                        earlier.extend(emitted.iter().map(|item| item.key.clone()));
                        items.extend(emitted);
                    }
                    WorkerMessage::Done { result, .. } => {
                        // Parallel work arrives in any order: the same input
                        // must give the same items, not the same arrival order.
                        items.sort_by(|a, b| a.key.cmp(&b.key));
                        return Ok(FixtureOutput { items, result });
                    }
                    WorkerMessage::Failed { error, .. } => return Err(error.message),
                    _ => {}
                }
            }
            other => return Err(format!("样例输出中断：{other:?}")),
        }
    }
}

/// Load a package directory and run the full self-check in a scratch
/// directory under the app's cache.
pub fn load_and_check(path: &Path, app_root: &Path) -> Result<CheckReport, String> {
    let ext = crate::load(path, crate::Origin::User).map_err(|problems| {
        problems
            .into_iter()
            .map(|p| p.message)
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    let workdir = app_root
        .join("cache/extension-checks")
        .join(&ext.manifest.id);
    let report = check_extension_fixtures(&ext, &RuntimePaths::new(app_root.to_owned()), &workdir);
    let _ = std::fs::remove_dir_all(&workdir);
    report
}
