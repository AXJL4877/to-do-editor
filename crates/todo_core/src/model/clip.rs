use crate::transform::Transform;
use serde::{Deserialize, Serialize};

use super::text::TextContent;
use super::{ClipAudio, ClipOrigin, Intro};
pub use crate::transform::ClipTransform;

pub const MIN_CLIP_DURATION: f64 = 1.0 / 240.0;
pub const MIN_CLIP_RATE: f64 = 0.1;
pub const MAX_CLIP_RATE: f64 = 8.0;

/// A timeline item: shared timing/placement plus what it shows.
///
/// JSON stays flat. Media clips keep the legacy `mediaId`/`offset`/`rate` fields;
/// text clips carry a `text` object instead and never have a source interval.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(try_from = "ClipRecord", into = "ClipRecord")]
pub struct Clip {
    pub id: String,
    pub start: f64,
    pub duration: f64,
    pub link_id: Option<String>,
    pub transform: Transform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<Intro>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<ClipAudio>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<ClipOrigin>,
    /// Seconds of opacity fade at the end of a picture clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outro: Option<f64>,
    pub source: ClipSource,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClipSource {
    Media(MediaSource),
    Text(TextContent),
}

/// Interval of a library item played at `rate`, starting at source time `offset`.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaSource {
    pub media_id: String,
    pub offset: f64,
    pub rate: f64,
}

impl Clip {
    pub fn media(&self) -> Option<&MediaSource> {
        match &self.source {
            ClipSource::Media(media) => Some(media),
            ClipSource::Text(_) => None,
        }
    }

    pub fn media_mut(&mut self) -> Option<&mut MediaSource> {
        match &mut self.source {
            ClipSource::Media(media) => Some(media),
            ClipSource::Text(_) => None,
        }
    }

    pub fn media_id(&self) -> Option<&str> {
        self.media().map(|media| media.media_id.as_str())
    }

    pub fn text(&self) -> Option<&TextContent> {
        match &self.source {
            ClipSource::Text(text) => Some(text),
            ClipSource::Media(_) => None,
        }
    }

    pub fn text_mut(&mut self) -> Option<&mut TextContent> {
        match &mut self.source {
            ClipSource::Text(text) => Some(text),
            ClipSource::Media(_) => None,
        }
    }

    pub fn is_text(&self) -> bool {
        matches!(self.source, ClipSource::Text(_))
    }
}

pub fn normalize_clip(
    id: String,
    media_id: String,
    start: f64,
    duration: f64,
    offset: f64,
    rate: f64,
) -> Clip {
    Clip {
        id,
        start: start.max(0.0),
        duration: duration.max(MIN_CLIP_DURATION),
        link_id: None,
        transform: Transform::default(),
        intro: None,
        audio: None,
        origin: None,
        outro: None,
        source: ClipSource::Media(MediaSource {
            media_id,
            offset: offset.max(0.0),
            rate: rate.clamp(MIN_CLIP_RATE, MAX_CLIP_RATE),
        }),
    }
}

pub fn text_clip(id: String, start: f64, duration: f64, content: TextContent) -> Clip {
    Clip {
        id,
        start: start.max(0.0),
        duration: duration.max(MIN_CLIP_DURATION),
        link_id: None,
        transform: Transform::default(),
        intro: None,
        audio: None,
        origin: None,
        outro: None,
        source: ClipSource::Text(content),
    }
}

/// Clamp persisted timing the same way constructors do, keeping identity and content.
pub(crate) fn normalize_existing(mut clip: Clip) -> Clip {
    clip.start = clip.start.max(0.0);
    clip.duration = clip.duration.max(MIN_CLIP_DURATION);
    if let Some(media) = clip.media_mut() {
        media.offset = media.offset.max(0.0);
        media.rate = media.rate.clamp(MIN_CLIP_RATE, MAX_CLIP_RATE);
    }
    clip
}

/// Source seconds covered by a media clip; text clips cover their own duration.
pub fn source_span(clip: &Clip) -> f64 {
    clip.duration * clip.media().map_or(1.0, |media| media.rate)
}

pub fn clip_end(clip: &Clip) -> f64 {
    clip.start + clip.duration
}

/// Wire shape of [`Clip`].
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClipRecord {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    media_id: Option<String>,
    start: f64,
    duration: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offset: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    link_id: Option<String>,
    #[serde(default, skip_serializing_if = "Transform::is_default")]
    transform: Transform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    intro: Option<Intro>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audio: Option<ClipAudio>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    origin: Option<ClipOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    outro: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<TextContent>,
}

impl TryFrom<ClipRecord> for Clip {
    type Error = String;

    fn try_from(record: ClipRecord) -> Result<Self, Self::Error> {
        let source = match (record.media_id, record.text) {
            (Some(media_id), None) => ClipSource::Media(MediaSource {
                media_id,
                offset: record
                    .offset
                    .ok_or_else(|| format!("clip {} is missing offset", record.id))?,
                rate: record
                    .rate
                    .ok_or_else(|| format!("clip {} is missing rate", record.id))?,
            }),
            (None, Some(text)) => {
                if record.offset.is_some() || record.rate.is_some() {
                    return Err(format!("text clip {} cannot have offset/rate", record.id));
                }
                ClipSource::Text(text)
            }
            (Some(_), Some(_)) => {
                return Err(format!("clip {} has both mediaId and text", record.id))
            }
            (None, None) => return Err(format!("clip {} needs mediaId or text", record.id)),
        };
        Ok(Clip {
            id: record.id,
            start: record.start,
            duration: record.duration,
            link_id: record.link_id,
            transform: record.transform,
            intro: record.intro,
            audio: record.audio,
            origin: record.origin,
            outro: record.outro.filter(|s| s.is_finite() && *s > 0.0),
            source,
        })
    }
}

impl From<Clip> for ClipRecord {
    fn from(clip: Clip) -> Self {
        let (media_id, offset, rate, text) = match clip.source {
            ClipSource::Media(media) => (
                Some(media.media_id),
                Some(media.offset),
                Some(media.rate),
                None,
            ),
            ClipSource::Text(text) => (None, None, None, Some(text)),
        };
        ClipRecord {
            id: clip.id,
            media_id,
            start: clip.start,
            duration: clip.duration,
            offset,
            rate,
            link_id: clip.link_id,
            transform: clip.transform,
            intro: clip.intro,
            audio: clip.audio,
            origin: clip.origin,
            outro: clip.outro,
            text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_clip_json_is_unchanged() {
        let clip = normalize_clip("c".into(), "m".into(), 1.0, 2.0, 0.5, 1.0);
        let value = serde_json::to_value(&clip).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"id": "c", "mediaId": "m", "start": 1.0, "duration": 2.0,
                "offset": 0.5, "rate": 1.0})
        );
        assert_eq!(serde_json::from_value::<Clip>(value).unwrap(), clip);
    }

    #[test]
    fn text_clip_json_has_text_and_no_source_interval() {
        let clip = text_clip("t".into(), 0.0, 3.0, TextContent::new("你好", "s"));
        let value = serde_json::to_value(&clip).unwrap();
        assert_eq!(value["text"]["text"], "你好");
        assert!(value.get("mediaId").is_none() && value.get("offset").is_none());
        assert_eq!(serde_json::from_value::<Clip>(value).unwrap(), clip);
        assert_eq!(source_span(&clip), 3.0);
    }

    #[test]
    fn ambiguous_records_are_rejected() {
        let both = r#"{"id":"x","mediaId":"m","start":0,"duration":1,"offset":0,"rate":1,
            "text":{"text":"a","styleId":"s"}}"#;
        assert!(serde_json::from_str::<Clip>(both).is_err());
        let neither = r#"{"id":"x","start":0,"duration":1}"#;
        assert!(serde_json::from_str::<Clip>(neither).is_err());
        let text_rate =
            r#"{"id":"x","start":0,"duration":1,"rate":2,"text":{"text":"a","styleId":"s"}}"#;
        assert!(serde_json::from_str::<Clip>(text_rate).is_err());
    }

    #[test]
    fn optional_effects_round_trip_without_changing_legacy_shape() {
        let mut clip = normalize_clip("c".into(), "m".into(), 0.0, 2.0, 0.0, 1.0);
        clip.intro = Some(Intro {
            kind: crate::IntroKind::Rise,
            duration: 0.5,
        });
        clip.audio = Some(crate::ClipAudio {
            gain: 0.75,
            fade_in: 0.2,
            fade_out: 0.3,
        });
        clip.origin = Some(crate::ClipOrigin {
            production: "p".into(),
            key: "s0/voice".into(),
            rev: "r1".into(),
        });
        let value = serde_json::to_value(&clip).unwrap();
        assert_eq!(value["intro"]["kind"], "rise");
        assert_eq!(serde_json::from_value::<Clip>(value).unwrap(), clip);
    }
}
