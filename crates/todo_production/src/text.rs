//! Text helpers with the exact semantics of the rules they were ported from
//! (JavaScript `\s`, `trim`, `Number()`, `Math.round`, `\p{L}\p{N}`), so the
//! port produces the same segments, titles and prompts as before.

/// JavaScript `\s` / `String.prototype.trim` whitespace.
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

pub fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_space)
}

pub fn js_trim_start(text: &str) -> &str {
    text.trim_start_matches(is_js_space)
}

/// Letters and digits only (`/[\p{L}\p{N}]/gu`); punctuation and spaces do not count.
pub fn char_count(text: &str) -> usize {
    text.chars()
        .filter(|c| c.is_alphabetic() || c.is_numeric())
        .count()
}

/// `s.replace(/\s+/g, ' ')`.
pub fn collapse_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if is_js_space(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// Narration text: keep line breaks (TTS splits sentences on them), collapse
/// other whitespace, at most one blank line.
pub fn clean_script(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    // [^\S\n]+ → ' '
    let mut spaced = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if c != '\n' && is_js_space(c) {
            if !in_space {
                spaced.push(' ');
            }
            in_space = true;
        } else {
            spaced.push(c);
            in_space = false;
        }
    }
    // / *\n */ → '\n'
    let mut lines: Vec<&str> = spaced.split('\n').collect();
    for line in &mut lines {
        *line = line.trim_matches(' ');
    }
    let joined = lines.join("\n");
    // \n{3,} → \n\n
    let mut out = String::with_capacity(joined.len());
    let mut newlines = 0;
    for c in joined.chars() {
        if c == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push(c);
            }
        } else {
            newlines = 0;
            out.push(c);
        }
    }
    js_trim(&out).to_owned()
}

/// Model output field: braces removed, whitespace collapsed, trimmed.
pub fn clean_text(text: &str) -> String {
    let without: String = text.chars().filter(|c| *c != '{' && *c != '}').collect();
    js_trim(&collapse_spaces(&without)).to_owned()
}

/// `String(v ?? '')` for a JSON value.
pub fn js_string(value: Option<&serde_json::Value>) -> String {
    use serde_json::Value;
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => js_number_string(n),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => js_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

fn js_number_string(n: &serde_json::Number) -> String {
    match n.as_f64() {
        Some(f) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{f:.0}"),
        Some(f) => f.to_string(),
        None => n.to_string(),
    }
}

/// `Number(v)`; `None` is `undefined` (NaN), `null` is 0.
pub fn js_number(value: Option<&serde_json::Value>) -> f64 {
    use serde_json::Value;
    match value {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => {
            let s = js_trim(s);
            if s.is_empty() {
                0.0
            } else {
                s.parse::<f64>().unwrap_or(f64::NAN)
            }
        }
        Some(Value::Array(_)) => {
            let text = js_string(value);
            js_number(Some(&Value::String(text)))
        }
        Some(Value::Object(_)) => f64::NAN,
    }
}

/// `Math.round`: halves round towards +infinity.
pub fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// `s.slice(0, max)` counted in UTF-16 units (astral characters are kept whole).
pub fn js_slice(text: &str, max: usize) -> String {
    let mut units = 0;
    let mut out = String::new();
    for c in text.chars() {
        units += c.len_utf16();
        if units > max {
            break;
        }
        out.push(c);
    }
    out
}

/// Python `round(x, digits)` (half-to-even on the exact binary value).
pub fn py_round(x: f64, digits: usize) -> f64 {
    format!("{x:.digits$}").parse().unwrap_or(x)
}
