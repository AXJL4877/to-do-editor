//! Builtin node: segment_script.
use super::types::{DataKind, NodeGroup, NodeSpec, ParamField, Port};

const FIELDS: &[ParamField] = &[
    ParamField::Select {
        id: "mode",
        label: "分段方式",
        hint: "语意=完整意群；字数=目标长度；段落=空行；语句=一句一段（。！？等）",
        options: &[
            ("semantic", "按语意"),
            ("length", "按字数"),
            ("paragraph", "按段落"),
            ("sentence", "按语句"),
        ],
        default: "semantic",
    },
    // Shown only when mode=length (inspector filters). Soft length hint for
    // semantic still uses this default inside the runner.
    ParamField::Number {
        id: "target",
        label: "目标字数",
        hint: "按字数分段时，尽量靠近这个长度",
        min: 20.0,
        max: 1000.0,
        step: 10.0,
        default: 120.0,
        unit: Some("字"),
    },
    ParamField::Text {
        id: "model",
        label: "模型",
        hint: "仅按语意时使用；留空跟随 AI 设置",
        default: "",
        placeholder: "deepseek-flash …",
    },
    ParamField::Number {
        id: "max",
        label: "分段字数上限",
        hint: "语意 / 按字数时的单段硬上限；超长会再拆。按语句、按段落不使用此上限。",
        min: 20.0,
        max: 1200.0,
        step: 10.0,
        default: 200.0,
        unit: Some("字"),
    },
];

pub const SPEC: NodeSpec = NodeSpec {
    type_name: "segment_script",
    label: "口播分段",
    group: NodeGroup::Script,
    summary: "按语意 / 字数 / 段落 / 语句切开整篇口播，不改写原文。",
    docs: "输入整篇 script_text，输出 segments。拼接各段 text 须还原原文。语意模式调 chat 只选切点；字数 / 段落 / 语句为纯规则（语句=按。！？等一句一段，不按字数再拆）。下游 tts 按段数逐段配音。",
    inputs: &[Port {
        name: "text",
        kind: DataKind::ScriptText,
        required: true,
    }],
    outputs: &[Port {
        name: "segments",
        kind: DataKind::Segments,
        required: false,
    }],
    fields: FIELDS,
};
