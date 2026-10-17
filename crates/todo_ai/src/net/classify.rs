//! Typed failure classes. Decisions are made on these, never on message text
//! matching at the retry site (the old pipeline's `TRANSIENT_RE`).
use std::time::Duration;

/// Where a transport failure happened relative to sending the request body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// DNS, refused, connect or TLS handshake: the request certainly never arrived.
    Connect,
    /// The request may have reached the provider (timeout or reset afterwards).
    Sent,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorClass {
    /// Network or gateway trouble; retry with backoff until the deadline.
    Transient,
    /// Throttled; wait at least `retry_after` when the provider says so.
    RateLimited { retry_after: Option<Duration> },
    /// Key rejected: pause the provider until the key changes.
    Auth,
    /// Balance or quota exhausted: pause the provider until topped up.
    Billing,
    /// The request itself is wrong; retrying cannot help.
    Invalid,
    /// Moderation refused the prompt; the caller may rewrite it.
    ContentPolicy,
    /// A billed request may have been processed; retry only within budget.
    Ambiguous,
}

/// Failure marks the toolkit treats as billing (`image_batch.py`).
const BILLING_MARKS: &[&str] = &[
    "insufficient",
    "余额",
    "quota",
    "billing",
    "recharge",
    "充值",
];
const POLICY_MARKS: &[&str] = &[
    "content_policy",
    "content policy",
    "moderation",
    "safety",
    "sensitive",
    "违规",
    "敏感",
    "审核",
];

fn contains_any(body: &str, marks: &[&str]) -> bool {
    let lower = body.to_lowercase();
    marks.iter().any(|mark| lower.contains(mark))
}

/// Classify an HTTP response status. `paid` marks requests billed per attempt:
/// a gateway error after such a request was sent may still have been charged.
pub fn classify_status(
    status: u16,
    body: &str,
    retry_after: Option<Duration>,
    paid: bool,
) -> ErrorClass {
    if status == 402 || ((400..500).contains(&status) && contains_any(body, BILLING_MARKS)) {
        return ErrorClass::Billing;
    }
    match status {
        401 | 403 => ErrorClass::Auth,
        429 => ErrorClass::RateLimited { retry_after },
        408 | 425 => ErrorClass::Transient,
        500..=599 if paid => ErrorClass::Ambiguous,
        500..=599 => ErrorClass::Transient,
        400..=499 if contains_any(body, POLICY_MARKS) => ErrorClass::ContentPolicy,
        _ => ErrorClass::Invalid,
    }
}

/// Classify a failure without a response.
pub fn classify_transport(stage: Stage, paid: bool) -> ErrorClass {
    match (stage, paid) {
        (Stage::Connect, _) | (Stage::Sent, false) => ErrorClass::Transient,
        (Stage::Sent, true) => ErrorClass::Ambiguous,
    }
}

/// `Retry-After` in delta-seconds form (HTTP dates are treated as absent).
pub fn parse_retry_after(value: &str) -> Option<Duration> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .map(Duration::from_secs_f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses() {
        assert_eq!(classify_status(401, "", None, false), ErrorClass::Auth);
        assert_eq!(classify_status(403, "", None, true), ErrorClass::Auth);
        assert_eq!(classify_status(402, "", None, false), ErrorClass::Billing);
        assert_eq!(
            classify_status(400, r#"{"error":{"message":"余额不足"}}"#, None, true),
            ErrorClass::Billing
        );
        assert_eq!(
            classify_status(429, r#"{"error":"insufficient_quota"}"#, None, false),
            ErrorClass::Billing
        );
        let wait = Some(Duration::from_secs(7));
        assert_eq!(
            classify_status(429, "slow down", wait, false),
            ErrorClass::RateLimited { retry_after: wait }
        );
        assert_eq!(
            classify_status(502, "<html>nginx</html>", None, false),
            ErrorClass::Transient
        );
        assert_eq!(
            classify_status(502, "<html>nginx</html>", None, true),
            ErrorClass::Ambiguous
        );
        assert_eq!(
            classify_status(400, "request rejected by content_policy", None, true),
            ErrorClass::ContentPolicy
        );
        assert_eq!(
            classify_status(400, "bad size", None, true),
            ErrorClass::Invalid
        );
        assert_eq!(classify_status(404, "", None, false), ErrorClass::Invalid);
    }

    #[test]
    fn transport_stage_decides_ambiguity() {
        assert_eq!(
            classify_transport(Stage::Connect, true),
            ErrorClass::Transient
        );
        assert_eq!(
            classify_transport(Stage::Sent, false),
            ErrorClass::Transient
        );
        assert_eq!(classify_transport(Stage::Sent, true), ErrorClass::Ambiguous);
    }

    #[test]
    fn retry_after_seconds() {
        assert_eq!(parse_retry_after(" 3 "), Some(Duration::from_secs(3)));
        assert_eq!(parse_retry_after("1.5"), Some(Duration::from_millis(1500)));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), None);
        assert_eq!(parse_retry_after("-1"), None);
    }
}
