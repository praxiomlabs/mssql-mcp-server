//! Tool input types with JSON Schema generation.

use mcpkit::ToolInput;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

/// Output format for query results.
///
/// This enum provides type-safe handling of output formats instead of raw strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Markdown table format (default).
    #[default]
    Table,
    /// JSON format.
    Json,
    /// CSV format.
    Csv,
}

impl OutputFormat {
    /// Get the format name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            OutputFormat::Table => "table",
            OutputFormat::Json => "json",
            OutputFormat::Csv => "csv",
        }
    }

    /// Generate JSON Schema for this type.
    pub fn tool_input_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "string",
            "enum": ["table", "json", "csv"],
            "default": "table",
            "description": "Output format: 'table' (markdown), 'json', or 'csv'"
        })
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for OutputFormat {
    type Err = InvalidOutputFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "table" | "markdown" => Ok(OutputFormat::Table),
            "json" => Ok(OutputFormat::Json),
            "csv" => Ok(OutputFormat::Csv),
            _ => Err(InvalidOutputFormatError(s.to_string())),
        }
    }
}

/// Error returned when parsing an invalid output format string.
#[derive(Debug, Clone)]
pub struct InvalidOutputFormatError(String);

impl fmt::Display for InvalidOutputFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Invalid output format '{}'. Valid formats: table, json, csv",
            self.0
        )
    }
}

impl std::error::Error for InvalidOutputFormatError {}

/// Export format for bulk data operations.
///
/// Similar to OutputFormat but includes additional formats suitable for data export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// CSV format (default).
    #[default]
    Csv,
    /// JSON array format.
    Json,
    /// JSON Lines format (one JSON object per line).
    JsonLines,
}

impl ExportFormat {
    /// Get the format name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Json => "json",
            ExportFormat::JsonLines => "json_lines",
        }
    }

    /// Generate JSON Schema for this type.
    pub fn tool_input_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "string",
            "enum": ["csv", "json", "json_lines"],
            "default": "csv",
            "description": "Export format: 'csv', 'json', or 'json_lines'"
        })
    }
}

impl fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for ExportFormat {
    type Err = InvalidExportFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "csv" => Ok(ExportFormat::Csv),
            "json" => Ok(ExportFormat::Json),
            "json_lines" | "jsonlines" | "jsonl" => Ok(ExportFormat::JsonLines),
            _ => Err(InvalidExportFormatError(s.to_string())),
        }
    }
}

/// Error returned when parsing an invalid export format string.
#[derive(Debug, Clone)]
pub struct InvalidExportFormatError(String);

impl fmt::Display for InvalidExportFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Invalid export format '{}'. Valid formats: csv, json, json_lines",
            self.0
        )
    }
}

impl std::error::Error for InvalidExportFormatError {}

/// Input for the `execute_query` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct ExecuteQueryInput {
    /// SQL query to execute.
    pub query: String,

    /// Maximum number of rows to return (default: server configured limit).
    #[serde(default)]
    pub max_rows: Option<usize>,

    /// Query timeout in seconds (default: server configured timeout).
    #[serde(default)]
    pub timeout_seconds: Option<u64>,

    /// Output format: 'table' (markdown), 'json', or 'csv' (default: table).
    #[serde(default)]
    pub format: OutputFormat,
}

/// Input for the `execute_procedure` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct ExecuteProcedureInput {
    /// Schema name (default: dbo).
    #[serde(default = "default_schema")]
    pub schema: String,

    /// Name of the stored procedure to execute.
    pub procedure: String,

    /// Parameters as key-value pairs.
    #[serde(default)]
    pub parameters: HashMap<String, serde_json::Value>,

    /// Execution timeout in seconds.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

fn default_schema() -> String {
    "dbo".to_string()
}

/// Input for the `execute_async` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct ExecuteAsyncInput {
    /// SQL query to execute asynchronously.
    pub query: String,

    /// Maximum number of rows to return.
    #[serde(default)]
    pub max_rows: Option<usize>,

    /// Per-query timeout in seconds. Overrides the global timeout for this query.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

/// Input for the `get_session_status` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct GetSessionStatusInput {
    /// Session ID from execute_async.
    pub session_id: String,

    /// Include query results if completed (default: true).
    #[serde(default = "default_true")]
    pub include_results: bool,
}

fn default_true() -> bool {
    true
}

/// Input for the `cancel_session` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct CancelSessionInput {
    /// Session ID to cancel.
    pub session_id: String,
}

/// Input for the `explain_query` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct ExplainQueryInput {
    /// SQL query to analyze.
    pub query: String,

    /// Plan type: 'estimated' or 'actual' (default: estimated).
    #[serde(default = "default_plan_type")]
    pub plan_type: String,
}

fn default_plan_type() -> String {
    "estimated".to_string()
}

/// Input for the `list_sessions` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct ListSessionsInput {
    /// Filter by status: 'running', 'completed', 'failed', 'cancelled', or 'all' (default: all).
    #[serde(default = "default_status_filter")]
    pub status: String,
}

fn default_status_filter() -> String {
    "all".to_string()
}

/// Input for the `health_check` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct HealthCheckInput {
    /// Include detailed diagnostics (pool stats, server version, etc.).
    #[serde(default)]
    pub detailed: bool,
}

/// Input for the `set_timeout` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct SetTimeoutInput {
    /// New default query timeout in seconds (1-3600).
    pub timeout_seconds: u64,
}

/// Input for the `get_timeout` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct GetTimeoutInput {
    /// Include additional timeout configuration details.
    #[serde(default)]
    pub detailed: bool,
}

/// Input for the `get_session_results` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct GetSessionResultsInput {
    /// Session ID from execute_async.
    pub session_id: String,

    /// Output format: 'table' (markdown), 'json', or 'csv' (default: table).
    #[serde(default)]
    pub format: OutputFormat,

    /// Maximum rows to return (default: all available).
    #[serde(default)]
    pub max_rows: Option<usize>,
}

// =========================================================================
// Parameterized Query Inputs
// =========================================================================

/// Input for the `execute_parameterized` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct ExecuteParameterizedInput {
    /// SQL query with parameter placeholders (@p1, @p2, @name, etc.).
    pub query: String,

    /// Parameters as key-value pairs. Keys are parameter names (with or without @), values are the parameter values.
    #[serde(default)]
    pub parameters: HashMap<String, Value>,

    /// Maximum number of rows to return (default: server configured limit).
    #[serde(default)]
    pub max_rows: Option<usize>,

    /// Output format: 'table' (markdown), 'json', or 'csv' (default: table).
    #[serde(default)]
    pub format: OutputFormat,
}

// =========================================================================
// Transaction Control Inputs
// =========================================================================

/// Input for the `begin_transaction` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct BeginTransactionInput {
    /// Optional name for the transaction.
    #[serde(default)]
    pub name: Option<String>,

    /// Transaction isolation level: 'read_uncommitted', 'read_committed', 'repeatable_read', 'serializable', 'snapshot' (default: read_committed).
    #[serde(default = "default_isolation_level")]
    pub isolation_level: String,
}

fn default_isolation_level() -> String {
    "read_committed".to_string()
}

/// Input for the `commit_transaction` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct CommitTransactionInput {
    /// Transaction ID from begin_transaction.
    pub transaction_id: String,
}

/// Input for the `rollback_transaction` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct RollbackTransactionInput {
    /// Transaction ID from begin_transaction.
    pub transaction_id: String,

    /// Optional savepoint name to rollback to (if not specified, rolls back entire transaction).
    #[serde(default)]
    pub savepoint: Option<String>,
}

/// Input for the `execute_in_transaction` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct ExecuteInTransactionInput {
    /// Transaction ID from begin_transaction.
    pub transaction_id: String,

    /// SQL statement to execute within the transaction.
    pub query: String,

    /// Parameters as key-value pairs for parameterized execution.
    #[serde(default)]
    pub parameters: HashMap<String, Value>,
}

// =========================================================================
// Pagination Inputs
// =========================================================================

/// Input for the `execute_paginated` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct ExecutePaginatedInput {
    /// SQL query to paginate (must include ORDER BY clause for consistent ordering).
    pub query: String,

    /// Number of rows per page (default: 100, max: 10000).
    #[serde(default = "default_page_size")]
    pub page_size: usize,

    /// Page number (1-based) for offset pagination, or cursor token from previous result.
    #[serde(default)]
    pub page: Option<PaginationPosition>,

    /// Output format: 'table' (markdown), 'json', or 'csv' (default: table).
    #[serde(default)]
    pub format: OutputFormat,
}

fn default_page_size() -> usize {
    100
}

/// Pagination position - either page number or cursor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PaginationPosition {
    /// Page number (1-based).
    PageNumber(usize),
    /// Cursor token from previous result.
    Cursor(String),
}

impl PaginationPosition {
    /// Generate JSON Schema for this type.
    pub fn tool_input_schema() -> serde_json::Value {
        serde_json::json!({
            "oneOf": [
                {"type": "integer", "description": "Page number (1-based)"},
                {"type": "string", "description": "Cursor token from previous result"}
            ],
            "description": "Page number (1-based) or cursor token from previous result"
        })
    }
}

// =========================================================================
// Database Switching Input
// =========================================================================

/// Input for the `switch_database` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct SwitchDatabaseInput {
    /// Name of the database to switch to.
    pub database: String,
}

// =========================================================================
// Index Recommendation Input
// =========================================================================

/// Input for the `recommend_indexes` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct RecommendIndexesInput {
    /// SQL query to analyze for missing index recommendations.
    pub query: String,

    /// Include information about existing indexes (default: true).
    #[serde(default = "default_true")]
    pub include_existing: bool,
}

// =========================================================================
// Schema Diff Input
// =========================================================================

/// Input for the `compare_schemas` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct CompareSchemaInput {
    /// Source schema name to compare from.
    pub source_schema: String,

    /// Target schema name to compare to.
    pub target_schema: String,

    /// Object types to compare: 'tables', 'views', 'procedures', 'all' (default: all).
    #[serde(default = "default_object_types")]
    pub object_types: String,
}

fn default_object_types() -> String {
    "all".to_string()
}

/// Input for the `compare_tables` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct CompareTablesInput {
    /// Source table in schema.table format.
    pub source_table: String,

    /// Target table in schema.table format.
    pub target_table: String,

    /// Compare indexes between tables (default: true).
    #[serde(default = "default_true")]
    pub compare_indexes: bool,

    /// Compare constraints between tables (default: true).
    #[serde(default = "default_true")]
    pub compare_constraints: bool,
}

// =========================================================================
// Data Sampling Input
// =========================================================================

/// Input for the `sample_data` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct SampleDataInput {
    /// Table to sample from in schema.table format.
    pub table: String,

    /// Number of rows to sample (default: 100).
    #[serde(default = "default_sample_size")]
    pub sample_size: usize,

    /// Sampling method: 'random', 'top', 'bottom', 'stratified' (default: random).
    #[serde(default = "default_sampling_method")]
    pub method: String,

    /// Column to stratify by (required for stratified sampling).
    #[serde(default)]
    pub stratify_column: Option<String>,

    /// Optional WHERE clause to filter rows before sampling (without 'WHERE' keyword).
    #[serde(default)]
    pub filter: Option<String>,

    /// Output format: 'table' (markdown), 'json', or 'csv' (default: table).
    #[serde(default)]
    pub format: OutputFormat,
}

fn default_sample_size() -> usize {
    100
}

fn default_sampling_method() -> String {
    "random".to_string()
}

// =========================================================================
// Bulk Operations Inputs
// =========================================================================

/// Input for the `bulk_insert` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct BulkInsertInput {
    /// Target table in schema.table format.
    pub table: String,

    /// Column names to insert into (in order).
    pub columns: Vec<String>,

    /// Array of rows, where each row is an array of values matching the columns.
    pub rows: Vec<Vec<Value>>,

    /// Number of rows per INSERT batch (default: 1000).
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

fn default_batch_size() -> usize {
    1000
}

/// Input for the `export_data` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct ExportDataInput {
    /// SQL SELECT query to export results from.
    pub query: String,

    /// Export format: 'csv', 'json', 'json_lines' (default: csv).
    #[serde(default)]
    pub format: ExportFormat,

    /// Include column headers in CSV output (default: true).
    #[serde(default = "default_true")]
    pub include_headers: bool,

    /// Maximum rows to export (default: no limit).
    #[serde(default)]
    pub max_rows: Option<usize>,
}

// =========================================================================
// Server Metrics Input
// =========================================================================

/// Input for the `get_metrics` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct GetMetricsInput {
    /// Metric categories: 'connections', 'queries', 'performance', 'memory', 'all' (default: all).
    #[serde(default = "default_metrics_categories")]
    pub categories: String,

    /// Time range in minutes for query statistics (default: 60).
    #[serde(default = "default_metrics_range")]
    pub time_range_minutes: u64,
}

fn default_metrics_categories() -> String {
    "all".to_string()
}

fn default_metrics_range() -> u64 {
    60
}

// =========================================================================
// Query Analysis Input
// =========================================================================

/// Input for the `analyze_query` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct AnalyzeQueryInput {
    /// SQL query to analyze for performance and optimization.
    pub query: String,

    /// Include table statistics in the analysis (default: true).
    #[serde(default = "default_true")]
    pub include_statistics: bool,

    /// Include index usage analysis (default: true).
    #[serde(default = "default_true")]
    pub include_index_analysis: bool,
}

// =========================================================================
// Cache Management Inputs
// =========================================================================

/// Input for the `get_cache_stats` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct GetCacheStatsInput {
    /// Include detailed cache entry information (default: false).
    #[serde(default)]
    pub detailed: bool,
}

/// Input for the `clear_cache` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct ClearCacheInput {
    /// Optional pattern to match entries to clear (clears all if not specified).
    #[serde(default)]
    pub pattern: Option<String>,
}

/// Input for the `execute_cached` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct ExecuteCachedInput {
    /// SQL SELECT query to execute (only SELECT queries are cached).
    pub query: String,

    /// Maximum number of rows to return (default: server configured limit).
    #[serde(default)]
    pub max_rows: Option<usize>,

    /// Custom cache TTL in seconds (default: server configured TTL).
    #[serde(default)]
    pub ttl_seconds: Option<u64>,

    /// Force refresh the cache, ignoring existing cached result (default: false).
    #[serde(default)]
    pub force_refresh: bool,

    /// Output format: 'table' (markdown), 'json', or 'csv' (default: table).
    #[serde(default)]
    pub format: OutputFormat,
}

// =========================================================================
// Pool Metrics Input
// =========================================================================

/// Input for the `get_pool_metrics` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct GetPoolMetricsInput {
    /// Include connection history and trends (default: false).
    #[serde(default)]
    pub include_history: bool,
}

// =========================================================================
// Internal Server Metrics Input
// =========================================================================

/// Input for the `get_internal_metrics` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct GetInternalMetricsInput {
    /// Include calculated rates and averages (default: true).
    #[serde(default = "default_true")]
    pub include_rates: bool,
}

// =========================================================================
// Pinned Session Inputs
// =========================================================================

/// Input for the `begin_pinned_session` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct BeginPinnedSessionInput {
    /// Optional name for the pinned session (for identification).
    #[serde(default)]
    pub name: Option<String>,
}

/// Input for the `execute_in_pinned_session` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct ExecuteInPinnedSessionInput {
    /// Session ID from begin_pinned_session.
    pub session_id: String,

    /// SQL statement to execute within the pinned session.
    pub query: String,

    /// Output format: 'table' (markdown), 'json', or 'csv' (default: table).
    #[serde(default)]
    pub format: OutputFormat,
}

/// Input for the `end_pinned_session` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct EndPinnedSessionInput {
    /// Session ID from begin_pinned_session.
    pub session_id: String,
}

/// Input for the `list_pinned_sessions` tool.
#[derive(Debug, Clone, Serialize, Deserialize, ToolInput)]
pub struct ListPinnedSessionsInput {
    /// Include detailed statistics for each session (default: false).
    #[serde(default)]
    pub detailed: bool,
}
