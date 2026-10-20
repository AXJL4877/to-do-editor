//! The built-in Agent panel drives the editor director: one request becomes a
//! reviewed edit that waits for the user's approval.
use super::*;
use crate::adapters::director::DirectorDriver;
use crate::adapters::production::host::HostReport;
use serde_json::Value;
use std::sync::mpsc;
use todo_application::director::{LogKind, Phase};
use todo_ui::{AgentAction, AgentApproval, AgentMessage, AgentRole};

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
            AgentAction::Send(prompt) => self.start_director(prompt, ctx),
            AgentAction::Stop => {
                self.director = None;
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

    fn start_director(&mut self, prompt: String, ctx: &egui::Context) {
        let Some(host) = self.production.as_mut() else {
            self.agent_say(AgentRole::System, "AI 服务没有启动，请先检查 AI 设置");
            return;
        };
        if self.workspace.editor().is_none() {
            self.agent_say(AgentRole::System, "请先打开一个工程");
            return;
        }
        self.agent_view.approval = None;
        match DirectorDriver::start_edit(
            crate::adapters::paths::repo_root(),
            prompt,
            &mut self.workspace,
            host,
        ) {
            Ok((driver, update)) => {
                self.director = Some(driver);
                self.director_logged = 0;
                self.handle(update, ctx);
                self.sync_director_view();
            }
            Err(error) => self.agent_say(AgentRole::System, &error),
        }
    }

    /// Every frame: renders and productions the director waits for.
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
        let (Some(driver), Some(host)) = (self.director.as_mut(), self.production.as_mut()) else {
            return;
        };
        let update = driver.on_reply(request_id, result, &mut self.workspace, host);
        self.handle(update, ctx);
        self.sync_director_view();
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
            Phase::Producing { .. } => "正在按风格配方成片（文案、配音、分镜、素材、入轨）…".into(),
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
            "已读取工程：{} 个媒体、{} 条轨道、{} 个片段，播放头 {:.2}s",
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
            format!("builtin-review-{}", context.revision),
            context.session_id,
            times,
            sender,
        );
    }
}
