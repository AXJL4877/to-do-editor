//! Built-in Agent panel: tool-calling loop over Control + preview_frames.
use super::tool_agent::{self, Step, MAX_TOOL_ROUNDS};
use super::*;
use crate::adapters::production::host::HostReport;
use serde_json::{json, Value};
use std::sync::mpsc;
use todo_application::director::{LogKind, Phase};
use todo_production::ChatMessage;
use todo_ui::{AgentAction, AgentApproval, AgentMessage, AgentRole};

#[derive(Default)]
pub(super) struct ToolAgentSession {
    pub messages: Vec<ChatMessage>,
    pub round: u32,
    /// Raw assistant JSON of the last model turn (for append_tool_turn).
    pub last_assistant: Option<String>,
}

impl App {
    pub(super) fn handle_agent_action(&mut self, action: AgentAction, ctx: &egui::Context) {
        match action {
            AgentAction::None => {}
            AgentAction::ReadContext => self.agent_read_context(),
            AgentAction::RenderCurrent => {
                let time = self
                    .workspace
                    .editor()
                    .map(|editor| editor.playhead())
                    .unwrap_or(0.0);
                self.request_agent_render(vec![time]);
            }
            AgentAction::Send(prompt) => self.start_tool_agent(prompt, ctx),
            AgentAction::Stop => {
                self.director = None;
                self.tool_agent = None;
                self.agent_view.busy = false;
                self.agent_view.status = Some("已停止".into());
            }
            AgentAction::Approve => {
                self.agent_view.approval = None;
                self.agent_view.messages.push(AgentMessage {
                    role: AgentRole::System,
                    text: "已通过审批，打开导出".into(),
                });
                todo_ui::set_export_open(ctx, true);
            }
        }
        if self.agent_view.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn start_tool_agent(&mut self, prompt: String, ctx: &egui::Context) {
        let Some(host) = self.production.as_mut() else {
            self.agent_say(AgentRole::System, "AI 服务没有启动，请先检查 AI 设置");
            return;
        };
        if self.workspace.editor().is_none() {
            self.agent_say(AgentRole::System, "请先打开一个工程");
            return;
        }
        self.director = None;
        self.agent_view.approval = None;
        let model = host
            .language_model()
            .unwrap_or_else(|| "deepseek-flash".into());
        let messages = tool_agent::seed_messages(&prompt);
        let request_id = tool_agent::next_request_id(0);
        match host.agent_task(
            request_id,
            tool_agent::language_request(messages.clone(), model),
        ) {
            Ok(()) => {
                self.tool_agent = Some(ToolAgentSession {
                    messages,
                    round: 0,
                    last_assistant: None,
                });
                self.agent_view.busy = true;
                self.agent_view.status = Some("Agent 思考中…".into());
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            Err(error) => self.agent_say(AgentRole::System, &error),
        }
    }

    /// Every frame: renders and productions the (legacy) director waits for.
    pub(super) fn poll_director(&mut self, ctx: &egui::Context) {
        let (Some(driver), Some(host)) = (self.director.as_mut(), self.production.as_mut()) else {
            return;
        };
        let update = driver.tick(&mut self.workspace, host);
        self.handle(update, ctx);
        self.sync_director_view();
        if self.agent_view.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
    }

    pub(super) fn agent_report(&mut self, report: HostReport, ctx: &egui::Context) {
        let HostReport::AgentReply { request_id, result } = report else {
            return;
        };
        if tool_agent::is_tool_request(&request_id) {
            self.on_tool_agent_reply(result, ctx);
            return;
        }
        let (Some(driver), Some(host)) = (self.director.as_mut(), self.production.as_mut()) else {
            return;
        };
        let update = driver.on_reply(request_id, result, &mut self.workspace, host);
        self.handle(update, ctx);
        self.sync_director_view();
    }

    fn on_tool_agent_reply(&mut self, result: Result<String, String>, ctx: &egui::Context) {
        let Some(session) = self.tool_agent.as_mut() else {
            return;
        };
        let raw = match result {
            Ok(text) => text,
            Err(error) => {
                self.tool_agent = None;
                self.agent_view.busy = false;
                self.agent_say(AgentRole::System, &format!("模型调用失败：{error}"));
                return;
            }
        };
        session.last_assistant = Some(raw.clone());
        match tool_agent::parse_step(&raw) {
            Step::Done { reply } => {
                self.tool_agent = None;
                self.agent_view.busy = false;
                self.agent_view.status = Some("完成".into());
                self.agent_view.messages.push(AgentMessage {
                    role: AgentRole::Agent,
                    text: reply,
                });
            }
            Step::Malformed { raw } => {
                self.tool_agent = None;
                self.agent_view.busy = false;
                self.agent_say(
                    AgentRole::System,
                    &format!("模型未按工具协议回复，已停止。原文：{}", truncate(&raw, 240)),
                );
            }
            Step::Tool {
                thought,
                name,
                args,
            } => {
                if let Some(thought) = thought.filter(|t| !t.trim().is_empty()) {
                    self.agent_view.messages.push(AgentMessage {
                        role: AgentRole::Agent,
                        text: thought,
                    });
                }
                self.agent_view.messages.push(AgentMessage {
                    role: AgentRole::Tool,
                    text: format!("{name} {}", compact_args(&args)),
                });
                self.agent_view.status = Some(format!("调用工具：{name}…"));
                let observation = self.execute_agent_tool(&name, &args, ctx);
                let Some(session) = self.tool_agent.as_mut() else {
                    return;
                };
                session.round += 1;
                if session.round >= MAX_TOOL_ROUNDS {
                    self.tool_agent = None;
                    self.agent_view.busy = false;
                    self.agent_say(
                        AgentRole::System,
                        &format!("已达工具轮次上限（{MAX_TOOL_ROUNDS}），请缩小任务再试"),
                    );
                    return;
                }
                let assistant = session.last_assistant.clone().unwrap_or(raw);
                tool_agent::append_tool_turn(&mut session.messages, &assistant, &observation);
                let messages = session.messages.clone();
                let round = session.round;
                let Some(host) = self.production.as_mut() else {
                    self.tool_agent = None;
                    self.agent_view.busy = false;
                    self.agent_say(AgentRole::System, "AI 服务已断开");
                    return;
                };
                let model = host
                    .language_model()
                    .unwrap_or_else(|| "deepseek-flash".into());
                match host.agent_task(
                    tool_agent::next_request_id(round),
                    tool_agent::language_request(messages, model),
                ) {
                    Ok(()) => {
                        self.agent_view.status = Some("Agent 继续思考…".into());
                        ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    }
                    Err(error) => {
                        self.tool_agent = None;
                        self.agent_view.busy = false;
                        self.agent_say(AgentRole::System, &error);
                    }
                }
            }
        }
    }

    fn execute_agent_tool(
        &mut self,
        name: &str,
        args: &Value,
        ctx: &egui::Context,
    ) -> Value {
        match name {
            "preview_frames" => self.tool_preview_frames(args),
            "timeline_strip" => self.tool_timeline_strip(args),
            "placement_status" => {
                let inflight = self.workspace.placement_inflight();
                let block = self.workspace.placement_gate_block_reason();
                json!({
                    "ok": true,
                    "inflight": inflight,
                    "blocked": block,
                })
            }
            _ => match tool_agent::control_command(name, args) {
                Ok(command) => self.tool_run_control(command, ctx),
                Err(error) => json!({ "ok": false, "message": error }),
            },
        }
    }

    fn tool_run_control(
        &mut self,
        command: todo_application::control::ControlCommand,
        ctx: &egui::Context,
    ) -> Value {
        let Some(context) = self.workspace.control_context() else {
            return json!({ "ok": false, "message": "请先打开一个工程" });
        };
        let request = tool_agent::make_control_request(
            context.session_id,
            Some(context.revision),
            command,
        );
        let outcome = if request.command.needs_production_host() {
            self.control_workflow(request)
        } else {
            self.workspace.control(request)
        };
        self.handle(outcome.update, ctx);
        tool_agent::outcome_json(&outcome.response.outcome)
    }

    fn tool_preview_frames(&mut self, args: &Value) -> Value {
        let Some(context) = self.workspace.control_context() else {
            return json!({ "ok": false, "message": "请先打开一个工程" });
        };
        let times: Vec<f64> = args
            .get("times")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|v| v.as_f64())
                    .filter(|t| t.is_finite() && *t >= 0.0)
                    .take(16)
                    .collect()
            })
            .unwrap_or_default();
        let times = if times.is_empty() {
            vec![context.playhead]
        } else {
            times
        };
        let root = crate::adapters::paths::repo_root();
        let dir = crate::adapters::agent_bridge::frame_output_dir(
            &root,
            &format!("agent-frames-{}", tool_agent::next_request_id(0)),
        );
        match crate::adapters::frame::render_frames(&root, &context.project, &times, &dir) {
            Ok(frames) => {
                self.agent_view.frames = frames
                    .iter()
                    .map(|f| f.png.to_string_lossy().into_owned())
                    .collect();
                json!({
                    "ok": true,
                    "frames": frames.iter().map(|f| json!({
                        "time": f.time,
                        "png": f.png.to_string_lossy(),
                        "width": f.width,
                        "height": f.height,
                    })).collect::<Vec<_>>(),
                })
            }
            Err(error) => json!({ "ok": false, "message": error }),
        }
    }

    fn tool_timeline_strip(&mut self, args: &Value) -> Value {
        let Some(context) = self.workspace.control_context() else {
            return json!({ "ok": false, "message": "请先打开一个工程" });
        };
        let every_frames = args
            .get("everyFrames")
            .or_else(|| args.get("every_frames"))
            .and_then(Value::as_u64)
            .unwrap_or(30) as u32;
        let max_frames = args
            .get("maxFrames")
            .or_else(|| args.get("max_frames"))
            .and_then(Value::as_u64)
            .unwrap_or(24) as u32;
        let thumb_height = args
            .get("thumbHeight")
            .or_else(|| args.get("thumb_height"))
            .and_then(Value::as_u64)
            .unwrap_or(120) as u32;
        let range = crate::adapters::frame::StripRange::parse(
            args.get("range")
                .and_then(Value::as_str)
                .unwrap_or("content"),
        );
        let root = crate::adapters::paths::repo_root();
        let dir = crate::adapters::agent_bridge::frame_output_dir(
            &root,
            &format!("agent-strip-{}", tool_agent::next_request_id(0)),
        );
        match crate::adapters::frame::render_timeline_strip(
            &root,
            &context.project,
            &dir,
            every_frames,
            max_frames,
            thumb_height,
            range,
        ) {
            Ok(strip) => {
                self.agent_view.frames = strip
                    .frames
                    .iter()
                    .map(|f| f.png.to_string_lossy().into_owned())
                    .collect();
                json!({
                    "ok": true,
                    "path": strip.path.to_string_lossy(),
                    "width": strip.width,
                    "height": strip.height,
                    "fps": strip.fps,
                    "everyFrames": strip.every_frames,
                    "frames": strip.frames.iter().map(|f| json!({
                        "time": f.time,
                        "png": f.png.to_string_lossy(),
                    })).collect::<Vec<_>>(),
                })
            }
            Err(error) => json!({ "ok": false, "message": error }),
        }
    }

    fn sync_director_view(&mut self) {
        let Some(driver) = &self.director else {
            return;
        };
        let director = driver.director();
        for entry in &director.log()[self.director_logged..] {
            let role = match entry.kind {
                LogKind::Director => AgentRole::Agent,
                LogKind::Reviewer => AgentRole::Reviewer,
                LogKind::Edit => AgentRole::Edit,
                LogKind::Problem => AgentRole::System,
            };
            self.agent_view.messages.push(AgentMessage {
                role,
                text: entry.text.clone(),
            });
        }
        self.director_logged = director.log().len();
        self.agent_view.busy = director.busy();
        self.agent_view.status = Some(match director.phase() {
            Phase::Understanding => "正在理解你的要求…".into(),
            Phase::Producing { .. } => "正在按风格配方成片…".into(),
            Phase::Editing => "正在规划编辑…".into(),
            Phase::Inspecting => "正在检查版式并渲染节目帧…".into(),
            Phase::Checking => "审片员逐张回答检查单…".into(),
            Phase::Redrawing { round } => format!("第 {round} 轮：重画没通过的画面…"),
            Phase::Approval => "完成，等你审批".into(),
            Phase::Failed { reason } => format!("失败：{reason}"),
        });
        if let Some(approval) = director.approval() {
            self.agent_view.frames = approval.frames.iter().map(|(_, p)| p.clone()).collect();
            self.agent_view.approval = Some(AgentApproval {
                summary: approval.summary.clone(),
                remaining: approval.remaining.clone(),
                frames: self.agent_view.frames.clone(),
            });
        }
    }

    fn agent_say(&mut self, role: AgentRole, text: &str) {
        self.agent_view.status = Some(text.to_owned());
        self.agent_view.messages.push(AgentMessage {
            role,
            text: text.to_owned(),
        });
    }

    pub(super) fn poll_agent_render(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.agent_render_rx else {
            return;
        };
        let Ok(response) = receiver.try_recv() else {
            return;
        };
        self.agent_render_rx = None;
        self.agent_view.busy = false;
        if response.get("status").and_then(Value::as_str) != Some("ok") {
            let message = response
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("节目帧渲染失败")
                .to_owned();
            self.agent_say(AgentRole::System, &message);
            return;
        }
        self.agent_view.frames = response
            .get("result")
            .and_then(|result| result.get("frames"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|frame| frame.get("png").and_then(Value::as_str).map(str::to_owned))
            .collect();
        let text = format!("节目帧已生成：{}", self.agent_view.frames.join("、"));
        self.agent_say(AgentRole::System, &text);
        ctx.request_repaint();
    }

    fn agent_read_context(&mut self) {
        let Some(context) = self.workspace.control_context() else {
            self.agent_say(AgentRole::System, "请先打开一个工程");
            return;
        };
        let clips: usize = context
            .project
            .tracks
            .iter()
            .map(|track| track.clips.len())
            .sum();
        let text = format!(
            "已读取工程：session={} rev={} · {} 个媒体、{} 条轨道、{} 个片段，播放头 {:.2}s",
            context.session_id,
            context.revision,
            context.project.media.len(),
            context.project.tracks.len(),
            clips,
            context.playhead
        );
        self.agent_say(AgentRole::System, &text);
    }

    fn request_agent_render(&mut self, times: Vec<f64>) {
        let Some(context) = self.workspace.control_context() else {
            self.agent_view.status = Some("请先打开一个工程".into());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.agent_render_rx = Some(receiver);
        self.agent_view.busy = true;
        self.agent_view.status = Some("正在渲染节目帧…".into());
        self.start_agent_render(
            "ui-render".into(),
            context.session_id,
            times,
            sender,
        );
    }
}

fn truncate(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        text.to_owned()
    } else {
        let cut: String = text.chars().take(max).collect();
        format!("{cut}…")
    }
}

fn compact_args(args: &Value) -> String {
    let s = args.to_string();
    if s.len() <= 120 {
        s
    } else {
        format!("{}…", &s[..120])
    }
}
