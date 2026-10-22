//! Builtin node: label_shots.
use super::types::{DataKind, NodeGroup, NodeSpec, Port};

pub const SPEC: NodeSpec = NodeSpec {
        type_name: "label_shots",
        label: "镜头标签",
        group: NodeGroup::Shots,
        summary: "为每镜打画面标签（LLM）。",
        docs: "story / sublime、元素与关系写入 plan。",
        inputs: &[Port {
            name: "shots",
            kind: DataKind::Any,
            required: true,
        }],
        outputs: &[Port {
            name: "plan",
            kind: DataKind::Any,
            required: false,
        }],
        fields: &[],
    };
