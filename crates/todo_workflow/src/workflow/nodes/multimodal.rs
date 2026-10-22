//! Builtin node: multimodal.
use super::types::{DataKind, NodeGroup, NodeSpec, ParamField, Port};

const FIELDS: &[ParamField] = &[
    ParamField::Select {
        id: "mode",
        label: "模式",
        hint: "看图说话 / 看图改图说明",
        options: &[("understand", "视觉理解"), ("edit_brief", "改图说明")],
        default: "understand",
    },
    ParamField::Number {
        id: "temperature",
        label: "温度",
        hint: "越高越发散",
        min: 0.0,
        max: 2.0,
        step: 0.1,
        default: 0.4,
        unit: None,
    },
];

pub const SPEC: NodeSpec = NodeSpec {
        type_name: "multimodal",
        label: "看图问答",
        group: NodeGroup::Ai,
        summary: "图文理解：图片 + 提问 → 文本回复。",
        docs: "视觉模型。images 来自素材容器或生图节点；prompt 为问题。输出 chat_reply。",
        inputs: &[
            Port {
                name: "prompt",
                kind: DataKind::ScriptText,
                required: true,
            },
            Port {
                name: "images",
                kind: DataKind::MediaBin,
                required: false,
            },
            Port {
                name: "image",
                kind: DataKind::ImageAsset,
                required: false,
            },
            Port {
                name: "start",
                kind: DataKind::Trigger,
                required: false,
            },
        ],
        outputs: &[Port {
            name: "reply",
            kind: DataKind::ChatReply,
            required: false,
        }],
        fields: FIELDS,
    };
