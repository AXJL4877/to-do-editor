//! Workflow packages: an extension that declares `graph.json` is a runnable
//! node graph. Discovery still goes through [`crate::registry`]; this module
//! loads and projects the graph for the host UI and future runners.
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use todo_production::extension::graph::{Graph, GraphEdge, GraphNode, GRAPH_SPEC};
use todo_production::extension::manifest::Problem;
use todo_production::extension::nodes::{self, DataKind, NodeSpec};

use crate::registry::Extension;

/// An installed package whose manifest points at a validated workflow graph.
#[derive(Debug, Clone)]
pub struct Workflow {
    pub extension_id: String,
    pub name: String,
    pub graph: Graph,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkflowNodeCard {
    pub id: String,
    pub node_type: String,
    pub label: String,
    pub summary: String,
    pub docs: String,
    pub inputs: Vec<WorkflowPortCard>,
    pub outputs: Vec<WorkflowPortCard>,
    pub x: f32,
    pub y: f32,
    /// Raw params from graph.json (for inspector).
    pub params: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkflowPortCard {
    pub name: String,
    pub kind: String,
    pub required: bool,
}

impl Extension {
    /// Load `manifest.graph` when present. Errors are install-time problems.
    pub fn workflow(&self) -> Result<Option<Workflow>, Vec<Problem>> {
        let Some(path) = &self.manifest.graph else {
            return Ok(None);
        };
        let text = self.read(path).ok_or_else(|| {
            vec![Problem {
                path: "graph".into(),
                message: format!("找不到 {path}"),
            }]
        })?;
        let graph = Graph::parse(&text, "graph").map_err(|p| vec![p])?;
        let ops: std::collections::BTreeSet<&str> = self
            .manifest
            .worker
            .iter()
            .flat_map(|w| w.ops.iter().map(String::as_str))
            .collect();
        let problems = graph.validate(&ops);
        if !problems.is_empty() {
            return Err(problems
                .into_iter()
                .map(|mut p| {
                    p.path = format!("graph.{}", p.path);
                    p
                })
                .collect());
        }
        Ok(Some(Workflow {
            extension_id: self.manifest.id.clone(),
            name: self.manifest.name.clone(),
            graph,
        }))
    }

    /// Write node params back into `graph.json` (creates the file key if needed).
    pub fn save_node_params(
        &self,
        node_id: &str,
        params: BTreeMap<String, Value>,
    ) -> Result<(), String> {
        let path = self
            .manifest
            .graph
            .as_ref()
            .ok_or_else(|| "扩展没有 graph".to_string())?;
        let graph_path = self.dir.join(path);
        let text = std::fs::read_to_string(&graph_path).map_err(|e| e.to_string())?;
        let mut graph: Graph = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let node = graph
            .nodes
            .iter_mut()
            .find(|n| n.id == node_id)
            .ok_or_else(|| format!("没有节点 {node_id}"))?;
        node.params = params;
        let ops: std::collections::BTreeSet<&str> = self
            .manifest
            .worker
            .iter()
            .flat_map(|w| w.ops.iter().map(String::as_str))
            .collect();
        let problems = graph.validate(&ops);
        if !problems.is_empty() {
            return Err(problems
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join("; "));
        }
        let out = serde_json::to_string_pretty(&graph).map_err(|e| e.to_string())?;
        let partial = graph_path.with_extension("json.part");
        std::fs::write(&partial, out.as_bytes()).map_err(|e| e.to_string())?;
        std::fs::rename(&partial, &graph_path).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Copy files into `assets/inbox/` and append them to a media_bin node's params.files.
    pub fn add_media_bin_files(
        &self,
        node_id: &str,
        sources: &[PathBuf],
    ) -> Result<usize, String> {
        if sources.is_empty() {
            return Ok(0);
        }
        let path = self
            .manifest
            .graph
            .as_ref()
            .ok_or_else(|| "扩展没有 graph".to_string())?;
        let graph_path = self.dir.join(path);
        let text = std::fs::read_to_string(&graph_path).map_err(|e| e.to_string())?;
        let mut graph: Graph = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let node = graph
            .nodes
            .iter_mut()
            .find(|n| n.id == node_id)
            .ok_or_else(|| format!("没有节点 {node_id}"))?;
        if node.node_type != "media_bin" {
            return Err("只能向素材容器添加文件".into());
        }
        let inbox = self.dir.join("assets/inbox");
        std::fs::create_dir_all(&inbox).map_err(|e| e.to_string())?;
        let mut files = node
            .params
            .get("files")
            .cloned()
            .unwrap_or_else(|| json!([]));
        let list = files
            .as_array_mut()
            .ok_or_else(|| "params.files 须为数组".to_string())?;
        let mut added = 0usize;
        for source in sources {
            if !source.is_file() {
                continue;
            }
            let kind = media_kind(source).ok_or_else(|| {
                format!("不支持的文件类型：{}", source.display())
            })?;
            let safe = safe_file_name(source);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dest_name = format!("{stamp}-{safe}");
            let dest = inbox.join(&dest_name);
            let partial = dest.with_extension("part");
            std::fs::copy(source, &partial).map_err(|e| e.to_string())?;
            std::fs::rename(&partial, &dest).map_err(|e| e.to_string())?;
            let rel = format!("assets/inbox/{dest_name}");
            list.push(json!({
                "path": rel,
                "kind": kind,
                "name": source.file_name().and_then(|n| n.to_str()).unwrap_or(&safe),
            }));
            added += 1;
        }
        node.params.insert("files".into(), files);
        let ops: std::collections::BTreeSet<&str> = self
            .manifest
            .worker
            .iter()
            .flat_map(|w| w.ops.iter().map(String::as_str))
            .collect();
        let problems = graph.validate(&ops);
        if !problems.is_empty() {
            return Err(problems
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join("; "));
        }
        let out = serde_json::to_string_pretty(&graph).map_err(|e| e.to_string())?;
        let partial = graph_path.with_extension("json.part");
        std::fs::write(&partial, out.as_bytes()).map_err(|e| e.to_string())?;
        std::fs::rename(&partial, &graph_path).map_err(|e| e.to_string())?;
        Ok(added)
    }

    /// Remove one file entry from a media_bin (does not delete the file on disk).
    pub fn remove_media_bin_file(&self, node_id: &str, rel_path: &str) -> Result<(), String> {
        let path = self
            .manifest
            .graph
            .as_ref()
            .ok_or_else(|| "扩展没有 graph".to_string())?;
        let graph_path = self.dir.join(path);
        let text = std::fs::read_to_string(&graph_path).map_err(|e| e.to_string())?;
        let mut graph: Graph = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let node = graph
            .nodes
            .iter_mut()
            .find(|n| n.id == node_id)
            .ok_or_else(|| format!("没有节点 {node_id}"))?;
        let files = node
            .params
            .get_mut("files")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "没有 files 列表".to_string())?;
        files.retain(|f| f.get("path").and_then(Value::as_str) != Some(rel_path));
        let out = serde_json::to_string_pretty(&graph).map_err(|e| e.to_string())?;
        let partial = graph_path.with_extension("json.part");
        std::fs::write(&partial, out.as_bytes()).map_err(|e| e.to_string())?;
        std::fs::rename(&partial, &graph_path).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Write the full graph (after caller mutated it) back to disk.
    pub fn save_graph(&self, graph: &Graph) -> Result<(), String> {
        let path = self
            .manifest
            .graph
            .as_ref()
            .ok_or_else(|| "扩展没有 graph".to_string())?;
        let graph_path = self.dir.join(path);
        let ops: std::collections::BTreeSet<&str> = self
            .manifest
            .worker
            .iter()
            .flat_map(|w| w.ops.iter().map(String::as_str))
            .collect();
        let problems = graph.validate(&ops);
        if !problems.is_empty() {
            return Err(problems
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join("; "));
        }
        let out = serde_json::to_string_pretty(graph).map_err(|e| e.to_string())?;
        let partial = graph_path.with_extension("json.part");
        std::fs::write(&partial, out.as_bytes()).map_err(|e| e.to_string())?;
        std::fs::rename(&partial, &graph_path).map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Scaffold a blank user workflow package under `user_dir/<id>/`.
/// Returns the new extension id.
pub fn create_blank_workflow(user_dir: &Path, name: &str) -> Result<String, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let id = format!("studio.workflow-{stamp}");
    let dir = user_dir.join(&id);
    if dir.exists() {
        return Err("工作流目录已存在，请重试".into());
    }
    std::fs::create_dir_all(dir.join("worker")).map_err(|e| e.to_string())?;
    let manifest = json!({
        "spec": "todo-extension/1",
        "id": id,
        "name": name,
        "version": "0.1.0",
        "description": "用户工作流",
        "kind": "workflow",
        "input": {},
        "params": [],
        "requires": {
            "runtime": { "type": "python", "version": ">=3.12" }
        },
        "worker": {
            "entry": "worker/main.py",
            "ops": ["run"],
            "timeoutSec": 600,
            "maxProcesses": 1,
            "memoryMiB": 512
        },
        "graph": "graph.json",
        "budget": {}
    });
    let graph = json!({
        "spec": GRAPH_SPEC,
        "nodes": [{
            "id": "media_bin",
            "type": "media_bin",
            "label": "素材容器",
            "x": 80,
            "y": 160,
            "params": {
                "accept": ".txt,.md,.mp3,.wav,.m4a,.mp4,.mov,.png,.jpg,.webp",
                "note": ""
            }
        }],
        "edges": []
    });
    let worker = r#"from todo_worker import Worker

worker = Worker(version="0.1.0")

@worker.op("run")
def run(ctx):
    ctx.progress("done", 1.0)
    return {"ok": True}

if __name__ == "__main__":
    worker.serve()
"#;
    let write = |rel: &str, text: &str| -> Result<(), String> {
        let path = dir.join(rel);
        let partial = {
            let mut p = path.as_os_str().to_owned();
            p.push(".part");
            PathBuf::from(p)
        };
        std::fs::write(&partial, text.as_bytes()).map_err(|e| e.to_string())?;
        std::fs::rename(&partial, &path).map_err(|e| e.to_string())?;
        Ok(())
    };
    write(
        "extension.json",
        &serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
    )?;
    write(
        "graph.json",
        &serde_json::to_string_pretty(&graph).map_err(|e| e.to_string())?,
    )?;
    write("worker/main.py", worker)?;
    // Validate before leaving a half package around.
    crate::registry::load(&dir, crate::registry::Origin::User).map_err(|ps| {
        let _ = std::fs::remove_dir_all(&dir);
        ps.iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    Ok(id)
}

/// Recursively copy a package directory (for builtin → user override).
pub fn copy_package(src: &Path, dst: &Path) -> Result<(), String> {
    if dst.exists() {
        return Ok(());
    }
    fn walk(from: &Path, to: &Path) -> Result<(), String> {
        std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(from).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') || name_str == "__pycache__" {
                continue;
            }
            let src_path = entry.path();
            let dst_path = to.join(&name);
            if src_path.is_dir() {
                walk(&src_path, &dst_path)?;
            } else {
                if let Some(parent) = dst_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::copy(&src_path, &dst_path).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    walk(src, dst)
}

fn media_kind(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "txt" | "md" => Some("text"),
        "mp3" | "wav" | "m4a" | "ogg" | "flac" | "aac" => Some("audio"),
        "mp4" | "mov" | "webm" | "mkv" | "avi" => Some("video"),
        "png" | "jpg" | "jpeg" | "webp" | "bmp" => Some("image"),
        _ => None,
    }
}

fn safe_file_name(path: &Path) -> String {
    let raw = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let safe: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ".-_".contains(ch) {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() {
        "file".into()
    } else {
        safe
    }
}

impl Workflow {
    /// UI cards in topological order (falls back to declaration order).
    /// Missing `x`/`y` get a layered auto-layout.
    pub fn node_cards(&self) -> Vec<WorkflowNodeCard> {
        let order = self
            .graph
            .topo_order()
            .unwrap_or_else(|| self.graph.nodes.iter().map(|n| n.id.as_str()).collect());
        let ranks = layered_ranks(&self.graph);
        order
            .into_iter()
            .enumerate()
            .filter_map(|(i, id)| {
                let node = self.graph.nodes.iter().find(|n| n.id == id)?;
                let (rank, slot) = ranks.get(id).copied().unwrap_or((i, 0));
                Some(node_card(node, rank, slot))
            })
            .collect()
    }

    pub fn edges(&self) -> &[GraphEdge] {
        &self.graph.edges
    }

    pub fn nodes(&self) -> &[GraphNode] {
        &self.graph.nodes
    }
}

const CARD_W: f32 = 200.0;
const CARD_H: f32 = 120.0;
const GAP_X: f32 = 64.0;
const GAP_Y: f32 = 48.0;

fn node_card(node: &GraphNode, rank: usize, slot: usize) -> WorkflowNodeCard {
    let spec = nodes::lookup(&node.node_type);
    let label = node
        .label
        .clone()
        .or_else(|| spec.map(|s| s.label.to_owned()))
        .unwrap_or_else(|| node.node_type.clone());
    let summary = spec.map(|s| s.summary.to_owned()).unwrap_or_default();
    let docs = spec.map(|s| s.docs.to_owned()).unwrap_or_default();
    let (inputs, outputs) = match spec {
        Some(s) => (ports(s.inputs), ports(s.outputs)),
        None => (Vec::new(), Vec::new()),
    };
    let x = node
        .x
        .map(|v| v as f32)
        .unwrap_or(40.0 + rank as f32 * (CARD_W + GAP_X));
    let y = node
        .y
        .map(|v| v as f32)
        .unwrap_or(40.0 + slot as f32 * (CARD_H + GAP_Y));
    let params: serde_json::Map<String, serde_json::Value> = node
        .params
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    WorkflowNodeCard {
        id: node.id.clone(),
        node_type: node.node_type.clone(),
        label,
        summary,
        docs,
        inputs,
        outputs,
        x,
        y,
        params,
    }
}

/// Longest-path rank from sources; slots pack nodes that share a rank.
fn layered_ranks(graph: &Graph) -> std::collections::BTreeMap<String, (usize, usize)> {
    use std::collections::BTreeMap;
    let order = graph
        .topo_order()
        .unwrap_or_else(|| graph.nodes.iter().map(|n| n.id.as_str()).collect());
    let mut rank: BTreeMap<String, usize> = BTreeMap::new();
    for id in &order {
        rank.insert((*id).to_owned(), 0);
    }
    for edge in &graph.edges {
        let from = rank.get(&edge.from).copied().unwrap_or(0);
        let entry = rank.entry(edge.to.clone()).or_insert(0);
        *entry = (*entry).max(from + 1);
    }
    let mut by_rank: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for id in order {
        let r = rank.get(id).copied().unwrap_or(0);
        by_rank.entry(r).or_default().push(id.to_owned());
    }
    let mut out = BTreeMap::new();
    for (r, ids) in by_rank {
        for (slot, id) in ids.into_iter().enumerate() {
            out.insert(id, (r, slot));
        }
    }
    out
}

fn ports(list: &[todo_production::extension::nodes::Port]) -> Vec<WorkflowPortCard> {
    list.iter()
        .map(|p| WorkflowPortCard {
            name: p.name.to_owned(),
            kind: p.kind.as_str().to_owned(),
            required: p.required,
        })
        .collect()
}

/// Kind name helper for tests / UI.
pub fn data_kind_label(kind: DataKind) -> &'static str {
    kind.as_str()
}

pub fn catalog() -> &'static [NodeSpec] {
    nodes::CATALOG
}

pub fn graph_spec() -> &'static str {
    GRAPH_SPEC
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{load, Origin};
    use std::path::PathBuf;

    fn spongebob() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../extensions/builtin/spongebob-koubo")
    }

    #[test]
    fn spongebob_loads_as_a_workflow() {
        let ext = load(&spongebob(), Origin::Builtin).expect("package");
        let wf = ext.workflow().expect("graph ok").expect("has graph");
        assert_eq!(wf.graph.spec, GRAPH_SPEC);
        let cards = wf.node_cards();
        assert!(cards.iter().any(|c| c.node_type == "tts" || c.id == "speech"));
        assert!(cards.iter().any(|c| c.id == "script"));
        assert_eq!(cards.last().map(|c| c.id.as_str()), Some("done"));
    }

    #[test]
    fn blank_workflow_scaffolds_under_user_dir() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp")
            .join(format!(
                "todo-wf-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        std::fs::create_dir_all(&root).expect("tmpdir");
        let id = create_blank_workflow(&root, "测试流").expect("create");
        let ext = load(&root.join(&id), Origin::User).expect("load");
        assert_eq!(ext.manifest.name, "测试流");
        let wf = ext.workflow().expect("ok").expect("graph");
        assert_eq!(wf.graph.nodes.len(), 1);
        assert_eq!(wf.graph.nodes[0].node_type, "media_bin");
        let _ = std::fs::remove_dir_all(&root);
    }
}
