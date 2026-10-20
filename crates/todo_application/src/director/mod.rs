//! The built-in Agent as a director: one bounded state machine from a user's
//! request to a cut waiting for approval.
//!
//! ```text
//! understand ─┬─ produce (style extension) → rules → checklist per picture
//!             │      → redraw the pictures that failed (≤ N rounds) → approval
//!             └─ edit (plan ControlEdits) → rules → frames → approval
//! ```
//!
//! The reviewer only makes choices: for every picture the style checks it
//! answers the style's closed yes/no questions, nothing else. What happens on
//! a "no" is fixed: the picture is redrawn from the same description by the
//! production (a new variant, replaced in place). Nothing a reviewer writes is
//! ever turned into an edit, so an uncertain reviewer can at worst cost one
//! redraw; it can never damage the cut.
//!
//! The director never touches the document or the network itself. It returns
//! [`Call`]s (ask a model, render frames, start or rerun a production, apply
//! edits) and is fed their results as [`Input`]s, so the same flow runs in the
//! editor, in a headless runner and in tests. Model output is always parsed
//! against a schema; anything malformed is dropped, never applied.
mod prompts;
#[cfg(test)]
mod tests;
mod times;

use crate::control::review::{inspect, Finding, Rubric, Severity};
use crate::control::ControlEdit;
use crate::Project;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use todo_production::ChatMessage;

pub use prompts::Target;
pub use times::review_times;

/// Pictures one edit plan may fetch or generate.
const MAX_PICTURES_PER_PLAN: usize = 3;

/// A style the director can produce with: an installed extension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    /// `id: label` of each parameter, for the model to fill.
    pub params: Vec<String>,
    /// The style's review (`recipe.json` → `review`): thresholds and checklist.
    #[serde(default)]
    pub rubric: Option<Rubric>,
    /// Pictures of the style's library an edit may swap in.
    #[serde(default)]
    pub assets: Vec<StyleAsset>,
    /// How the style's pictures are drawn (`recipe.json` → `card.style`),
    /// appended to every picture an edit generates.
    #[serde(default)]
    pub draw_style: Option<String>,
}

/// One picture of a style's asset library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleAsset {
    pub id: String,
    pub name: String,
    pub desc: String,
    /// Absolute file path.
    pub path: String,
}

/// Where a replacement picture comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum PictureSource {
    /// A file to import (a library asset).
    File(String),
    /// A picture to generate (transparent cut-out), then import.
    Draw(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Redraw rounds after the first check.
    pub fix_rounds: u32,
    /// Frames shown to the reviewer per request.
    pub review_frames: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fix_rounds: 2,
            review_frames: 8,
        }
    }
}

/// Work the host must do for the director.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    /// A language-model request answered with one JSON object.
    Language {
        id: String,
        messages: Vec<ChatMessage>,
    },
    /// A multimodal request over rendered frames.
    Vision {
        id: String,
        prompt: String,
        frames: Vec<String>,
    },
    /// Program frames at these seconds, rendered like the preview.
    Render { id: String, times: Vec<f64> },
    /// Start a production of `extension` and report when it is placed.
    Produce {
        extension: String,
        params: BTreeMap<String, Value>,
        script: String,
    },
    /// Change these parameters of the production started by `Produce` (the
    /// others stay) and report, as `Produced`, when the rerun is placed.
    Rerun { params: BTreeMap<String, Value> },
    /// Apply edits as one undo step (dry run first).
    Apply { id: String, edits: Vec<ControlEdit> },
    /// Get a picture into the project library; answer with its media id.
    Picture { id: String, source: PictureSource },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Reply {
        id: String,
        result: Result<String, String>,
    },
    Frames {
        id: String,
        result: Result<Vec<(f64, String)>, String>,
    },
    /// A production (or its rerun) is placed.
    Produced { result: Result<(), String> },
    Applied {
        id: String,
        result: Result<usize, String>,
    },
    /// The media id of a requested picture.
    Picture {
        id: String,
        result: Result<String, String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "phase")]
pub enum Phase {
    Understanding,
    Producing {
        extension: String,
    },
    Editing,
    /// Rules, then rendering the frames to check or to show.
    Inspecting,
    /// The reviewer answers the checklist.
    Checking,
    /// The production redraws the pictures that failed.
    Redrawing {
        round: u32,
    },
    /// Done; waiting for the user.
    Approval,
    Failed {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogKind {
    Director,
    Reviewer,
    Edit,
    Problem,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogEntry {
    pub kind: LogKind,
    pub text: String,
}

/// What the user approves.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub summary: String,
    pub frames: Vec<(f64, String)>,
    pub remaining: Vec<String>,
    pub fix_rounds: u32,
}

#[derive(Debug, Clone)]
pub struct Director {
    run: String,
    request: String,
    styles: Vec<StyleInfo>,
    limits: Limits,
    rubric: Rubric,
    phase: Phase,
    /// Producing with a style (else editing the open project).
    producing: bool,
    seq: u64,
    waiting: Option<String>,
    round: u32,
    findings: Vec<Finding>,
    frames: Vec<(f64, String)>,
    /// Pictures of the current check, their frames, and the next one to ask.
    targets: Vec<Target>,
    asked: usize,
    /// Pictures that failed the current check: key → failed question ids.
    rejected: BTreeMap<String, (f64, Vec<String>)>,
    /// Redraw variant per picture key, sent to the production.
    redraws: BTreeMap<String, u32>,
    /// The latest frame of every checked picture, for the approval.
    shown: BTreeMap<String, (f64, String)>,
    /// The edit conversation, so a rejected batch can be corrected once.
    edit_messages: Vec<ChatMessage>,
    edit_retried: bool,
    log: Vec<LogEntry>,
    approval: Option<Approval>,
    /// Pictures of the chosen style (or of every style when editing).
    assets: Vec<StyleAsset>,
    draw_style: Option<String>,
    /// Picture swaps still to fetch, edits waiting for them, swaps done.
    pictures: Vec<(String, PictureSource)>,
    picture_clip: Option<String>,
    held_edits: Vec<ControlEdit>,
    swaps: Vec<ControlEdit>,
}

impl Director {
    /// `run` prefixes every request id; it must differ between runs that
    /// share a host or a frames folder (the driver uses a timestamp).
    pub fn start(
        run: String,
        request: String,
        styles: Vec<StyleInfo>,
        project: &Project,
    ) -> (Self, Vec<Call>) {
        Self::with_limits(run, request, styles, project, Limits::default())
    }

    /// Start the built-in editor agent without selecting a production style.
    ///
    /// Template-backed productions are started explicitly by the production
    /// panel. Keeping this path separate means an edit request can never
    /// accidentally turn into a production using whichever extension happens
    /// to be installed (or appear first in the registry).
    pub fn start_edit(run: String, request: String, project: &Project) -> (Self, Vec<Call>) {
        let mut director = Self::initialized(run, request, Vec::new(), Limits::default());
        director.phase = Phase::Editing;
        director.edit_messages = prompts::edit(&director.request, &[], project);
        let messages = director.edit_messages.clone();
        let call = director.ask(messages);
        (director, vec![call])
    }

    pub fn with_limits(
        run: String,
        request: String,
        styles: Vec<StyleInfo>,
        project: &Project,
        limits: Limits,
    ) -> (Self, Vec<Call>) {
        let mut director = Self::initialized(run, request, styles, limits);
        let messages = prompts::understand(&director.request, &director.styles, project);
        let call = director.ask(messages);
        (director, vec![call])
    }

    fn initialized(run: String, request: String, styles: Vec<StyleInfo>, limits: Limits) -> Self {
        let assets = styles.iter().flat_map(|s| s.assets.clone()).collect();
        Self {
            run,
            request,
            styles,
            limits,
            rubric: Rubric::default(),
            phase: Phase::Understanding,
            producing: false,
            seq: 0,
            waiting: None,
            round: 0,
            findings: Vec::new(),
            frames: Vec::new(),
            targets: Vec::new(),
            asked: 0,
            rejected: BTreeMap::new(),
            redraws: BTreeMap::new(),
            shown: BTreeMap::new(),
            edit_messages: Vec::new(),
            edit_retried: false,
            log: Vec::new(),
            approval: None,
            assets,
            draw_style: None,
            pictures: Vec::new(),
            picture_clip: None,
            held_edits: Vec::new(),
            swaps: Vec::new(),
        }
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn log(&self) -> &[LogEntry] {
        &self.log
    }

    pub fn approval(&self) -> Option<&Approval> {
        self.approval.as_ref()
    }

    /// Still working (not waiting for the user, not failed).
    pub fn busy(&self) -> bool {
        !matches!(self.phase, Phase::Approval | Phase::Failed { .. })
    }

    fn next_id(&mut self, kind: &str) -> String {
        self.seq += 1;
        let id = format!("{}-{kind}-{}", self.run, self.seq);
        self.waiting = Some(id.clone());
        id
    }

    fn ask(&mut self, messages: Vec<ChatMessage>) -> Call {
        Call::Language {
            id: self.next_id("ask"),
            messages,
        }
    }

    fn note(&mut self, kind: LogKind, text: impl Into<String>) {
        self.log.push(LogEntry {
            kind,
            text: text.into(),
        });
    }

    fn fail(&mut self, reason: impl Into<String>) -> Vec<Call> {
        let reason = reason.into();
        self.note(LogKind::Problem, reason.clone());
        self.phase = Phase::Failed { reason };
        self.waiting = None;
        Vec::new()
    }

    /// Feed a result; returns the next work. Results for calls the director
    /// is not waiting for (stale ids) are ignored.
    pub fn on(&mut self, input: Input, project: &Project) -> Vec<Call> {
        let id = match &input {
            Input::Reply { id, .. }
            | Input::Frames { id, .. }
            | Input::Applied { id, .. }
            | Input::Picture { id, .. } => Some(id.clone()),
            Input::Produced { .. } => None,
        };
        if id.is_some() && id != self.waiting {
            return Vec::new();
        }
        match (self.phase.clone(), input) {
            (Phase::Understanding, Input::Reply { result, .. }) => match result {
                Ok(text) => self.understood(&text, project),
                Err(error) => self.fail(format!("理解指令失败：{error}")),
            },
            (Phase::Producing { .. }, Input::Produced { result }) => match result {
                Ok(()) => {
                    self.note(LogKind::Director, "成片已入轨，开始审片");
                    self.inspect(project, None)
                }
                Err(error) => self.fail(format!("成片失败：{error}")),
            },
            (Phase::Redrawing { .. }, Input::Produced { result }) => match result {
                Ok(()) => {
                    let keys: Vec<String> = self.rejected.keys().cloned().collect();
                    self.note(
                        LogKind::Edit,
                        format!("已重画 {} 张画面，原位替换", keys.len()),
                    );
                    self.inspect(project, Some(keys))
                }
                Err(error) => {
                    self.note(LogKind::Problem, format!("重画失败：{error}"));
                    self.finish()
                }
            },
            (Phase::Editing, Input::Reply { result, .. }) => match result {
                Ok(text) => self.planned_edits(&text, project),
                Err(error) => {
                    self.note(LogKind::Problem, format!("规划编辑失败：{error}"));
                    self.finish()
                }
            },
            (Phase::Editing, Input::Applied { result, .. }) => match result {
                Ok(steps) => {
                    self.note(LogKind::Edit, format!("已执行 {steps} 个编辑步骤"));
                    self.inspect(project, None)
                }
                Err(error) if !self.edit_retried => {
                    self.edit_retried = true;
                    self.note(
                        LogKind::Problem,
                        format!("编辑被拒绝：{error}，让模型修正一次"),
                    );
                    self.edit_messages.push(ChatMessage {
                        role: todo_production::ChatRole::User,
                        content: format!(
                            "编辑器拒绝了这批编辑：{error}\n请只修正出错的步骤，重新输出完整 JSON。"
                        ),
                    });
                    let messages = self.edit_messages.clone();
                    vec![self.ask(messages)]
                }
                Err(error) => {
                    self.note(LogKind::Problem, format!("编辑仍被拒绝，停止修改：{error}"));
                    self.finish()
                }
            },
            (Phase::Editing, Input::Picture { result, .. }) => {
                let clip = self.picture_clip.take().unwrap_or_default();
                match result {
                    Ok(media_id) => {
                        self.note(LogKind::Edit, format!("片段 {clip} 换上了新画面"));
                        self.swaps.push(ControlEdit::ReplaceMedia {
                            clip: crate::control::ClipRef::Id { id: clip },
                            media_id,
                        });
                    }
                    Err(error) => {
                        self.note(LogKind::Problem, format!("片段 {clip} 换画面失败：{error}"))
                    }
                }
                self.next_picture()
            }
            (Phase::Inspecting, Input::Frames { result, .. }) => match result {
                Ok(frames) => {
                    self.frames = frames;
                    if self.producing {
                        self.check_next()
                    } else {
                        self.finish()
                    }
                }
                Err(error) => {
                    self.note(LogKind::Problem, format!("渲染节目帧失败：{error}"));
                    self.finish()
                }
            },
            (Phase::Checking, Input::Reply { result, .. }) => {
                let batch =
                    self.asked..(self.asked + self.limits.review_frames).min(self.targets.len());
                let answers = result.and_then(|text| {
                    prompts::parse_checklist(&text, batch.len(), &self.rubric.checks)
                });
                match answers {
                    Ok(failed) => {
                        for (offset, failed) in failed.into_iter().enumerate() {
                            if failed.is_empty() {
                                continue;
                            }
                            let target = self.targets[batch.start + offset].clone();
                            let names = prompts::question_names(&self.rubric.checks, &failed);
                            self.note(
                                LogKind::Reviewer,
                                format!("{:.1}s {} 未通过：{names}", target.at, target.key),
                            );
                            self.rejected.insert(target.key, (target.at, failed));
                        }
                    }
                    // An unreadable answer is "unsure" for every picture.
                    Err(error) => self.note(LogKind::Problem, format!("审片回答无法读取：{error}")),
                }
                self.asked = batch.end;
                self.check_next()
            }
            _ => Vec::new(),
        }
    }

    fn understood(&mut self, text: &str, project: &Project) -> Vec<Call> {
        let intent = match prompts::parse_intent(text, &self.styles) {
            Ok(intent) => intent,
            Err(error) => return self.fail(format!("无法理解指令：{error}")),
        };
        if !intent.reply.is_empty() {
            self.note(LogKind::Director, intent.reply.clone());
        }
        match intent.mode {
            prompts::Mode::Produce {
                extension,
                mut params,
                script,
            } => {
                if let Some(style) = self.styles.iter().find(|s| s.id == extension) {
                    if let Some(rubric) = &style.rubric {
                        self.rubric = rubric.clone();
                    }
                    // Redraws are the checklist's to request, never the model's.
                    if let Some(redraw) = &self.rubric.redraw {
                        params.remove(redraw);
                    }
                    self.assets = style.assets.clone();
                    self.draw_style = style.draw_style.clone();
                    self.note(LogKind::Director, format!("用「{}」配方成片", style.name));
                }
                self.producing = true;
                self.phase = Phase::Producing {
                    extension: extension.clone(),
                };
                self.waiting = None;
                vec![Call::Produce {
                    extension,
                    params,
                    script,
                }]
            }
            prompts::Mode::Edit => {
                self.phase = Phase::Editing;
                self.edit_messages = prompts::edit(&self.request, &self.assets, project);
                let messages = self.edit_messages.clone();
                vec![self.ask(messages)]
            }
        }
    }

    fn planned_edits(&mut self, text: &str, project: &Project) -> Vec<Call> {
        self.edit_messages.push(ChatMessage {
            role: todo_production::ChatRole::Assistant,
            content: text.to_owned(),
        });
        let plan = match prompts::parse_edits(text, Some(project)) {
            Ok(plan) => plan,
            Err(error) => {
                self.note(LogKind::Problem, format!("编辑计划格式不对：{error}"));
                return self.finish();
            }
        };
        if !plan.notes.is_empty() {
            self.note(LogKind::Director, plan.notes);
        }
        for skipped in plan.skipped {
            self.note(LogKind::Problem, skipped);
        }
        // Picture swaps are fetched first, then applied with the edits as
        // one undo step.
        self.pictures.clear();
        self.swaps.clear();
        for fix in plan.pictures {
            let source = match fix.source {
                prompts::PictureChoice::Library(id) => {
                    match self.assets.iter().find(|a| a.id == id) {
                        Some(asset) => PictureSource::File(asset.path.clone()),
                        None => {
                            self.note(LogKind::Problem, format!("素材库里没有 {id}，跳过"));
                            continue;
                        }
                    }
                }
                prompts::PictureChoice::Draw(prompt) => {
                    PictureSource::Draw(match &self.draw_style {
                        Some(style) => format!("{}. {style}", prompt.trim_end_matches('.')),
                        None => prompt,
                    })
                }
            };
            self.pictures.push((fix.clip, source));
        }
        self.pictures.truncate(MAX_PICTURES_PER_PLAN);
        self.held_edits = plan.edits;
        self.next_picture()
    }

    fn next_picture(&mut self) -> Vec<Call> {
        if !self.pictures.is_empty() {
            let (clip, source) = self.pictures.remove(0);
            self.picture_clip = Some(clip);
            let id = self.next_id("picture");
            return vec![Call::Picture { id, source }];
        }
        let mut edits = std::mem::take(&mut self.swaps);
        edits.append(&mut self.held_edits);
        if edits.is_empty() {
            self.note(LogKind::Director, "没有需要执行的编辑");
            return self.finish();
        }
        let id = self.next_id("apply");
        vec![Call::Apply { id, edits }]
    }

    /// Rules first (reported, never acted on by a model); then render the
    /// frames to check (producing: the style's pictures, `only` these keys
    /// after a redraw) or to show (editing).
    fn inspect(&mut self, project: &Project, only: Option<Vec<String>>) -> Vec<Call> {
        self.phase = Phase::Inspecting;
        self.findings = inspect(project, &self.rubric);
        let blocks = self
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Block)
            .count();
        self.note(
            LogKind::Reviewer,
            format!("规则检查：{} 项（阻塞 {blocks}）", self.findings.len()),
        );
        let times = if self.producing {
            self.targets = if self.rubric.checks.is_empty() {
                Vec::new()
            } else {
                prompts::targets(project, &self.rubric)
            };
            if let Some(keys) = &only {
                self.targets.retain(|t| keys.contains(&t.key));
            }
            self.asked = 0;
            self.rejected.clear();
            self.targets.iter().map(|t| t.at).collect()
        } else {
            review_times(project, self.limits.review_frames)
        };
        if times.is_empty() {
            return self.finish();
        }
        let id = self.next_id("frames");
        vec![Call::Render { id, times }]
    }

    /// Ask the next batch of the checklist, or decide when all are answered.
    fn check_next(&mut self) -> Vec<Call> {
        if self.asked == 0 {
            // One frame per picture, matched by time.
            let rendered = std::mem::take(&mut self.frames);
            for target in &self.targets {
                if let Some(frame) = rendered
                    .iter()
                    .filter(|(t, _)| (t - target.at).abs() < 0.05)
                    .min_by(|a, b| (a.0 - target.at).abs().total_cmp(&(b.0 - target.at).abs()))
                {
                    self.frames.push(frame.clone());
                    self.shown.insert(target.key.clone(), frame.clone());
                }
            }
        }
        if self.asked >= self.targets.len() || self.frames.len() < self.targets.len() {
            return self.decide();
        }
        self.phase = Phase::Checking;
        let end = (self.asked + self.limits.review_frames).min(self.targets.len());
        let prompt = prompts::checklist(&self.rubric.checks, &self.targets[self.asked..end]);
        let frames = self.frames[self.asked..end]
            .iter()
            .map(|(_, path)| path.clone())
            .collect();
        let id = self.next_id("check");
        vec![Call::Vision { id, prompt, frames }]
    }

    /// Redraw what failed, or hand the cut to the user.
    fn decide(&mut self) -> Vec<Call> {
        let passed = self.targets.len() - self.rejected.len();
        self.note(
            LogKind::Reviewer,
            format!("检查单：{} 张画面，{passed} 张通过", self.targets.len()),
        );
        let Some(param) = self.rubric.redraw.clone() else {
            return self.finish();
        };
        if self.rejected.is_empty() || self.round >= self.limits.fix_rounds {
            return self.finish();
        }
        self.round += 1;
        for key in self.rejected.keys() {
            self.redraws.insert(key.clone(), self.round);
        }
        self.phase = Phase::Redrawing { round: self.round };
        self.waiting = None;
        self.note(
            LogKind::Director,
            format!(
                "第 {} 轮：用同样的分镜描述重画 {} 张画面",
                self.round,
                self.rejected.len()
            ),
        );
        let value = self
            .redraws
            .iter()
            .map(|(key, variant)| format!("{key}:{variant}"))
            .collect::<Vec<_>>()
            .join(",");
        vec![Call::Rerun {
            params: BTreeMap::from([(param, Value::String(value))]),
        }]
    }

    fn finish(&mut self) -> Vec<Call> {
        let mut remaining: Vec<String> = self.findings.iter().map(|f| f.message.clone()).collect();
        for (key, (at, failed)) in &self.rejected {
            remaining.push(format!(
                "{at:.1}s {key} 未通过：{}",
                prompts::question_names(&self.rubric.checks, failed)
            ));
        }
        let summary = match (remaining.is_empty(), self.producing) {
            (true, true) => format!("成片已完成，重画 {} 轮，检查全部通过。", self.round),
            (false, true) => format!(
                "成片已完成，重画 {} 轮，还有 {} 处需要你看一下。",
                self.round,
                remaining.len()
            ),
            (true, false) => "改好了，规则检查没有问题。".to_owned(),
            (false, false) => format!("改好了，还有 {} 处需要你看一下。", remaining.len()),
        };
        self.note(LogKind::Director, summary.clone());
        let mut frames: Vec<(f64, String)> = if self.producing {
            self.shown.values().cloned().collect()
        } else {
            self.frames.clone()
        };
        frames.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.approval = Some(Approval {
            summary,
            frames,
            remaining,
            fix_rounds: self.round,
        });
        self.phase = Phase::Approval;
        self.waiting = None;
        Vec::new()
    }
}
