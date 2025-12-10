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
}

/// Truncate a string for logging purposes.
fn truncate_for_log(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
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
