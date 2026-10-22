//! Builtin node: llm_chat.
use super::types::{DataKind, NodeGroup, NodeSpec, ParamField, Port};

const FIELDS: &[ParamField] = &[
    ParamField::Select {
        id: "capability",
        label: "能力",
        hint: "走宿主绑定的聊天模型",
        options: &[("chat", "文本对话"), ("reasoning", "长推理")],
        default: "chat",
    },
    ParamField::Text {
        id: "system",
        label: "系统提示",
        hint: "可选；覆盖默认角色设定",
        default: "",
        placeholder: "你是…",
    },
    ParamField::Text {
        id: "model",
        label: "模型",
        hint: "留空用 AI 设置里的聊天模型；填网关上的模型名可覆盖",
        default: "",
        placeholder: "deepseek-flash / claude-fable-5 …",
    },
    ParamField::Select {
        id: "json",
        label: "输出",
        hint: "强制 JSON（分段、抽字段这类下游要结构时打开）",
        options: &[("text", "自然语言"), ("json", "强制 JSON")],
        default: "text",
    },
    ParamField::Number {
        id: "temperature",
        label: "温度",
        hint: "越高越发散",
        min: 0.0,
        max: 2.0,
        step: 0.1,
        default: 0.7,
        unit: None,
    },
];

pub const SPEC: NodeSpec = NodeSpec {
        type_name: "llm_chat",
        label: "对话",
        group: NodeGroup::Ai,
        summary: "调用宿主 LLM：文案进、回复出。",
        docs: "走 AI 设置里的聊天模型。prompt 为用户话；可选 system。输出 chat_reply（可当文案接到分段/TTS）。",
        inputs: &[
            Port {
                name: "prompt",
                kind: DataKind::ScriptText,
                required: true,
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
