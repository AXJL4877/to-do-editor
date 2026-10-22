//! Builtin node: script.
use super::types::{DataKind, NodeGroup, NodeSpec, Port};

pub const SPEC: NodeSpec = NodeSpec {
        type_name: "script",
        label: "原始口播",
        group: NodeGroup::Script,
        summary: "读取文案或按主题生成，输出整篇口播文本。",
        docs: "优先用上游素材容器的 text；否则用成片输入框文案；仍为空则按主题 write_script。",
        inputs: &[Port {
            name: "text",
            kind: DataKind::ScriptText,
            required: false,
        }],
        outputs: &[Port {
            name: "script",
            kind: DataKind::ScriptText,
            required: false,
        }],
        fields: &[],
    };
