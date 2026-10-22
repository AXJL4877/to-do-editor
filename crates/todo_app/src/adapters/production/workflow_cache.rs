//! Parsed graphs are shared across UI frames. Graph edits remain visible through
//! the file's modification stamp, including edits made outside the application.
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use todo_workflow::{Extension, Workflow};

#[derive(Default)]
pub(super) struct WorkflowCache {
    entries: RefCell<HashMap<PathBuf, CachedGraph>>,
}

struct CachedGraph {
    stamp: (SystemTime, u64),
    workflow: Option<Arc<Workflow>>,
}

impl WorkflowCache {
    pub(super) fn get(&self, extension: &Extension) -> Option<Arc<Workflow>> {
        let path = extension.manifest.graph.as_ref()?.clone();
        let graph_path = extension.dir.join(path);
        let metadata = std::fs::metadata(&graph_path).ok()?;
        let stamp = (metadata.modified().ok()?, metadata.len());
        if let Some(entry) = self.entries.borrow().get(&graph_path) {
            if entry.stamp == stamp {
                return entry.workflow.clone();
            }
        }
        let workflow = extension.workflow().ok().flatten().map(Arc::new);
        self.entries.borrow_mut().insert(
            graph_path,
            CachedGraph { stamp, workflow: workflow.clone() },
        );
        workflow
    }

    pub(super) fn clear(&self) {
        self.entries.borrow_mut().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_large_relay_graph_is_reused_and_edits_invalidate_it() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target").join(format!("todo-workflow-cache-{}-{:?}",
            std::process::id(), SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = todo_workflow::create_blank_workflow(&dir, "cache test").unwrap();
        let extension = todo_workflow::registry::load(&dir.join(id), todo_workflow::Origin::User).unwrap();
        let mut graph = extension.workflow().unwrap().unwrap().graph;
        graph.nodes = (0..200).map(|i| {
            let mut node = graph.nodes[0].clone();
            node.id = format!("relay_{i}");
            node.node_type = "relay".into();
            node.params.clear();
            node.params.insert("blocks".into(), serde_json::json!([
                {"id":"s1", "text":"大量中转文本".repeat(200)}
            ]));
            node
        }).collect();
        extension.save_graph(&graph).unwrap();
        let cache = WorkflowCache::default();
        let first = cache.get(&extension).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..60 {
            let next = cache.get(&extension).unwrap();
            assert!(Arc::ptr_eq(&first, &next), "unchanged graph was reloaded for a UI frame");
        }
        eprintln!("200 relay nodes, 60 cached frame reads: {:?}", start.elapsed());
        graph.nodes[0].label = Some("changed graph label".into());
        extension.save_graph(&graph).unwrap();
        let edited = cache.get(&extension).unwrap();
        assert!(!Arc::ptr_eq(&first, &edited));
        assert_eq!(edited.graph.nodes[0].label.as_deref(), Some("changed graph label"));
        std::fs::write(extension.dir.join("graph.json"), "invalid").unwrap();
        assert!(cache.get(&extension).is_none(), "invalid edits must not return a stale graph");
        extension.save_graph(&graph).unwrap();
        assert!(cache.get(&extension).is_some());
        cache.clear();
        assert!(cache.entries.borrow().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
