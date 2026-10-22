//! Presentation model for the AI production panel. It contains no registry,
//! filesystem or service handles; adapters supply extension cards, and the
//! production cards come from the application's status. Everything here is
//! worded for the person making the video: no task keys, segment or element
//! ids, error classes or extension ids reach the panel text.
use std::collections::BTreeMap;
use todo_application::{
    FailedWork, ProductionStatus, StageKind, StageStatus, StatusNode, StatusState,
};
use todo_production::Production;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionStatus {
    MissingKey,
    RuntimeMissing,
    SelfCheckFailed,
    Checking,
    Ready,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    Text(String),
    Number(f64),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceView {
    pub value: String,
    pub label: String,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamInput {
    Select(Vec<ChoiceView>),
    Number {
        min: f64,
        max: f64,
        step: f64,
        unit: Option<String>,
    },
    Text {
        max_length: usize,
        placeholder: Option<String>,
    },
    Bool,
    /// A voice id; `choices` are suggestions, any id (e.g. `S_…`) is allowed.
    Voice(Vec<ChoiceView>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamView {
    pub id: String,
    pub label: String,
    pub hint: Option<String>,
    pub group: String,
    /// Shown without expanding "more".
    pub quick: bool,
    pub when: BTreeMap<String, Vec<String>>,
    pub input: ParamInput,
    pub default: ParamValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScriptView {
    pub label: String,
    pub max_length: usize,
    pub placeholder: String,
    /// Empty is allowed: the style writes the script from its parameters.
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionCardView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: ExtensionStatus,
    pub problem: Option<String>,
    pub preview: Option<String>,
    pub params: Vec<ParamView>,
    pub script: Option<ScriptView>,
}

impl ExtensionCardView {
    /// One short line for the style picker; the full description is a tooltip.
    pub fn summary(&self) -> String {
        let first = self
            .description
            .split(['。', '；', '\n'])
            .next()
            .unwrap_or_default()
            .trim();
        let mut out: String = first.chars().take(28).collect();
        if first.chars().count() > 28 {
            out.push('…');
        }
        out
    }

    /// Why "start" is not available yet, in the user's terms.
    pub fn blocker(&self) -> Option<&'static str> {
        match self.status {
            ExtensionStatus::Ready => None,
            ExtensionStatus::Checking => Some("正在检查运行环境，请稍候"),
            ExtensionStatus::MissingKey => Some("还没有填写 AI 服务的 Key"),
            ExtensionStatus::RuntimeMissing => Some("缺少运行环境，请重新安装软件"),
            ExtensionStatus::SelfCheckFailed => Some("这个风格暂时不可用"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductionNodeStatus {
    Queued,
    Running,
    Waiting,
    Attention,
    Complete,
    Failed,
}

impl From<StatusState> for ProductionNodeStatus {
    fn from(state: StatusState) -> Self {
        match state {
            StatusState::Queued => Self::Queued,
            StatusState::Running => Self::Running,
            StatusState::Waiting => Self::Waiting,
            StatusState::Attention => Self::Attention,
            StatusState::Done => Self::Complete,
            StatusState::Failed => Self::Failed,
        }
    }
}

/// One step of the pipeline strip: 配音 3/5.
#[derive(Debug, Clone, PartialEq)]
pub struct StageView {
    pub label: &'static str,
    pub count: Option<String>,
    pub status: ProductionNodeStatus,
}

/// What the button next to a problem does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProblemAction {
    Retry,
    /// The request may already have been charged: confirm first.
    ConfirmRetry,
    /// Fix the key, balance or model in AI settings, then retry.
    OpenSettings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProblemView {
    /// The request to retry; `None` retries every failed composition.
    pub key: Option<String>,
    /// What failed: "第 2 段配音".
    pub title: String,
    /// Why, in plain words.
    pub reason: String,
    /// The service's own message, behind "详情".
    pub detail: Option<String>,
    pub action: ProblemAction,
}

/// One node of the production flow map (stage → segment → element).
#[derive(Debug, Clone, PartialEq)]
pub struct FlowNodeView {
    pub id: String,
    pub label: String,
    pub status: ProductionNodeStatus,
    /// Count, phase, or a short note under the label.
    pub detail: Option<String>,
    pub children: Vec<FlowNodeView>,
}

/// Inspector field for a workflow node (mirrors catalog ParamField).
#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowParamFieldView {
    Select {
        id: String,
        label: String,
        hint: String,
        options: Vec<(String, String)>,
        value: String,
    },
    Number {
        id: String,
        label: String,
        hint: String,
        min: f64,
        max: f64,
        step: f64,
        unit: Option<String>,
        value: f64,
    },
    Text {
        id: String,
        label: String,
        hint: String,
        placeholder: String,
        value: String,
    },
    FileKinds {
        id: String,
        label: String,
        hint: String,
        accept: String,
    },
}

/// One node on the workflow panel (from `todo-graph/1`).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowNodeView {
    pub id: String,
    pub node_type: String,
    pub label: String,
    pub summary: String,
    /// Longer help for the inspector.
    pub docs: String,
    pub status: ProductionNodeStatus,
    pub detail: Option<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    /// Canvas position in graph space (top-left of the card).
    pub x: f32,
    pub y: f32,
    /// Declared + resolved params for the inspector.
    pub fields: Vec<WorkflowParamFieldView>,
    /// Files attached to a media_bin / relay (from params.files).
    pub files: Vec<WorkflowMediaFileView>,
    /// Text blocks stored on a relay (from params.blocks or last run segments).
    pub text_blocks: Vec<WorkflowTextBlockView>,
    /// Raw params JSON object (for copy/paste).
    pub params_json: String,
    /// Image this node produced in the last run (path, for the preview).
    pub image: Option<String>,
    /// First output payload of the last run (pretty JSON, truncated).
    pub output: Option<String>,
    /// 该输出里的正文字数（"共 5200 字"）。
    pub output_chars: Option<usize>,
    /// 该节点最近一次运行的日志（命令窗）。
    pub logs: Vec<String>,
    /// image_gen 等节点是否有可复用的磁盘缓存。
    pub has_cache: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowMediaFileView {
    pub path: String,
    pub kind: String,
    pub name: String,
    /// Absolute path for inspector thumbnails (image file or video source).
    pub preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowTextBlockView {
    pub id: String,
    pub index: usize,
    pub text: String,
    pub chars: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorkflowEdgeView {
    pub from: String,
    pub to: String,
    pub out: Option<String>,
    pub input: Option<String>,
}

/// One entry in the "add node" palette (type_name → display label).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowTypeChoice {
    pub type_name: String,
    pub label: String,
    /// Menu section title（基础 / 大模型 / …）.
    pub group: String,
    /// Stable sort key for menu sections (smaller first).
    pub group_order: u8,
}

/// Graph projection for the workflow panel / flow window.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorkflowPanelView {
    pub nodes: Vec<WorkflowNodeView>,
    pub edges: Vec<WorkflowEdgeView>,
    /// Built-in types the user can insert from the canvas menu.
    pub palette: Vec<WorkflowTypeChoice>,
    /// 正在运行时的进度文字（"3/12 · 等待 tts"）。
    pub running: Option<String>,
    /// 本次运行的原始终端内容（run.log 尾部，直接显示）。
    pub log_tail: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProductionView {
    pub id: String,
    pub title: String,
    /// Extension package id (for workflow graph edits).
    pub extension_id: String,
    /// Display name of the style (extension) that made it.
    pub style: String,
    pub status: ProductionNodeStatus,
    /// One line on what is happening now.
    pub headline: String,
    /// 0..=1 over all stages.
    pub progress: f32,
    pub stages: Vec<StageView>,
    pub problems: Vec<ProblemView>,
    /// Notes from the pipeline worth reading (not errors).
    pub notes: Vec<String>,
    /// Generated clips the user deleted from the timeline.
    pub deleted: usize,
    pub segments: Vec<SegmentView>,
    /// Mind-map tree for the flow detail window.
    pub flow: FlowNodeView,
    /// Declared workflow graph (workflow extensions); drives the workflow panel.
    pub workflow: Option<WorkflowPanelView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentView {
    pub id: String,
    /// "第 1 段" or the segment's title.
    pub label: String,
    pub text: String,
    pub status: ProductionNodeStatus,
    pub elements: Vec<ElementView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ElementView {
    pub id: String,
    pub label: String,
    pub draw: String,
    pub status: ProductionNodeStatus,
    pub picked: Option<String>,
    pub variants: Vec<VariantView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariantView {
    pub id: String,
    /// "候选 1".
    pub label: String,
    pub file: Option<String>,
    pub locked: bool,
}

/// One workflow package in the side dock list (has graph.json).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowEntryView {
    pub extension_id: String,
    pub name: String,
    pub summary: String,
    pub origin_user: bool,
    pub status: ExtensionStatus,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProductionPanelView {
    pub extensions: Vec<ExtensionCardView>,
    pub productions: Vec<ProductionView>,
    /// Packages with a workflow graph (side dock list).
    pub workflows: Vec<WorkflowEntryView>,
    /// Productions need an open project.
    pub project_open: bool,
    /// The AI service could not start: why, in the user's terms.
    pub service_problem: Option<String>,
}

/// Manifest `when` rules: a field is visible only if every dependency has one
/// of the listed values. Unknown values simply hide the dependent field.
pub fn when_visible(
    when: &BTreeMap<String, Vec<String>>,
    values: &BTreeMap<String, String>,
) -> bool {
    when.iter().all(|(key, allowed)| {
        values
            .get(key)
            .is_some_and(|value| allowed.iter().any(|item| item == value))
    })
}

fn stage_label(kind: StageKind) -> &'static str {
    match kind {
        StageKind::Outline => "分段",
        StageKind::Voice => "配音",
        StageKind::Subtitles => "字幕",
        StageKind::Storyboard => "分镜",
        StageKind::Images => "画面",
        StageKind::Compose => "合成",
        StageKind::Generate => "生成",
        StageKind::Timeline => "入轨",
    }
}

fn is_workflow(status: &ProductionStatus) -> bool {
    status
        .stages
        .iter()
        .any(|s| s.kind == StageKind::Generate)
        && !status.stages.iter().any(|s| {
            matches!(
                s.kind,
                StageKind::Outline
                    | StageKind::Voice
                    | StageKind::Storyboard
                    | StageKind::Images
                    | StageKind::Compose
            )
        })
}

fn leaf(id: &str, label: &str, detail: &str, status: ProductionNodeStatus) -> FlowNodeView {
    FlowNodeView {
        id: id.into(),
        label: label.into(),
        status,
        detail: (!detail.is_empty()).then(|| detail.into()),
        children: Vec::new(),
    }
}

fn branch(
    id: &str,
    label: &str,
    detail: Option<String>,
    status: ProductionNodeStatus,
    children: Vec<FlowNodeView>,
) -> FlowNodeView {
    FlowNodeView {
        id: id.into(),
        label: label.into(),
        status,
        detail,
        children,
    }
}

/// Tint workflow nodes from `progress` phase (worker reports node id).
pub fn paint_workflow_panel(panel: &mut WorkflowPanelView, status: &ProductionStatus) {
    // Host already stamped per-node status from the runner report. Do not
    // flatten everything to「完成」when the generate stage is Done — that hid
    // real 入轨/节点失败 while the header said the workflow had ended.
    let has_runner_status = panel.nodes.iter().any(|n| {
        !matches!(n.status, ProductionNodeStatus::Queued)
    });
    if has_runner_status {
        return;
    }
    let generate = status.stages.iter().find(|s| s.kind == StageKind::Generate);
    let overall = generate.map(|s| s.state).unwrap_or(StatusState::Queued);
    let phase = generate.and_then(|s| s.phase.as_ref());
    let current_id = phase.map(|(p, _)| p.as_str());
    let fraction = phase.map(|(_, f)| *f);
    let run = match overall {
        StatusState::Done => ProductionNodeStatus::Complete,
        StatusState::Failed => ProductionNodeStatus::Failed,
        StatusState::Running | StatusState::Waiting => ProductionNodeStatus::Running,
        _ => ProductionNodeStatus::Queued,
    };
    let ids: Vec<String> = panel.nodes.iter().map(|n| n.id.clone()).collect();
    let current_idx = current_id.and_then(|id| ids.iter().position(|x| x == id));
    for (i, node) in panel.nodes.iter_mut().enumerate() {
        if overall == StatusState::Done || current_id == Some("done") {
            node.status = ProductionNodeStatus::Complete;
            node.detail = None;
            continue;
        }
        match current_idx {
            Some(c) if i < c => {
                node.status = ProductionNodeStatus::Complete;
                node.detail = None;
            }
            Some(c) if i == c => {
                node.status = run;
                node.detail = fraction.map(|f| format!("{:.0}%", f * 100.0));
            }
            Some(_) => {
                node.status = ProductionNodeStatus::Queued;
                node.detail = None;
            }
            None if run == ProductionNodeStatus::Running && i == 0 => {
                node.status = ProductionNodeStatus::Running;
                node.detail = None;
            }
            None => {
                node.status = ProductionNodeStatus::Queued;
                node.detail = None;
            }
        }
        if overall == StatusState::Failed && current_idx == Some(i) {
            node.status = ProductionNodeStatus::Failed;
        }
    }
}

fn workflow_flow(
    status: &ProductionStatus,
    panel: &WorkflowPanelView,
    overall: ProductionNodeStatus,
    headline: &str,
) -> FlowNodeView {
    let children = panel
        .nodes
        .iter()
        .map(|n| {
            let mut kids = Vec::new();
            if !n.inputs.is_empty() {
                kids.push(leaf(
                    &format!("{}.in", n.id),
                    &format!("入 {}", n.inputs.join(", ")),
                    "",
                    n.status,
                ));
            }
            if !n.outputs.is_empty() {
                kids.push(leaf(
                    &format!("{}.out", n.id),
                    &format!("出 {}", n.outputs.join(", ")),
                    "",
                    n.status,
                ));
            }
            if !n.summary.is_empty() {
                kids.push(leaf(
                    &format!("{}.sum", n.id),
                    &n.summary,
                    "",
                    n.status,
                ));
            }
            branch(
                &n.id,
                &n.label,
                n.detail.clone().or_else(|| Some(n.node_type.clone())),
                n.status,
                kids,
            )
        })
        .collect();
    FlowNodeView {
        id: status.id.clone(),
        label: title_of(status),
        status: overall,
        detail: Some(headline.to_owned()),
        children,
    }
}

fn narration_flow(
    status: &ProductionStatus,
    segments: &[SegmentView],
    overall: ProductionNodeStatus,
    headline: &str,
) -> FlowNodeView {
    // Host-orchestrated narration: stages already mirror plan_work order.
    let mut children = Vec::new();
    for stage in &status.stages {
        let mut kids = Vec::new();
        match stage.kind {
            StageKind::Outline => {
                kids.push(leaf(
                    "narr.outline",
                    "语义 / 段落切分",
                    "提取：段文本、标题、角色外观",
                    stage.state.into(),
                ));
            }
            StageKind::Voice => {
                for segment in segments {
                    kids.push(FlowNodeView {
                        id: format!("voice:{}", segment.id),
                        label: segment.label.clone(),
                        status: segment.status,
                        detail: Some("段配音 + 句/字时间".into()),
                        children: Vec::new(),
                    });
                }
            }
            StageKind::Subtitles => {
                kids.push(leaf(
                    "narr.subs",
                    "字幕 cue",
                    "由配音时间轴切条",
                    stage.state.into(),
                ));
            }
            StageKind::Storyboard => {
                for segment in segments {
                    kids.push(FlowNodeView {
                        id: format!("board:{}", segment.id),
                        label: segment.label.clone(),
                        status: segment.status,
                        detail: Some("黑板与元素规划".into()),
                        children: Vec::new(),
                    });
                }
            }
            StageKind::Images => {
                for segment in segments {
                    kids.push(FlowNodeView {
                        id: format!("img:{}", segment.id),
                        label: segment.label.clone(),
                        status: segment.status,
                        detail: (!segment.elements.is_empty())
                            .then(|| format!("{} 个元素", segment.elements.len())),
                        children: segment
                            .elements
                            .iter()
                            .map(|element| FlowNodeView {
                                id: element.id.clone(),
                                label: if element.label.is_empty() {
                                    element.draw.chars().take(18).collect()
                                } else {
                                    element.label.clone()
                                },
                                status: element.status,
                                detail: None,
                                children: Vec::new(),
                            })
                            .collect(),
                    });
                }
            }
            StageKind::Compose => {
                kids.push(leaf(
                    "narr.bake",
                    "烘焙 / 底板",
                    "reveal 图层或静帧 + 入场",
                    stage.state.into(),
                ));
            }
            StageKind::Timeline => {
                kids.push(leaf(
                    "narr.place",
                    "对账入轨",
                    "登记素材后放入时间轴",
                    stage.state.into(),
                ));
            }
            StageKind::Generate => {}
        }
        let detail = stage
            .phase
            .as_ref()
            .map(|(phase, fraction)| format!("{phase} · {:.0}%", fraction * 100.0))
            .or_else(|| (stage.total > 0).then(|| format!("{}/{}", stage.done, stage.total)));
        let label = match stage.kind {
            StageKind::Outline => "拆分文案",
            StageKind::Voice => "配音",
            StageKind::Subtitles => "字幕",
            StageKind::Storyboard => "分镜设计",
            StageKind::Images => "生成画面",
            StageKind::Compose => "合成图层",
            StageKind::Generate => "扩展生成",
            StageKind::Timeline => "放入时间轴",
        };
        children.push(branch(
            &format!("stage:{}", stage_label(stage.kind)),
            label,
            detail,
            stage.state.into(),
            kids,
        ));
    }
    FlowNodeView {
        id: status.id.clone(),
        label: title_of(status),
        status: overall,
        detail: Some(headline.to_owned()),
        children,
    }
}

fn flow_tree(
    status: &ProductionStatus,
    segments: &[SegmentView],
    overall: ProductionNodeStatus,
    headline: &str,
    workflow: Option<&WorkflowPanelView>,
) -> FlowNodeView {
    if let Some(panel) = workflow {
        workflow_flow(status, panel, overall, headline)
    } else if is_workflow(status) {
        // Workflow extension without a loaded graph: show a single generate node.
        FlowNodeView {
            id: status.id.clone(),
            label: title_of(status),
            status: overall,
            detail: Some(headline.to_owned()),
            children: vec![branch(
                "generate",
                "工作流",
                Some("扩展未声明 graph.json".into()),
                overall,
                Vec::new(),
            )],
        }
    } else {
        narration_flow(status, segments, overall, headline)
    }
}

fn stage_doing(kind: StageKind) -> &'static str {
    match kind {
        StageKind::Outline => "正在理解文案并切分段落",
        StageKind::Voice => "正在生成配音",
        StageKind::Subtitles => "正在对齐字幕",
        StageKind::Storyboard => "正在设计分镜",
        StageKind::Images => "正在生成画面素材",
        StageKind::Compose => "正在合成书写动画",
        StageKind::Generate => "正在生成",
        StageKind::Timeline => "正在放入时间轴",
    }
}

fn stage_view(stage: &StageStatus) -> StageView {
    let counted = !matches!(stage.kind, StageKind::Outline | StageKind::Generate);
    StageView {
        label: stage_label(stage.kind),
        count: (counted && stage.total > 0).then(|| format!("{}/{}", stage.done, stage.total)),
        status: stage.state.into(),
    }
}

/// Plain-words reason and the fitting fix for a service error class.
fn failure_reason(class: &str, billed: bool) -> (&'static str, ProblemAction) {
    let (reason, action) = match class {
        "auth" => ("Key 无效或已过期", ProblemAction::OpenSettings),
        "billing" => ("账户余额不足", ProblemAction::OpenSettings),
        "rate_limited" => ("请求太频繁，被服务方限流", ProblemAction::Retry),
        "content_policy" => (
            "内容没有通过服务方的安全审核，可改写描述后重试",
            ProblemAction::Retry,
        ),
        "invalid" => (
            "服务不接受这个请求（模型或音色可能不可用）",
            ProblemAction::OpenSettings,
        ),
        "transient" => ("网络或服务暂时不可用", ProblemAction::Retry),
        "ambiguous" => ("没有收到结果，可能已经扣费", ProblemAction::ConfirmRetry),
        "storage" => ("结果保存失败", ProblemAction::Retry),
        _ => ("生成失败", ProblemAction::Retry),
    };
    if billed && action == ProblemAction::Retry {
        (reason, ProblemAction::ConfirmRetry)
    } else {
        (reason, action)
    }
}

fn failed_title(work: &FailedWork) -> String {
    match work {
        FailedWork::Outline => "文案分段".into(),
        FailedWork::Voice { segment } => format!("第 {} 段配音", segment + 1),
        FailedWork::Storyboard { segment } => format!("第 {} 段分镜", segment + 1),
        FailedWork::Image { label } => format!("画面「{label}」"),
        FailedWork::Other => "生成任务".into(),
    }
}

fn title_of(status: &ProductionStatus) -> String {
    if !status.title.trim().is_empty() {
        return status.title.clone();
    }
    let script = status.script.trim();
    let first = script
        .split(['\n', '。', '！', '？', '，'])
        .find(|s| !s.trim().is_empty())
        .unwrap_or(script)
        .trim();
    if first.is_empty() {
        return "未命名成片".into();
    }
    let mut out: String = first.chars().take(16).collect();
    if first.chars().count() > 16 {
        out.push('…');
    }
    out
}

fn headline(
    stages: &[StageStatus],
    problems: usize,
    deleted: usize,
    attention: &[String],
) -> String {
    if attention.iter().any(|line| line.starts_with("入轨未成功")) {
        return "入轨卡住了：点「重试」，或检查轨道是否锁定".into();
    }
    let finished = !stages.is_empty() && stages.iter().all(|s| s.state == StatusState::Done);
    // Deletion count must stay visible even when workdir notes become problem cards.
    if finished && deleted > 0 {
        let workdir = attention.iter().any(|line| line.contains("工作目录已切换"));
        return if workdir {
            format!("已完成 · 时间轴已清空（删了 {deleted}）· 续跑工作目录已切换")
        } else if problems > 0 {
            format!("已完成 · 时间轴已清空（删了 {deleted}）· 另有需处理项")
        } else {
            format!("已完成 · 时间轴已清空（删了 {deleted}），从媒体库拖回或重跑")
        };
    }
    if problems > 0 {
        return format!("有 {problems} 项需要处理 · 看下方原因与下一步（缺 Key → 打开 AI 设置）");
    }
    let pending = stages.iter().find(|s| s.state != StatusState::Done);
    match pending {
        Some(stage) if stage.state == StatusState::Waiting => "网络不稳定，正在自动重试…".into(),
        Some(StageStatus {
            phase: Some((phase, fraction)),
            ..
        }) if !phase.trim().is_empty() => {
            format!("{phase}（{:.0}%）", fraction.clamp(0.0, 1.0) * 100.0)
        }
        Some(stage) if stage.total > 1 && stage.kind != StageKind::Timeline => {
            format!(
                "{}（{}/{}）",
                stage_doing(stage.kind),
                stage.done,
                stage.total
            )
        }
        Some(stage) => stage_doing(stage.kind).into(),
        None if attention.iter().any(|line| line.contains("工作目录已切换")) => {
            "续跑注意：工作目录已切换，见下方说明".into()
        }
        None => "已完成，全部内容已放入时间轴".into(),
    }
}

fn progress(stages: &[StageStatus]) -> f32 {
    if stages.is_empty() {
        return 0.0;
    }
    let sum: f32 = stages
        .iter()
        .map(|s| match (s.state, s.total, &s.phase) {
            (StatusState::Done, _, _) => 1.0,
            (_, _, Some((_, fraction))) => fraction.clamp(0.0, 1.0) as f32,
            (_, 0, _) => 0.0,
            (_, total, _) => s.done as f32 / total as f32,
        })
        .sum();
    (sum / stages.len() as f32).clamp(0.0, 1.0)
}

fn overall(stages: &[StageStatus], problems: usize) -> ProductionNodeStatus {
    if problems > 0 {
        return ProductionNodeStatus::Failed;
    }
    let states: Vec<StatusState> = stages.iter().map(|s| s.state).collect();
    if states.iter().all(|s| *s == StatusState::Done) {
        ProductionNodeStatus::Complete
    } else if states.contains(&StatusState::Waiting) {
        ProductionNodeStatus::Waiting
    } else {
        ProductionNodeStatus::Running
    }
}

/// Element and segment states from the status tree, by id.
fn node_states(nodes: &[StatusNode], out: &mut BTreeMap<String, ProductionNodeStatus>) {
    for node in nodes {
        out.insert(node.key.clone(), node.state.into());
        node_states(&node.children, out);
    }
}

pub fn production_view(status: &ProductionStatus, extension_name: Option<&str>) -> ProductionView {
    production_view_with_document(status, extension_name, None, None)
}

pub fn production_view_with_document(
    status: &ProductionStatus,
    extension_name: Option<&str>,
    document: Option<&Production>,
    workflow: Option<WorkflowPanelView>,
) -> ProductionView {
    let mut problems: Vec<ProblemView> = status
        .failures
        .iter()
        .map(|failure| {
            let (reason, action) = failure_reason(&failure.class, failure.possibly_billed);
            ProblemView {
                key: Some(failure.key.0.clone()),
                title: failed_title(&failure.work),
                reason: reason.into(),
                detail: (!failure.message.trim().is_empty()).then(|| failure.message.clone()),
                action,
            }
        })
        .collect();
    if status.run_failures > 0 {
        problems.push(ProblemView {
            key: None,
            title: if status.run_failures == 1 {
                "画面合成".into()
            } else {
                format!("{} 处画面合成", status.run_failures)
            },
            reason: "合成程序出错".into(),
            detail: None,
            action: ProblemAction::Retry,
        });
    }
    // Placement / workdir attention → actionable problem cards (30s scan).
    for note in &status.attention {
        if note.starts_with("入轨未成功：") {
            problems.push(ProblemView {
                key: None,
                title: "入轨暂停".into(),
                reason: "生成内容未能写入时间轴（轨道可能锁定或重叠）".into(),
                detail: Some(format!("{note} 下一步：解锁轨道或点「重试」。")),
                action: ProblemAction::Retry,
            });
        } else if note.contains("工作目录已切换") {
            problems.push(ProblemView {
                key: None,
                title: "续跑工作目录".into(),
                reason: "工作目录已换，已入轨相对路径可能失效".into(),
                detail: Some(format!(
                    "{note} 下一步：复用原 production workdir 续跑，或重跑并重新入轨。"
                )),
                action: ProblemAction::Retry,
            });
        }
    }
    let mut states = BTreeMap::new();
    node_states(&status.nodes, &mut states);
    let state_of = |id: &str| {
        states
            .get(id)
            .copied()
            .unwrap_or(ProductionNodeStatus::Queued)
    };
    let segments = document
        .map(|document| {
            document
                .segments
                .iter()
                .enumerate()
                .map(|(index, segment)| SegmentView {
                    id: segment.id.clone(),
                    label: if segment.title.is_empty() {
                        format!("第 {} 段", index + 1)
                    } else {
                        format!("第 {} 段 · {}", index + 1, segment.title)
                    },
                    text: segment.text.clone(),
                    status: state_of(&segment.id),
                    elements: segment
                        .boards
                        .iter()
                        .flat_map(|board| board.elements.iter())
                        .map(|element| ElementView {
                            id: element.id.clone(),
                            label: element.label.clone(),
                            draw: element.draw.clone(),
                            status: state_of(&element.id),
                            picked: element.picked().map(|variant| variant.id.clone()),
                            variants: element
                                .variants
                                .iter()
                                .enumerate()
                                .map(|(n, variant)| VariantView {
                                    id: variant.id.clone(),
                                    label: if variant.locked {
                                        format!("上传 {}", n + 1)
                                    } else {
                                        format!("候选 {}", n + 1)
                                    },
                                    file: variant.file.clone(),
                                    locked: variant.locked,
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let overall_status = overall(&status.stages, problems.len());
    let headline_text = headline(
        &status.stages,
        problems.len(),
        status.deleted,
        &status.attention,
    );
    let mut workflow = workflow;
    if let Some(panel) = workflow.as_mut() {
        paint_workflow_panel(panel, status);
    }
    let flow = flow_tree(
        status,
        &segments,
        overall_status,
        &headline_text,
        workflow.as_ref(),
    );
    ProductionView {
        id: status.id.clone(),
        title: title_of(status),
        extension_id: status.extension.clone(),
        style: extension_name.unwrap_or("AI 成片").to_owned(),
        status: overall_status,
        headline: headline_text,
        progress: progress(&status.stages),
        stages: status.stages.iter().map(stage_view).collect(),
        // Notes already promoted to problem cards are omitted to avoid double text.
        notes: status
            .attention
            .iter()
            .filter(|note| {
                !note.starts_with("入轨未成功：") && !note.contains("工作目录已切换")
            })
            .cloned()
            .collect(),
        deleted: status.deleted,
        problems,
        segments,
        flow,
        workflow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_application::{FailureStatus, TaskKey};

    fn status(stages: Vec<StageStatus>) -> ProductionStatus {
        ProductionStatus {
            id: "p".into(),
            title: String::new(),
            extension: "builtin.chalk-hand".into(),
            script: "假如说你是小薇，你怎么看待复利".into(),
            done: 0,
            total: 0,
            stages,
            nodes: Vec::new(),
            attention: Vec::new(),
            failures: Vec::new(),
            run_failures: 0,
            deleted: 0,
        }
    }

    fn st(kind: StageKind, done: usize, total: usize, state: StatusState) -> StageStatus {
        StageStatus {
            kind,
            done,
            total,
            state,
            phase: None,
        }
    }

    #[test]
    fn a_workflow_reports_its_own_phase() {
        let mut generate = st(StageKind::Generate, 0, 1, StatusState::Running);
        generate.phase = Some(("split_shots".into(), 0.42));
        let panel = WorkflowPanelView {
            nodes: vec![
                WorkflowNodeView {
                    id: "script".into(),
                    node_type: "script".into(),
                    label: "原始口播".into(),
                    summary: "".into(),
                    docs: "".into(),
                    status: ProductionNodeStatus::Queued,
                    detail: None,
                    inputs: vec![],
                    outputs: vec!["script:script_text".into()],
                    x: 40.0,
                    y: 40.0,
                    fields: vec![],
                    files: vec![],
                    text_blocks: vec![],
                    params_json: "{}".into(),
            image: None,
            output: None,
            output_chars: None,
            logs: Vec::new(),
                    has_cache: false,
                },
                WorkflowNodeView {
                    id: "speech".into(),
                    node_type: "tts".into(),
                    label: "口播转 TTS".into(),
                    summary: "".into(),
                    docs: "".into(),
                    status: ProductionNodeStatus::Queued,
                    detail: None,
                    inputs: vec!["text:script_text".into()],
                    outputs: vec!["clips:speech_clips".into()],
                    x: 300.0,
                    y: 40.0,
                    fields: vec![],
                    files: vec![],
                    text_blocks: vec![],
                    params_json: "{}".into(),
            image: None,
            output: None,
            output_chars: None,
            logs: Vec::new(),
                    has_cache: false,
                },
                WorkflowNodeView {
                    id: "split_shots".into(),
                    node_type: "split_shots".into(),
                    label: "一句一镜".into(),
                    summary: "".into(),
                    docs: "".into(),
                    status: ProductionNodeStatus::Queued,
                    detail: None,
                    inputs: vec![],
                    outputs: vec![],
                    x: 560.0,
                    y: 40.0,
                    fields: vec![],
                    files: vec![],
                    text_blocks: vec![],
                    params_json: "{}".into(),
            image: None,
            output: None,
            output_chars: None,
            logs: Vec::new(),
                    has_cache: false,
                },
            ],
            edges: vec![],
            palette: vec![],
            running: None,
            log_tail: None,
        };
        let view = production_view_with_document(
            &status(vec![
                generate,
                st(StageKind::Timeline, 2, 4, StatusState::Running),
            ]),
            Some("海绵宝宝口播"),
            None,
            Some(panel),
        );
        assert_eq!(view.headline, "split_shots（42%）");
        assert!(view.progress > 0.2 && view.progress < 0.8);
        let wf = view.workflow.as_ref().expect("workflow");
        assert_eq!(wf.nodes[0].status, ProductionNodeStatus::Complete);
        assert_eq!(wf.nodes[1].status, ProductionNodeStatus::Complete);
        assert_eq!(wf.nodes[2].status, ProductionNodeStatus::Running);
        assert_eq!(view.flow.children[2].label, "一句一镜");
    }

    #[test]
    fn paint_does_not_hide_failed_emit_when_run_done() {
        let mut panel = WorkflowPanelView {
            nodes: vec![
                WorkflowNodeView {
                    id: "tts".into(),
                    node_type: "tts".into(),
                    label: "口播转 TTS".into(),
                    summary: "".into(),
                    docs: "".into(),
                    status: ProductionNodeStatus::Complete,
                    detail: None,
                    inputs: vec![],
                    outputs: vec![],
                    x: 0.0,
                    y: 0.0,
                    fields: vec![],
                    files: vec![],
                    text_blocks: vec![],
                    params_json: "{}".into(),
                    image: None,
                    output: None,
                    output_chars: None,
                    logs: Vec::new(),
                    has_cache: false,
                },
                WorkflowNodeView {
                    id: "emit".into(),
                    node_type: "emit_sequence".into(),
                    label: "顺序入轨".into(),
                    summary: "".into(),
                    docs: "".into(),
                    status: ProductionNodeStatus::Failed,
                    detail: Some("入轨失败".into()),
                    inputs: vec![],
                    outputs: vec![],
                    x: 0.0,
                    y: 0.0,
                    fields: vec![],
                    files: vec![],
                    text_blocks: vec![],
                    params_json: "{}".into(),
                    image: None,
                    output: None,
                    output_chars: None,
                    logs: Vec::new(),
                    has_cache: false,
                },
            ],
            edges: vec![],
            palette: vec![],
            running: None,
            log_tail: None,
        };
        let mut generate = st(StageKind::Generate, 1, 1, StatusState::Done);
        generate.phase = Some(("emit".into(), 1.0));
        paint_workflow_panel(&mut panel, &status(vec![generate]));
        assert_eq!(panel.nodes[0].status, ProductionNodeStatus::Complete);
        assert_eq!(
            panel.nodes[1].status,
            ProductionNodeStatus::Failed,
            "入轨失败节点不得被刷成完成"
        );
        assert_eq!(panel.nodes[1].detail.as_deref(), Some("入轨失败"));
    }

    #[test]
    fn when_rules_are_all_of() {
        let when = BTreeMap::from([
            ("enter".into(), vec!["mix".into()]),
            ("hand".into(), vec!["chalk".into(), "none".into()]),
        ]);
        let values = BTreeMap::from([
            ("enter".into(), "mix".into()),
            ("hand".into(), "chalk".into()),
        ]);
        assert!(when_visible(&when, &values));
        assert!(!when_visible(
            &when,
            &BTreeMap::from([("enter".into(), "fade".into())])
        ));
    }

    #[test]
    fn the_card_speaks_in_stages_and_never_shows_internal_ids() {
        let view = production_view(
            &status(vec![
                st(StageKind::Outline, 1, 1, StatusState::Done),
                st(StageKind::Voice, 2, 3, StatusState::Running),
                st(StageKind::Images, 0, 4, StatusState::Queued),
                st(StageKind::Timeline, 3, 5, StatusState::Running),
            ]),
            Some("黑板手绘"),
        );
        assert_eq!(view.style, "黑板手绘");
        assert_eq!(view.title, "假如说你是小薇");
        assert_eq!(view.headline, "正在生成配音（2/3）");
        assert_eq!(view.stages[1].label, "配音");
        assert_eq!(view.stages[1].count.as_deref(), Some("2/3"));
        assert_eq!(view.stages[0].count, None);
        assert_eq!(view.status, ProductionNodeStatus::Running);
        // User-facing fields must not leak package ids (extension_id is internal).
        let text = format!(
            "{}{}{:?}{:?}",
            view.headline, view.title, view.stages, view.flow
        );
        assert!(!text.contains("builtin."), "{text}");
        assert!(!text.contains("ApplyGenerated"), "{text}");
    }

    #[test]
    fn failures_name_the_work_and_offer_the_right_fix() {
        let mut s = status(vec![st(StageKind::Voice, 0, 2, StatusState::Failed)]);
        s.failures = vec![
            FailureStatus {
                key: TaskKey("k1".into()),
                work: FailedWork::Voice { segment: 1 },
                class: "auth".into(),
                message: "401 invalid key".into(),
                possibly_billed: false,
            },
            FailureStatus {
                key: TaskKey("k2".into()),
                work: FailedWork::Image {
                    label: "存钱罐".into(),
                },
                class: "transient".into(),
                message: "timeout".into(),
                possibly_billed: true,
            },
        ];
        s.run_failures = 2;
        let view = production_view(&s, None);
        assert_eq!(view.status, ProductionNodeStatus::Failed);
        assert_eq!(view.problems[0].title, "第 2 段配音");
        assert_eq!(view.problems[0].action, ProblemAction::OpenSettings);
        assert_eq!(view.problems[1].title, "画面「存钱罐」");
        assert_eq!(view.problems[1].action, ProblemAction::ConfirmRetry);
        assert_eq!(view.problems[2].key, None);
        assert_eq!(view.problems[2].title, "2 处画面合成");
        assert!(view.headline.starts_with("有 3 项需要处理"));
    }

    #[test]
    fn a_finished_production_says_so_and_counts_deletions() {
        let mut s = status(vec![
            st(StageKind::Voice, 2, 2, StatusState::Done),
            st(StageKind::Timeline, 9, 9, StatusState::Done),
        ]);
        assert_eq!(
            production_view(&s, None).headline,
            "已完成，全部内容已放入时间轴"
        );
        s.deleted = 2;
        let view = production_view(&s, None);
        assert_eq!(view.status, ProductionNodeStatus::Complete);
        assert!((view.progress - 1.0).abs() < 1e-6);
        assert!(
            view.headline.contains("时间轴已清空") && view.headline.contains("删了 2"),
            "headline={}",
            view.headline
        );
        // Workdir problem cards must not bury the deletion count.
        s.attention
            .push("工作目录已切换（a → b）。已入轨相对路径可能失效。".into());
        let view = production_view(&s, None);
        assert!(
            view.headline.contains("删了 2") && view.headline.contains("工作目录"),
            "headline={}",
            view.headline
        );
    }

    #[test]
    fn document_projection_exposes_segments_elements_and_variants() {
        let mut document = Production::new(
            "p",
            todo_production::PackRef {
                id: "builtin.chalk-hand".into(),
                version: "1".into(),
            },
            "hello",
            Default::default(),
            todo_production::SplitMode::Paragraph,
            todo_production::segment::Limits {
                target: 120,
                max: 200,
            },
        );
        let mut segment = document.new_segment("hello".into(), String::new());
        let mut board = todo_production::production::Board {
            id: "b1".into(),
            start: 0,
            elements: Vec::new(),
            render: None,
        };
        board.elements.push(document.new_element(
            0,
            "主角",
            "一只海绵宝宝",
            todo_production::storyboard::Role::Hero,
            None,
            2,
        ));
        segment.boards.push(board);
        document.segments.push(segment);
        let view = production_view_with_document(&status(Vec::new()), None, Some(&document), None);
        assert_eq!(view.segments.len(), 1);
        assert_eq!(view.segments[0].label, "第 1 段");
        assert_eq!(view.segments[0].elements[0].variants.len(), 2);
        assert_eq!(view.segments[0].elements[0].variants[1].label, "候选 2");
        assert_eq!(view.segments[0].elements[0].label, "主角");
    }
}
