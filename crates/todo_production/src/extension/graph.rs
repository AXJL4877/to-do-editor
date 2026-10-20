//! Workflow graph IR (`todo-graph/1`): nodes + edges declared as data.
//! Validation only; the host runner comes later. See extension-spec § graph.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::manifest::Problem;
use super::nodes::{self, builtin_type_names};

pub const GRAPH_SPEC: &str = "todo-graph/1";

const MAX_NODES: usize = 256;
const MAX_EDGES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Graph {
    pub spec: String,
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphNode {
    pub id: String,
    /// Built-in name (`tts`, `segment_script`, …) or `python:<op>`.
    #[serde(rename = "type")]
    pub node_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
    /// Optional free-canvas position (graph space, top-left).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    /// Optional named output port on `from`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
    /// Optional named input port on `to`.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "in")]
    pub input: Option<String>,
}

impl Graph {
    /// Parse package text; errors become a single problem at `path`.
    pub fn parse(text: &str, path: &str) -> Result<Self, Problem> {
        serde_json::from_str(text).map_err(|e| Problem {
            path: path.into(),
            message: format!("无法解析：{e}"),
        })
    }

    /// Full install/run checks (types, edges, cycles, required inputs wired).
    pub fn validate(&self, worker_ops: &BTreeSet<&str>) -> Vec<Problem> {
        self.validate_inner(worker_ops, true)
    }

    /// Canvas edit / save: same as [`validate`] but allows unwired required inputs
    /// so users can drop incomplete nodes (e.g. llm_chat) then connect later.
    pub fn validate_draft(&self, worker_ops: &BTreeSet<&str>) -> Vec<Problem> {
        self.validate_inner(worker_ops, false)
    }

    fn validate_inner(&self, worker_ops: &BTreeSet<&str>, require_inputs: bool) -> Vec<Problem> {
        let mut problems = Vec::new();
        let mut problem = |path: &str, message: String| {
            problems.push(Problem {
                path: path.into(),
                message,
            })
        };

        if self.spec != GRAPH_SPEC {
            problem("spec", format!("须为 {GRAPH_SPEC}"));
        }
        if self.nodes.is_empty() {
            problem("nodes", "至少需要一个节点".into());
        }
        if self.nodes.len() > MAX_NODES {
            problem("nodes", format!("最多 {MAX_NODES} 个节点"));
        }
        if self.edges.len() > MAX_EDGES {
            problem("edges", format!("最多 {MAX_EDGES} 条边"));
        }

        let mut ids = BTreeSet::new();
        let mut id_to_type: BTreeMap<String, String> = BTreeMap::new();
        for (i, node) in self.nodes.iter().enumerate() {
            let at = format!("nodes[{i}]");
            if !valid_node_id(&node.id) {
                problem(
                    &format!("{at}.id"),
                    "须为小写字母开头，字母数字_与-，最长 64".into(),
                );
            } else if !ids.insert(node.id.as_str()) {
                problem(&format!("{at}.id"), format!("重复的节点 id {}", node.id));
            } else {
                id_to_type.insert(node.id.clone(), node.node_type.clone());
            }
            if let Some(label) = &node.label {
                if label.is_empty() || label.chars().count() > 80 {
                    problem(&format!("{at}.label"), "1~80 字".into());
                }
            }
            if let Some(op) = node.node_type.strip_prefix("python:") {
                if !valid_op_name(op) {
                    problem(
                        &format!("{at}.type"),
                        "python: 后须为小写字母开头的操作名".into(),
                    );
                } else if !worker_ops.contains(op) {
                    problem(
                        &format!("{at}.type"),
                        format!("工作程序没有操作 {op}"),
                    );
                }
            } else if !nodes::is_known_type(&node.node_type) {
                problem(
                    &format!("{at}.type"),
                    format!(
                        "未知类型 {:?}（内置：{}；或 python:<op>）",
                        node.node_type,
                        builtin_type_names().join(", ")
                    ),
                );
            }
            if node.params.len() > 64 {
                problem(&format!("{at}.params"), "最多 64 个参数".into());
            } else {
                for p in nodes::validate_params(&node.node_type, &node.params) {
                    problem(&format!("{at}.{}", p.path), p.message);
                }
            }
        }

        let id_set: BTreeSet<&str> = ids;
        // Incoming wires per (node, input port) for required-port check.
        let mut wired: BTreeSet<(String, String)> = BTreeSet::new();

        for (i, edge) in self.edges.iter().enumerate() {
            let at = format!("edges[{i}]");
            if !id_set.contains(edge.from.as_str()) {
                problem(&format!("{at}.from"), format!("没有节点 {}", edge.from));
                continue;
            }
            if !id_set.contains(edge.to.as_str()) {
                problem(&format!("{at}.to"), format!("没有节点 {}", edge.to));
                continue;
            }
            if edge.from == edge.to {
                problem(&at, "禁止自环".into());
                continue;
            }
            for (field, port) in [("out", &edge.out), ("in", &edge.input)] {
                if let Some(name) = port {
                    if !valid_port_name(name) {
                        problem(
                            &format!("{at}.{field}"),
                            "端口名：小写字母开头，字母数字_，最长 32".into(),
                        );
                    }
                }
            }
            let from_ty = id_to_type.get(&edge.from).map(String::as_str).unwrap_or("");
            let to_ty = id_to_type.get(&edge.to).map(String::as_str).unwrap_or("");
            match nodes::resolve_edge_ports(
                from_ty,
                to_ty,
                edge.out.as_deref(),
                edge.input.as_deref(),
            ) {
                Ok((_out_port, in_port)) => {
                    wired.insert((edge.to.clone(), in_port.name.to_owned()));
                }
                Err(message) => problem(&at, message),
            }
        }

        if require_inputs {
            for (i, node) in self.nodes.iter().enumerate() {
                let Some(spec) = nodes::lookup(&node.node_type) else {
                    continue;
                };
                for port in spec.inputs.iter().filter(|p| p.required) {
                    if !wired.contains(&(node.id.clone(), port.name.to_owned())) {
                        problem(
                            &format!("nodes[{i}]"),
                            format!("必需输入端口 {} 未连接", port.name),
                        );
                    }
                }
            }
        }

        if problems.is_empty() {
            if let Some(cycle_hint) = find_cycle(&self.nodes, &self.edges) {
                problems.push(Problem {
                    path: "edges".into(),
                    message: format!("图中有环（涉及 {cycle_hint}）"),
                });
            }
        }

        problems
    }

    /// Kahn topological order; `None` if cyclic.
    pub fn topo_order(&self) -> Option<Vec<&str>> {
        let n = self.nodes.len();
        if n == 0 {
            return Some(Vec::new());
        }
        let index: BTreeMap<&str, usize> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (node.id.as_str(), i))
            .collect();
        let mut indegree = vec![0usize; n];
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for edge in &self.edges {
            let Some(&u) = index.get(edge.from.as_str()) else {
                return None;
            };
            let Some(&v) = index.get(edge.to.as_str()) else {
                return None;
            };
            if u == v {
                return None;
            }
            adj[u].push(v);
            indegree[v] += 1;
        }
        let mut queue: VecDeque<usize> = indegree
            .iter()
            .enumerate()
            .filter_map(|(i, d)| (*d == 0).then_some(i))
            .collect();
        let mut order = Vec::with_capacity(n);
        while let Some(u) = queue.pop_front() {
            order.push(self.nodes[u].id.as_str());
            for &v in &adj[u] {
                indegree[v] -= 1;
                if indegree[v] == 0 {
                    queue.push_back(v);
                }
            }
        }
        (order.len() == n).then_some(order)
    }
}

fn valid_node_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && id.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn valid_op_name(op: &str) -> bool {
    let mut chars = op.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && op.len() <= 32
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn valid_port_name(name: &str) -> bool {
    valid_op_name(name)
}

fn find_cycle(nodes: &[GraphNode], edges: &[GraphEdge]) -> Option<String> {
    let graph = Graph {
        spec: GRAPH_SPEC.into(),
        nodes: nodes.to_vec(),
        edges: edges.to_vec(),
    };
    if graph.topo_order().is_some() {
        return None;
    }
    edges
        .first()
        .map(|e| format!("{}→{}", e.from, e.to))
        .or_else(|| nodes.first().map(|n| n.id.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Graph {
        Graph {
            spec: GRAPH_SPEC.into(),
            nodes: vec![
                GraphNode {
                    id: "script".into(),
                    node_type: "script".into(),
                    label: Some("口播".into()),
                    params: BTreeMap::new(),
                x: None,
                y: None,
                },
                GraphNode {
                    id: "tts".into(),
                    node_type: "tts".into(),
                    label: None,
                    params: BTreeMap::new(),
                x: None,
                y: None,
                },
                GraphNode {
                    id: "done".into(),
                    node_type: "done".into(),
                    label: None,
                    params: BTreeMap::new(),
                x: None,
                y: None,
                },
            ],
            edges: vec![
                GraphEdge {
                    from: "script".into(),
                    to: "tts".into(),
                    out: Some("script".into()),
                    input: Some("text".into()),
                },
                GraphEdge {
                    from: "tts".into(),
                    to: "done".into(),
                    out: Some("audio".into()),
                    input: Some("result".into()),
                },
            ],
        }
    }

    #[test]
    fn a_linear_graph_is_valid_and_ordered() {
        let g = sample();
        assert!(g.validate(&BTreeSet::new()).is_empty(), "{:?}", g.validate(&BTreeSet::new()));
        assert_eq!(g.topo_order(), Some(vec!["script", "tts", "done"]));
    }

    #[test]
    fn segment_script_requires_text_wire() {
        let g = Graph {
            spec: GRAPH_SPEC.into(),
            nodes: vec![
                GraphNode {
                    id: "seg".into(),
                    node_type: "segment_script".into(),
                    label: None,
                    params: BTreeMap::from([("mode".into(), json!("semantic"))]),
                x: None,
                y: None,
                },
                GraphNode {
                    id: "done".into(),
                    node_type: "done".into(),
                    label: None,
                    params: BTreeMap::new(),
                x: None,
                y: None,
                },
            ],
            edges: vec![],
        };
        let found = g.validate(&BTreeSet::new());
        assert!(
            found.iter().any(|p| p.message.contains("text") && p.message.contains("未连接")),
            "{found:?}"
        );
        assert!(
            g.validate_draft(&BTreeSet::new()).is_empty(),
            "draft save must allow unwired required ports"
        );
    }

    #[test]
    fn cycles_and_unknown_types_are_rejected() {
        let mut g = sample();
        g.edges.push(GraphEdge {
            from: "done".into(),
            to: "script".into(),
            out: None,
            input: None,
        });
        let found = g.validate(&BTreeSet::new());
        assert!(
            found.iter().any(|p| p.message.contains("环") || p.message.contains("输出")),
            "{found:?}"
        );

        let mut bad = sample();
        bad.nodes[0].node_type = "magic".into();
        let found = bad.validate(&BTreeSet::new());
        assert!(
            found.iter().any(|p| p.path.contains("type")),
            "{found:?}"
        );
    }

    #[test]
    fn python_nodes_must_match_worker_ops() {
        let g = Graph {
            spec: GRAPH_SPEC.into(),
            nodes: vec![GraphNode {
                id: "custom".into(),
                node_type: "python:bake_board".into(),
                label: None,
                params: BTreeMap::from([("board".into(), json!(1))]),
            x: None,
            y: None,
            }],
            edges: vec![],
        };
        assert!(g
            .validate(&BTreeSet::from(["run"]))
            .iter()
            .any(|p| p.message.contains("bake_board")));
        assert!(g
            .validate(&BTreeSet::from(["bake_board"]))
            .is_empty());
    }
}
