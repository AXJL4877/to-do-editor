//! Builtin node: emit_visuals — place pictures on the clip clock only.
use super::types::{DataKind, NodeGroup, NodeSpec, Port};

pub const SPEC: NodeSpec = NodeSpec {
    type_name: "emit_visuals",
    label: "画面入轨",
    group: NodeGroup::Timeline,
    summary: "clips 当尺；visuals 画面素材按尺挂轨（不排配音、不传 wav）。",
    docs: "clips：来自 TTS.clips 的纯时间尺（不要接 media）。\n\
visuals：生图 / 中转画面。\n\
带 script/text 时按文案吸附逐字轴；否则按 id / 下标铺满对应段窗。",
    inputs: &[
        Port {
            name: "clips",
            kind: DataKind::SpeechClips,
            required: true,
        },
        Port {
            name: "visuals",
            kind: DataKind::Any,
            required: true,
        },
    ],
    outputs: &[Port {
        name: "items",
        kind: DataKind::Items,
        required: false,
    }],
    fields: &[],
};
