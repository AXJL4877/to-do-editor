//! Subtitle cues from the script and the narration's character times: cut at
//! punctuation, soft-split long runs without breaking words, time each cue by
//! aligning its characters, and hold or release at the voice's pauses.
//! Port of the editor's `burn_subtitles` cue rules (TTS path).
mod keywords;

pub use keywords::KEYWORDS;
use serde::{Deserialize, Serialize};

pub const MIN_CHARS: usize = 4;
/// Silences shorter than this are not pauses.
pub const PAUSE_GAP: f64 = 0.25;
/// Only a pause this long keeps the screen empty between cues.
const HOLD_SILENCE: f64 = 0.35;
/// Gaps shorter than this are bridged by extending the previous cue.
const FILL_GAP: f64 = 0.28;
/// Extra time after a cue's last character, covering the tail of the voice.
const TAIL_PAD: f64 = 0.12;
const MIN_CHAR_SEC: f64 = 0.15;
const MIN_CUE_SEC: f64 = 0.75;
const SOFT_AFTER: &[char] = &['的', '了', '着', '过', '呢', '吧', '啊', '呀', '嘛', '么'];
const SOFT_BEFORE_WORDS: &[char] = &['是', '在', '和', '与', '而', '但'];
const NUDGE_EXTRA: &[&str] = &[
    "第一", "第二", "第三", "第四", "第五", "核心", "并表", "时点",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cue {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// Han characters, ASCII letters and digits only (what is shown on screen).
pub fn content_chars(text: &str) -> String {
    text.chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c) || c.is_ascii_alphanumeric())
        .collect()
}

fn find(haystack: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&j| haystack[j..j + needle.len()] == *needle)
}

/// A cut inside a keyword moves before it, or after it when that would leave
/// too short a piece.
fn nudge_off_keywords(span: &[char], cut: usize) -> usize {
    if cut == 0 || cut >= span.len() {
        return cut;
    }
    for keyword in KEYWORDS.iter().chain(NUDGE_EXTRA) {
        let keyword: Vec<char> = keyword.chars().collect();
        if keyword.len() < 2 {
            continue;
        }
        let mut from = 0;
        while let Some(j) = find(span, &keyword, from) {
            if j < cut && cut < j + keyword.len() {
                let (before, after) = (j, j + keyword.len());
                if before >= MIN_CHARS {
                    return before;
                }
                if after <= span.len() {
                    return after;
                }
                return if before > 0 { before } else { after };
            }
            from = j + 1;
        }
    }
    cut
}

fn soft_split(text: &[char], max_chars: usize) -> Vec<Vec<char>> {
    if text.len() <= max_chars {
        return if text.is_empty() {
            vec![]
        } else {
            vec![text.to_vec()]
        };
    }
    let mut out = Vec::new();
    let mut i = 0;
    let n = text.len();
    while i < n {
        let remain = n - i;
        if remain <= max_chars {
            out.push(text[i..].to_vec());
            break;
        }
        let window = &text[i..i + max_chars];
        // After a particle first, then after 是/在/和/与/而/但 in the back half.
        let mut cut = (MIN_CHARS..window.len())
            .rev()
            .find(|&j| SOFT_AFTER.contains(&window[j]))
            .map(|j| j + 1);
        if cut.is_none() {
            let lower = MIN_CHARS.max(max_chars / 2);
            cut = (lower..window.len())
                .rev()
                .find(|&j| SOFT_BEFORE_WORDS.contains(&window[j]))
                .map(|j| j + 1);
        }
        let mut cut = cut.unwrap_or(max_chars);
        cut = nudge_off_keywords(&text[i..(i + max_chars + 12).min(n)], cut);
        if cut < MIN_CHARS {
            cut = max_chars;
        }
        // Never leave a tiny tail.
        if remain - cut.min(remain) < MIN_CHARS && remain <= max_chars + MIN_CHARS {
            out.push(text[i..].to_vec());
            break;
        }
        out.push(text[i..i + cut].to_vec());
        i += cut;
    }
    out.into_iter().filter(|c| !c.is_empty()).collect()
}

fn merge_short(chunks: Vec<Vec<char>>, max_chars: usize) -> Vec<Vec<char>> {
    let mut merged: Vec<Vec<char>> = Vec::new();
    for chunk in chunks {
        let fits = |last: &Vec<char>| last.len() + chunk.len() <= max_chars + 2;
        match merged.last_mut() {
            Some(last) if (chunk.len() < MIN_CHARS || last.len() < MIN_CHARS) && fits(last) => {
                last.extend(chunk)
            }
            _ => merged.push(chunk),
        }
    }
    merged
}

/// Merge neighbours when a cut falls inside a keyword (slightly over the limit is fine).
fn repair_keyword_splits(chunks: Vec<Vec<char>>, max_chars: usize) -> Vec<Vec<char>> {
    if chunks.len() < 2 {
        return chunks;
    }
    let keywords: Vec<Vec<char>> = KEYWORDS
        .iter()
        .map(|k| k.chars().collect::<Vec<_>>())
        .filter(|k| k.len() >= 2)
        .collect();
    let mut iter = chunks.into_iter();
    let mut out = vec![iter.next().unwrap()];
    for chunk in iter {
        let left = out.last().unwrap();
        let boundary = left.len();
        let merged: Vec<char> = left.iter().chain(chunk.iter()).copied().collect();
        let straddles = keywords.iter().any(|k| {
            let mut from = 0;
            while let Some(j) = find(&merged, k, from) {
                if j < boundary && boundary < j + k.len() {
                    return true;
                }
                from = j + 1;
            }
            false
        });
        if straddles && merged.len() <= max_chars + 4usize.max(8usize.min(chunk.len())) {
            *out.last_mut().unwrap() = merged;
        } else {
            out.push(chunk);
        }
    }
    out
}

/// Script → on-screen subtitle lines without punctuation.
pub fn split_subtitles(text: &str, max_chars: usize) -> Vec<String> {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let mut units: Vec<Vec<char>> = Vec::new();
    for sentence in split_after(&t, &['。', '！', '？', '；', '!', '?', ';']) {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }
        for bit in split_after(sentence, &['，', '、', ',', ':']) {
            let body: Vec<char> = content_chars(bit).chars().collect();
            if !body.is_empty() {
                units.push(body);
            }
        }
    }
    let chunks: Vec<Vec<char>> = units
        .iter()
        .flat_map(|u| soft_split(u, max_chars))
        .collect();
    let chunks = merge_short(chunks, max_chars);
    repair_keyword_splits(chunks, max_chars)
        .into_iter()
        .map(|c| c.into_iter().collect())
        .collect()
}

/// Split after every occurrence of any mark (zero-width `(?<=[...])` split).
fn split_after<'a>(text: &'a str, marks: &[char]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut from = 0;
    for (i, c) in text.char_indices() {
        if marks.contains(&c) {
            let end = i + c.len_utf8();
            out.push(&text[from..end]);
            from = end;
        }
    }
    out.push(&text[from..]);
    out
}

/// Character times of `narration` by edit-distance alignment to the timed
/// characters; unmatched ones are interpolated between matched neighbours.
pub fn align_chars(
    narration: &[char],
    timed: &[(char, f64, f64)],
    duration: f64,
) -> Vec<(f64, f64)> {
    let n = narration.len();
    if n == 0 {
        return vec![];
    }
    if timed.is_empty() {
        let step = duration / n as f64;
        return (0..n)
            .map(|i| (i as f64 * step, (i + 1) as f64 * step))
            .collect();
    }
    let m = timed.len();
    // Match 0 / mismatch 2 / insertion or deletion 1.
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    let mut back = vec![vec![0u8; m + 1]; n + 1];
    for (i, row) in dp.iter_mut().enumerate().skip(1) {
        row[0] = i as u32;
        back[i][0] = 1;
    }
    for j in 1..=m {
        dp[0][j] = j as u32;
        back[0][j] = 2;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = if narration[i - 1] == timed[j - 1].0 {
                0
            } else {
                2
            };
            let diagonal = dp[i - 1][j - 1] + cost;
            let up = dp[i - 1][j] + 1;
            let left = dp[i][j - 1] + 1;
            (dp[i][j], back[i][j]) = if diagonal <= up && diagonal <= left {
                (diagonal, 0)
            } else if up <= left {
                (up, 1)
            } else {
                (left, 2)
            };
        }
    }
    let mut times: Vec<Option<(f64, f64)>> = vec![None; n];
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && back[i][j] == 0 {
            times[i - 1] = Some((timed[j - 1].1, timed[j - 1].2));
            i -= 1;
            j -= 1;
        } else if i > 0 && (j == 0 || back[i][j] == 1) {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    fill_gaps(times, duration)
}

fn fill_gaps(mut times: Vec<Option<(f64, f64)>>, duration: f64) -> Vec<(f64, f64)> {
    let n = times.len();
    if times.iter().all(Option::is_none) {
        let step = (duration / n as f64).max(0.02);
        return (0..n)
            .map(|i| (i as f64 * step, duration.min((i + 1) as f64 * step)))
            .collect();
    }
    if times[0].is_none() {
        let first = times.iter().position(Option::is_some).unwrap();
        let (t0, t1) = (0.0, times[first].unwrap().0);
        for (k, slot) in times.iter_mut().enumerate().take(first) {
            let a = t0 + (t1 - t0) * k as f64 / first as f64;
            let b = t0 + (t1 - t0) * (k + 1) as f64 / first as f64;
            *slot = Some((a, b.max(a + 0.02)));
        }
    }
    if times[n - 1].is_none() {
        let last = times.iter().rposition(Option::is_some).unwrap();
        let (t0, t1) = (times[last].unwrap().1, duration);
        let gap = (n - 1 - last) as f64;
        for (j, k) in (last + 1..n).enumerate() {
            let a = t0 + (t1 - t0) * j as f64 / gap;
            let b = t0 + (t1 - t0) * (j + 1) as f64 / gap;
            times[k] = Some((a, b.max(a + 0.02)));
        }
    }
    let mut i = 0;
    while i < n {
        if times[i].is_some() {
            i += 1;
            continue;
        }
        let left = i - 1;
        let mut right = i;
        while right < n && times[right].is_none() {
            right += 1;
        }
        let t_left = times[left].unwrap().1;
        let mut t_right = times[right].unwrap().0;
        if t_right < t_left + 0.02 {
            t_right = duration.min(t_left + 0.02 * (right - left) as f64);
        }
        let holes = right - i;
        for (j, k) in (i..right).enumerate() {
            let a = t_left + (t_right - t_left) * (j + 1) as f64 / (holes + 1) as f64;
            let mut b = t_left + (t_right - t_left) * (j + 2) as f64 / (holes + 1) as f64;
            if j == holes - 1 {
                b = t_right;
            }
            times[k] = Some((a, b.max(a + 0.02)));
        }
        i = right;
    }
    let mut out = Vec::with_capacity(n);
    let mut previous_end = 0.0f64;
    for (a, b) in times.into_iter().map(Option::unwrap) {
        let mut a = a.max(previous_end);
        let mut b = b.max(a + 0.02).min(duration);
        if a >= duration {
            a = (duration - 0.04).max(0.0);
            b = duration;
        }
        out.push((a, b));
        previous_end = a;
    }
    out
}

fn snap_to_speech(t: f64, silences: &[(f64, f64)], min_silence: f64) -> f64 {
    let mut sorted = silences.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    for (s0, s1) in sorted {
        if s1 - s0 < min_silence {
            continue;
        }
        if s0 - 0.03 <= t && t < s1 {
            return s1;
        }
    }
    t
}

/// Subtitle cues for `script` along a narration timeline and its audio.
/// Shared by the narration recipe and the `subtitles` service of workers.
pub fn timeline_cues(
    script: &str,
    timeline: &crate::voice::Timeline,
    pcm: crate::voice::Pcm,
    max_chars: usize,
) -> Vec<Cue> {
    let chars: Vec<(char, f64, f64)> = timeline
        .chars
        .iter()
        .filter_map(|c| Some((c.text.chars().next()?, c.start, c.end)))
        .collect();
    let silences = detect_silences(pcm.samples, pcm.rate, -30.0, PAUSE_GAP);
    cues(
        script,
        &chars,
        timeline.duration.max(pcm.duration()),
        &silences,
        max_chars,
    )
}

/// Cues timed by the narration: never before the voice says them, never
/// overlapping the next, bridged over short gaps and released at long pauses.
pub fn cues(
    script: &str,
    chars: &[(char, f64, f64)],
    duration: f64,
    silences: &[(f64, f64)],
    max_chars: usize,
) -> Vec<Cue> {
    let segments = split_subtitles(script, max_chars);
    if segments.is_empty() {
        return vec![];
    }
    let full: Vec<char> = segments.iter().flat_map(|s| s.chars()).collect();
    let times = align_chars(&full, chars, duration);
    let mut long: Vec<(f64, f64)> = silences
        .iter()
        .copied()
        .filter(|(a, b)| b - a >= HOLD_SILENCE)
        .collect();
    long.sort_by(|a, b| a.partial_cmp(b).unwrap());

    struct Raw {
        start: f64,
        voice_end: f64,
        end: f64,
        text: String,
    }
    let mut raw: Vec<Raw> = Vec::new();
    let mut index = 0;
    for segment in &segments {
        let n = segment.chars().count();
        if n == 0 {
            continue;
        }
        let span = &times[index.min(times.len())..(index + n).min(times.len())];
        let (Some(first), Some(last)) = (span.first(), span.last()) else {
            break;
        };
        let start = snap_to_speech(first.0, silences, 0.15);
        let voice_end = last
            .1
            .max(start + (MIN_CUE_SEC * 0.6).max(n as f64 * MIN_CHAR_SEC * 0.85))
            .min(duration);
        raw.push(Raw {
            start,
            voice_end,
            end: voice_end,
            text: content_chars(segment),
        });
        index += n;
    }
    for i in 1..raw.len() {
        let previous = raw[i - 1].voice_end;
        if raw[i].start < previous + 0.04 {
            raw[i].start = previous + 0.04;
        }
        raw[i].start = snap_to_speech(raw[i].start, silences, 0.15);
        if raw[i].voice_end <= raw[i].start {
            raw[i].voice_end = duration.min(raw[i].start + MIN_CUE_SEC);
        }
    }
    for i in 0..raw.len() {
        if raw[i].text.is_empty() {
            continue;
        }
        let (start, voice_end) = (raw[i].start, raw[i].voice_end);
        let mut end = duration.min(voice_end + TAIL_PAD);
        if let Some(next) = raw.get(i + 1) {
            let next_start = next.start;
            let gap = next_start - voice_end;
            let breath = long.iter().any(|(s0, _)| start < *s0 && *s0 < next_start);
            if !breath && 0.0 < gap && gap <= FILL_GAP {
                end = end.max(next_start - 0.04);
            } else if breath {
                if let Some((s0, _)) = long
                    .iter()
                    .find(|(s0, _)| voice_end - 0.05 <= *s0 && *s0 < next_start)
                {
                    end = end.min(voice_end.max(*s0));
                }
            }
            end = end.min(next_start - 0.03);
        } else {
            // The last cue covers the end of the audio.
            end = duration.min((voice_end + TAIL_PAD).max(duration - 0.04));
        }
        if end <= start {
            end = duration.min(start + 0.25);
        }
        raw[i].end = end;
    }
    raw.into_iter()
        .filter(|r| !r.text.is_empty() && r.end > r.start + 0.08)
        .map(|r| Cue {
            text: r.text,
            start: r.start,
            end: r.end,
        })
        .collect()
}

/// Silent spans like FFmpeg `silencedetect=noise=-30dB:d=0.25`: every sample
/// below the threshold for at least `min_duration` seconds.
pub fn detect_silences(
    samples: &[i16],
    rate: u32,
    noise_db: f64,
    min_duration: f64,
) -> Vec<(f64, f64)> {
    // FFmpeg's int16 path: `noise * INT16_MAX` truncated, strictly below.
    let threshold = (10f64.powf(noise_db / 20.0) * 32767.0) as i32;
    let min_samples = (min_duration * f64::from(rate)).round() as usize;
    let mut out = Vec::new();
    let mut run_start: Option<usize> = None;
    for (i, sample) in samples.iter().enumerate() {
        let silent = i32::from(*sample).abs() < threshold;
        match (silent, run_start) {
            (true, None) => run_start = Some(i),
            (false, Some(start)) => {
                if i - start >= min_samples {
                    out.push((start as f64 / f64::from(rate), i as f64 / f64::from(rate)));
                }
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = run_start {
        if samples.len() - start >= min_samples {
            out.push((
                start as f64 / f64::from(rate),
                samples.len() as f64 / f64::from(rate),
            ));
        }
    }
    out
}
