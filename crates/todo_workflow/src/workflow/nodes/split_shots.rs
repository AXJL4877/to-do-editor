//! Builtin node: split_shots.
use super::types::{DataKind, NodeGroup, NodeSpec, Port};

pub const SPEC: NodeSpec = NodeSpec {
        type_name: "split_shots",
        label: "拆镜",
        group: NodeGroup::Shots,
        summary: "配音、字幕和口播段落 → 稳定镜头区间。",
        docs: "优先按口播段落对齐字幕，再合并过短 cue；没有段落时退回字幕断点。每个 shot 都带可追溯的 segmentIndex。",
        inputs: &[
            Port {
                name: "speech",
                kind: DataKind::SpeechClip,
                required: true,
            },
            Port {
                name: "cues",
                kind: DataKind::SubtitleCues,
                required: false,
            },
            Port {
                name: "segments",
                kind: DataKind::Segments,
                required: false,
            },
        ],
        outputs: &[Port {
            name: "shots",
            kind: DataKind::Any,
            required: false,
        }],
        fields: &[],
    };
