//! Builtin node: choose_motions.
use super::types::{DataKind, NodeGroup, NodeSpec, Port};

pub const SPEC: NodeSpec = NodeSpec {
        type_name: "choose_motions",
        label: "动效选择",
        group: NodeGroup::Shots,
        summary: "白名单动效写入镜头计划。",
        docs: "种子随机 + 白名单。",
        inputs: &[Port {
            name: "plan",
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
