use std::time::{Duration, SystemTime};

use reqwest::header::{HeaderMap, RETRY_AFTER};
use serde::Deserialize;

/// How the client retries HTTP 429 (rate limited) responses.
///
/// Only 429 is retried. Each wait is, in order of preference: the
/// `Retry-After` header (seconds or an HTTP date), the body's
/// `retryAfterSecs`, or exponential backoff from `initial_backoff` capped at
/// `max_backoff`. A random jitter of up to `max_jitter` is added to every
/// wait. A wait that would run past `budget` (measured from the first
/// attempt) is not attempted; the 429 is returned as
/// [`Error::RateLimited`](crate::Error::RateLimited) instead.
///
/// ```
/// use std::time::Duration;
/// use hydromancer::RetryPolicy;
///
/// let mut policy = RetryPolicy::default();
/// policy.max_retries = 5;
/// policy.budget = Duration::from_secs(120);
/// # let _ = policy;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RetryPolicy {
    /// Retries after the first attempt. `0` disables retries. Default 3.
    pub max_retries: u32,
    /// Total time allowed for all attempts and waits. Default 90 s.
    pub budget: Duration,
    /// First backoff when the server gives no wait time. Default 1 s.
    pub initial_backoff: Duration,
    /// Cap for the exponential backoff (not for server-given waits). Default 30 s.
    pub max_backoff: Duration,
    /// Upper bound of the random jitter added to each wait. Default 1 s.
    pub max_jitter: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            budget: Duration::from_secs(90),
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
            max_jitter: Duration::from_secs(1),
        }
    }
}

impl RetryPolicy {
    /// A policy that never retries.
    pub fn none() -> Self {
        Self {
            max_retries: 0,
            ..Self::default()
        }
    }

    /// The wait before retry number `retry` (0-based), without jitter.
    pub(crate) fn wait(&self, server_hint: Option<Duration>, retry: u32) -> Duration {
        match server_hint {
            Some(d) => d,
            None => {
                let factor = 2u32.saturating_pow(retry.min(31));
                self.initial_backoff.saturating_mul(factor).min(self.max_backoff)
            }
        }
    }

    pub(crate) fn jitter(&self) -> Duration {
        Duration::try_from_secs_f64(self.max_jitter.as_secs_f64() * fastrand::f64()).unwrap_or(self.max_jitter)
    }
}

/// The JSON body of a 429.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RateLimitBody {
    pub retry_after_secs: Option<f64>,
    pub remaining: Option<u64>,
    pub required: Option<u64>,
    pub correlation_id: Option<String>,
}

impl RateLimitBody {
    pub(crate) fn parse(body: &[u8]) -> Self {
        serde_json::from_slice(body).unwrap_or_default()
    }

    pub(crate) fn retry_after(&self) -> Option<Duration> {
        self.retry_after_secs.and_then(|s| Duration::try_from_secs_f64(s).ok())
    }
}

/// Parses `Retry-After` as delta-seconds or an HTTP date.
pub(crate) fn retry_after_header(headers: &HeaderMap, now: SystemTime) -> Option<Duration> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    if let Ok(secs) = value.parse::<f64>() {
        return Duration::try_from_secs_f64(secs).ok();
    }
    let at = httpdate::parse_http_date(value).ok()?;
    Some(at.duration_since(now).unwrap_or(Duration::ZERO))
}

pub(crate) fn header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn headers(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(RETRY_AFTER, HeaderValue::from_str(v).unwrap());
        h
    }

    #[test]
    fn retry_after_seconds() {
        assert_eq!(
            retry_after_header(&headers("12"), SystemTime::now()),
            Some(Duration::from_secs(12))
        );
        assert_eq!(
            retry_after_header(&headers(" 0 "), SystemTime::now()),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn retry_after_http_date() {
        let now = httpdate::parse_http_date("Fri, 30 Oct 2026 01:00:00 GMT").unwrap();
        let h = headers("Fri, 30 Oct 2026 01:00:07 GMT");
        assert_eq!(retry_after_header(&h, now), Some(Duration::from_secs(7)));
        // A date in the past means "retry now".
        let h = headers("Fri, 30 Oct 2026 00:59:00 GMT");
        assert_eq!(retry_after_header(&h, now), Some(Duration::ZERO));
    }

    #[test]
    fn retry_after_garbage_and_missing() {
        assert_eq!(retry_after_header(&headers("soon"), SystemTime::now()), None);
        assert_eq!(retry_after_header(&headers("-3"), SystemTime::now()), None);
        assert_eq!(retry_after_header(&headers("1e300"), SystemTime::now()), None);
        assert_eq!(
            retry_after_header(&headers("18446744073709551615"), SystemTime::now()),
            Some(Duration::from_secs(u64::MAX))
        );
        assert_eq!(retry_after_header(&HeaderMap::new(), SystemTime::now()), None);
    }

    #[test]
    fn body_retry_after() {
        let b = RateLimitBody::parse(br#"{"error":"Rate limit exceeded","retryAfterSecs":12,"remaining":8,"required":20,"correlationId":"AB12"}"#);
        assert_eq!(b.retry_after(), Some(Duration::from_secs(12)));
        assert_eq!((b.remaining, b.required), (Some(8), Some(20)));
        assert_eq!(b.correlation_id.as_deref(), Some("AB12"));
        assert!(RateLimitBody::parse(b"not json").retry_after().is_none());
        // Unrepresentable values are ignored instead of panicking.
        assert!(RateLimitBody::parse(br#"{"retryAfterSecs":1e300}"#)
            .retry_after()
            .is_none());
        assert!(RateLimitBody::parse(br#"{"retryAfterSecs":-1}"#)
            .retry_after()
            .is_none());
    }

    #[test]
    fn backoff_is_exponential_and_capped() {
        let p = RetryPolicy::default();
        let waits: Vec<u64> = (0..7).map(|r| p.wait(None, r).as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30]);
        assert_eq!(p.wait(None, 1000), Duration::from_secs(30));
        // Server guidance is used as is, even above the backoff cap.
        assert_eq!(p.wait(Some(Duration::from_secs(45)), 0), Duration::from_secs(45));
    }

    #[test]
    fn jitter_is_bounded() {
        let p = RetryPolicy::default();
        for _ in 0..100 {
            assert!(p.jitter() <= p.max_jitter);
        }
        let huge = RetryPolicy {
            max_jitter: Duration::MAX,
            initial_backoff: Duration::MAX,
            ..RetryPolicy::default()
        };
        let _ = huge.jitter();
        assert_eq!(huge.wait(None, 5), huge.max_backoff);
    }
}
