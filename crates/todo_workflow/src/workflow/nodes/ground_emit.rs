//! Builtin node: ground_emit.
use super::types::{DataKind, NodeGroup, NodeSpec, Port};

pub const SPEC: NodeSpec = NodeSpec {
        type_name: "ground_emit",
        label: "底板字幕入轨",
        group: NodeGroup::Timeline,
        summary: "背景板、字幕、升华字 emit。",
        docs: "分镜定稿后的第二批入轨。",
        inputs: &[Port {
            name: "plan",
            kind: DataKind::Any,
            required: true,
        }],
        outputs: &[Port {
            name: "items",
            kind: DataKind::Items,
            required: false,
        }],
        fields: &[],
    };
