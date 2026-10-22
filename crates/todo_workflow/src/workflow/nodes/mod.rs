//! Built-in workflow node catalog: ports, params, and payload validation.
//! Graph install checks types/params/edges; runners call [`validate_payload`]
//! on each node's inputs and outputs.

mod types;
mod validate;

mod trigger;
mod media_bin;
mod llm_chat;
mod image_gen;
mod multimodal;
mod script;
mod segment_script;
mod edit_text;
mod image_prompt;
mod tts;

pub use image_prompt::DEFAULT_SYSTEM as IMAGE_PROMPT_DEFAULT_SYSTEM;
mod subtitles;
mod emit;
mod emit_sequence;
mod emit_visuals;
mod split_shots;
mod label_shots;
mod choose_motions;
mod motion;
mod ground_emit;
mod picture_batch;
mod done;
mod python;
mod relay;

pub use types::{DataKind, NodeGroup, NodeSpec, ParamField, Port};
pub use validate::{resolve_edge_ports, validate_params, validate_payload};

/// Canonical builtins (aliases like `speech` → `tts` resolved via [`lookup`]).
pub const CATALOG: &[NodeSpec] = &[
    trigger::SPEC,
    media_bin::SPEC,
    relay::SPEC,
    llm_chat::SPEC,
    image_gen::SPEC,
    multimodal::SPEC,
    script::SPEC,
    segment_script::SPEC,
    edit_text::SPEC,
    image_prompt::SPEC,
    tts::SPEC,
    subtitles::SPEC,
    emit::SPEC,
    emit_sequence::SPEC,
    emit_visuals::SPEC,
    split_shots::SPEC,
    label_shots::SPEC,
    choose_motions::SPEC,
    motion::SPEC,
    ground_emit::SPEC,
    picture_batch::SPEC,
    done::SPEC,
];

/// Resolve a node `type` string to its spec (`speech` aliases `tts`).
pub fn lookup(node_type: &str) -> Option<&'static NodeSpec> {
    if node_type == "speech" {
        return CATALOG.iter().find(|s| s.type_name == "tts");
    }
    if node_type.starts_with("python:") {
        return Some(&python::SPEC);
    }
    CATALOG.iter().find(|s| s.type_name == node_type)
}

pub fn is_known_type(node_type: &str) -> bool {
    lookup(node_type).is_some()
}

pub fn builtin_type_names() -> Vec<&'static str> {
    let mut names: Vec<&str> = CATALOG.iter().map(|s| s.type_name).collect();
    names.push("speech"); // documented alias of tts
    names
}
