//! Presentation model for the AI production panel. It contains no registry,
//! filesystem or service handles; adapters supply extension cards, and the
//! production tree comes from the application's status.
use std::collections::BTreeMap;
use todo_application::{ProductionStatus, StatusNode, StatusState};
use todo_production::Production;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionStatus {
    MissingKey,
    RuntimeMissing,
    SelfCheckFailed,
    Checking,
    Ready,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    Text(String),
    Number(f64),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceView {
    pub value: String,
    pub label: String,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamInput {
    Select(Vec<ChoiceView>),
    Number {
        min: f64,
        max: f64,
        step: f64,
        unit: Option<String>,
    },
    Text {
        max_length: usize,
        placeholder: Option<String>,
    },
    Bool,
    /// A voice id; `choices` are suggestions, any id (e.g. `S_…`) is allowed.
    Voice(Vec<ChoiceView>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamView {
    pub id: String,
    pub label: String,
    pub hint: Option<String>,
    pub group: String,
    /// Shown without expanding "more".
    pub quick: bool,
    pub when: BTreeMap<String, Vec<String>>,
    pub input: ParamInput,
    pub default: ParamValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScriptView {
    pub label: String,
    pub max_length: usize,
    pub placeholder: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionCardView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: ExtensionStatus,
    pub problem: Option<String>,
    pub preview: Option<String>,
    pub params: Vec<ParamView>,
    pub script: Option<ScriptView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductionNodeStatus {
    Queued,
    Running,
    Waiting,
    Attention,
    Complete,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProductionNodeView {
    pub key: String,
    pub label: String,
    pub status: ProductionNodeStatus,
    pub detail: Option<String>,
    pub children: Vec<ProductionNodeView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FailureView {
    pub key: String,
    pub message: String,
    pub possibly_billed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProductionView {
    pub id: String,
    pub title: String,
    pub extension: String,
    pub completed: usize,
    pub total: usize,
    pub nodes: Vec<ProductionNodeView>,
    pub attention: Vec<String>,
    pub failures: Vec<FailureView>,
    pub segments: Vec<SegmentView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentView {
    pub id: String,
    pub text: String,
    pub elements: Vec<ElementView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ElementView {
    pub id: String,
    pub label: String,
    pub draw: String,
    pub picked: Option<String>,
    pub variants: Vec<VariantView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariantView {
    pub id: String,
    pub file: Option<String>,
    pub locked: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProductionPanelView {
    pub extensions: Vec<ExtensionCardView>,
    pub productions: Vec<ProductionView>,
    /// Productions need an open project.
    pub project_open: bool,
}

/// Manifest `when` rules: a field is visible only if every dependency has one
/// of the listed values. Unknown values simply hide the dependent field.
pub fn when_visible(
    when: &BTreeMap<String, Vec<String>>,
    values: &BTreeMap<String, String>,
) -> bool {
    when.iter().all(|(key, allowed)| {
        values
            .get(key)
            .is_some_and(|value| allowed.iter().any(|item| item == value))
    })
}

fn node(node: &StatusNode) -> ProductionNodeView {
    ProductionNodeView {
        key: node.key.clone(),
        label: node.label.clone(),
        status: match node.state {
            StatusState::Queued => ProductionNodeStatus::Queued,
            StatusState::Running => ProductionNodeStatus::Running,
            StatusState::Waiting => ProductionNodeStatus::Waiting,
            StatusState::Attention => ProductionNodeStatus::Attention,
            StatusState::Done => ProductionNodeStatus::Complete,
            StatusState::Failed => ProductionNodeStatus::Failed,
        },
        detail: node.detail.clone(),
        children: node.children.iter().map(self::node).collect(),
    }
}

pub fn production_view(status: &ProductionStatus, extension_name: Option<&str>) -> ProductionView {
    production_view_with_document(status, extension_name, None)
}

pub fn production_view_with_document(
    status: &ProductionStatus,
    extension_name: Option<&str>,
    document: Option<&Production>,
) -> ProductionView {
    ProductionView {
        id: status.id.clone(),
        title: status.title.clone(),
        extension: extension_name.unwrap_or(&status.extension).to_owned(),
        completed: status.done,
        total: status.total,
        nodes: status.nodes.iter().map(node).collect(),
        attention: status.attention.clone(),
        failures: status
            .failures
            .iter()
            .map(|(key, message, billed)| FailureView {
                key: key.0.clone(),
                message: message.clone(),
                possibly_billed: *billed,
            })
            .collect(),
        segments: document
            .map(|document| {
                document
                    .segments
                    .iter()
                    .map(|segment| SegmentView {
                        id: segment.id.clone(),
                        text: segment.text.clone(),
                        elements: segment
                            .boards
                            .iter()
                            .flat_map(|board| board.elements.iter())
                            .map(|element| ElementView {
                                id: element.id.clone(),
                                label: element.label.clone(),
                                draw: element.draw.clone(),
                                picked: element.picked().map(|variant| variant.id.clone()),
                                variants: element
                                    .variants
                                    .iter()
                                    .map(|variant| VariantView {
                                        id: variant.id.clone(),
                                        file: variant.file.clone(),
                                        locked: variant.locked,
                                    })
                                    .collect(),
                            })
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn when_rules_are_all_of() {
        let when = BTreeMap::from([
            ("enter".into(), vec!["mix".into()]),
            ("hand".into(), vec!["chalk".into(), "none".into()]),
        ]);
        let values = BTreeMap::from([
            ("enter".into(), "mix".into()),
            ("hand".into(), "chalk".into()),
        ]);
        assert!(when_visible(&when, &values));
        assert!(!when_visible(
            &when,
            &BTreeMap::from([("enter".into(), "fade".into())])
        ));
    }

    #[test]
    fn status_maps_to_the_panel_tree() {
        let status = ProductionStatus {
            id: "p".into(),
            title: "复利".into(),
            extension: "builtin.chalk-hand".into(),
            done: 1,
            total: 2,
            nodes: vec![StatusNode {
                key: "g1".into(),
                label: "第 1 段".into(),
                state: StatusState::Waiting,
                detail: Some("等待网络".into()),
                children: vec![],
            }],
            attention: vec![],
            failures: vec![(todo_application::TaskKey("k".into()), "超时".into(), true)],
        };
        let view = production_view(&status, Some("黑板手绘"));
        assert_eq!(view.extension, "黑板手绘");
        assert_eq!(view.nodes[0].status, ProductionNodeStatus::Waiting);
        assert!(view.failures[0].possibly_billed);
    }

    #[test]
    fn document_projection_exposes_segments_elements_and_variants() {
        let mut document = Production::new(
            "p",
            todo_production::PackRef {
                id: "builtin.chalk-hand".into(),
                version: "1".into(),
            },
            "hello",
            Default::default(),
            todo_production::SplitMode::Paragraph,
            todo_production::segment::Limits {
                target: 120,
                max: 200,
            },
        );
        let mut segment = document.new_segment("hello".into(), String::new());
        let mut board = todo_production::production::Board {
            id: "b1".into(),
            start: 0,
            elements: Vec::new(),
            render: None,
        };
        board.elements.push(document.new_element(
            0,
            "主角",
            "一只海绵宝宝",
            todo_production::storyboard::Role::Hero,
            None,
            2,
        ));
        segment.boards.push(board);
        document.segments.push(segment);
        let status = ProductionStatus {
            id: "p".into(),
            title: "测试".into(),
            extension: "builtin.chalk-hand".into(),
            done: 0,
            total: 1,
            nodes: Vec::new(),
            attention: Vec::new(),
            failures: Vec::new(),
        };
        let view = production_view_with_document(&status, None, Some(&document));
        assert_eq!(view.segments.len(), 1);
        assert_eq!(view.segments[0].elements[0].variants.len(), 2);
        assert_eq!(view.segments[0].elements[0].label, "主角");
    }
}
