//! Builtin node: picture_batch.
use super::types::{DataKind, NodeGroup, NodeSpec, Port};

pub const SPEC: NodeSpec = NodeSpec {
        type_name: "picture_batch",
        label: "逐镜画面",
        group: NodeGroup::Shots,
        summary: "生图 / 元素 / 排版后按镜 emit。",
        docs: "齐图后再 shot_items emit。",
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
