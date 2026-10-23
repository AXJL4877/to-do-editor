//! Builtin node: wait until every wired upstream is done and host placement is idle.
use super::types::{DataKind, NodeGroup, NodeSpec, Port};

/// Fan-in slots: graph waits for every wired optional before this node runs.
const INPUTS: &[Port] = &[
    Port {
        name: "in1",
        kind: DataKind::Any,
        required: false,
    },
    Port {
        name: "in2",
        kind: DataKind::Any,
        required: false,
    },
    Port {
        name: "in3",
        kind: DataKind::Any,
        required: false,
    },
    Port {
        name: "in4",
        kind: DataKind::Any,
        required: false,
    },
    Port {
        name: "in5",
        kind: DataKind::Any,
        required: false,
    },
    Port {
        name: "in6",
        kind: DataKind::Any,
        required: false,
    },
    Port {
        name: "in7",
        kind: DataKind::Any,
        required: false,
    },
    Port {
        name: "in8",
        kind: DataKind::Any,
        required: false,
    },
];

pub const SPEC: NodeSpec = NodeSpec {
    type_name: "placement_ready",
    label: "入轨就绪",
    group: NodeGroup::Timeline,
    summary: "等所有已接线上游跑完，且时间轴入轨落地后，再往下发脉冲。",
    docs: "把多个入轨 / TTS / 生图支路接到 in1…in8。图侧等全部上游结束；宿主再等 adopt+Place 空闲，然后输出 ready，供截帧等节点使用。",
    inputs: INPUTS,
    outputs: &[Port {
        name: "ready",
        kind: DataKind::Trigger,
        required: false,
    }],
    fields: &[],
};
