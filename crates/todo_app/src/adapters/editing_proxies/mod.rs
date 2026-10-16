//! One cancellable proxy job at a time; source media remains immediately usable.
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    thread::{self, JoinHandle},
};
use todo_media::{EditProxy, MediaError, MediaService};

struct Job {
    id: u64,
    session: u64,
    path: String,
    cancelled: Arc<AtomicBool>,
}
struct Completion {
    id: u64,
    session: u64,
    path: String,
    result: Result<EditProxy, MediaError>,
}
struct Active {
    id: u64,
    path: String,
    cancelled: Arc<AtomicBool>,
}

pub struct EditingProxies {
    tx: Option<SyncSender<Job>>,
    rx: Receiver<Completion>,
    worker: Option<JoinHandle<()>>,
    session: u64,
    next_id: u64,
    sources: Vec<String>,
    ready: HashMap<String, PathBuf>,
    failed: HashSet<String>,
    active: Option<Active>,
}

impl EditingProxies {
    pub fn start(repo: PathBuf) -> Self {
        let (tx, jobs) = mpsc::sync_channel::<Job>(1);
        let (done, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let service = MediaService::new(repo);
            while let Ok(job) = jobs.recv() {
                let result = service.get_edit_proxy_cancellable(&job.path, job.cancelled);
                if done
                    .send(Completion {
                        id: job.id,
                        session: job.session,
                        path: job.path,
                        result,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            tx: Some(tx),
            rx,
            worker: Some(worker),
            session: 0,
            next_id: 0,
            sources: Vec::new(),
            ready: HashMap::new(),
            failed: HashSet::new(),
            active: None,
        }
    }

    pub fn set_sources(&mut self, session: u64, sources: Vec<String>) {
        if self.session != session {
            self.cancel_active();
            self.ready.clear();
            self.failed.clear();
            self.session = session;
        }
        if self
            .active
            .as_ref()
            .is_some_and(|job| !sources.contains(&job.path))
        {
            self.cancel_active();
        }
        self.ready.retain(|path, _| sources.contains(path));
        self.failed.retain(|path| sources.contains(path));
        self.sources = sources;
    }

    pub fn poll(&mut self) -> Vec<(String, PathBuf)> {
        let mut completed = Vec::new();
        while let Ok(done) = self.rx.try_recv() {
            if done.session != self.session
                || !self.active.as_ref().is_some_and(|job| job.id == done.id)
            {
                continue;
            }
            self.active = None;
            if !self.sources.contains(&done.path) {
                continue;
            }
            match done.result {
                Ok(proxy) => {
                    self.ready.insert(done.path.clone(), proxy.path.clone());
                    completed.push((done.path, proxy.path));
                }
                Err(MediaError::Cancelled) => {}
                Err(_) => {
                    self.failed.insert(done.path);
                }
            }
        }
        completed
    }

    pub fn pump(&mut self, priority: Option<&str>) {
        if self.active.is_some() {
            return;
        }
        let wanted =
            |path: &&String| !self.ready.contains_key(*path) && !self.failed.contains(*path);
        let next = priority
            .and_then(|path| {
                self.sources
                    .iter()
                    .find(|source| source.as_str() == path)
                    .filter(wanted)
            })
            .or_else(|| self.sources.iter().find(wanted));
        let Some(path) = next.cloned() else {
            return;
        };
        let Some(tx) = &self.tx else {
            return;
        };
        self.next_id = self.next_id.wrapping_add(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let job = Job {
            id: self.next_id,
            session: self.session,
            path: path.clone(),
            cancelled: cancelled.clone(),
        };
        match tx.try_send(job) {
            Ok(()) => {
                self.active = Some(Active {
                    id: self.next_id,
                    path,
                    cancelled,
                })
            }
            Err(mpsc::TrySendError::Full(_)) => {}
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.failed.insert(path);
            }
        }
    }

    pub fn summary(&self) -> Option<String> {
        if self.sources.is_empty() || (self.ready.len() == self.sources.len()) {
            return None;
        }
        Some(if self.failed.is_empty() {
            format!("正在优化预览 {}/{}", self.ready.len(), self.sources.len())
        } else {
            format!(
                "预览优化 {}/{} · {} 项使用原素材",
                self.ready.len(),
                self.sources.len(),
                self.failed.len()
            )
        })
    }

    fn cancel_active(&mut self) {
        if let Some(active) = self.active.take() {
            active.cancelled.store(true, Ordering::Release);
        }
    }

    pub fn shutdown(&mut self) {
        self.cancel_active();
        self.tx.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for EditingProxies {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests;
