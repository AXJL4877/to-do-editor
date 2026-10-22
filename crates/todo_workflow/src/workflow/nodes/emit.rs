//! Builtin node: emit.
use super::types::{DataKind, NodeGroup, NodeSpec, Port};

pub const SPEC: NodeSpec = NodeSpec {
    type_name: "emit",
    label: "入轨",
    group: NodeGroup::Timeline,
    summary: "把配音 / 画面推上时间轴；可按文案吸附到逐字时间。",
    docs: "时间基准永远是 clips（尺），不是生图段。\n\
接法：\n\
1) payload：旧行为。\n\
2) clips + media：尺 + 声素材入声轨。\n\
3) 再加 visuals：画面按尺对齐。\n\
推荐拆法：TTS.clips+media → 顺序入轨；TTS.clips+图 → 画面入轨。",
    inputs: &[
        Port {
            name: "payload",
            kind: DataKind::Any,
            required: false,
        },
        Port {
            name: "clips",
            kind: DataKind::SpeechClips,
            required: false,
        },
        Port {
            name: "media",
            kind: DataKind::MediaBin,
            required: false,
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
