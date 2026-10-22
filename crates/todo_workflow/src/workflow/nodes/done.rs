//! Builtin node: done.
use super::types::{DataKind, NodeGroup, NodeSpec, Port};

pub const SPEC: NodeSpec = NodeSpec {
        type_name: "done",
        label: "完成",
        group: NodeGroup::Basic,
        summary: "收束并回传摘要。",
        docs: "工作流终点。",
        inputs: &[Port {
            name: "result",
            kind: DataKind::Any,
            required: false,
        }],
        outputs: &[],
        fields: &[],
    };
