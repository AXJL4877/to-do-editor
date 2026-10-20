//! `extension.json` (spec `todo-extension/1`): types, validation and parameter
//! resolution. See `docs/architecture/extension-spec.md`.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const EXTENSION_SPEC: &str = "todo-extension/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// The host's narration pipeline, configured by `narration`.
    Narration,
    /// A worker program runs the whole flow (`worker.ops` contains `run`).
    Workflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Service {
    Chat,
    Image,
    Speech,
    Subtitles,
    Probe,
    /// PNG frames → video with the host's quality presets (packed alpha when transparent).
    Encode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub spec: String,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub kind: Kind,
    /// What the user provides; nothing at all is fine (parameters only).
    #[serde(default)]
    pub input: Input,
    #[serde(default)]
    pub params: Vec<Param>,
    #[serde(default)]
    pub quick: Vec<String>,
    pub requires: Requires,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<Worker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narration: Option<Narration>,
    /// Optional workflow graph IR path (`todo-graph/1`), package-relative.
    /// Declares nodes/edges for future host runners; today validated at install.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
    #[serde(default)]
    pub budget: BTreeMap<Service, u32>,
    #[serde(default)]
    pub previews: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Input {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<ScriptInput>,
    /// Media the user picks (timeline selection or library items).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    Video,
    Image,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MediaInput {
    pub label: String,
    pub kinds: Vec<MediaKind>,
    #[serde(default = "one_usize")]
    pub min: usize,
    #[serde(default = "one_usize")]
    pub max: usize,
}

fn one_usize() -> usize {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptInput {
    pub label: String,
    pub max_length: usize,
    #[serde(default)]
    pub placeholder: String,
    /// The extension can work without a script (it writes one from params).
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Requires {
    #[serde(default)]
    pub services: BTreeSet<Service>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Runtime>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Runtime {
    #[serde(rename = "type")]
    pub kind: RuntimeKind,
    #[serde(default)]
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeKind {
    /// The packaged Python (`dependencies/python`), entry is a `.py` file.
    Python,
    /// A native executable shipped in the package (`.exe`).
    Native,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Worker {
    pub entry: String,
    pub ops: Vec<String>,
    pub timeout_sec: u32,
    #[serde(default = "one")]
    pub max_processes: u32,
    #[serde(default = "default_memory", rename = "memoryMiB")]
    pub memory_mib: u32,
}

fn one() -> u32 {
    1
}
fn default_memory() -> u32 {
    1024
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Narration {
    pub planner: Planner,
    pub assets: Assets,
    pub segment: SegmentLimits,
    pub board_sec: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pacing: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitles: Option<SubtitleConfig>,
    /// Worker op that turns a board into layer items; none = stills.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bake: Option<String>,
    /// Worker op that makes the background plate under every board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plate: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Planner {
    pub prompt: String,
    pub max_elements: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Assets {
    pub template: String,
    pub roles: BTreeMap<String, String>,
    #[serde(default)]
    pub avoid: Vec<String>,
    pub size: String,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub transparent: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SegmentLimits {
    pub target: usize,
    pub max: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubtitleConfig {
    pub max_chars: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Group {
    Story,
    Voice,
    Scene,
    Timing,
    Stroke,
    Text,
    Output,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Prompt fragment used for `{{param}}` placeholders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Param {
    Select {
        id: String,
        label: String,
        group: Group,
        default: String,
        options: Vec<SelectOption>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        when: Option<BTreeMap<String, Vec<String>>>,
    },
    Number {
        id: String,
        label: String,
        group: Group,
        default: f64,
        min: f64,
        max: f64,
        step: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        when: Option<BTreeMap<String, Vec<String>>>,
    },
    Text {
        id: String,
        label: String,
        group: Group,
        default: String,
        max_length: usize,
        /// Allowed characters: a simple `[class]{min,max}` style is not
        /// supported; use `charset` instead (letters, digits and these symbols).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        charset: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        when: Option<BTreeMap<String, Vec<String>>>,
    },
    Bool {
        id: String,
        label: String,
        group: Group,
        default: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        when: Option<BTreeMap<String, Vec<String>>>,
    },
    Voice {
        id: String,
        label: String,
        group: Group,
        default: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        when: Option<BTreeMap<String, Vec<String>>>,
    },
}

impl Param {
    pub fn id(&self) -> &str {
        match self {
            Self::Select { id, .. }
            | Self::Number { id, .. }
            | Self::Text { id, .. }
            | Self::Bool { id, .. }
            | Self::Voice { id, .. } => id,
        }
    }
    fn when(&self) -> Option<&BTreeMap<String, Vec<String>>> {
        match self {
            Self::Select { when, .. }
            | Self::Number { when, .. }
            | Self::Text { when, .. }
            | Self::Bool { when, .. }
            | Self::Voice { when, .. } => when.as_ref(),
        }
    }
}

/// One problem found while validating; validation reports them all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

fn is_slug(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 40
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn valid_id(id: &str) -> bool {
    id.split_once('.')
        .is_some_and(|(author, name)| is_slug(author) && is_slug(name))
}

pub fn valid_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 6 && p.chars().all(|c| c.is_ascii_digit()))
}

fn valid_param_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && id.len() <= 32
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn valid_option_value(value: &str) -> bool {
    (1..=32).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.:-".contains(c))
}

/// A package-relative path without `..`, roots, drive letters or reserved names.
pub fn valid_relative_path(path: &str) -> bool {
    const RESERVED: &[&str] = &["con", "prn", "aux", "nul"];
    if path.is_empty()
        || path.len() > 240
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
    {
        return false;
    }
    path.split('/').all(|part| {
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        !part.is_empty()
            && part != "."
            && part != ".."
            && !part.ends_with('.')
            && !part.ends_with(' ')
            && !part.chars().any(|c| c.is_control())
            && !RESERVED.contains(&stem.as_str())
            && !(stem.len() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && stem.as_bytes()[3].is_ascii_digit())
    })
}

/// `{{ name }}` placeholders of a template, or the first malformed brace.
pub fn placeholders(template: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        let after = &rest[open + 2..];
        let close = after.find("}}").ok_or("未闭合的 {{")?;
        let name = after[..close].trim_matches(|c: char| c.is_ascii_whitespace());
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("不合法的占位符 {{{{{name}}}}}"));
        }
        out.push(name.to_owned());
        rest = &after[close + 2..];
    }
    if rest.contains("}}") {
        return Err("多余的 }}".into());
    }
    Ok(out)
}

impl Manifest {
    /// Every problem, empty when valid. `read` returns package text files
    /// (prompts) by relative path, `None` when missing.
    pub fn validate(&self, read: &dyn Fn(&str) -> Option<String>) -> Vec<Problem> {
        let mut problems = Vec::new();
        let mut problem = |path: &str, message: String| {
            problems.push(Problem {
                path: path.into(),
                message,
            })
        };
        if self.spec != EXTENSION_SPEC {
            problem("spec", format!("须为 {EXTENSION_SPEC}"));
        }
        if !valid_id(&self.id) {
            problem("id", "形如 作者.名称（小写字母、数字、-）".into());
        }
        if !valid_version(&self.version) {
            problem("version", "须为 x.y.z".into());
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 24 {
            problem("name", "1~24 个字".into());
        }
        if let Some(script) = &self.input.script {
            if !(1..=20_000).contains(&script.max_length) {
                problem("input.script.maxLength", "1~20000".into());
            }
        }
        if let Some(media) = &self.input.media {
            if media.kinds.is_empty() || media.min > media.max || !(1..=100).contains(&media.max) {
                problem("input.media", "kinds 非空，0 ≤ min ≤ max ≤ 100".into());
            }
        }
        // Parameters.
        let mut ids = BTreeSet::new();
        let selects: BTreeMap<&str, Vec<&str>> = self
            .params
            .iter()
            .filter_map(|p| match p {
                Param::Select { id, options, .. } => Some((
                    id.as_str(),
                    options.iter().map(|o| o.value.as_str()).collect(),
                )),
                _ => None,
            })
            .collect();
        for (i, param) in self.params.iter().enumerate() {
            let at = format!("params[{i}]");
            if !valid_param_id(param.id()) {
                problem(&at, format!("id {:?} 不合法", param.id()));
            }
            if !ids.insert(param.id()) {
                problem(&at, format!("id {:?} 重复", param.id()));
            }
            if let Some(when) = param.when() {
                for (other, values) in when {
                    match selects.get(other.as_str()) {
                        Some(allowed) if other != param.id() => {
                            for value in values.iter().filter(|v| !allowed.contains(&v.as_str())) {
                                problem(&at, format!("when.{other} 没有选项 {value:?}"));
                            }
                        }
                        _ => problem(&at, format!("when 只能依赖其他 select 参数（{other}）")),
                    }
                }
            }
            match param {
                Param::Select {
                    options, default, ..
                } => {
                    if !(1..=256).contains(&options.len()) {
                        problem(&at, "选项须 1~256 个".into());
                    }
                    let mut values = BTreeSet::new();
                    for option in options {
                        if !valid_option_value(&option.value)
                            || !values.insert(option.value.as_str())
                        {
                            problem(&at, format!("选项值 {:?} 不合法或重复", option.value));
                        }
                    }
                    if !values.contains(default.as_str()) {
                        problem(&at, format!("默认值 {default:?} 不在选项中"));
                    }
                }
                Param::Number {
                    min,
                    max,
                    step,
                    default,
                    ..
                } => {
                    if !(min.is_finite()
                        && max.is_finite()
                        && step.is_finite()
                        && *step > 0.0
                        && min < max)
                    {
                        problem(&at, "min < max，step > 0".into());
                    } else if default < min || default > max {
                        problem(&at, "默认值越界".into());
                    }
                }
                Param::Text {
                    max_length,
                    default,
                    ..
                } => {
                    if !(1..=2000).contains(max_length) || default.chars().count() > *max_length {
                        problem(&at, "maxLength 须 1~2000 且默认值不超长".into());
                    }
                }
                Param::Bool { .. } | Param::Voice { .. } => {}
            }
        }
        if self.quick.len() > 6 || self.quick.iter().any(|q| !ids.contains(q.as_str())) {
            problem("quick", "最多 6 个，且须是已声明的参数".into());
        }
        // Worker.
        match (&self.worker, &self.requires.runtime) {
            (Some(worker), Some(runtime)) => {
                let suffix = match runtime.kind {
                    RuntimeKind::Python => ".py",
                    RuntimeKind::Native => ".exe",
                };
                if !valid_relative_path(&worker.entry) || !worker.entry.ends_with(suffix) {
                    problem("worker.entry", format!("须为包内 {suffix} 相对路径"));
                }
                if worker.ops.is_empty() || worker.ops.iter().any(|op| !valid_param_id(op)) {
                    problem("worker.ops", "至少一个，名称为小写字母开头的标识符".into());
                }
                if !(10..=7200).contains(&worker.timeout_sec) {
                    problem("worker.timeoutSec", "10~7200".into());
                }
                if !(1..=16).contains(&worker.max_processes)
                    || !(64..=16384).contains(&worker.memory_mib)
                {
                    problem("worker", "maxProcesses 1~16，memoryMiB 64~16384".into());
                }
            }
            (Some(_), None) => problem("requires.runtime", "有工作程序时必须声明运行时".into()),
            (None, Some(_)) => problem("requires.runtime", "没有工作程序时不要声明运行时".into()),
            (None, None) => {}
        }
        let ops: BTreeSet<&str> = self
            .worker
            .iter()
            .flat_map(|w| w.ops.iter().map(String::as_str))
            .collect();
        // Optional graph IR (workflow only).
        match (self.kind, &self.graph) {
            (Kind::Narration, Some(_)) => {
                problem("graph", "只用于 kind = workflow".into());
            }
            (_, Some(path)) => {
                if !valid_relative_path(path) || !path.ends_with(".json") {
                    problem("graph", "须为包内 .json 相对路径".into());
                } else {
                    match read(path) {
                        None => problem("graph", format!("找不到 {path}")),
                        Some(text) => match crate::extension::graph::Graph::parse(&text, "graph") {
                            Err(p) => problem(&p.path, p.message),
                            Ok(graph) => {
                                // 画布保存/安装只做草稿校验：必需端口未接也能先建图，
                                // 真正的运行前检查见 check.rs 与 runner::preflight。
                                for p in graph.validate_draft(&ops) {
                                    problem(&format!("graph.{}", p.path), p.message);
                                }
                            }
                        },
                    }
                }
            }
            (_, None) => {}
        }
        // Kind specifics.
        match (self.kind, &self.narration) {
            (Kind::Workflow, None) => {
                if !ops.contains("run") {
                    problem("worker.ops", "workflow 扩展必须提供 run".into());
                }
            }
            (Kind::Workflow, Some(_)) => problem("narration", "只用于 kind = narration".into()),
            (Kind::Narration, None) => problem("narration", "kind = narration 时必需".into()),
            (Kind::Narration, Some(n)) => {
                for service in [Service::Chat, Service::Image, Service::Speech] {
                    if !self.requires.services.contains(&service) {
                        problem("requires.services", format!("口播流水线需要 {service:?}"));
                    }
                }
                if self.input.script.is_none() {
                    problem("input.script", "口播流水线需要文案输入".into());
                }
                if !(1..=8).contains(&n.planner.max_elements) {
                    problem("narration.planner.maxElements", "1~8".into());
                }
                match read(&n.planner.prompt) {
                    Some(text) if !text.trim().is_empty() => {}
                    _ => problem(
                        "narration.planner.prompt",
                        format!("找不到提示词 {}", n.planner.prompt),
                    ),
                }
                match read(&n.assets.template).map(|t| placeholders(&t)) {
                    None => problem(
                        "narration.assets.template",
                        format!("找不到模板 {}", n.assets.template),
                    ),
                    Some(Err(e)) => problem("narration.assets.template", e),
                    Some(Ok(names)) => {
                        if !names.iter().any(|n| n == "draw") {
                            problem("narration.assets.template", "必须包含 {{draw}}".into());
                        }
                        let prompt_selects: BTreeSet<&str> = self
                            .params
                            .iter()
                            .filter_map(|p| match p {
                                Param::Select { id, options, .. }
                                    if options.iter().all(|o| o.prompt.is_some()) =>
                                {
                                    Some(id.as_str())
                                }
                                _ => None,
                            })
                            .collect();
                        for name in names
                            .iter()
                            .filter(|n| !["draw", "label", "role"].contains(&n.as_str()))
                        {
                            if !prompt_selects.contains(name.as_str()) {
                                problem(
                                    "narration.assets.template",
                                    format!(
                                        "{{{{{name}}}}} 不是每个选项都带 prompt 的 select 参数"
                                    ),
                                );
                            }
                        }
                    }
                }
                for role in ["hero", "support", "accent"] {
                    if !n.assets.roles.contains_key(role) {
                        problem("narration.assets.roles", format!("缺少 {role}"));
                    }
                }
                if !crate::extension::valid_size(&n.assets.size) {
                    problem("narration.assets.size", "须为 WxH".into());
                }
                if !(20..=1000).contains(&n.segment.target)
                    || n.segment.max < n.segment.target
                    || n.segment.max > 1200
                {
                    problem("narration.segment", "20 ≤ target ≤ max ≤ 1200".into());
                }
                if !(n.board_sec.is_finite() && (2.0..=60.0).contains(&n.board_sec)) {
                    problem("narration.boardSec", "2~60 秒".into());
                }
                for (path, op) in [("narration.bake", &n.bake), ("narration.plate", &n.plate)] {
                    if let Some(op) = op {
                        if !ops.contains(op.as_str()) {
                            problem(path, format!("工作程序没有操作 {op}"));
                        }
                    }
                }
                if let Some(subtitles) = &n.subtitles {
                    if !(6..=40).contains(&subtitles.max_chars) {
                        problem("narration.subtitles.maxChars", "6~40".into());
                    }
                }
                if let Some(pacing) = &n.pacing {
                    if let Err(e) = crate::extension::pacing_from(pacing) {
                        problem("narration.pacing", e);
                    }
                }
            }
        }
        for (service, limit) in &self.budget {
            if !self.requires.services.contains(service) || *limit == 0 {
                problem(
                    "budget",
                    format!("{service:?} 未在 requires.services 声明或上限为 0"),
                );
            }
        }
        for (i, preview) in self.previews.iter().enumerate() {
            if !valid_relative_path(preview) {
                problem(&format!("previews[{i}]"), "须为包内相对路径".into());
            }
        }
        problems
    }

    /// User values → complete parameters: defaults filled, unknown keys,
    /// out-of-range numbers and invalid options rejected, numbers snapped to `step`.
    pub fn resolve_params(
        &self,
        given: &BTreeMap<String, serde_json::Value>,
    ) -> Result<BTreeMap<String, serde_json::Value>, Problem> {
        use serde_json::Value;
        let reject = |id: &str, message: String| Problem {
            path: format!("params.{id}"),
            message,
        };
        for key in given.keys() {
            if !self.params.iter().any(|p| p.id() == key) {
                return Err(reject(key, "未声明的参数".into()));
            }
        }
        let mut out = BTreeMap::new();
        for param in &self.params {
            let value = given.get(param.id());
            let resolved = match param {
                Param::Select {
                    id,
                    default,
                    options,
                    ..
                } => {
                    let v = value
                        .map_or(Some(default.as_str()), Value::as_str)
                        .ok_or_else(|| reject(id, "须为字符串".into()))?;
                    if !options.iter().any(|o| o.value == v) {
                        return Err(reject(id, format!("没有选项 {v:?}")));
                    }
                    Value::from(v)
                }
                Param::Number {
                    id,
                    default,
                    min,
                    max,
                    step,
                    ..
                } => {
                    let v = value
                        .map_or(Some(*default), Value::as_f64)
                        .ok_or_else(|| reject(id, "须为数字".into()))?;
                    if !v.is_finite() || v < *min || v > *max {
                        return Err(reject(id, format!("须在 {min}~{max}")));
                    }
                    let snapped = min + ((v - min) / step).round() * step;
                    let snapped = (snapped.min(*max) * 1e9).round() / 1e9;
                    serde_json::Number::from_f64(snapped)
                        .map(Value::Number)
                        .unwrap_or(Value::Null)
                }
                Param::Text {
                    id,
                    default,
                    max_length,
                    charset,
                    ..
                } => {
                    let v = value
                        .map_or(Some(default.as_str()), Value::as_str)
                        .ok_or_else(|| reject(id, "须为字符串".into()))?;
                    if v.chars().count() > *max_length {
                        return Err(reject(id, format!("最多 {max_length} 字")));
                    }
                    if let Some(extra) = charset {
                        if !v.chars().all(|c| c.is_alphanumeric() || extra.contains(c)) {
                            return Err(reject(id, "含有不允许的字符".into()));
                        }
                    }
                    Value::from(v)
                }
                Param::Bool { id, default, .. } => Value::from(
                    value
                        .map_or(Some(*default), Value::as_bool)
                        .ok_or_else(|| reject(id, "须为 true/false".into()))?,
                ),
                Param::Voice { id, default, .. } => {
                    let v = value
                        .map_or(Some(default.as_str()), Value::as_str)
                        .ok_or_else(|| reject(id, "须为音色 id".into()))?;
                    if v.is_empty()
                        || v.len() > 64
                        || !v
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
                    {
                        return Err(reject(id, "音色 id 不合法".into()));
                    }
                    Value::from(v)
                }
            };
            out.insert(param.id().to_owned(), resolved);
        }
        Ok(out)
    }

    /// `{{name}}` values from select parameters whose options all carry prompts.
    pub fn prompt_vars(
        &self,
        params: &BTreeMap<String, serde_json::Value>,
    ) -> BTreeMap<String, String> {
        self.params
            .iter()
            .filter_map(|p| match p {
                Param::Select { id, options, .. } if options.iter().all(|o| o.prompt.is_some()) => {
                    let chosen = params.get(id)?.as_str()?;
                    let prompt = options.iter().find(|o| o.value == chosen)?.prompt.clone()?;
                    Some((id.clone(), prompt))
                }
                _ => None,
            })
            .collect()
    }
}
