use serde::{Deserialize, Serialize};

use super::clip::Clip;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    /// Styled text; drawn above every video track.
    Text,
    Video,
    Audio,
    Sfx,
}

impl TrackKind {
    /// List / timeline section order. Lower draws higher and covers in preview.
    pub fn section_rank(self) -> u8 {
        match self {
            Self::Text => 0,
            Self::Video => 1,
            Self::Audio => 2,
            Self::Sfx => 3,
        }
    }

    /// Clips may move between lanes of one family: text, video, or audio/sfx.
    pub fn same_family(self, other: Self) -> bool {
        let family = |kind: Self| match kind {
            Self::Text => 0,
            Self::Video => 1,
            Self::Audio | Self::Sfx => 2,
        };
        family(self) == family(other)
    }

    /// Picture tracks take part in preview compositing.
    pub fn is_visual(self) -> bool {
        matches!(self, Self::Text | Self::Video)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub name: String,
    pub kind: TrackKind,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub clips: Vec<Clip>,
}

pub fn create_default_tracks() -> Vec<Track> {
    vec![
        Track {
            id: "v1".into(),
            name: "视频1".into(),
            kind: TrackKind::Video,
            muted: false,
            locked: false,
            clips: vec![],
        },
        Track {
            id: "a1".into(),
            name: "对白".into(),
            kind: TrackKind::Audio,
            muted: false,
            locked: false,
            clips: vec![],
        },
        Track {
            id: "sfx1".into(),
            name: "音效".into(),
            kind: TrackKind::Sfx,
            muted: false,
            locked: false,
            clips: vec![],
        },
    ]
}
