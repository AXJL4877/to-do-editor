//! Durable task journal: one JSON record per task key, written atomically.
//! The key is the task fingerprint, so a request is one record forever: a
//! finished record is the cache, an interrupted one resumes instead of
//! submitting again.
use crate::artifacts::ArtifactRef;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use todo_production::{TaskKey, TaskSpec};

const RECORD_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskState {
    Pending,
    /// The request is being sent. Found at startup, its outcome is unknown.
    Submitting,
    /// Accepted by the provider under `remote_id`; poll, never submit again.
    Submitted,
    /// The result exists at `remote_url`; fetch it, never generate again.
    Downloading,
    Done,
    Failed,
    /// A billed request with an unknown outcome and no retry budget left.
    Ambiguous,
}

impl TaskState {
    pub fn is_final(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Ambiguous)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub version: u32,
    pub key: TaskKey,
    pub spec: TaskSpec,
    pub state: TaskState,
    /// Times the request was sent.
    pub submits: u32,
    /// Failed attempts since the last success or manual retry.
    pub failures: u32,
    /// Extra billed attempts spent after unknown outcomes.
    pub paid_retries: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ArtifactRef>,
    /// Structured provider output that is not a file (chat text, timestamps).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Unix milliseconds.
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("unknown task {0}")]
    Unknown(TaskKey),
}

/// What startup recovery did to an interrupted record.
#[derive(Debug, Clone, PartialEq)]
pub struct Recovered {
    pub key: TaskKey,
    pub from: TaskState,
    pub to: TaskState,
}

pub struct Journal {
    directory: PathBuf,
    records: HashMap<TaskKey, TaskRecord>,
    /// Unreadable record files, left untouched for inspection.
    pub skipped: Vec<PathBuf>,
}

fn io(path: &Path, source: std::io::Error) -> JournalError {
    JournalError::Io {
        path: path.into(),
        source,
    }
}

impl Journal {
    /// Load every record and settle those a crash interrupted. `paid_budget`
    /// is the number of extra billed attempts a task may spend.
    pub fn open(
        directory: impl Into<PathBuf>,
        paid_budget: u32,
        now: u64,
    ) -> Result<(Self, Vec<Recovered>), JournalError> {
        let directory = directory.into();
        fs::create_dir_all(&directory).map_err(|e| io(&directory, e))?;
        let mut journal = Self {
            directory,
            records: HashMap::new(),
            skipped: Vec::new(),
        };
        let entries = fs::read_dir(&journal.directory).map_err(|e| io(&journal.directory, e))?;
        for entry in entries {
            let path = entry.map_err(|e| io(&journal.directory, e))?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let record = fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<TaskRecord>(&bytes).ok())
                .filter(|record| {
                    record.version == RECORD_VERSION
                        && record.spec.key() == record.key
                        && path.file_stem().and_then(|s| s.to_str()) == Some(record.key.as_str())
                });
            match record {
                Some(record) => {
                    journal.records.insert(record.key.clone(), record);
                }
                None => journal.skipped.push(path),
            }
        }
        let mut recovered = Vec::new();
        let interrupted: Vec<TaskKey> = journal
            .records
            .values()
            .filter(|record| record.state == TaskState::Submitting)
            .map(|record| record.key.clone())
            .collect();
        for key in interrupted {
            let state = journal.settle_interrupted(&key, paid_budget, now)?.state;
            recovered.push(Recovered {
                key,
                from: TaskState::Submitting,
                to: state,
            });
        }
        Ok((journal, recovered))
    }

    /// A send was interrupted (crash, cancel): its outcome is unknown. Free
    /// requests simply run again; billed ones spend the paid retry budget.
    pub fn settle_interrupted(
        &mut self,
        key: &TaskKey,
        paid_budget: u32,
        now: u64,
    ) -> Result<&TaskRecord, JournalError> {
        self.update(key, now, |record| {
            if record.state != TaskState::Submitting {
                return;
            }
            if !record.spec.capability().is_paid_per_attempt() {
                record.state = TaskState::Pending;
            } else if record.paid_retries < paid_budget {
                record.paid_retries += 1;
                record.state = TaskState::Pending;
            } else {
                record.state = TaskState::Ambiguous;
                record.error_class = Some("ambiguous".into());
                record.error = Some("interrupted while sending; may have been billed".into());
            }
        })
    }

    pub fn get(&self, key: &TaskKey) -> Option<&TaskRecord> {
        self.records.get(key)
    }

    pub fn records(&self) -> impl Iterator<Item = &TaskRecord> {
        self.records.values()
    }

    /// The record of `spec`, created as `Pending` when it does not exist yet.
    pub fn ensure(&mut self, spec: &TaskSpec, now: u64) -> Result<&TaskRecord, JournalError> {
        let key = spec.key();
        if !self.records.contains_key(&key) {
            let record = TaskRecord {
                version: RECORD_VERSION,
                key: key.clone(),
                spec: spec.clone(),
                state: TaskState::Pending,
                submits: 0,
                failures: 0,
                paid_retries: 0,
                remote_id: None,
                remote_url: None,
                artifact: None,
                output: None,
                error_class: None,
                error: None,
                created_at: now,
                updated_at: now,
            };
            self.persist(&record)?;
            self.records.insert(key.clone(), record);
        }
        Ok(&self.records[&key])
    }

    /// Change a record and persist it before returning; the record on disk
    /// is never behind the one in memory.
    pub fn update(
        &mut self,
        key: &TaskKey,
        now: u64,
        change: impl FnOnce(&mut TaskRecord),
    ) -> Result<&TaskRecord, JournalError> {
        let mut record = self
            .records
            .get(key)
            .cloned()
            .ok_or_else(|| JournalError::Unknown(key.clone()))?;
        change(&mut record);
        record.updated_at = now;
        self.persist(&record)?;
        self.records.insert(key.clone(), record);
        Ok(&self.records[key])
    }

    /// The user asks again after a final failure: start over with fresh counters.
    pub fn retry(&mut self, key: &TaskKey, now: u64) -> Result<&TaskRecord, JournalError> {
        self.update(key, now, |record| {
            if matches!(record.state, TaskState::Failed | TaskState::Ambiguous) {
                record.state = if record.remote_url.is_some() {
                    TaskState::Downloading
                } else {
                    TaskState::Pending
                };
                record.failures = 0;
                record.paid_retries = 0;
                record.error_class = None;
                record.error = None;
            }
        })
    }

    fn path(&self, key: &TaskKey) -> PathBuf {
        self.directory.join(format!("{key}.json"))
    }

    fn persist(&self, record: &TaskRecord) -> Result<(), JournalError> {
        let path = self.path(&record.key);
        let temporary = self
            .directory
            .join(format!(".{}.{}.tmp", record.key, std::process::id()));
        let bytes = serde_json::to_vec_pretty(record).expect("records serialize");
        let write = || -> std::io::Result<()> {
            let mut file = fs::File::create(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &path)
        };
        write().map_err(|e| {
            let _ = fs::remove_file(&temporary);
            io(&path, e)
        })
    }
}
