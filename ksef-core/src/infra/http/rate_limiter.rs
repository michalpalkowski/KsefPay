use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::error::KSeFError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RateLimitCategory {
    Auth,
    Session,
    Invoice,
    Query,
    PublicKey,
    TestData,
    Default,
}

#[derive(Debug, Clone, Copy)]
pub struct RateLimitThresholds {
    pub per_second: usize,
    pub per_minute: usize,
    pub per_hour: usize,
    pub burst: usize,
}

impl Default for RateLimitThresholds {
    fn default() -> Self {
        Self {
            per_second: 10,
            per_minute: 300,
            per_hour: 10_000,
            burst: 5,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RateLimitConfig {
    pub default: RateLimitThresholds,
    pub categories: HashMap<RateLimitCategory, RateLimitThresholds>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitStatus {
    pub second_used: usize,
    pub second_limit: usize,
    pub minute_used: usize,
    pub minute_limit: usize,
    pub hour_used: usize,
    pub hour_limit: usize,
}

#[derive(Default, Debug)]
struct WindowCounters {
    second: VecDeque<Instant>,
    minute: VecDeque<Instant>,
    hour: VecDeque<Instant>,
}

#[derive(Debug, Clone)]
pub struct TokenBucketRateLimiter {
    config: Arc<Mutex<RateLimitConfig>>,
    buckets: Arc<Mutex<HashMap<RateLimitCategory, WindowCounters>>>,
}

impl Default for TokenBucketRateLimiter {
    fn default() -> Self {
        Self::new(RateLimitConfig::default())
    }
}

impl TokenBucketRateLimiter {
    #[must_use]
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config: Arc::new(Mutex::new(config)),
            buckets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn acquire(&self, category: RateLimitCategory) -> Result<(), KSeFError> {
        let now = Instant::now();
        let thresholds = self.thresholds_for(category).await;

        let mut buckets = self.buckets.lock().await;
        let counters = buckets.entry(category).or_default();

        prune_expired(counters, now);

        let second_limit = thresholds.per_second.saturating_add(thresholds.burst);
        let second_exceeded = counters.second.len() >= second_limit;
        let minute_exceeded = counters.minute.len() >= thresholds.per_minute;
        let hour_exceeded = counters.hour.len() >= thresholds.per_hour;

        if second_exceeded || minute_exceeded || hour_exceeded {
            let retry_after_ms = compute_retry_after_ms(
                now,
                counters,
                second_exceeded,
                minute_exceeded,
                hour_exceeded,
            );
            return Err(KSeFError::RateLimited { retry_after_ms });
        }

        counters.second.push_back(now);
        counters.minute.push_back(now);
        counters.hour.push_back(now);

        Ok(())
    }

    pub async fn status(&self, category: RateLimitCategory) -> RateLimitStatus {
        let now = Instant::now();
        let thresholds = self.thresholds_for(category).await;

        let mut buckets = self.buckets.lock().await;
        let counters = buckets.entry(category).or_default();
        prune_expired(counters, now);

        RateLimitStatus {
            second_used: counters.second.len(),
            second_limit: thresholds.per_second.saturating_add(thresholds.burst),
            minute_used: counters.minute.len(),
            minute_limit: thresholds.per_minute,
            hour_used: counters.hour.len(),
            hour_limit: thresholds.per_hour,
        }
    }

    pub async fn update_category_limits(
        &self,
        category: RateLimitCategory,
        thresholds: RateLimitThresholds,
    ) {
        let mut config = self.config.lock().await;
        config.categories.insert(category, thresholds);
    }

    async fn thresholds_for(&self, category: RateLimitCategory) -> RateLimitThresholds {
        let config = self.config.lock().await;
        config
            .categories
            .get(&category)
            .copied()
            .unwrap_or(config.default)
    }
}

fn prune_expired(counters: &mut WindowCounters, now: Instant) {
    prune_window(&mut counters.second, now, Duration::from_secs(1));
    prune_window(&mut counters.minute, now, Duration::from_secs(60));
    prune_window(&mut counters.hour, now, Duration::from_secs(3600));
}

fn prune_window(window: &mut VecDeque<Instant>, now: Instant, duration: Duration) {
    while let Some(front) = window.front() {
        if now.duration_since(*front) >= duration {
            window.pop_front();
        } else {
            break;
        }
    }
}

fn compute_retry_after_ms(
    now: Instant,
    counters: &WindowCounters,
    second_exceeded: bool,
    minute_exceeded: bool,
    hour_exceeded: bool,
) -> u64 {
    let mut waits = Vec::new();

    if second_exceeded && let Some(front) = counters.second.front() {
        waits.push(remaining_ms(now, *front, Duration::from_secs(1)));
    }
    if minute_exceeded && let Some(front) = counters.minute.front() {
        waits.push(remaining_ms(now, *front, Duration::from_secs(60)));
    }
    if hour_exceeded && let Some(front) = counters.hour.front() {
        waits.push(remaining_ms(now, *front, Duration::from_secs(3600)));
    }

    waits.into_iter().max().unwrap_or(1).max(1)
}

fn remaining_ms(now: Instant, start: Instant, window: Duration) -> u64 {
    let elapsed = now.duration_since(start);
    let remaining = window.checked_sub(elapsed).unwrap_or_default();
    if remaining.is_zero() {
        return 1;
    }
    u64::try_from(remaining.as_millis()).unwrap_or(1).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strict_thresholds() -> RateLimitThresholds {
        RateLimitThresholds {
            per_second: 2,
            per_minute: 5,
            per_hour: 10,
            burst: 0,
        }
    }

    fn limiter_with_thresholds(thresholds: RateLimitThresholds) -> TokenBucketRateLimiter {
        let mut categories = HashMap::new();
        categories.insert(RateLimitCategory::Auth, thresholds);
        categories.insert(RateLimitCategory::Session, thresholds);
        categories.insert(RateLimitCategory::Invoice, thresholds);

        TokenBucketRateLimiter::new(RateLimitConfig {
            default: thresholds,
            categories,
        })
    }

    #[tokio::test(start_paused = true)]
    async fn acquire_succeeds_under_limit() {
        let limiter = limiter_with_thresholds(strict_thresholds());

        limiter.acquire(RateLimitCategory::Auth).await.unwrap();
        limiter.acquire(RateLimitCategory::Auth).await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn acquire_returns_rate_limited_when_exceeded() {
        let limiter = limiter_with_thresholds(strict_thresholds());

        limiter.acquire(RateLimitCategory::Auth).await.unwrap();
        limiter.acquire(RateLimitCategory::Auth).await.unwrap();

        let err = limiter.acquire(RateLimitCategory::Auth).await.unwrap_err();
        match err {
            KSeFError::RateLimited { retry_after_ms } => assert!(retry_after_ms >= 1),
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn window_resets_after_time() {
        let limiter = limiter_with_thresholds(strict_thresholds());

        limiter.acquire(RateLimitCategory::Auth).await.unwrap();
        limiter.acquire(RateLimitCategory::Auth).await.unwrap();
        assert!(matches!(
            limiter.acquire(RateLimitCategory::Auth).await,
            Err(KSeFError::RateLimited { .. })
        ));

        tokio::time::advance(Duration::from_secs(1)).await;
        limiter.acquire(RateLimitCategory::Auth).await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn per_category_isolation_works() {
        let limiter = limiter_with_thresholds(strict_thresholds());

        limiter.acquire(RateLimitCategory::Auth).await.unwrap();
        limiter.acquire(RateLimitCategory::Auth).await.unwrap();
        assert!(matches!(
            limiter.acquire(RateLimitCategory::Auth).await,
            Err(KSeFError::RateLimited { .. })
        ));

        limiter.acquire(RateLimitCategory::Session).await.unwrap();
        limiter.acquire(RateLimitCategory::Session).await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn burst_allows_extra_requests() {
        let mut config = RateLimitConfig::default();
        config.default = RateLimitThresholds {
            per_second: 1,
            per_minute: 5,
            per_hour: 10,
            burst: 2,
        };
        let limiter = TokenBucketRateLimiter::new(config);

        limiter.acquire(RateLimitCategory::Default).await.unwrap();
        limiter.acquire(RateLimitCategory::Default).await.unwrap();
        limiter.acquire(RateLimitCategory::Default).await.unwrap();

        assert!(matches!(
            limiter.acquire(RateLimitCategory::Default).await,
            Err(KSeFError::RateLimited { .. })
        ));
    }
}
