use crate::{cache::MediaCache, MediaTools};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct MediaService {
    pub(crate) root: PathBuf,
    pub(crate) tools: MediaTools,
    pub(crate) cache: Arc<MediaCache>,
    pub(crate) timeout: Duration,
}
impl MediaService {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let tools = MediaTools::discover(&root);
        Self::with_tools(root, tools)
    }
    pub fn with_tools(root: impl Into<PathBuf>, tools: MediaTools) -> Self {
        let root = root.into();
        let cache = Arc::new(MediaCache::new(root.join("cache/media")));
        Self {
            root,
            tools,
            cache,
            timeout: Duration::from_secs(120),
        }
    }
    pub fn with_cache_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.cache = Arc::new(MediaCache::new(directory.into()));
        self
    }
    pub fn with_tool_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    pub fn tools(&self) -> &MediaTools {
        &self.tools
    }
    pub(crate) fn resolve(&self, stored: &str) -> Result<PathBuf, crate::MediaError> {
        super::files::resolve(&self.root, Path::new(stored))
    }
}
