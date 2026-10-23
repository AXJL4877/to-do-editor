//! Builtin node: capture timeline frames on an absolute clock and stitch them.
use super::types::{DataKind, NodeGroup, NodeSpec, ParamField, Port};

const FIELDS: &[ParamField] = &[
    ParamField::Number {
        id: "everyFrames",
        label: "间隔帧数",
        hint: "按时间轴绝对时间取样：第 0、N、2N… 帧（秒 = 帧号 / fps）",
        min: 1.0,
        max: 300.0,
        step: 1.0,
        default: 30.0,
        unit: Some("帧"),
    },
    ParamField::Number {
        id: "maxFrames",
        label: "最多帧数",
        hint: "防止超长时间线一次截太多；超出则均匀截断",
        min: 1.0,
        max: 64.0,
        step: 1.0,
        default: 24.0,
        unit: Some("张"),
    },
    ParamField::Number {
        id: "thumbHeight",
        label: "拼图单格高",
        hint: "拼接接触片时每格缩放到的高度（像素）",
        min: 64.0,
        max: 480.0,
        step: 8.0,
        default: 160.0,
        unit: Some("px"),
    },
    ParamField::Select {
        id: "range",
        label: "取样范围",
        hint: "content=已有片段末端；workspace=工程时间轴长度",
        options: &[("content", "内容时长"), ("workspace", "工作区时长")],
        default: "content",
    },
];

pub const SPEC: NodeSpec = NodeSpec {
    type_name: "timeline_strip",
    label: "时间轴截帧",
    group: NodeGroup::Timeline,
    summary: "按绝对时间每 N 帧截一张节目监视器画面，拼成接触片供下游 AI 看。",
    docs: "不读上游媒体，直接渲染当前工程时间轴。默认每 30 帧一张（30fps 即每秒）。输出 image=拼接图，frames=单帧列表（可接中转预览）。",
    inputs: &[Port {
        name: "start",
        kind: DataKind::Trigger,
        required: false,
    }],
    outputs: &[
        Port {
            name: "image",
            kind: DataKind::ImageAsset,
            required: false,
        },
        Port {
            name: "frames",
            kind: DataKind::ImageAssets,
            required: false,
        },
    ],
    fields: FIELDS,
};
