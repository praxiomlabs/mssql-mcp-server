//! Tool input types with JSON Schema generation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

/// Output format for query results.
///
/// This enum provides type-safe handling of output formats instead of raw strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExecuteQueryInput {
    /// The SQL query to execute.
    #[schemars(description = "SQL query to execute")]
    pub query: String,

    /// Maximum number of rows to return (optional, uses server default if not specified).
    #[serde(default)]
    #[schemars(description = "Maximum number of rows to return (default: server configured limit)")]
    pub max_rows: Option<usize>,

    /// Query timeout in seconds (optional, uses server default if not specified).
    #[serde(default)]
    #[schemars(description = "Query timeout in seconds (default: server configured timeout)")]
    pub timeout_seconds: Option<u64>,

    /// Output format for query results.
    #[serde(default)]
    #[schemars(description = "Output format: 'table' (markdown), 'json', or 'csv' (default: table)")]
    pub format: OutputFormat,
}

/// Input for the `execute_procedure` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExecuteProcedureInput {
    /// Schema name of the stored procedure (default: dbo).
    #[serde(default = "default_schema")]
    #[schemars(description = "Schema name (default: dbo)")]
    pub schema: String,

    /// Name of the stored procedure.
    #[schemars(description = "Name of the stored procedure to execute")]
    pub procedure: String,

    /// Parameters to pass to the procedure as a JSON object.
    #[serde(default)]
    #[schemars(description = "Parameters as key-value pairs")]
    pub parameters: HashMap<String, serde_json::Value>,

    /// Query timeout in seconds.
    #[serde(default)]
    #[schemars(description = "Execution timeout in seconds")]
    pub timeout_seconds: Option<u64>,
}

fn default_schema() -> String {
    "dbo".to_string()
}

/// Input for the `execute_async` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExecuteAsyncInput {
    /// The SQL query to execute asynchronously.
    #[schemars(description = "SQL query to execute asynchronously")]
    pub query: String,

    /// Maximum number of rows to return.
    #[serde(default)]
    #[schemars(description = "Maximum number of rows to return")]
    pub max_rows: Option<usize>,
}

/// Input for the `get_session_status` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetSessionStatusInput {
    /// Session ID returned from execute_async.
    #[schemars(description = "Session ID from execute_async")]
    pub session_id: String,

    /// Whether to include query results if completed.
    #[serde(default = "default_true")]
    #[schemars(description = "Include query results if completed (default: true)")]
    pub include_results: bool,
}

fn default_true() -> bool {
    true
}

/// Input for the `cancel_session` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CancelSessionInput {
    /// Session ID to cancel.
    #[schemars(description = "Session ID to cancel")]
    pub session_id: String,
}

/// Input for the `explain_query` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExplainQueryInput {
    /// The SQL query to explain.
    #[schemars(description = "SQL query to analyze")]
    pub query: String,

    /// Type of plan: "estimated" or "actual".
    #[serde(default = "default_plan_type")]
    #[schemars(description = "Plan type: 'estimated' or 'actual' (default: estimated)")]
    pub plan_type: String,
}

fn default_plan_type() -> String {
    "estimated".to_string()
}

/// Input for the `list_sessions` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListSessionsInput {
    /// Filter by status: "running", "completed", "failed", "cancelled", or "all".
    #[serde(default = "default_status_filter")]
    #[schemars(description = "Filter by status: 'running', 'completed', 'failed', 'cancelled', or 'all' (default: all)")]
    pub status: String,
}

fn default_status_filter() -> String {
    "all".to_string()
}

/// Input for the `health_check` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HealthCheckInput {
    /// Whether to include detailed diagnostics.
    #[serde(default)]
    #[schemars(description = "Include detailed diagnostics (pool stats, server version, etc.)")]
    pub detailed: bool,
}

/// Input for the `set_timeout` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetTimeoutInput {
    /// New default timeout in seconds.
    #[schemars(description = "New default query timeout in seconds (1-3600)")]
    pub timeout_seconds: u64,
}

/// Input for the `get_timeout` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetTimeoutInput {
    /// Include timeout history if available.
    #[serde(default)]
    #[schemars(description = "Include additional timeout configuration details")]
    pub detailed: bool,
}

/// Input for the `get_session_results` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetSessionResultsInput {
    /// Session ID returned from execute_async.
    #[schemars(description = "Session ID from execute_async")]
    pub session_id: String,

    /// Output format for query results.
    #[serde(default)]
    #[schemars(description = "Output format: 'table' (markdown), 'json', or 'csv' (default: table)")]
    pub format: OutputFormat,

    /// Maximum number of rows to return from the results.
    #[serde(default)]
    #[schemars(description = "Maximum rows to return (default: all available)")]
    pub max_rows: Option<usize>,
}

// =========================================================================
// Parameterized Query Inputs
// =========================================================================

/// Input for the `execute_parameterized` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExecuteParameterizedInput {
    /// The SQL query with parameter placeholders (@p1, @p2, etc. or named @param).
    #[schemars(description = "SQL query with parameter placeholders (@p1, @p2, @name, etc.)")]
    pub query: String,

    /// Parameters as key-value pairs (parameter name to value).
    #[serde(default)]
    #[schemars(description = "Parameters as key-value pairs. Keys are parameter names (with or without @), values are the parameter values.")]
    pub parameters: HashMap<String, Value>,

    /// Maximum number of rows to return.
    #[serde(default)]
    #[schemars(description = "Maximum number of rows to return (default: server configured limit)")]
    pub max_rows: Option<usize>,

    /// Output format for query results.
    #[serde(default)]
    #[schemars(description = "Output format: 'table' (markdown), 'json', or 'csv' (default: table)")]
    pub format: OutputFormat,
}

// =========================================================================
// Transaction Control Inputs
// =========================================================================

/// Input for the `begin_transaction` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BeginTransactionInput {
    /// Optional transaction name.
    #[serde(default)]
    #[schemars(description = "Optional name for the transaction")]
    pub name: Option<String>,

    /// Isolation level for the transaction.
    #[serde(default = "default_isolation_level")]
    #[schemars(description = "Transaction isolation level: 'read_uncommitted', 'read_committed', 'repeatable_read', 'serializable', 'snapshot' (default: read_committed)")]
    pub isolation_level: String,
}

fn default_isolation_level() -> String {
    "read_committed".to_string()
}

/// Input for the `commit_transaction` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitTransactionInput {
    /// Transaction ID returned from begin_transaction.
    #[schemars(description = "Transaction ID from begin_transaction")]
    pub transaction_id: String,
}

/// Input for the `rollback_transaction` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RollbackTransactionInput {
    /// Transaction ID returned from begin_transaction.
    #[schemars(description = "Transaction ID from begin_transaction")]
    pub transaction_id: String,

    /// Optional savepoint name to rollback to.
    #[serde(default)]
    #[schemars(description = "Optional savepoint name to rollback to (if not specified, rolls back entire transaction)")]
    pub savepoint: Option<String>,
}

/// Input for the `execute_in_transaction` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExecuteInTransactionInput {
    /// Transaction ID returned from begin_transaction.
    #[schemars(description = "Transaction ID from begin_transaction")]
    pub transaction_id: String,

    /// The SQL statement to execute within the transaction.
    #[schemars(description = "SQL statement to execute within the transaction")]
    pub query: String,

    /// Parameters for parameterized query execution.
    #[serde(default)]
    #[schemars(description = "Parameters as key-value pairs for parameterized execution")]
    pub parameters: HashMap<String, Value>,
}

// =========================================================================
// Pagination Inputs
// =========================================================================

/// Input for the `execute_paginated` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExecutePaginatedInput {
    /// The SQL query to paginate (must include ORDER BY clause).
    #[schemars(description = "SQL query to paginate (must include ORDER BY clause for consistent ordering)")]
    pub query: String,

    /// Number of rows per page.
    #[serde(default = "default_page_size")]
    #[schemars(description = "Number of rows per page (default: 100, max: 10000)")]
    pub page_size: usize,

    /// Page number (1-based) or cursor from previous result.
    #[serde(default)]
    #[schemars(description = "Page number (1-based) for offset pagination, or cursor token from previous result")]
    pub page: Option<PaginationPosition>,

    /// Output format for query results.
    #[serde(default)]
    #[schemars(description = "Output format: 'table' (markdown), 'json', or 'csv' (default: table)")]
    pub format: OutputFormat,
}

fn default_page_size() -> usize {
    100
}

/// Pagination position - either page number or cursor.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PaginationPosition {
    /// Page number (1-based).
    PageNumber(usize),
    /// Cursor token from previous result.
    Cursor(String),
}

// =========================================================================
// Database Switching Input
// =========================================================================

/// Input for the `switch_database` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SwitchDatabaseInput {
    /// Name of the database to switch to.
    #[schemars(description = "Name of the database to switch to")]
    pub database: String,
}

// =========================================================================
// Index Recommendation Input
// =========================================================================

/// Input for the `recommend_indexes` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RecommendIndexesInput {
    /// The SQL query to analyze for index recommendations.
    #[schemars(description = "SQL query to analyze for missing index recommendations")]
    pub query: String,

    /// Whether to include existing indexes in the analysis.
    #[serde(default = "default_true")]
    #[schemars(description = "Include information about existing indexes (default: true)")]
    pub include_existing: bool,
}

// =========================================================================
// Schema Diff Input
// =========================================================================

/// Input for the `compare_schemas` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompareSchemaInput {
    /// Source schema for comparison.
    #[schemars(description = "Source schema name to compare from")]
    pub source_schema: String,

    /// Target schema for comparison.
    #[schemars(description = "Target schema name to compare to")]
    pub target_schema: String,

    /// Types of objects to compare.
    #[serde(default = "default_object_types")]
    #[schemars(description = "Object types to compare: 'tables', 'views', 'procedures', 'all' (default: all)")]
    pub object_types: String,
}

fn default_object_types() -> String {
    "all".to_string()
}

/// Input for the `compare_tables` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompareTablesInput {
    /// Source table (schema.table format).
    #[schemars(description = "Source table in schema.table format")]
    pub source_table: String,

    /// Target table (schema.table format).
    #[schemars(description = "Target table in schema.table format")]
    pub target_table: String,

    /// Whether to compare indexes.
    #[serde(default = "default_true")]
    #[schemars(description = "Compare indexes between tables (default: true)")]
    pub compare_indexes: bool,

    /// Whether to compare constraints.
    #[serde(default = "default_true")]
    #[schemars(description = "Compare constraints between tables (default: true)")]
    pub compare_constraints: bool,
}

// =========================================================================
// Data Sampling Input
// =========================================================================

/// Input for the `sample_data` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SampleDataInput {
    /// Table to sample from (schema.table format).
    #[schemars(description = "Table to sample from in schema.table format")]
    pub table: String,

    /// Number of rows to sample.
    #[serde(default = "default_sample_size")]
    #[schemars(description = "Number of rows to sample (default: 100)")]
    pub sample_size: usize,

    /// Sampling method.
    #[serde(default = "default_sampling_method")]
    #[schemars(description = "Sampling method: 'random', 'top', 'bottom', 'stratified' (default: random)")]
    pub method: String,

    /// Column to stratify by (for stratified sampling).
    #[serde(default)]
    #[schemars(description = "Column to stratify by (required for stratified sampling)")]
    pub stratify_column: Option<String>,

    /// Optional WHERE clause filter.
    #[serde(default)]
    #[schemars(description = "Optional WHERE clause to filter rows before sampling (without 'WHERE' keyword)")]
    pub filter: Option<String>,

    /// Output format for sample results.
    #[serde(default)]
    #[schemars(description = "Output format: 'table' (markdown), 'json', or 'csv' (default: table)")]
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
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BulkInsertInput {
    /// Target table (schema.table format).
    #[schemars(description = "Target table in schema.table format")]
    pub table: String,

    /// Column names for the insert.
    #[schemars(description = "Column names to insert into (in order)")]
    pub columns: Vec<String>,

    /// Rows of data to insert (array of arrays).
    #[schemars(description = "Array of rows, where each row is an array of values matching the columns")]
    pub rows: Vec<Vec<Value>>,

    /// Batch size for chunked inserts.
    #[serde(default = "default_batch_size")]
    #[schemars(description = "Number of rows per INSERT batch (default: 1000)")]
    pub batch_size: usize,
}

fn default_batch_size() -> usize {
    1000
}

/// Input for the `export_data` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportDataInput {
    /// Query to export results from.
    #[schemars(description = "SQL SELECT query to export results from")]
    pub query: String,

    /// Export format for the data.
    #[serde(default)]
    #[schemars(description = "Export format: 'csv', 'json', 'json_lines' (default: csv)")]
    pub format: ExportFormat,

    /// Include headers in CSV output.
    #[serde(default = "default_true")]
    #[schemars(description = "Include column headers in CSV output (default: true)")]
    pub include_headers: bool,

    /// Maximum rows to export.
    #[serde(default)]
    #[schemars(description = "Maximum rows to export (default: no limit)")]
    pub max_rows: Option<usize>,
}

// =========================================================================
// Server Metrics Input
// =========================================================================

/// Input for the `get_metrics` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetMetricsInput {
    /// Categories of metrics to retrieve.
    #[serde(default = "default_metrics_categories")]
    #[schemars(description = "Metric categories: 'connections', 'queries', 'performance', 'memory', 'all' (default: all)")]
    pub categories: String,

    /// Time range for query statistics (in minutes).
    #[serde(default = "default_metrics_range")]
    #[schemars(description = "Time range in minutes for query statistics (default: 60)")]
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
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AnalyzeQueryInput {
    /// The SQL query to analyze.
    #[schemars(description = "SQL query to analyze for performance and optimization")]
    pub query: String,

    /// Include table statistics in analysis.
    #[serde(default = "default_true")]
    #[schemars(description = "Include table statistics in the analysis (default: true)")]
    pub include_statistics: bool,

    /// Include index usage analysis.
    #[serde(default = "default_true")]
    #[schemars(description = "Include index usage analysis (default: true)")]
    pub include_index_analysis: bool,
}

// =========================================================================
// Cache Management Inputs
// =========================================================================

/// Input for the `get_cache_stats` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetCacheStatsInput {
    /// Whether to include detailed entry information.
    #[serde(default)]
    #[schemars(description = "Include detailed cache entry information (default: false)")]
    pub detailed: bool,
}

/// Input for the `clear_cache` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClearCacheInput {
    /// Optional pattern to match entries to clear.
    #[serde(default)]
    #[schemars(description = "Optional pattern to match entries to clear (clears all if not specified)")]
    pub pattern: Option<String>,
}

/// Input for the `execute_cached` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExecuteCachedInput {
    /// The SQL query to execute.
    #[schemars(description = "SQL SELECT query to execute (only SELECT queries are cached)")]
    pub query: String,

    /// Maximum number of rows to return.
    #[serde(default)]
    #[schemars(description = "Maximum number of rows to return (default: server configured limit)")]
    pub max_rows: Option<usize>,

    /// Custom TTL in seconds for this query's cache entry.
    #[serde(default)]
    #[schemars(description = "Custom cache TTL in seconds (default: server configured TTL)")]
    pub ttl_seconds: Option<u64>,

    /// Force refresh the cache (bypass existing cached result).
    #[serde(default)]
    #[schemars(description = "Force refresh the cache, ignoring existing cached result (default: false)")]
    pub force_refresh: bool,

    /// Output format for query results.
    #[serde(default)]
    #[schemars(description = "Output format: 'table' (markdown), 'json', or 'csv' (default: table)")]
    pub format: OutputFormat,
}

// =========================================================================
// Pool Metrics Input
// =========================================================================

/// Input for the `get_pool_metrics` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetPoolMetricsInput {
    /// Whether to include connection history.
    #[serde(default)]
    #[schemars(description = "Include connection history and trends (default: false)")]
    pub include_history: bool,
}

// =========================================================================
// Internal Server Metrics Input
// =========================================================================

/// Input for the `get_internal_metrics` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetInternalMetricsInput {
    /// Include calculated rates (queries/sec, hit rate, etc).
    #[serde(default = "default_true")]
    #[schemars(description = "Include calculated rates and averages (default: true)")]
    pub include_rates: bool,
}
