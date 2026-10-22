//! Builtin node: edit_text — rewrite each upstream text block via LLM.
use super::types::{DataKind, NodeGroup, NodeSpec, ParamField, Port};

const FIELDS: &[ParamField] = &[
    ParamField::Text {
        id: "model",
        label: "模型",
        hint: "留空用 AI 设置里的聊天模型；填网关上的模型名可覆盖",
        default: "",
        placeholder: "deepseek-flash / claude-fable-5 …",
    },
    ParamField::Text {
        id: "system",
        label: "系统提示词",
        hint: "告诉模型怎么改每一段；段数由节点保证与上游一致",
        default: "你是口播文案编辑。改写用户给出的这一段口播，保持原意与口语感，只输出改写后的正文，不要标题、编号或解释。",
        placeholder: "你是…",
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
    type_name: "edit_text",
    label: "编辑文本",
    group: NodeGroup::Script,
    summary: "按上游文本块逐段调用大模型改写，输出相同数量的文本块。",
    docs: "输入 text/segments（N 块）。对每一块单独请求 LLM（可选模型、系统提示词）。输出仍是 N 块 segments，可直接接 TTS。块数由节点保证，不依赖模型自己数段。",
    inputs: &[Port {
        name: "text",
        kind: DataKind::ScriptText,
        required: true,
    }],
    outputs: &[Port {
        name: "text",
        kind: DataKind::Segments,
        required: false,
    }],
    fields: FIELDS,
};
