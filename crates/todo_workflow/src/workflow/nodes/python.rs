//! Builtin node: python:* worker ops.

use super::types::{DataKind, NodeGroup, NodeSpec, Port};

pub const SPEC: NodeSpec = NodeSpec {
    type_name: "python",
    label: "自定义 Python",
    group: NodeGroup::Basic,
    summary: "调用扩展 worker 的单个 op。",
    docs: "python:<op>，op 须在 worker.ops 中声明。",
    inputs: &[Port {
        name: "in",
        kind: DataKind::Any,
        required: false,
    }],
    outputs: &[Port {
        name: "out",
        kind: DataKind::Any,
        required: false,
    }],
    fields: &[],
};
