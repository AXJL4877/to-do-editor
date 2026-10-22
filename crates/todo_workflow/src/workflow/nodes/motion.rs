use super::types::{DataKind, NodeGroup, NodeSpec, ParamField, Port};

const INTROS: &[(&str, &str)] = &[
    ("", "无"),
    ("fade", "淡入"),
    ("rise", "上浮"),
    ("pop", "弹出"),
    ("slideLeft", "左侧滑入"),
    ("slideRight", "右侧滑入"),
    ("drop", "落下弹跳"),
    ("stamp", "盖章"),
    ("grow", "放大出现"),
];

const OUTROS: &[(&str, &str)] = &[
    ("", "无"),
    ("fadeOut", "淡出"),
    ("shrinkOut", "缩小消失"),
    ("sinkOut", "下沉消失"),
    ("slideOutLeft", "向左滑出"),
    ("slideOutRight", "向右滑出"),
    ("popOut", "弹出消失"),
];

const TRANSITIONS: &[(&str, &str)] = &[
    ("", "无"),
    ("dissolve", "叠化"),
    ("crossfade", "交叉淡化"),
    ("dip", "淡出淡入"),
    ("pushLeft", "向左推"),
    ("pushRight", "向右推"),
    ("coverLeft", "从右覆盖"),
    ("coverRight", "从左覆盖"),
    ("zoomIn", "推近叠化"),
];

const FIELDS: &[ParamField] = &[
    ParamField::Select {
        id: "intro",
        label: "入场",
        hint: "动效库 intro；入轨时写到片段 intro",
        options: INTROS,
        default: "pop",
    },
    ParamField::Select {
        id: "outro",
        label: "出场",
        hint: "动效库 outro；入轨时写到片段 outro",
        options: OUTROS,
        default: "fadeOut",
    },
    ParamField::Select {
        id: "transition",
        label: "转场",
        hint: "动效库 transition；与同轨前一条衔接",
        options: TRANSITIONS,
        default: "",
    },
];

pub const SPEC: NodeSpec = NodeSpec {
    type_name: "motion",
    label: "动效",
    group: NodeGroup::Shots,
    summary: "给元素绑定入场/出场/转场，入轨自动带上。",
    docs: "接生图的 image / images。输出仍是元素，并附 intro、outro、transition，入轨时自动落到片段上。",
    inputs: &[Port {
        name: "elements",
        kind: DataKind::ImageAssets,
        required: true,
    }],
    outputs: &[
        Port {
            name: "element",
            kind: DataKind::ImageAsset,
            required: false,
        },
        Port {
            name: "elements",
            kind: DataKind::ImageAssets,
            required: false,
        },
    ],
    fields: FIELDS,
};
