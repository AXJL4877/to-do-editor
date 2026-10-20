//! Narration timing: what is sent to TTS, and the sentence → phrase → character
//! timeline built from its word timestamps, corrected against the audio.
//! One model for every consumer (storyboard anchors, subtitles, placement).
//! Port of the editor's `tts-timeline` (sanitize / timing / retime / pause).
use crate::storyboard::Phrase;
use crate::text::py_round;
use serde::{Deserialize, Serialize};

/// Python `str.isspace`.
fn is_py_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | '\u{1c}'..='\u{1f}' | ' ' | '\u{85}' | '\u{a0}' | '\u{1680}'
        | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}

fn py_strip(text: &str) -> &str {
    text.trim_matches(is_py_space)
}

/// `[text](url)` / `![alt](url)` → text.
fn strip_links(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'scan: while i < chars.len() {
        let open = if chars[i] == '!' && chars.get(i + 1) == Some(&'[') {
            i + 1
        } else {
            i
        };
        if chars[open] == '[' {
            if let Some(close) = chars[open + 1..]
                .iter()
                .position(|c| *c == ']')
                .map(|p| open + 1 + p)
            {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = chars[close + 2..]
                        .iter()
                        .position(|c| *c == ')')
                        .map(|p| close + 2 + p)
                    {
                        out.extend(&chars[open + 1..close]);
                        i = end + 1;
                        continue 'scan;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// `re.sub(open (.+?) close, r"\1")` where `guard` characters may not touch the
/// delimiters (Python's `(?<!x)` / `(?!x)` around single-character markers).
fn strip_pairs(text: &str, delimiter: &str, guard: Option<char>) -> String {
    let chars: Vec<char> = text.chars().collect();
    let d: Vec<char> = delimiter.chars().collect();
    let at = |i: usize| i + d.len() <= chars.len() && chars[i..i + d.len()] == d[..];
    let guarded = |i: usize| match guard {
        None => true,
        Some(g) => (i == 0 || chars[i - 1] != g) && chars.get(i + d.len()) != Some(&g),
    };
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if at(i) && guarded(i) {
            // Non-greedy body of at least one character, not crossing a line.
            let mut j = i + d.len() + 1;
            let mut found = None;
            while j <= chars.len() && chars[j - 1] != '\n' {
                if at(j) && guarded(j) {
                    found = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(j) = found {
                out.extend(&chars[i + d.len()..j]);
                i = j + d.len();
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// `` `code` `` → code (body may span lines, no backticks).
fn strip_code(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            if let Some(p) = chars[i + 1..].iter().position(|c| *c == '`') {
                if p > 0 {
                    out.extend(&chars[i + 1..i + 1 + p]);
                    i += p + 2;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// `(?m)^#{1,6}\s+` removed.
fn strip_headings(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if i == 0 || chars[i - 1] == '\n' {
            let hashes = chars[i..].iter().take_while(|c| **c == '#').count();
            if (1..=6).contains(&hashes) && chars.get(i + hashes).is_some_and(|c| is_py_space(*c)) {
                let mut j = i + hashes;
                while j < chars.len() && is_py_space(chars[j]) {
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

const NOISE: &[char] = &[
    '*', '_', '`', '~', '|', '#', '<', '>', '\\', '{', '}', '＊', '＿', '｀', '～', '｜', '＃',
    '＜', '＞', '＼', '｛', '｝', '\u{200b}', '\u{200c}', '\u{200d}', '\u{feff}',
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spoken {
    pub original: String,
    /// Sent to TTS and used for every timestamp.
    pub spoken: String,
    pub changed: bool,
}

/// Remove markdown and markers TTS would read aloud (`*` as 星星).
pub fn sanitize_for_speech(text: &str) -> Spoken {
    let mut s = strip_links(text);
    s = strip_pairs(&s, "**", None);
    s = strip_pairs(&s, "__", None);
    s = strip_pairs(&s, "*", Some('*'));
    s = strip_pairs(&s, "_", Some('_'));
    s = strip_code(&s);
    s = strip_headings(&s);
    s.retain(|c| !NOISE.contains(&c));
    // [ \t　]+ → ' '
    let mut collapsed = String::with_capacity(s.len());
    let mut in_space = false;
    for c in s.chars() {
        if matches!(c, ' ' | '\t' | '\u{3000}') {
            if !in_space {
                collapsed.push(' ');
            }
            in_space = true;
        } else {
            collapsed.push(c);
            in_space = false;
        }
    }
    // \n{3,} → \n\n
    let mut out = String::with_capacity(collapsed.len());
    let mut newlines = 0;
    for c in collapsed.chars() {
        newlines = if c == '\n' { newlines + 1 } else { 0 };
        if newlines <= 2 {
            out.push(c);
        }
    }
    let original = py_strip(text).to_owned();
    let spoken = py_strip(&out).to_owned();
    Spoken {
        changed: spoken != original,
        original,
        spoken,
    }
}

/// A timed token from the TTS word stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub word: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharTime {
    pub id: usize,
    pub text: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhraseTime {
    pub id: usize,
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub char_ids: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SentenceTime {
    pub id: usize,
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub phrase_ids: Vec<usize>,
}

/// The narration timeline in seconds from the start of the audio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub text: String,
    pub duration: f64,
    pub sentences: Vec<SentenceTime>,
    pub phrases: Vec<PhraseTime>,
    pub chars: Vec<CharTime>,
}

const SENTENCE_END: &[char] = &['。', '！', '？', '!', '?', '…', '\n'];
const PHRASE_END: &[char] = &['，', '、', '；', '：', ',', ';'];
/// Not spoken: pauses and quotes (`retime.PUNCT`).
const PUNCT: &[char] = &[
    '，', '。', '、', '；', '：', '！', '？', ',', '.', ';', ':', '!', '?', '…', '—', '"', '\'',
    '“', '”', '‘', '’', '（', '）', '(', ')', '《', '》', '【', '】', ' ', '\n',
];

fn is_punct(text: &str) -> bool {
    let mut chars = text.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if PUNCT.contains(&c))
}

fn ends_with(text: &str, marks: &[char]) -> bool {
    text.chars().last().is_some_and(|c| marks.contains(&c))
}

/// Per-sentence word groups from the raw TTS `sentence` objects.
fn raw_groups(sentences_raw: &[serde_json::Value]) -> Vec<Vec<Word>> {
    sentences_raw
        .iter()
        .filter_map(|sentence| {
            let words: Vec<Word> = sentence
                .get("words")?
                .as_array()?
                .iter()
                .filter_map(|w| {
                    let text = w.get("word").and_then(|v| v.as_str()).unwrap_or_default();
                    let time = |key| {
                        w.get(key)
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.0)
                    };
                    (!text.is_empty()).then(|| Word {
                        word: text.into(),
                        start: time("startTime"),
                        end: time("endTime"),
                    })
                })
                .collect();
            (!words.is_empty()).then_some(words)
        })
        .collect()
}

/// Sentences → phrases (split after `，、；：`) → characters (multi-character
/// tokens share their time evenly). Raw sentence groups are preferred; plain
/// word lists are grouped at sentence-ending punctuation.
/// The narration timeline of one speech result: provider timestamps laid
/// onto the text that was sent, then re-timed and snapped to the audio.
/// Shared by the narration recipe and the `speech` service of workers.
pub fn speech_timeline(
    spoken: &str,
    sentences_raw: &[serde_json::Value],
    words: &[Word],
    pcm: Pcm,
) -> Timeline {
    let mut timeline = build_timeline(spoken, sentences_raw, words);
    timeline.retime(pcm);
    let _ = timeline.snap_sentence_edges(pcm);
    timeline
}

pub fn build_timeline(text: &str, sentences_raw: &[serde_json::Value], words: &[Word]) -> Timeline {
    let mut groups = raw_groups(sentences_raw);
    let from_raw = !groups.is_empty();
    if groups.is_empty() {
        let mut current = Vec::new();
        for word in words.iter().filter(|w| !w.word.is_empty()) {
            current.push(word.clone());
            if ends_with(&word.word, SENTENCE_END) {
                groups.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            groups.push(current);
        }
    }
    let mut timeline = Timeline {
        text: text.into(),
        duration: 0.0,
        sentences: Vec::new(),
        phrases: Vec::new(),
        chars: Vec::new(),
    };
    for (si, group) in groups.iter().enumerate() {
        let mut buffers: Vec<Vec<&Word>> = Vec::new();
        let mut buffer = Vec::new();
        for token in group {
            buffer.push(token);
            if ends_with(&token.word, PHRASE_END) || ends_with(&token.word, SENTENCE_END) {
                buffers.push(std::mem::take(&mut buffer));
            }
        }
        if !buffer.is_empty() {
            buffers.push(buffer);
        }
        let mut phrase_ids = Vec::new();
        for tokens in buffers {
            let mut char_ids = Vec::new();
            for token in &tokens {
                let units: Vec<char> = token.word.chars().collect();
                let span = (token.end - token.start).max(0.0);
                let step = if span > 0.0 {
                    span / units.len() as f64
                } else {
                    0.0
                };
                for (i, c) in units.iter().enumerate() {
                    let (a, b) = if units.len() == 1 {
                        (token.start, token.end)
                    } else {
                        let a = token.start + step * i as f64;
                        let b = if i == units.len() - 1 {
                            token.end
                        } else {
                            token.start + step * (i + 1) as f64
                        };
                        (a, b)
                    };
                    let id = timeline.chars.len();
                    timeline.chars.push(CharTime {
                        id,
                        text: c.to_string(),
                        start: py_round(a, 4),
                        end: py_round(b, 4),
                    });
                    char_ids.push(id);
                }
            }
            let id = timeline.phrases.len();
            timeline.phrases.push(PhraseTime {
                id,
                text: tokens.iter().map(|t| t.word.as_str()).collect(),
                start: py_round(tokens[0].start, 4),
                end: py_round(tokens[tokens.len() - 1].end, 4),
                char_ids,
            });
            phrase_ids.push(id);
        }
        let joined: String = group.iter().map(|t| t.word.as_str()).collect();
        let text = if joined.is_empty() && from_raw {
            sentences_raw
                .get(si)
                .and_then(|s| s.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or_default()
                .to_owned()
        } else {
            joined
        };
        timeline.sentences.push(SentenceTime {
            id: si,
            text,
            start: py_round(group[0].start, 4),
            end: py_round(group[group.len() - 1].end, 4),
            phrase_ids,
        });
    }
    let duration = timeline
        .chars
        .last()
        .map(|c| c.end)
        .or_else(|| timeline.phrases.last().map(|p| p.end))
        .or_else(|| timeline.sentences.last().map(|s| s.end))
        .unwrap_or(0.0);
    timeline.duration = py_round(duration, 4);
    timeline
}

/// 16-bit mono PCM.
#[derive(Debug, Clone, Copy)]
pub struct Pcm<'a> {
    pub samples: &'a [i16],
    pub rate: u32,
}

impl Pcm<'_> {
    pub fn duration(&self) -> f64 {
        if self.rate == 0 {
            0.0
        } else {
            self.samples.len() as f64 / f64::from(self.rate)
        }
    }
}

/// Parse a PCM16 mono WAV (the format our TTS adapter writes).
pub fn read_wav(bytes: &[u8]) -> Option<(Vec<i16>, u32)> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut position = 12;
    let mut rate = None;
    let mut format_ok = false;
    while position + 8 <= bytes.len() {
        let id = &bytes[position..position + 4];
        let size = u32::from_le_bytes(bytes[position + 4..position + 8].try_into().ok()?) as usize;
        let body = &bytes[position + 8..(position + 8 + size).min(bytes.len())];
        if id == b"fmt " && body.len() >= 16 {
            let channels = u16::from_le_bytes([body[2], body[3]]);
            let bits = u16::from_le_bytes([body[14], body[15]]);
            rate = Some(u32::from_le_bytes(body[4..8].try_into().ok()?));
            format_ok = channels == 1 && bits == 16;
        } else if id == b"data" {
            if !format_ok {
                return None;
            }
            let samples = body
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes(*b))
                .collect();
            return Some((samples, rate?));
        }
        position += 8 + size + (size & 1);
    }
    None
}

const SILENCE_DBFS: f64 = -40.0;

/// (total duration, end of the last voiced 20 ms frame).
fn speech_span(pcm: Pcm) -> (f64, f64) {
    let duration = pcm.duration();
    let hop = ((f64::from(pcm.rate) * 0.02) as usize).max(1);
    let samples = pcm.samples;
    if samples.is_empty() {
        return (duration, duration);
    }
    let floor = 32768.0 * 10f64.powf(SILENCE_DBFS / 20.0);
    let mut i = (samples.len() - 1) / hop * hop;
    loop {
        let segment = &samples[i..(i + hop).min(samples.len())];
        let energy: f64 = segment.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
        if (energy / segment.len() as f64).sqrt() >= floor {
            return (
                duration,
                duration.min((i + segment.len()) as f64 / f64::from(pcm.rate)),
            );
        }
        if i < hop {
            return (duration, duration);
        }
        i -= hop;
    }
}

fn crushed(chars: &[&CharTime]) -> bool {
    let spoken: Vec<&&CharTime> = chars.iter().filter(|c| !is_punct(&c.text)).collect();
    match (spoken.first(), spoken.last()) {
        // Normal speech is 0.15–0.25 s per character.
        (Some(first), Some(last)) => (last.end - first.start) / (spoken.len() as f64) < 0.07,
        _ => false,
    }
}

impl Timeline {
    /// Phrase and sentence bounds follow their characters.
    fn sync(&mut self) {
        for phrase in &mut self.phrases {
            if let (Some(first), Some(last)) = (phrase.char_ids.first(), phrase.char_ids.last()) {
                phrase.start = self.chars[*first].start;
                phrase.end = self.chars[*last].end;
            }
        }
        for sentence in &mut self.sentences {
            if let (Some(first), Some(last)) =
                (sentence.phrase_ids.first(), sentence.phrase_ids.last())
            {
                sentence.start = self.phrases[*first].start;
                sentence.end = self.phrases[*last].end;
            }
        }
    }

    /// Take the real audio length, and re-spread trailing phrases the API
    /// squeezed into the final instant over the actual speech.
    pub fn retime(&mut self, pcm: Pcm) {
        let (duration, speech_end) = speech_span(pcm);
        if duration > 0.0 {
            self.duration = py_round(duration, 4);
        }
        if self.chars.is_empty() || self.phrases.is_empty() {
            return;
        }
        let mut k = self.phrases.len();
        while k > 0 {
            let chars: Vec<&CharTime> = self.phrases[k - 1]
                .char_ids
                .iter()
                .map(|id| &self.chars[*id])
                .collect();
            if !crushed(&chars) {
                break;
            }
            k -= 1;
        }
        let tail: Vec<usize> = self.phrases[k..]
            .iter()
            .flat_map(|p| p.char_ids.iter().copied())
            .collect();
        if let (Some(&first), Some(&last)) = (tail.first(), tail.last()) {
            let t0 = self.chars[first].start;
            let t1 = speech_end.max(self.chars[last].end);
            if t1 - t0 > 0.05 {
                // A character is one share, an inner pause mark 0.6; a final mark hangs after the voice.
                let last_punct = is_punct(&self.chars[last].text);
                let body = if last_punct {
                    &tail[..tail.len() - 1]
                } else {
                    &tail[..]
                };
                let weights: Vec<f64> = body
                    .iter()
                    .map(|id| {
                        if is_punct(&self.chars[*id].text) {
                            0.6
                        } else {
                            1.0
                        }
                    })
                    .collect();
                let total: f64 = weights.iter().sum();
                let total = if total == 0.0 { 1.0 } else { total };
                let mut t = t0;
                for (id, w) in body.iter().zip(&weights) {
                    self.chars[*id].start = py_round(t, 4);
                    t += (t1 - t0) * w / total;
                    self.chars[*id].end = py_round(t, 4);
                }
                if last_punct {
                    self.chars[last].start = py_round(t1, 4);
                    self.chars[last].end = py_round(t1.max(duration.min(t1 + 0.15)), 4);
                }
            }
        }
        self.sync();
    }

    /// Sentence starts move back to the real voice onset and ends forward to
    /// its release (the API is often 0.05–0.8 s off). Returns edits made.
    pub fn snap_sentence_edges(&mut self, pcm: Pcm) -> usize {
        const FRAME: f64 = 0.01;
        const GAP_FRAMES: usize = 3;
        let hop = ((f64::from(pcm.rate) * FRAME) as usize).max(1);
        let frames = pcm.samples.len() / hop;
        let quiet: Vec<bool> = (0..frames)
            .map(|f| {
                let window = &pcm.samples[f * hop..(f + 1) * hop];
                let rms = (window
                    .iter()
                    .map(|s| f64::from(*s) * f64::from(*s))
                    .sum::<f64>()
                    / hop as f64)
                    .sqrt();
                20.0 * (rms / 32768.0 + 1e-9).log10() < SILENCE_DBFS
            })
            .collect();
        // A pause is at least GAP_FRAMES quiet frames; single dips between characters are voice.
        let loud: Vec<bool> = (0..frames)
            .map(|i| {
                let run = (i.saturating_sub(1)..=(i + 1).min(frames.saturating_sub(1)))
                    .filter(|&j| quiet[j])
                    .count();
                run < GAP_FRAMES
            })
            .collect();
        let reach = (0.8 / FRAME) as usize;
        let bounds: Vec<(f64, f64)> = self.sentences.iter().map(|s| (s.start, s.end)).collect();
        let mut moved = 0;
        for k in 0..self.sentences.len() {
            let ids: Vec<usize> = self.sentences[k]
                .phrase_ids
                .iter()
                .flat_map(|p| self.phrases[*p].char_ids.iter().copied())
                .collect();
            let spoken: Vec<usize> = ids
                .iter()
                .copied()
                .filter(|id| !is_punct(&self.chars[*id].text))
                .collect();
            let (Some(&first), Some(&last)) = (spoken.first(), spoken.last()) else {
                continue;
            };
            let lo = if k > 0 { bounds[k - 1].1 } else { 0.0 };
            let hi = bounds.get(k + 1).map_or(frames as f64 * FRAME, |b| b.0);
            let f = (self.chars[first].start / FRAME) as usize;
            let mut g = f;
            while g > 0
                && f - g < reach
                && g as f64 * FRAME > lo
                && loud.get(g - 1).copied().unwrap_or(false)
            {
                g -= 1;
            }
            if g < f {
                self.chars[first].start = py_round(g as f64 * FRAME, 4);
                moved += 1;
            }
            let e = (self.chars[last].end / FRAME) as usize;
            let mut h = e;
            while h < loud.len() && h - e < reach && (h + 1) as f64 * FRAME < hi && loud[h] {
                h += 1;
            }
            if h > e {
                let end = py_round(h as f64 * FRAME, 4);
                self.chars[last].end = end;
                let after = ids
                    .iter()
                    .position(|id| *id == last)
                    .map_or(ids.len(), |p| p + 1);
                for id in &ids[after..] {
                    let c = &mut self.chars[*id];
                    c.start = c.start.max(end);
                    c.end = c.end.max(c.start);
                }
                moved += 1;
            }
        }
        self.sync();
        moved
    }

    /// Flat phrase list with sentence numbers: the anchors boards refer to.
    pub fn phrases(&self) -> Vec<Phrase> {
        let mut out = Vec::new();
        for (si, sentence) in self.sentences.iter().enumerate() {
            for id in &sentence.phrase_ids {
                let p = &self.phrases[*id];
                let text = crate::text::js_trim(&p.text);
                if text.is_empty() || !p.start.is_finite() || !p.end.is_finite() {
                    continue;
                }
                out.push(Phrase {
                    text: text.into(),
                    start: p.start,
                    end: p.start.max(p.end),
                    sentence: si,
                });
            }
        }
        out
    }
}
