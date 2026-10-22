//! Builtin node: media_bin.
use super::types::{DataKind, NodeGroup, NodeSpec, ParamField, Port};

const FIELDS: &[ParamField] = &[
    ParamField::FileKinds {
        id: "accept",
        label: "可放入的文件",
        hint: "容器可挂口播文案、配音参考、成片素材等，供下游读取",
        accept: ".txt,.md,.mp3,.wav,.m4a,.mp4,.mov,.png,.jpg,.webp",
    },
    ParamField::Text {
        id: "note",
        label: "备注",
        hint: "仅说明，不影响运行",
        default: "",
        placeholder: "例如：本集口播稿 + BGM",
    },
];

pub const SPEC: NodeSpec = NodeSpec {
        type_name: "media_bin",
        label: "素材容器",
        group: NodeGroup::Basic,
        summary: "挂载 txt / mp3 / mp4 / 图片等输入文件。",
        docs: "可放入口播文案（txt/md）、音频（mp3/wav）、视频（mp4）与参考图。files=素材清单；text=若含文本文件则抽出文案；image / images 供接到生图 ref、看图等。可选接 trigger。画布可拖入文件创建。",
        inputs: &[Port {
            name: "start",
            kind: DataKind::Trigger,
            required: false,
        }],
        outputs: &[
            Port {
                name: "files",
                kind: DataKind::MediaBin,
                required: false,
            },
            Port {
                name: "text",
                kind: DataKind::ScriptText,
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
        ],
        fields: FIELDS,
    };
