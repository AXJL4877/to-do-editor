//! Long narration → spoken segments (one voice track and one group of boards
//! each). Units are sentences (clauses for very long ones); a blank line is a
//! soft boundary, a line of `---` a hard one. The model only chooses cut points,
//! it never rewrites the text: joining the segments gives back the script.
//! Rules are a faithful port of the editor's `segment.mjs`.
use crate::tasks::{ChatMessage, ChatRole};
use crate::text::{
    char_count, collapse_spaces, is_js_space, js_number, js_round, js_slice, js_string, js_trim,
    js_trim_start,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unit {
    /// Original text including punctuation and the following line breaks.
    pub text: String,
    pub chars: usize,
    /// A clause cut out of an over-long sentence.
    pub clause: bool,
    /// A paragraph boundary follows.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub para: bool,
    /// A `---` hard boundary follows.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hard: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// Preferred characters per segment.
    pub target: usize,
    /// Never more than this.
    pub max: usize,
}

impl Limits {
    /// Sentences longer than this are split into clauses.
    pub fn clause_over(self) -> usize {
        let over = js_round(self.target as f64 * 0.9).max(60.0) as usize;
        over.min(self.max)
    }
}

const SENTENCE_END: &[char] = &['。', '！', '？', '!', '?', '…', '；', ';'];
const CLAUSE_END: &[char] = &['，', ',', '、', '：', ':'];
const CLOSERS: &[char] = &['”', '’', '」', '』', '）', ')', ']'];
const GLUE_OPENERS: &[char] = &['“', '"', '「', '『', '（', '('];
/// A sentence starting with one of these continues the previous one.
const GLUE_WORDS: &[&str] = &[
    "于是",
    "所以",
    "因为",
    "而是",
    "但是",
    "可是",
    "不过",
    "然而",
    "因此",
    "这叫",
    "这就是",
    "这便是",
    "说白了",
    "也就是说",
    "换句话说",
    "换言之",
    "比如",
    "例如",
    "结果",
    "接着",
    "然后",
    "同时",
    "并且",
    "而且",
    "甚至",
    "否则",
    "那么",
];

pub fn is_glued(text: &str) -> bool {
    let rest = js_trim_start(text).trim_start_matches(GLUE_OPENERS);
    GLUE_WORDS.iter().any(|word| rest.starts_with(word))
}

fn can_cut_before(unit: &Unit) -> bool {
    !is_glued(&unit.text)
}

/// `split(/(DELIM+CLOSERS*)/)` re-joined as text+delimiter pieces, blanks dropped.
fn split_keep(text: &str, delimiters: &[char]) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut piece = String::new();
    let mut i = 0;
    while i < chars.len() {
        if delimiters.contains(&chars[i]) {
            while i < chars.len() && delimiters.contains(&chars[i]) {
                piece.push(chars[i]);
                i += 1;
            }
            while i < chars.len() && CLOSERS.contains(&chars[i]) {
                piece.push(chars[i]);
                i += 1;
            }
            if !js_trim(&piece).is_empty() {
                out.push(std::mem::take(&mut piece));
            } else {
                piece.clear();
            }
        } else {
            piece.push(chars[i]);
            i += 1;
        }
    }
    if !js_trim(&piece).is_empty() {
        out.push(piece);
    }
    out
}

/// `split(/\n\s*\n/)`: a whitespace run containing two or more line breaks.
fn split_paragraphs(text: &str) -> Vec<&str> {
    let bytes: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut from = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].1 == '\n' {
            let mut j = i;
            let mut last_newline = i;
            while j < bytes.len() && is_js_space(bytes[j].1) {
                if bytes[j].1 == '\n' {
                    last_newline = j;
                }
                j += 1;
            }
            if last_newline > i {
                out.push(&text[from..bytes[i].0]);
                from = bytes[last_newline].0 + 1;
                i = last_newline + 1;
                continue;
            }
        }
        i += 1;
    }
    out.push(&text[from..]);
    out
}

fn is_rule_line(line: &str) -> bool {
    let core = line.trim_matches(|c| c == ' ' || c == '\t');
    core.len() >= 3 && core.chars().all(|c| c == '-')
}

/// Script → sentence units. Over-long sentences (`> clause_over` characters)
/// become clauses.
pub fn split_units(script: &str, clause_over: usize) -> Vec<Unit> {
    let script = script.replace("\r\n", "\n").replace('\r', "\n");
    let mut blocks = vec![String::new()];
    for line in script.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        if is_rule_line(content) {
            // The rule line itself is removed; its newline starts the next block.
            blocks.push(if line.ends_with('\n') {
                "\n".into()
            } else {
                String::new()
            });
        } else {
            blocks.last_mut().unwrap().push_str(line);
        }
    }
    let mut units: Vec<Unit> = Vec::new();
    for block in blocks {
        let before = units.len();
        units.extend(block_units(&block, clause_over));
        if units.len() > before && before > 0 {
            units[before - 1].hard = true;
        }
    }
    units
}

fn trim_trailing_newlines(text: &str) -> &str {
    text.trim_end_matches('\n')
}

fn block_units(block: &str, clause_over: usize) -> Vec<Unit> {
    let mut units: Vec<Unit> = Vec::new();
    for para in split_paragraphs(js_trim(block)) {
        let lines: Vec<&str> = para.split('\n').collect();
        let start = units.len();
        for line in &lines {
            for sentence in split_keep(line, SENTENCE_END) {
                if char_count(&sentence) > clause_over {
                    units.extend(
                        split_keep(&sentence, CLAUSE_END)
                            .into_iter()
                            .map(|text| Unit {
                                text,
                                chars: 0,
                                clause: true,
                                para: false,
                                hard: false,
                            }),
                    );
                } else {
                    units.push(Unit {
                        text: sentence,
                        chars: 0,
                        clause: false,
                        para: false,
                        hard: false,
                    });
                }
            }
            // The line break belongs to the line's last sentence.
            if units.len() > start && lines.len() > 1 {
                units.last_mut().unwrap().text.push('\n');
            }
        }
        if units.len() > start {
            let last = units.last_mut().unwrap();
            last.para = true;
            last.text = format!("{}\n\n", trim_trailing_newlines(&last.text));
        }
    }
    let mut out: Vec<Unit> = Vec::new();
    for mut unit in units {
        if !unit.para {
            let trimmed = trim_trailing_newlines(&unit.text).len();
            if trimmed < unit.text.len() {
                unit.text.truncate(trimmed);
                unit.text.push('\n');
            }
        }
        unit.chars = char_count(&unit.text);
        if unit.chars == 0 && js_trim(&unit.text).is_empty() {
            continue;
        }
        // Bare punctuation joins the previous unit.
        match out.last_mut() {
            Some(last) if unit.chars == 0 => {
                last.text.push_str(&unit.text);
                last.para = last.para || unit.para;
            }
            _ => out.push(unit),
        }
    }
    out
}

fn span(units: &[Unit], a: usize, b: usize) -> usize {
    units[a..b].iter().map(|u| u.chars).sum()
}

/// Proposed cut points (first unit of each segment) → cut points within the
/// limits: never before a continuing sentence, hard boundaries always, long
/// segments split near the target, tiny ones merged into the previous one.
pub fn enforce_limits(units: &[Unit], starts: &[i64], limits: Limits) -> Vec<usize> {
    let n = units.len();
    let mut cuts: BTreeSet<usize> = std::iter::once(0)
        .chain(
            starts
                .iter()
                .filter(|s| **s > 0 && (**s as usize) < n)
                .map(|s| *s as usize),
        )
        .collect();
    cuts = cuts
        .into_iter()
        .map(|cut| {
            let mut k = cut;
            while k > 0 && !can_cut_before(&units[k]) {
                k -= 1;
            }
            k
        })
        .collect();
    let hard: BTreeSet<usize> = units
        .iter()
        .enumerate()
        .filter(|(i, u)| u.hard && i + 1 < n)
        .map(|(i, _)| i + 1)
        .collect();
    cuts.extend(hard.iter().copied());
    let cuts: Vec<usize> = cuts.into_iter().collect();

    let mut out = Vec::new();
    for (i, &a) in cuts.iter().enumerate() {
        let b = cuts.get(i + 1).copied().unwrap_or(n);
        out.extend(split_long(units, a, b, limits));
    }
    let mut merged: Vec<usize> = Vec::new();
    for (i, &a) in out.iter().enumerate() {
        let b = out.get(i + 1).copied().unwrap_or(n);
        if let Some(&previous) = merged.last() {
            if !hard.contains(&a)
                && (span(units, a, b) as f64) < limits.target as f64 * 0.3
                && span(units, previous, b) <= limits.max
            {
                continue;
            }
        }
        merged.push(a);
    }
    merged
}

/// `[a, b)` over `max` is split recursively near the target; cuts prefer
/// paragraph ends and never fall before a continuing sentence when avoidable.
fn split_long(units: &[Unit], a: usize, b: usize, limits: Limits) -> Vec<usize> {
    let total = span(units, a, b);
    if total <= limits.max || b - a <= 1 {
        return vec![a];
    }
    let target = limits.target as f64;
    let max = limits.max;
    let mut best: Option<usize> = None;
    let mut best_score = f64::INFINITY;
    let mut acc = 0usize;
    for i in a + 1..b {
        acc += units[i - 1].chars;
        if !can_cut_before(&units[i]) {
            continue;
        }
        let rest = total - acc;
        if acc > max && best.is_some() {
            break;
        }
        let mut score = (acc as f64 - target).abs();
        if units[i - 1].para {
            score -= target * 0.25;
        }
        if units[i - 1].clause {
            score += target * 0.15;
        }
        if (rest as f64) < target * 0.35 {
            score += target;
        }
        if acc > max {
            score += 10.0 * (acc - max) as f64;
        }
        if score < best_score {
            best_score = score;
            best = Some(i);
        }
    }
    let best = match best {
        Some(best) => best,
        None => {
            // Only continuing sentences: cut nearest the target rather than exceed the maximum.
            let mut acc = 0usize;
            let mut best = a + 1;
            let mut best_score = f64::INFINITY;
            for i in a + 1..b {
                acc += units[i - 1].chars;
                let score = (acc as f64 - target).abs();
                if score < best_score {
                    best_score = score;
                    best = i;
                }
            }
            best
        }
    };
    let mut out = split_long(units, a, best, limits);
    out.extend(split_long(units, best, b, limits));
    out
}

/// Cut points by length; with `paragraph`, paragraph ends are preferred cuts.
pub fn heuristic_starts(units: &[Unit], limits: Limits, paragraph: bool) -> Vec<usize> {
    let target = limits.target as f64;
    let mut starts = vec![0i64];
    let mut len = 0usize;
    for (i, unit) in units.iter().enumerate() {
        if i > 0 && can_cut_before(unit) {
            let previous = &units[i - 1];
            let at_para = previous.hard
                || (previous.para
                    && (len as f64)
                        >= if paragraph {
                            target * 0.5
                        } else {
                            target * 0.75
                        });
            let full = (len as f64) >= target * 0.6 && (len + unit.chars) as f64 > target;
            if at_para || full || len + unit.chars > limits.max {
                starts.push(i as i64);
                len = 0;
            }
        }
        len += unit.chars;
    }
    enforce_limits(units, &starts, limits)
}

/// Segment texts in order (line breaks kept, ends trimmed).
pub fn segment_texts(units: &[Unit], starts: &[usize]) -> Vec<String> {
    starts
        .iter()
        .enumerate()
        .map(|(i, &a)| {
            let b = starts.get(i + 1).copied().unwrap_or(units.len());
            let text: String = units[a..b].iter().map(|u| u.text.as_str()).collect();
            js_trim(&text).to_owned()
        })
        .collect()
}

pub fn build_segment_messages(units: &[Unit], limits: Limits, title: &str) -> Vec<ChatMessage> {
    let mut lines = Vec::new();
    for (i, unit) in units.iter().enumerate() {
        lines.push(format!(
            "[{i}] ({}字) {}",
            unit.chars,
            js_trim(&unit.text).replace('\n', " ")
        ));
        if unit.hard {
            lines.push("---".into());
        } else if unit.para {
            lines.push(String::new());
        }
    }
    let (target, max) = (limits.target, limits.max);
    let mut system = vec![
        "你是口播短视频的剪辑编辑。一条长口播会被切成若干「口播段」，每段单独配音，并配一组边讲边画的画面。".to_owned(),
        "输入是编号后的句子表，每行「[编号] (字数) 句子」，空行是原文的段落边界。".into(),
        "只输出一个 JSON 对象，不要解释：".into(),
        "{".into(),
        "  \"title\": \"不超过 12 字的全片标题\",".into(),
        "  \"cast\": [ 反复出现的人物 / 角色（0~6 个），每个 { \"name\": \"口播里的称呼\", \"look\": \"英文外观描述，≤120 字符，画成极简粉笔小人时的固定特征：身形、帽子、衣着、道具\" } ],".into(),
        "  \"segments\": [ 按顺序，每段 { \"start\": 这段从哪个句子编号开始（第一段必须是 0）, \"title\": \"不超过 10 字的小标题\" } ]".into(),
        "}".into(),
        "切分规则：".into(),
        format!("1. 语意为主：一段讲完一个完整意群（一个场景 / 一个论点 / 一个概念的定义和解释）。尽量每段约 {target} 字（按句子字数累加），完整意群可以略长，但硬上限 {max} 字。不要为了凑字数切碎。"),
        "2. 空行是强边界，优先在段落边界切；「---」行是作者指定的换段点，那里必须切开。".into(),
        "3. 不要在以「于是 / 所以 / 因为 / 而是 / 但是 / 这叫 / 这就是 / 说白了 / 也就是说」等承接词开头的句子前切开；「这叫××」和紧跟的人话定义必须在同一段。".into(),
        "4. 开场白（自我介绍、抛问题）可以单独成短段；结尾的总结、关注引导也可以单独成段。".into(),
        "5. 只决定在哪切，不改写、不增删原文。".into(),
    ];
    if !title.is_empty() {
        system.push(format!("全片暂定标题：{title}"));
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CastMember {
    pub name: String,
    /// English look, quoted verbatim whenever the character is drawn.
    pub look: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outline {
    pub title: String,
    pub cast: Vec<CastMember>,
    pub starts: Vec<usize>,
    pub titles: Vec<String>,
}

/// `String(v ?? '')` with braces and line breaks removed, collapsed, trimmed, sliced.
fn clean_field(value: Option<&serde_json::Value>, max: usize) -> String {
    let text: String = js_string(value)
        .chars()
        .map(|c| {
            if matches!(c, '{' | '}' | '\r' | '\n') {
                ' '
            } else {
                c
            }
        })
        .collect();
    js_slice(js_trim(&collapse_spaces(&text)), max)
}

/// Model output → outline within the limits. The model's cut points pass
/// through the same rules as every other mode.
pub fn sanitize_outline(
    raw: &serde_json::Value,
    units: &[Unit],
    limits: Limits,
) -> Result<Outline, String> {
    let object = raw.as_object().ok_or("分段结果不是对象")?;
    let field =
        |value: &serde_json::Value, key: &str| value.as_object().and_then(|o| o.get(key)).cloned();
    let list: Vec<(i64, String)> = object
        .get("segments")
        .and_then(|v| v.as_array())
        .map(|segments| {
            segments
                .iter()
                .map(|s| {
                    let start = js_round(js_number(field(s, "start").as_ref()));
                    (start, clean_field(field(s, "title").as_ref(), 10))
                })
                .filter(|(start, _)| {
                    start.is_finite() && *start >= 0.0 && (*start as usize) < units.len()
                })
                .map(|(start, title)| (start as i64, title))
                .collect()
        })
        .unwrap_or_default();
    if list.is_empty() {
        return Err("分段结果里没有段".into());
    }
    let title_at: HashMap<i64, String> = list.iter().cloned().collect();
    let starts = enforce_limits(
        units,
        &list.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
        limits,
    );
    let cast = object
        .get("cast")
        .and_then(|v| v.as_array())
        .map(|members| {
            members
                .iter()
                .map(|c| CastMember {
                    name: clean_field(field(c, "name").as_ref(), 12),
                    look: clean_field(field(c, "look").as_ref(), 160),
                })
                .filter(|c| !c.name.is_empty() && !c.look.is_empty())
                .take(6)
                .collect()
        })
        .unwrap_or_default();
    let titles = starts
        .iter()
        .map(|s| title_at.get(&(*s as i64)).cloned().unwrap_or_default())
        .collect();
    Ok(Outline {
        title: clean_field(object.get("title"), 12),
        cast,
        starts,
        titles,
    })
}
