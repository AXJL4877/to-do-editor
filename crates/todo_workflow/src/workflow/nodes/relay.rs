//! Builtin node: relay (中转).
//! Mid-store for files and text blocks (e.g. from 口播分段).

use super::types::{DataKind, NodeGroup, NodeSpec, ParamField, Port};

const FIELDS: &[ParamField] = &[
    ParamField::FileKinds {
        id: "accept",
        label: "可放入的文件",
        hint: "几乎任意类型；按扩展名自动归类（未知则为 other）",
        accept: ".json,.txt,.md,.csv,.xml,.html,.srt,.ass,.mp3,.wav,.m4a,.ogg,.flac,.aac,.mp4,.mov,.webm,.mkv,.avi,.png,.jpg,.jpeg,.webp,.bmp,.gif,.pdf,.zip,*",
    },
    ParamField::Text {
        id: "note",
        label: "备注",
        hint: "仅说明，不影响运行",
        default: "",
        placeholder: "例如：本镜成片 + 旁白分段",
    },
];

pub const SPEC: NodeSpec = NodeSpec {
    type_name: "relay",
    label: "中转",
    group: NodeGroup::Basic,
    summary: "中间仓：图片 / 文本 / 视频汇合预览后再分发。",
    docs: "in 接任意上游（分段、图、视频、素材仓）。检查器可预览文本块、图片与视频首帧。files / image 旁路；text 拼合全文；images 多图。",
    inputs: &[
        Port {
            name: "in",
            kind: DataKind::Any,
            required: false,
        },
        Port {
            name: "files",
            kind: DataKind::MediaBin,
            required: false,
        },
        Port {
            name: "image",
            kind: DataKind::ImageAsset,
            required: false,
        },
    ],
    outputs: &[
        Port {
            name: "segments",
            kind: DataKind::Segments,
            required: false,
        },
        Port {
            name: "files",
            kind: DataKind::MediaBin,
            required: false,
        },
        Port {
            name: "image",
            kind: DataKind::ImageAsset,
            required: false,
        },
        Port {
            name: "images",
            kind: DataKind::ImageAssets,
            required: false,
        },
        Port {
            name: "text",
            kind: DataKind::ScriptText,
            required: false,
        },
        Port {
            name: "json",
            kind: DataKind::Any,
            required: false,
        },
    ],
    fields: FIELDS,
};
