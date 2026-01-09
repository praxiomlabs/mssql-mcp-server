//! Query execution and result handling.

use crate::database::types::{SqlValue, TypeMapper};
use crate::database::ConnectionPool;
use crate::error::ServerError;
use crate::resilience::{RetryConfig, with_retry};
use futures_util::TryStreamExt;
use mssql_client::{TvpColumn, TvpRow, TvpValue};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::timeout;
use tracing::{debug, info};

/// A single row of query results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultRow {
    /// Column values indexed by column name.
    #[serde(flatten)]
    pub columns: HashMap<String, SqlValue>,
}

impl ResultRow {
    /// Create a new result row.
    pub fn new() -> Self {
        Self {
            columns: HashMap::new(),
        }
    }

    /// Get a value by column name.
    pub fn get(&self, column: &str) -> Option<&SqlValue> {
        self.columns.get(column)
    }

    /// Insert a value.
    pub fn insert(&mut self, column: String, value: SqlValue) {
        self.columns.insert(column, value);
    }
}

impl Default for ResultRow {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of executing multiple statements in a transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionBatchResult {
    /// Total number of rows affected across all statements.
    pub total_rows_affected: u64,

    /// Number of statements that executed successfully.
    pub successful_statements: usize,

    /// Total number of statements attempted.
    pub total_statements: usize,

    /// Errors encountered (if continue_on_error was true).
    pub errors: Vec<String>,

    /// Total execution time in milliseconds.
    pub execution_time_ms: u64,
}

/// Result of a query execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    /// Column names in order.
    pub columns: Vec<ColumnInfo>,

    /// Result rows.
    pub rows: Vec<ResultRow>,

    /// Number of rows affected (for INSERT/UPDATE/DELETE).
    pub rows_affected: u64,

    /// Execution time in milliseconds.
    pub execution_time_ms: u64,

    /// Whether results were truncated due to row limit.
    pub truncated: bool,
}

/// Information about a result column.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    /// Column name.
    pub name: String,

    /// SQL type name.
    pub sql_type: String,

    /// Whether the column is nullable.
    pub nullable: bool,
}

impl QueryResult {
    /// Create an empty query result.
    pub fn empty() -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            rows_affected: 0,
            execution_time_ms: 0,
            truncated: false,
        }
    }

    /// Format the result as a markdown table.
    pub fn to_markdown_table(&self) -> String {
        if self.columns.is_empty() {
            if self.rows_affected > 0 {
                return format!(
                    "Query executed successfully. {} row(s) affected.",
                    self.rows_affected
                );
            }
            return "Query executed successfully. No results returned.".to_string();
        }

        let mut output = String::new();

        // Header row
        let headers: Vec<&str> = self.columns.iter().map(|c| c.name.as_str()).collect();
        output.push_str("| ");
        output.push_str(&headers.join(" | "));
        output.push_str(" |\n");

        // Separator row
        output.push_str("| ");
        output.push_str(
            &headers
                .iter()
                .map(|h| "-".repeat(h.len().max(3)))
                .collect::<Vec<_>>()
                .join(" | "),
        );
        output.push_str(" |\n");

        // Data rows
        for row in &self.rows {
            output.push_str("| ");
            let values: Vec<String> = self
                .columns
                .iter()
                .map(|col| {
                    row.get(&col.name)
                        .map(|v| v.to_display_string())
                        .unwrap_or_else(|| "NULL".to_string())
                })
                .collect();
            output.push_str(&values.join(" | "));
            output.push_str(" |\n");
        }

        // Footer
        output.push_str(&format!("\n_{} row(s)_", self.rows.len()));
        if self.truncated {
            output.push_str(" _(truncated)_");
        }
        output.push_str(&format!(" _({} ms)_", self.execution_time_ms));

        output
    }

    /// Format the result as CSV.
    pub fn to_csv(&self) -> String {
        if self.columns.is_empty() {
            return String::new();
        }

        let mut output = String::new();

        // Header row
        let headers: Vec<&str> = self.columns.iter().map(|c| c.name.as_str()).collect();
        output.push_str(&headers.join(","));
        output.push('\n');

        // Data rows
        for row in &self.rows {
            let values: Vec<String> = self
                .columns
                .iter()
                .map(|col| {
                    let value = row
                        .get(&col.name)
                        .map(|v| v.to_display_string())
                        .unwrap_or_default();
                    // Escape CSV values
                    if value.contains(',') || value.contains('"') || value.contains('\n') {
                        format!("\"{}\"", value.replace('"', "\"\""))
                    } else {
                        value
                    }
                })
                .collect();
            output.push_str(&values.join(","));
            output.push('\n');
        }

        output
    }
}

/// Result containing multiple result sets from a single query.
///
/// This is returned when a query contains multiple SELECT statements
/// (e.g., `SELECT 1; SELECT 2;`), each of which produces its own result set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiQueryResult {
    /// Individual result sets from the query.
    pub result_sets: Vec<QueryResult>,

    /// Total execution time in milliseconds.
    pub execution_time_ms: u64,
}

impl MultiQueryResult {
    /// Create an empty multi-query result.
    pub fn empty() -> Self {
        Self {
            result_sets: Vec::new(),
            execution_time_ms: 0,
        }
    }

    /// Create a multi-query result with a single result set.
    pub fn single(result: QueryResult) -> Self {
        let execution_time_ms = result.execution_time_ms;
        Self {
            result_sets: vec![result],
            execution_time_ms,
        }
    }

    /// Get the total number of result sets.
    pub fn result_count(&self) -> usize {
        self.result_sets.len()
    }

    /// Get the total number of rows across all result sets.
    pub fn total_rows(&self) -> usize {
        self.result_sets.iter().map(|r| r.rows.len()).sum()
    }

    /// Check if any result set was truncated.
    pub fn any_truncated(&self) -> bool {
        self.result_sets.iter().any(|r| r.truncated)
    }

    /// Format all result sets as markdown tables.
    pub fn to_markdown_table(&self) -> String {
        if self.result_sets.is_empty() {
            return "Query executed successfully. No results returned.".to_string();
        }

        // If only one result set, delegate to single-result formatting
        if self.result_sets.len() == 1 {
            return self.result_sets[0].to_markdown_table();
        }

        let mut output = String::new();

        for (idx, result) in self.result_sets.iter().enumerate() {
            if idx > 0 {
                output.push_str("\n\n---\n\n");
            }

            output.push_str(&format!("**Result Set {} of {}**\n\n", idx + 1, self.result_sets.len()));

            if result.columns.is_empty() {
                if result.rows_affected > 0 {
                    output.push_str(&format!("{} row(s) affected.", result.rows_affected));
                } else {
                    output.push_str("No results.");
                }
            } else {
                // Header row
                let headers: Vec<&str> = result.columns.iter().map(|c| c.name.as_str()).collect();
                output.push_str("| ");
                output.push_str(&headers.join(" | "));
                output.push_str(" |\n");

                // Separator row
                output.push_str("| ");
                output.push_str(
                    &headers
                        .iter()
                        .map(|h| "-".repeat(h.len().max(3)))
                        .collect::<Vec<_>>()
                        .join(" | "),
                );
                output.push_str(" |\n");

                // Data rows
                for row in &result.rows {
                    output.push_str("| ");
                    let values: Vec<String> = result
                        .columns
                        .iter()
                        .map(|col| {
                            row.get(&col.name)
                                .map(|v| v.to_display_string())
                                .unwrap_or_else(|| "NULL".to_string())
                        })
                        .collect();
                    output.push_str(&values.join(" | "));
                    output.push_str(" |\n");
                }

                output.push_str(&format!("\n_{} row(s)_", result.rows.len()));
                if result.truncated {
                    output.push_str(" _(truncated)_");
                }
            }
        }

        output.push_str(&format!("\n\n_Total: {} result set(s), {} ms_", self.result_sets.len(), self.execution_time_ms));

        output
    }

    /// Format all result sets as CSV (concatenated with blank line separators).
    pub fn to_csv(&self) -> String {
        if self.result_sets.is_empty() {
            return String::new();
        }

        // If only one result set, delegate to single-result formatting
        if self.result_sets.len() == 1 {
            return self.result_sets[0].to_csv();
        }

        self.result_sets
            .iter()
            .map(|r| r.to_csv())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Result of SQL syntax validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Whether the query syntax is valid.
    pub valid: bool,

    /// Error message if syntax is invalid.
    pub error_message: Option<String>,

    /// Line number where the error occurred (if available).
    pub error_line: Option<u32>,

    /// Character position where the error occurred (if available).
    pub error_position: Option<u32>,

    /// Time taken to validate in milliseconds.
    pub validation_time_ms: u64,
}

impl ValidationResult {
    /// Create a successful validation result.
    pub fn success(validation_time_ms: u64) -> Self {
        Self {
            valid: true,
            error_message: None,
            error_line: None,
            error_position: None,
            validation_time_ms,
        }
    }

    /// Create a failed validation result.
    pub fn failure(error_message: String, validation_time_ms: u64) -> Self {
        let error_line = extract_line_number(&error_message);
        Self {
            valid: false,
            error_message: Some(error_message),
            error_line,
            error_position: None,
            validation_time_ms,
        }
    }

    /// Format as a human-readable message.
    pub fn to_message(&self) -> String {
        if self.valid {
            format!("Syntax is valid. ({}ms)", self.validation_time_ms)
        } else {
            let mut msg = String::from("Syntax error: ");
            if let Some(ref error) = self.error_message {
                msg.push_str(error);
            }
            if let Some(line) = self.error_line {
                msg.push_str(&format!("\n  Line: {}", line));
            }
            msg.push_str(&format!("\n  Validated in {}ms", self.validation_time_ms));
            msg
        }
    }
}

/// Extract line number from SQL Server error message.
///
/// SQL Server error messages often contain "Line X" to indicate error location.
fn extract_line_number(error_message: &str) -> Option<u32> {
    // SQL Server error format: "... Line X ..." or "... line X ..."
    use regex::Regex;
    use once_cell::sync::Lazy;

    static LINE_PATTERN: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)\bLine\s+(\d+)\b").expect("Invalid regex pattern for line number extraction")
    });

    LINE_PATTERN.captures(error_message).and_then(|caps| {
        caps.get(1).and_then(|m| m.as_str().parse().ok())
    })
}

/// Query executor for running SQL queries.
pub struct QueryExecutor {
    pool: Arc<ConnectionPool>,
    max_rows: usize,
    retry_config: Option<RetryConfig>,
}

impl QueryExecutor {
    /// Create a new query executor.
    pub fn new(pool: Arc<ConnectionPool>, max_rows: usize) -> Self {
        Self {
            pool,
            max_rows,
            retry_config: None,
        }
    }

    /// Create a new query executor with retry support for transient failures.
    pub fn with_retry(pool: Arc<ConnectionPool>, max_rows: usize, retry_config: RetryConfig) -> Self {
        Self {
            pool,
            max_rows,
            retry_config: Some(retry_config),
        }
    }

    /// Enable or update retry configuration.
    pub fn set_retry_config(&mut self, config: RetryConfig) {
        self.retry_config = Some(config);
    }

    /// Disable retry (execute queries once without retry).
    pub fn disable_retry(&mut self) {
        self.retry_config = None;
    }

    /// Check if retry is enabled.
    pub fn retry_enabled(&self) -> bool {
        self.retry_config.is_some()
    }

    /// Execute a query and return results.
    pub async fn execute(&self, query: &str) -> Result<QueryResult, ServerError> {
        self.execute_with_limit(query, self.max_rows).await
    }

    /// Execute a query with a specific row limit.
    pub async fn execute_with_limit(
        &self,
        query: &str,
        max_rows: usize,
    ) -> Result<QueryResult, ServerError> {
        self.execute_with_options(query, max_rows, None).await
    }

    /// Execute a query with a specific timeout (in seconds).
    pub async fn execute_with_timeout(
        &self,
        query: &str,
        timeout_seconds: u64,
    ) -> Result<QueryResult, ServerError> {
        self.execute_with_options(query, self.max_rows, Some(timeout_seconds))
            .await
    }

    /// Execute a query with configurable row limit and timeout.
    ///
    /// This is the primary execution method that supports both row limits and timeouts.
    /// If retry is enabled, transient failures will be automatically retried.
    /// Other execute methods delegate to this one.
    pub async fn execute_with_options(
        &self,
        query: &str,
        max_rows: usize,
        timeout_seconds: Option<u64>,
    ) -> Result<QueryResult, ServerError> {
        debug!(
            "Executing query (max_rows={}, timeout={:?}s, retry={}): {}",
            max_rows,
            timeout_seconds,
            self.retry_enabled(),
            truncate_for_log(query, 200)
        );

        // Use retry if enabled
        if let Some(ref retry_config) = self.retry_config {
            let pool = self.pool.clone();
            let query_owned = query.to_string();

            with_retry(retry_config, || {
                let pool = pool.clone();
                let query = query_owned.clone();
                async move {
                    Self::execute_query_inner(&pool, &query, max_rows, timeout_seconds).await
                }
            })
            .await
        } else {
            Self::execute_query_inner(&self.pool, query, max_rows, timeout_seconds).await
        }
    }

    /// Inner query execution (without retry logic).
    async fn execute_query_inner(
        pool: &Arc<ConnectionPool>,
        query: &str,
        max_rows: usize,
        timeout_seconds: Option<u64>,
    ) -> Result<QueryResult, ServerError> {
        let start = Instant::now();

        // Wrap execution in timeout if specified
        let execution_future = async {
            let mut conn = pool.get().await.map_err(|e| {
                ServerError::connection(format!("Failed to get connection from pool: {}", e))
            })?;

            let stream = conn
                .query(query, &[])
                .await
                .map_err(|e| ServerError::query_error(format!("Query execution failed: {}", e)))?;

            // Use streaming to process rows - stops at max_rows without loading all into memory
            Self::process_stream_static(stream, max_rows, start).await
        };

        let result = if let Some(secs) = timeout_seconds {
            let duration = Duration::from_secs(secs);
            timeout(duration, execution_future)
                .await
                .map_err(|_| ServerError::timeout(secs))?
        } else {
            execution_future.await
        }?;

        debug!(
            "Query completed: {} rows in {} ms",
            result.rows.len(),
            result.execution_time_ms
        );

        Ok(result)
    }

    /// Execute a query that modifies data (INSERT/UPDATE/DELETE).
    pub async fn execute_non_query(&self, query: &str) -> Result<QueryResult, ServerError> {
        let start = Instant::now();

        debug!("Executing non-query: {}", truncate_for_log(query, 200));

        let mut conn = self.pool.get().await.map_err(|e| {
            ServerError::connection(format!("Failed to get connection from pool: {}", e))
        })?;

        // Execute query - returns rows affected directly as u64
        let rows_affected = conn
            .execute(query, &[])
            .await
            .map_err(|e| ServerError::query_error(format!("Non-query execution failed: {}", e)))?;

        debug!("Non-query completed: {} rows affected", rows_affected);

        Ok(QueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            rows_affected,
            execution_time_ms: start.elapsed().as_millis() as u64,
            truncated: false,
        })
    }

    /// Execute a raw SQL statement.
    ///
    /// This is required for DDL statements that must be the only/first statement
    /// in a batch, such as CREATE VIEW, CREATE PROCEDURE, CREATE FUNCTION, and
    /// CREATE TRIGGER.
    pub async fn execute_raw(&self, query: &str) -> Result<QueryResult, ServerError> {
        let start = Instant::now();

        debug!("Executing raw query: {}", truncate_for_log(query, 200));

        let mut conn = self.pool.get().await.map_err(|e| {
            ServerError::connection(format!("Failed to get connection from pool: {}", e))
        })?;

        // Execute raw SQL
        let stream = conn
            .query(query, &[])
            .await
            .map_err(|e| ServerError::query_error(format!("Raw query failed: {}", e)))?;

        // Use streaming to process rows - stops at max_rows without loading all into memory
        let result = self.process_stream(stream, self.max_rows, start).await?;

        debug!(
            "Raw query completed: {} rows in {} ms",
            result.rows.len(),
            result.execution_time_ms
        );

        Ok(result)
    }

    /// Execute a query that may return multiple result sets.
    ///
    /// This uses `query_multiple` to properly capture all result sets from queries
    /// containing multiple SELECT statements (e.g., `SELECT 1; SELECT 2;`).
    pub async fn execute_multi_result(
        &self,
        query: &str,
        max_rows_per_result: usize,
    ) -> Result<MultiQueryResult, ServerError> {
        let start = Instant::now();

        debug!(
            "Executing multi-result query: {}",
            truncate_for_log(query, 200)
        );

        let mut conn = self.pool.get().await.map_err(|e| {
            ServerError::connection(format!("Failed to get connection from pool: {}", e))
        })?;

        // Get the underlying client to access query_multiple
        let client = conn.client_mut().ok_or_else(|| {
            ServerError::connection("Connection not available".to_string())
        })?;

        // Use query_multiple to get all result sets
        let mut multi_stream = client
            .query_multiple(query, &[])
            .await
            .map_err(|e| ServerError::query_error(format!("Multi-result query failed: {}", e)))?;

        let mut result_sets = Vec::new();
        let result_count = multi_stream.result_count();

        debug!("Query returned {} result set(s)", result_count);

        // Process each result set
        loop {
            let result_set_start = Instant::now();
            let mut columns = Vec::new();
            let mut rows = Vec::new();
            let mut truncated = false;

            // Get column info for this result set
            if let Some(cols) = multi_stream.columns() {
                for col in cols {
                    columns.push(ColumnInfo {
                        name: col.name.clone(),
                        sql_type: if !col.type_name.is_empty() {
                            col.type_name.clone()
                        } else {
                            "unknown".to_string()
                        },
                        nullable: col.nullable,
                    });
                }
            }

            // Collect rows for this result set
            while let Some(row) = multi_stream
                .next_row()
                .await
                .map_err(|e| ServerError::query_error(format!("Failed to read row: {}", e)))?
            {
                if rows.len() >= max_rows_per_result {
                    truncated = true;
                    continue; // Drain remaining rows but don't store
                }

                let mut result_row = ResultRow::new();
                for (idx, col) in columns.iter().enumerate() {
                    let value = TypeMapper::extract_column(&row, idx);
                    result_row.insert(col.name.clone(), value);
                }
                rows.push(result_row);
            }

            // Only add result set if it has columns (skip empty result sets from non-SELECT statements)
            if !columns.is_empty() || !rows.is_empty() {
                result_sets.push(QueryResult {
                    columns,
                    rows,
                    rows_affected: 0,
                    execution_time_ms: result_set_start.elapsed().as_millis() as u64,
                    truncated,
                });
            }

            // Move to next result set
            if !multi_stream
                .next_result()
                .await
                .map_err(|e| ServerError::query_error(format!("Failed to advance to next result: {}", e)))?
            {
                break;
            }
        }

        let execution_time_ms = start.elapsed().as_millis() as u64;

        debug!(
            "Multi-result query completed: {} result set(s) in {} ms",
            result_sets.len(),
            execution_time_ms
        );

        Ok(MultiQueryResult {
            result_sets,
            execution_time_ms,
        })
    }

    /// Check if a query likely contains multiple SELECT statements.
    ///
    /// This is a heuristic check to determine if `execute_multi_result` should be used.
    /// Returns true if the query appears to have multiple SELECT statements separated
    /// by semicolons (outside of string literals).
    pub fn has_multiple_result_sets(query: &str) -> bool {
        let normalized = remove_leading_sql_comments(query).to_uppercase();

        // Simple heuristic: count SELECT keywords that are likely to be statements
        // This is imperfect but catches common cases
        let mut select_count = 0;
        let mut in_string = false;
        let mut chars = normalized.chars().peekable();
        let mut buffer = String::new();

        while let Some(c) = chars.next() {
            // Track string literals
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next(); // Skip escaped quote
                } else {
                    in_string = !in_string;
                }
                continue;
            }

            if in_string {
                continue;
            }

            // Build word buffer
            if c.is_alphabetic() {
                buffer.push(c);
            } else {
                if buffer == "SELECT" {
                    select_count += 1;
                }
                buffer.clear();
            }
        }

        // Check final buffer
        if buffer == "SELECT" {
            select_count += 1;
        }

        select_count > 1
    }

    /// Check if a query requires raw execution (batch-first DDL statements).
    ///
    /// Returns true for CREATE VIEW, CREATE PROCEDURE, CREATE FUNCTION,
    /// CREATE TRIGGER, ALTER VIEW, ALTER PROCEDURE, ALTER FUNCTION, ALTER TRIGGER.
    pub fn requires_raw_execution(query: &str) -> bool {
        let trimmed = query.trim();

        // Remove leading comments to get to the actual statement
        let normalized = remove_leading_sql_comments(trimmed).to_uppercase();

        // Check for batch-first DDL patterns
        let batch_first_patterns = [
            "CREATE VIEW",
            "CREATE PROCEDURE",
            "CREATE PROC",
            "CREATE FUNCTION",
            "CREATE TRIGGER",
            "CREATE SCHEMA",
            "ALTER VIEW",
            "ALTER PROCEDURE",
            "ALTER PROC",
            "ALTER FUNCTION",
            "ALTER TRIGGER",
            "ALTER SCHEMA",
        ];

        batch_first_patterns
            .iter()
            .any(|pattern| normalized.starts_with(pattern))
    }

    /// Execute a query with SHOWPLAN enabled on a dedicated connection.
    ///
    /// This executes SET SHOWPLAN_TEXT ON, the query, and SET SHOWPLAN_TEXT OFF
    /// as separate batches on the same connection. SQL Server requires SHOWPLAN
    /// to be the only statement in its batch.
    pub async fn execute_with_showplan(
        &self,
        query: &str,
        plan_type: &str,
    ) -> Result<QueryResult, ServerError> {
        let start = Instant::now();

        debug!(
            "Executing query with showplan ({}): {}",
            plan_type,
            truncate_for_log(query, 200)
        );

        let mut conn = self.pool.get().await.map_err(|e| {
            ServerError::connection(format!("Failed to get connection from pool: {}", e))
        })?;

        // Determine which SET statements to use based on plan type
        let (set_on, set_off) = match plan_type.to_lowercase().as_str() {
            "actual" => (
                "SET STATISTICS PROFILE ON; SET STATISTICS IO ON; SET STATISTICS TIME ON",
                "SET STATISTICS PROFILE OFF; SET STATISTICS IO OFF; SET STATISTICS TIME OFF",
            ),
            _ => ("SET SHOWPLAN_ALL ON", "SET SHOWPLAN_ALL OFF"),
        };

        // For estimated plans, we need to execute SET SHOWPLAN separately
        if plan_type.to_lowercase() != "actual" {
            // Execute SET SHOWPLAN_ALL ON as its own batch
            conn.execute(set_on, &[])
                .await
                .map_err(|e| ServerError::query_error(format!("Failed to enable SHOWPLAN: {}", e)))?;

            // Execute the query - with SHOWPLAN ON, this returns the execution plan
            let stream = conn.query(query, &[]).await.map_err(|e| {
                ServerError::query_error(format!("Failed to get execution plan: {}", e))
            })?;

            let rows: Vec<mssql_client::Row> = stream.try_collect().await.map_err(|e| {
                ServerError::query_error(format!("Failed to collect execution plan: {}", e))
            })?;

            // Turn off SHOWPLAN (best effort)
            let _ = conn.execute(set_off, &[]).await;

            let result = self.process_rows(rows, self.max_rows, start)?;

            debug!(
                "Showplan query completed: {} rows in {} ms",
                result.rows.len(),
                result.execution_time_ms
            );

            Ok(result)
        } else {
            // For actual execution plans, we can use STATISTICS which don't have
            // the same batch restriction
            let full_query = format!("{}\n{}\n{}", set_on, query, set_off);
            let stream = conn.query(&full_query, &[]).await.map_err(|e| {
                ServerError::query_error(format!("Failed to execute with statistics: {}", e))
            })?;

            let rows: Vec<mssql_client::Row> = stream.try_collect().await.map_err(|e| {
                ServerError::query_error(format!("Failed to collect statistics results: {}", e))
            })?;

            let result = self.process_rows(rows, self.max_rows, start)?;

            debug!(
                "Statistics query completed: {} rows in {} ms",
                result.rows.len(),
                result.execution_time_ms
            );

            Ok(result)
        }
    }

    /// Process query result rows into a QueryResult.
    fn process_rows(
        &self,
        rows: Vec<mssql_client::Row>,
        max_rows: usize,
        start: Instant,
    ) -> Result<QueryResult, ServerError> {
        let mut columns: Vec<ColumnInfo> = Vec::new();
        let mut result_rows: Vec<ResultRow> = Vec::new();
        let mut truncated = false;

        for (idx, row) in rows.into_iter().enumerate() {
            // Extract column info from the first row
            if columns.is_empty() {
                let row_columns = row.columns();
                for (i, col) in row_columns.iter().enumerate() {
                    // Get column name from metadata
                    let name = col.name.clone();

                    // Use type info from column metadata if available, otherwise infer from value
                    let sql_type = if !col.type_name.is_empty() {
                        col.type_name.clone()
                    } else {
                        let sample_value = TypeMapper::extract_column(&row, i);
                        TypeMapper::sql_type_name_from_value(&sample_value).to_string()
                    };

                    columns.push(ColumnInfo {
                        name,
                        sql_type,
                        nullable: col.nullable,
                    });
                }
            }

            // Check row limit
            if idx >= max_rows {
                truncated = true;
                continue;
            }

            // Extract row data
            let mut result_row = ResultRow::new();
            for (col_idx, col) in columns.iter().enumerate() {
                let value = TypeMapper::extract_column(&row, col_idx);
                result_row.insert(col.name.clone(), value);
            }
            result_rows.push(result_row);
        }

        Ok(QueryResult {
            columns,
            rows: result_rows,
            rows_affected: 0,
            execution_time_ms: start.elapsed().as_millis() as u64,
            truncated,
        })
    }

    /// Process a query stream into a QueryResult, stopping at max_rows.
    ///
    /// This is more memory-efficient than `process_rows` as it stops reading
    /// from the stream after reaching max_rows instead of collecting all rows first.
    async fn process_stream<S>(
        &self,
        stream: S,
        max_rows: usize,
        start: Instant,
    ) -> Result<QueryResult, ServerError>
    where
        S: futures_util::Stream<Item = Result<mssql_client::Row, mssql_client::Error>>
            + Unpin,
    {
        Self::process_stream_static(stream, max_rows, start).await
    }

    /// Static version of process_stream for use in retry closures.
    ///
    /// This method doesn't require `&self`, making it usable in contexts where
    /// capturing self in a closure is problematic (e.g., retry loops).
    async fn process_stream_static<S>(
        mut stream: S,
        max_rows: usize,
        start: Instant,
    ) -> Result<QueryResult, ServerError>
    where
        S: futures_util::Stream<Item = Result<mssql_client::Row, mssql_client::Error>>
            + Unpin,
    {
        use futures_util::TryStreamExt;

        let mut columns: Vec<ColumnInfo> = Vec::new();
        let mut result_rows: Vec<ResultRow> = Vec::new();
        let mut truncated = false;
        let mut row_count = 0;

        // Process rows one at a time from the stream
        while let Some(row) = stream.try_next().await.map_err(|e| {
            ServerError::query_error(format!("Failed to read row from stream: {}", e))
        })? {
            // Extract column info from the first row
            if columns.is_empty() {
                let row_columns = row.columns();
                for (i, col) in row_columns.iter().enumerate() {
                    let name = col.name.clone();
                    let sql_type = if !col.type_name.is_empty() {
                        col.type_name.clone()
                    } else {
                        let sample_value = TypeMapper::extract_column(&row, i);
                        TypeMapper::sql_type_name_from_value(&sample_value).to_string()
                    };

                    columns.push(ColumnInfo {
                        name,
                        sql_type,
                        nullable: col.nullable,
                    });
                }
            }

            // Check if we've reached the limit
            if row_count >= max_rows {
                truncated = true;
                // Stop reading from the stream - this is the key optimization!
                // We don't need to read remaining rows just to count them.
                break;
            }

            // Extract row data
            let mut result_row = ResultRow::new();
            for (col_idx, col) in columns.iter().enumerate() {
                let value = TypeMapper::extract_column(&row, col_idx);
                result_row.insert(col.name.clone(), value);
            }
            result_rows.push(result_row);
            row_count += 1;
        }

        Ok(QueryResult {
            columns,
            rows: result_rows,
            rows_affected: 0,
            execution_time_ms: start.elapsed().as_millis() as u64,
            truncated,
        })
    }

    /// Validate SQL syntax without executing the query.
    ///
    /// Uses SET PARSEONLY ON to check if the SQL syntax is valid without
    /// actually executing the statement. This is useful for dry-run validation.
    ///
    /// Returns Ok(()) if syntax is valid, or an error describing the syntax issue.
    pub async fn validate_syntax(&self, query: &str) -> Result<ValidationResult, ServerError> {
        let start = Instant::now();

        debug!("Validating query syntax: {}", truncate_for_log(query, 200));

        let mut conn = self.pool.get().await.map_err(|e| {
            ServerError::connection(format!("Failed to get connection from pool: {}", e))
        })?;

        // Enable PARSEONLY mode - this parses but doesn't execute
        conn.query("SET PARSEONLY ON", &[])
            .await
            .map_err(|e| ServerError::query_error(format!("Failed to enable PARSEONLY: {}", e)))?
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| ServerError::query_error(format!("Failed to enable PARSEONLY: {}", e)))?;

        // Try to parse the query
        let validation_result = match conn.query(query, &[]).await {
            Ok(stream) => {
                // Drain the stream (even in PARSEONLY mode, we need to consume the result)
                let _ = stream.try_collect::<Vec<_>>().await;
                ValidationResult {
                    valid: true,
                    error_message: None,
                    error_line: None,
                    error_position: None,
                    validation_time_ms: start.elapsed().as_millis() as u64,
                }
            }
            Err(e) => {
                let error_str = e.to_string();
                // Try to extract line number from error message
                // SQL Server errors often include "Line X" in the message
                let error_line = extract_line_number(&error_str);

                ValidationResult {
                    valid: false,
                    error_message: Some(error_str),
                    error_line,
                    error_position: None,
                    validation_time_ms: start.elapsed().as_millis() as u64,
                }
            }
        };

        // Disable PARSEONLY mode to return connection to normal state
        let _ = conn
            .query("SET PARSEONLY OFF", &[])
            .await
            .map_err(|e| {
                debug!("Failed to disable PARSEONLY: {}", e);
            });

        debug!(
            "Syntax validation completed in {} ms: valid={}",
            validation_result.validation_time_ms, validation_result.valid
        );

        Ok(validation_result)
    }

    /// Execute a multi-batch query, splitting on GO separators.
    ///
    /// GO is not a T-SQL command - it's a batch separator used by tools like SSMS.
    /// This method splits the script on GO and executes each batch sequentially.
    /// Results from all batches are combined into a single result.
    pub async fn execute_multi_batch(&self, script: &str) -> Result<QueryResult, ServerError> {
        let start = Instant::now();
        let batches = split_on_go(script);
        let total_batches = batches.len();

        debug!(
            "Executing multi-batch query with {} batch(es)",
            total_batches
        );

        let mut combined_columns: Vec<ColumnInfo> = Vec::new();
        let mut combined_rows: Vec<ResultRow> = Vec::new();
        let mut batch_num = 0;

        let mut conn = self.pool.get().await.map_err(|e| {
            ServerError::connection(format!("Failed to get connection from pool: {}", e))
        })?;

        for batch in batches {
            let trimmed = batch.trim();
            if trimmed.is_empty() {
                continue;
            }

            batch_num += 1;
            let batch_preview = truncate_for_log(trimmed, 60);

            // Progress feedback at INFO level for visibility
            info!(
                batch = batch_num,
                total = total_batches,
                "Executing batch {}/{}: {}",
                batch_num,
                total_batches,
                batch_preview
            );

            let batch_start = Instant::now();

            // Execute each batch and collect results
            let stream = conn
                .query(trimmed, &[])
                .await
                .map_err(|e| {
                    ServerError::query_error(format!(
                        "Batch {}/{} failed: {}\n  SQL: {}",
                        batch_num, total_batches, e, batch_preview
                    ))
                })?;

            let rows: Vec<mssql_client::Row> = stream.try_collect().await.map_err(|e| {
                ServerError::query_error(format!(
                    "Batch {}/{} result collection failed: {}\n  SQL: {}",
                    batch_num, total_batches, e, batch_preview
                ))
            })?;

            let batch_elapsed = batch_start.elapsed().as_millis();
            debug!(
                "Batch {}/{} completed in {} ms, {} rows",
                batch_num,
                total_batches,
                batch_elapsed,
                rows.len()
            );

            // Process results from this batch
            for row in rows {
                // Extract column info from the first row if we haven't yet
                if combined_columns.is_empty() {
                    let row_columns = row.columns();
                    for (i, col) in row_columns.iter().enumerate() {
                        let name = col.name.clone();
                        let sql_type = if !col.type_name.is_empty() {
                            col.type_name.clone()
                        } else {
                            let sample_value = TypeMapper::extract_column(&row, i);
                            TypeMapper::sql_type_name_from_value(&sample_value).to_string()
                        };

                        combined_columns.push(ColumnInfo {
                            name,
                            sql_type,
                            nullable: col.nullable,
                        });
                    }
                }

                // Check row limit
                if combined_rows.len() >= self.max_rows {
                    continue;
                }

                // Extract row data
                let mut result_row = ResultRow::new();
                for (idx, col) in combined_columns.iter().enumerate() {
                    let value = TypeMapper::extract_column(&row, idx);
                    result_row.insert(col.name.clone(), value);
                }
                combined_rows.push(result_row);
            }
        }

        let truncated = combined_rows.len() >= self.max_rows;

        // Summary at INFO level
        info!(
            batches = batch_num,
            rows = combined_rows.len(),
            elapsed_ms = start.elapsed().as_millis() as u64,
            "Multi-batch execution completed"
        );

        Ok(QueryResult {
            columns: combined_columns,
            rows: combined_rows,
            rows_affected: 0, // Multi-batch doesn't track rows affected
            execution_time_ms: start.elapsed().as_millis() as u64,
            truncated,
        })
    }

    /// Execute a multi-batch query with optional database context.
    ///
    /// Like execute_multi_batch, but prepends USE [database] to each batch
    /// to ensure all batches run in the correct database context.
    pub async fn execute_multi_batch_with_db(
        &self,
        script: &str,
        database: Option<&str>,
    ) -> Result<QueryResult, ServerError> {
        let start = Instant::now();
        let batches = split_on_go(script);
        let total_batches = batches.len();

        debug!(
            "Executing multi-batch query with {} batch(es), database: {:?}",
            total_batches,
            database
        );

        let mut combined_columns: Vec<ColumnInfo> = Vec::new();
        let mut combined_rows: Vec<ResultRow> = Vec::new();
        let mut batch_num = 0;

        let mut conn = self.pool.get().await.map_err(|e| {
            ServerError::connection(format!("Failed to get connection from pool: {}", e))
        })?;

        for batch in batches {
            let trimmed = batch.trim();
            if trimmed.is_empty() {
                continue;
            }

            batch_num += 1;

            // Prepend USE [database] to each batch if a database is specified
            let effective_batch = match database {
                Some(db) => format!("USE [{}];\n{}", db, trimmed),
                None => trimmed.to_string(),
            };

            let batch_preview = truncate_for_log(trimmed, 60);

            // Progress feedback at INFO level for visibility
            info!(
                batch = batch_num,
                total = total_batches,
                database = database.unwrap_or("default"),
                "Executing batch {}/{}: {}",
                batch_num,
                total_batches,
                batch_preview
            );

            let batch_start = Instant::now();

            // Execute each batch and collect results
            let stream = conn
                .query(&effective_batch, &[])
                .await
                .map_err(|e| {
                    ServerError::query_error(format!(
                        "Batch {}/{} failed: {}\n  SQL: {}",
                        batch_num, total_batches, e, batch_preview
                    ))
                })?;

            let rows: Vec<mssql_client::Row> = stream.try_collect().await.map_err(|e| {
                ServerError::query_error(format!(
                    "Batch {}/{} result collection failed: {}\n  SQL: {}",
                    batch_num, total_batches, e, batch_preview
                ))
            })?;

            let batch_elapsed = batch_start.elapsed().as_millis();
            debug!(
                "Batch {}/{} completed in {} ms, {} rows",
                batch_num,
                total_batches,
                batch_elapsed,
                rows.len()
            );

            // Process results from this batch
            for row in rows {
                // Extract column info from the first row if we haven't yet
                if combined_columns.is_empty() {
                    let row_columns = row.columns();
                    for (i, col) in row_columns.iter().enumerate() {
                        let name = col.name.clone();
                        let sql_type = if !col.type_name.is_empty() {
                            col.type_name.clone()
                        } else {
                            let sample_value = TypeMapper::extract_column(&row, i);
                            TypeMapper::sql_type_name_from_value(&sample_value).to_string()
                        };

                        combined_columns.push(ColumnInfo {
                            name,
                            sql_type,
                            nullable: col.nullable,
                        });
                    }
                }

                // Check row limit
                if combined_rows.len() >= self.max_rows {
                    continue;
                }

                // Extract row data
                let mut result_row = ResultRow::new();
                for (idx, col) in combined_columns.iter().enumerate() {
                    let value = TypeMapper::extract_column(&row, idx);
                    result_row.insert(col.name.clone(), value);
                }
                combined_rows.push(result_row);
            }
        }

        let truncated = combined_rows.len() >= self.max_rows;

        // Summary at INFO level
        info!(
            batches = batch_num,
            rows = combined_rows.len(),
            elapsed_ms = start.elapsed().as_millis() as u64,
            database = database.unwrap_or("default"),
            "Multi-batch execution completed"
        );

        Ok(QueryResult {
            columns: combined_columns,
            rows: combined_rows,
            rows_affected: 0,
            execution_time_ms: start.elapsed().as_millis() as u64,
            truncated,
        })
    }

    /// Execute multiple statements in a single transaction.
    ///
    /// This method holds onto a single connection for all statements and wraps
    /// them in BEGIN TRANSACTION / COMMIT TRANSACTION. If any statement fails,
    /// the transaction is rolled back.
    ///
    /// Returns the total number of rows affected across all statements.
    pub async fn execute_in_transaction(
        &self,
        statements: &[String],
        continue_on_error: bool,
    ) -> Result<TransactionBatchResult, ServerError> {
        let start = Instant::now();
        let total_statements = statements.len();

        debug!(
            "Executing {} statements in transaction (continue_on_error={})",
            total_statements, continue_on_error
        );

        let mut conn = self.pool.get().await.map_err(|e| {
            ServerError::connection(format!("Failed to get connection from pool: {}", e))
        })?;

        // Begin transaction
        conn.execute("BEGIN TRANSACTION", &[])
            .await
            .map_err(|e| ServerError::query_error(format!("Failed to begin transaction: {}", e)))?;

        let mut total_rows_affected: u64 = 0;
        let mut successful_statements = 0;
        let mut errors: Vec<String> = Vec::new();

        for (idx, stmt) in statements.iter().enumerate() {
            let stmt_preview = truncate_for_log(stmt, 60);
            debug!(
                "Executing statement {}/{}: {}",
                idx + 1,
                total_statements,
                stmt_preview
            );

            match conn.execute(stmt, &[]).await {
                Ok(rows_affected) => {
                    total_rows_affected += rows_affected;
                    successful_statements += 1;
                }
                Err(e) => {
                    let error_msg = format!(
                        "Statement {}/{} failed: {}\n  SQL: {}",
                        idx + 1,
                        total_statements,
                        e,
                        stmt_preview
                    );

                    if continue_on_error {
                        errors.push(error_msg);
                        debug!("Continuing after error in statement {}", idx + 1);
                    } else {
                        // Rollback and return error
                        debug!("Rolling back transaction due to error");
                        let _ = conn.execute("ROLLBACK TRANSACTION", &[]).await;
                        return Err(ServerError::query_error(error_msg));
                    }
                }
            }
        }

        // Commit transaction
        if let Err(e) = conn.execute("COMMIT TRANSACTION", &[]).await {
            // Try to rollback if commit fails
            let _ = conn.execute("ROLLBACK TRANSACTION", &[]).await;
            return Err(ServerError::query_error(format!(
                "Failed to commit transaction: {}",
                e
            )));
        }

        let execution_time_ms = start.elapsed().as_millis() as u64;

        debug!(
            "Transaction completed: {} rows affected in {} ms",
            total_rows_affected, execution_time_ms
        );

        Ok(TransactionBatchResult {
            total_rows_affected,
            successful_statements,
            total_statements,
            errors,
            execution_time_ms,
        })
    }

    /// Check if a query contains GO batch separators.
    pub fn contains_go_separator(query: &str) -> bool {
        // GO must be on its own line (possibly with whitespace)
        for line in query.lines() {
            let trimmed = line.trim();
            if trimmed.eq_ignore_ascii_case("GO") {
                return true;
            }
            // Also match "GO n" for repeated execution (e.g., "GO 5")
            if trimmed.to_uppercase().starts_with("GO ")
                && trimmed[3..].trim().chars().all(|c| c.is_ascii_digit())
            {
                return true;
            }
        }
        false
    }

    /// Execute a query with a Table-Valued Parameter (TVP).
    ///
    /// TVPs allow passing structured data to stored procedures as a single parameter.
    /// The table type must exist in the database before using this method.
    ///
    /// # Arguments
    ///
    /// * `query` - SQL query containing the TVP parameter (e.g., "EXEC MyProc @ids = @p1")
    /// * `tvp` - The TvpValue containing the data
    /// * `max_rows` - Maximum rows to return
    pub async fn execute_with_tvp(
        &self,
        query: &str,
        tvp: TvpValue,
        max_rows: usize,
    ) -> Result<QueryResult, ServerError> {
        let start = Instant::now();

        debug!(
            "Executing query with TVP (type={}, rows={}, columns={}): {}",
            tvp.type_name,
            tvp.rows.len(),
            tvp.columns.len(),
            truncate_for_log(query, 200)
        );

        let mut conn = self.pool.get().await.map_err(|e| {
            ServerError::connection(format!("Failed to get connection from pool: {}", e))
        })?;

        // Execute with TVP as parameter
        let stream = conn
            .query(query, &[&tvp])
            .await
            .map_err(|e| ServerError::query_error(format!("TVP query execution failed: {}", e)))?;

        // Process the result stream
        let result = self.process_stream(stream, max_rows, start).await?;

        debug!(
            "TVP query completed: {} rows in {} ms",
            result.rows.len(),
            result.execution_time_ms
        );

        Ok(result)
    }

    /// Build a TvpValue from column definitions and row data.
    ///
    /// # Arguments
    ///
    /// * `type_name` - SQL Server table type name (e.g., "dbo.IntIdList")
    /// * `columns` - Column definitions with name and SQL type
    /// * `rows` - Row data as JSON values
    ///
    /// # Returns
    ///
    /// A TvpValue ready to be passed to execute_with_tvp.
    pub fn build_tvp(
        type_name: &str,
        columns: &[(String, String)],
        rows: &[Vec<serde_json::Value>],
    ) -> Result<TvpValue, ServerError> {
        use mssql_client::SqlValue as MssqlSqlValue;

        // Build column definitions
        let tvp_columns: Vec<TvpColumn> = columns
            .iter()
            .enumerate()
            .map(|(ordinal, (name, sql_type))| TvpColumn::new(name.clone(), sql_type.clone(), ordinal))
            .collect();

        // Build rows by converting JSON values to SqlValue
        let mut tvp_rows = Vec::with_capacity(rows.len());
        for (row_idx, row) in rows.iter().enumerate() {
            if row.len() != columns.len() {
                return Err(ServerError::validation(format!(
                    "Row {} has {} values but {} columns defined",
                    row_idx,
                    row.len(),
                    columns.len()
                )));
            }

            let values: Result<Vec<MssqlSqlValue>, ServerError> = row
                .iter()
                .enumerate()
                .map(|(col_idx, value)| {
                    json_to_sql_value(value).map_err(|e| {
                        ServerError::validation(format!(
                            "Row {}, column {}: {}",
                            row_idx, col_idx, e
                        ))
                    })
                })
                .collect();

            tvp_rows.push(TvpRow::new(values?));
        }

        // Build the TvpValue directly
        Ok(TvpValue {
            type_name: type_name.to_string(),
            columns: tvp_columns,
            rows: tvp_rows,
        })
    }
}

/// Convert a JSON value to a SQL value for TVP parameters.
fn json_to_sql_value(value: &serde_json::Value) -> Result<mssql_client::SqlValue, String> {
    use mssql_client::SqlValue;

    match value {
        serde_json::Value::Null => Ok(SqlValue::Null),
        serde_json::Value::Bool(b) => Ok(SqlValue::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                // Use appropriate size based on value range
                if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
                    Ok(SqlValue::Int(i as i32))
                } else {
                    Ok(SqlValue::BigInt(i))
                }
            } else if let Some(f) = n.as_f64() {
                // Use Double for f64 values
                Ok(SqlValue::Double(f))
            } else {
                Err(format!("Unsupported number: {}", n))
            }
        }
        serde_json::Value::String(s) => Ok(SqlValue::String(s.clone())),
        serde_json::Value::Array(_) => Err("Arrays not supported as TVP column values".to_string()),
        serde_json::Value::Object(_) => {
            Err("Objects not supported as TVP column values".to_string())
        }
    }
}

/// Truncate a string for logging purposes.
fn truncate_for_log(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

/// Split a SQL script on GO batch separators.
///
/// GO must be on its own line (possibly with whitespace and optional count).
/// Returns a vector of batch strings.
fn split_on_go(script: &str) -> Vec<String> {
    let mut batches = Vec::new();
    let mut current_batch = String::new();

    for line in script.lines() {
        let trimmed = line.trim();

        // Check if this line is a GO statement
        let (is_go, repeat_count) = if trimmed.eq_ignore_ascii_case("GO") {
            (true, 1)
        } else if trimmed.to_uppercase().starts_with("GO ") {
            // Check for "GO n" syntax
            let count_str = trimmed[3..].trim();
            if let Ok(n) = count_str.parse::<usize>() {
                (true, n.max(1))
            } else {
                (false, 1)
            }
        } else {
            (false, 1)
        };

        if is_go {
            // End current batch and add it (potentially multiple times for GO n)
            let batch = current_batch.trim().to_string();
            if !batch.is_empty() {
                for _ in 0..repeat_count {
                    batches.push(batch.clone());
                }
            }
            current_batch.clear();
        } else {
            // Add line to current batch
            if !current_batch.is_empty() {
                current_batch.push('\n');
            }
            current_batch.push_str(line);
        }
    }

    // Add final batch if not empty
    let batch = current_batch.trim().to_string();
    if !batch.is_empty() {
        batches.push(batch);
    }

    batches
}

/// Remove leading SQL comments from a query string.
///
/// This handles both line comments (--) and block comments (/* */).
fn remove_leading_sql_comments(query: &str) -> String {
    let mut result = query.to_string();

    loop {
        let trimmed = result.trim_start();

        // Remove line comments
        if trimmed.starts_with("--") {
            if let Some(newline_pos) = trimmed.find('\n') {
                result = trimmed[newline_pos + 1..].to_string();
                continue;
            } else {
                // Entire query is a comment
                return String::new();
            }
        }

        // Remove block comments
        if trimmed.starts_with("/*") {
            if let Some(end_pos) = trimmed.find("*/") {
                result = trimmed[end_pos + 2..].to_string();
                continue;
            } else {
                // Unclosed comment
                return String::new();
            }
        }

        break;
    }

    result.trim_start().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_result_row() {
        let mut row = ResultRow::new();
        row.insert("id".to_string(), SqlValue::I32(1));
        row.insert("name".to_string(), SqlValue::String("test".to_string()));

        assert_eq!(
            row.get("id").map(|v| v.to_display_string()),
            Some("1".to_string())
        );
        assert_eq!(
            row.get("name").map(|v| v.to_display_string()),
            Some("test".to_string())
        );
        assert!(row.get("missing").is_none());
    }

    #[test]
    fn test_query_result_empty() {
        let result = QueryResult::empty();
        assert!(result.columns.is_empty());
        assert!(result.rows.is_empty());
        assert_eq!(result.rows_affected, 0);
    }

    #[test]
    fn test_markdown_table() {
        let mut result = QueryResult::empty();
        result.columns = vec![
            ColumnInfo {
                name: "id".to_string(),
                sql_type: "INT".to_string(),
                nullable: false,
            },
            ColumnInfo {
                name: "name".to_string(),
                sql_type: "VARCHAR".to_string(),
                nullable: true,
            },
        ];

        let mut row1 = ResultRow::new();
        row1.insert("id".to_string(), SqlValue::I32(1));
        row1.insert("name".to_string(), SqlValue::String("Alice".to_string()));

        let mut row2 = ResultRow::new();
        row2.insert("id".to_string(), SqlValue::I32(2));
        row2.insert("name".to_string(), SqlValue::String("Bob".to_string()));

        result.rows = vec![row1, row2];
        result.execution_time_ms = 5;

        let md = result.to_markdown_table();
        assert!(md.contains("| id | name |"));
        assert!(md.contains("| 1 | Alice |"));
        assert!(md.contains("| 2 | Bob |"));
        assert!(md.contains("2 row(s)"));
    }

    #[test]
    fn test_csv_output() {
        let mut result = QueryResult::empty();
        result.columns = vec![
            ColumnInfo {
                name: "id".to_string(),
                sql_type: "INT".to_string(),
                nullable: false,
            },
            ColumnInfo {
                name: "name".to_string(),
                sql_type: "VARCHAR".to_string(),
                nullable: true,
            },
        ];

        let mut row = ResultRow::new();
        row.insert("id".to_string(), SqlValue::I32(1));
        row.insert(
            "name".to_string(),
            SqlValue::String("value, with comma".to_string()),
        );
        result.rows = vec![row];

        let csv = result.to_csv();
        assert!(csv.contains("id,name"));
        assert!(csv.contains("\"value, with comma\"")); // Should be quoted
    }

    #[test]
    fn test_truncate_for_log() {
        assert_eq!(truncate_for_log("short", 10), "short");
        assert_eq!(
            truncate_for_log("this is a long string", 10),
            "this is a ..."
        );
    }

    #[test]
    fn test_split_on_go() {
        let script = "SELECT 1\nGO\nSELECT 2";
        let batches = split_on_go(script);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0], "SELECT 1");
        assert_eq!(batches[1], "SELECT 2");
    }

    #[test]
    fn test_split_on_go_repeat() {
        let script = "INSERT INTO t VALUES (1)\nGO 3";
        let batches = split_on_go(script);
        assert_eq!(batches.len(), 3);
    }

    #[test]
    fn test_requires_raw_execution() {
        assert!(QueryExecutor::requires_raw_execution(
            "CREATE VIEW v AS SELECT 1"
        ));
        assert!(QueryExecutor::requires_raw_execution(
            "  CREATE PROCEDURE p AS BEGIN SELECT 1 END"
        ));
        assert!(QueryExecutor::requires_raw_execution(
            "-- comment\nCREATE FUNCTION f() RETURNS INT AS BEGIN RETURN 1 END"
        ));
        assert!(!QueryExecutor::requires_raw_execution(
            "SELECT * FROM sys.tables"
        ));
        assert!(!QueryExecutor::requires_raw_execution(
            "INSERT INTO t VALUES (1)"
        ));
    }

    #[test]
    fn test_contains_go_separator() {
        assert!(QueryExecutor::contains_go_separator(
            "SELECT 1\nGO\nSELECT 2"
        ));
        assert!(QueryExecutor::contains_go_separator(
            "SELECT 1\n  GO  \nSELECT 2"
        ));
        assert!(QueryExecutor::contains_go_separator("SELECT 1\nGO 5"));
        assert!(!QueryExecutor::contains_go_separator("SELECT 1; SELECT 2"));
        assert!(!QueryExecutor::contains_go_separator("SELECT 'GO' AS word"));
    }

    #[test]
    fn test_has_multiple_result_sets() {
        // Should detect multiple SELECTs
        assert!(QueryExecutor::has_multiple_result_sets(
            "SELECT 1; SELECT 2"
        ));
        assert!(QueryExecutor::has_multiple_result_sets(
            "SELECT * FROM t1; SELECT * FROM t2;"
        ));
        assert!(QueryExecutor::has_multiple_result_sets(
            "SELECT COUNT(*) FROM sys.procedures; SELECT COUNT(*) FROM sys.tables"
        ));

        // Should not trigger for single SELECT
        assert!(!QueryExecutor::has_multiple_result_sets("SELECT * FROM t1"));
        assert!(!QueryExecutor::has_multiple_result_sets(
            "SELECT * FROM t1 WHERE name = 'SELECT'"
        ));

        // Should not trigger for subqueries (SELECT inside string)
        assert!(!QueryExecutor::has_multiple_result_sets(
            "SELECT * FROM t1 WHERE name = 'test SELECT value'"
        ));

        // Note: UNION queries have multiple SELECT keywords but produce one result set.
        // The heuristic will detect them as "multiple" but execute_multi_result handles
        // this correctly (it will just get one result set back).
        // This is acceptable behavior - false positives are fine, false negatives are not.
        assert!(QueryExecutor::has_multiple_result_sets(
            "SELECT 1 UNION SELECT 2"
        ));
    }

    #[test]
    fn test_multi_query_result_single() {
        let result = QueryResult {
            columns: vec![ColumnInfo {
                name: "id".to_string(),
                sql_type: "INT".to_string(),
                nullable: false,
            }],
            rows: vec![],
            rows_affected: 0,
            execution_time_ms: 10,
            truncated: false,
        };

        let multi = MultiQueryResult::single(result);
        assert_eq!(multi.result_count(), 1);
        assert_eq!(multi.total_rows(), 0);
        assert!(!multi.any_truncated());
    }

    #[test]
    fn test_multi_query_result_formatting() {
        let result1 = QueryResult {
            columns: vec![ColumnInfo {
                name: "a".to_string(),
                sql_type: "INT".to_string(),
                nullable: false,
            }],
            rows: {
                let mut row = ResultRow::new();
                row.insert("a".to_string(), SqlValue::I32(1));
                vec![row]
            },
            rows_affected: 0,
            execution_time_ms: 5,
            truncated: false,
        };

        let result2 = QueryResult {
            columns: vec![ColumnInfo {
                name: "b".to_string(),
                sql_type: "INT".to_string(),
                nullable: false,
            }],
            rows: {
                let mut row = ResultRow::new();
                row.insert("b".to_string(), SqlValue::I32(2));
                vec![row]
            },
            rows_affected: 0,
            execution_time_ms: 5,
            truncated: false,
        };

        let multi = MultiQueryResult {
            result_sets: vec![result1, result2],
            execution_time_ms: 10,
        };

        assert_eq!(multi.result_count(), 2);
        assert_eq!(multi.total_rows(), 2);

        let markdown = multi.to_markdown_table();
        assert!(markdown.contains("Result Set 1 of 2"));
        assert!(markdown.contains("Result Set 2 of 2"));
        assert!(markdown.contains("| a |"));
        assert!(markdown.contains("| b |"));
    }

    #[test]
    fn test_extract_line_number() {
        // Standard SQL Server error format
        assert_eq!(extract_line_number("Incorrect syntax near 'FROM'. Line 5"), Some(5));
        assert_eq!(extract_line_number("Line 1: Invalid column name"), Some(1));
        assert_eq!(extract_line_number("Error at line 10"), Some(10));

        // Case insensitivity
        assert_eq!(extract_line_number("ERROR LINE 42"), Some(42));
        assert_eq!(extract_line_number("line 99 has error"), Some(99));

        // No line number
        assert_eq!(extract_line_number("Invalid syntax"), None);
        assert_eq!(extract_line_number(""), None);

        // Line in different context (should still match)
        assert_eq!(extract_line_number("Procedure or function expects parameter '@id'. Line 15."), Some(15));
    }

    #[test]
    fn test_validation_result_success() {
        let result = ValidationResult::success(10);
        assert!(result.valid);
        assert!(result.error_message.is_none());
        assert!(result.error_line.is_none());
        assert_eq!(result.validation_time_ms, 10);

        let msg = result.to_message();
        assert!(msg.contains("valid"));
        assert!(msg.contains("10ms"));
    }

    #[test]
    fn test_validation_result_failure() {
        let result = ValidationResult::failure(
            "Incorrect syntax near 'FROM'. Line 5".to_string(),
            15,
        );
        assert!(!result.valid);
        assert!(result.error_message.is_some());
        assert_eq!(result.error_line, Some(5)); // Extracted from message
        assert_eq!(result.validation_time_ms, 15);

        let msg = result.to_message();
        assert!(msg.contains("Syntax error"));
        assert!(msg.contains("Line: 5"));
    }
}
