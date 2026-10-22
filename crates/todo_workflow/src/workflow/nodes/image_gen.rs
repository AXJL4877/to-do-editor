use super::types::{DataKind, NodeGroup, NodeSpec, ParamField, Port};

const FIELDS: &[ParamField] = &[
    ParamField::Select {
        id: "size",
        label: "尺寸",
        hint: "生图分辨率预设",
        options: &[
            ("1024x1024", "1024²"),
            ("1280x720", "1280×720"),
            ("720x1280", "720×1280"),
            ("1920x1080", "1920×1080"),
        ],
        default: "1024x1024",
    },
    ParamField::Select {
        id: "transparent",
        label: "透明底",
        hint: "需要抠图/元素层时打开（宿主支持时生效）",
        options: &[("false", "不透明"), ("true", "透明 PNG")],
        default: "false",
    },
    ParamField::Select {
        id: "quality",
        label: "质量",
        hint: "部分模型支持；留默认即可",
        options: &[("default", "默认"), ("high", "高"), ("low", "低")],
        default: "default",
    },
    ParamField::Text {
        id: "style",
        label: "风格提示",
        hint: "附加在每一段 prompt 后的风格词",
        default: "",
        placeholder: "扁平插画，干净背景",
    },
    ParamField::Text {
        id: "model",
        label: "模型",
        hint: "留空用 AI 设置里的生图模型；可填网关上的其它模型名",
        default: "",
        placeholder: "gpt-image-2 …",
    },
];

pub const SPEC: NodeSpec = NodeSpec {
    type_name: "image_gen",
    label: "生图",
    group: NodeGroup::Ai,
    summary: "文生图：单段一张；多文本块并行生图，完成一张就能预览。",
    docs: "prompt 接整篇或分段。多段时各段并行生图，每完成一张就写入 images 供下游中转预览。全部完成后输出 image（首张）+ images。ref 可选参考图（各段共用）。",
    inputs: &[
        Port {
            name: "prompt",
            kind: DataKind::ScriptText,
            required: true,
        },
        Port {
            name: "ref",
            kind: DataKind::ImageAsset,
            required: false,
        },
        Port {
            name: "start",
            kind: DataKind::Trigger,
            required: false,
        },
    ],
    outputs: &[
        Port {
            name: "image",
            kind: DataKind::ImageAsset,
            required: false,
        },
        Port {
            name: "images",
            kind: DataKind::ImageAssets,
            required: false,
        },
    ],
    fields: FIELDS,
};
