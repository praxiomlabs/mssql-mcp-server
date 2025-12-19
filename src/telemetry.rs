//! OpenTelemetry integration for metrics and tracing export.
//!
//! This module provides observability features including:
//! - Metrics export (query counts, latency, errors)
//! - Distributed tracing integration
//! - Connection pool statistics
//! - Correlation ID tracking for request tracing
//!
//! Requires the `telemetry` feature flag.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Generate a new correlation ID for request tracing.
///
/// Correlation IDs are used to track requests across the system and
/// can be included in logs for debugging and tracing purposes.
pub fn generate_correlation_id() -> String {
    Uuid::new_v4().to_string()
}

/// Generate a short correlation ID (8 characters) for compact logging.
pub fn generate_short_correlation_id() -> String {
    Uuid::new_v4().to_string()[..8].to_string()
}

/// Request context for carrying correlation information through the request lifecycle.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// Unique identifier for this request.
    pub correlation_id: String,
    /// When the request was received.
    pub start_time: Instant,
    /// Client identifier (if available).
    pub client_id: Option<String>,
    /// Tool or resource being accessed.
    pub operation: Option<String>,
}

impl RequestContext {
    /// Create a new request context with a generated correlation ID.
    pub fn new() -> Self {
        Self {
            correlation_id: generate_short_correlation_id(),
            start_time: Instant::now(),
            client_id: None,
            operation: None,
        }
    }

    /// Create a new request context with a specific correlation ID.
    pub fn with_correlation_id(correlation_id: impl Into<String>) -> Self {
        Self {
            correlation_id: correlation_id.into(),
            start_time: Instant::now(),
            client_id: None,
            operation: None,
        }
    }

    /// Set the client identifier.
    pub fn with_client(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = Some(client_id.into());
        self
    }

    /// Set the operation name.
    pub fn with_operation(mut self, operation: impl Into<String>) -> Self {
        self.operation = Some(operation.into());
        self
    }

    /// Get the elapsed time since the request started.
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Create a log prefix for consistent logging format.
    pub fn log_prefix(&self) -> String {
        match (&self.client_id, &self.operation) {
            (Some(client), Some(op)) => format!("[{}] [{}] [{}]", self.correlation_id, client, op),
            (Some(client), None) => format!("[{}] [{}]", self.correlation_id, client),
            (None, Some(op)) => format!("[{}] [{}]", self.correlation_id, op),
            (None, None) => format!("[{}]", self.correlation_id),
        }
    }
}

impl Default for RequestContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Server metrics collection.
///
/// This struct collects metrics that can be exported via OpenTelemetry
/// or retrieved through the API.
#[derive(Debug, Default)]
pub struct ServerMetrics {
    /// Total number of queries executed.
    pub queries_total: AtomicU64,

    /// Total number of successful queries.
    pub queries_success: AtomicU64,

    /// Total number of failed queries.
    pub queries_failed: AtomicU64,

    /// Total query execution time in milliseconds.
    pub query_time_ms_total: AtomicU64,

    /// Number of active connections.
    pub active_connections: AtomicU64,

    /// Total number of connections created.
    pub connections_total: AtomicU64,

    /// Number of connection errors.
    pub connection_errors: AtomicU64,

    /// Total number of transactions started.
    pub transactions_total: AtomicU64,

    /// Number of committed transactions.
    pub transactions_committed: AtomicU64,

    /// Number of rolled back transactions.
    pub transactions_rolled_back: AtomicU64,

    /// Cache hits.
    pub cache_hits: AtomicU64,

    /// Cache misses.
    pub cache_misses: AtomicU64,

    /// Total bytes transferred.
    pub bytes_transferred: AtomicU64,
}

impl ServerMetrics {
    /// Create a new metrics collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a query execution.
    pub fn record_query(&self, success: bool, duration: Duration) {
        self.queries_total.fetch_add(1, Ordering::Relaxed);
        if success {
            self.queries_success.fetch_add(1, Ordering::Relaxed);
        } else {
            self.queries_failed.fetch_add(1, Ordering::Relaxed);
        }
        self.query_time_ms_total
            .fetch_add(duration.as_millis() as u64, Ordering::Relaxed);
    }

    /// Record a transaction start.
    pub fn record_transaction_start(&self) {
        self.transactions_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a transaction commit.
    pub fn record_transaction_commit(&self) {
        self.transactions_committed.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a transaction rollback.
    pub fn record_transaction_rollback(&self) {
        self.transactions_rolled_back
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record a cache hit.
    pub fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a cache miss.
    pub fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Record bytes transferred.
    pub fn record_bytes(&self, bytes: u64) {
        self.bytes_transferred.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Get a snapshot of current metrics.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            queries_total: self.queries_total.load(Ordering::Relaxed),
            queries_success: self.queries_success.load(Ordering::Relaxed),
            queries_failed: self.queries_failed.load(Ordering::Relaxed),
            query_time_ms_total: self.query_time_ms_total.load(Ordering::Relaxed),
            active_connections: self.active_connections.load(Ordering::Relaxed),
            connections_total: self.connections_total.load(Ordering::Relaxed),
            connection_errors: self.connection_errors.load(Ordering::Relaxed),
            transactions_total: self.transactions_total.load(Ordering::Relaxed),
            transactions_committed: self.transactions_committed.load(Ordering::Relaxed),
            transactions_rolled_back: self.transactions_rolled_back.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
            bytes_transferred: self.bytes_transferred.load(Ordering::Relaxed),
        }
    }

    /// Calculate average query time in milliseconds.
    pub fn avg_query_time_ms(&self) -> f64 {
        let total = self.queries_total.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        let time = self.query_time_ms_total.load(Ordering::Relaxed);
        time as f64 / total as f64
    }

    /// Calculate query success rate as a percentage.
    pub fn success_rate(&self) -> f64 {
        let total = self.queries_total.load(Ordering::Relaxed);
        if total == 0 {
            return 100.0;
        }
        let success = self.queries_success.load(Ordering::Relaxed);
        (success as f64 / total as f64) * 100.0
    }

    /// Calculate cache hit rate as a percentage.
    pub fn cache_hit_rate(&self) -> f64 {
        let hits = self.cache_hits.load(Ordering::Relaxed);
        let misses = self.cache_misses.load(Ordering::Relaxed);
        let total = hits + misses;
        if total == 0 {
            return 0.0;
        }
        (hits as f64 / total as f64) * 100.0
    }
}

/// Snapshot of metrics at a point in time.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MetricsSnapshot {
    pub queries_total: u64,
    pub queries_success: u64,
    pub queries_failed: u64,
    pub query_time_ms_total: u64,
    pub active_connections: u64,
    pub connections_total: u64,
    pub connection_errors: u64,
    pub transactions_total: u64,
    pub transactions_committed: u64,
    pub transactions_rolled_back: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub bytes_transferred: u64,
}

impl MetricsSnapshot {
    /// Calculate average query time in milliseconds.
    pub fn avg_query_time_ms(&self) -> f64 {
        if self.queries_total == 0 {
            return 0.0;
        }
        self.query_time_ms_total as f64 / self.queries_total as f64
    }

    /// Calculate query success rate as a percentage.
    pub fn success_rate(&self) -> f64 {
        if self.queries_total == 0 {
            return 100.0;
        }
        (self.queries_success as f64 / self.queries_total as f64) * 100.0
    }

    /// Calculate cache hit rate as a percentage.
    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            return 0.0;
        }
        (self.cache_hits as f64 / total as f64) * 100.0
    }
}

/// Shared metrics type for thread-safe access.
pub type SharedMetrics = Arc<ServerMetrics>;

/// Create a new shared metrics collector.
pub fn new_shared_metrics() -> SharedMetrics {
    Arc::new(ServerMetrics::new())
}

/// Query timer for measuring execution duration.
pub struct QueryTimer {
    start: Instant,
    metrics: SharedMetrics,
}

impl QueryTimer {
    /// Start a new query timer.
    pub fn start(metrics: SharedMetrics) -> Self {
        Self {
            start: Instant::now(),
            metrics,
        }
    }

    /// Stop the timer and record the result.
    pub fn stop(self, success: bool) -> Duration {
        let duration = self.start.elapsed();
        self.metrics.record_query(success, duration);
        duration
    }
}

/// Telemetry configuration.
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Whether telemetry is enabled.
    pub enabled: bool,

    /// OTLP endpoint for exporting metrics.
    pub otlp_endpoint: Option<String>,

    /// Service name for telemetry.
    pub service_name: String,

    /// Export interval in seconds.
    pub export_interval_seconds: u64,

    /// Whether to export metrics.
    pub export_metrics: bool,

    /// Whether to export traces.
    pub export_traces: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            otlp_endpoint: None,
            service_name: "mssql-mcp-server".to_string(),
            export_interval_seconds: 60,
            export_metrics: true,
            export_traces: false,
        }
    }
}

impl TelemetryConfig {
    /// Create configuration from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(enabled) = std::env::var("MSSQL_TELEMETRY_ENABLED") {
            config.enabled = enabled.to_lowercase() == "true" || enabled == "1";
        }

        if let Ok(endpoint) = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
            config.otlp_endpoint = Some(endpoint);
            config.enabled = true; // Enable if endpoint is set
        }

        if let Ok(name) = std::env::var("OTEL_SERVICE_NAME") {
            config.service_name = name;
        }

        if let Ok(interval) = std::env::var("MSSQL_TELEMETRY_INTERVAL") {
            if let Ok(secs) = interval.parse() {
                config.export_interval_seconds = secs;
            }
        }

        if let Ok(metrics) = std::env::var("MSSQL_TELEMETRY_METRICS") {
            config.export_metrics = metrics.to_lowercase() == "true" || metrics == "1";
        }

        if let Ok(traces) = std::env::var("MSSQL_TELEMETRY_TRACES") {
            config.export_traces = traces.to_lowercase() == "true" || traces == "1";
        }

        config
    }
}

/// OpenTelemetry provider (only available with `telemetry` feature).
#[cfg(feature = "telemetry")]
pub mod otel {
    use super::*;
    use opentelemetry::metrics::{Counter, Histogram, MeterProvider, UpDownCounter};
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::runtime::Tokio;
    use std::time::Duration;
    use tracing::info;

    /// OpenTelemetry metrics instruments.
    pub struct OtelMetrics {
        pub queries_total: Counter<u64>,
        pub queries_success: Counter<u64>,
        pub queries_failed: Counter<u64>,
        pub query_duration: Histogram<f64>,
        pub active_connections: UpDownCounter<i64>,
        pub cache_hits: Counter<u64>,
        pub cache_misses: Counter<u64>,
    }

    /// Initialize OpenTelemetry metrics exporter.
    pub fn init_metrics(config: &TelemetryConfig) -> Result<SdkMeterProvider, anyhow::Error> {
        let endpoint = config
            .otlp_endpoint
            .clone()
            .unwrap_or_else(|| "http://localhost:4317".to_string());

        let exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_endpoint(&endpoint)
            .build()?;

        let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter, Tokio)
            .with_interval(Duration::from_secs(config.export_interval_seconds))
            .build();

        let provider = SdkMeterProvider::builder().with_reader(reader).build();

        info!(
            "OpenTelemetry metrics initialized, exporting to {}",
            endpoint
        );

        Ok(provider)
    }

    /// Create OpenTelemetry metrics instruments.
    pub fn create_instruments(provider: &SdkMeterProvider) -> OtelMetrics {
        let meter = provider.meter("mssql-mcp-server");

        OtelMetrics {
            queries_total: meter
                .u64_counter("mssql.queries.total")
                .with_description("Total number of queries executed")
                .build(),
            queries_success: meter
                .u64_counter("mssql.queries.success")
                .with_description("Number of successful queries")
                .build(),
            queries_failed: meter
                .u64_counter("mssql.queries.failed")
                .with_description("Number of failed queries")
                .build(),
            query_duration: meter
                .f64_histogram("mssql.query.duration")
                .with_description("Query execution duration in milliseconds")
                .with_unit("ms")
                .build(),
            active_connections: meter
                .i64_up_down_counter("mssql.connections.active")
                .with_description("Number of active database connections")
                .build(),
            cache_hits: meter
                .u64_counter("mssql.cache.hits")
                .with_description("Number of cache hits")
                .build(),
            cache_misses: meter
                .u64_counter("mssql.cache.misses")
                .with_description("Number of cache misses")
                .build(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_recording() {
        let metrics = ServerMetrics::new();

        metrics.record_query(true, Duration::from_millis(100));
        metrics.record_query(true, Duration::from_millis(200));
        metrics.record_query(false, Duration::from_millis(50));

        assert_eq!(metrics.queries_total.load(Ordering::Relaxed), 3);
        assert_eq!(metrics.queries_success.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.queries_failed.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.query_time_ms_total.load(Ordering::Relaxed), 350);
    }

    #[test]
    fn test_avg_query_time() {
        let metrics = ServerMetrics::new();

        metrics.record_query(true, Duration::from_millis(100));
        metrics.record_query(true, Duration::from_millis(200));

        let avg = metrics.avg_query_time_ms();
        assert!((avg - 150.0).abs() < 0.01);
    }

    #[test]
    fn test_success_rate() {
        let metrics = ServerMetrics::new();

        metrics.record_query(true, Duration::from_millis(100));
        metrics.record_query(true, Duration::from_millis(100));
        metrics.record_query(true, Duration::from_millis(100));
        metrics.record_query(false, Duration::from_millis(100));

        let rate = metrics.success_rate();
        assert!((rate - 75.0).abs() < 0.01);
    }

    #[test]
    fn test_cache_hit_rate() {
        let metrics = ServerMetrics::new();

        metrics.record_cache_hit();
        metrics.record_cache_hit();
        metrics.record_cache_hit();
        metrics.record_cache_miss();

        let rate = metrics.cache_hit_rate();
        assert!((rate - 75.0).abs() < 0.01);
    }

    #[test]
    fn test_metrics_snapshot() {
        let metrics = ServerMetrics::new();

        metrics.record_query(true, Duration::from_millis(100));
        metrics.record_transaction_start();
        metrics.record_transaction_commit();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.queries_total, 1);
        assert_eq!(snapshot.queries_success, 1);
        assert_eq!(snapshot.transactions_total, 1);
        assert_eq!(snapshot.transactions_committed, 1);
    }

    #[test]
    fn test_telemetry_config_defaults() {
        let config = TelemetryConfig::default();
        assert!(!config.enabled);
        assert!(config.otlp_endpoint.is_none());
        assert_eq!(config.service_name, "mssql-mcp-server");
        assert_eq!(config.export_interval_seconds, 60);
    }

    #[test]
    fn test_empty_metrics_rates() {
        let metrics = ServerMetrics::new();

        // Should return 100% success rate when no queries
        assert_eq!(metrics.success_rate(), 100.0);

        // Should return 0% cache hit rate when no cache accesses
        assert_eq!(metrics.cache_hit_rate(), 0.0);

        // Should return 0 avg time when no queries
        assert_eq!(metrics.avg_query_time_ms(), 0.0);
    }

    #[test]
    fn test_correlation_id_generation() {
        let id1 = generate_correlation_id();
        let id2 = generate_correlation_id();

        // IDs should be unique
        assert_ne!(id1, id2);

        // Full UUID format (36 chars with hyphens)
        assert_eq!(id1.len(), 36);
    }

    #[test]
    fn test_short_correlation_id() {
        let short_id = generate_short_correlation_id();

        // Short ID should be 8 characters
        assert_eq!(short_id.len(), 8);
    }

    #[test]
    fn test_request_context() {
        let ctx = RequestContext::new();

        // Should have a short correlation ID
        assert_eq!(ctx.correlation_id.len(), 8);

        // Should have no client or operation
        assert!(ctx.client_id.is_none());
        assert!(ctx.operation.is_none());
    }

    #[test]
    fn test_request_context_with_details() {
        let ctx = RequestContext::new()
            .with_client("test-client")
            .with_operation("execute_query");

        assert!(ctx.client_id.is_some());
        assert!(ctx.operation.is_some());
        assert_eq!(ctx.client_id.unwrap(), "test-client");
        assert_eq!(ctx.operation.unwrap(), "execute_query");
    }

    #[test]
    fn test_request_context_log_prefix() {
        let ctx = RequestContext::with_correlation_id("abc12345")
            .with_client("client1")
            .with_operation("query");

        let prefix = ctx.log_prefix();
        assert_eq!(prefix, "[abc12345] [client1] [query]");

        let ctx_no_op = RequestContext::with_correlation_id("abc12345").with_client("client1");
        assert_eq!(ctx_no_op.log_prefix(), "[abc12345] [client1]");

        let ctx_no_client = RequestContext::with_correlation_id("abc12345").with_operation("query");
        assert_eq!(ctx_no_client.log_prefix(), "[abc12345] [query]");

        let ctx_minimal = RequestContext::with_correlation_id("abc12345");
        assert_eq!(ctx_minimal.log_prefix(), "[abc12345]");
    }
}
