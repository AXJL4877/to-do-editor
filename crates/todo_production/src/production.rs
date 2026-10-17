//! The production document: one narration from outline to timeline.
//! production → segments (one voice each) → boards → elements → variants.
//! Every generated node remembers the task key that produced it; a node whose
//! current request has a different key is stale. Edits never generate; they
//! change the document and the next planning pass finds what to redo.
use crate::segment::{CastMember, Limits};
use crate::storyboard::{Role, DRAW_MAX, LABEL_MAX};
use crate::subtitles::Cue;
use crate::tasks::TaskKey;
use crate::text::{char_count, js_round, js_slice, js_trim};
use crate::voice::Timeline;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PRODUCTION_SPEC: &str = "todo-production/2";
pub const SEGMENT_TEXT_MAX: usize = 1200;
pub const MAX_VARIANTS: usize = 6;
pub const MAX_EDITS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SplitMode {
    /// The model chooses cut points by meaning.
    Semantic,
    Paragraph,
    Length,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Ai,
    User,
    Fallback,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackRef {
    pub id: String,
    pub version: String,
}

/// How the script was cut into segments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineInfo {
    pub source: Source,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Voice {
    /// The speech task that produced it.
    pub task: TaskKey,
    /// Audio file (artifact path relative to the project or cache).
    pub audio: String,
    pub timeline: Timeline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanState {
    /// Planning inputs of the last AI plan (voice, pacing, cast, prompt, model);
    /// not the request itself, whose neighbour context must not force replans.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<String>,
    /// The voice the phrase anchors refer to.
    pub voice: TaskKey,
    pub source: Source,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VariantSource {
    Ai,
    Upload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variant {
    pub id: String,
    pub source: VariantSource,
    /// The image task that produced `file`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Chosen by the user: never regenerated automatically.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Element {
    pub id: String,
    /// Phrase index where drawing starts.
    pub at: usize,
    pub label: String,
    pub draw: String,
    pub role: Role,
    pub weight: f64,
    pub variants: Vec<Variant>,
    pub pick: usize,
}

impl Element {
    pub fn picked(&self) -> Option<&Variant> {
        self.variants
            .get(self.pick)
            .or_else(|| self.variants.first())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub id: String,
    /// Phrase index where the board appears.
    pub start: usize,
    pub elements: Vec<Element>,
    /// Baked layers and the inputs fingerprint they were made from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render: Option<Rendered>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rendered {
    pub fingerprint: String,
    pub result: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subtitles {
    /// Voice and options they were cut from.
    pub fingerprint: String,
    pub cues: Vec<Cue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<Voice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanState>,
    #[serde(default)]
    pub boards: Vec<Board>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitles: Option<Subtitles>,
}

/// What reached the timeline: artifact key → revision, and keys the user deleted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    /// Timeline position of the first segment, seconds.
    pub t0: f64,
    pub auto: bool,
    #[serde(default)]
    pub placed: BTreeMap<String, String>,
    /// (key, revision) pairs removed by the user: not placed again unless regenerated.
    #[serde(default)]
    pub tombstones: BTreeSet<(String, String)>,
    /// Where each key was last put by the production. A clip elsewhere was
    /// moved by the user and keeps its position.
    #[serde(default)]
    pub positions: BTreeMap<String, f64>,
    /// The transform each key was last given by the production
    /// (`x y scale scaleX scaleY opacity`); a different one is the user's.
    #[serde(default)]
    pub transforms: BTreeMap<String, [f64; 6]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRecord {
    /// Extension, input and parameters the run was made from.
    pub fingerprint: String,
    #[serde(default)]
    pub items: Vec<crate::extension::items::Item>,
    /// The worker's `done.result`, once finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Production {
    pub spec: String,
    pub id: String,
    pub pack: PackRef,
    pub title: String,
    #[serde(default)]
    pub title_locked: bool,
    /// Cleaned narration; always the segments joined by blank lines once outlined.
    pub script: String,
    pub params: BTreeMap<String, serde_json::Value>,
    pub split: SplitMode,
    pub limits: Limits,
    #[serde(default)]
    pub cast: Vec<CastMember>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<OutlineInfo>,
    #[serde(default)]
    pub segments: Vec<Segment>,
    #[serde(default)]
    pub placement: Placement,
    /// The baked background plate (recipes with a plate operation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plate: Option<Rendered>,
    /// The last run of a `workflow` extension and the items it produced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<WorkflowRecord>,
    /// Saved revision; incremented on every persisted change.
    pub rev: u64,
    #[serde(default)]
    pub counters: BTreeMap<char, u32>,
}

/// A rejected edit; the message is shown to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditError(pub String);

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EditError {}

fn fail<T>(message: impl Into<String>) -> Result<T, EditError> {
    Err(EditError(message.into()))
}

/// Edit input text: line breaks normalised, braces removed, trimmed, bounded.
fn edit_text(value: &str, max: usize) -> String {
    let normalised = value.replace("\r\n", "\n").replace('\r', "\n");
    let stripped: String = normalised
        .chars()
        .filter(|c| *c != '{' && *c != '}')
        .collect();
    js_slice(js_trim(&stripped), max)
}

impl Production {
    pub fn new(
        id: impl Into<String>,
        pack: PackRef,
        script: &str,
        params: BTreeMap<String, serde_json::Value>,
        split: SplitMode,
        limits: Limits,
    ) -> Self {
        Self {
            spec: PRODUCTION_SPEC.into(),
            id: id.into(),
            pack,
            title: String::new(),
            title_locked: false,
            script: crate::text::clean_script(script),
            params,
            split,
            limits,
            cast: Vec::new(),
            outline: None,
            segments: Vec::new(),
            placement: Placement {
                auto: true,
                ..Placement::default()
            },
            plate: None,
            workflow: None,
            rev: 0,
            counters: BTreeMap::new(),
        }
    }

    /// `g` segment, `b` board, `e` element, `v` variant.
    pub fn next_id(&mut self, kind: char) -> String {
        let counter = self.counters.entry(kind).or_insert(0);
        *counter += 1;
        format!("{kind}{counter}")
    }

    pub fn new_segment(&mut self, text: String, title: String) -> Segment {
        Segment {
            id: self.next_id('g'),
            title,
            text,
            voice: None,
            plan: None,
            boards: Vec::new(),
            subtitles: None,
        }
    }

    pub fn new_variant(&mut self) -> Variant {
        Variant {
            id: self.next_id('v'),
            source: VariantSource::Ai,
            task: None,
            file: None,
            locked: false,
            error: None,
        }
    }

    pub fn new_element(
        &mut self,
        at: usize,
        label: &str,
        draw: &str,
        role: Role,
        weight: Option<f64>,
        variants: usize,
    ) -> Element {
        let label = js_slice(js_trim(label), LABEL_MAX);
        Element {
            id: self.next_id('e'),
            at,
            label: if label.is_empty() {
                "元素".into()
            } else {
                label
            },
            draw: js_slice(js_trim(draw), DRAW_MAX),
            role,
            weight: weight
                .filter(|w| w.is_finite())
                .map_or(if role == Role::Hero { 1.5 } else { 1.0 }, |w| {
                    w.clamp(0.5, 3.0)
                }),
            variants: (0..variants.max(1)).map(|_| self.new_variant()).collect(),
            pick: 0,
        }
    }

    fn sync_script(&mut self) {
        self.script = self
            .segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
    }

    fn segment_index(&self, id: &str) -> Result<usize, EditError> {
        self.segments
            .iter()
            .position(|s| s.id == id)
            .map_or_else(|| fail(format!("找不到口播段 {id}")), Ok)
    }

    fn board_index(&self, id: &str) -> Result<(usize, usize), EditError> {
        for (g, segment) in self.segments.iter().enumerate() {
            if let Some(b) = segment.boards.iter().position(|b| b.id == id) {
                return Ok((g, b));
            }
        }
        fail(format!("找不到镜头 {id}"))
    }

    fn element_index(&self, id: &str) -> Result<(usize, usize, usize), EditError> {
        for (g, segment) in self.segments.iter().enumerate() {
            for (b, board) in segment.boards.iter().enumerate() {
                if let Some(e) = board.elements.iter().position(|e| e.id == id) {
                    return Ok((g, b, e));
                }
            }
        }
        fail(format!("找不到元素 {id}"))
    }

    /// Apply a batch: all edits or none.
    pub fn apply_edits(&mut self, edits: &[Edit], max_elements: usize) -> Result<(), EditError> {
        if edits.is_empty() {
            return fail("没有编辑操作");
        }
        if edits.len() > MAX_EDITS {
            return fail(format!("一次最多 {MAX_EDITS} 个编辑操作"));
        }
        let mut draft = self.clone();
        for edit in edits {
            draft.apply(edit, max_elements)?;
        }
        *self = draft;
        Ok(())
    }

    fn touch_plan(&mut self, g: usize) {
        // A user-edited storyboard is not replanned while the voice stays the same.
        if let Some(plan) = &mut self.segments[g].plan {
            plan.source = Source::User;
        }
    }

    /// A new description means a new candidate; old ones stay selectable.
    fn push_variant(&mut self, g: usize, b: usize, e: usize, variant: Variant) {
        let element = &mut self.segments[g].boards[b].elements[e];
        if element.variants.len() >= MAX_VARIANTS {
            if let Some(drop) = (0..element.variants.len()).find(|&i| i != element.pick) {
                element.variants.remove(drop);
                if element.pick > drop {
                    element.pick -= 1;
                }
            }
        }
        element.variants.push(variant);
        element.pick = element.variants.len() - 1;
    }

    fn apply(&mut self, edit: &Edit, max_elements: usize) -> Result<(), EditError> {
        match edit {
            Edit::SetTitle { title } => {
                let title = edit_text(title, 24);
                if !title.is_empty() {
                    self.title = title;
                    self.title_locked = true;
                }
            }
            Edit::SetParams { params } => self.params = params.clone(),
            Edit::SetSegmentText { segment, text } => {
                let g = self.segment_index(segment)?;
                let text = edit_text(text, SEGMENT_TEXT_MAX);
                if char_count(&text) == 0 {
                    return fail("口播段里没有可朗读的文字");
                }
                if text != self.segments[g].text {
                    self.segments[g].text = text;
                    // New voice: old phrase anchors are meaningless, the AI replans.
                    if let Some(plan) = &mut self.segments[g].plan {
                        plan.source = Source::Ai;
                    }
                    self.sync_script();
                }
            }
            Edit::SplitSegment { segment, at } => {
                let g = self.segment_index(segment)?;
                let chars: Vec<char> = self.segments[g].text.chars().collect();
                if *at == 0 || *at >= chars.len() {
                    return fail("切分位置要在这段文字中间");
                }
                let a: String = chars[..*at].iter().collect();
                let b: String = chars[*at..].iter().collect();
                let (a, b) = (js_trim(&a).to_owned(), js_trim(&b).to_owned());
                if a.is_empty() || b.is_empty() {
                    return fail("切分后两边都要有文字");
                }
                let second = self.new_segment(b, String::new());
                self.segments[g].text = a;
                if let Some(plan) = &mut self.segments[g].plan {
                    plan.source = Source::Ai;
                }
                self.segments.insert(g + 1, second);
                self.sync_script();
            }
            Edit::MergeSegment { segment } => {
                let g = self.segment_index(segment)?;
                let Some(next) = self.segments.get(g + 1) else {
                    return fail("已经是最后一段，没有可合并的下一段");
                };
                let merged = format!("{}\n{}", self.segments[g].text, next.text);
                if merged.chars().count() > SEGMENT_TEXT_MAX {
                    return fail(format!("合并后超过 {SEGMENT_TEXT_MAX} 字"));
                }
                self.segments[g].text = merged;
                if let Some(plan) = &mut self.segments[g].plan {
                    plan.source = Source::Ai;
                }
                self.segments.remove(g + 1);
                self.sync_script();
            }
            Edit::RemoveSegment { segment } => {
                let g = self.segment_index(segment)?;
                if self.segments.len() <= 1 {
                    return fail("至少要留一段口播");
                }
                self.segments.remove(g);
                self.sync_script();
            }
            Edit::ReplanSegment { segment } => {
                let g = self.segment_index(segment)?;
                self.segments[g].plan = None;
            }
            Edit::SetElement { element, patch } => {
                let (g, b, e) = self.element_index(element)?;
                let draw = patch.draw.as_deref().map(|d| edit_text(d, DRAW_MAX));
                if draw.as_deref() == Some("") {
                    return fail("画面描述不能为空");
                }
                let target = &mut self.segments[g].boards[b].elements[e];
                if let Some(label) = patch
                    .label
                    .as_deref()
                    .map(|l| edit_text(l, LABEL_MAX))
                    .filter(|l| !l.is_empty())
                {
                    target.label = label;
                }
                if let Some(role) = patch.role {
                    target.role = role;
                }
                if let Some(weight) = patch.weight {
                    if !weight.is_finite() {
                        return fail("权重须为数字");
                    }
                    target.weight = ((weight * 10.0).round() / 10.0).clamp(0.5, 3.0);
                }
                if let Some(at) = patch.at {
                    target.at = at;
                }
                if let Some(draw) = draw.filter(|d| *d != target.draw) {
                    target.draw = draw;
                    let variant = self.new_variant();
                    self.push_variant(g, b, e, variant);
                }
                self.touch_plan(g);
            }
            Edit::AddElement { board, element } => {
                let (g, b) = self.board_index(board)?;
                let target = &self.segments[g].boards[b];
                if target.elements.len() >= max_elements {
                    return fail(format!("每个镜头最多 {max_elements} 个元素"));
                }
                let draw = edit_text(&element.draw, DRAW_MAX);
                if draw.is_empty() {
                    return fail("新元素要有画面描述");
                }
                let at = element
                    .at
                    .unwrap_or_else(|| target.elements.last().map_or(target.start, |e| e.at));
                let role = element.role.unwrap_or(if target.elements.is_empty() {
                    Role::Hero
                } else {
                    Role::Support
                });
                let new = self.new_element(
                    at,
                    element.label.as_deref().unwrap_or_default(),
                    &draw,
                    role,
                    element.weight,
                    1,
                );
                let elements = &mut self.segments[g].boards[b].elements;
                elements.push(new);
                elements.sort_by_key(|e| e.at);
                self.touch_plan(g);
            }
            Edit::RemoveElement { element } => {
                let (g, b, e) = self.element_index(element)?;
                let boards = &mut self.segments[g].boards;
                boards[b].elements.remove(e);
                if boards[b].elements.is_empty() {
                    boards.remove(b);
                }
                self.touch_plan(g);
            }
            Edit::MoveElement { element, delta } => {
                let (g, b, e) = self.element_index(element)?;
                let elements = &mut self.segments[g].boards[b].elements;
                let j = if *delta < 0 {
                    e.checked_sub(1)
                } else {
                    Some(e + 1)
                };
                if let Some(j) = j.filter(|j| *j < elements.len()) {
                    // Writing order is anchor order: swap the anchors too.
                    elements.swap(e, j);
                    let (a, c) = (elements[e].at, elements[j].at);
                    elements[e].at = c;
                    elements[j].at = a;
                    self.touch_plan(g);
                }
            }
            Edit::AddVariant { element } => {
                let (g, b, e) = self.element_index(element)?;
                if self.segments[g].boards[b].elements[e].variants.len() >= MAX_VARIANTS {
                    return fail(format!("每个元素最多留 {MAX_VARIANTS} 张候选，先删掉一张"));
                }
                let variant = self.new_variant();
                self.push_variant(g, b, e, variant);
            }
            Edit::PickVariant { element, variant } => {
                let (g, b, e) = self.element_index(element)?;
                let target = &mut self.segments[g].boards[b].elements[e];
                let Some(k) = target.variants.iter().position(|v| v.id == *variant) else {
                    return fail("找不到这张候选");
                };
                target.pick = k;
                target.variants[k].locked = true;
            }
            Edit::RemoveVariant { element, variant } => {
                let (g, b, e) = self.element_index(element)?;
                let target = &mut self.segments[g].boards[b].elements[e];
                if target.variants.len() <= 1 {
                    return fail("至少留一张候选");
                }
                let Some(k) = target.variants.iter().position(|v| v.id == *variant) else {
                    return fail("找不到这张候选");
                };
                target.variants.remove(k);
                if target.pick >= k {
                    target.pick = target.pick.saturating_sub(1);
                }
            }
            Edit::UploadVariant { element, file } => {
                let (g, b, e) = self.element_index(element)?;
                if js_trim(file).is_empty() {
                    return fail("上传的图片不存在");
                }
                let mut variant = self.new_variant();
                variant.source = VariantSource::Upload;
                variant.file = Some(file.clone());
                variant.locked = true;
                self.push_variant(g, b, e, variant);
            }
            Edit::AddBoard {
                segment,
                start,
                element,
            } => {
                let g = self.segment_index(segment)?;
                let draw = edit_text(&element.draw, DRAW_MAX);
                if draw.is_empty() {
                    return fail("新镜头至少要有一个带画面描述的元素");
                }
                let first = self.new_element(
                    *start,
                    element.label.as_deref().unwrap_or("新元素"),
                    &draw,
                    element.role.unwrap_or(Role::Hero),
                    element.weight,
                    1,
                );
                let board = Board {
                    id: self.next_id('b'),
                    start: *start,
                    elements: vec![first],
                    render: None,
                };
                let boards = &mut self.segments[g].boards;
                boards.push(board);
                boards.sort_by_key(|b| b.start);
                self.touch_plan(g);
            }
            Edit::RemoveBoard { board } => {
                let (g, b) = self.board_index(board)?;
                if self.segments[g].boards.len() <= 1 {
                    return fail("每段至少留一个镜头；不想要这段画面可以删掉整段");
                }
                self.segments[g].boards.remove(b);
                self.touch_plan(g);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ElementPatch {
    pub label: Option<String>,
    pub draw: Option<String>,
    pub role: Option<Role>,
    pub weight: Option<f64>,
    pub at: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewElement {
    pub label: Option<String>,
    pub draw: String,
    pub role: Option<Role>,
    pub weight: Option<f64>,
    pub at: Option<usize>,
}

/// Document edits (JSON `op` names match the previous editor).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Edit {
    SetTitle {
        title: String,
    },
    SetParams {
        params: BTreeMap<String, serde_json::Value>,
    },
    SetSegmentText {
        segment: String,
        text: String,
    },
    /// Split before character `at`.
    SplitSegment {
        segment: String,
        at: usize,
    },
    MergeSegment {
        segment: String,
    },
    RemoveSegment {
        segment: String,
    },
    ReplanSegment {
        segment: String,
    },
    SetElement {
        element: String,
        patch: ElementPatch,
    },
    AddElement {
        board: String,
        element: NewElement,
    },
    RemoveElement {
        element: String,
    },
    MoveElement {
        element: String,
        delta: i32,
    },
    AddVariant {
        element: String,
    },
    PickVariant {
        element: String,
        variant: String,
    },
    RemoveVariant {
        element: String,
        variant: String,
    },
    UploadVariant {
        element: String,
        file: String,
    },
    AddBoard {
        segment: String,
        start: usize,
        element: NewElement,
    },
    RemoveBoard {
        board: String,
    },
}

/// `Math.max(0, Math.round(Number(x) || 0))` for loosely typed anchors.
pub fn anchor(value: f64) -> usize {
    if value.is_finite() {
        js_round(value).max(0.0) as usize
    } else {
        0
    }
}
