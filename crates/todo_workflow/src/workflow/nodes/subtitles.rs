//! Builtin node: subtitles — cues + styled text items for the timeline.
use super::types::{DataKind, NodeGroup, NodeSpec, ParamField, Port};

const FIELDS: &[ParamField] = &[
    ParamField::Select {
        id: "styleId",
        label: "字幕样式",
        hint: "工程内置花字样式（含字体）。换样式即换字体观感。",
        options: &[
            ("subtitle-kuaile", "圆体白字粗黑边（站酷快乐体）"),
            ("subtitle-white-heavy", "白字粗黑边（Noto Sans）"),
        ],
        default: "subtitle-kuaile",
    },
    ParamField::Number {
        id: "fontSize",
        label: "字号",
        hint: "相对画布高度的比例（0.04 ≈ 小字幕，0.08 ≈ 大字幕）",
        min: 0.03,
        max: 0.14,
        step: 0.005,
        default: 0.055,
        unit: None,
    },
    ParamField::Number {
        id: "y",
        label: "垂直位置",
        hint: "相对画布中心：0 居中，正数偏下，负数偏上。常见字幕约 0.32～0.40。",
        min: -0.45,
        max: 0.45,
        step: 0.01,
        default: 0.36,
        unit: None,
    },
    ParamField::Number {
        id: "maxChars",
        label: "每条字数",
        hint: "在自然停顿与标点处切分；超过上限会继续拆开",
        min: 6.0,
        max: 32.0,
        step: 1.0,
        default: 18.0,
        unit: Some("字"),
    },
    ParamField::Text {
        id: "canvas",
        label: "工程画幅",
        hint: "运行时由宿主填入当前工程分辨率，只读参考",
        default: "",
        placeholder: "打开工程后自动读取",
    },
];

pub const SPEC: NodeSpec = NodeSpec {
    type_name: "subtitles",
    label: "字幕",
    group: NodeGroup::Audio,
    summary: "逐字时间轴 + 口播原稿 → 可选样式的字幕（可入轨预览）。",
    docs: "script：口播原稿。speech：时间尺（TTS.clips，可无 file）。\n\
media：声素材（TTS.media）；speech 无 file 时从这里取 wav。\n\
按时间轴切字幕，输出 cues 与可入轨文字条。",
    inputs: &[
        Port {
            name: "script",
            kind: DataKind::ScriptText,
            required: true,
        },
        Port {
            name: "speech",
            kind: DataKind::SpeechClip,
            required: true,
        },
        Port {
            name: "media",
            kind: DataKind::MediaBin,
            required: false,
        },
    ],
    outputs: &[
        Port {
            name: "cues",
            kind: DataKind::SubtitleCues,
            required: false,
        },
        Port {
            name: "items",
            kind: DataKind::Items,
            required: false,
        },
    ],
    fields: FIELDS,
};
