//! Built-in Agent as a tool loop: the model picks Control / preview tools;
//! the desktop executes them and feeds results back until `done`.
use serde_json::{json, Value};
use todo_application::control::{
    ControlCommand, ControlEdit, ControlRequest, ControlStatus,
};
use todo_production::{ChatMessage, ChatRole, LanguageRequest, TaskRequest};

pub const MAX_TOOL_ROUNDS: u32 = 12;
const TOOL_REQ_PREFIX: &str = "tool-agent-";

pub fn is_tool_request(request_id: &str) -> bool {
    request_id.starts_with(TOOL_REQ_PREFIX)
}

pub fn next_request_id(round: u32) -> String {
    format!("{TOOL_REQ_PREFIX}{round}")
}

pub fn system_prompt() -> String {
    r#"你是成片编辑 Agent。你只能通过 JSON 调用工具，不要直接改工程文件。

每次回复必须是一个 JSON 对象，二选一：
1) 调用工具：
{"thought":"简短理由","tool":"<工具名>","args":{...}}
2) 结束并对用户说话：
{"done":true,"reply":"给用户看的中文结论"}

可用工具（只能用这些名字）：
- get_context：无参数。读工程 session/revision/媒体与轨道摘要。
- describe：可选 {"at": 秒}。元素级几何快照。
- apply_edits：{"edits":[...ControlEdit],"dryRun":false}。安全编辑时间轴。
- undo / redo：无参数。
- import_media：仅登记**未合成**的单层素材（透明元素图、底板图、音轨等）；禁止导入「背景+元素烤在一起」的成片视频/整图再 place。
- get_import_job：{"jobId": 数字}。查询导入任务状态。
- list_extensions：无参数。列出可运行的扩展包。
- import_extension：{"path":".todoext 路径","dryRun":false}。安装扩展包。
- get_production_context：可选 {"productionId":"..."}。入轨/运行态。
- get_workflow_run：可选 {"runId":数字,"productionId":"..."}。
- start_workflow：{"extensionId":"...","input":{},"params":{},"dryRun":false}。成片主路径；扩展须 emit 多条分层 todo-items（如 studio.spongebob-koubo）。
- cancel_workflow / retry_workflow：可选 extensionId / productionId / runId / taskKey。
- preview_frames：{"times":[秒,...]}，最多 16 个。按绝对时间渲染节目帧。
- timeline_strip：{"everyFrames":30,"maxFrames":24,"thumbHeight":120,"range":"content"|"workspace"}。时间轴接触条截帧。
- placement_status：无参数。是否仍有入轨在飞、是否被 gate 拒绝。

规则：画面必须分层（bg / 元素 / 花字 / 配音各 clip），禁止整段合成片入轨；成片用 start_workflow 分层扩展，不要自烤成片再 import+place；改工程前 get_context 拿 revision；可先 dryRun；看画面用 preview_frames / timeline_strip；每次只调用一个工具。"#.into()
}

pub fn language_request(messages: Vec<ChatMessage>, model: String) -> TaskRequest {
    TaskRequest::Language(LanguageRequest {
        model,
        messages,
        temperature: 0.2,
        json: true,
    })
}

pub fn seed_messages(user: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            role: ChatRole::System,
            content: system_prompt(),
        },
        ChatMessage {
            role: ChatRole::User,
            content: user.to_owned(),
        },
    ]
}

#[derive(Debug, Clone)]
pub enum Step {
    /// Show `reply` to the user and stop.
    Done { reply: String },
    /// Run this tool, then continue the loop with the observation.
    Tool {
        thought: Option<String>,
        name: String,
        args: Value,
    },
    /// Model output was unusable.
    Malformed { raw: String },
}

pub fn parse_step(raw: &str) -> Step {
    let trimmed = raw.trim();
    let value: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => {
            // Sometimes the model wraps JSON in fences.
            let inner = trimmed
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();
            match serde_json::from_str(inner) {
                Ok(v) => v,
                Err(_) => return Step::Malformed { raw: raw.to_owned() },
            }
        }
    };
    if value.get("done").and_then(Value::as_bool) == Some(true) {
        let reply = value
            .get("reply")
            .and_then(Value::as_str)
            .unwrap_or("好的。")
            .to_owned();
        return Step::Done { reply };
    }
    let Some(name) = value.get("tool").and_then(Value::as_str).map(str::to_owned) else {
        return Step::Malformed { raw: raw.to_owned() };
    };
    let args = value.get("args").cloned().unwrap_or_else(|| json!({}));
    let thought = value
        .get("thought")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Step::Tool {
        thought,
        name,
        args,
    }
}

pub fn control_command(name: &str, args: &Value) -> Result<ControlCommand, String> {
    match name {
        "get_context" => Ok(ControlCommand::GetContext),
        "describe" => Ok(ControlCommand::Describe {
            at: args.get("at").and_then(Value::as_f64),
        }),
        "undo" => Ok(ControlCommand::Undo),
        "redo" => Ok(ControlCommand::Redo),
        "list_extensions" => Ok(ControlCommand::ListExtensions),
        "get_import_job" => {
            let job_id = args
                .get("jobId")
                .or_else(|| args.get("job_id"))
                .and_then(Value::as_u64)
                .ok_or_else(|| "get_import_job 需要 args.jobId".to_string())?;
            Ok(ControlCommand::GetImportJob { job_id })
        }
        "import_extension" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "import_extension 需要 args.path".to_string())?;
            Ok(ControlCommand::ImportExtension {
                path: std::path::PathBuf::from(path),
                dry_run: args
                    .get("dryRun")
                    .or_else(|| args.get("dry_run"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        }
        "get_production_context" => Ok(ControlCommand::GetProductionContext {
            production_id: args
                .get("productionId")
                .or_else(|| args.get("production_id"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        }),
        "import_media" => {
            let paths = args
                .get("paths")
                .and_then(Value::as_array)
                .ok_or_else(|| "import_media 需要 args.paths 数组".to_string())?
                .iter()
                .filter_map(|v| v.as_str().map(std::path::PathBuf::from))
                .collect::<Vec<_>>();
            if paths.is_empty() {
                return Err("import_media.paths 不能为空".into());
            }
            Ok(ControlCommand::ImportMedia {
                paths,
                folder_id: args
                    .get("folderId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        }
        "apply_edits" => {
            let edits: Vec<ControlEdit> = serde_json::from_value(
                args.get("edits")
                    .cloned()
                    .ok_or_else(|| "apply_edits 需要 args.edits".to_string())?,
            )
            .map_err(|e| format!("edits 无法解析：{e}"))?;
            if edits.is_empty() {
                return Err("apply_edits.edits 不能为空".into());
            }
            Ok(ControlCommand::ApplyEdits {
                edits,
                dry_run: args
                    .get("dryRun")
                    .or_else(|| args.get("dry_run"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        }
        "start_workflow" => {
            let extension_id = args
                .get("extensionId")
                .or_else(|| args.get("extension_id"))
                .and_then(Value::as_str)
                .ok_or_else(|| "start_workflow 需要 extensionId".to_string())?
                .to_owned();
            let params = args.get("params").and_then(|v| {
                v.as_object().map(|obj| {
                    obj.iter()
                        .filter_map(|(k, val)| {
                            Some((
                                k.clone(),
                                val.as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| val.to_string()),
                            ))
                        })
                        .collect()
                })
            });
            Ok(ControlCommand::StartWorkflow {
                extension_id,
                input: args.get("input").cloned(),
                params,
                dry_run: args
                    .get("dryRun")
                    .or_else(|| args.get("dry_run"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        }
        "cancel_workflow" => Ok(ControlCommand::CancelWorkflow {
            extension_id: string_arg(args, "extensionId").or_else(|| string_arg(args, "extension_id")),
            production_id: string_arg(args, "productionId")
                .or_else(|| string_arg(args, "production_id")),
            run_id: args
                .get("runId")
                .or_else(|| args.get("run_id"))
                .and_then(Value::as_u64),
        }),
        "retry_workflow" => Ok(ControlCommand::RetryWorkflow {
            production_id: string_arg(args, "productionId")
                .or_else(|| string_arg(args, "production_id")),
            extension_id: string_arg(args, "extensionId").or_else(|| string_arg(args, "extension_id")),
            run_id: args
                .get("runId")
                .or_else(|| args.get("run_id"))
                .and_then(Value::as_u64),
            task_key: string_arg(args, "taskKey").or_else(|| string_arg(args, "task_key")),
        }),
        "get_workflow_run" => Ok(ControlCommand::GetWorkflowRun {
            run_id: args
                .get("runId")
                .or_else(|| args.get("run_id"))
                .and_then(Value::as_u64),
            production_id: string_arg(args, "productionId")
                .or_else(|| string_arg(args, "production_id")),
        }),
        other => Err(format!("未知工具：{other}（preview_frames / placement_status 由宿主本地处理）")),
    }
}

fn string_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_owned)
}

pub fn make_control_request(
    session_id: u64,
    revision: Option<u64>,
    command: ControlCommand,
) -> ControlRequest {
    ControlRequest {
        request_id: format!("agent-{}", uuid_lite()),
        session_id,
        expected_revision: revision,
        command,
    }
}

fn uuid_lite() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

pub fn outcome_json(status: &ControlStatus) -> Value {
    match status {
        ControlStatus::Ok { result } => json!({ "ok": true, "result": result }),
        ControlStatus::Error { error } => {
            json!({ "ok": false, "code": error.code, "message": error.message })
        }
    }
}

pub fn append_tool_turn(
    messages: &mut Vec<ChatMessage>,
    assistant_raw: &str,
    observation: &Value,
) {
    messages.push(ChatMessage {
        role: ChatRole::Assistant,
        content: assistant_raw.to_owned(),
    });
    messages.push(ChatMessage {
        role: ChatRole::User,
        content: format!(
            "工具结果（JSON）：\n{}\n请继续调用工具，或用 {{\"done\":true,\"reply\":\"...\"}} 结束。",
            serde_json::to_string_pretty(observation).unwrap_or_else(|_| observation.to_string())
        ),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_and_done() {
        let step = parse_step(r#"{"thought":"看一眼","tool":"get_context","args":{}}"#);
        assert!(matches!(step, Step::Tool { name, .. } if name == "get_context"));
        let step = parse_step(r#"{"done":true,"reply":"完成了"}"#);
        assert!(matches!(step, Step::Done { reply } if reply == "完成了"));
    }

    #[test]
    fn builds_undo() {
        let cmd = control_command("undo", &json!({})).unwrap();
        assert!(matches!(cmd, ControlCommand::Undo));
    }
}
