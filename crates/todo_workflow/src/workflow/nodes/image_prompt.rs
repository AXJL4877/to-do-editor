//! Builtin node: image_prompt — oral blocks → same-count visual prompts (one LLM call).
use super::types::{DataKind, NodeGroup, NodeSpec, ParamField, Port};

/// Default system prompt: clarify vague oral lines; lock cardinality; no franchise bias.
pub const DEFAULT_SYSTEM: &str = "\
你是口播分镜的生图提示词编剧（题材不限）。用户按时间顺序给出若干口播文本块（各带稳定 id）。\
请为每一块写出一条可直接送进文生图模型的中文画面描述。\n\n\
你的工作是把口播里说不清的地方补清楚，而不是另编故事：\n\
- 主语不明（他/她/它/这/那/有人/对方）→ 根据上文写回具体是谁或什么；\n\
- 动作不明（省略、含糊）→ 写成看得见的具体动作；\n\
- 情绪/状态不明 → 用表情、姿态或氛围写清楚，仍须能从上下文合理推断。\n\n\
硬性规则：\n\
1. 只输出一个 JSON 对象：{\"prompts\":[{\"id\":\"<与输入相同>\",\"prompt\":\"<画面描述>\"},…]}。\
prompts 条数必须与输入块数完全一致，id 一一对应，不得增删、合并或调换顺序。\n\
2. 全片人物与场景指代保持连续：前面出现过的主体，后面不要无故换成别人或别的东西。\n\
3. 每条 prompt 只写这一镜该看见的画面（主体、动作、情绪/状态、环境、构图感），\
不要口播原文、不要对白字幕、不要镜号说明、不要「如图」「根据上文」等元话语。\n\
4. 口播没依据的细节不要大幅发挥；不要绑定特定动画 IP 或固定人设，除非口播或用户补充里已经写明。\n\
5. 避免血腥色情。";

const FIELDS: &[ParamField] = &[
    ParamField::Text {
        id: "model",
        label: "模型",
        hint: "留空用 AI 设置里的聊天模型；填网关上的模型名可覆盖",
        default: "",
        placeholder: "deepseek-flash / claude-fable-5 …",
    },
    ParamField::Text {
        id: "system",
        label: "系统提示词",
        hint: "覆盖默认说明；留空即可——默认会补清主语/动作/情绪并锁死条数",
        default: "",
        placeholder: "留空 = 内置通用生图提示词编剧",
    },
    ParamField::Text {
        id: "cast",
        label: "补充设定",
        hint: "可选，一般留空。仅当口播推不出外形/身份时再写一句；平时靠上下文推断",
        default: "",
        placeholder: "可选，如：主角是穿蓝衬衫的上班族",
    },
    ParamField::Text {
        id: "style",
        label: "画风",
        hint: "可选。需要统一画风时再填；留空则不强制",
        default: "",
        placeholder: "可选，如：扁平插画，干净背景",
    },
    ParamField::Number {
        id: "temperature",
        label: "温度",
        hint: "越低越稳；补清指代与锁条数时建议 0.2～0.4",
        min: 0.0,
        max: 2.0,
        step: 0.1,
        default: 0.3,
        unit: None,
    },
];

pub const SPEC: NodeSpec = NodeSpec {
    type_name: "image_prompt",
    label: "生图提示词",
    group: NodeGroup::Ai,
    summary: "口播整篇写成同数量生图提示：补清主语、动作、情绪，锁死条数。",
    docs: "输入 text/segments（N 块口播）。一次调用聊天模型，输出仍是 N 块画面描述，\
id 对齐，可接 image_gen.prompt。默认从上下文补清不明主语/动作/情绪，不绑定特定 IP；\
条数必须等于 N。补充设定与画风均可选，一般留空。",
    inputs: &[Port {
        name: "text",
        kind: DataKind::ScriptText,
        required: true,
    }],
    outputs: &[Port {
        name: "prompt",
        kind: DataKind::Segments,
        required: false,
    }],
    fields: FIELDS,
};
