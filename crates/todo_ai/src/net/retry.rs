//! What to do after a failed attempt. Pure: time and randomness are inputs.
use super::classify::ErrorClass;
use std::time::Duration;
use todo_production::Capability;

#[derive(Debug, Clone, PartialEq)]
pub struct RetryPolicy {
    pub base: Duration,
    pub cap: Duration,
    /// Upper bound honoured for a provider's `Retry-After`.
    pub max_retry_after: Duration,
    /// Extra billed attempts allowed after unknown outcomes, per task.
    pub paid_retries: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            base: Duration::from_secs(1),
            cap: Duration::from_secs(60),
            max_retry_after: Duration::from_secs(300),
            paid_retries: 2,
        }
    }
}

/// How long a task keeps retrying transient trouble before it fails.
pub fn deadline(capability: Capability) -> Duration {
    match capability {
        Capability::Chat | Capability::Speech => Duration::from_secs(10 * 60),
        Capability::Image => Duration::from_secs(20 * 60),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Retry {
        after: Duration,
    },
    /// The provider needs the user (key or balance); every task of it waits.
    PauseProvider,
    /// Final for this request. `possibly_billed` marks an exhausted paid budget.
    GiveUp {
        possibly_billed: bool,
    },
}

/// Attempt bookkeeping of one task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Attempts {
    /// Failed attempts so far (including the one being decided).
    pub failures: u32,
    /// Extra billed attempts already spent after unknown outcomes.
    pub paid_retries: u32,
}

impl RetryPolicy {
    /// Full-jitter exponential backoff; `unit` is a uniform sample in `[0, 1)`.
    pub fn backoff(&self, failures: u32, unit: f64) -> Duration {
        let exponent = failures.saturating_sub(1).min(16);
        let ceiling = self.base.saturating_mul(1u32 << exponent).min(self.cap);
        // Never retry instantly: keep at least a tenth of the base.
        let floor = self.base / 10;
        ceiling.mul_f64(unit.clamp(0.0, 1.0)).max(floor)
    }

    pub fn decide(
        &self,
        class: &ErrorClass,
        attempts: Attempts,
        elapsed: Duration,
        deadline: Duration,
        unit: f64,
    ) -> Decision {
        let after = self.backoff(attempts.failures, unit);
        let in_time = |wait: Duration| elapsed.saturating_add(wait) < deadline;
        match class {
            ErrorClass::Auth | ErrorClass::Billing => Decision::PauseProvider,
            ErrorClass::Invalid | ErrorClass::ContentPolicy => Decision::GiveUp {
                possibly_billed: false,
            },
            ErrorClass::Transient if in_time(after) => Decision::Retry { after },
            ErrorClass::RateLimited { retry_after } => {
                let wait = retry_after
                    .map(|wait| wait.min(self.max_retry_after))
                    .map_or(after, |wait| wait.max(after));
                if in_time(wait) {
                    Decision::Retry { after: wait }
                } else {
                    Decision::GiveUp {
                        possibly_billed: false,
                    }
                }
            }
            ErrorClass::Ambiguous
                if attempts.paid_retries < self.paid_retries && in_time(after) =>
            {
                Decision::Retry { after }
            }
            ErrorClass::Ambiguous => Decision::GiveUp {
                possibly_billed: true,
            },
            ErrorClass::Transient => Decision::GiveUp {
                possibly_billed: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempts(failures: u32, paid_retries: u32) -> Attempts {
        Attempts {
            failures,
            paid_retries,
        }
    }

    #[test]
    fn backoff_grows_exponentially_with_jitter_and_a_cap() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.backoff(1, 1.0), Duration::from_secs(1));
        assert_eq!(policy.backoff(3, 1.0), Duration::from_secs(4));
        assert_eq!(policy.backoff(30, 1.0), Duration::from_secs(60));
        assert_eq!(policy.backoff(3, 0.5), Duration::from_secs(2));
        assert_eq!(policy.backoff(3, 0.0), Duration::from_millis(100));
    }

    #[test]
    fn transient_trouble_retries_until_the_deadline() {
        let policy = RetryPolicy::default();
        let deadline = deadline(Capability::Chat);
        assert_eq!(
            policy.decide(
                &ErrorClass::Transient,
                attempts(5, 0),
                Duration::from_secs(60),
                deadline,
                1.0
            ),
            Decision::Retry {
                after: Duration::from_secs(16)
            }
        );
        assert_eq!(
            policy.decide(
                &ErrorClass::Transient,
                attempts(9, 0),
                Duration::from_secs(599),
                deadline,
                1.0
            ),
            Decision::GiveUp {
                possibly_billed: false
            }
        );
    }

    #[test]
    fn rate_limits_honour_retry_after() {
        let policy = RetryPolicy::default();
        let class = ErrorClass::RateLimited {
            retry_after: Some(Duration::from_secs(30)),
        };
        assert_eq!(
            policy.decide(
                &class,
                attempts(1, 0),
                Duration::ZERO,
                deadline(Capability::Image),
                1.0
            ),
            Decision::Retry {
                after: Duration::from_secs(30)
            }
        );
        let huge = ErrorClass::RateLimited {
            retry_after: Some(Duration::from_secs(9999)),
        };
        assert_eq!(
            policy.decide(
                &huge,
                attempts(1, 0),
                Duration::ZERO,
                deadline(Capability::Image),
                1.0
            ),
            Decision::Retry {
                after: Duration::from_secs(300)
            }
        );
    }

    #[test]
    fn unknown_billed_outcomes_spend_a_budget() {
        let policy = RetryPolicy::default();
        let d = deadline(Capability::Image);
        assert!(matches!(
            policy.decide(
                &ErrorClass::Ambiguous,
                attempts(1, 1),
                Duration::ZERO,
                d,
                1.0
            ),
            Decision::Retry { .. }
        ));
        assert_eq!(
            policy.decide(
                &ErrorClass::Ambiguous,
                attempts(3, 2),
                Duration::ZERO,
                d,
                1.0
            ),
            Decision::GiveUp {
                possibly_billed: true
            }
        );
    }

    #[test]
    fn key_and_balance_problems_pause_instead_of_failing() {
        let policy = RetryPolicy::default();
        let d = deadline(Capability::Speech);
        for class in [ErrorClass::Auth, ErrorClass::Billing] {
            assert_eq!(
                policy.decide(&class, attempts(1, 0), Duration::ZERO, d, 0.5),
                Decision::PauseProvider
            );
        }
        assert_eq!(
            policy.decide(
                &ErrorClass::ContentPolicy,
                attempts(1, 0),
                Duration::ZERO,
                d,
                0.5
            ),
            Decision::GiveUp {
                possibly_billed: false
            }
        );
    }
}
