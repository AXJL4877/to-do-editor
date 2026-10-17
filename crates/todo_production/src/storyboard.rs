//! Boards (shots) and their elements for one spoken segment, timed by the
//! phrase index of its voice: the model picks where boards change and where
//! each element starts, these rules make the plan valid and timed. Port of the
//! editor's `sanitizeNarrationPlan` / `fallbackNarrationPlan` /
//! `buildBoardMessages` / `assetPrompt`. Timing lives in [`crate::pacing`].
use crate::tasks::{ChatMessage, ChatRole};
use crate::text::{clean_text, js_number, js_round, js_slice, js_string};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_BOARDS: usize = 80;
pub const MAX_ELEMENTS: usize = 200;
pub const LABEL_MAX: usize = 12;
pub const DRAW_MAX: usize = 240;
/// Leaves the end of every board still (see `pacing`).
pub const BREATHING_RULE: &str =
    "留白：每块最后约四分之一的时间不要再安排新元素，让画面停下来给观众看清楚。";

/// A timed phrase of the segment's voice; `sentence` groups phrases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Phrase {
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub sentence: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Hero,
    Support,
    Accent,
}

impl Role {
    fn parse(value: Option<&serde_json::Value>) -> Option<Self> {
        match value.and_then(|v| v.as_str()) {
            Some("hero") => Some(Self::Hero),
            Some("support") => Some(Self::Support),
            Some("accent") => Some(Self::Accent),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hero => "hero",
            Self::Support => "support",
            Self::Accent => "accent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedElement {
    /// Phrase index where drawing starts.
    pub at: usize,
    pub label: String,
    /// English image description.
    pub draw: String,
    pub role: Role,
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedBoard {
    /// Phrase index where the board appears (a sentence start; the first is 0).
    pub start: usize,
    pub elements: Vec<PlannedElement>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardPlan {
    pub title: String,
    pub notes: String,
    pub scenes: Vec<PlannedBoard>,
}

fn get<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    value.as_object().and_then(|o| o.get(key))
}

fn clean(value: Option<&serde_json::Value>) -> String {
    clean_text(&js_string(value))
}

/// Model output → a valid plan: board changes on sentence starts, increasing,
/// the first at 0; element anchors clamped into their board and sorted.
pub fn sanitize_plan(
    raw: &serde_json::Value,
    max_elements_per_board: usize,
    phrases: &[Phrase],
    max_boards: usize,
) -> Result<BoardPlan, String> {
    if !raw.is_object() {
        return Err("规划不是对象".into());
    }
    let n = phrases.len();
    if n == 0 {
        return Err("配音没有短语时间戳".into());
    }
    let cap = MAX_BOARDS.min(if max_boards == 0 {
        MAX_BOARDS
    } else {
        max_boards
    });
    let sentence_head = |value: Option<&serde_json::Value>| {
        let number = js_number(value);
        let number = if number.is_nan() || number == 0.0 {
            0.0
        } else {
            number
        };
        let mut k = (js_round(number).max(0.0) as usize).min(n - 1);
        while k > 0 && phrases[k - 1].sentence == phrases[k].sentence {
            k -= 1;
        }
        k
    };
    let mut list: Vec<(usize, Vec<serde_json::Value>)> = get(raw, "scenes")
        .and_then(|v| v.as_array())
        .map(|scenes| {
            scenes
                .iter()
                .map(|s| {
                    let elements = get(s, "elements")
                        .and_then(|v| v.as_array())
                        .map(|e| {
                            e.iter()
                                .filter(|e| !clean(get(e, "draw")).is_empty())
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default();
                    (sentence_head(get(s, "start")), elements)
                })
                .filter(|(_, elements): &(usize, Vec<serde_json::Value>)| !elements.is_empty())
                .collect()
        })
        .unwrap_or_default();
    list.sort_by_key(|(start, _)| *start);
    let mut scenes: Vec<(usize, Vec<serde_json::Value>)> = Vec::new();
    for (start, elements) in list {
        match scenes.last_mut() {
            Some(last) if last.0 == start => last.1.extend(elements),
            _ => {
                let start = if scenes.is_empty() { 0 } else { start };
                scenes.push((start, elements));
            }
        }
    }
    while scenes.len() > cap {
        let tail = scenes.pop().unwrap();
        scenes.last_mut().unwrap().1.extend(tail.1);
    }
    let mut total = 0;
    let mut out: Vec<PlannedBoard> = Vec::new();
    for (k, (start, elements)) in scenes.iter().enumerate() {
        let end = scenes.get(k + 1).map_or(n, |next| next.0);
        let mut chosen: Vec<PlannedElement> = Vec::new();
        for e in elements {
            if chosen.len() >= max_elements_per_board || total >= MAX_ELEMENTS {
                break;
            }
            let draw = js_slice(&clean(get(e, "draw")), DRAW_MAX);
            if draw.is_empty() {
                continue;
            }
            let at = js_number(get(e, "at"));
            let at = if at.is_nan() || at == 0.0 {
                *start as f64
            } else {
                at
            };
            let at = (js_round(at).max(*start as f64) as usize).min(end - 1);
            let role = Role::parse(get(e, "role")).unwrap_or(if chosen.is_empty() {
                Role::Hero
            } else {
                Role::Support
            });
            let label = js_slice(&clean(get(e, "label")), LABEL_MAX);
            let label = if label.is_empty() {
                format!("元素 {}", total + 1)
            } else {
                label
            };
            chosen.push(PlannedElement {
                at,
                label,
                draw,
                role,
                weight: if role == Role::Hero { 1.5 } else { 1.0 },
            });
            total += 1;
        }
        chosen.sort_by_key(|e| e.at);
        // Only empty once the element budget is spent: the rest stays on the previous board.
        if !chosen.is_empty() {
            out.push(PlannedBoard {
                start: if out.is_empty() { 0 } else { *start },
                elements: chosen,
            });
        }
    }
    if out.is_empty() {
        return Err("规划里没有可用的元素".into());
    }
    let title = js_slice(&clean(get(raw, "title")), 12);
    let title = if title.is_empty() {
        out[0].elements[0].label.clone()
    } else {
        title
    };
    Ok(BoardPlan {
        title,
        notes: js_slice(&clean(get(raw, "notes")), 120),
        scenes: out,
    })
}

/// Without the model: one board per sentence (neighbours merged beyond the
/// board cap), the sentence itself as the hero element.
pub fn fallback_plan(
    max_elements_per_board: usize,
    phrases: &[Phrase],
) -> Result<BoardPlan, String> {
    let heads: Vec<usize> = (0..phrases.len())
        .filter(|&k| k == 0 || phrases[k].sentence != phrases[k - 1].sentence)
        .collect();
    let step = heads.len().div_ceil(MAX_BOARDS).max(1);
    let mut scenes = Vec::new();
    for k in (0..heads.len()).step_by(step) {
        let from = phrases[heads[k]].sentence;
        let until = heads.get(k + step).map(|&h| phrases[h].sentence);
        let text: String = phrases
            .iter()
            .filter(|p| p.sentence >= from && until.is_none_or(|u| p.sentence < u))
            .map(|p| p.text.as_str())
            .collect();
        scenes.push(serde_json::json!({
            "start": heads[k],
            "elements": [{ "at": heads[k], "label": js_slice(&text, LABEL_MAX), "draw": text, "role": "hero" }],
        }));
    }
    let all: String = phrases.iter().map(|p| p.text.as_str()).collect();
    let raw = serde_json::json!({ "title": js_slice(&all, 12), "scenes": scenes });
    sanitize_plan(&raw, max_elements_per_board, phrases, 0)
}

/// `Number.prototype.toFixed`: ties go up, decided on the exact binary value.
pub fn js_to_fixed(x: f64, digits: usize) -> String {
    let exact = format!("{:.*}", digits + 25, x.abs());
    let (int_part, frac) = exact.split_once('.').unwrap_or((&exact, ""));
    let mut kept: Vec<u8> = int_part
        .bytes()
        .chain(frac.bytes().take(digits))
        .map(|b| b - b'0')
        .collect();
    if frac.as_bytes().get(digits).is_some_and(|d| *d >= b'5') {
        let mut i = kept.len();
        loop {
            if i == 0 {
                kept.insert(0, 1);
                break;
            }
            i -= 1;
            if kept[i] == 9 {
                kept[i] = 0;
            } else {
                kept[i] += 1;
                break;
            }
        }
    }
    let digits_text: String = kept.iter().map(|d| char::from(b'0' + d)).collect();
    let split = digits_text.len() - digits;
    let mut out = String::new();
    if x.is_sign_negative() && kept.iter().any(|d| *d != 0) {
        out.push('-');
    }
    out.push_str(&digits_text[..split]);
    if digits > 0 {
        out.push('.');
        out.push_str(&digits_text[split..]);
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct BoardPrompt<'a> {
    pub board_sec: f64,
    pub max_boards: usize,
    pub cast: &'a [crate::segment::CastMember],
    pub title: &'a str,
    pub segment_title: &'a str,
    /// Tail of the previous segment and head of the next, for continuity.
    pub previous: &'a str,
    pub next: &'a str,
}

pub fn build_board_messages(
    planner_system: &str,
    max_elements_per_board: usize,
    phrases: &[Phrase],
    o: &BoardPrompt,
) -> Vec<ChatMessage> {
    let total = phrases.last().map_or(0.0, |p| p.end);
    let suggest = js_round(total / o.board_sec).max(1.0);
    let mut system = vec![
        planner_system.to_owned(),
        String::new(),
        "这是整条口播中的一段。输入是本段的短语表，每行「[编号] 开始秒数 | 短语」，空行分隔句子。短语编号就是时间锚点。".into(),
        "只输出一个 JSON 对象，不要解释：".into(),
        "{".into(),
        "  \"scenes\": [ 按时间先后排列的镜头，每块：".into(),
        "    { \"start\": 这块从哪个短语编号开始（第一块必须是 0）,".into(),
        format!("      \"elements\": [ 1~{max_elements_per_board} 个，按出现先后排列，每个："),
        format!("        {{ \"at\": 说到哪个短语时开始画（编号，须落在本块的短语范围内）, \"label\": \"不超过 {LABEL_MAX} 字的中文名\", \"draw\": \"英文画面描述，具体到物件、人物动作和短标签文字，不超过 {DRAW_MAX} 字符\", \"role\": \"hero | support | accent\" }} ] }}"),
        "  ],".into(),
        "  \"notes\": \"一句话说明怎么拆的\"".into(),
        "}".into(),
        "拆分规则：".into(),
        "1. 按意思换镜：一块讲一件事，话题转折、进入下一个要点时才换；换镜点只能落在句子开头。".into(),
        format!(
            "2. 本段约 {} 秒，大约每 {} 秒一块，建议 {} 块，最多 {} 块；本段很短时一块就够。",
            js_round(total),
            o.board_sec,
            suggest,
            o.max_boards
        ),
        "3. 元素要在讲到它的那一刻出现：at 选第一次提到这个东西的短语；同一块里 at 递增，相邻元素最好隔开一两个短语，给书写留时间。".into(),
        "4. 每块第一个元素 role=hero；关键数字、结论可以用 accent 写成 ≤6 字的大字。".into(),
        "5. 只画本段口播里说到的东西，不要发明数字和概念；上下文只用来保持画面连贯。".into(),
    ];
    if !o.cast.is_empty() {
        system.push("6. 角色设定：画到下面这些角色时，draw 里必须带上它的外观描述（英文原样），保证全片同一个人长得一样：".into());
        system.extend(
            o.cast
                .iter()
                .map(|c| format!("   - {}：{}", c.name, c.look)),
        );
    }
    // Not in the old prompt: late elements left boards no time to be looked at.
    system.push(BREATHING_RULE.into());
    system.push(String::new());
    let title = if o.title.is_empty() {
        "（未定）"
    } else {
        o.title
    };
    let segment = if o.segment_title.is_empty() {
        String::new()
    } else {
        format!(" · 本段：{}", o.segment_title)
    };
    system.push(format!("全片：{title}{segment}"));
    if !o.previous.is_empty() {
        system.push(format!("上一段结尾：{}", o.previous));
    }
    if !o.next.is_empty() {
        system.push(format!("下一段开头：{}", o.next));
    }
    system.retain(|line| !line.is_empty());
    let mut lines = Vec::new();
    for (k, p) in phrases.iter().enumerate() {
        if k > 0 && p.sentence != phrases[k - 1].sentence {
            lines.push(String::new());
        }
        lines.push(format!("[{k}] {}s | {}", js_to_fixed(p.start, 1), p.text));
    }
    vec![
        ChatMessage {
            role: ChatRole::System,
            content: system.join("\n"),
        },
        ChatMessage {
            role: ChatRole::User,
            content: lines.join("\n"),
        },
    ]
}

/// Image prompt: `{{name}}` placeholders (whitespace allowed inside) replaced
/// from `vars`, unknown ones removed, then the avoid list.
pub fn asset_prompt(template: &str, vars: &BTreeMap<String, String>, avoid: &[String]) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let placeholder = after.find("}}").and_then(|close| {
            let name = after[..close].trim_matches(|c: char| c.is_ascii_whitespace());
            (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                .then_some((name, close))
        });
        match placeholder {
            Some((name, close)) => {
                out.push_str(vars.get(name).map_or("", String::as_str));
                rest = &after[close + 2..];
            }
            None => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    if !avoid.is_empty() {
        out.push_str(&format!(" Avoid: {}.", avoid.join(", ")));
    }
    out
}
