//! Per-provider circuit breaker. While the network is down every task of the
//! provider waits for one probe instead of each failing on its own.
use std::collections::VecDeque;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct BreakerPolicy {
    /// Consecutive transient failures within `window` that open the circuit.
    pub threshold: usize,
    pub window: Duration,
    /// Wait before letting a single probe through.
    pub open_for: Duration,
}

impl Default for BreakerPolicy {
    fn default() -> Self {
        Self {
            threshold: 5,
            window: Duration::from_secs(60),
            open_for: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Permit {
    /// Send the request.
    Go,
    /// Send it as the single probe; report its outcome.
    Probe,
    /// Do not send; ask again after this long.
    Wait(Duration),
    /// The provider needs the user (key or balance).
    Paused,
}

#[derive(Debug, Clone, PartialEq)]
enum State {
    Closed,
    Open { until: Duration },
    Probing,
}

/// Times are durations since an arbitrary monotonic origin.
#[derive(Debug, Clone)]
pub struct Breaker {
    policy: BreakerPolicy,
    state: State,
    failures: VecDeque<Duration>,
    paused: bool,
}

impl Breaker {
    pub fn new(policy: BreakerPolicy) -> Self {
        Self {
            policy,
            state: State::Closed,
            failures: VecDeque::new(),
            paused: false,
        }
    }

    pub fn permit(&mut self, now: Duration) -> Permit {
        if self.paused {
            return Permit::Paused;
        }
        match self.state {
            State::Closed => Permit::Go,
            State::Open { until } if now < until => Permit::Wait(until - now),
            State::Open { .. } => {
                self.state = State::Probing;
                Permit::Probe
            }
            // Others wait for the probe's verdict; poll again shortly.
            State::Probing => Permit::Wait(Duration::from_secs(1)),
        }
    }

    pub fn is_open(&self) -> bool {
        !matches!(self.state, State::Closed)
    }

    /// Any successful exchange (even a 4xx answer) proves the network works.
    pub fn success(&mut self) {
        self.failures.clear();
        self.state = State::Closed;
    }

    /// A transient network/gateway failure.
    pub fn failure(&mut self, now: Duration) {
        if self.state == State::Probing {
            self.state = State::Open {
                until: now + self.policy.open_for,
            };
            return;
        }
        self.failures.push_back(now);
        while self
            .failures
            .front()
            .is_some_and(|first| now.saturating_sub(*first) > self.policy.window)
        {
            self.failures.pop_front();
        }
        if self.failures.len() >= self.policy.threshold {
            self.failures.clear();
            self.state = State::Open {
                until: now + self.policy.open_for,
            };
        }
    }

    /// Key rejected or balance exhausted.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// The key or balance changed; let requests through again.
    pub fn resume(&mut self) {
        self.paused = false;
        self.success();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    #[test]
    fn opens_after_repeated_failures_and_recovers_through_one_probe() {
        let mut breaker = Breaker::new(BreakerPolicy::default());
        for t in 0..4 {
            breaker.failure(s(t));
            assert_eq!(breaker.permit(s(t)), Permit::Go);
        }
        breaker.failure(s(4));
        assert_eq!(breaker.permit(s(5)), Permit::Wait(s(29)));
        assert_eq!(breaker.permit(s(34)), Permit::Probe);
        // Everyone else waits for the probe.
        assert_eq!(breaker.permit(s(34)), Permit::Wait(s(1)));
        breaker.failure(s(35));
        assert_eq!(breaker.permit(s(36)), Permit::Wait(s(29)));
        assert_eq!(breaker.permit(s(65)), Permit::Probe);
        breaker.success();
        assert_eq!(breaker.permit(s(66)), Permit::Go);
        assert!(!breaker.is_open());
    }

    #[test]
    fn failures_outside_the_window_do_not_accumulate() {
        let mut breaker = Breaker::new(BreakerPolicy::default());
        for t in [0, 20, 40, 70, 100] {
            breaker.failure(s(t));
        }
        assert_eq!(breaker.permit(s(100)), Permit::Go);
    }

    #[test]
    fn pause_lasts_until_resumed() {
        let mut breaker = Breaker::new(BreakerPolicy::default());
        breaker.pause();
        assert_eq!(breaker.permit(s(1000)), Permit::Paused);
        breaker.resume();
        assert_eq!(breaker.permit(s(1000)), Permit::Go);
    }
}
