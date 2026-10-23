//! Agent Bridge / Control 工作流命令：与 UI `ProductionAction::WorkflowRun` 等同校验与启动路径。
//! 禁止旁路改 production.json；所有启动都经 `Workspace::bind_canvas_workflow` + `Host::start_workflow`。

use std::collections::BTreeMap;
use std::path::Path;

use todo_application::control::{
    ControlCommand, ControlError, ControlOutcome, ControlRequest, ControlResult, ControlStatus,
    ExtensionSummary, WorkflowStarted, WorkflowValidation, CONTROL_API_VERSION,
};
use todo_application::{AppUpdate, Workspace};
use todo_workflow::registry::Registry;
use todo_workflow::workflow::protocol::Canvas;

use super::host::ProductionHost;

/// Validate that an extension exists and its graph loads (dry-run of start).
pub fn validate_extension_start(
    registry: &Registry,
    extension_id: &str,
) -> Result<WorkflowValidation, ControlError> {
    let Some(extension) = registry.get(extension_id) else {
        let known: Vec<String> = registry
            .list()
            .map(|ext| ext.manifest.id.clone())
            .collect();
        let hint = if known.is_empty() {
            "先 list_extensions".into()
        } else {
            format!("可用：{}", known.join(", "))
        };
        // Common doc/stale alias: builtin.spongebob-koubo → studio.spongebob-koubo
        let alias_hint = if extension_id == "builtin.spongebob-koubo"
            && known.iter().any(|id| id == "studio.spongebob-koubo")
        {
            "；是否想用 studio.spongebob-koubo？"
        } else {
            ""
        };
        return Ok(WorkflowValidation {
            dry_run: true,
            extension_id: extension_id.into(),
            ok: false,
            problems: vec![format!("找不到扩展 {extension_id}")],
            message: format!("找不到扩展 {extension_id}；{hint}{alias_hint}"),
        });
    };
    if extension.manifest.graph.is_none() {
        return Ok(WorkflowValidation {
            dry_run: true,
            extension_id: extension_id.into(),
            ok: false,
            problems: vec!["扩展没有 graph.json / manifest.graph".into()],
            message: "该扩展不是可运行的工作流图包".into(),
        });
    }
    match extension.workflow() {
        Ok(Some(_)) => Ok(WorkflowValidation {
            dry_run: true,
            extension_id: extension_id.into(),
            ok: true,
            problems: Vec::new(),
            message: "校验通过：扩展与 graph 可加载（dry-run，未启动）".into(),
        }),
        Ok(None) => Ok(WorkflowValidation {
            dry_run: true,
            extension_id: extension_id.into(),
            ok: false,
            problems: vec!["manifest 声明了 graph 但未能加载".into()],
            message: "工作流图缺失".into(),
        }),
        Err(problems) => {
            let messages: Vec<String> = problems
                .iter()
                .map(|p| format!("{}: {}", p.path, p.message))
                .collect();
            Ok(WorkflowValidation {
                dry_run: true,
                extension_id: extension_id.into(),
                ok: false,
                problems: messages.clone(),
                message: messages.join("; "),
            })
        }
    }
}

/// Offline helper: scan builtin/user dirs without a live AI host.
pub fn validate_start_offline(
    root: &Path,
    extension_id: &str,
) -> Result<WorkflowValidation, ControlError> {
    let registry = Registry::scan(
        &root.join("extensions/builtin"),
        &root.join("extensions/user"),
    );
    validate_extension_start(&registry, extension_id)
}

pub fn list_extension_summaries(host: &ProductionHost) -> Vec<ExtensionSummary> {
    host.extension_summaries()
}

fn script_from_input(input: &Option<serde_json::Value>) -> String {
    match input {
        None => String::new(),
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(value) => value
            .get("script")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
    }
}

fn edits_from_params(
    params: &Option<BTreeMap<String, String>>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    // UI edits are node_id → {param → value}. Agent may pass flat "node.param" keys
    // or a single JSON object string under "_edits"; keep flat map as node "" for now
    // when no dotted keys — host `apply_workflow_edits` expects nested maps.
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let Some(params) = params else {
        return out;
    };
    for (key, value) in params {
        if let Some((node, param)) = key.split_once('.') {
            out.entry(node.to_owned())
                .or_default()
                .insert(param.to_owned(), value.clone());
        } else {
            out.entry(String::new())
                .or_default()
                .insert(key.clone(), value.clone());
        }
    }
    out
}

fn reply(
    request: &ControlRequest,
    workspace: &Workspace,
    result: Result<ControlResult, ControlError>,
    update: AppUpdate,
) -> ControlOutcome {
    ControlOutcome {
        update,
        response: todo_application::control::ControlResponse {
            api_version: CONTROL_API_VERSION,
            request_id: request.request_id.clone(),
            session_id: workspace.control_context().map(|c| c.session_id).unwrap_or(0),
            revision: workspace.control_context().map(|c| c.revision),
            outcome: match result {
                Ok(result) => ControlStatus::Ok { result },
                Err(error) => ControlStatus::Error { error },
            },
        },
    }
}

fn check_session(workspace: &Workspace, request: &ControlRequest) -> Result<(), ControlError> {
    if request.request_id.trim().is_empty() || request.request_id.len() > 128 {
        return Err(ControlError::new(
            "invalid_request",
            "requestId must contain 1..128 nonblank bytes",
        ));
    }
    let Some(ctx) = workspace.control_context() else {
        return Err(ControlError::new(
            "no_project",
            "Open a project before using workflow capabilities",
        ));
    };
    if request.session_id != ctx.session_id {
        return Err(ControlError::new(
            "session_conflict",
            "Read a new context before controlling this project session",
        ));
    }
    Ok(())
}

fn check_mutation(workspace: &Workspace, request: &ControlRequest) -> Result<(), ControlError> {
    check_session(workspace, request)?;
    let revision = request.expected_revision.ok_or_else(|| {
        ControlError::new("revision_required", "A mutation requires expectedRevision")
    })?;
    let ctx = workspace.control_context().unwrap();
    if ctx.revision != revision {
        return Err(ControlError::new(
            "revision_conflict",
            "Project content changed; read a new context and replan",
        ));
    }
    if ctx.busy {
        return Err(ControlError::new(
            "editor_busy",
            "Wait for the active gesture, import or project transition",
        ));
    }
    Ok(())
}

/// Execute a host-backed workflow control command (UI-equivalent).
pub fn execute(
    workspace: &mut Workspace,
    host: &mut ProductionHost,
    canvas: Canvas,
    request: ControlRequest,
) -> ControlOutcome {
    match &request.command {
        ControlCommand::ListExtensions => {
            if let Err(error) = check_session(workspace, &request) {
                return reply(&request, workspace, Err(error), AppUpdate::default());
            }
            let extensions = list_extension_summaries(host);
            reply(
                &request,
                workspace,
                Ok(ControlResult::Extensions { extensions }),
                AppUpdate::default(),
            )
        }
        ControlCommand::StartWorkflow {
            extension_id,
            input,
            params,
            dry_run,
        } => {
            if *dry_run {
                if let Err(error) = check_mutation(workspace, &request) {
                    return reply(&request, workspace, Err(error), AppUpdate::default());
                }
                let validation = match validate_extension_start(host.registry_ref(), extension_id)
                {
                    Ok(v) => v,
                    Err(error) => {
                        return reply(&request, workspace, Err(error), AppUpdate::default())
                    }
                };
                return reply(
                    &request,
                    workspace,
                    Ok(ControlResult::WorkflowValidation { validation }),
                    AppUpdate::default(),
                );
            }
            if let Err(error) = check_mutation(workspace, &request) {
                return reply(&request, workspace, Err(error), AppUpdate::default());
            }
            match start_workflow_like_ui(workspace, host, canvas, extension_id, input, params) {
                Ok((started, update)) => reply(
                    &request,
                    workspace,
                    Ok(ControlResult::WorkflowStarted { started }),
                    update,
                ),
                Err(error) => reply(&request, workspace, Err(error), AppUpdate::default()),
            }
        }
        ControlCommand::CancelWorkflow {
            extension_id,
            production_id,
            run_id,
        } => {
            if let Err(error) = check_mutation(workspace, &request) {
                return reply(&request, workspace, Err(error), AppUpdate::default());
            }
            let ext = resolve_extension_id(workspace, host, extension_id, production_id, *run_id);
            match ext {
                Ok(id) => match host.stop_workflow(&id) {
                    Ok(()) => reply(
                        &request,
                        workspace,
                        Ok(ControlResult::WorkflowCancelled { extension_id: id }),
                        AppUpdate::default(),
                    ),
                    Err(message) => reply(
                        &request,
                        workspace,
                        Err(ControlError::new("cancel_failed", &message)),
                        AppUpdate::default(),
                    ),
                },
                Err(error) => reply(&request, workspace, Err(error), AppUpdate::default()),
            }
        }
        ControlCommand::RetryWorkflow {
            production_id,
            extension_id: _,
            run_id: _,
            task_key,
        } => {
            if let Err(error) = check_mutation(workspace, &request) {
                return reply(&request, workspace, Err(error), AppUpdate::default());
            }
            let Some(pid) = production_id.clone().or_else(|| {
                workspace.production_ids().into_iter().next()
            }) else {
                return reply(
                    &request,
                    workspace,
                    Err(ControlError::new(
                        "production_not_found",
                        "没有可重试的成片；先 start_workflow 或指定 productionId",
                    )),
                    AppUpdate::default(),
                );
            };
            let key = task_key
                .as_ref()
                .map(|k| todo_application::TaskKey(k.clone()));
            let update = workspace.retry_production(&pid, key);
            let tasks = update
                .effects
                .iter()
                .filter(|e| matches!(e, todo_application::Effect::Production(_)))
                .count();
            reply(
                &request,
                workspace,
                Ok(ControlResult::WorkflowRetry {
                    production_id: pid,
                    tasks,
                }),
                update,
            )
        }
        ControlCommand::ImportExtension { path, dry_run } => {
            // dry-run only validates the archive; commit still needs revision.
            if *dry_run {
                if let Err(error) = check_session(workspace, &request) {
                    return reply(&request, workspace, Err(error), AppUpdate::default());
                }
            } else if let Err(error) = check_mutation(workspace, &request) {
                return reply(&request, workspace, Err(error), AppUpdate::default());
            }
            match host.import_extension(path, *dry_run) {
                Ok(extension) => {
                    let message = if *dry_run {
                        format!(
                            "校验通过：{} ({})，未写入磁盘",
                            extension.name, extension.id
                        )
                    } else {
                        format!(
                            "已安装 {} ({})；list_extensions 可见",
                            extension.name, extension.id
                        )
                    };
                    reply(
                        &request,
                        workspace,
                        Ok(ControlResult::ExtensionImported {
                            extension,
                            dry_run: *dry_run,
                            message,
                        }),
                        AppUpdate::default(),
                    )
                }
                Err(message) => reply(
                    &request,
                    workspace,
                    Err(ControlError::new("import_extension_failed", &message)),
                    AppUpdate::default(),
                ),
            }
        }
        other => reply(
            &request,
            workspace,
            Err(ControlError::new(
                "invalid_request",
                &format!("workflow_control 不处理 {:?}", command_type(other)),
            )),
            AppUpdate::default(),
        ),
    }
}

fn command_type(command: &ControlCommand) -> &'static str {
    match command {
        ControlCommand::GetContext => "get_context",
        ControlCommand::ListExtensions => "list_extensions",
        ControlCommand::StartWorkflow { .. } => "start_workflow",
        ControlCommand::CancelWorkflow { .. } => "cancel_workflow",
        ControlCommand::RetryWorkflow { .. } => "retry_workflow",
        ControlCommand::ImportExtension { .. } => "import_extension",
        _ => "other",
    }
}

fn resolve_extension_id(
    workspace: &Workspace,
    host: &ProductionHost,
    extension_id: &Option<String>,
    production_id: &Option<String>,
    _run_id: Option<u64>,
) -> Result<String, ControlError> {
    if let Some(id) = extension_id {
        return Ok(id.clone());
    }
    if let Some(pid) = production_id {
        if let Some(doc) = workspace.production(pid) {
            return Ok(doc.pack.id.clone());
        }
        return Err(ControlError::new(
            "production_not_found",
            &format!("没有成片 {pid}"),
        ));
    }
    if let Some(id) = host.live_workflow_extension() {
        return Ok(id);
    }
    Err(ControlError::new(
        "workflow_not_running",
        "请指定 extensionId，或先启动工作流",
    ))
}

fn start_workflow_like_ui(
    workspace: &mut Workspace,
    host: &mut ProductionHost,
    canvas: Canvas,
    extension_id: &str,
    input: &Option<serde_json::Value>,
    params: &Option<BTreeMap<String, String>>,
) -> Result<(WorkflowStarted, AppUpdate), ControlError> {
    let validation = validate_extension_start(host.registry_ref(), extension_id)?;
    if !validation.ok {
        return Err(ControlError::new("workflow_invalid", &validation.message));
    }
    let script = script_from_input(input);
    let (project, project_doc) = {
        let editor = workspace
            .editor()
            .ok_or_else(|| ControlError::new("project_required", "请先打开一个工程"))?;
        (editor.project_name().to_owned(), editor.project().clone())
    };
    let edits = edits_from_params(params);
    let version = host
        .extension_version(extension_id)
        .unwrap_or_else(|| "0".into());
    // Alloc is only a bind fallback; host always runs in productions/.../work.
    let (_alloc_abs, alloc_rel) = host
        .alloc_workflow_workdir(extension_id)
        .map_err(|e| ControlError::new("workdir_failed", &e))?;
    let run_id = workspace
        .bind_canvas_workflow(extension_id, &version, &script, alloc_rel, canvas)
        .map_err(|e| ControlError::new("bind_failed", &e))?;
    let production_id = workspace
        .production_status()
        .into_iter()
        .find(|s| s.extension == extension_id)
        .map(|s| s.id)
        .unwrap_or_default();
    let durable_rel = host
        .production_workdir(&project, &production_id)
        .map_err(|e| ControlError::new("workdir_failed", &e))?;
    let durable_abs = host
        .workdir_path(&project, &production_id)
        .map_err(|e| ControlError::new("workdir_failed", &e))?;
    std::fs::create_dir_all(&durable_abs)
        .map_err(|e| ControlError::new("workdir_failed", &e.to_string()))?;
    workspace
        .set_production_workdir(&production_id, durable_rel.clone())
        .map_err(|e| ControlError::new("workdir_failed", &e))?;
    // Place items kept across bind before the graph runs (cached graphs may
    // finish without re-emitting). Return adopt/place effects so the bridge
    // host applies them.
    let place = workspace.advance_productions();
    host.start_workflow(
        extension_id,
        script,
        &edits,
        run_id,
        durable_abs,
        Some(&project_doc),
    )
    .map_err(|e| ControlError::new("start_failed", &e))?;
    let log_path = host
        .workflow_log_path(extension_id)
        .map(|p| p.to_string_lossy().into_owned());
    Ok((
        WorkflowStarted {
            extension_id: extension_id.into(),
            production_id,
            run_id,
            workdir: durable_rel,
            log_path,
        },
        place,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_application::control::ControlCommand;

    #[test]
    fn dry_run_rejects_unknown_extension_offline() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let result = validate_start_offline(&root, "no.such.extension.zzz").unwrap();
        assert!(!result.ok);
        assert!(result.message.contains("找不到扩展"));
    }

    #[test]
    fn dry_run_accepts_a_builtin_graph_when_present() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let registry = Registry::scan(
            &root.join("extensions/builtin"),
            &root.join("extensions/user"),
        );
        let Some(ext) = registry.list().find(|e| e.manifest.graph.is_some()) else {
            // No graph packs in this checkout — still prove the validator path.
            let result = validate_extension_start(&registry, "missing").unwrap();
            assert!(!result.ok);
            return;
        };
        let result = validate_extension_start(&registry, &ext.manifest.id).unwrap();
        assert!(
            result.ok,
            "expected ok for {}: {}",
            ext.manifest.id, result.message
        );
        assert!(result.dry_run);
    }

    #[test]
    fn import_extension_rejects_illegal_package_via_host() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/todo-host-import-bad");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("extensions/user")).expect("mkdir user");
        std::fs::create_dir_all(root.join("extensions/builtin")).expect("mkdir builtin");
        let path = root.join("bad.todoext");
        std::fs::write(&path, b"not-a-zip").expect("write bad pack");
        let mut host = ProductionHost::start(&root).expect("host");
        let err = host.import_extension(&path, true).unwrap_err();
        assert!(
            err.contains("扩展包")
                || err.contains("无法读取")
                || err.contains("zip")
                || err.contains("Archive"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_extension_installs_and_lists_via_host() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let builtin = repo.join("extensions/builtin");
        let Ok(entries) = std::fs::read_dir(&builtin) else {
            return;
        };
        let Some(src) = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.join("extension.json").is_file())
        else {
            return;
        };
        let bytes = match todo_workflow::pack(&src) {
            Ok(b) => b,
            Err(_) => return,
        };
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/todo-host-import-ok");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("extensions/user")).expect("mkdir user");
        std::fs::create_dir_all(root.join("extensions/builtin")).expect("mkdir builtin");
        let path = root.join("pack.todoext");
        std::fs::write(&path, &bytes).expect("write pack");
        let mut host = ProductionHost::start(&root).expect("host");
        let dry = host.import_extension(&path, true).expect("dry-run");
        assert_eq!(dry.status, "validated");
        let installed = host.import_extension(&path, false).expect("install");
        assert_eq!(installed.origin, "user");
        let listed = host.extension_summaries();
        assert!(
            listed.iter().any(|e| e.id == installed.id),
            "list_extensions missing {}: {:?}",
            installed.id,
            listed.iter().map(|e| e.id.as_str()).collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_extension_command_deserializes() {
        let value = serde_json::json!({
            "type": "import_extension",
            "path": "C:/tmp/demo.todoext",
            "dryRun": true
        });
        let command: ControlCommand = serde_json::from_value(value).unwrap();
        assert!(matches!(
            command,
            ControlCommand::ImportExtension { dry_run: true, .. }
        ));
        assert!(command.needs_production_host());
    }

    #[test]
    fn start_workflow_command_deserializes() {
        let value = serde_json::json!({
            "type": "start_workflow",
            "extensionId": "studio.demo",
            "dryRun": true,
            "input": { "script": "hello" }
        });
        let command: ControlCommand = serde_json::from_value(value).unwrap();
        assert!(matches!(
            command,
            ControlCommand::StartWorkflow {
                dry_run: true,
                ..
            }
        ));
        assert!(command.needs_production_host());
    }

    /// Same path as Agent Bridge → runtime::control_workflow (no GUI).
    #[test]
    fn bridge_equivalent_list_import_and_start_dry_run() {
        use todo_application::control::{
            ControlRequest, ControlResult, ControlStatus, CONTROL_API_VERSION,
        };
        use todo_core::create_project;

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/todo-bridge-eq");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("extensions/user")).expect("user");
        // Point builtin at repo builtins so list/start dry-run sees real packs.
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let builtin_src = repo.join("extensions/builtin");
        let builtin_dst = root.join("extensions/builtin");
        if builtin_src.is_dir() {
            // Shallow copy first-level packages only (enough for list/validate).
            std::fs::create_dir_all(&builtin_dst).ok();
            if let Ok(entries) = std::fs::read_dir(&builtin_src) {
                for entry in entries.flatten() {
                    let from = entry.path();
                    if !from.is_dir() {
                        continue;
                    }
                    let to = builtin_dst.join(entry.file_name());
                    let _ = std::fs::create_dir_all(&to);
                    if let Ok(files) = std::fs::read_dir(&from) {
                        for f in files.flatten() {
                            let p = f.path();
                            if p.is_file() {
                                let _ = std::fs::copy(&p, to.join(f.file_name()));
                            }
                        }
                    }
                }
            }
        } else {
            std::fs::create_dir_all(&builtin_dst).ok();
        }

        let mut host = ProductionHost::start(&root).expect("host");
        let mut ws = Workspace::with_project(create_project(None, None, None, None, None), "p");
        let canvas = Canvas {
            width: 1920,
            height: 1080,
            fps: 30.0,
        };
        let ctx = ws.control_context().expect("open project");

        let list = execute(
            &mut ws,
            &mut host,
            canvas.clone(),
            ControlRequest {
                request_id: "list-1".into(),
                session_id: ctx.session_id,
                expected_revision: None,
                command: ControlCommand::ListExtensions,
            },
        );
        assert_eq!(list.response.api_version, CONTROL_API_VERSION);
        match list.response.outcome {
            ControlStatus::Ok {
                result: ControlResult::Extensions { extensions },
            } => {
                // Shallow copy may omit nested graphs; empty list is ok — the
                // command must still succeed (Bridge-equivalent ok path).
                let _ = extensions;
            }
            other => panic!("list_extensions failed: {other:?}"),
        }

        let bad = root.join("bad.todoext");
        std::fs::write(&bad, b"not-a-zip").unwrap();
        let import = execute(
            &mut ws,
            &mut host,
            canvas.clone(),
            ControlRequest {
                request_id: "imp-1".into(),
                session_id: ctx.session_id,
                expected_revision: Some(ctx.revision),
                command: ControlCommand::ImportExtension {
                    path: bad,
                    dry_run: true,
                },
            },
        );
        match import.response.outcome {
            ControlStatus::Error { error } => {
                assert_eq!(error.code, "import_extension_failed");
                assert!(!error.message.is_empty());
            }
            other => panic!("expected import failure, got {other:?}"),
        }

        let ctx = ws.control_context().unwrap();
        let start = execute(
            &mut ws,
            &mut host,
            canvas,
            ControlRequest {
                request_id: "start-1".into(),
                session_id: ctx.session_id,
                expected_revision: Some(ctx.revision),
                command: ControlCommand::StartWorkflow {
                    extension_id: "no.such.extension".into(),
                    input: None,
                    params: None,
                    dry_run: true,
                },
            },
        );
        match start.response.outcome {
            ControlStatus::Ok {
                result: ControlResult::WorkflowValidation { validation },
            } => {
                assert!(!validation.ok);
                assert!(!validation.message.is_empty());
            }
            other => panic!("expected dry-run validation, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    /// JSONL Agent Bridge → queue → `execute` → reply（与 runtime::poll_agent_bridge 同源）。
    #[test]
    fn jsonl_bridge_list_extensions_end_to_end() {
        use crate::adapters::agent_bridge::{AgentBridge, AgentRequest};
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpStream;
        use std::thread;
        use std::time::{Duration, Instant};
        use todo_core::create_project;

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/todo-jsonl-e2e");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("extensions/user")).unwrap();
        std::fs::create_dir_all(root.join("extensions/builtin")).unwrap();

        let bridge = AgentBridge::start(&root).expect("bridge");
        let mut host = ProductionHost::start(&root).expect("host");
        let mut ws = Workspace::with_project(create_project(None, None, None, None, None), "p");
        let canvas = Canvas {
            width: 1920,
            height: 1080,
            fps: 30.0,
        };
        let session_id = ws.control_context().unwrap().session_id;
        let port = bridge.port();

        let client = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match TcpStream::connect(("127.0.0.1", port)) {
                    Ok(s) => break s,
                    Err(_) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("connect: {e}"),
                }
            };
            writeln!(
                stream,
                r#"{{"requestId":"jsonl-list","sessionId":{session_id},"command":{{"type":"list_extensions"}}}}"#
            )
            .unwrap();
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).unwrap();
            line
        });

        let deadline = Instant::now() + Duration::from_secs(3);
        let request = loop {
            if let Some(request) = bridge.try_recv() {
                break request;
            }
            assert!(Instant::now() < deadline, "bridge did not queue request");
            thread::sleep(Duration::from_millis(10));
        };
        let AgentRequest::Control { request, reply } = request else {
            panic!("expected control request");
        };
        assert!(matches!(
            request.command,
            ControlCommand::ListExtensions
        ));
        let outcome = execute(&mut ws, &mut host, canvas, request);
        reply
            .send(serde_json::to_value(&outcome.response).expect("serialize"))
            .unwrap();

        let line = client.join().expect("client");
        let value: serde_json::Value = serde_json::from_str(line.trim()).expect("json");
        assert_eq!(value["status"], "ok", "{value}");
        assert_eq!(value["result"]["type"], "extensions", "{value}");

        drop(bridge);
        let _ = std::fs::remove_dir_all(&root);
    }
}
