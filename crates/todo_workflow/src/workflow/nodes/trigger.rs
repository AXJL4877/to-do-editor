//! Builtin node: trigger.
use super::types::{DataKind, NodeGroup, NodeSpec, Port};

pub const SPEC: NodeSpec = NodeSpec {
        type_name: "trigger",
        label: "触发器",
        group: NodeGroup::Basic,
        summary: "手动或定时启动整条工作流。",
        docs: "工作流入口开关。点击运行时从此口发出 trigger；可接到素材容器 / LLM / 脚本等下游。无输入口。",
        inputs: &[],
        outputs: &[Port {
            name: "start",
            kind: DataKind::Trigger,
            required: false,
        }],
        fields: &[],
    };
