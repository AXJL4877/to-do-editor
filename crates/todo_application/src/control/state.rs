use super::*;
use crate::Ticket;
use std::collections::{HashSet, VecDeque};

const RESPONSE_WINDOW: usize = 128;
const REQUEST_LIMIT: usize = 16_384;

#[derive(Default)]
pub(crate) struct ControlState {
    responses: VecDeque<(ControlRequest, ControlResponse)>,
    seen: HashSet<String>,
    imports: VecDeque<ImportRecord>,
}

struct ImportRecord {
    job: ImportJob,
    append: bool,
}

impl ControlState {
    pub fn replay(
        &self,
        request: &ControlRequest,
    ) -> Result<Option<ControlResponse>, ControlError> {
        if let Some((previous, response)) = self
            .responses
            .iter()
            .find(|(previous, _)| previous.request_id == request.request_id)
        {
            return if previous == request {
                Ok(Some(response.clone()))
            } else {
                Err(ControlError::new(
                    "request_id_conflict",
                    "This requestId was already used with different parameters",
                ))
            };
        }
        if self.seen.contains(&request.request_id) {
            return Err(ControlError::new(
                "request_expired",
                "The receipt expired; this requestId cannot execute again",
            ));
        }
        if self.seen.len() >= REQUEST_LIMIT {
            return Err(ControlError::new(
                "request_history_full",
                "Reopen the project to start a new control session",
            ));
        }
        Ok(None)
    }

    pub fn remember(&mut self, request: ControlRequest, response: ControlResponse) {
        self.seen.insert(request.request_id.clone());
        self.responses.push_back((request, response));
        if self.responses.len() > RESPONSE_WINDOW {
            self.responses.pop_front();
        }
    }

    pub fn begin_import(&mut self, ticket: Ticket, total: usize, append: bool) {
        self.imports.push_back(ImportRecord {
            append,
            job: ImportJob {
                job_id: ticket.id,
                session_id: ticket.session,
                status: ImportStatus::Running,
                total,
                completed: 0,
                media_ids: Vec::new(),
                errors: Vec::new(),
            },
        });
        if self.imports.len() > RESPONSE_WINDOW {
            self.imports.pop_front();
        }
    }

    pub fn jobs(&self) -> Vec<ImportJob> {
        self.imports
            .iter()
            .map(|record| record.job.clone())
            .collect()
    }
    pub fn job(&self, id: u64) -> Option<ImportJob> {
        self.imports
            .iter()
            .find(|record| record.job.job_id == id)
            .map(|record| record.job.clone())
    }
    pub fn append_import(&self, ticket: Ticket) -> bool {
        self.imports
            .iter()
            .find(|record| record.job.job_id == ticket.id)
            .is_none_or(|record| record.append)
    }
    pub fn already_imported(&self, ticket: Ticket, id: &str) -> bool {
        self.imports
            .iter()
            .find(|record| record.job.job_id == ticket.id)
            .is_some_and(|record| record.job.media_ids.iter().any(|item| item == id))
    }
    pub fn imported(&mut self, ticket: Ticket, id: String, placement_error: Option<String>) {
        if let Some(record) = self
            .imports
            .iter_mut()
            .find(|record| record.job.job_id == ticket.id)
        {
            if !record.job.media_ids.contains(&id) {
                record.job.media_ids.push(id);
                record.job.completed += 1;
                if let Some(message) = placement_error {
                    record.job.errors.push(message);
                }
            }
        }
    }
    pub fn import_error(&mut self, ticket: Ticket, message: String) {
        if let Some(record) = self
            .imports
            .iter_mut()
            .find(|record| record.job.job_id == ticket.id)
        {
            record.job.errors.push(message);
            record.job.completed = (record.job.completed + 1).min(record.job.total);
        }
    }
    pub fn finish_import(&mut self, ticket: Ticket) {
        if let Some(record) = self
            .imports
            .iter_mut()
            .find(|record| record.job.job_id == ticket.id)
        {
            if record.job.completed < record.job.total && record.job.errors.is_empty() {
                record
                    .job
                    .errors
                    .push("Import ended before all files completed".into());
            }
            record.job.status = if record.job.errors.is_empty() {
                ImportStatus::Completed
            } else if record.job.media_ids.is_empty() {
                ImportStatus::Failed
            } else {
                ImportStatus::CompletedWithErrors
            };
        }
    }
}
