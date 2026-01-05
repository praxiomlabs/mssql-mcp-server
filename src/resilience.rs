//! Resilience patterns for handling transient failures.
//!
//! This module provides:
//! - Retry logic with configurable backoff strategies
//! - Circuit breaker pattern for fail-fast behavior
//! - Bulkhead pattern (future)

use crate::error::ServerError;
use parking_lot::RwLock;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_attempts: u32,
    /// Initial delay between retries.
    pub initial_delay: Duration,
    /// Maximum delay between retries.
    pub max_delay: Duration,
    /// Backoff multiplier (for exponential backoff).
    pub multiplier: f64,
    /// Whether to add jitter to delays.
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            multiplier: 2.0,
            jitter: true,
        }
    }
}

impl RetryConfig {
    /// Create a retry config with no retries (just execute once).
    pub fn no_retry() -> Self {
        Self {
            max_attempts: 1,
            ..Default::default()
        }
    }

    /// Create a retry config optimized for database operations.
    pub fn database() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            multiplier: 2.0,
            jitter: true,
        }
    }

    /// Create a retry config optimized for connection establishment.
    pub fn connection() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            jitter: true,
        }
    }

    /// Create configuration from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(attempts) = std::env::var("MSSQL_RETRY_MAX_ATTEMPTS") {
            if let Ok(n) = attempts.parse() {
                config.max_attempts = n;
            }
        }

        if let Ok(delay) = std::env::var("MSSQL_RETRY_INITIAL_DELAY_MS") {
            if let Ok(ms) = delay.parse() {
                config.initial_delay = Duration::from_millis(ms);
            }
        }

        if let Ok(delay) = std::env::var("MSSQL_RETRY_MAX_DELAY_MS") {
            if let Ok(ms) = delay.parse() {
                config.max_delay = Duration::from_millis(ms);
            }
        }

        if let Ok(mult) = std::env::var("MSSQL_RETRY_MULTIPLIER") {
            if let Ok(m) = mult.parse() {
                config.multiplier = m;
            }
        }

        config
    }

    /// Calculate the delay for a given attempt number (0-indexed).
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }

        let base_delay_ms = self.initial_delay.as_millis() as f64
            * self.multiplier.powi(attempt.saturating_sub(1) as i32);

        let capped_delay_ms = base_delay_ms.min(self.max_delay.as_millis() as f64);

        let final_delay_ms = if self.jitter {
            // Add +/- 25% jitter
            let jitter_factor = 0.75 + (rand_jitter() * 0.5);
            capped_delay_ms * jitter_factor
        } else {
            capped_delay_ms
        };

        Duration::from_millis(final_delay_ms as u64)
    }
}

/// Simple pseudo-random jitter factor between 0.0 and 1.0.
///
/// Uses a simple approach based on system time to avoid adding rand dependency.
fn rand_jitter() -> f64 {
    use std::time::SystemTime;

    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);

    (nanos as f64) / (u32::MAX as f64)
}

/// Result of a retry operation.
#[derive(Debug)]
pub struct RetryResult<T> {
    /// The successful result, if any.
    pub value: Option<T>,
    /// Number of attempts made.
    pub attempts: u32,
    /// Total time spent (including delays).
    pub total_duration: Duration,
    /// The last error, if the operation failed.
    pub last_error: Option<ServerError>,
}

impl<T> RetryResult<T> {
    /// Check if the operation succeeded.
    pub fn is_success(&self) -> bool {
        self.value.is_some()
    }

    /// Convert to a standard Result.
    pub fn into_result(self) -> Result<T, ServerError> {
        match self.value {
            Some(v) => Ok(v),
            None => Err(self
                .last_error
                .unwrap_or_else(|| ServerError::internal("Retry failed with no error captured"))),
        }
    }
}

/// Execute an async operation with retry logic.
///
/// The operation will be retried if it returns an error and the error
/// is considered transient (according to `ServerError::is_transient()`).
///
/// # Example
///
/// ```ignore
/// let config = RetryConfig::database();
/// let result = retry_async(&config, || async {
///     execute_query("SELECT 1").await
/// }).await;
/// ```
pub async fn retry_async<F, Fut, T>(config: &RetryConfig, operation: F) -> RetryResult<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, ServerError>>,
{
    let start = std::time::Instant::now();
    let mut last_error = None;

    for attempt in 0..config.max_attempts {
        // Apply delay before retry (not on first attempt)
        if attempt > 0 {
            let delay = config.delay_for_attempt(attempt);
            debug!("Retry attempt {} after {:?} delay", attempt + 1, delay);
            sleep(delay).await;
        }

        match operation().await {
            Ok(value) => {
                return RetryResult {
                    value: Some(value),
                    attempts: attempt + 1,
                    total_duration: start.elapsed(),
                    last_error: None,
                };
            }
            Err(e) => {
                if !e.is_transient() {
                    // Non-transient error, don't retry
                    debug!("Non-transient error, not retrying: {}", e);
                    return RetryResult {
                        value: None,
                        attempts: attempt + 1,
                        total_duration: start.elapsed(),
                        last_error: Some(e),
                    };
                }

                warn!(
                    "Transient error on attempt {}/{}: {}",
                    attempt + 1,
                    config.max_attempts,
                    e
                );
                last_error = Some(e);
            }
        }
    }

    RetryResult {
        value: None,
        attempts: config.max_attempts,
        total_duration: start.elapsed(),
        last_error,
    }
}

/// Simple retry wrapper that returns Result.
///
/// This is a convenience function that wraps `retry_async` and returns
/// a standard Result.
pub async fn with_retry<F, Fut, T>(config: &RetryConfig, operation: F) -> Result<T, ServerError>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, ServerError>>,
{
    retry_async(config, operation).await.into_result()
}

// =============================================================================
// Circuit Breaker Pattern
// =============================================================================

/// Configuration for circuit breaker behavior.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before opening the circuit.
    pub failure_threshold: u32,
    /// Number of consecutive successes needed to close the circuit from half-open.
    pub success_threshold: u32,
    /// Duration to keep the circuit open before transitioning to half-open.
    pub reset_timeout: Duration,
    /// Maximum number of requests allowed in half-open state.
    pub half_open_max_requests: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 3,
            reset_timeout: Duration::from_secs(30),
            half_open_max_requests: 3,
        }
    }
}

impl CircuitBreakerConfig {
    /// Create configuration optimized for database connections.
    pub fn database() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 2,
            reset_timeout: Duration::from_secs(30),
            half_open_max_requests: 2,
        }
    }

    /// Create a more aggressive configuration for critical paths.
    pub fn aggressive() -> Self {
        Self {
            failure_threshold: 3,
            success_threshold: 2,
            reset_timeout: Duration::from_secs(60),
            half_open_max_requests: 1,
        }
    }

    /// Create configuration from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(threshold) = std::env::var("MSSQL_CB_FAILURE_THRESHOLD") {
            if let Ok(n) = threshold.parse() {
                config.failure_threshold = n;
            }
        }

        if let Ok(threshold) = std::env::var("MSSQL_CB_SUCCESS_THRESHOLD") {
            if let Ok(n) = threshold.parse() {
                config.success_threshold = n;
            }
        }

        if let Ok(timeout) = std::env::var("MSSQL_CB_RESET_TIMEOUT_SECS") {
            if let Ok(secs) = timeout.parse() {
                config.reset_timeout = Duration::from_secs(secs);
            }
        }

        if let Ok(max) = std::env::var("MSSQL_CB_HALF_OPEN_MAX_REQUESTS") {
            if let Ok(n) = max.parse() {
                config.half_open_max_requests = n;
            }
        }

        config
    }
}

/// Circuit breaker states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Circuit is closed, requests flow normally.
    Closed,
    /// Circuit is open, requests fail immediately.
    Open,
    /// Circuit is half-open, limited requests are allowed to test recovery.
    HalfOpen,
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CircuitState::Closed => write!(f, "closed"),
            CircuitState::Open => write!(f, "open"),
            CircuitState::HalfOpen => write!(f, "half-open"),
        }
    }
}

/// Internal state for the circuit breaker.
struct CircuitBreakerState {
    state: CircuitState,
    failure_count: u32,
    success_count: u32,
    last_failure_time: Option<Instant>,
    half_open_requests: u32,
}

impl CircuitBreakerState {
    fn new() -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            success_count: 0,
            last_failure_time: None,
            half_open_requests: 0,
        }
    }
}

/// Circuit breaker for protecting against cascading failures.
///
/// The circuit breaker pattern prevents a failing operation from being
/// repeatedly attempted, allowing the system to recover.
///
/// # States
///
/// - **Closed**: Normal operation. Failures are counted.
/// - **Open**: Circuit is tripped. Requests fail immediately without attempting
///   the operation. After `reset_timeout`, transitions to half-open.
/// - **Half-Open**: Limited requests are allowed through to test if the
///   underlying issue is resolved. On success, closes. On failure, re-opens.
///
/// # Example
///
/// ```ignore
/// let breaker = CircuitBreaker::new(CircuitBreakerConfig::default());
///
/// let result = breaker.call(|| async {
///     execute_database_query().await
/// }).await;
/// ```
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: RwLock<CircuitBreakerState>,
    total_calls: AtomicU64,
    total_successes: AtomicU64,
    total_failures: AtomicU64,
    total_rejections: AtomicU64,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with the given configuration.
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: RwLock::new(CircuitBreakerState::new()),
            total_calls: AtomicU64::new(0),
            total_successes: AtomicU64::new(0),
            total_failures: AtomicU64::new(0),
            total_rejections: AtomicU64::new(0),
        }
    }

    /// Create a new circuit breaker with default configuration.
    pub fn default_config() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }

    /// Get the current state of the circuit breaker.
    pub fn state(&self) -> CircuitState {
        let state = self.state.read();
        self.effective_state(&state)
    }

    /// Get statistics about the circuit breaker.
    pub fn stats(&self) -> CircuitBreakerStats {
        let state = self.state.read();
        CircuitBreakerStats {
            state: self.effective_state(&state),
            total_calls: self.total_calls.load(Ordering::Relaxed),
            total_successes: self.total_successes.load(Ordering::Relaxed),
            total_failures: self.total_failures.load(Ordering::Relaxed),
            total_rejections: self.total_rejections.load(Ordering::Relaxed),
            consecutive_failures: state.failure_count,
            consecutive_successes: state.success_count,
        }
    }

    /// Execute an async operation through the circuit breaker.
    ///
    /// Returns `Err(ServerError::circuit_open())` if the circuit is open.
    pub async fn call<F, Fut, T>(&self, operation: F) -> Result<T, ServerError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, ServerError>>,
    {
        self.total_calls.fetch_add(1, Ordering::Relaxed);

        // Check if we're allowed to proceed
        if !self.should_allow_request() {
            self.total_rejections.fetch_add(1, Ordering::Relaxed);
            debug!("Circuit breaker rejecting request (circuit open)");
            return Err(ServerError::circuit_open(self.config.reset_timeout.as_secs()));
        }

        // Execute the operation
        match operation().await {
            Ok(result) => {
                self.record_success();
                Ok(result)
            }
            Err(e) => {
                // Only count transient errors as failures
                if e.is_transient() {
                    self.record_failure();
                }
                Err(e)
            }
        }
    }

    /// Check if a request should be allowed through.
    fn should_allow_request(&self) -> bool {
        let mut state = self.state.write();
        let effective = self.effective_state(&state);

        match effective {
            CircuitState::Closed => true,
            CircuitState::Open => false,
            CircuitState::HalfOpen => {
                // Allow limited requests in half-open state
                if state.half_open_requests < self.config.half_open_max_requests {
                    state.half_open_requests += 1;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Get the effective state, considering timeout transitions.
    fn effective_state(&self, state: &CircuitBreakerState) -> CircuitState {
        match state.state {
            CircuitState::Open => {
                // Check if we should transition to half-open
                if let Some(last_failure) = state.last_failure_time {
                    if last_failure.elapsed() >= self.config.reset_timeout {
                        return CircuitState::HalfOpen;
                    }
                }
                CircuitState::Open
            }
            other => other,
        }
    }

    /// Record a successful operation.
    fn record_success(&self) {
        self.total_successes.fetch_add(1, Ordering::Relaxed);

        let mut state = self.state.write();
        let effective = self.effective_state(&state);

        match effective {
            CircuitState::Closed => {
                // Reset failure count on success
                state.failure_count = 0;
            }
            CircuitState::HalfOpen => {
                state.success_count += 1;
                debug!(
                    "Circuit breaker half-open: {}/{} successes",
                    state.success_count, self.config.success_threshold
                );

                // Check if we can close the circuit
                if state.success_count >= self.config.success_threshold {
                    info!("Circuit breaker closing after successful recovery");
                    state.state = CircuitState::Closed;
                    state.failure_count = 0;
                    state.success_count = 0;
                    state.half_open_requests = 0;
                    state.last_failure_time = None;
                }
            }
            CircuitState::Open => {
                // This shouldn't happen, but handle it gracefully
                warn!("Unexpected success recorded while circuit is open");
            }
        }
    }

    /// Record a failed operation.
    fn record_failure(&self) {
        self.total_failures.fetch_add(1, Ordering::Relaxed);

        let mut state = self.state.write();
        let effective = self.effective_state(&state);

        match effective {
            CircuitState::Closed => {
                state.failure_count += 1;
                state.success_count = 0;
                state.last_failure_time = Some(Instant::now());

                debug!(
                    "Circuit breaker closed: {}/{} failures",
                    state.failure_count, self.config.failure_threshold
                );

                // Check if we should open the circuit
                if state.failure_count >= self.config.failure_threshold {
                    warn!(
                        "Circuit breaker opening after {} consecutive failures",
                        state.failure_count
                    );
                    state.state = CircuitState::Open;
                }
            }
            CircuitState::HalfOpen => {
                // Any failure in half-open state re-opens the circuit
                warn!("Circuit breaker re-opening after failure in half-open state");
                state.state = CircuitState::Open;
                state.failure_count = self.config.failure_threshold;
                state.success_count = 0;
                state.half_open_requests = 0;
                state.last_failure_time = Some(Instant::now());
            }
            CircuitState::Open => {
                // Update last failure time
                state.last_failure_time = Some(Instant::now());
            }
        }
    }

    /// Manually reset the circuit breaker to closed state.
    pub fn reset(&self) {
        let mut state = self.state.write();
        info!("Circuit breaker manually reset to closed state");
        state.state = CircuitState::Closed;
        state.failure_count = 0;
        state.success_count = 0;
        state.half_open_requests = 0;
        state.last_failure_time = None;
    }
}

/// Statistics from a circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreakerStats {
    /// Current state of the circuit.
    pub state: CircuitState,
    /// Total number of calls attempted.
    pub total_calls: u64,
    /// Total number of successful calls.
    pub total_successes: u64,
    /// Total number of failed calls.
    pub total_failures: u64,
    /// Total number of calls rejected due to open circuit.
    pub total_rejections: u64,
    /// Current consecutive failure count.
    pub consecutive_failures: u32,
    /// Current consecutive success count (in half-open).
    pub consecutive_successes: u32,
}

impl CircuitBreakerStats {
    /// Calculate the success rate (0.0 to 1.0).
    pub fn success_rate(&self) -> f64 {
        let completed = self.total_successes + self.total_failures;
        if completed == 0 {
            1.0
        } else {
            self.total_successes as f64 / completed as f64
        }
    }

    /// Calculate the rejection rate (0.0 to 1.0).
    pub fn rejection_rate(&self) -> f64 {
        if self.total_calls == 0 {
            0.0
        } else {
            self.total_rejections as f64 / self.total_calls as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn test_retry_config_defaults() {
        let config = RetryConfig::default();
        assert_eq!(config.max_attempts, 3);
        assert!(config.jitter);
    }

    #[test]
    fn test_retry_config_no_retry() {
        let config = RetryConfig::no_retry();
        assert_eq!(config.max_attempts, 1);
    }

    #[test]
    fn test_delay_calculation() {
        let config = RetryConfig {
            max_attempts: 5,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            multiplier: 2.0,
            jitter: false,
        };

        // First attempt has no delay
        assert_eq!(config.delay_for_attempt(0), Duration::ZERO);

        // Subsequent attempts have exponential delay
        assert_eq!(config.delay_for_attempt(1), Duration::from_millis(100));
        assert_eq!(config.delay_for_attempt(2), Duration::from_millis(200));
        assert_eq!(config.delay_for_attempt(3), Duration::from_millis(400));
    }

    #[test]
    fn test_delay_cap() {
        let config = RetryConfig {
            max_attempts: 10,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(5),
            multiplier: 10.0,
            jitter: false,
        };

        // After a few attempts, delay should be capped
        let delay = config.delay_for_attempt(5);
        assert_eq!(delay, Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_retry_success_first_attempt() {
        let config = RetryConfig::default();
        let counter = AtomicU32::new(0);

        let result = retry_async(&config, || async {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ServerError>("success")
        })
        .await;

        assert!(result.is_success());
        assert_eq!(result.attempts, 1);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_success_after_failures() {
        let config = RetryConfig {
            max_attempts: 5,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(10),
            multiplier: 2.0,
            jitter: false,
        };
        let counter = AtomicU32::new(0);

        let result = retry_async(&config, || {
            let count = counter.fetch_add(1, Ordering::SeqCst) + 1;
            async move {
                if count < 3 {
                    Err(ServerError::timeout(30)) // Transient error
                } else {
                    Ok("success")
                }
            }
        })
        .await;

        assert!(result.is_success());
        assert_eq!(result.attempts, 3);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_non_transient_error() {
        let config = RetryConfig::default();
        let counter = AtomicU32::new(0);

        let result = retry_async(&config, || {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Err::<(), _>(ServerError::auth("Invalid credentials")) }
        })
        .await;

        assert!(!result.is_success());
        assert_eq!(result.attempts, 1); // Should not retry
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_exhausted() {
        let config = RetryConfig {
            max_attempts: 3,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(10),
            multiplier: 2.0,
            jitter: false,
        };
        let counter = AtomicU32::new(0);

        let result = retry_async(&config, || {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Err::<(), _>(ServerError::timeout(30)) }
        })
        .await;

        assert!(!result.is_success());
        assert_eq!(result.attempts, 3);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    // =========================================================================
    // Circuit Breaker Tests
    // =========================================================================

    #[test]
    fn test_circuit_breaker_config_defaults() {
        let config = CircuitBreakerConfig::default();
        assert_eq!(config.failure_threshold, 5);
        assert_eq!(config.success_threshold, 3);
        assert_eq!(config.reset_timeout, Duration::from_secs(30));
        assert_eq!(config.half_open_max_requests, 3);
    }

    #[test]
    fn test_circuit_breaker_initial_state() {
        let breaker = CircuitBreaker::default_config();
        assert_eq!(breaker.state(), CircuitState::Closed);

        let stats = breaker.stats();
        assert_eq!(stats.total_calls, 0);
        assert_eq!(stats.total_successes, 0);
        assert_eq!(stats.total_failures, 0);
        assert_eq!(stats.total_rejections, 0);
    }

    #[tokio::test]
    async fn test_circuit_breaker_success() {
        let breaker = CircuitBreaker::default_config();
        let counter = AtomicU32::new(0);

        let result = breaker
            .call(|| async {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<_, ServerError>("success")
            })
            .await;

        assert!(result.is_ok());
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert_eq!(breaker.state(), CircuitState::Closed);

        let stats = breaker.stats();
        assert_eq!(stats.total_calls, 1);
        assert_eq!(stats.total_successes, 1);
        assert_eq!(stats.total_failures, 0);
    }

    #[tokio::test]
    async fn test_circuit_breaker_opens_after_threshold() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            success_threshold: 2,
            reset_timeout: Duration::from_secs(60),
            half_open_max_requests: 1,
        };
        let breaker = CircuitBreaker::new(config);

        // Generate failures to trip the circuit
        for _ in 0..3 {
            let _ = breaker
                .call(|| async { Err::<(), _>(ServerError::timeout(30)) })
                .await;
        }

        // Circuit should now be open
        assert_eq!(breaker.state(), CircuitState::Open);

        // Next call should be rejected without executing
        let result = breaker
            .call(|| async { Ok::<_, ServerError>("this should not execute") })
            .await;

        assert!(result.is_err());
        if let Err(ServerError::CircuitOpen { .. }) = result {
            // Expected
        } else {
            panic!("Expected CircuitOpen error");
        }

        let stats = breaker.stats();
        assert_eq!(stats.total_failures, 3);
        assert_eq!(stats.total_rejections, 1);
    }

    #[tokio::test]
    async fn test_circuit_breaker_success_resets_failures() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            success_threshold: 2,
            reset_timeout: Duration::from_secs(60),
            half_open_max_requests: 1,
        };
        let breaker = CircuitBreaker::new(config);

        // Generate some failures
        for _ in 0..2 {
            let _ = breaker
                .call(|| async { Err::<(), _>(ServerError::timeout(30)) })
                .await;
        }

        // Success should reset failure count
        let _ = breaker.call(|| async { Ok::<_, ServerError>(()) }).await;

        // Two more failures should not trip the circuit (count reset to 0 + 2)
        for _ in 0..2 {
            let _ = breaker
                .call(|| async { Err::<(), _>(ServerError::timeout(30)) })
                .await;
        }

        // Circuit should still be closed
        assert_eq!(breaker.state(), CircuitState::Closed);
    }

    #[test]
    fn test_circuit_breaker_manual_reset() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            success_threshold: 1,
            reset_timeout: Duration::from_secs(60),
            half_open_max_requests: 1,
        };
        let breaker = CircuitBreaker::new(config);

        // Manually manipulate state by recording failures
        {
            let mut state = breaker.state.write();
            state.state = CircuitState::Open;
            state.failure_count = 5;
        }

        assert_eq!(breaker.state(), CircuitState::Open);

        // Reset should restore closed state
        breaker.reset();
        assert_eq!(breaker.state(), CircuitState::Closed);
    }

    #[test]
    fn test_circuit_breaker_stats_rates() {
        let stats = CircuitBreakerStats {
            state: CircuitState::Closed,
            total_calls: 100,
            total_successes: 80,
            total_failures: 20,
            total_rejections: 10,
            consecutive_failures: 0,
            consecutive_successes: 0,
        };

        assert!((stats.success_rate() - 0.8).abs() < 0.001);
        assert!((stats.rejection_rate() - 0.1).abs() < 0.001);
    }

    #[test]
    fn test_circuit_state_display() {
        assert_eq!(CircuitState::Closed.to_string(), "closed");
        assert_eq!(CircuitState::Open.to_string(), "open");
        assert_eq!(CircuitState::HalfOpen.to_string(), "half-open");
    }

    #[tokio::test]
    async fn test_circuit_breaker_non_transient_error_not_counted() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 1,
            reset_timeout: Duration::from_secs(60),
            half_open_max_requests: 1,
        };
        let breaker = CircuitBreaker::new(config);

        // Non-transient errors should not trip the circuit
        for _ in 0..5 {
            let _ = breaker
                .call(|| async { Err::<(), _>(ServerError::auth("bad credentials")) })
                .await;
        }

        // Circuit should still be closed
        assert_eq!(breaker.state(), CircuitState::Closed);

        let stats = breaker.stats();
        assert_eq!(stats.total_failures, 0); // Non-transient not counted as failures
    }
}
