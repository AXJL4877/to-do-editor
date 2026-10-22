//! Builtin node: tts.
use super::types::{DataKind, NodeGroup, NodeSpec, ParamField, Port};

const VOICES: &[(&str, &str)] = &[
    ("", "跟随 AI 设置"),
    ("S_RksT6Z6g2", "克隆音色（口播）"),
    ("zh_female_mizai_uranus_bigtts", "咪仔（女）"),
    ("zh_male_m191_uranus_bigtts", "云舟（男）"),
    ("zh_female_vv_uranus_bigtts", "Vivi（女）"),
];

const FIELDS: &[ParamField] = &[
    ParamField::Select {
        id: "voice",
        label: "音色",
        hint: "克隆音色需本机火山账号已复刻；「跟随 AI 设置」用设置页默认说话人",
        options: VOICES,
        default: "",
    },
    ParamField::Number {
        id: "rate",
        label: "语速",
        hint: "相对默认语速的百分比偏移",
        min: -50.0,
        max: 100.0,
        step: 1.0,
        default: 0.0,
        unit: Some("%"),
    },
    ParamField::Select {
        id: "handoff",
        label: "出流方式",
        hint: "阻塞：全部段配完再交给下游；流式：每段完成立即入轨，不等整批",
        options: &[
            ("wait", "阻塞（整批）"),
            ("stream", "流式（逐段入轨）"),
        ],
        default: "wait",
    },
];

pub const SPEC: NodeSpec = NodeSpec {
    type_name: "tts",
    label: "口播转 TTS",
    group: NodeGroup::Audio,
    summary: "逐段配音 → clips（时间尺）+ media（声素材），二者分开。",
    docs: "text 接 script_text 或 segments。上游 N 块 → N 次配音。\n\
clips：纯时间尺（id / duration / timeline.chars），不含 wav。\n\
media：声素材 MediaBin（path + kind=audio + id），与 clips 按 id 对齐。\n\
推荐：clips → 顺序入轨.clips / 画面入轨.clips；media → 顺序入轨.media。\n\
出流：阻塞=整批再交下游；流式=每段完成即入声轨。",
    inputs: &[Port {
        name: "text",
        kind: DataKind::ScriptText,
        required: true,
    }],
    outputs: &[
        Port {
            name: "clips",
            kind: DataKind::SpeechClips,
            required: false,
        },
        Port {
            name: "media",
            kind: DataKind::MediaBin,
            required: false,
        },
    ],
    fields: FIELDS,
};
