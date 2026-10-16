//! Resource task identity and retry policy, independent of worker threads and egui.
use crate::{MediaTask, Ticket};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct PeakStrip {
    pub duration: f64,
    pub peaks: Vec<f64>,
}

#[derive(Debug, Default, Clone)]
pub struct PeaksCache {
    strips: HashMap<String, PeakStrip>,
}
impl PeaksCache {
    pub fn get(&self, path: &str) -> Option<&PeakStrip> {
        self.strips.get(path)
    }
    pub fn contains(&self, path: &str) -> bool {
        self.strips.contains_key(path)
    }
    pub fn insert(&mut self, path: String, duration: f64, peaks: Vec<f64>) {
        self.strips.insert(path, PeakStrip { duration, peaks });
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssetRequest {
    Peaks { path: String, bins: u32 },
    Thumbnail { path: String, width: u32 },
}
impl AssetRequest {
    fn key(&self) -> String {
        match self {
            Self::Peaks { path, bins } => format!("peaks:{bins}:{path}"),
            Self::Thumbnail { path, width } => format!("thumb:{width}:{path}"),
        }
    }
    pub(crate) fn task(&self) -> MediaTask {
        match self {
            Self::Peaks { path, bins } => MediaTask::Peaks {
                path: path.clone(),
                bins: *bins,
            },
            Self::Thumbnail { path, width } => MediaTask::Thumbnail {
                path: path.clone(),
                width: *width,
            },
        }
    }
}

#[derive(Default)]
pub(crate) struct MediaJobs {
    pub import: Option<Ticket>,
    inflight: HashMap<Ticket, String>,
    ready: HashSet<String>,
    failures: HashMap<String, (u8, f64)>,
}
impl MediaJobs {
    pub fn request(&mut self, ticket: Ticket, request: &AssetRequest, now: f64) -> bool {
        let key = request.key();
        if self.inflight.len() >= 12
            || self.ready.contains(&key)
            || self.inflight.values().any(|item| item == &key)
            || self
                .failures
                .get(&key)
                .is_some_and(|(n, retry)| *n >= 3 || now < *retry)
        {
            return false;
        }
        self.inflight.insert(ticket, key);
        true
    }
    pub fn complete(&mut self, ticket: Ticket, success: bool, now: f64) -> bool {
        let Some(key) = self.inflight.remove(&ticket) else {
            return false;
        };
        if success {
            self.ready.insert(key.clone());
            self.failures.remove(&key);
        } else {
            let count = self.failures.get(&key).map_or(1, |(n, _)| n + 1);
            self.failures
                .insert(key, (count, now + 2.0_f64.powi(i32::from(count))));
        }
        true
    }
    pub fn busy(&self) -> bool {
        self.import.is_some() || !self.inflight.is_empty()
    }
}
