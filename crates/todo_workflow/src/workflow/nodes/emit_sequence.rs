//! Builtin node: emit_sequence — place voice on the clip clock.
use super::types::{DataKind, NodeGroup, NodeSpec, Port};

pub const SPEC: NodeSpec = NodeSpec {
    type_name: "emit_sequence",
    label: "顺序入轨",
    group: NodeGroup::Timeline,
    summary: "clips 当尺；media 声素材按尺入声轨。",
    docs: "clips：唯一时间尺（来自 TTS.clips，无 wav）。\n\
media：声素材（来自 TTS.media）。\n\
visuals 可选：画面也可挂本节点（兼容）；更清晰请用「画面入轨」。\n\
流式 TTS 已入声轨时跳过配音重复排轨。",
    inputs: &[
        Port {
            name: "clips",
            kind: DataKind::SpeechClips,
            required: true,
        },
        Port {
            name: "media",
            kind: DataKind::MediaBin,
            required: true,
        },
        Port {
            name: "visuals",
            kind: DataKind::Any,
            required: false,
        },
    ],
    outputs: &[Port {
        name: "items",
        kind: DataKind::Items,
        required: false,
    }],
    fields: &[],
};
