use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Simple fixed-window limiter keyed by client IP.
///
/// Keeps up to `limit` requests within `window`. When limit is exceeded,
/// returns Retry-After seconds.
#[derive(Clone)]
pub struct AuthRateLimiter {
    inner: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    limit: usize,
    window: Duration,
}

impl AuthRateLimiter {
    #[must_use]
    pub fn new(limit: usize, window: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            limit,
            window,
        }
    }

    /// Check and record a request.
    ///
    /// Returns `None` when request is allowed, or `Some(retry_after_seconds)`
    /// when the limit has been reached.
    pub fn check(&self, key: &str) -> Option<u64> {
        let now = Instant::now();
        let mut map = self.inner.lock().expect("auth rate limiter lock");
        let hits = map.entry(key.to_string()).or_default();

        hits.retain(|ts| now.duration_since(*ts) < self.window);

        if hits.len() >= self.limit {
            let oldest = hits[0];
            let retry = self
                .window
                .saturating_sub(now.duration_since(oldest))
                .as_secs()
                .max(1);
            return Some(retry);
        }

        hits.push(now);
        None
    }
}

impl Default for AuthRateLimiter {
    fn default() -> Self {
        Self::new(10, Duration::from_secs(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_eleventh_request_in_window() {
        let limiter = AuthRateLimiter::new(10, Duration::from_secs(60));

        for _ in 0..10 {
            assert!(limiter.check("127.0.0.1").is_none());
        }

        let retry_after = limiter.check("127.0.0.1");
        assert!(retry_after.is_some());
        assert!(retry_after.unwrap_or_default() >= 1);
    }

    #[test]
    fn uses_separate_buckets_per_ip() {
        let limiter = AuthRateLimiter::new(1, Duration::from_secs(60));

        assert!(limiter.check("10.0.0.1").is_none());
        assert!(limiter.check("10.0.0.1").is_some());

        // Different client key should not be affected.
        assert!(limiter.check("10.0.0.2").is_none());
    }
}
