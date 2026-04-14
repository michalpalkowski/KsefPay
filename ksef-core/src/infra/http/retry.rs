use std::future::Future;
use std::time::Duration;

use rand::Rng;

use crate::error::KSeFError;

pub trait RetryableError {
    fn is_retryable(&self) -> bool;

    fn retry_after_ms(&self) -> Option<u64> {
        None
    }
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub multiplier_numerator: u32,
    pub multiplier_denominator: u32,
    pub jitter_percent: u8,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 1000,
            max_delay_ms: 30_000,
            multiplier_numerator: 2,
            multiplier_denominator: 1,
            jitter_percent: 25,
        }
    }
}

impl RetryPolicy {
    #[must_use]
    pub fn next_delay_ms<E: RetryableError>(&self, attempt: u32, error: &E) -> u64 {
        if let Some(retry_after_ms) = error.retry_after_ms() {
            return retry_after_ms.min(self.max_delay_ms);
        }

        let base_delay = self.base_delay_ms(attempt);
        self.apply_jitter(base_delay)
    }

    pub async fn execute<F, Fut, T, E>(&self, mut operation: F) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: RetryableError,
    {
        let mut retries_done = 0u32;

        loop {
            match operation().await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    if !error.is_retryable() || retries_done >= self.max_retries {
                        return Err(error);
                    }

                    let delay_ms = self.next_delay_ms(retries_done, &error);
                    retries_done += 1;
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
            }
        }
    }

    fn apply_jitter(&self, delay_ms: u64) -> u64 {
        if delay_ms == 0 || self.jitter_percent == 0 {
            return delay_ms;
        }

        let lower_percent = 100_u16.saturating_sub(u16::from(self.jitter_percent));
        let upper_percent = 100_u16.saturating_add(u16::from(self.jitter_percent));
        let sampled_percent = rand::thread_rng().gen_range(lower_percent..=upper_percent);

        let jittered = (u128::from(delay_ms) * u128::from(sampled_percent) + 50) / 100;
        let jittered_u64 = u64::try_from(jittered).unwrap_or(self.max_delay_ms);
        jittered_u64.min(self.max_delay_ms)
    }

    fn base_delay_ms(&self, attempt: u32) -> u64 {
        if self.initial_delay_ms == 0 {
            return 0;
        }

        let denominator = u64::from(self.multiplier_denominator.max(1));
        let numerator = u64::from(self.multiplier_numerator.max(1));

        let mut delay = self.initial_delay_ms.min(self.max_delay_ms);
        for _ in 0..attempt {
            if delay >= self.max_delay_ms {
                break;
            }

            let multiplied = u128::from(delay).saturating_mul(u128::from(numerator));
            let rounded_up = (multiplied + u128::from(denominator - 1)) / u128::from(denominator);
            let next_delay = u64::try_from(rounded_up).unwrap_or(self.max_delay_ms);
            delay = next_delay.min(self.max_delay_ms);
        }

        delay
    }
}

impl RetryableError for KSeFError {
    fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimited { .. } => true,
            Self::ApiError(detail) => detail.status_code == 429 || detail.status_code >= 500,
            Self::HttpError { status, .. } => *status == 429 || *status >= 500,
            Self::RequestFailed(err) => err.is_timeout() || err.is_connect() || err.is_request(),
            _ => false,
        }
    }

    fn retry_after_ms(&self) -> Option<u64> {
        match self {
            Self::RateLimited { retry_after_ms } => Some(*retry_after_ms),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::task::yield_now;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestError {
        retryable: bool,
        retry_after_ms: Option<u64>,
    }

    impl RetryableError for TestError {
        fn is_retryable(&self) -> bool {
            self.retryable
        }

        fn retry_after_ms(&self) -> Option<u64> {
            self.retry_after_ms
        }
    }

    fn zero_delay_policy() -> RetryPolicy {
        RetryPolicy {
            max_retries: 3,
            initial_delay_ms: 0,
            max_delay_ms: 0,
            multiplier_numerator: 2,
            multiplier_denominator: 1,
            jitter_percent: 0,
        }
    }

    #[tokio::test]
    async fn execute_succeeds_on_first_try() {
        let policy = zero_delay_policy();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();

        let result: Result<u32, TestError> = policy
            .execute(|| {
                let calls_inner = calls_clone.clone();
                async move {
                    calls_inner.fetch_add(1, Ordering::SeqCst);
                    Ok(42)
                }
            })
            .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn execute_retries_on_retryable_error_then_succeeds() {
        let policy = zero_delay_policy();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();

        let result: Result<u32, TestError> = policy
            .execute(|| {
                let calls_inner = calls_clone.clone();
                async move {
                    let current = calls_inner.fetch_add(1, Ordering::SeqCst);
                    if current < 2 {
                        Err(TestError {
                            retryable: true,
                            retry_after_ms: None,
                        })
                    } else {
                        Ok(7)
                    }
                }
            })
            .await;

        assert_eq!(result.unwrap(), 7);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn execute_stops_after_max_retries() {
        let policy = RetryPolicy {
            max_retries: 2,
            ..zero_delay_policy()
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();

        let result: Result<u32, TestError> = policy
            .execute(|| {
                let calls_inner = calls_clone.clone();
                async move {
                    calls_inner.fetch_add(1, Ordering::SeqCst);
                    Err(TestError {
                        retryable: true,
                        retry_after_ms: None,
                    })
                }
            })
            .await;

        assert_eq!(
            result.unwrap_err(),
            TestError {
                retryable: true,
                retry_after_ms: None
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn execute_does_not_retry_non_retryable_error() {
        let policy = zero_delay_policy();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();

        let result: Result<u32, TestError> = policy
            .execute(|| {
                let calls_inner = calls_clone.clone();
                async move {
                    calls_inner.fetch_add(1, Ordering::SeqCst);
                    Err(TestError {
                        retryable: false,
                        retry_after_ms: None,
                    })
                }
            })
            .await;

        assert_eq!(
            result.unwrap_err(),
            TestError {
                retryable: false,
                retry_after_ms: None
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn execute_respects_retry_after_delay() {
        let policy = RetryPolicy {
            max_retries: 1,
            initial_delay_ms: 0,
            max_delay_ms: 1_000,
            multiplier_numerator: 2,
            multiplier_denominator: 1,
            jitter_percent: 0,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();

        let handle = tokio::spawn(async move {
            policy
                .execute(|| {
                    let calls_inner = calls_clone.clone();
                    async move {
                        let current = calls_inner.fetch_add(1, Ordering::SeqCst);
                        if current == 0 {
                            Err(TestError {
                                retryable: true,
                                retry_after_ms: Some(250),
                            })
                        } else {
                            Ok(1u32)
                        }
                    }
                })
                .await
        });

        yield_now().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        tokio::time::advance(Duration::from_millis(249)).await;
        yield_now().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        tokio::time::advance(Duration::from_millis(1)).await;
        let result = handle.await.unwrap();
        assert_eq!(result.unwrap(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn next_delay_ms_applies_max_bound() {
        let policy = RetryPolicy {
            max_retries: 3,
            initial_delay_ms: 1000,
            max_delay_ms: 1500,
            multiplier_numerator: 3,
            multiplier_denominator: 1,
            jitter_percent: 0,
        };

        let delay = policy.next_delay_ms(
            2,
            &TestError {
                retryable: true,
                retry_after_ms: None,
            },
        );
        assert_eq!(delay, 1500);
    }

    #[test]
    fn ksef_error_retryable_contract() {
        let rate_limited = KSeFError::RateLimited {
            retry_after_ms: 500,
        };
        assert!(rate_limited.is_retryable());
        assert_eq!(rate_limited.retry_after_ms(), Some(500));

        let server_error = KSeFError::HttpError {
            status: 503,
            body: "upstream error".to_string(),
        };
        assert!(server_error.is_retryable());
        assert_eq!(server_error.retry_after_ms(), None);

        let client_error = KSeFError::HttpError {
            status: 400,
            body: "bad request".to_string(),
        };
        assert!(!client_error.is_retryable());
    }
}
