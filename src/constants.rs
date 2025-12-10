//! Centralized constants for the MSSQL MCP Server.
//!
//! This module contains all magic numbers and default values used throughout
//! the codebase, making them easy to find, understand, and modify.

use std::time::Duration;

// =============================================================================
// Timeout Constants
// =============================================================================

/// Default connection timeout in seconds.
pub const DEFAULT_CONNECTION_TIMEOUT_SECS: u64 = 30;

/// Default query timeout in seconds.
pub const DEFAULT_QUERY_TIMEOUT_SECS: u64 = 30;

/// Default HTTP request timeout in seconds.
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;

/// Default connection timeout as Duration.
pub const DEFAULT_CONNECTION_TIMEOUT: Duration = Duration::from_secs(DEFAULT_CONNECTION_TIMEOUT_SECS);

/// Default query timeout as Duration.
pub const DEFAULT_QUERY_TIMEOUT: Duration = Duration::from_secs(DEFAULT_QUERY_TIMEOUT_SECS);

// =============================================================================
// Connection Pool Constants
// =============================================================================

/// Default minimum connections in pool.
pub const DEFAULT_MIN_CONNECTIONS: u32 = 1;

/// Default maximum connections in pool.
pub const DEFAULT_MAX_CONNECTIONS: u32 = 10;

/// Default connection idle timeout in seconds.
pub const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 600;

// =============================================================================
// Result Size Constants
// =============================================================================

/// Default maximum result rows.
pub const DEFAULT_MAX_RESULT_ROWS: usize = 10_000;

/// Maximum allowed page size for pagination.
pub const MAX_PAGE_SIZE: usize = 10_000;

/// Minimum allowed page size for pagination.
pub const MIN_PAGE_SIZE: usize = 1;

/// Default page size for pagination.
pub const DEFAULT_PAGE_SIZE: usize = 100;

/// Default sample size for data sampling.
pub const DEFAULT_SAMPLE_SIZE: usize = 100;

/// Maximum sample size for data sampling.
pub const MAX_SAMPLE_SIZE: usize = 10_000;

/// Default batch size for bulk inserts.
pub const DEFAULT_BATCH_SIZE: usize = 1000;

// =============================================================================
// Cache Constants
// =============================================================================

/// Default cache TTL in seconds.
pub const DEFAULT_CACHE_TTL_SECS: u64 = 60;

/// Default cache TTL as Duration.
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(DEFAULT_CACHE_TTL_SECS);

/// Default maximum cache size in MB.
pub const DEFAULT_CACHE_MAX_SIZE_MB: usize = 100;

/// Default maximum cache entries.
pub const DEFAULT_CACHE_MAX_ENTRIES: usize = 1000;

// =============================================================================
// Shutdown Constants
// =============================================================================

/// Default shutdown drain timeout in seconds.
pub const DEFAULT_DRAIN_TIMEOUT_SECS: u64 = 30;

/// Default shutdown force timeout in seconds.
pub const DEFAULT_FORCE_TIMEOUT_SECS: u64 = 10;

/// Default shutdown drain timeout as Duration.
pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(DEFAULT_DRAIN_TIMEOUT_SECS);

/// Default shutdown force timeout as Duration.
pub const DEFAULT_FORCE_TIMEOUT: Duration = Duration::from_secs(DEFAULT_FORCE_TIMEOUT_SECS);

/// Sleep interval during drain phase.
pub const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(500);

// =============================================================================
// Session and State Constants
// =============================================================================

/// Default session/state cleanup interval in seconds.
pub const DEFAULT_CLEANUP_INTERVAL_SECS: u64 = 60;

/// Default cleanup interval as Duration.
pub const DEFAULT_CLEANUP_INTERVAL: Duration = Duration::from_secs(DEFAULT_CLEANUP_INTERVAL_SECS);

/// Maximum session limit.
pub const DEFAULT_SESSION_LIMIT: usize = 100;

/// Maximum transaction limit.
pub const DEFAULT_TRANSACTION_LIMIT: usize = 50;

/// Session progress when complete.
pub const SESSION_PROGRESS_COMPLETE: u8 = 100;

// =============================================================================
// Rate Limiting Constants
// =============================================================================

/// Default rate limit in requests per minute.
pub const DEFAULT_RATE_LIMIT_RPM: u32 = 100;

// =============================================================================
// Telemetry Constants
// =============================================================================

/// Default metrics export interval in seconds.
pub const DEFAULT_EXPORT_INTERVAL_SECS: u64 = 60;

// =============================================================================
// Logging Constants
// =============================================================================

/// Default truncation length for query logging.
pub const LOG_QUERY_TRUNCATE_LENGTH: usize = 100;

/// Extended truncation length for detailed logging.
pub const LOG_QUERY_EXTENDED_TRUNCATE_LENGTH: usize = 500;

// =============================================================================
// Metrics Query Constants
// =============================================================================

/// Default time range for metrics queries in minutes.
pub const DEFAULT_METRICS_TIME_RANGE_MINUTES: u64 = 60;

// =============================================================================
// Percentage Constants (for calculations)
// =============================================================================

/// Percentage multiplier for rate calculations.
pub const PERCENTAGE_MULTIPLIER: f64 = 100.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeout_durations() {
        assert_eq!(DEFAULT_CONNECTION_TIMEOUT, Duration::from_secs(30));
        assert_eq!(DEFAULT_QUERY_TIMEOUT, Duration::from_secs(30));
    }

    #[test]
    fn test_cache_durations() {
        assert_eq!(DEFAULT_CACHE_TTL, Duration::from_secs(60));
        assert_eq!(DEFAULT_CLEANUP_INTERVAL, Duration::from_secs(60));
    }

    #[test]
    fn test_shutdown_durations() {
        assert_eq!(DEFAULT_DRAIN_TIMEOUT, Duration::from_secs(30));
        assert_eq!(DEFAULT_FORCE_TIMEOUT, Duration::from_secs(10));
    }

    #[test]
    fn test_page_size_bounds() {
        assert!(DEFAULT_PAGE_SIZE >= MIN_PAGE_SIZE);
        assert!(DEFAULT_PAGE_SIZE <= MAX_PAGE_SIZE);
    }

    #[test]
    fn test_sample_size_bounds() {
        assert!(DEFAULT_SAMPLE_SIZE <= MAX_SAMPLE_SIZE);
    }
}
