//! Query execution and result handling.

use crate::database::types::{SqlValue, TypeMapper};
use crate::database::ConnectionPool;
use crate::error::McpError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;
use tiberius::QueryStream;
use tracing::debug;

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
                return format!("Query executed successfully. {} row(s) affected.", self.rows_affected);
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

/// Query executor for running SQL queries.
pub struct QueryExecutor {
    pool: ConnectionPool,
    max_rows: usize,
}

impl QueryExecutor {
    /// Create a new query executor.
    pub fn new(pool: ConnectionPool, max_rows: usize) -> Self {
        Self { pool, max_rows }
    }

    /// Execute a query and return results.
    pub async fn execute(&self, query: &str) -> Result<QueryResult, McpError> {
        self.execute_with_limit(query, self.max_rows).await
    }

    /// Execute a query with a specific row limit.
    pub async fn execute_with_limit(
        &self,
        query: &str,
        max_rows: usize,
    ) -> Result<QueryResult, McpError> {
        let start = Instant::now();

        debug!("Executing query: {}", truncate_for_log(query, 200));

        let mut conn = self.pool.get().await?;

        // Execute query
        let stream = conn.query(query, &[]).await?;

        // Process results
        let result = self.process_stream(stream, max_rows, start).await?;

        debug!(
            "Query completed: {} rows in {} ms",
            result.rows.len(),
            result.execution_time_ms
        );

        Ok(result)
    }

    /// Execute a query that modifies data (INSERT/UPDATE/DELETE).
    pub async fn execute_non_query(&self, query: &str) -> Result<QueryResult, McpError> {
        let start = Instant::now();

        debug!("Executing non-query: {}", truncate_for_log(query, 200));

        let mut conn = self.pool.get().await?;

        // Execute query
        let result = conn.execute(query, &[]).await?;

        let rows_affected = result.rows_affected().iter().sum();

        debug!("Non-query completed: {} rows affected", rows_affected);

        Ok(QueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            rows_affected,
            execution_time_ms: start.elapsed().as_millis() as u64,
            truncated: false,
        })
    }

    /// Execute a raw SQL statement using simple_query.
    ///
    /// This is required for DDL statements that must be the only/first statement
    /// in a batch, such as CREATE VIEW, CREATE PROCEDURE, CREATE FUNCTION, and
    /// CREATE TRIGGER. The regular `query()` method uses sp_executesql which
    /// creates its own batch context, breaking these requirements.
    ///
    /// This method sends raw SQL text directly without parameterization.
    pub async fn execute_raw(&self, query: &str) -> Result<QueryResult, McpError> {
        let start = Instant::now();

        debug!("Executing raw query: {}", truncate_for_log(query, 200));

        let mut conn = self.pool.get().await?;

        // Use simple_query which sends raw SQL without sp_executesql wrapper
        let results = conn
            .simple_query(query)
            .await
            .map_err(|e| McpError::query_error(format!("Raw query failed: {}", e)))?
            .into_results()
            .await
            .map_err(|e| McpError::query_error(format!("Failed to get results: {}", e)))?;

        let result = self.convert_simple_query_results(results, start);

        debug!(
            "Raw query completed: {} rows in {} ms",
            result.rows.len(),
            result.execution_time_ms
        );

        Ok(result)
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
            "ALTER VIEW",
            "ALTER PROCEDURE",
            "ALTER PROC",
            "ALTER FUNCTION",
            "ALTER TRIGGER",
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
    ) -> Result<QueryResult, McpError> {
        let start = Instant::now();

        debug!(
            "Executing query with showplan ({}): {}",
            plan_type,
            truncate_for_log(query, 200)
        );

        let mut conn = self.pool.get().await?;

        // Determine which SET statements to use based on plan type
        let (set_on, set_off) = match plan_type.to_lowercase().as_str() {
            "actual" => (
                "SET STATISTICS PROFILE ON; SET STATISTICS IO ON; SET STATISTICS TIME ON",
                "SET STATISTICS PROFILE OFF; SET STATISTICS IO OFF; SET STATISTICS TIME OFF",
            ),
            _ => ("SET SHOWPLAN_ALL ON", "SET SHOWPLAN_ALL OFF"),
        };

        // For estimated plans, we need to execute SET SHOWPLAN separately
        // because SQL Server requires it to be the only statement in the batch
        if plan_type.to_lowercase() != "actual" {
            // Execute SET SHOWPLAN_ALL ON as its own batch - must consume results
            conn.simple_query(set_on)
                .await
                .map_err(|e| McpError::query_error(format!("Failed to enable SHOWPLAN: {}", e)))?
                .into_results()
                .await
                .map_err(|e| McpError::query_error(format!("Failed to enable SHOWPLAN: {}", e)))?;

            // Execute the query using simple_query - with SHOWPLAN ON, this returns
            // the execution plan instead of executing the query. We use simple_query
            // because SHOWPLAN results come back as regular result sets.
            let results = conn
                .simple_query(query)
                .await
                .map_err(|e| McpError::query_error(format!("Failed to get execution plan: {}", e)))?
                .into_results()
                .await
                .map_err(|e| McpError::query_error(format!("Failed to get execution plan: {}", e)))?;

            // Turn off SHOWPLAN (best effort - if this fails, the connection will be invalid
            // but bb8 will handle that on next use)
            if let Ok(stream) = conn.simple_query(set_off).await {
                let _ = stream.into_results().await;
            }

            // Convert the results to our QueryResult format
            // SHOWPLAN_ALL returns columns: StmtText, StmtId, NodeId, Parent, PhysicalOp,
            // LogicalOp, Argument, DefinedValues, EstimateRows, EstimateIO, EstimateCPU,
            // AvgRowSize, TotalSubtreeCost, OutputList, Warnings, Type, Parallel, EstimateExecutions
            let result = self.convert_simple_query_results(results, start);

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
            let stream = conn
                .query(&full_query, &[])
                .await
                .map_err(|e| McpError::query_error(format!("Failed to execute with statistics: {}", e)))?;

            let result = self.process_stream(stream, self.max_rows, start).await?;

            debug!(
                "Statistics query completed: {} rows in {} ms",
                result.rows.len(),
                result.execution_time_ms
            );

            Ok(result)
        }
    }

    /// Process a query stream into a QueryResult.
    async fn process_stream(
        &self,
        mut stream: QueryStream<'_>,
        max_rows: usize,
        start: Instant,
    ) -> Result<QueryResult, McpError> {
        use futures_util::stream::TryStreamExt;

        let mut columns: Vec<ColumnInfo> = Vec::new();
        let mut rows: Vec<ResultRow> = Vec::new();
        let rows_affected: u64 = 0;
        let mut truncated = false;

        // Process result sets
        while let Some(item) = stream.try_next().await? {
            match item {
                tiberius::QueryItem::Metadata(meta) => {
                    // Extract column information
                    columns = meta
                        .columns()
                        .iter()
                        .map(|col| ColumnInfo {
                            name: col.name().to_string(),
                            sql_type: TypeMapper::sql_type_name(col).to_string(),
                            // Tiberius doesn't expose nullable info at the column type level
                            // Default to true (nullable) for safety
                            nullable: true,
                        })
                        .collect();
                }
                tiberius::QueryItem::Row(row) => {
                    if rows.len() >= max_rows {
                        truncated = true;
                        // Skip remaining rows but continue to get rows_affected
                        continue;
                    }

                    let mut result_row = ResultRow::new();
                    for (idx, col) in columns.iter().enumerate() {
                        let value = TypeMapper::extract_column(&row, idx);
                        result_row.insert(col.name.clone(), value);
                    }
                    rows.push(result_row);
                }
            }
        }

        // Note: rows_affected is only available for execute() calls, not query()
        // For SELECT queries, rows_affected stays at 0

        Ok(QueryResult {
            columns,
            rows,
            rows_affected,
            execution_time_ms: start.elapsed().as_millis() as u64,
            truncated,
        })
    }

    /// Convert results from simple_query().into_results() to our QueryResult format.
    ///
    /// This is used for SHOWPLAN queries where we need to use simple_query instead
    /// of the parameterized query method.
    fn convert_simple_query_results(
        &self,
        results: Vec<Vec<tiberius::Row>>,
        start: Instant,
    ) -> QueryResult {
        let mut columns: Vec<ColumnInfo> = Vec::new();
        let mut rows: Vec<ResultRow> = Vec::new();

        for result_set in results {
            // Process each row in the result set
            for row in result_set {
                // Extract column info from the first row if we haven't yet
                if columns.is_empty() {
                    columns = row
                        .columns()
                        .iter()
                        .map(|col| ColumnInfo {
                            name: col.name().to_string(),
                            sql_type: TypeMapper::sql_type_name(col).to_string(),
                            nullable: true,
                        })
                        .collect();
                }

                // Extract row data
                let mut result_row = ResultRow::new();
                for (idx, col) in columns.iter().enumerate() {
                    let value = TypeMapper::extract_column(&row, idx);
                    result_row.insert(col.name.clone(), value);
                }
                rows.push(result_row);
            }
        }

        QueryResult {
            columns,
            rows,
            rows_affected: 0,
            execution_time_ms: start.elapsed().as_millis() as u64,
            truncated: false,
        }
    }

    /// Execute a multi-batch query, splitting on GO separators.
    ///
    /// GO is not a T-SQL command - it's a batch separator used by tools like SSMS.
    /// This method splits the script on GO and executes each batch sequentially.
    /// Results from all batches are combined into a single result.
    pub async fn execute_multi_batch(&self, script: &str) -> Result<QueryResult, McpError> {
        let start = Instant::now();
        let batches = split_on_go(script);

        debug!("Executing multi-batch query with {} batch(es)", batches.len());

        let mut combined_columns: Vec<ColumnInfo> = Vec::new();
        let mut combined_rows: Vec<ResultRow> = Vec::new();
        let mut batch_num = 0;

        let mut conn = self.pool.get().await?;

        for batch in batches {
            let trimmed = batch.trim();
            if trimmed.is_empty() {
                continue;
            }

            batch_num += 1;
            debug!("Executing batch {}: {}", batch_num, truncate_for_log(trimmed, 100));

            // Use simple_query for each batch so DDL works correctly
            let results = conn
                .simple_query(trimmed)
                .await
                .map_err(|e| McpError::query_error(format!("Batch {} failed: {}", batch_num, e)))?
                .into_results()
                .await
                .map_err(|e| McpError::query_error(format!("Batch {} failed: {}", batch_num, e)))?;

            // Process results from this batch
            for result_set in results {
                for row in result_set {
                    // Extract column info from the first row if we haven't yet
                    if combined_columns.is_empty() {
                        combined_columns = row
                            .columns()
                            .iter()
                            .map(|col| ColumnInfo {
                                name: col.name().to_string(),
                                sql_type: TypeMapper::sql_type_name(col).to_string(),
                                nullable: true,
                            })
                            .collect();
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
        }

        let truncated = combined_rows.len() >= self.max_rows;

        debug!(
            "Multi-batch query completed: {} batches, {} rows in {} ms",
            batch_num,
            combined_rows.len(),
            start.elapsed().as_millis()
        );

        Ok(QueryResult {
            columns: combined_columns,
            rows: combined_rows,
            rows_affected: 0, // Multi-batch doesn't track rows affected
            execution_time_ms: start.elapsed().as_millis() as u64,
            truncated,
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
        assert_eq!(truncate_for_log("this is a long string", 10), "this is a ...");
    }
}
