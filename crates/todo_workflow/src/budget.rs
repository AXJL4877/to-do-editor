//! Per-production service budgets for extension workers.
//!
//! A budget is charged by the semantic request fingerprint, so retries and
//! worker restarts do not consume another unit for the same call.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use crate::workflow::manifest::Service;
use crate::workflow::protocol::{ServiceError, ServiceErrorClass};
use todo_production::tasks::fingerprint;

#[derive(Debug, Clone)]
pub struct Budget {
    limits: BTreeMap<Service, u32>,
    charged: Arc<Mutex<BTreeMap<Service, BTreeSet<String>>>>,
}

impl Budget {
    pub fn new(limits: &BTreeMap<Service, u32>) -> Self {
        Self {
            limits: limits.clone(),
            charged: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Charge one distinct request. Repeating an identical request is free.
    pub fn charge(
        &self,
        service: Service,
        request: &serde_json::Value,
    ) -> Result<(), ServiceError> {
        let key = fingerprint(request);
        let mut charged = self
            .charged
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let entries = charged.entry(service).or_default();
        if entries.contains(&key) {
            return Ok(());
        }
        if self
            .limits
            .get(&service)
            .is_some_and(|limit| entries.len() >= *limit as usize)
        {
            return Err(ServiceError {
                class: ServiceErrorClass::Budget,
                message: format!("服务 {service:?} 已达到本次成片预算"),
            });
        }
        entries.insert(key);
        Ok(())
    }

    pub fn used(&self, service: Service) -> usize {
        self.charged
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&service)
            .map_or(0, BTreeSet::len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn identical_requests_charge_once() {
        let limits = BTreeMap::from([(Service::Image, 1)]);
        let budget = Budget::new(&limits);
        let request = json!({"prompt":"same", "size":"1024x1024"});
        budget.charge(Service::Image, &request).unwrap();
        budget.charge(Service::Image, &request).unwrap();
        assert_eq!(budget.used(Service::Image), 1);
    }

    #[test]
    fn a_different_request_exhausts_the_limit() {
        let limits = BTreeMap::from([(Service::Image, 1)]);
        let budget = Budget::new(&limits);
        budget.charge(Service::Image, &json!({"n":1})).unwrap();
        let error = budget.charge(Service::Image, &json!({"n":2})).unwrap_err();
        assert_eq!(error.class, ServiceErrorClass::Budget);
    }
}
