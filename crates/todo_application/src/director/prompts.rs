//! What the director asks models, and the strict parsing of their answers.
use super::{StyleAsset, StyleInfo};
use crate::control::review::{Check, Rubric};
use crate::control::{project_view, ControlEdit};
use crate::Project;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fmt::Write;
use todo_core::{clip_end, find_clip, motion::builtin_motions, MotionKind};
use todo_production::{ChatMessage, ChatRole};

fn system(content: String) -> ChatMessage {
    ChatMessage {
        role: ChatRole::System,
        content,
    }
}

fn user(content: String) -> ChatMessage {
    ChatMessage {
        role: ChatRole::User,
        content,
    }
}

/// The first JSON object in a model answer (models like code fences).
pub fn json_object(text: &str) -> Result<Value, String> {
    let start = text.find('{').ok_or("回答里没有 JSON")?;
    let end = text.rfind('}').ok_or("回答里没有 JSON")?;
    if end < start {
        return Err("回答里没有 JSON".into());
    }
    serde_json::from_str(&text[start..=end]).map_err(|e| format!("JSON 无法解析：{e}"))
}

// ------------------------------------------------------------ understand

pub enum Mode {
    Produce {
        extension: String,
        params: BTreeMap<String, Value>,
        script: String,
    },
    Edit,
}

pub struct Intent {
    pub mode: Mode,
    pub reply: String,
}

pub fn understand(request: &str, styles: &[StyleInfo], project: &Project) -> Vec<ChatMessage> {
    let mut catalog = String::new();
    for style in styles {
        let _ = writeln!(
            catalog,
            "- id `{}`：{}。{}\n  参数：{}",
            style.id,
            style.name,
            style.description,
            style.params.join("；")
        );
    }
    let clips: usize = project.tracks.iter().map(|t| t.clips.len()).sum();
    vec![
        system(format!(
            "你是视频剪辑软件 TO DO 的内置导演 Agent。先判断用户要做什么：\n\
1. `produce`：从零做一支新视频（给了主题或文案，要某种风格）。必须从下面的风格里选一个 extension。\
参数只填用户明确说到的（例如主题、时长）；用户没提的参数一律不要填，交给风格的默认值。\n\
2. `edit`：修改当前工程里已有的内容（调位置、改字、加转场、删片段等）。\n\n\
可用风格：\n{catalog}\n当前工程有 {clips} 个片段。\n\n\
只输出一个 JSON 对象：\n\
{{\"mode\":\"produce\",\"extension\":\"风格 id\",\"params\":{{\"topic\":\"…\",\"seconds\":\"30\"}},\"script\":\"用户给出的完整文案，没有就留空\",\"reply\":\"一句话告诉用户你要做什么\"}}\n\
或 {{\"mode\":\"edit\",\"reply\":\"…\"}}"
        )),
        user(request.to_owned()),
    ]
}

pub fn parse_intent(text: &str, styles: &[StyleInfo]) -> Result<Intent, String> {
    let value = json_object(text)?;
    let reply = value["reply"].as_str().unwrap_or_default().to_owned();
    match value["mode"].as_str() {
        Some("produce") => {
            let extension = value["extension"].as_str().unwrap_or_default().to_owned();
            if !styles.iter().any(|s| s.id == extension) {
                return Err(format!("没有风格 {extension}"));
            }
            let params = value["params"]
                .as_object()
                .map(|map| {
                    map.iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default();
            Ok(Intent {
                mode: Mode::Produce {
                    extension,
                    params,
                    script: value["script"]
                        .as_str()
                        .unwrap_or_default()
                        .trim()
                        .to_owned(),
                },
                reply,
            })
        }
        Some("edit") => Ok(Intent {
            mode: Mode::Edit,
            reply,
        }),
        other => Err(format!("未知的 mode {other:?}")),
    }
}

// ------------------------------------------------------------ the document

/// One line per element: what a model needs to name and place things.
pub fn compact_view(project: &Project, at: Option<f64>) -> String {
    let view = project_view(project, at);
    let mut out = format!(
        "画布 {}x{}，时长 {:.2}s。元素（id | 轨道 | 类型 | 名称 | 时间 | 画面矩形 x,y,w,h（画布比例）| 变换 | 动效）：\n",
        view.canvas.width, view.canvas.height, view.content_duration
    );
    let order: Vec<String> = view
        .tracks
        .iter()
        .map(|t| format!("{}({:?})", t.id, t.kind).to_lowercase())
        .collect();
    let _ = writeln!(
        out,
        "轨道从上到下（上面的盖住下面的）：{}",
        order.join(" > ")
    );
    for element in view.elements.iter().take(160) {
        let rect = element
            .rect
            .map(|r| format!("{:.2},{:.2},{:.2},{:.2}", r.nx, r.ny, r.nw, r.nh))
            .unwrap_or_else(|| "-".into());
        let t = element.effective.unwrap_or(element.transform);
        let mut motion = Vec::new();
        if let Some(m) = &element.intro {
            motion.push(format!("入场 {}", m.kind));
        }
        if let Some(m) = &element.outro {
            motion.push(format!("出场 {}", m.kind));
        }
        if let Some(m) = &element.transition {
            motion.push(format!(
                "转场 {}{}",
                m.kind,
                if m.active { "" } else { "(未生效)" }
            ));
        }
        let name: String = element.name.chars().take(18).collect();
        let _ = writeln!(
            out,
            "{} | {} | {} | {} | {:.2}–{:.2}s | {} | x {:.2} y {:.2} s {:.2} o {:.2} | {}",
            element.clip_id,
            element.track_id,
            element.kind,
            name,
            element.start,
            element.end,
            rect,
            t.x,
            t.y,
            t.scale,
            t.opacity,
            motion.join("，")
        );
    }
    out
}

/// 样片结构 + 四条路线 + 动效映射 + 硬约束。进 planning prompt，控制篇幅与节奏。
pub const ROUTE_BRIEF: &str = r#"【样片与路线】根目录样片：13 镜 / 179s，平均 13.8s（4.4–36.6s）；字幕 60 条，单行 ≤14 字、中位 2.16s；开场黑底毛笔红字 1.5s；结尾黑底标题 ≥5s；每镜 2–4 个元素。
路线：A 样板复刻＝句群切镜，每镜 1–3 句且 ≥3.5s｜B 讲解＝一句一镜 + 关键词大字｜C 对比＝左右分栏 + 数字强调｜D 卡点＝1.5–2.5s 一镜、切点贴拍点。
动效映射：换镜 dissolve 0.15；卡片入场 pop 或 rise 0.3–0.4；退场 fadeOut 0.3；强调 stamp 0.35、grow 0.4；对比揭示 pushLeft 0.4；结尾卡 fade 0.4。
硬约束：① 只用上面和列表里的 motion id，不许造；② 一次 edits 只改一个镜头或一组字幕；③ 不动底板之外图层的 z 序；④ 字幕时间来自配音时间戳，不许估；⑤ 修复只动 transform/motion/start/duration，不删素材。"#;

fn motion_ids(project: &Project, kind: MotionKind) -> String {
    builtin_motions()
        .iter()
        .chain(&project.motions)
        .filter(|p| p.kind == kind)
        .map(|p| format!("{}（{}）", p.id, p.name))
        .collect::<Vec<_>>()
        .join("、")
}

fn edit_rules(project: &Project) -> String {
    format!(
        "可用编辑（ControlEdit，type 用 snake_case，字段 camelCase；clip 引用写 {{\"type\":\"id\",\"id\":\"片段 id\"}}）：\n\
- {{\"type\":\"set_transform\",\"clip\":…,\"transform\":{{\"x\":0.1,\"y\":0.05,\"scale\":0.6}}}}  x/y 是相对画布中心的偏移（画布宽/高的比例），只写要改的字段\n\
- {{\"type\":\"set_text\",\"clip\":…,\"text\":\"新文字\"}}\n\
- {{\"type\":\"trim_clip\",\"clip\":…,\"edge\":\"out\",\"timelineTime\":5.2,\"linked\":false}}\n\
- {{\"type\":\"move_clips\",\"clips\":[…],\"delta\":0.5,\"trackId\":null,\"linked\":false}}\n\
- {{\"type\":\"delete_clips\",\"clips\":[…],\"ripple\":false,\"linked\":false}}\n\
- {{\"type\":\"set_intro\",\"clip\":…,\"intro\":{{\"kind\":\"pop\",\"duration\":0.3}}}}  入场可选：{}\n\
- {{\"type\":\"set_outro\",\"clip\":…,\"outro\":{{\"kind\":\"fadeOut\",\"duration\":0.3}}}}  出场可选：{}\n\
- {{\"type\":\"set_transition\",\"clip\":…,\"transition\":{{\"kind\":\"dissolve\",\"duration\":0.3}}}}  转场（从同一轨道紧挨着的前一个片段切进来）可选：{}\n\
任何字段写 null 表示去掉该动效。不要编造 id。",
        motion_ids(project, MotionKind::Intro),
        motion_ids(project, MotionKind::Outro),
        motion_ids(project, MotionKind::Transition)
    )
}

const PLAN_FORMAT: &str =
    "只输出一个 JSON 对象：{\"notes\":\"一句话说明改了什么\",\"edits\":[…]};没有要改的就输出空数组。\n示例（照抄字段名，action 名用列表里的 type）：\n{\"notes\":\"第二镜换叠化并入场上浮\",\"edits\":[\n{\"type\":\"set_transition\",\"clip\":{\"type\":\"id\",\"id\":\"c02\"},\"transition\":{\"kind\":\"dissolve\",\"duration\":0.15}},\n{\"type\":\"set_intro\",\"clip\":{\"type\":\"id\",\"id\":\"c02\"},\"intro\":{\"kind\":\"rise\",\"duration\":0.35}}]}";

pub fn edit(request: &str, assets: &[StyleAsset], project: &Project) -> Vec<ChatMessage> {
    let mut library = String::new();
    for asset in assets {
        let _ = writeln!(library, "- {}：{}（{}）", asset.id, asset.name, asset.desc);
    }
    let pictures = format!(
        "用户要换画面时写在 pictures 里（最多 {} 张）：\n\
- 从风格素材库换：{{\"clip\":\"片段 id\",\"library\":\"素材 id\"}}\n\
- 库里没有合适的就生成：{{\"clip\":\"片段 id\",\"draw\":\"英文画面描述\"}}\n\
素材库：\n{}",
        super::MAX_PICTURES_PER_PLAN,
        if library.is_empty() {
            "（无）\n".to_owned()
        } else {
            library
        }
    );
    vec![
        system(format!(
            "你是 TO DO 的剪辑 Agent，按用户要求修改当前工程。只改用户提到的内容。\n\n{ROUTE_BRIEF}\n\n{}\n\n{}\n\n{pictures}\n{PLAN_FORMAT}",
            compact_view(project, None),
            edit_rules(project)
        )),
        user(request.to_owned()),
    ]
}

pub struct EditPlan {
    pub notes: String,
    pub edits: Vec<ControlEdit>,
    pub pictures: Vec<PictureFix>,
    /// Edits that could not be read; skipped, reported to the log.
    pub skipped: Vec<String>,
}

pub enum PictureChoice {
    Library(String),
    Draw(String),
}

pub struct PictureFix {
    pub clip: String,
    pub source: PictureChoice,
}

/// Parse edits strictly after completing the shorthands models write:
/// partial transforms (filled from the clip), `set_text` with plain text,
/// `clipIds` lists.
pub fn parse_edits(text: &str, project: Option<&Project>) -> Result<EditPlan, String> {
    let value = json_object(text)?;
    let notes = value["notes"].as_str().unwrap_or_default().to_owned();
    let raw = value["edits"].as_array().cloned().unwrap_or_default();
    let mut edits = Vec::new();
    let mut skipped = Vec::new();
    // Models sometimes put picture swaps among the edits.
    let mut picture_values: Vec<Value> = value["pictures"].as_array().cloned().unwrap_or_default();
    for (index, mut edit) in raw.into_iter().enumerate() {
        let kind = edit["type"].as_str().unwrap_or_default().to_owned();
        if matches!(
            kind.as_str(),
            "pictures" | "picture" | "replace_picture" | "swap_picture"
        ) {
            match edit.get("pictures").and_then(Value::as_array) {
                Some(list) => picture_values.extend(list.iter().cloned()),
                None => picture_values.push(edit),
            }
            continue;
        }
        if let Some(object) = edit.as_object_mut() {
            complete(object, project);
        }
        match serde_json::from_value::<ControlEdit>(edit) {
            Ok(parsed) => edits.push(parsed),
            Err(error) => skipped.push(format!(
                "第 {index} 个编辑（{kind}）不合法，已跳过：{error}"
            )),
        }
    }
    if edits.is_empty() && picture_values.is_empty() && !skipped.is_empty() {
        return Err(skipped.join("；"));
    }
    let mut pictures = Vec::new();
    for fix in &picture_values {
        let clip = fix["clip"]
            .as_str()
            .or_else(|| fix["clip"]["id"].as_str())
            .unwrap_or_default()
            .to_owned();
        let source = match (fix["library"].as_str(), fix["draw"].as_str()) {
            (Some(id), _) if !id.trim().is_empty() => PictureChoice::Library(id.trim().into()),
            (_, Some(prompt)) if !prompt.trim().is_empty() => {
                PictureChoice::Draw(prompt.trim().into())
            }
            _ => continue,
        };
        if !clip.is_empty() && project.is_none_or(|p| find_clip(p, &clip).is_some()) {
            pictures.push(PictureFix { clip, source });
        }
    }
    Ok(EditPlan {
        notes,
        edits,
        pictures,
        skipped,
    })
}

fn clip_ref(value: &Value) -> Option<Value> {
    match value {
        Value::String(id) => Some(json!({"type": "id", "id": id})),
        Value::Object(_) => Some(value.clone()),
        _ => None,
    }
}

fn complete(object: &mut Map<String, Value>, project: Option<&Project>) {
    if let Some(clip) = object.get("clip").and_then(clip_ref) {
        object.insert("clip".into(), clip);
    }
    if let Some(ids) = object.remove("clipIds").and_then(|v| v.as_array().cloned()) {
        object.insert(
            "clips".into(),
            Value::Array(ids.iter().filter_map(clip_ref).collect()),
        );
    }
    if let Some(Value::Array(clips)) = object.get("clips").cloned() {
        object.insert(
            "clips".into(),
            Value::Array(clips.iter().filter_map(clip_ref).collect()),
        );
    }
    let clip_id = object
        .get("clip")
        .and_then(|c| c.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let current = project
        .zip(clip_id.as_deref())
        .and_then(|(p, id)| find_clip(p, id));
    match object.get("type").and_then(Value::as_str) {
        Some("set_transform") => {
            let mut full = current
                .as_ref()
                .map(|found| serde_json::to_value(found.clip.transform).unwrap())
                .unwrap_or_else(
                    || json!({"x":0.0,"y":0.0,"scale":1.0,"scaleX":1.0,"scaleY":1.0,"opacity":1.0}),
                );
            if let Some(Value::Object(given)) = object.get("transform") {
                for (k, v) in given {
                    full[k] = v.clone();
                }
            }
            object.insert("transform".into(), full);
        }
        Some("set_text") if !object.contains_key("content") => {
            let text = object
                .remove("text")
                .unwrap_or(Value::String(String::new()));
            let mut content = current
                .as_ref()
                .and_then(|found| found.clip.text().cloned())
                .map(|content| serde_json::to_value(content).unwrap())
                .unwrap_or_else(|| json!({"styleId": "clean-white-outline"}));
            content["text"] = text;
            object.insert("content".into(), content);
        }
        Some("delete_clips" | "move_clips" | "trim_clip") => {
            object.entry("linked").or_insert(Value::Bool(false));
            if object.get("type").and_then(Value::as_str) == Some("delete_clips") {
                object.entry("ripple").or_insert(Value::Bool(false));
            }
            if object.get("type").and_then(Value::as_str) == Some("move_clips") {
                object.entry("trackId").or_insert(Value::Null);
            }
        }
        Some("set_intro") => {
            object.entry("intro").or_insert(Value::Null);
        }
        Some("set_outro") => {
            object.entry("outro").or_insert(Value::Null);
        }
        Some("set_transition") => {
            object.entry("transition").or_insert(Value::Null);
        }
        _ => {}
    }
}

// ------------------------------------------------------------ checklist

/// One picture the checklist looks at: a clip the style checks, at a moment
/// where it stands still, with what is being said meanwhile.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    /// Production key (stable across redraws; clip ids are not).
    pub key: String,
    pub clip_id: String,
    pub at: f64,
    pub line: String,
}

/// `*` matches any run of characters.
pub fn key_matches(pattern: &str, key: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == key,
        Some((head, rest)) => {
            let Some(tail) = key.strip_prefix(head) else {
                return false;
            };
            (0..=tail.len())
                .filter(|i| tail.is_char_boundary(*i))
                .any(|i| key_matches(rest, &tail[i..]))
        }
    }
}

/// The pictures the style checks, in time order. Each is looked at after
/// its entrance or transition has settled and before it leaves.
pub fn targets(project: &Project, rubric: &Rubric) -> Vec<Target> {
    let clips = || project.tracks.iter().flat_map(|t| &t.clips);
    let mut out: Vec<Target> = clips()
        .filter(|c| c.text().is_none())
        .filter_map(|c| {
            let key = &c.origin.as_ref()?.key;
            if !rubric.checked.iter().any(|p| key_matches(p, key)) {
                return None;
            }
            let (start, end) = (c.start, clip_end(c));
            let d = end - start;
            let low = start + (0.8f64).min(d * 0.5);
            let high = end - (0.45f64).min(d * 0.25);
            let at = (start + d * 0.6).clamp(low, high.max(low));
            let mut lines: Vec<(f64, &str)> = clips()
                .filter(|l| {
                    l.origin
                        .as_ref()
                        .is_some_and(|o| rubric.lines.iter().any(|p| o.key.starts_with(p.as_str())))
                        && l.start < end - 0.1
                        && clip_end(l) > start + 0.1
                })
                .filter_map(|l| Some((l.start, l.text()?.text.as_str())))
                .collect();
            lines.sort_by(|a, b| a.0.total_cmp(&b.0));
            Some(Target {
                key: key.clone(),
                clip_id: c.id.clone(),
                at: (at * 1000.0).round() / 1000.0,
                line: lines.iter().map(|(_, t)| *t).collect::<Vec<_>>().join(""),
            })
        })
        .collect();
    out.sort_by(|a, b| a.at.total_cmp(&b.at));
    out
}

/// The reviewer's only job: answer closed questions about each picture.
pub fn checklist(checks: &[Check], targets: &[Target]) -> String {
    let mut out = format!(
        "你是审片员，只做选择题：不写评语，不提建议。\
下面依次给出 {} 张节目帧，每张是成片里一张插画卡片停稳时的整帧画面（背景板和底部字幕也在画面里，只判断插画卡片本身）。\
对每张图逐题回答 \"yes\"、\"no\" 或 \"unsure\"。只有一眼就能确定不符合时才答 \"no\"；拿不准就答 \"unsure\"。\n\n题目：\n",
        targets.len()
    );
    for check in checks {
        let _ = writeln!(out, "- `{}`：{}", check.id, check.question);
    }
    out.push('\n');
    for (index, target) in targets.iter().enumerate() {
        let line = if target.line.is_empty() {
            "（无）"
        } else {
            target.line.as_str()
        };
        let _ = writeln!(out, "第 {index} 张：这时配音说的是「{line}」");
    }
    let example: Vec<String> = checks
        .iter()
        .map(|c| format!("\"{}\":\"yes\"", c.id))
        .collect();
    let _ = write!(
        out,
        "\n只输出一个 JSON 对象：{{\"cards\":[{{\"index\":0,\"answers\":{{{}}}}}]}}，每张图一项。",
        example.join(",")
    );
    out
}

/// The questions each picture failed (an explicit "no"); anything else,
/// including a missing or unreadable answer, counts as unsure and passes.
pub fn parse_checklist(
    text: &str,
    count: usize,
    checks: &[Check],
) -> Result<Vec<Vec<String>>, String> {
    let value = json_object(text)?;
    let list = value["cards"].as_array().ok_or("缺少 cards")?;
    let mut failed = vec![Vec::new(); count];
    for card in list {
        let Some(index) = card["index"]
            .as_u64()
            .map(|i| i as usize)
            .filter(|i| *i < count)
        else {
            continue;
        };
        for check in checks {
            let no = match &card["answers"][check.id.as_str()] {
                Value::Bool(b) => !b,
                Value::String(s) => {
                    matches!(s.trim().to_lowercase().as_str(), "no" | "否" | "不是")
                }
                _ => false,
            };
            if no && !failed[index].contains(&check.id) {
                failed[index].push(check.id.clone());
            }
        }
    }
    Ok(failed)
}

/// Question ids as the user reads them.
pub fn question_names(checks: &[Check], ids: &[String]) -> String {
    ids.iter()
        .map(|id| {
            checks
                .iter()
                .find(|c| &c.id == id)
                .map_or(id.clone(), |c| c.question.clone())
        })
        .collect::<Vec<_>>()
        .join("；")
}

#[cfg(test)]
mod route_brief_tests {
    use super::*;

    #[test]
    fn brief_carries_the_sample_numbers_and_constraints() {
        for needle in [
            "13 镜",
            "2.16s",
            "dissolve 0.15",
            "pop",
            "fadeOut",
            "不删素材",
        ] {
            assert!(ROUTE_BRIEF.contains(needle), "missing {needle}");
        }
    }

    #[test]
    fn edit_prompt_injects_the_brief() {
        let project = todo_core::create_project(None, None, None, None, None);
        let messages = edit("把第二个镜头换成上浮入场", &[], &project);
        let system = &messages[0].content;
        assert!(system.contains("样片与路线"), "brief not injected");
        assert!(system.contains("set_intro"), "edit rules missing");
    }
}
