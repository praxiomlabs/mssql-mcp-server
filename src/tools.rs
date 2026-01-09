//! MCP Tools for SQL Server operations.
//!
//! Tools are action-oriented operations that execute queries and procedures:
//!
//! - `execute_query`: Execute arbitrary SQL queries
//! - `execute_parameterized`: Execute parameterized queries (SQL injection safe)
//! - `execute_procedure`: Execute stored procedures
//! - `execute_with_tvp`: Execute queries with Table-Valued Parameters
//! - `execute_async`: Start async query execution
//! - `get_session_status`: Check async query status
//! - `get_session_results`: Retrieve async query results
//! - `cancel_session`: Cancel running async query
//! - `explain_query`: Get query execution plan
//! - `list_sessions`: List async query sessions
//! - `health_check`: Test database connectivity
//! - `set_timeout`: Adjust default query timeout at runtime
//! - `get_timeout`: Get current query timeout configuration
//! - `execute_paginated`: Execute paginated queries
//! - `begin_transaction`: Start a database transaction
//! - `commit_transaction`: Commit a transaction
//! - `rollback_transaction`: Rollback a transaction
//! - `execute_in_transaction`: Execute SQL in a transaction
//! - `begin_pinned_session`: Start a pinned session for temp tables
//! - `execute_in_pinned_session`: Execute SQL in a pinned session
//! - `end_pinned_session`: End a pinned session
//! - `list_pinned_sessions`: List active pinned sessions
//! - `switch_database`: Switch to a different database
//! - `recommend_indexes`: Get index recommendations for a query
//! - `compare_schemas`: Compare two database schemas
//! - `compare_tables`: Compare two tables
//! - `sample_data`: Sample data from a table
//! - `bulk_insert`: Bulk insert data into a table
//! - `export_data`: Export query results
//! - `get_metrics`: Get server performance metrics
//! - `analyze_query`: Analyze query performance
//! - `get_pool_metrics`: Get connection pool statistics
//! - `get_internal_metrics`: Get internal server metrics (queries, cache, etc.)
//! - `validate_syntax`: Validate SQL syntax without executing (dry-run)

mod inputs;

pub use inputs::*;

use crate::security::{parse_qualified_name, safe_identifier, validate_identifier};
use crate::server::MssqlMcpServer;
use crate::state::{IsolationLevel, SessionStatus, TransactionStatus};
use mcpkit::prelude::*;
use mcpkit::types::ResourceContents;
use serde_json::json;
use tracing::{debug, info, warn};

/// MCP server implementation containing all tools, resources, and prompts.
///
/// The `#[mcp_server]` macro generates the MCP protocol infrastructure
/// for all `#[tool]`, `#[resource]`, and `#[prompt]` annotated methods.
#[mcp_server(
    name = "mssql-mcp-server",
    version = "0.1.0",
    instructions = "SQL Server database operations - query execution, metadata, and administration"
)]
impl MssqlMcpServer {
    // =========================================================================
    // Query Execution Tools
    // =========================================================================

    /// Execute a SQL query and return results.
    ///
    /// This tool executes arbitrary SQL queries against the database.
    /// Queries are validated according to the server's security configuration.
    #[tool(description = "Execute a SQL query and return results. Supports SELECT, INSERT, UPDATE, DELETE based on security mode.", destructive = true)]
    pub async fn execute_query(
        &self,
        input: ExecuteQueryInput,
    ) -> Result<ToolOutput, McpError> {
        use crate::database::QueryExecutor;

        debug!("Executing query: {}", truncate_for_log(&input.query, 100));

        // Validate the query
        if let Err(e) = self.validate_query(&input.query) {
            return Ok(ToolOutput::error(format!("Query validation failed: {}", e)));
        }

        // Get current database from state (for switch_database support)
        // Pool connections don't persist database context, so we need to prepend USE
        let current_db = {
            let state = self.state.read().await;
            state.current_database().map(|s| s.to_string())
        };

        // Determine row limit
        let max_rows = input
            .max_rows
            .unwrap_or(self.config.security.max_result_rows);

        // Check execution mode on the ORIGINAL query (before USE prefix)
        // This ensures pattern detection works correctly for batch-first DDL
        if QueryExecutor::contains_go_separator(&input.query) {
            // Multi-batch query with GO separators
            // Pass database context so each batch gets the USE prefix
            debug!("Using multi-batch execution for script with GO separators");
            let result = match self
                .executor
                .execute_multi_batch_with_db(&input.query, current_db.as_deref())
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("Multi-batch execution failed: {}", e);
                    return Ok(ToolOutput::error(format!("Query execution failed: {}", e)));
                }
            };

            // Format output based on requested format
            let output = match input.format {
                OutputFormat::Json => serde_json::to_string_pretty(&result).unwrap_or_else(|e| {
                    warn!("Failed to serialize query result to JSON: {}", e);
                    format!("Failed to serialize result: {}", e)
                }),
                OutputFormat::Csv => result.to_csv(),
                OutputFormat::Table => result.to_markdown_table(),
            };

            return Ok(ToolOutput::text(output));
        }

        if QueryExecutor::requires_raw_execution(&input.query) {
            // Batch-first DDL statements (CREATE VIEW/PROC/FUNC/TRIGGER/SCHEMA)
            // must be executed using simple_query to avoid sp_executesql wrapper
            debug!("Using raw execution for batch-first DDL statement");
            let effective_query = match &current_db {
                Some(db) => format!("USE [{}];\n{}", db, input.query),
                None => input.query.clone(),
            };
            let result = match self.executor.execute_raw(&effective_query).await {
                Ok(r) => r,
                Err(e) => {
                    warn!("Raw query execution failed: {}", e);
                    return Ok(ToolOutput::error(format!("Query execution failed: {}", e)));
                }
            };

            // Format output based on requested format
            let output = match input.format {
                OutputFormat::Json => serde_json::to_string_pretty(&result).unwrap_or_else(|e| {
                    warn!("Failed to serialize query result to JSON: {}", e);
                    format!("Failed to serialize result: {}", e)
                }),
                OutputFormat::Csv => result.to_csv(),
                OutputFormat::Table => result.to_markdown_table(),
            };

            return Ok(ToolOutput::text(output));
        }

        // Check for multiple result sets (multiple SELECT statements)
        if QueryExecutor::has_multiple_result_sets(&input.query) {
            debug!("Using multi-result execution for query with multiple SELECTs");
            let effective_query = match &current_db {
                Some(db) => format!("USE [{}];\n{}", db, input.query),
                None => input.query.clone(),
            };
            let result = match self
                .executor
                .execute_multi_result(&effective_query, max_rows)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("Multi-result query execution failed: {}", e);
                    return Ok(ToolOutput::error(format!("Query execution failed: {}", e)));
                }
            };

            // Format output based on requested format
            let output = match input.format {
                OutputFormat::Json => serde_json::to_string_pretty(&result).unwrap_or_else(|e| {
                    warn!("Failed to serialize query result to JSON: {}", e);
                    format!("Failed to serialize result: {}", e)
                }),
                OutputFormat::Csv => result.to_csv(),
                OutputFormat::Table => result.to_markdown_table(),
            };

            return Ok(ToolOutput::text(output));
        }

        // Standard execution with optional database context
        let effective_query = match &current_db {
            Some(db) => format!("USE [{}];\n{}", db, input.query),
            None => input.query.clone(),
        };
        let result = match self
            .executor
            .execute_with_options(&effective_query, max_rows, input.timeout_seconds)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("Query execution failed: {}", e);
                return Ok(ToolOutput::error(format!("Query execution failed: {}", e)));
            }
        };

        // Format output based on requested format
        let output = match input.format {
            OutputFormat::Json => serde_json::to_string_pretty(&result).unwrap_or_else(|e| {
                warn!("Failed to serialize query result to JSON: {}", e);
                format!("Failed to serialize result: {}", e)
            }),
            OutputFormat::Csv => result.to_csv(),
            OutputFormat::Table => result.to_markdown_table(),
        };

        Ok(ToolOutput::text(output))
    }

    /// Explain a SQL query's execution plan.
    ///
    /// Returns the estimated or actual execution plan for analysis.
    #[tool(description = "Get the execution plan for a SQL query. Useful for query optimization.", read_only = true, idempotent = true)]
    pub async fn explain_query(
        &self,
        input: ExplainQueryInput,
    ) -> Result<ToolOutput, McpError> {
        debug!("Explaining query: {}", truncate_for_log(&input.query, 100));

        // Use the executor's showplan method which handles the batch separation correctly
        let result = match self
            .executor
            .execute_with_showplan(&input.query, &input.plan_type)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolOutput::error(format!("Failed to get execution plan: {}", e)));
            }
        };

        let output = result.to_markdown_table();
        Ok(ToolOutput::text(output))
    }

    // =========================================================================
    // Stored Procedure Tools
    // =========================================================================

    /// Execute a stored procedure.
    ///
    /// This tool executes stored procedures with parameter binding.
    #[tool(description = "Execute a stored procedure with parameters. Returns result sets and output parameters.", destructive = true)]
    pub async fn execute_procedure(
        &self,
        input: ExecuteProcedureInput,
    ) -> Result<ToolOutput, McpError> {
        debug!("Executing procedure: {}.{}", input.schema, input.procedure);

        // Build the EXEC statement with validated and escaped identifiers
        let escaped_schema = match safe_identifier(&input.schema) {
            Ok(s) => s,
            Err(e) => return Ok(ToolOutput::error(format!("Invalid schema name: {}", e))),
        };
        let escaped_procedure = match safe_identifier(&input.procedure) {
            Ok(p) => p,
            Err(e) => return Ok(ToolOutput::error(format!("Invalid procedure name: {}", e))),
        };
        let proc_name = format!("{}.{}", escaped_schema, escaped_procedure);

        // Build parameter list
        let params = if input.parameters.is_empty() {
            String::new()
        } else {
            let param_strs: Vec<String> = input
                .parameters
                .iter()
                .map(|(name, value)| {
                    let param_name = if name.starts_with('@') {
                        name.clone()
                    } else {
                        format!("@{}", name)
                    };
                    let param_value = format_parameter_value(value);
                    format!("{} = {}", param_name, param_value)
                })
                .collect();
            format!(" {}", param_strs.join(", "))
        };

        let query = format!("EXEC {}{}", proc_name, params);

        // Execute the procedure
        let result = match self.executor.execute(&query).await {
            Ok(r) => r,
            Err(e) => {
                warn!("Procedure execution failed: {}", e);
                return Ok(ToolOutput::error(format!("Procedure execution failed: {}", e)));
            }
        };

        let output = result.to_markdown_table();
        Ok(ToolOutput::text(output))
    }

    /// Execute a query with a Table-Valued Parameter (TVP).
    ///
    /// TVPs allow passing structured data to stored procedures as a single parameter.
    /// This is more efficient than multiple INSERT statements or temporary tables for
    /// bulk operations.
    ///
    /// Prerequisites:
    /// - A table type must exist in the database (CREATE TYPE schema.TypeName AS TABLE...)
    /// - The query should reference the TVP parameter (e.g., @p1 or @tvp)
    #[tool(description = "Execute a query or stored procedure with a Table-Valued Parameter (TVP). Enables efficient bulk data passing to stored procedures.", destructive = true)]
    pub async fn execute_with_tvp(
        &self,
        input: ExecuteWithTvpInput,
    ) -> Result<ToolOutput, McpError> {
        use crate::database::QueryExecutor;

        debug!(
            "Executing query with TVP: type={}, rows={}",
            input.tvp_type_name,
            input.rows.len()
        );

        // Validate columns
        if input.columns.is_empty() {
            return Ok(ToolOutput::error("At least one column definition is required for TVP"));
        }

        // Validate rows have correct column count
        for (idx, row) in input.rows.iter().enumerate() {
            if row.len() != input.columns.len() {
                return Ok(ToolOutput::error(format!(
                    "Row {} has {} values but {} columns defined",
                    idx,
                    row.len(),
                    input.columns.len()
                )));
            }
        }

        // Build column definitions
        let columns: Vec<(String, String)> = input
            .columns
            .iter()
            .map(|c| (c.name.clone(), c.sql_type.clone()))
            .collect();

        // Build the TVP
        let tvp = match QueryExecutor::build_tvp(&input.tvp_type_name, &columns, &input.rows) {
            Ok(t) => t,
            Err(e) => {
                return Ok(ToolOutput::error(format!("Failed to build TVP: {}", e)));
            }
        };

        // Execute the query with the TVP parameter
        let max_rows = self.config.security.max_result_rows;
        let result = match self.executor.execute_with_tvp(&input.query, tvp, max_rows).await {
            Ok(r) => r,
            Err(e) => {
                warn!("TVP query execution failed: {}", e);
                return Ok(ToolOutput::error(format!("TVP query execution failed: {}", e)));
            }
        };

        let output = match input.format {
            OutputFormat::Json => serde_json::to_string_pretty(&result).unwrap_or_else(|e| {
                warn!("Failed to serialize TVP query result to JSON: {}", e);
                format!("Failed to serialize result: {}", e)
            }),
            OutputFormat::Csv => result.to_csv(),
            OutputFormat::Table => result.to_markdown_table(),
        };

        Ok(ToolOutput::text(output))
    }

    // =========================================================================
    // Async Session Tools
    // =========================================================================

    /// Start an asynchronous query execution.
    ///
    /// Returns a session ID that can be used to check status and retrieve results.
    /// Supports native SQL Server query cancellation via the `cancel_session` tool.
    #[tool(description = "Start an asynchronous query execution. Returns a session ID to check status later.", destructive = true)]
    pub async fn execute_async(
        &self,
        input: ExecuteAsyncInput,
    ) -> Result<ToolOutput, McpError> {
        debug!("Starting async query execution");

        // Validate the query
        if let Err(e) = self.validate_query(&input.query) {
            return Ok(ToolOutput::error(format!("Query validation failed: {}", e)));
        }

        // Create a new session
        let session_id = {
            let mut state = self.state.write().await;
            match state.create_session(input.query.clone(), self.config.session.max_sessions) {
                Ok(id) => id,
                Err(e) => {
                    return Ok(ToolOutput::error(format!("Failed to create session: {}", e)));
                }
            }
        };

        // Get a connection from the pool to access the cancel handle
        let mut conn = match self.pool.get().await {
            Ok(c) => c,
            Err(e) => {
                // Clean up the session we just created
                let mut state = self.state.write().await;
                if let Some(session) = state.get_session_mut(&session_id) {
                    session.fail(format!("Failed to get connection: {}", e));
                }
                return Ok(ToolOutput::error(format!(
                    "Failed to get connection from pool: {}",
                    e
                )));
            }
        };

        // Extract cancel handle before moving connection to spawned task
        let cancel_handle = conn.client().map(|c| c.cancel_handle());
        if let Some(ref handle) = cancel_handle {
            let mut state = self.state.write().await;
            state.store_cancel_handle(&session_id, handle.clone());
        }

        // Spawn the async execution task with the connection
        let state = self.state.clone();
        let max_rows = input
            .max_rows
            .unwrap_or(self.config.security.max_result_rows);
        let timeout_seconds = input.timeout_seconds;
        let query = input.query;
        let sid = session_id.clone();

        tokio::spawn(async move {
            use crate::database::{QueryColumnInfo as ColumnInfo, QueryResult, ResultRow, TypeMapper};
            use futures_util::TryStreamExt;
            use std::time::{Duration, Instant};

            let start = Instant::now();

            // Execute the query on the dedicated connection
            let result = async {
                let stream = conn
                    .query(&query, &[])
                    .await
                    .map_err(|e| format!("Query execution failed: {}", e))?;

                // Process the stream with row limit
                let mut columns: Vec<ColumnInfo> = Vec::new();
                let mut rows = Vec::new();
                let mut truncated = false;
                let mut row_count = 0;

                futures_util::pin_mut!(stream);
                while let Some(row) = stream.try_next().await.map_err(|e| format!("Failed to read row: {}", e))? {
                    // Extract column info from first row
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

                    if row_count >= max_rows {
                        truncated = true;
                        break;
                    }

                    let mut result_row = ResultRow::new();
                    for (col_idx, col) in columns.iter().enumerate() {
                        let value = TypeMapper::extract_column(&row, col_idx);
                        result_row.insert(col.name.clone(), value);
                    }
                    rows.push(result_row);
                    row_count += 1;
                }

                Ok::<_, String>(QueryResult {
                    columns,
                    rows,
                    rows_affected: 0,
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    truncated,
                })
            };

            // Apply timeout if specified
            let result = if let Some(secs) = timeout_seconds {
                match tokio::time::timeout(Duration::from_secs(secs), result).await {
                    Ok(r) => r,
                    Err(_) => Err(format!("Query timed out after {} seconds", secs)),
                }
            } else {
                result.await
            };

            // Update session state and clean up cancel handle
            let mut state = state.write().await;
            // Remove the cancel handle now that the query is complete
            state.remove_cancel_handle(&sid);

            if let Some(session) = state.get_session_mut(&sid) {
                match result {
                    Ok(r) => {
                        info!("Async query {} completed successfully", sid);
                        session.complete(r);
                    }
                    Err(e) => {
                        warn!("Async query {} failed: {}", sid, e);
                        session.fail(e);
                    }
                }
            }
        });

        let response = json!({
            "session_id": session_id,
            "status": "running",
            "message": "Query execution started. Use get_session_status to check progress.",
            "cancellable": cancel_handle.is_some()
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| format!("Session ID: {}", session_id)),
        ))
    }

    /// Get the status of an async query session.
    #[tool(description = "Get the status and results of an async query session.", read_only = true, idempotent = true)]
    pub async fn get_session_status(
        &self,
        input: GetSessionStatusInput,
    ) -> Result<ToolOutput, McpError> {
        let state = self.state.read().await;

        let session = match state.get_session(&input.session_id) {
            Some(s) => s,
            None => {
                return Ok(ToolOutput::error(format!(
                    "Session not found: {}",
                    input.session_id
                )));
            }
        };

        let mut response = json!({
            "session_id": session.id,
            "status": session.status.to_string(),
            "progress": session.progress,
            "created_at": session.created_at.to_rfc3339(),
            "updated_at": session.updated_at.to_rfc3339(),
            "age_seconds": session.age_seconds(),
        });

        // Add error message if failed
        if let Some(ref error) = session.error {
            response["error"] = json!(error);
        }

        // Add results if completed and requested
        if input.include_results && session.status == SessionStatus::Completed {
            if let Some(ref result) = session.result {
                response["result"] = json!({
                    "row_count": result.rows.len(),
                    "columns": result.columns.iter().map(|c| &c.name).collect::<Vec<_>>(),
                    "execution_time_ms": result.execution_time_ms,
                    "truncated": result.truncated,
                    "data": result.to_markdown_table(),
                });
            }
        }

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| "Error serializing response".to_string()),
        ))
    }

    /// Cancel a running async query session.
    ///
    /// This uses native SQL Server query cancellation via Attention packets
    /// when available, which stops the query execution on the server side.
    #[tool(description = "Cancel a running async query session.", destructive = true, idempotent = true)]
    pub async fn cancel_session(
        &self,
        input: CancelSessionInput,
    ) -> Result<ToolOutput, McpError> {
        // First, check session exists and is running
        let (is_running, has_cancel_handle) = {
            let state = self.state.read().await;
            match state.get_session(&input.session_id) {
                Some(s) => (s.is_running(), state.has_cancel_handle(&input.session_id)),
                None => {
                    return Ok(ToolOutput::error(format!(
                        "Session not found: {}",
                        input.session_id
                    )));
                }
            }
        };

        if !is_running {
            let state = self.state.read().await;
            if let Some(session) = state.get_session(&input.session_id) {
                return Ok(ToolOutput::error(format!(
                    "Session {} is not running (status: {})",
                    input.session_id, session.status
                )));
            }
        }

        // Attempt native SQL Server cancellation if cancel handle is available
        let native_cancel_result = if has_cancel_handle {
            // Get the cancel handle and call cancel
            // Note: We need to clone the handle since we can't hold state lock during async cancel
            let cancel_handle = {
                let state = self.state.read().await;
                state.get_cancel_handle(&input.session_id).cloned()
            };

            if let Some(handle) = cancel_handle {
                debug!("Sending native SQL Server cancellation for session {}", input.session_id);
                match handle.cancel().await {
                    Ok(()) => {
                        info!("Native cancellation sent for session {}", input.session_id);
                        Some(Ok(()))
                    }
                    Err(e) => {
                        warn!("Native cancellation failed for session {}: {}", input.session_id, e);
                        Some(Err(e.to_string()))
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        // Update session state
        let mut state = self.state.write().await;

        // Remove the cancel handle
        state.remove_cancel_handle(&input.session_id);

        if let Some(session) = state.get_session_mut(&input.session_id) {
            session.cancel();
        }

        info!("Session {} cancelled", input.session_id);

        let message = match &native_cancel_result {
            Some(Ok(())) => "Session cancelled with native SQL Server cancellation".to_string(),
            Some(Err(e)) => format!("Session cancelled (native cancellation failed: {})", e),
            None => "Session cancelled (no native cancel handle available)".to_string(),
        };

        let response = json!({
            "session_id": input.session_id,
            "status": "cancelled",
            "native_cancellation": native_cancel_result.as_ref().map(|r| r.is_ok()).unwrap_or(false),
            "message": message
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| "Session cancelled".to_string()),
        ))
    }

    /// List all async query sessions.
    #[tool(description = "List all async query sessions with optional status filter.", read_only = true, idempotent = true)]
    pub async fn list_sessions(
        &self,
        input: ListSessionsInput,
    ) -> Result<ToolOutput, McpError> {
        let state = self.state.read().await;

        let sessions = match input.status.to_lowercase().as_str() {
            "running" => state.list_sessions_by_status(SessionStatus::Running),
            "completed" => state.list_sessions_by_status(SessionStatus::Completed),
            "failed" => state.list_sessions_by_status(SessionStatus::Failed),
            "cancelled" => state.list_sessions_by_status(SessionStatus::Cancelled),
            _ => state.list_sessions(),
        };

        let response = json!({
            "total_count": sessions.len(),
            "running_count": state.running_session_count(),
            "sessions": sessions,
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| "Error listing sessions".to_string()),
        ))
    }

    /// Get the results of an async query session.
    ///
    /// Retrieves the results from a completed async query session with formatting options.
    #[tool(description = "Get the results of a completed async query session with formatting options.", read_only = true, idempotent = true)]
    pub async fn get_session_results(
        &self,
        input: GetSessionResultsInput,
    ) -> Result<ToolOutput, McpError> {
        let state = self.state.read().await;

        let session = match state.get_session(&input.session_id) {
            Some(s) => s,
            None => {
                return Ok(ToolOutput::error(format!(
                    "Session not found: {}",
                    input.session_id
                )));
            }
        };

        // Check if session is completed
        if session.status != SessionStatus::Completed {
            return Ok(ToolOutput::error(format!(
                "Session {} is not completed (status: {}). Use get_session_status to check progress.",
                input.session_id, session.status
            )));
        }

        // Get the results
        let result = match &session.result {
            Some(r) => r,
            None => {
                return Ok(ToolOutput::error("Session completed but no results available"));
            }
        };

        // Apply row limit if specified
        let rows_to_show = input.max_rows.unwrap_or(result.rows.len());
        let truncated_by_request = rows_to_show < result.rows.len();

        // Format output based on requested format
        let output = match input.format {
            OutputFormat::Json => {
                let limited_result = if truncated_by_request {
                    let mut limited = result.clone();
                    limited.rows.truncate(rows_to_show);
                    limited.truncated = true;
                    limited
                } else {
                    result.clone()
                };
                serde_json::to_string_pretty(&limited_result).unwrap_or_else(|e| {
                    warn!("Failed to serialize session result to JSON: {}", e);
                    format!("Failed to serialize result: {}", e)
                })
            }
            OutputFormat::Csv => {
                if truncated_by_request {
                    let mut limited = result.clone();
                    limited.rows.truncate(rows_to_show);
                    limited.to_csv()
                } else {
                    result.to_csv()
                }
            }
            OutputFormat::Table => {
                if truncated_by_request {
                    let mut limited = result.clone();
                    limited.rows.truncate(rows_to_show);
                    limited.to_markdown_table()
                } else {
                    result.to_markdown_table()
                }
            }
        };

        Ok(ToolOutput::text(output))
    }

    // =========================================================================
    // Diagnostics Tools
    // =========================================================================

    /// Check database connectivity and health.
    ///
    /// Returns connection status and optionally detailed diagnostics.
    #[tool(description = "Test database connectivity and return health status with optional diagnostics.", read_only = true, idempotent = true)]
    pub async fn health_check(
        &self,
        input: HealthCheckInput,
    ) -> Result<ToolOutput, McpError> {
        debug!("Performing health check (detailed: {})", input.detailed);

        let start = std::time::Instant::now();

        // Try to execute a simple query to test connectivity
        let connectivity_result = self.executor.execute("SELECT 1 AS health_check").await;

        let latency_ms = start.elapsed().as_millis() as u64;
        let healthy = connectivity_result.is_ok();

        let mut response = json!({
            "healthy": healthy,
            "latency_ms": latency_ms,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });

        if !healthy {
            if let Err(ref e) = connectivity_result {
                response["error"] = json!(e.to_string());
            }
        }

        // Add detailed diagnostics if requested
        if input.detailed && healthy {
            // Get server info
            match self.metadata.get_server_info().await {
                Ok(info) => {
                    response["server"] = json!({
                        "version": info.product_version,
                        "edition": info.edition,
                        "server_name": info.server_name,
                        "collation": info.collation,
                    });
                }
                Err(e) => {
                    response["server_info_error"] = json!(e.to_string());
                }
            }

            // Get pool statistics
            let pool_status = self.pool.status();
            response["pool"] = json!({
                "total_connections": pool_status.total,
                "available_connections": pool_status.available,
                "in_use_connections": pool_status.in_use,
                "max_connections": pool_status.max,
            });

            // Get session statistics
            let state = self.state.read().await;
            response["sessions"] = json!({
                "total": state.total_session_count(),
                "running": state.running_session_count(),
            });

            // Configuration summary (includes runtime-modifiable settings)
            response["config"] = json!({
                "validation_mode": format!("{:?}", self.config.security.validation_mode),
                "max_result_rows": self.config.security.max_result_rows,
                "query_timeout_seconds": state.default_timeout(),
                "initial_timeout_seconds": self.config.query.default_timeout.as_secs(),
            });
        }

        let status_text = if healthy { "healthy" } else { "unhealthy" };
        info!("Health check completed: {} ({}ms)", status_text, latency_ms);

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| format!("Health: {}", status_text)),
        ))
    }

    /// Set the default query timeout.
    ///
    /// Adjusts the default timeout for query execution at runtime.
    /// This affects all subsequent queries that don't specify their own timeout.
    #[tool(description = "Set the default query timeout in seconds. Affects subsequent query executions.", idempotent = true)]
    pub async fn set_timeout(
        &self,
        input: SetTimeoutInput,
    ) -> Result<ToolOutput, McpError> {
        // Validate timeout range (1 second to 1 hour)
        if input.timeout_seconds < 1 || input.timeout_seconds > 3600 {
            return Ok(ToolOutput::error(
                "Timeout must be between 1 and 3600 seconds (1 hour)",
            ));
        }

        // Get current timeout and update to new value
        let old_timeout_secs = {
            let state = self.state.read().await;
            state.default_timeout()
        };

        // Update the runtime timeout in shared state
        {
            let mut state = self.state.write().await;
            state.set_default_timeout(input.timeout_seconds);
        }

        info!(
            "Timeout changed: {}s -> {}s",
            old_timeout_secs, input.timeout_seconds
        );

        let response = json!({
            "previous_timeout_seconds": old_timeout_secs,
            "new_timeout_seconds": input.timeout_seconds,
            "status": "applied",
            "note": "Default timeout updated. All subsequent queries will use this timeout unless overridden."
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| "Timeout updated".to_string()),
        ))
    }

    /// Get the current query timeout configuration.
    ///
    /// Returns the current default timeout and related configuration.
    #[tool(description = "Get the current default query timeout and related configuration.", read_only = true, idempotent = true)]
    pub async fn get_timeout(
        &self,
        input: GetTimeoutInput,
    ) -> Result<ToolOutput, McpError> {
        let state = self.state.read().await;
        let current_timeout = state.default_timeout();
        let initial_timeout = self.config.query.default_timeout.as_secs();
        let max_timeout = self.config.query.max_timeout.as_secs();

        let mut response = json!({
            "current_timeout_seconds": current_timeout,
            "initial_timeout_seconds": initial_timeout,
            "max_timeout_seconds": max_timeout,
        });

        if input.detailed {
            response["is_modified"] = json!(current_timeout != initial_timeout);
            response["caching_enabled"] = json!(self.config.query.enable_caching);
            if self.config.query.enable_caching {
                response["cache_ttl_seconds"] = json!(self.config.query.cache_ttl.as_secs());
            }
        }

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| format!("Current timeout: {}s", current_timeout)),
        ))
    }

    // =========================================================================
    // Parameterized Query Tools
    // =========================================================================

    /// Execute a parameterized SQL query.
    ///
    /// This tool provides safe execution of queries with parameters,
    /// preventing SQL injection at the protocol level.
    #[tool(description = "Execute a SQL query with parameters. Safer than raw queries as parameters are bound separately.", destructive = true)]
    pub async fn execute_parameterized(
        &self,
        input: ExecuteParameterizedInput,
    ) -> Result<ToolOutput, McpError> {
        debug!(
            "Executing parameterized query: {}",
            truncate_for_log(&input.query, 100)
        );

        // Validate the query structure (without parameters)
        if let Err(e) = self.validate_query(&input.query) {
            return Ok(ToolOutput::error(format!("Query validation failed: {}", e)));
        }

        // Build the query with parameterized values
        // For SQL Server, we use sp_executesql for true parameterization
        let (exec_query, param_declarations, param_values) =
            build_parameterized_query(&input.query, &input.parameters)?;

        let full_query = if param_declarations.is_empty() {
            input.query.clone()
        } else {
            format!(
                "EXEC sp_executesql N'{}', N'{}', {}",
                exec_query.replace('\'', "''"),
                param_declarations,
                param_values
            )
        };

        let max_rows = input
            .max_rows
            .unwrap_or(self.config.security.max_result_rows);

        let result = match self
            .executor
            .execute_with_limit(&full_query, max_rows)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("Parameterized query execution failed: {}", e);
                return Ok(ToolOutput::error(format!("Query execution failed: {}", e)));
            }
        };

        let output = match input.format {
            OutputFormat::Json => serde_json::to_string_pretty(&result).unwrap_or_else(|e| {
                warn!(
                    "Failed to serialize parameterized query result to JSON: {}",
                    e
                );
                format!("Failed to serialize result: {}", e)
            }),
            OutputFormat::Csv => result.to_csv(),
            OutputFormat::Table => result.to_markdown_table(),
        };

        Ok(ToolOutput::text(output))
    }

    // =========================================================================
    // Transaction Control Tools
    // =========================================================================

    /// Begin a new database transaction.
    #[tool(description = "Start a new database transaction with optional name and isolation level.", destructive = true)]
    pub async fn begin_transaction(
        &self,
        input: BeginTransactionInput,
    ) -> Result<ToolOutput, McpError> {
        debug!(
            "Beginning transaction with isolation: {}",
            input.isolation_level
        );

        // Parse isolation level
        let isolation_level = input
            .isolation_level
            .parse::<IsolationLevel>()
            .unwrap_or_default();

        // Create transaction state (this generates the transaction ID)
        let transaction_id = {
            let mut state = self.state.write().await;
            match state.create_transaction(
                input.name.clone(),
                isolation_level,
                self.config.session.max_sessions, // Use same limit for transactions
            ) {
                Ok(id) => id,
                Err(e) => {
                    return Ok(ToolOutput::error(format!("Failed to create transaction: {}", e)));
                }
            }
        };

        // Use TransactionManager to create dedicated connection and begin transaction
        if let Err(e) = self
            .transaction_manager
            .begin_transaction(
                &transaction_id,
                isolation_level,
                input.name.as_deref(),
            )
            .await
        {
            // Clean up state on failure
            let mut state = self.state.write().await;
            state.remove_transaction(&transaction_id);
            return Ok(ToolOutput::error(format!("Failed to begin transaction: {}", e)));
        }

        info!(
            "Transaction {} started with dedicated connection",
            transaction_id
        );

        let response = json!({
            "transaction_id": transaction_id,
            "name": input.name,
            "isolation_level": isolation_level.to_string(),
            "status": "active",
            "message": "Transaction started. Use execute_in_transaction to run queries, then commit_transaction or rollback_transaction."
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| format!("Transaction ID: {}", transaction_id)),
        ))
    }

    /// Commit a transaction.
    #[tool(description = "Commit a transaction, making all changes permanent.", destructive = true)]
    pub async fn commit_transaction(
        &self,
        input: CommitTransactionInput,
    ) -> Result<ToolOutput, McpError> {
        debug!("Committing transaction: {}", input.transaction_id);

        // Check transaction exists and is active, get the name
        let tx_name = {
            let state = self.state.read().await;
            match state.get_transaction(&input.transaction_id) {
                Some(tx) if tx.status != TransactionStatus::Active => {
                    return Ok(ToolOutput::error(format!(
                        "Transaction {} is not active (status: {})",
                        input.transaction_id, tx.status
                    )));
                }
                None => {
                    return Ok(ToolOutput::error(format!(
                        "Transaction not found: {}",
                        input.transaction_id
                    )));
                }
                Some(tx) => tx.name.clone(),
            }
        };

        // Use TransactionManager to commit on the dedicated connection
        if let Err(e) = self
            .transaction_manager
            .commit_transaction(&input.transaction_id, tx_name.as_deref())
            .await
        {
            return Ok(ToolOutput::error(format!("Failed to commit transaction: {}", e)));
        }

        // Update state
        let statement_count = {
            let mut state = self.state.write().await;
            if let Some(tx) = state.get_transaction_mut(&input.transaction_id) {
                tx.commit();
                tx.statement_count
            } else {
                0
            }
        };

        info!("Transaction {} committed", input.transaction_id);

        let response = json!({
            "transaction_id": input.transaction_id,
            "status": "committed",
            "statements_executed": statement_count,
            "message": "Transaction committed successfully"
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| "Transaction committed".to_string()),
        ))
    }

    /// Rollback a transaction.
    #[tool(description = "Rollback a transaction, undoing all changes.", destructive = true, idempotent = true)]
    pub async fn rollback_transaction(
        &self,
        input: RollbackTransactionInput,
    ) -> Result<ToolOutput, McpError> {
        debug!("Rolling back transaction: {}", input.transaction_id);

        // Check transaction exists and is active
        let tx_name = {
            let state = self.state.read().await;
            match state.get_transaction(&input.transaction_id) {
                Some(tx) if tx.status != TransactionStatus::Active => {
                    return Ok(ToolOutput::error(format!(
                        "Transaction {} is not active (status: {})",
                        input.transaction_id, tx.status
                    )));
                }
                None => {
                    return Ok(ToolOutput::error(format!(
                        "Transaction not found: {}",
                        input.transaction_id
                    )));
                }
                Some(tx) => tx.name.clone(),
            }
        };

        // Use TransactionManager to rollback on the dedicated connection
        let transaction_ended = match self
            .transaction_manager
            .rollback_transaction(
                &input.transaction_id,
                tx_name.as_deref(),
                input.savepoint.as_deref(),
            )
            .await
        {
            Ok(ended) => ended,
            Err(e) => {
                return Ok(ToolOutput::error(format!("Failed to rollback transaction: {}", e)));
            }
        };

        // Update state (only if full rollback, not savepoint)
        if transaction_ended {
            let mut state = self.state.write().await;
            if let Some(tx) = state.get_transaction_mut(&input.transaction_id) {
                tx.rollback();
            }
        }

        info!(
            "Transaction {} rolled back (savepoint: {:?})",
            input.transaction_id, input.savepoint
        );

        let response = json!({
            "transaction_id": input.transaction_id,
            "status": if input.savepoint.is_some() { "active" } else { "rolled_back" },
            "savepoint": input.savepoint,
            "message": if input.savepoint.is_some() {
                "Rolled back to savepoint"
            } else {
                "Transaction rolled back successfully"
            }
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| "Transaction rolled back".to_string()),
        ))
    }

    /// Execute SQL within a transaction.
    #[tool(description = "Execute a SQL statement within an active transaction.", destructive = true)]
    pub async fn execute_in_transaction(
        &self,
        input: ExecuteInTransactionInput,
    ) -> Result<ToolOutput, McpError> {
        debug!(
            "Executing in transaction {}: {}",
            input.transaction_id,
            truncate_for_log(&input.query, 100)
        );

        // Validate transaction is active
        {
            let state = self.state.read().await;
            match state.get_transaction(&input.transaction_id) {
                Some(tx) if tx.status != TransactionStatus::Active => {
                    return Ok(ToolOutput::error(format!(
                        "Transaction {} is not active (status: {})",
                        input.transaction_id, tx.status
                    )));
                }
                None => {
                    return Ok(ToolOutput::error(format!(
                        "Transaction not found: {}",
                        input.transaction_id
                    )));
                }
                _ => {}
            }
        }

        // Validate the query
        if let Err(e) = self.validate_query(&input.query) {
            return Ok(ToolOutput::error(format!("Query validation failed: {}", e)));
        }

        // Build query with parameters if provided
        let query = if input.parameters.is_empty() {
            input.query.clone()
        } else {
            let (exec_query, param_declarations, param_values) =
                build_parameterized_query(&input.query, &input.parameters)?;
            if param_declarations.is_empty() {
                input.query.clone()
            } else {
                format!(
                    "EXEC sp_executesql N'{}', N'{}', {}",
                    exec_query.replace('\'', "''"),
                    param_declarations,
                    param_values
                )
            }
        };

        // Execute the query using TransactionManager on the dedicated connection
        let result = match self
            .transaction_manager
            .execute_in_transaction(&input.transaction_id, &query)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("Transaction query failed: {}", e);
                return Ok(ToolOutput::error(format!("Query execution failed: {}", e)));
            }
        };

        // Update transaction state
        {
            let mut state = self.state.write().await;
            if let Some(tx) = state.get_transaction_mut(&input.transaction_id) {
                tx.record_statement();
            }
        }

        let output = result.to_markdown_table();
        Ok(ToolOutput::text(output))
    }

    // =========================================================================
    // Pinned Session Tools (for temp tables, session state)
    // =========================================================================

    /// Begin a pinned session for temp tables and session state.
    ///
    /// Pinned sessions hold a dedicated connection, allowing temp tables (#tables),
    /// session variables, and SET options to persist across multiple queries.
    #[tool(description = "Start a pinned session with a dedicated connection. Use for temp tables (#tables) and session-scoped state that needs to persist across queries.", destructive = true)]
    pub async fn begin_pinned_session(
        &self,
        input: BeginPinnedSessionInput,
    ) -> Result<ToolOutput, McpError> {
        debug!("Beginning pinned session");

        // Generate session ID
        let session_id = format!(
            "session_{}",
            uuid::Uuid::new_v4()
                .to_string()
                .split('-')
                .next()
                .unwrap_or("unknown")
        );

        // Create the session
        let session_info = match self.session_manager.begin_session(&session_id).await {
            Ok(info) => info,
            Err(e) => {
                return Ok(ToolOutput::error(format!("Failed to begin session: {}", e)));
            }
        };

        info!(
            "Pinned session {} started with dedicated connection",
            session_id
        );

        let response = json!({
            "session_id": session_id,
            "name": input.name,
            "status": "active",
            "created_at": format!("{:?}", session_info.created_at.elapsed()),
            "message": "Pinned session started. Use execute_in_pinned_session for temp tables and session state. Remember to call end_pinned_session when done."
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| format!("Session ID: {}", session_id)),
        ))
    }

    /// Execute SQL within a pinned session.
    #[tool(description = "Execute a SQL statement within a pinned session. Temp tables and session state persist across calls.", destructive = true)]
    pub async fn execute_in_pinned_session(
        &self,
        input: ExecuteInPinnedSessionInput,
    ) -> Result<ToolOutput, McpError> {
        debug!(
            "Executing in pinned session {}: {}",
            input.session_id,
            truncate_for_log(&input.query, 100)
        );

        // Validate the query
        if let Err(e) = self.validate_query(&input.query) {
            return Ok(ToolOutput::error(format!("Query validation failed: {}", e)));
        }

        // Execute using SessionManager
        let result = match self
            .session_manager
            .execute_in_session(&input.session_id, &input.query)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("Session query failed: {}", e);
                return Ok(ToolOutput::error(format!("Query execution failed: {}", e)));
            }
        };

        // Format output based on requested format
        let output = match input.format {
            OutputFormat::Json => serde_json::to_string_pretty(&result)
                .unwrap_or_else(|e| format!("Failed to serialize result: {}", e)),
            OutputFormat::Csv => result.to_csv(),
            OutputFormat::Table => result.to_markdown_table(),
        };

        Ok(ToolOutput::text(output))
    }

    /// End a pinned session and release its connection.
    #[tool(description = "End a pinned session and release its dedicated connection. Any temp tables will be automatically dropped.", destructive = true, idempotent = true)]
    pub async fn end_pinned_session(
        &self,
        input: EndPinnedSessionInput,
    ) -> Result<ToolOutput, McpError> {
        debug!("Ending pinned session: {}", input.session_id);

        let session_info = match self.session_manager.end_session(&input.session_id).await {
            Ok(info) => info,
            Err(e) => {
                return Ok(ToolOutput::error(format!("Failed to end session: {}", e)));
            }
        };

        info!(
            "Pinned session {} ended after {} queries",
            input.session_id, session_info.query_count
        );

        let response = json!({
            "session_id": input.session_id,
            "status": "ended",
            "queries_executed": session_info.query_count,
            "duration_ms": session_info.created_at.elapsed().as_millis(),
            "message": "Session ended and connection released"
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response).unwrap_or_else(|_| "Session ended".to_string()),
        ))
    }

    /// List all active pinned sessions.
    #[tool(description = "List all active pinned sessions with their statistics.", read_only = true, idempotent = true)]
    pub async fn list_pinned_sessions(
        &self,
        input: ListPinnedSessionsInput,
    ) -> Result<ToolOutput, McpError> {
        debug!("Listing pinned sessions");

        let sessions = self.session_manager.list_sessions().await;

        let session_list: Vec<serde_json::Value> = sessions
            .iter()
            .map(|s| {
                let mut info = json!({
                    "session_id": s.id,
                    "query_count": s.query_count,
                    "age_ms": s.created_at.elapsed().as_millis(),
                    "idle_ms": s.last_activity.elapsed().as_millis(),
                });
                if input.detailed {
                    info["created_at_relative"] =
                        json!(format!("{:?} ago", s.created_at.elapsed()));
                    info["last_activity_relative"] =
                        json!(format!("{:?} ago", s.last_activity.elapsed()));
                }
                info
            })
            .collect();

        let response = json!({
            "count": sessions.len(),
            "sessions": session_list
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| format!("{} sessions", sessions.len())),
        ))
    }

    // =========================================================================
    // Pagination Tools
    // =========================================================================

    /// Execute a paginated query.
    #[tool(description = "Execute a SQL query with pagination support. Query must include ORDER BY for consistent results.", read_only = true)]
    pub async fn execute_paginated(
        &self,
        input: ExecutePaginatedInput,
    ) -> Result<ToolOutput, McpError> {
        debug!(
            "Executing paginated query: {}",
            truncate_for_log(&input.query, 100)
        );

        // Validate the query
        if let Err(e) = self.validate_query(&input.query) {
            return Ok(ToolOutput::error(format!("Query validation failed: {}", e)));
        }

        // Check for ORDER BY clause (required for consistent pagination)
        let query_upper = input.query.to_uppercase();
        if !query_upper.contains("ORDER BY") {
            return Ok(ToolOutput::error(
                "Paginated queries must include an ORDER BY clause for consistent results",
            ));
        }

        // Validate page size
        let page_size = input.page_size.clamp(1, 10000);

        // Determine offset based on page position
        let offset = match &input.page {
            Some(PaginationPosition::PageNumber(page)) => {
                let page_num = (*page).max(1);
                (page_num - 1) * page_size
            }
            Some(PaginationPosition::Cursor(cursor)) => {
                // Decode cursor (base64 encoded offset)
                match decode_cursor(cursor) {
                    Ok(off) => off,
                    Err(e) => {
                        return Ok(ToolOutput::error(format!("Invalid cursor: {}", e)));
                    }
                }
            }
            None => 0,
        };

        // Build paginated query using OFFSET-FETCH
        let paginated_query = format!(
            "{} OFFSET {} ROWS FETCH NEXT {} ROWS ONLY",
            input.query, offset, page_size
        );

        let result = match self.executor.execute(&paginated_query).await {
            Ok(r) => r,
            Err(e) => {
                warn!("Paginated query failed: {}", e);
                return Ok(ToolOutput::error(format!("Query execution failed: {}", e)));
            }
        };

        // Calculate pagination info
        let has_more = result.rows.len() == page_size;
        let next_cursor = if has_more {
            Some(encode_cursor(offset + page_size))
        } else {
            None
        };

        let current_page = (offset / page_size) + 1;

        // Format output
        let data_output = match input.format {
            OutputFormat::Json => serde_json::to_string_pretty(&result).unwrap_or_else(|e| {
                warn!("Failed to serialize paginated result to JSON: {}", e);
                format!("Failed to serialize result: {}", e)
            }),
            OutputFormat::Csv => result.to_csv(),
            OutputFormat::Table => result.to_markdown_table(),
        };

        let response = json!({
            "data": data_output,
            "pagination": {
                "page": current_page,
                "page_size": page_size,
                "row_count": result.rows.len(),
                "has_more": has_more,
                "next_cursor": next_cursor,
                "offset": offset,
            },
            "execution_time_ms": result.execution_time_ms,
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response).unwrap_or_else(|e| {
                warn!("Failed to serialize pagination response: {}", e);
                format!("Pagination error: {}", e)
            }),
        ))
    }

    // =========================================================================
    // Database Management Tools
    // =========================================================================

    /// Switch to a different database.
    #[tool(description = "Switch the connection to a different database on the same server.", idempotent = true)]
    pub async fn switch_database(
        &self,
        input: SwitchDatabaseInput,
    ) -> Result<ToolOutput, McpError> {
        debug!("Switching to database: {}", input.database);

        // Validate database name
        if let Err(e) = validate_identifier(&input.database) {
            return Ok(ToolOutput::error(format!("Invalid database name: {}", e)));
        }

        let escaped_db = match safe_identifier(&input.database) {
            Ok(db) => db,
            Err(e) => return Ok(ToolOutput::error(format!("Invalid database name: {}", e))),
        };

        // Execute USE statement
        let query = format!("USE {}", escaped_db);
        if let Err(e) = self.executor.execute(&query).await {
            return Ok(ToolOutput::error(format!("Failed to switch database: {}", e)));
        }

        // Update state
        {
            let mut state = self.state.write().await;
            state.set_current_database(Some(input.database.clone()));
        }

        info!("Switched to database: {}", input.database);

        let response = json!({
            "database": input.database,
            "status": "switched",
            "message": format!("Now using database: {}", input.database)
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| format!("Switched to {}", input.database)),
        ))
    }

    // =========================================================================
    // Index Analysis Tools
    // =========================================================================

    /// Get index recommendations for a query.
    #[tool(description = "Analyze a SQL query and recommend indexes for better performance.", read_only = true, idempotent = true)]
    pub async fn recommend_indexes(
        &self,
        input: RecommendIndexesInput,
    ) -> Result<ToolOutput, McpError> {
        debug!(
            "Analyzing query for index recommendations: {}",
            truncate_for_log(&input.query, 100)
        );

        // First, get the estimated execution plan
        let _plan_query = format!("SET SHOWPLAN_XML ON; {} SET SHOWPLAN_XML OFF;", input.query);

        // Get missing index recommendations from DMVs
        let missing_indexes_query = r#"
            SELECT TOP 20
                mig.index_group_handle,
                mid.index_handle,
                CONVERT(DECIMAL(10,2), migs.avg_total_user_cost * migs.avg_user_impact * (migs.user_seeks + migs.user_scans)) AS improvement_measure,
                'CREATE INDEX [IX_' +
                    REPLACE(REPLACE(REPLACE(mid.statement, '[', ''), ']', ''), '.', '_') + '_' +
                    REPLACE(REPLACE(REPLACE(ISNULL(mid.equality_columns, ''), '[', ''), ']', ''), ', ', '_') +
                '] ON ' + mid.statement + ' (' + ISNULL(mid.equality_columns, '') +
                CASE WHEN mid.equality_columns IS NOT NULL AND mid.inequality_columns IS NOT NULL THEN ',' ELSE '' END +
                ISNULL(mid.inequality_columns, '') + ')' +
                CASE WHEN mid.included_columns IS NOT NULL THEN ' INCLUDE (' + mid.included_columns + ')' ELSE '' END AS create_index_statement,
                mid.equality_columns,
                mid.inequality_columns,
                mid.included_columns,
                migs.user_seeks,
                migs.user_scans,
                migs.avg_total_user_cost,
                migs.avg_user_impact
            FROM sys.dm_db_missing_index_groups mig
            INNER JOIN sys.dm_db_missing_index_group_stats migs ON migs.group_handle = mig.index_group_handle
            INNER JOIN sys.dm_db_missing_index_details mid ON mig.index_handle = mid.index_handle
            WHERE mid.database_id = DB_ID()
            ORDER BY improvement_measure DESC
        "#;

        let missing_result = match self.executor.execute(missing_indexes_query).await {
            Ok(r) => r,
            Err(e) => {
                warn!("Failed to get missing indexes: {}", e);
                return Ok(ToolOutput::error(format!("Failed to analyze indexes: {}", e)));
            }
        };

        let mut response = json!({
            "query": truncate_for_log(&input.query, 500),
            "missing_indexes": [],
            "recommendations": [],
        });

        // Process missing indexes
        let mut recommendations: Vec<serde_json::Value> = Vec::new();
        for row in &missing_result.rows {
            if let Some(create_stmt) = row.get("create_index_statement") {
                recommendations.push(json!({
                    "type": "missing_index",
                    "create_statement": create_stmt.to_display_string(),
                    "improvement_measure": row.get("improvement_measure")
                        .map(|v| v.to_display_string()).unwrap_or_default(),
                    "equality_columns": row.get("equality_columns")
                        .map(|v| v.to_display_string()),
                    "inequality_columns": row.get("inequality_columns")
                        .map(|v| v.to_display_string()),
                    "included_columns": row.get("included_columns")
                        .map(|v| v.to_display_string()),
                }));
            }
        }

        response["recommendations"] = json!(recommendations);

        // Get existing indexes if requested
        if input.include_existing {
            let existing_query = r#"
                SELECT
                    s.name AS schema_name,
                    t.name AS table_name,
                    i.name AS index_name,
                    i.type_desc AS index_type,
                    i.is_unique,
                    i.is_primary_key,
                    STUFF((
                        SELECT ', ' + c.name
                        FROM sys.index_columns ic
                        INNER JOIN sys.columns c ON ic.object_id = c.object_id AND ic.column_id = c.column_id
                        WHERE ic.object_id = i.object_id AND ic.index_id = i.index_id
                        AND ic.is_included_column = 0
                        ORDER BY ic.key_ordinal
                        FOR XML PATH('')
                    ), 1, 2, '') AS key_columns,
                    STUFF((
                        SELECT ', ' + c.name
                        FROM sys.index_columns ic
                        INNER JOIN sys.columns c ON ic.object_id = c.object_id AND ic.column_id = c.column_id
                        WHERE ic.object_id = i.object_id AND ic.index_id = i.index_id
                        AND ic.is_included_column = 1
                        ORDER BY ic.key_ordinal
                        FOR XML PATH('')
                    ), 1, 2, '') AS included_columns
                FROM sys.indexes i
                INNER JOIN sys.tables t ON i.object_id = t.object_id
                INNER JOIN sys.schemas s ON t.schema_id = s.schema_id
                WHERE i.type > 0
                AND t.is_ms_shipped = 0
                ORDER BY s.name, t.name, i.name
            "#;

            if let Ok(existing) = self.executor.execute(existing_query).await {
                let existing_indexes: Vec<serde_json::Value> = existing
                    .rows
                    .iter()
                    .map(|row| {
                        json!({
                            "schema": row.get("schema_name").map(|v| v.to_display_string()),
                            "table": row.get("table_name").map(|v| v.to_display_string()),
                            "index_name": row.get("index_name").map(|v| v.to_display_string()),
                            "type": row.get("index_type").map(|v| v.to_display_string()),
                            "is_unique": row.get("is_unique").map(|v| v.to_display_string()),
                            "key_columns": row.get("key_columns").map(|v| v.to_display_string()),
                            "included_columns": row.get("included_columns").map(|v| v.to_display_string()),
                        })
                    })
                    .collect();
                response["existing_indexes"] = json!(existing_indexes);
            }
        }

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| "Index analysis failed".to_string()),
        ))
    }

    // =========================================================================
    // Schema Comparison Tools
    // =========================================================================

    /// Compare two database schemas.
    #[tool(description = "Compare objects between two schemas in the same database.", read_only = true, idempotent = true)]
    pub async fn compare_schemas(
        &self,
        input: CompareSchemaInput,
    ) -> Result<ToolOutput, McpError> {
        debug!(
            "Comparing schemas: {} vs {}",
            input.source_schema, input.target_schema
        );

        // Validate schema names
        if let Err(e) = validate_identifier(&input.source_schema) {
            return Ok(ToolOutput::error(format!("Invalid source schema name: {}", e)));
        }
        if let Err(e) = validate_identifier(&input.target_schema) {
            return Ok(ToolOutput::error(format!("Invalid target schema name: {}", e)));
        }

        let include_tables = input.object_types == "all" || input.object_types == "tables";
        let include_views = input.object_types == "all" || input.object_types == "views";
        let include_procedures = input.object_types == "all" || input.object_types == "procedures";

        let mut differences: Vec<serde_json::Value> = Vec::new();

        // Compare tables
        if include_tables {
            let tables_query = format!(
                r#"
                SELECT
                    COALESCE(s.name, t.name) AS table_name,
                    CASE
                        WHEN s.name IS NULL THEN 'only_in_target'
                        WHEN t.name IS NULL THEN 'only_in_source'
                        ELSE 'in_both'
                    END AS status
                FROM (
                    SELECT TABLE_NAME AS name
                    FROM INFORMATION_SCHEMA.TABLES
                    WHERE TABLE_SCHEMA = '{0}' AND TABLE_TYPE = 'BASE TABLE'
                ) s
                FULL OUTER JOIN (
                    SELECT TABLE_NAME AS name
                    FROM INFORMATION_SCHEMA.TABLES
                    WHERE TABLE_SCHEMA = '{1}' AND TABLE_TYPE = 'BASE TABLE'
                ) t ON s.name = t.name
                "#,
                input.source_schema.replace('\'', "''"),
                input.target_schema.replace('\'', "''")
            );

            if let Ok(result) = self.executor.execute(&tables_query).await {
                for row in &result.rows {
                    let status = row
                        .get("status")
                        .map(|v| v.to_display_string())
                        .unwrap_or_default();
                    if status != "in_both" {
                        differences.push(json!({
                            "type": "table",
                            "name": row.get("table_name").map(|v| v.to_display_string()),
                            "status": status,
                        }));
                    }
                }
            }
        }

        // Compare views
        if include_views {
            let views_query = format!(
                r#"
                SELECT
                    COALESCE(s.name, t.name) AS view_name,
                    CASE
                        WHEN s.name IS NULL THEN 'only_in_target'
                        WHEN t.name IS NULL THEN 'only_in_source'
                        ELSE 'in_both'
                    END AS status
                FROM (
                    SELECT TABLE_NAME AS name
                    FROM INFORMATION_SCHEMA.VIEWS
                    WHERE TABLE_SCHEMA = '{0}'
                ) s
                FULL OUTER JOIN (
                    SELECT TABLE_NAME AS name
                    FROM INFORMATION_SCHEMA.VIEWS
                    WHERE TABLE_SCHEMA = '{1}'
                ) t ON s.name = t.name
                "#,
                input.source_schema.replace('\'', "''"),
                input.target_schema.replace('\'', "''")
            );

            if let Ok(result) = self.executor.execute(&views_query).await {
                for row in &result.rows {
                    let status = row
                        .get("status")
                        .map(|v| v.to_display_string())
                        .unwrap_or_default();
                    if status != "in_both" {
                        differences.push(json!({
                            "type": "view",
                            "name": row.get("view_name").map(|v| v.to_display_string()),
                            "status": status,
                        }));
                    }
                }
            }
        }

        // Compare procedures
        if include_procedures {
            let procs_query = format!(
                r#"
                SELECT
                    COALESCE(s.name, t.name) AS procedure_name,
                    CASE
                        WHEN s.name IS NULL THEN 'only_in_target'
                        WHEN t.name IS NULL THEN 'only_in_source'
                        ELSE 'in_both'
                    END AS status
                FROM (
                    SELECT ROUTINE_NAME AS name
                    FROM INFORMATION_SCHEMA.ROUTINES
                    WHERE ROUTINE_SCHEMA = '{0}' AND ROUTINE_TYPE = 'PROCEDURE'
                ) s
                FULL OUTER JOIN (
                    SELECT ROUTINE_NAME AS name
                    FROM INFORMATION_SCHEMA.ROUTINES
                    WHERE ROUTINE_SCHEMA = '{1}' AND ROUTINE_TYPE = 'PROCEDURE'
                ) t ON s.name = t.name
                "#,
                input.source_schema.replace('\'', "''"),
                input.target_schema.replace('\'', "''")
            );

            if let Ok(result) = self.executor.execute(&procs_query).await {
                for row in &result.rows {
                    let status = row
                        .get("status")
                        .map(|v| v.to_display_string())
                        .unwrap_or_default();
                    if status != "in_both" {
                        differences.push(json!({
                            "type": "procedure",
                            "name": row.get("procedure_name").map(|v| v.to_display_string()),
                            "status": status,
                        }));
                    }
                }
            }
        }

        let response = json!({
            "source_schema": input.source_schema,
            "target_schema": input.target_schema,
            "object_types": input.object_types,
            "difference_count": differences.len(),
            "differences": differences,
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| "Schema comparison failed".to_string()),
        ))
    }

    /// Compare two tables.
    #[tool(description = "Compare the structure of two tables, including columns, indexes, and constraints.", read_only = true, idempotent = true)]
    pub async fn compare_tables(
        &self,
        input: CompareTablesInput,
    ) -> Result<ToolOutput, McpError> {
        debug!(
            "Comparing tables: {} vs {}",
            input.source_table, input.target_table
        );

        // Parse table names
        let (source_schema, source_table) = parse_table_name(&input.source_table)?;
        let (target_schema, target_table) = parse_table_name(&input.target_table)?;

        // Compare columns
        let columns_query = format!(
            r#"
            SELECT
                COALESCE(s.column_name, t.column_name) AS column_name,
                s.data_type AS source_type,
                t.data_type AS target_type,
                s.character_maximum_length AS source_max_length,
                t.character_maximum_length AS target_max_length,
                s.is_nullable AS source_nullable,
                t.is_nullable AS target_nullable,
                CASE
                    WHEN s.column_name IS NULL THEN 'only_in_target'
                    WHEN t.column_name IS NULL THEN 'only_in_source'
                    WHEN s.data_type <> t.data_type THEN 'type_mismatch'
                    WHEN ISNULL(s.character_maximum_length, 0) <> ISNULL(t.character_maximum_length, 0) THEN 'length_mismatch'
                    WHEN s.is_nullable <> t.is_nullable THEN 'nullable_mismatch'
                    ELSE 'identical'
                END AS status
            FROM (
                SELECT COLUMN_NAME AS column_name, DATA_TYPE AS data_type,
                       CHARACTER_MAXIMUM_LENGTH AS character_maximum_length, IS_NULLABLE AS is_nullable
                FROM INFORMATION_SCHEMA.COLUMNS
                WHERE TABLE_SCHEMA = '{0}' AND TABLE_NAME = '{1}'
            ) s
            FULL OUTER JOIN (
                SELECT COLUMN_NAME AS column_name, DATA_TYPE AS data_type,
                       CHARACTER_MAXIMUM_LENGTH AS character_maximum_length, IS_NULLABLE AS is_nullable
                FROM INFORMATION_SCHEMA.COLUMNS
                WHERE TABLE_SCHEMA = '{2}' AND TABLE_NAME = '{3}'
            ) t ON s.column_name = t.column_name
            ORDER BY COALESCE(s.column_name, t.column_name)
            "#,
            source_schema.replace('\'', "''"),
            source_table.replace('\'', "''"),
            target_schema.replace('\'', "''"),
            target_table.replace('\'', "''")
        );

        let columns_result = match self.executor.execute(&columns_query).await {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolOutput::error(format!("Failed to compare columns: {}", e)));
            }
        };

        let column_diffs: Vec<serde_json::Value> = columns_result
            .rows
            .iter()
            .filter(|row| {
                row.get("status")
                    .map(|v| v.to_display_string())
                    .unwrap_or_default()
                    != "identical"
            })
            .map(|row| {
                json!({
                    "column": row.get("column_name").map(|v| v.to_display_string()),
                    "status": row.get("status").map(|v| v.to_display_string()),
                    "source_type": row.get("source_type").map(|v| v.to_display_string()),
                    "target_type": row.get("target_type").map(|v| v.to_display_string()),
                })
            })
            .collect();

        let response = json!({
            "source_table": input.source_table,
            "target_table": input.target_table,
            "column_differences": column_diffs,
            "difference_count": column_diffs.len(),
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| "Table comparison failed".to_string()),
        ))
    }

    // =========================================================================
    // Data Sampling Tools
    // =========================================================================

    /// Sample data from a table.
    #[tool(description = "Get a random or stratified sample of data from a table.", read_only = true)]
    pub async fn sample_data(
        &self,
        input: SampleDataInput,
    ) -> Result<ToolOutput, McpError> {
        debug!(
            "Sampling {} rows from {} using method {}",
            input.sample_size, input.table, input.method
        );

        // Parse table name
        let (schema, table) = parse_table_name(&input.table)?;
        let escaped_table = format!(
            "{}.{}",
            safe_identifier(&schema).map_err(|e| McpError::invalid_params("schema", e.to_string()))?,
            safe_identifier(&table).map_err(|e| McpError::invalid_params("table", e.to_string()))?
        );

        let sample_size = input.sample_size.clamp(1, 10000);

        // Build filter clause
        let filter_clause = match &input.filter {
            Some(f) if !f.is_empty() => format!("WHERE {}", f),
            _ => String::new(),
        };

        // Build query based on method
        let query = match input.method.to_lowercase().as_str() {
            "top" => {
                format!(
                    "SELECT TOP {} * FROM {} {} ORDER BY (SELECT NULL)",
                    sample_size, escaped_table, filter_clause
                )
            }
            "bottom" => {
                // Use a subquery to reverse order
                format!(
                    r#"
                    SELECT * FROM (
                        SELECT TOP {} *, ROW_NUMBER() OVER (ORDER BY (SELECT NULL) DESC) AS _rn
                        FROM {} {}
                    ) t ORDER BY _rn DESC
                    "#,
                    sample_size, escaped_table, filter_clause
                )
            }
            "stratified" => {
                match &input.stratify_column {
                    Some(col) => {
                        // Validate column name
                        if let Err(e) = validate_identifier(col) {
                            return Ok(ToolOutput::error(format!("Invalid stratify column name: {}", e)));
                        }
                        let escaped_col = safe_identifier(col)
                            .map_err(|e| McpError::invalid_params("stratify_column", e.to_string()))?;
                        format!(
                            r#"
                            SELECT * FROM (
                                SELECT *, ROW_NUMBER() OVER (PARTITION BY {} ORDER BY NEWID()) AS _rn
                                FROM {} {}
                            ) t WHERE _rn <= {} / (SELECT COUNT(DISTINCT {}) FROM {} {})
                            "#,
                            escaped_col,
                            escaped_table,
                            filter_clause,
                            sample_size,
                            escaped_col,
                            escaped_table,
                            filter_clause
                        )
                    }
                    None => {
                        return Ok(ToolOutput::error(
                            "Stratified sampling requires stratify_column parameter",
                        ));
                    }
                }
            }
            _ => {
                // Default: random sampling using TABLESAMPLE or NEWID()
                format!(
                    "SELECT TOP {} * FROM {} {} ORDER BY NEWID()",
                    sample_size, escaped_table, filter_clause
                )
            }
        };

        let result = match self.executor.execute(&query).await {
            Ok(r) => r,
            Err(e) => {
                warn!("Sample query failed: {}", e);
                return Ok(ToolOutput::error(format!("Failed to sample data: {}", e)));
            }
        };

        let output = match input.format {
            OutputFormat::Json => serde_json::to_string_pretty(&result).unwrap_or_else(|e| {
                warn!("Failed to serialize sample data to JSON: {}", e);
                format!("Failed to serialize result: {}", e)
            }),
            OutputFormat::Csv => result.to_csv(),
            OutputFormat::Table => result.to_markdown_table(),
        };

        Ok(ToolOutput::text(output))
    }

    // =========================================================================
    // Bulk Operations Tools
    // =========================================================================

    /// Bulk insert data into a table.
    #[tool(description = "Insert multiple rows into a table efficiently using batched INSERT statements. Automatically splits large datasets into batches for memory efficiency. Supports optional transaction wrapping for atomicity. Native BCP protocol support is planned for future releases.", destructive = true)]
    pub async fn bulk_insert(
        &self,
        input: BulkInsertInput,
    ) -> Result<ToolOutput, McpError> {
        debug!(
            "Bulk inserting {} rows into {} (native_bcp={}, transaction={}, continue_on_error={})",
            input.rows.len(),
            input.table,
            input.use_native_bcp,
            input.use_transaction,
            input.continue_on_error
        );

        if input.rows.is_empty() {
            return Ok(ToolOutput::error("No rows to insert"));
        }

        if input.columns.is_empty() {
            return Ok(ToolOutput::error("No columns specified"));
        }

        // Parse and validate table name
        let (schema, table) = parse_table_name(&input.table)?;
        let escaped_table = format!(
            "{}.{}",
            safe_identifier(&schema).map_err(|e| McpError::invalid_params("schema", e.to_string()))?,
            safe_identifier(&table).map_err(|e| McpError::invalid_params("table", e.to_string()))?
        );

        // Validate and escape column names
        let escaped_columns: Result<Vec<String>, _> =
            input.columns.iter().map(|c| safe_identifier(c)).collect();
        let escaped_columns =
            escaped_columns.map_err(|e| McpError::invalid_params("columns", e.to_string()))?;

        let batch_size = input.batch_size.clamp(1, 5000);

        // Check if native BCP was requested
        // NOTE: Native BCP is not yet available in mssql-client v0.5.2
        // The library has BCP packet building infrastructure but doesn't expose
        // Client.bulk_insert() yet. This will be enabled in a future release.
        let bcp_available = self.bulk_insert_manager.is_native_bcp_available();
        if input.use_native_bcp && !bcp_available {
            debug!("Native BCP requested but not available, using INSERT statements");
        }

        // Use batched INSERT statements
        debug!("Using batched INSERT statements (batch_size={})", batch_size);

        // Build all INSERT statements
        let statements: Vec<String> = input
            .rows
            .chunks(batch_size)
            .map(|chunk| {
                let values: Vec<String> = chunk
                    .iter()
                    .map(|row| {
                        let formatted_values: Vec<String> =
                            row.iter().map(format_parameter_value).collect();
                        format!("({})", formatted_values.join(", "))
                    })
                    .collect();

                format!(
                    "INSERT INTO {} ({}) VALUES {}",
                    escaped_table,
                    escaped_columns.join(", "),
                    values.join(", ")
                )
            })
            .collect();

        let total_batches = statements.len();

        // Execute based on transaction mode
        if input.use_transaction {
            // Use transactional execution for atomicity
            match self
                .executor
                .execute_in_transaction(&statements, input.continue_on_error)
                .await
            {
                Ok(result) => {
                    let response = json!({
                        "table": input.table,
                        "rows_requested": input.rows.len(),
                        "rows_inserted": result.total_rows_affected,
                        "batch_size": batch_size,
                        "batches": total_batches,
                        "successful_batches": result.successful_statements,
                        "errors": result.errors,
                        "status": if result.errors.is_empty() { "success" } else { "partial" },
                        "execution_time_ms": result.execution_time_ms,
                        "method": "insert_statements",
                        "transaction": true,
                        "native_bcp_requested": input.use_native_bcp,
                        "native_bcp_available": bcp_available,
                    });

                    Ok(ToolOutput::text(
                        serde_json::to_string_pretty(&response)
                            .unwrap_or_else(|_| format!("Inserted {} rows", result.total_rows_affected)),
                    ))
                }
                Err(e) => {
                    let response = json!({
                        "table": input.table,
                        "rows_requested": input.rows.len(),
                        "rows_inserted": 0,
                        "batch_size": batch_size,
                        "batches": total_batches,
                        "status": "failed",
                        "error": e.to_string(),
                        "method": "insert_statements",
                        "transaction": true,
                        "rolled_back": true,
                        "native_bcp_requested": input.use_native_bcp,
                        "native_bcp_available": bcp_available,
                    });

                    Ok(ToolOutput::text(
                        serde_json::to_string_pretty(&response)
                            .unwrap_or_else(|_| format!("Insert failed: {}", e)),
                    ))
                }
            }
        } else {
            // Non-transactional: execute each batch independently
            let mut total_inserted: u64 = 0;
            let mut successful_batches = 0;
            let mut errors: Vec<String> = Vec::new();

            for (idx, stmt) in statements.iter().enumerate() {
                match self.executor.execute(stmt).await {
                    Ok(result) => {
                        total_inserted += result.rows_affected;
                        successful_batches += 1;
                    }
                    Err(e) => {
                        let error_msg = format!("Batch {}/{} failed: {}", idx + 1, total_batches, e);
                        errors.push(error_msg);

                        if !input.continue_on_error {
                            // Stop on first error
                            break;
                        }
                    }
                }
            }

            let response = json!({
                "table": input.table,
                "rows_requested": input.rows.len(),
                "rows_inserted": total_inserted,
                "batch_size": batch_size,
                "batches": total_batches,
                "successful_batches": successful_batches,
                "errors": errors,
                "status": if errors.is_empty() { "success" } else if successful_batches > 0 { "partial" } else { "failed" },
                "method": "insert_statements",
                "transaction": false,
                "native_bcp_requested": input.use_native_bcp,
                "native_bcp_available": bcp_available,
            });

            Ok(ToolOutput::text(
                serde_json::to_string_pretty(&response)
                    .unwrap_or_else(|_| format!("Inserted {} rows", total_inserted)),
            ))
        }
    }

    /// Export query results to various formats.
    #[tool(description = "Export query results in CSV, JSON, or JSON Lines format.", read_only = true)]
    pub async fn export_data(
        &self,
        input: ExportDataInput,
    ) -> Result<ToolOutput, McpError> {
        debug!("Exporting data: {}", truncate_for_log(&input.query, 100));

        // Validate query
        if let Err(e) = self.validate_query(&input.query) {
            return Ok(ToolOutput::error(format!("Query validation failed: {}", e)));
        }

        let max_rows = input
            .max_rows
            .unwrap_or(self.config.security.max_result_rows);

        let result = match self
            .executor
            .execute_with_limit(&input.query, max_rows)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("Export query failed: {}", e);
                return Ok(ToolOutput::error(format!("Query execution failed: {}", e)));
            }
        };

        let output = match input.format {
            ExportFormat::Json => serde_json::to_string_pretty(&result).unwrap_or_else(|e| {
                warn!("Failed to serialize export to JSON: {}", e);
                format!("Failed to serialize result: {}", e)
            }),
            ExportFormat::JsonLines => {
                // One JSON object per line
                result
                    .rows
                    .iter()
                    .filter_map(|row| {
                        serde_json::to_string(&row.columns)
                            .map_err(|e| {
                                warn!("Failed to serialize row to JSON Lines: {}", e);
                                e
                            })
                            .ok()
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            ExportFormat::Csv => {
                // CSV with optional headers
                let mut csv_output = String::new();
                if input.include_headers && !result.columns.is_empty() {
                    csv_output.push_str(
                        &result
                            .columns
                            .iter()
                            .map(|c| c.name.as_str())
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                    csv_output.push('\n');
                }
                csv_output.push_str(&result.to_csv());
                // Remove duplicate header if to_csv includes it
                if input.include_headers {
                    csv_output
                } else {
                    // Skip first line if to_csv added header
                    let lines: Vec<&str> = csv_output.lines().skip(1).collect();
                    lines.join("\n")
                }
            }
        };

        let response = json!({
            "format": input.format.as_str(),
            "row_count": result.rows.len(),
            "column_count": result.columns.len(),
            "truncated": result.truncated,
            "data": output,
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response).unwrap_or_else(|e| {
                warn!("Failed to serialize export response: {}", e);
                format!("Export failed: {}", e)
            }),
        ))
    }

    // =========================================================================
    // Server Metrics Tools
    // =========================================================================

    /// Get server performance metrics.
    #[tool(description = "Get SQL Server performance metrics including connections, queries, and memory usage.", read_only = true, idempotent = true)]
    pub async fn get_metrics(
        &self,
        input: GetMetricsInput,
    ) -> Result<ToolOutput, McpError> {
        debug!(
            "Getting server metrics for categories: {}",
            input.categories
        );

        let mut metrics = json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });

        let include_all = input.categories == "all";

        // Connection metrics
        if include_all || input.categories.contains("connections") {
            let conn_query = r#"
                SELECT
                    COUNT(*) AS total_connections,
                    SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END) AS active_connections,
                    SUM(CASE WHEN status = 'sleeping' THEN 1 ELSE 0 END) AS idle_connections
                FROM sys.dm_exec_sessions
                WHERE is_user_process = 1
            "#;

            if let Ok(result) = self.executor.execute(conn_query).await {
                if let Some(row) = result.rows.first() {
                    metrics["connections"] = json!({
                        "total": row.get("total_connections").map(|v| v.to_display_string()),
                        "active": row.get("active_connections").map(|v| v.to_display_string()),
                        "idle": row.get("idle_connections").map(|v| v.to_display_string()),
                    });
                }
            }
        }

        // Query metrics
        if include_all || input.categories.contains("queries") {
            let query_stats = format!(
                r#"
                SELECT TOP 10
                    SUBSTRING(qt.text, 1, 200) AS query_text,
                    qs.execution_count,
                    qs.total_worker_time / 1000 AS total_cpu_ms,
                    qs.total_elapsed_time / 1000 AS total_duration_ms,
                    qs.total_logical_reads,
                    qs.total_physical_reads,
                    qs.last_execution_time
                FROM sys.dm_exec_query_stats qs
                CROSS APPLY sys.dm_exec_sql_text(qs.sql_handle) qt
                WHERE qs.last_execution_time > DATEADD(minute, -{}, GETDATE())
                ORDER BY qs.total_elapsed_time DESC
            "#,
                input.time_range_minutes
            );

            if let Ok(result) = self.executor.execute(&query_stats).await {
                let top_queries: Vec<serde_json::Value> = result
                    .rows
                    .iter()
                    .map(|row| {
                        json!({
                            "query": row.get("query_text").map(|v| v.to_display_string()),
                            "executions": row.get("execution_count").map(|v| v.to_display_string()),
                            "total_cpu_ms": row.get("total_cpu_ms").map(|v| v.to_display_string()),
                            "total_duration_ms": row.get("total_duration_ms").map(|v| v.to_display_string()),
                        })
                    })
                    .collect();
                metrics["top_queries"] = json!(top_queries);
            }
        }

        // Memory metrics
        if include_all || input.categories.contains("memory") {
            let memory_query = r#"
                SELECT
                    physical_memory_in_use_kb / 1024 AS memory_used_mb,
                    locked_page_allocations_kb / 1024 AS locked_pages_mb,
                    large_page_allocations_kb / 1024 AS large_pages_mb,
                    total_virtual_address_space_kb / 1024 AS virtual_space_mb,
                    available_commit_limit_kb / 1024 AS available_commit_mb
                FROM sys.dm_os_process_memory
            "#;

            if let Ok(result) = self.executor.execute(memory_query).await {
                if let Some(row) = result.rows.first() {
                    metrics["memory"] = json!({
                        "used_mb": row.get("memory_used_mb").map(|v| v.to_display_string()),
                        "locked_pages_mb": row.get("locked_pages_mb").map(|v| v.to_display_string()),
                        "virtual_space_mb": row.get("virtual_space_mb").map(|v| v.to_display_string()),
                    });
                }
            }
        }

        // Performance counters
        if include_all || input.categories.contains("performance") {
            let perf_query = r#"
                SELECT
                    object_name,
                    counter_name,
                    cntr_value
                FROM sys.dm_os_performance_counters
                WHERE counter_name IN (
                    'Page life expectancy',
                    'Buffer cache hit ratio',
                    'Batch Requests/sec',
                    'SQL Compilations/sec',
                    'SQL Re-Compilations/sec',
                    'User Connections'
                )
            "#;

            if let Ok(result) = self.executor.execute(perf_query).await {
                let counters: Vec<serde_json::Value> = result
                    .rows
                    .iter()
                    .map(|row| {
                        json!({
                            "counter": row.get("counter_name").map(|v| v.to_display_string()),
                            "value": row.get("cntr_value").map(|v| v.to_display_string()),
                        })
                    })
                    .collect();
                metrics["performance_counters"] = json!(counters);
            }
        }

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&metrics)
                .unwrap_or_else(|_| "Failed to get metrics".to_string()),
        ))
    }

    /// Analyze a query for performance issues.
    #[tool(description = "Analyze a SQL query for performance issues and optimization opportunities.", read_only = true, idempotent = true)]
    pub async fn analyze_query(
        &self,
        input: AnalyzeQueryInput,
    ) -> Result<ToolOutput, McpError> {
        debug!("Analyzing query: {}", truncate_for_log(&input.query, 100));

        let mut analysis = json!({
            "query": truncate_for_log(&input.query, 500),
            "analysis": {},
        });

        // Get estimated execution plan as XML
        let plan_query = format!(
            "SET SHOWPLAN_XML ON; EXEC sp_executesql N'{}'; SET SHOWPLAN_XML OFF;",
            input.query.replace('\'', "''")
        );

        // Try to get plan info
        if let Ok(plan_result) = self.executor.execute(&plan_query).await {
            if !plan_result.rows.is_empty() {
                analysis["execution_plan"] = json!({
                    "status": "retrieved",
                    "note": "Execution plan retrieved successfully"
                });
            }
        }

        // Get table statistics if requested
        if input.include_statistics {
            // Extract table names from query (simple heuristic)
            let stats_query = r#"
                SELECT
                    OBJECT_SCHEMA_NAME(s.object_id) AS schema_name,
                    OBJECT_NAME(s.object_id) AS table_name,
                    s.name AS stats_name,
                    STATS_DATE(s.object_id, s.stats_id) AS last_updated,
                    sp.rows,
                    sp.modification_counter
                FROM sys.stats s
                CROSS APPLY sys.dm_db_stats_properties(s.object_id, s.stats_id) sp
                WHERE OBJECT_SCHEMA_NAME(s.object_id) NOT IN ('sys', 'INFORMATION_SCHEMA')
                ORDER BY sp.modification_counter DESC
            "#;

            if let Ok(result) = self.executor.execute(stats_query).await {
                let stats: Vec<serde_json::Value> = result
                    .rows
                    .iter()
                    .take(20)
                    .map(|row| {
                        json!({
                            "schema": row.get("schema_name").map(|v| v.to_display_string()),
                            "table": row.get("table_name").map(|v| v.to_display_string()),
                            "stats_name": row.get("stats_name").map(|v| v.to_display_string()),
                            "last_updated": row.get("last_updated").map(|v| v.to_display_string()),
                            "rows": row.get("rows").map(|v| v.to_display_string()),
                            "modifications": row.get("modification_counter").map(|v| v.to_display_string()),
                        })
                    })
                    .collect();
                analysis["statistics"] = json!(stats);
            }
        }

        // Check for missing indexes
        if input.include_index_analysis {
            let missing_query = r#"
                SELECT TOP 10
                    mid.statement AS table_name,
                    mid.equality_columns,
                    mid.inequality_columns,
                    mid.included_columns,
                    CONVERT(DECIMAL(10,2), migs.avg_total_user_cost * migs.avg_user_impact * (migs.user_seeks + migs.user_scans)) AS impact
                FROM sys.dm_db_missing_index_groups mig
                INNER JOIN sys.dm_db_missing_index_group_stats migs ON migs.group_handle = mig.index_group_handle
                INNER JOIN sys.dm_db_missing_index_details mid ON mig.index_handle = mid.index_handle
                WHERE mid.database_id = DB_ID()
                ORDER BY impact DESC
            "#;

            if let Ok(result) = self.executor.execute(missing_query).await {
                let missing: Vec<serde_json::Value> = result
                    .rows
                    .iter()
                    .map(|row| {
                        json!({
                            "table": row.get("table_name").map(|v| v.to_display_string()),
                            "equality_columns": row.get("equality_columns").map(|v| v.to_display_string()),
                            "inequality_columns": row.get("inequality_columns").map(|v| v.to_display_string()),
                            "included_columns": row.get("included_columns").map(|v| v.to_display_string()),
                            "impact": row.get("impact").map(|v| v.to_display_string()),
                        })
                    })
                    .collect();
                analysis["missing_indexes"] = json!(missing);
            }
        }

        // Check for query patterns that might cause issues
        let query_upper = input.query.to_uppercase();
        let mut warnings: Vec<String> = Vec::new();

        if query_upper.contains("SELECT *") {
            warnings.push("Query uses SELECT * - consider specifying explicit columns".to_string());
        }
        if !query_upper.contains("WHERE") && query_upper.contains("SELECT") {
            warnings.push("Query has no WHERE clause - may scan entire table".to_string());
        }
        if query_upper.contains("LIKE '%") {
            warnings.push(
                "Query uses leading wildcard in LIKE - cannot use index efficiently".to_string(),
            );
        }
        if query_upper.contains("OR ") {
            warnings.push("Query contains OR conditions - may prevent index usage".to_string());
        }
        if query_upper.contains("NOT IN") || query_upper.contains("NOT EXISTS") {
            warnings.push("Query uses NOT IN/NOT EXISTS - consider alternatives".to_string());
        }

        analysis["warnings"] = json!(warnings);

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&analysis)
                .unwrap_or_else(|_| "Analysis failed".to_string()),
        ))
    }

    // =========================================================================
    // Connection Pool Metrics Tools
    // =========================================================================

    /// Get connection pool metrics and statistics.
    ///
    /// Returns information about the connection pool including
    /// active connections, idle connections, and pool configuration.
    #[tool(description = "Get connection pool metrics including active connections, idle connections, and pool health.", read_only = true, idempotent = true)]
    pub async fn get_pool_metrics(
        &self,
        input: GetPoolMetricsInput,
    ) -> Result<ToolOutput, McpError> {
        debug!("Getting connection pool metrics");

        let pool_status = self.pool.status();

        let mut response = json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "pool": {
                "total_connections": pool_status.total,
                "available_connections": pool_status.available,
                "in_use_connections": pool_status.in_use,
                "max_connections": pool_status.max,
                "utilization_percent": pool_status.utilization()
            },
            "config": {
                "max_connections": self.config.database.pool.max_connections,
                "min_connections": self.config.database.pool.min_connections,
                "connection_timeout_seconds": self.config.database.pool.connection_timeout.as_secs(),
                "idle_timeout_seconds": self.config.database.pool.idle_timeout.as_secs(),
            }
        });

        // Add health assessment
        let healthy = pool_status.available > 0 || !pool_status.is_at_capacity();
        response["health"] = json!({
            "status": if healthy { "healthy" } else { "degraded" },
            "has_available_connections": pool_status.available > 0,
            "at_capacity": pool_status.is_at_capacity(),
        });

        if input.include_history {
            // Get session state for historical context
            let state = self.state.read().await;
            response["sessions"] = json!({
                "total": state.total_session_count(),
                "running": state.running_session_count(),
            });
            response["transactions"] = json!({
                "total": state.total_transaction_count(),
                "active": state.active_transaction_count(),
            });
        }

        info!(
            "Pool metrics: {}/{} connections in use",
            pool_status.in_use, pool_status.total
        );

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| "Failed to get pool metrics".to_string()),
        ))
    }

    /// Get internal server metrics.
    ///
    /// Returns metrics collected by the server including query counts,
    /// latency statistics, cache performance, and transaction counts.
    #[tool(description = "Get internal server metrics including query counts, latency, cache stats, and transaction counts.", read_only = true, idempotent = true)]
    pub async fn get_internal_metrics(
        &self,
        input: GetInternalMetricsInput,
    ) -> Result<ToolOutput, McpError> {
        debug!("Getting internal server metrics");

        let snapshot = self.metrics.snapshot();

        let mut response = json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "queries": {
                "total": snapshot.queries_total,
                "success": snapshot.queries_success,
                "failed": snapshot.queries_failed,
                "total_time_ms": snapshot.query_time_ms_total,
            },
            "transactions": {
                "total": snapshot.transactions_total,
                "committed": snapshot.transactions_committed,
                "rolled_back": snapshot.transactions_rolled_back,
            },
            "cache": {
                "hits": snapshot.cache_hits,
                "misses": snapshot.cache_misses,
            },
            "network": {
                "bytes_transferred": snapshot.bytes_transferred,
            }
        });

        if input.include_rates {
            response["rates"] = json!({
                "query_success_rate_percent": snapshot.success_rate(),
                "avg_query_time_ms": snapshot.avg_query_time_ms(),
                "cache_hit_rate_percent": snapshot.cache_hit_rate(),
            });
        }

        info!(
            "Internal metrics: {} queries, {:.1}% success rate",
            snapshot.queries_total,
            snapshot.success_rate()
        );

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| "Failed to get internal metrics".to_string()),
        ))
    }

    // -------------------------------------------------------------------------
    // Validation Tools
    // -------------------------------------------------------------------------

    #[tool(description = "Validate SQL query syntax without executing. Use for dry-run validation before running DDL or complex queries.", read_only = true, idempotent = true)]
    pub async fn validate_syntax_tool(
        &self,
        input: ValidateSyntaxInput,
    ) -> Result<ToolOutput, McpError> {
        debug!("Validating query syntax");

        // If a database is specified, switch to it first
        let effective_query = if let Some(ref db) = input.database {
            format!("USE [{}];\n{}", db.replace(']', "]]"), input.query)
        } else {
            input.query.clone()
        };

        let result = self
            .executor
            .validate_syntax(&effective_query)
            .await
            .map_err(|e| McpError::internal(format!("Validation failed: {}", e)))?;

        if result.valid {
            info!("Query syntax is valid");
        } else {
            info!(
                error = result.error_message.as_deref().unwrap_or("Unknown error"),
                line = result.error_line,
                "Query syntax validation failed"
            );
        }

        Ok(ToolOutput::text(result.to_message()))
    }

    // =========================================================================
    // Resources (read-only metadata access)
    // =========================================================================

    /// Get SQL Server information including version, edition, and configuration.
    #[resource(
        uri_pattern = "mssql://server/info",
        name = "Server Information",
        description = "SQL Server version, edition, and configuration details",
        mime_type = "application/json"
    )]
    pub async fn resource_server_info(&self, uri: &str) -> Result<ResourceContents, McpError> {
        let info = self
            .metadata
            .get_server_info()
            .await
            .map_err(|e| McpError::internal(format!("Failed to get server info: {}", e)))?;

        ResourceContents::json(uri, &info)
            .map_err(|e| McpError::internal(format!("Failed to serialize server info: {}", e)))
    }

    /// List all databases on the server.
    #[resource(
        uri_pattern = "mssql://databases",
        name = "Databases",
        description = "List of all databases on the server",
        mime_type = "application/json"
    )]
    pub async fn resource_databases(&self, uri: &str) -> Result<ResourceContents, McpError> {
        let databases = self
            .metadata
            .list_databases()
            .await
            .map_err(|e| McpError::internal(format!("Failed to list databases: {}", e)))?;

        let response = serde_json::json!({
            "count": databases.len(),
            "databases": databases,
        });

        ResourceContents::json(uri, &response)
            .map_err(|e| McpError::internal(format!("Failed to serialize databases: {}", e)))
    }

    /// List all schemas in the current database.
    #[resource(
        uri_pattern = "mssql://schemas",
        name = "Schemas",
        description = "List of schemas in the current database",
        mime_type = "application/json"
    )]
    pub async fn resource_schemas(&self, uri: &str) -> Result<ResourceContents, McpError> {
        if !self.is_database_mode() {
            return Err(McpError::ResourceAccessDenied {
                uri: uri.to_string(),
                reason: Some("Schemas resource requires database mode (connect to a specific database)".to_string()),
            });
        }

        let schemas = self
            .metadata
            .list_schemas()
            .await
            .map_err(|e| McpError::internal(format!("Failed to list schemas: {}", e)))?;

        let response = serde_json::json!({
            "count": schemas.len(),
            "schemas": schemas,
        });

        ResourceContents::json(uri, &response)
            .map_err(|e| McpError::internal(format!("Failed to serialize schemas: {}", e)))
    }

    /// List all tables in the current database.
    #[resource(
        uri_pattern = "mssql://tables",
        name = "Tables",
        description = "List of all tables with row counts and sizes",
        mime_type = "application/json"
    )]
    pub async fn resource_tables(&self, uri: &str) -> Result<ResourceContents, McpError> {
        if !self.is_database_mode() {
            return Err(McpError::ResourceAccessDenied {
                uri: uri.to_string(),
                reason: Some("Tables resource requires database mode (connect to a specific database)".to_string()),
            });
        }

        let tables = self
            .metadata
            .list_tables(None)
            .await
            .map_err(|e| McpError::internal(format!("Failed to list tables: {}", e)))?;

        let response = serde_json::json!({
            "count": tables.len(),
            "tables": tables,
        });

        ResourceContents::json(uri, &response)
            .map_err(|e| McpError::internal(format!("Failed to serialize tables: {}", e)))
    }

    /// Get detailed information about a specific table.
    #[resource(
        uri_pattern = "mssql://tables/{schema}/{table}",
        name = "Table Details",
        description = "Get detailed information about a specific table including columns",
        mime_type = "application/json"
    )]
    pub async fn resource_table_details(&self, uri: &str) -> Result<ResourceContents, McpError> {
        if !self.is_database_mode() {
            return Err(McpError::ResourceAccessDenied {
                uri: uri.to_string(),
                reason: Some("Table details resource requires database mode".to_string()),
            });
        }

        // Parse schema and table from URI: mssql://tables/{schema}/{table}
        let (schema, table) = parse_resource_path(uri, "tables")?;

        // Validate identifiers
        validate_identifier(&schema).map_err(|e| {
            McpError::invalid_params("table_details", format!("Invalid schema '{}': {}", schema, e))
        })?;
        validate_identifier(&table).map_err(|e| {
            McpError::invalid_params("table_details", format!("Invalid table '{}': {}", table, e))
        })?;

        let columns = self
            .metadata
            .get_table_columns(&schema, &table)
            .await
            .map_err(|e| McpError::internal(format!("Failed to get table columns: {}", e)))?;

        if columns.is_empty() {
            return Err(McpError::resource_not_found(uri));
        }

        let response = serde_json::json!({
            "schema": schema,
            "table": table,
            "column_count": columns.len(),
            "columns": columns,
        });

        ResourceContents::json(uri, &response)
            .map_err(|e| McpError::internal(format!("Failed to serialize table details: {}", e)))
    }

    /// List all views in the current database.
    #[resource(
        uri_pattern = "mssql://views",
        name = "Views",
        description = "List of all views in the database",
        mime_type = "application/json"
    )]
    pub async fn resource_views(&self, uri: &str) -> Result<ResourceContents, McpError> {
        if !self.is_database_mode() {
            return Err(McpError::ResourceAccessDenied {
                uri: uri.to_string(),
                reason: Some("Views resource requires database mode (connect to a specific database)".to_string()),
            });
        }

        let views = self
            .metadata
            .list_views(None)
            .await
            .map_err(|e| McpError::internal(format!("Failed to list views: {}", e)))?;

        let response = serde_json::json!({
            "count": views.len(),
            "views": views,
        });

        ResourceContents::json(uri, &response)
            .map_err(|e| McpError::internal(format!("Failed to serialize views: {}", e)))
    }

    /// Get detailed information about a specific view.
    #[resource(
        uri_pattern = "mssql://views/{schema}/{view}",
        name = "View Details",
        description = "Get detailed information about a specific view including definition",
        mime_type = "application/json"
    )]
    pub async fn resource_view_details(&self, uri: &str) -> Result<ResourceContents, McpError> {
        if !self.is_database_mode() {
            return Err(McpError::ResourceAccessDenied {
                uri: uri.to_string(),
                reason: Some("View details resource requires database mode".to_string()),
            });
        }

        let (schema, view) = parse_resource_path(uri, "views")?;

        validate_identifier(&schema).map_err(|e| {
            McpError::invalid_params("view_details", format!("Invalid schema '{}': {}", schema, e))
        })?;
        validate_identifier(&view).map_err(|e| {
            McpError::invalid_params("view_details", format!("Invalid view '{}': {}", view, e))
        })?;

        let views = self
            .metadata
            .list_views(Some(&schema))
            .await
            .map_err(|e| McpError::internal(format!("Failed to list views: {}", e)))?;

        let view_info = views
            .iter()
            .find(|v| v.view_name.eq_ignore_ascii_case(&view))
            .ok_or_else(|| McpError::resource_not_found(uri))?;

        let response = serde_json::json!({
            "schema": view_info.schema_name,
            "view": view_info.view_name,
            "definition": view_info.definition,
            "is_updatable": view_info.is_updatable,
        });

        ResourceContents::json(uri, &response)
            .map_err(|e| McpError::internal(format!("Failed to serialize view details: {}", e)))
    }

    /// List all stored procedures in the current database.
    #[resource(
        uri_pattern = "mssql://procedures",
        name = "Stored Procedures",
        description = "List of stored procedures",
        mime_type = "application/json"
    )]
    pub async fn resource_procedures(&self, uri: &str) -> Result<ResourceContents, McpError> {
        if !self.is_database_mode() {
            return Err(McpError::ResourceAccessDenied {
                uri: uri.to_string(),
                reason: Some("Procedures resource requires database mode (connect to a specific database)".to_string()),
            });
        }

        let procedures = self
            .metadata
            .list_procedures(None)
            .await
            .map_err(|e| McpError::internal(format!("Failed to list procedures: {}", e)))?;

        let response = serde_json::json!({
            "count": procedures.len(),
            "procedures": procedures,
        });

        ResourceContents::json(uri, &response)
            .map_err(|e| McpError::internal(format!("Failed to serialize procedures: {}", e)))
    }

    /// Get detailed information about a specific stored procedure.
    #[resource(
        uri_pattern = "mssql://procedures/{schema}/{procedure}",
        name = "Procedure Details",
        description = "Get detailed information about a stored procedure including parameters",
        mime_type = "application/json"
    )]
    pub async fn resource_procedure_details(
        &self,
        uri: &str,
    ) -> Result<ResourceContents, McpError> {
        if !self.is_database_mode() {
            return Err(McpError::ResourceAccessDenied {
                uri: uri.to_string(),
                reason: Some("Procedure details resource requires database mode".to_string()),
            });
        }

        let (schema, procedure) = parse_resource_path(uri, "procedures")?;

        validate_identifier(&schema).map_err(|e| {
            McpError::invalid_params(
                "procedure_details",
                format!("Invalid schema '{}': {}", schema, e),
            )
        })?;
        validate_identifier(&procedure).map_err(|e| {
            McpError::invalid_params(
                "procedure_details",
                format!("Invalid procedure '{}': {}", procedure, e),
            )
        })?;

        let definition = self
            .metadata
            .get_procedure_definition(&schema, &procedure)
            .await
            .map_err(|e| McpError::internal(format!("Failed to get procedure definition: {}", e)))?;

        let parameters = self
            .metadata
            .get_procedure_parameters(&schema, &procedure)
            .await
            .map_err(|e| {
                McpError::internal(format!("Failed to get procedure parameters: {}", e))
            })?;

        let response = serde_json::json!({
            "schema": schema,
            "procedure": procedure,
            "definition": definition,
            "parameter_count": parameters.len(),
            "parameters": parameters,
        });

        ResourceContents::json(uri, &response)
            .map_err(|e| McpError::internal(format!("Failed to serialize procedure details: {}", e)))
    }

    /// List all user-defined functions in the current database.
    #[resource(
        uri_pattern = "mssql://functions",
        name = "Functions",
        description = "List of user-defined functions (scalar and table-valued)",
        mime_type = "application/json"
    )]
    pub async fn resource_functions(&self, uri: &str) -> Result<ResourceContents, McpError> {
        if !self.is_database_mode() {
            return Err(McpError::ResourceAccessDenied {
                uri: uri.to_string(),
                reason: Some("Functions resource requires database mode (connect to a specific database)".to_string()),
            });
        }

        let functions = self
            .metadata
            .list_functions(None)
            .await
            .map_err(|e| McpError::internal(format!("Failed to list functions: {}", e)))?;

        let response = serde_json::json!({
            "count": functions.len(),
            "functions": functions,
        });

        ResourceContents::json(uri, &response)
            .map_err(|e| McpError::internal(format!("Failed to serialize functions: {}", e)))
    }

    /// Get detailed information about a specific user-defined function.
    #[resource(
        uri_pattern = "mssql://functions/{schema}/{function}",
        name = "Function Details",
        description = "Get detailed information about a user-defined function including parameters",
        mime_type = "application/json"
    )]
    pub async fn resource_function_details(&self, uri: &str) -> Result<ResourceContents, McpError> {
        if !self.is_database_mode() {
            return Err(McpError::ResourceAccessDenied {
                uri: uri.to_string(),
                reason: Some("Function details resource requires database mode".to_string()),
            });
        }

        let (schema, function) = parse_resource_path(uri, "functions")?;

        validate_identifier(&schema).map_err(|e| {
            McpError::invalid_params(
                "function_details",
                format!("Invalid schema '{}': {}", schema, e),
            )
        })?;
        validate_identifier(&function).map_err(|e| {
            McpError::invalid_params(
                "function_details",
                format!("Invalid function '{}': {}", function, e),
            )
        })?;

        let functions = self
            .metadata
            .list_functions(Some(&schema))
            .await
            .map_err(|e| McpError::internal(format!("Failed to list functions: {}", e)))?;

        let func_info = functions
            .iter()
            .find(|f| f.function_name.eq_ignore_ascii_case(&function))
            .ok_or_else(|| McpError::resource_not_found(uri))?;

        let parameters = self
            .metadata
            .get_function_parameters(&schema, &function)
            .await
            .map_err(|e| McpError::internal(format!("Failed to get function parameters: {}", e)))?;

        let response = serde_json::json!({
            "schema": func_info.schema_name,
            "function": func_info.function_name,
            "type": func_info.function_type,
            "return_type": func_info.return_type,
            "created": func_info.create_date,
            "modified": func_info.modify_date,
            "parameter_count": parameters.len(),
            "parameters": parameters,
            "definition": func_info.definition,
        });

        ResourceContents::json(uri, &response)
            .map_err(|e| McpError::internal(format!("Failed to serialize function details: {}", e)))
    }

    /// List all triggers in the current database.
    #[resource(
        uri_pattern = "mssql://triggers",
        name = "Triggers",
        description = "List of database triggers",
        mime_type = "application/json"
    )]
    pub async fn resource_triggers(&self, uri: &str) -> Result<ResourceContents, McpError> {
        if !self.is_database_mode() {
            return Err(McpError::ResourceAccessDenied {
                uri: uri.to_string(),
                reason: Some("Triggers resource requires database mode (connect to a specific database)".to_string()),
            });
        }

        let triggers = self
            .metadata
            .list_triggers(None)
            .await
            .map_err(|e| McpError::internal(format!("Failed to list triggers: {}", e)))?;

        let response = serde_json::json!({
            "count": triggers.len(),
            "triggers": triggers,
        });

        ResourceContents::json(uri, &response)
            .map_err(|e| McpError::internal(format!("Failed to serialize triggers: {}", e)))
    }

    /// Get detailed information about a specific trigger.
    #[resource(
        uri_pattern = "mssql://triggers/{schema}/{trigger}",
        name = "Trigger Details",
        description = "Get detailed information about a trigger including definition",
        mime_type = "application/json"
    )]
    pub async fn resource_trigger_details(&self, uri: &str) -> Result<ResourceContents, McpError> {
        if !self.is_database_mode() {
            return Err(McpError::ResourceAccessDenied {
                uri: uri.to_string(),
                reason: Some("Trigger details resource requires database mode".to_string()),
            });
        }

        let (schema, trigger) = parse_resource_path(uri, "triggers")?;

        validate_identifier(&schema).map_err(|e| {
            McpError::invalid_params(
                "trigger_details",
                format!("Invalid schema '{}': {}", schema, e),
            )
        })?;
        validate_identifier(&trigger).map_err(|e| {
            McpError::invalid_params(
                "trigger_details",
                format!("Invalid trigger '{}': {}", trigger, e),
            )
        })?;

        let triggers = self
            .metadata
            .list_triggers(Some(&schema))
            .await
            .map_err(|e| McpError::internal(format!("Failed to list triggers: {}", e)))?;

        let trigger_info = triggers
            .iter()
            .find(|t| t.trigger_name.eq_ignore_ascii_case(&trigger))
            .ok_or_else(|| McpError::resource_not_found(uri))?;

        let response = serde_json::json!({
            "schema": trigger_info.schema_name,
            "trigger": trigger_info.trigger_name,
            "parent_object": trigger_info.parent_object,
            "type": trigger_info.trigger_type,
            "status": if trigger_info.is_disabled { "Disabled" } else { "Enabled" },
            "events": trigger_info.trigger_events,
            "created": trigger_info.create_date,
            "modified": trigger_info.modify_date,
            "definition": trigger_info.definition,
        });

        ResourceContents::json(uri, &response)
            .map_err(|e| McpError::internal(format!("Failed to serialize trigger details: {}", e)))
    }

    // =========================================================================
    // Prompts - AI-assisted SQL generation and analysis
    // =========================================================================

    /// Generate a SELECT query for a table with its schema information.
    #[prompt(description = "Generate a SELECT query for a table with its schema information")]
    pub async fn query_table(
        &self,
        schema: Option<String>,
        table: String,
        columns: Option<String>,
        filter: Option<String>,
    ) -> Result<GetPromptResult, McpError> {
        let schema = schema.as_deref().unwrap_or("dbo");

        // Get table columns from metadata
        let column_info = self
            .metadata
            .get_table_columns(schema, &table)
            .await
            .map_err(|e| McpError::internal(format!("Failed to get table columns: {}", e)))?;

        if column_info.is_empty() {
            return Err(McpError::invalid_params(
                "query_table",
                format!("Table not found: {}.{}", schema, table),
            ));
        }

        // Build schema description
        let schema_desc = column_info
            .iter()
            .map(|c| {
                format!(
                    "  - {} ({}{}){}",
                    c.column_name,
                    c.data_type,
                    if c.is_nullable { ", nullable" } else { "" },
                    if c.is_identity { " [IDENTITY]" } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let mut prompt_text = format!(
            r#"Generate a SELECT query for the table [{schema}].[{table}].

## Table Schema

{schema_desc}

## Requirements
"#
        );

        if let Some(cols) = &columns {
            prompt_text.push_str(&format!("- Select only these columns: {}\n", cols));
        } else {
            prompt_text.push_str("- Select all relevant columns\n");
        }

        if let Some(f) = &filter {
            prompt_text.push_str(&format!("- Filter condition: {}\n", f));
        }

        prompt_text.push_str(
            r#"
## Guidelines
- Use proper bracket notation for identifiers: [schema].[table].[column]
- Add appropriate TOP or OFFSET/FETCH for large tables
- Consider using aliases for readability
- Include ORDER BY for deterministic results
"#,
        );

        Ok(GetPromptResult {
            description: Some(format!("Query builder for {}.{}", schema, table)),
            messages: vec![PromptMessage {
                role: Role::User,
                content: Content::text(prompt_text),
            }],
        })
    }

    /// Analyze a table's schema and suggest optimizations or improvements.
    #[prompt(description = "Analyze a table's schema and suggest optimizations or improvements")]
    pub async fn analyze_schema(
        &self,
        schema: Option<String>,
        table: String,
    ) -> Result<GetPromptResult, McpError> {
        let schema = schema.as_deref().unwrap_or("dbo");

        // Get table columns
        let columns = self
            .metadata
            .get_table_columns(schema, &table)
            .await
            .map_err(|e| McpError::internal(format!("Failed to get table columns: {}", e)))?;

        if columns.is_empty() {
            return Err(McpError::invalid_params(
                "analyze_schema",
                format!("Table not found: {}.{}", schema, table),
            ));
        }

        let column_desc = columns
            .iter()
            .map(|c| {
                format!(
                    "| {} | {} | {} | {} | {} | {} |",
                    c.column_name,
                    c.data_type,
                    c.max_length.map(|l| l.to_string()).unwrap_or("-".to_string()),
                    if c.is_nullable { "Yes" } else { "No" },
                    if c.is_identity { "Yes" } else { "No" },
                    c.default_value.as_deref().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt_text = format!(
            r#"Analyze the schema of table [{schema}].[{table}] and provide recommendations.

## Current Schema

| Column | Type | Max Length | Nullable | Identity | Default |
|--------|------|------------|----------|----------|---------|
{column_desc}

## Analysis Requested

Please analyze this schema and provide:

1. **Data Type Review**
   - Are the data types appropriate for the column names/purposes?
   - Could any types be optimized (e.g., varchar(max) -> varchar(n))?

2. **Nullability Assessment**
   - Which nullable columns might benefit from NOT NULL constraints?
   - Are there potential data integrity issues?

3. **Indexing Suggestions**
   - Based on likely query patterns, which columns should be indexed?
   - Any composite index recommendations?

4. **Best Practices**
   - Naming convention compliance
   - Potential normalization issues
   - Missing audit columns (CreatedAt, UpdatedAt, etc.)

5. **Performance Considerations**
   - Data type sizes and storage efficiency
   - Potential query performance impacts
"#
        );

        Ok(GetPromptResult {
            description: Some(format!("Schema analysis for {}.{}", schema, table)),
            messages: vec![PromptMessage {
                role: Role::User,
                content: Content::text(prompt_text),
            }],
        })
    }

    /// Generate an INSERT statement template for a table.
    #[prompt(description = "Generate an INSERT statement template for a table")]
    pub async fn generate_insert(
        &self,
        schema: Option<String>,
        table: String,
    ) -> Result<GetPromptResult, McpError> {
        let schema = schema.as_deref().unwrap_or("dbo");

        // Get table columns
        let columns = self
            .metadata
            .get_table_columns(schema, &table)
            .await
            .map_err(|e| McpError::internal(format!("Failed to get table columns: {}", e)))?;

        if columns.is_empty() {
            return Err(McpError::invalid_params(
                "generate_insert",
                format!("Table not found: {}.{}", schema, table),
            ));
        }

        // Filter out identity and computed columns
        let insertable_columns: Vec<_> = columns
            .iter()
            .filter(|c| !c.is_identity && !c.is_computed)
            .collect();

        let column_list = insertable_columns
            .iter()
            .map(|c| format!("[{}]", c.column_name))
            .collect::<Vec<_>>()
            .join(", ");

        let value_placeholders = insertable_columns
            .iter()
            .map(|c| {
                let placeholder = match c.data_type.to_uppercase().as_str() {
                    "INT" | "BIGINT" | "SMALLINT" | "TINYINT" => "0".to_string(),
                    "BIT" => "0".to_string(),
                    "DECIMAL" | "NUMERIC" | "MONEY" | "SMALLMONEY" => "0.00".to_string(),
                    "FLOAT" | "REAL" => "0.0".to_string(),
                    "DATE" => "'YYYY-MM-DD'".to_string(),
                    "TIME" => "'HH:MM:SS'".to_string(),
                    "DATETIME" | "DATETIME2" | "SMALLDATETIME" => "'YYYY-MM-DD HH:MM:SS'".to_string(),
                    "UNIQUEIDENTIFIER" => "NEWID()".to_string(),
                    _ => format!("N'<{}>'", c.column_name),
                };
                format!("{} /* {} {} */", placeholder, c.column_name, c.data_type)
            })
            .collect::<Vec<_>>()
            .join(",\n    ");

        let column_desc = insertable_columns
            .iter()
            .map(|c| {
                format!(
                    "- {} ({}){}",
                    c.column_name,
                    c.data_type,
                    if !c.is_nullable { " - REQUIRED" } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt_text = format!(
            r#"Generate an INSERT statement for [{schema}].[{table}].

## Insertable Columns

{column_desc}

## Template

```sql
INSERT INTO [{schema}].[{table}] (
    {column_list}
)
VALUES (
    {value_placeholders}
);
```

## Instructions

Replace the placeholder values with actual data. Note:
- Columns marked REQUIRED cannot be NULL
- String values should use N'...' for Unicode support
- Date/time values should use ISO format
- UNIQUEIDENTIFIER can use NEWID() for auto-generation
"#
        );

        Ok(GetPromptResult {
            description: Some(format!("INSERT template for {}.{}", schema, table)),
            messages: vec![PromptMessage {
                role: Role::User,
                content: Content::text(prompt_text),
            }],
        })
    }

    /// Explain what a stored procedure does and how to call it.
    #[prompt(description = "Explain what a stored procedure does and how to call it")]
    pub async fn explain_procedure(
        &self,
        schema: Option<String>,
        procedure: String,
    ) -> Result<GetPromptResult, McpError> {
        let schema = schema.as_deref().unwrap_or("dbo");

        // Get procedure definition
        let definition = self
            .metadata
            .get_procedure_definition(schema, &procedure)
            .await
            .map_err(|e| McpError::internal(format!("Failed to get procedure definition: {}", e)))?;

        // Get procedure parameters
        let parameters = self
            .metadata
            .get_procedure_parameters(schema, &procedure)
            .await
            .map_err(|e| McpError::internal(format!("Failed to get procedure parameters: {}", e)))?;

        let param_desc = if parameters.is_empty() {
            "This procedure has no parameters.".to_string()
        } else {
            parameters
                .iter()
                .map(|p| {
                    format!(
                        "- {} ({}){}{}",
                        p.parameter_name,
                        p.data_type,
                        if p.is_output { " OUTPUT" } else { "" },
                        if p.has_default { " [has default]" } else { "" }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let definition_text = definition.unwrap_or_else(|| "(Definition not available)".to_string());

        let prompt_text = format!(
            r#"Explain the stored procedure [{schema}].[{procedure}].

## Parameters

{param_desc}

## Definition

```sql
{definition_text}
```

## Please Explain

1. **Purpose**: What does this procedure do?
2. **Parameters**: Explain each parameter and its purpose
3. **Logic Flow**: Step-by-step explanation of what happens
4. **Return Values**: What does it return? (result sets, output parameters)
5. **Example Usage**: Show how to call this procedure
6. **Potential Issues**: Any edge cases or error conditions to watch for?
"#
        );

        Ok(GetPromptResult {
            description: Some(format!("Explanation of {}.{}", schema, procedure)),
            messages: vec![PromptMessage {
                role: Role::User,
                content: Content::text(prompt_text),
            }],
        })
    }

    /// Analyze a SQL query and suggest optimizations.
    #[prompt(description = "Analyze a SQL query and suggest optimizations")]
    pub fn optimize_query(
        &self,
        query: String,
    ) -> Result<GetPromptResult, McpError> {
        let prompt_text = format!(
            r#"Analyze and optimize the following SQL query.

## Original Query

```sql
{query}
```

## Analysis Requested

1. **Query Structure**
   - Is the query logically correct?
   - Any syntax issues or anti-patterns?

2. **Performance Issues**
   - Potential table scans
   - Missing indexes
   - Inefficient joins
   - Subquery vs JOIN considerations

3. **Optimizations**
   - Rewrite suggestions
   - Index recommendations
   - Query hints if appropriate

4. **Best Practices**
   - SET NOCOUNT ON for procedures
   - Avoiding SELECT *
   - Proper use of CTEs vs temp tables

5. **Optimized Version**
   - Provide the optimized query with comments
"#
        );

        Ok(GetPromptResult {
            description: Some("Query optimization analysis".to_string()),
            messages: vec![PromptMessage {
                role: Role::User,
                content: Content::text(prompt_text),
            }],
        })
    }

    /// Help debug a SQL Server error with context and suggestions.
    #[prompt(description = "Help debug a SQL Server error with context and suggestions")]
    pub fn debug_error(
        &self,
        error: String,
        query: Option<String>,
    ) -> Result<GetPromptResult, McpError> {
        let mut prompt_text = format!(
            r#"Help debug this SQL Server error.

## Error Message

```
{error}
```
"#
        );

        if let Some(q) = &query {
            prompt_text.push_str(&format!(
                r#"
## Query That Caused the Error

```sql
{q}
```
"#
            ));
        }

        prompt_text.push_str(
            r#"
## Please Provide

1. **Error Explanation**: What does this error mean?
2. **Common Causes**: Why does this error typically occur?
3. **Diagnosis Steps**: How to investigate further
4. **Solutions**: How to fix the issue
5. **Prevention**: How to avoid this error in the future
"#,
        );

        Ok(GetPromptResult {
            description: Some("SQL Server error debugging".to_string()),
            messages: vec![PromptMessage {
                role: Role::User,
                content: Content::text(prompt_text),
            }],
        })
    }
}

// =========================================================================
// Helper Functions
// =========================================================================

/// Parse a resource path to extract schema and object name.
///
/// Expected format: `mssql://{type}/{schema}/{name}` or `mssql://{type}/{qualified_name}`
fn parse_resource_path(uri: &str, resource_type: &str) -> Result<(String, String), McpError> {
    let prefix = format!("mssql://{}/", resource_type);
    let path = uri
        .strip_prefix(&prefix)
        .ok_or_else(|| McpError::invalid_params(resource_type, format!("Invalid URI: {}", uri)))?;

    // Try path format first: schema/name
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        [schema, name] => Ok(((*schema).to_string(), (*name).to_string())),
        [qualified] => {
            // Try qualified format: schema.name
            match parse_qualified_name(qualified) {
                Ok((Some(schema), name)) => Ok((schema, name)),
                Ok((None, name)) => Ok(("dbo".to_string(), name)),
                Err(e) => Err(McpError::invalid_params(
                    resource_type,
                    format!("Invalid qualified name '{}': {}", qualified, e),
                )),
            }
        }
        _ => Err(McpError::invalid_params(
            resource_type,
            format!("Invalid resource path: {}", path),
        )),
    }
}

/// Truncate a string for logging.
fn truncate_for_log(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

/// Format a parameter value for SQL execution.
fn format_parameter_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => {
            // Escape single quotes for SQL
            format!("N'{}'", s.replace('\'', "''"))
        }
        serde_json::Value::Array(arr) => {
            // Convert array to comma-separated string for table-valued parameters
            let elements: Vec<String> = arr.iter().map(format_parameter_value).collect();
            elements.join(", ")
        }
        serde_json::Value::Object(_) => {
            // For objects, serialize as JSON string
            format!(
                "N'{}'",
                serde_json::to_string(value)
                    .unwrap_or_default()
                    .replace('\'', "''")
            )
        }
    }
}

/// Build a parameterized query for sp_executesql.
///
/// Returns (query, param_declarations, param_values) tuple.
fn build_parameterized_query(
    query: &str,
    parameters: &std::collections::HashMap<String, serde_json::Value>,
) -> Result<(String, String, String), McpError> {
    if parameters.is_empty() {
        return Ok((query.to_string(), String::new(), String::new()));
    }

    let mut declarations: Vec<String> = Vec::new();
    let mut values: Vec<String> = Vec::new();

    for (name, value) in parameters {
        // Normalize parameter name (ensure it starts with @)
        let param_name = if name.starts_with('@') {
            name.clone()
        } else {
            format!("@{}", name)
        };

        // Determine SQL type based on JSON type
        let sql_type = match value {
            serde_json::Value::Null => "NVARCHAR(MAX)",
            serde_json::Value::Bool(_) => "BIT",
            serde_json::Value::Number(n) => {
                if n.is_i64() {
                    "BIGINT"
                } else if n.is_f64() {
                    "FLOAT"
                } else {
                    "DECIMAL(38, 10)"
                }
            }
            serde_json::Value::String(_) => "NVARCHAR(MAX)",
            serde_json::Value::Array(_) => "NVARCHAR(MAX)",
            serde_json::Value::Object(_) => "NVARCHAR(MAX)",
        };

        declarations.push(format!("{} {}", param_name, sql_type));
        values.push(format!(
            "{} = {}",
            param_name,
            format_parameter_value(value)
        ));
    }

    Ok((
        query.to_string(),
        declarations.join(", "),
        values.join(", "),
    ))
}

/// Encode an offset as a cursor string.
fn encode_cursor(offset: usize) -> String {
    use std::io::Write;
    let mut bytes = Vec::new();
    write!(&mut bytes, "cursor:{}", offset).unwrap();
    // Simple base64-like encoding using hex
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Decode a cursor string to an offset.
fn decode_cursor(cursor: &str) -> Result<usize, String> {
    // Decode hex string back to bytes
    let bytes: Result<Vec<u8>, _> = (0..cursor.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cursor[i..i + 2], 16))
        .collect();

    let bytes = bytes.map_err(|_| "Invalid cursor format")?;
    let s = String::from_utf8(bytes).map_err(|_| "Invalid cursor encoding")?;

    if !s.starts_with("cursor:") {
        return Err("Invalid cursor prefix".to_string());
    }

    s[7..]
        .parse::<usize>()
        .map_err(|_| "Invalid cursor offset".to_string())
}

/// Parse a table name in schema.table format.
fn parse_table_name(table_ref: &str) -> Result<(String, String), McpError> {
    match parse_qualified_name(table_ref) {
        Ok((Some(schema), table)) => Ok((schema, table)),
        Ok((None, table)) => Ok(("dbo".to_string(), table)),
        Err(e) => Err(McpError::invalid_params(
            "table",
            format!("Invalid table name '{}': {}", table_ref, e),
        )),
    }
}

// =========================================================================
// CompletionHandler Implementation
// =========================================================================

use mcpkit::{CompletionHandler, Context};

/// Completion suggestions for SQL Server objects.
///
/// Provides autocompletion for:
/// - Resource URIs (tables, views, procedures, functions, triggers)
/// - Prompt arguments (schema names, table names, procedure names)
impl CompletionHandler for MssqlMcpServer {
    /// Complete a partial resource URI.
    ///
    /// Supports completion for resource patterns like:
    /// - `mssql://tables/{schema}/{table}`
    /// - `mssql://views/{schema}/{view}`
    /// - `mssql://procedures/{schema}/{procedure}`
    /// - `mssql://functions/{schema}/{function}`
    /// - `mssql://triggers/{schema}/{trigger}`
    async fn complete_resource(
        &self,
        partial_uri: &str,
        _ctx: &Context<'_>,
    ) -> Result<Vec<String>, McpError> {
        debug!("Completing resource URI: {}", partial_uri);

        // Parse the partial URI to determine what we're completing
        let completions = if partial_uri.starts_with("mssql://tables/") {
            self.complete_table_resource(partial_uri).await?
        } else if partial_uri.starts_with("mssql://views/") {
            self.complete_view_resource(partial_uri).await?
        } else if partial_uri.starts_with("mssql://procedures/") {
            self.complete_procedure_resource(partial_uri).await?
        } else if partial_uri.starts_with("mssql://functions/") {
            self.complete_function_resource(partial_uri).await?
        } else if partial_uri.starts_with("mssql://triggers/") {
            self.complete_trigger_resource(partial_uri).await?
        } else if partial_uri.starts_with("mssql://") {
            // Complete top-level resource types
            vec![
                "mssql://server/info".to_string(),
                "mssql://databases".to_string(),
                "mssql://schemas".to_string(),
                "mssql://tables".to_string(),
                "mssql://views".to_string(),
                "mssql://procedures".to_string(),
                "mssql://functions".to_string(),
                "mssql://triggers".to_string(),
            ]
            .into_iter()
            .filter(|uri| uri.starts_with(partial_uri))
            .collect()
        } else {
            Vec::new()
        };

        Ok(completions)
    }

    /// Complete a partial prompt argument.
    ///
    /// Supports completion for prompts:
    /// - `query_table`, `analyze_schema`, `generate_insert`: schema, table
    /// - `explain_procedure`: schema, procedure
    async fn complete_prompt_arg(
        &self,
        prompt_name: &str,
        arg_name: &str,
        partial_value: &str,
        _ctx: &Context<'_>,
    ) -> Result<Vec<String>, McpError> {
        debug!(
            "Completing prompt '{}' arg '{}' with prefix '{}'",
            prompt_name, arg_name, partial_value
        );

        let completions = match (prompt_name, arg_name) {
            // Schema completion for all prompts that have schema arguments
            (_, "schema") => self.complete_schemas(partial_value).await?,

            // Table completion for table-related prompts
            ("query_table" | "analyze_schema" | "generate_insert", "table") => {
                self.complete_tables(partial_value).await?
            }

            // Procedure completion for procedure-related prompts
            ("explain_procedure", "procedure") => self.complete_procedures(partial_value).await?,

            // Column completion would be context-dependent (needs table name)
            // For now, return empty
            (_, "columns") => Vec::new(),

            // Query and error completion - no suggestions
            (_, "query" | "error") => Vec::new(),

            _ => Vec::new(),
        };

        Ok(completions)
    }
}

/// Helper methods for completion queries.
impl MssqlMcpServer {
    /// Complete table resource URIs.
    async fn complete_table_resource(&self, partial_uri: &str) -> Result<Vec<String>, McpError> {
        let prefix = "mssql://tables/";
        let path = partial_uri.strip_prefix(prefix).unwrap_or("");

        if path.is_empty() || !path.contains('/') {
            // Complete schema names
            let schemas = self.get_schema_names().await?;
            Ok(schemas
                .into_iter()
                .filter(|s| s.starts_with(path))
                .map(|s| format!("{}{}/", prefix, s))
                .collect())
        } else {
            // Complete table names within schema
            let parts: Vec<&str> = path.split('/').collect();
            let schema = parts[0];
            let table_prefix = parts.get(1).unwrap_or(&"");

            let tables = self.get_table_names(schema).await?;
            Ok(tables
                .into_iter()
                .filter(|t| t.starts_with(table_prefix))
                .map(|t| format!("{}{}/{}", prefix, schema, t))
                .collect())
        }
    }

    /// Complete view resource URIs.
    async fn complete_view_resource(&self, partial_uri: &str) -> Result<Vec<String>, McpError> {
        let prefix = "mssql://views/";
        let path = partial_uri.strip_prefix(prefix).unwrap_or("");

        if path.is_empty() || !path.contains('/') {
            let schemas = self.get_schema_names().await?;
            Ok(schemas
                .into_iter()
                .filter(|s| s.starts_with(path))
                .map(|s| format!("{}{}/", prefix, s))
                .collect())
        } else {
            let parts: Vec<&str> = path.split('/').collect();
            let schema = parts[0];
            let view_prefix = parts.get(1).unwrap_or(&"");

            let views = self.get_view_names(schema).await?;
            Ok(views
                .into_iter()
                .filter(|v| v.starts_with(view_prefix))
                .map(|v| format!("{}{}/{}", prefix, schema, v))
                .collect())
        }
    }

    /// Complete procedure resource URIs.
    async fn complete_procedure_resource(
        &self,
        partial_uri: &str,
    ) -> Result<Vec<String>, McpError> {
        let prefix = "mssql://procedures/";
        let path = partial_uri.strip_prefix(prefix).unwrap_or("");

        if path.is_empty() || !path.contains('/') {
            let schemas = self.get_schema_names().await?;
            Ok(schemas
                .into_iter()
                .filter(|s| s.starts_with(path))
                .map(|s| format!("{}{}/", prefix, s))
                .collect())
        } else {
            let parts: Vec<&str> = path.split('/').collect();
            let schema = parts[0];
            let proc_prefix = parts.get(1).unwrap_or(&"");

            let procs = self.get_procedure_names(schema).await?;
            Ok(procs
                .into_iter()
                .filter(|p| p.starts_with(proc_prefix))
                .map(|p| format!("{}{}/{}", prefix, schema, p))
                .collect())
        }
    }

    /// Complete function resource URIs.
    async fn complete_function_resource(&self, partial_uri: &str) -> Result<Vec<String>, McpError> {
        let prefix = "mssql://functions/";
        let path = partial_uri.strip_prefix(prefix).unwrap_or("");

        if path.is_empty() || !path.contains('/') {
            let schemas = self.get_schema_names().await?;
            Ok(schemas
                .into_iter()
                .filter(|s| s.starts_with(path))
                .map(|s| format!("{}{}/", prefix, s))
                .collect())
        } else {
            let parts: Vec<&str> = path.split('/').collect();
            let schema = parts[0];
            let func_prefix = parts.get(1).unwrap_or(&"");

            let funcs = self.get_function_names(schema).await?;
            Ok(funcs
                .into_iter()
                .filter(|f| f.starts_with(func_prefix))
                .map(|f| format!("{}{}/{}", prefix, schema, f))
                .collect())
        }
    }

    /// Complete trigger resource URIs.
    async fn complete_trigger_resource(&self, partial_uri: &str) -> Result<Vec<String>, McpError> {
        let prefix = "mssql://triggers/";
        let path = partial_uri.strip_prefix(prefix).unwrap_or("");

        if path.is_empty() || !path.contains('/') {
            let schemas = self.get_schema_names().await?;
            Ok(schemas
                .into_iter()
                .filter(|s| s.starts_with(path))
                .map(|s| format!("{}{}/", prefix, s))
                .collect())
        } else {
            let parts: Vec<&str> = path.split('/').collect();
            let schema = parts[0];
            let trigger_prefix = parts.get(1).unwrap_or(&"");

            let triggers = self.get_trigger_names(schema).await?;
            Ok(triggers
                .into_iter()
                .filter(|t| t.starts_with(trigger_prefix))
                .map(|t| format!("{}{}/{}", prefix, schema, t))
                .collect())
        }
    }

    /// Complete schema names for prompt arguments.
    async fn complete_schemas(&self, prefix: &str) -> Result<Vec<String>, McpError> {
        let schemas = self.get_schema_names().await?;
        Ok(schemas
            .into_iter()
            .filter(|s| s.to_lowercase().starts_with(&prefix.to_lowercase()))
            .collect())
    }

    /// Complete table names for prompt arguments.
    async fn complete_tables(&self, prefix: &str) -> Result<Vec<String>, McpError> {
        // If prefix contains a dot, assume schema.table format
        if let Some((schema, table_prefix)) = prefix.split_once('.') {
            let tables = self.get_table_names(schema).await?;
            Ok(tables
                .into_iter()
                .filter(|t| t.to_lowercase().starts_with(&table_prefix.to_lowercase()))
                .map(|t| format!("{}.{}", schema, t))
                .collect())
        } else {
            // Search across all schemas (limited to dbo for performance)
            let tables = self.get_table_names("dbo").await?;
            Ok(tables
                .into_iter()
                .filter(|t| t.to_lowercase().starts_with(&prefix.to_lowercase()))
                .collect())
        }
    }

    /// Complete procedure names for prompt arguments.
    async fn complete_procedures(&self, prefix: &str) -> Result<Vec<String>, McpError> {
        if let Some((schema, proc_prefix)) = prefix.split_once('.') {
            let procs = self.get_procedure_names(schema).await?;
            Ok(procs
                .into_iter()
                .filter(|p| p.to_lowercase().starts_with(&proc_prefix.to_lowercase()))
                .map(|p| format!("{}.{}", schema, p))
                .collect())
        } else {
            let procs = self.get_procedure_names("dbo").await?;
            Ok(procs
                .into_iter()
                .filter(|p| p.to_lowercase().starts_with(&prefix.to_lowercase()))
                .collect())
        }
    }

    /// Get schema names from the database.
    async fn get_schema_names(&self) -> Result<Vec<String>, McpError> {
        use crate::database::types::SqlValue;

        let query = "SELECT name FROM sys.schemas WHERE name NOT IN ('sys', 'INFORMATION_SCHEMA', 'guest') ORDER BY name";
        let result = self
            .executor
            .execute_raw(query)
            .await
            .map_err(|e| McpError::internal(format!("Failed to get schemas: {}", e)))?;

        Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                row.columns.get("name").and_then(|v| match v {
                    SqlValue::String(s) => Some(s.clone()),
                    _ => None,
                })
            })
            .collect())
    }

    /// Get table names for a schema.
    async fn get_table_names(&self, schema: &str) -> Result<Vec<String>, McpError> {
        use crate::database::types::SqlValue;

        let safe_schema = safe_identifier(schema)
            .map_err(|e| McpError::invalid_params("schema", e.to_string()))?;
        let query = format!(
            "SELECT name FROM sys.tables WHERE schema_id = SCHEMA_ID('{}') ORDER BY name",
            safe_schema
        );
        let result = self
            .executor
            .execute_raw(&query)
            .await
            .map_err(|e| McpError::internal(format!("Failed to get tables: {}", e)))?;

        Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                row.columns.get("name").and_then(|v| match v {
                    SqlValue::String(s) => Some(s.clone()),
                    _ => None,
                })
            })
            .collect())
    }

    /// Get view names for a schema.
    async fn get_view_names(&self, schema: &str) -> Result<Vec<String>, McpError> {
        use crate::database::types::SqlValue;

        let safe_schema = safe_identifier(schema)
            .map_err(|e| McpError::invalid_params("schema", e.to_string()))?;
        let query = format!(
            "SELECT name FROM sys.views WHERE schema_id = SCHEMA_ID('{}') ORDER BY name",
            safe_schema
        );
        let result = self
            .executor
            .execute_raw(&query)
            .await
            .map_err(|e| McpError::internal(format!("Failed to get views: {}", e)))?;

        Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                row.columns.get("name").and_then(|v| match v {
                    SqlValue::String(s) => Some(s.clone()),
                    _ => None,
                })
            })
            .collect())
    }

    /// Get procedure names for a schema.
    async fn get_procedure_names(&self, schema: &str) -> Result<Vec<String>, McpError> {
        use crate::database::types::SqlValue;

        let safe_schema = safe_identifier(schema)
            .map_err(|e| McpError::invalid_params("schema", e.to_string()))?;
        let query = format!(
            "SELECT name FROM sys.procedures WHERE schema_id = SCHEMA_ID('{}') ORDER BY name",
            safe_schema
        );
        let result = self
            .executor
            .execute_raw(&query)
            .await
            .map_err(|e| McpError::internal(format!("Failed to get procedures: {}", e)))?;

        Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                row.columns.get("name").and_then(|v| match v {
                    SqlValue::String(s) => Some(s.clone()),
                    _ => None,
                })
            })
            .collect())
    }

    /// Get function names for a schema.
    async fn get_function_names(&self, schema: &str) -> Result<Vec<String>, McpError> {
        use crate::database::types::SqlValue;

        let safe_schema = safe_identifier(schema)
            .map_err(|e| McpError::invalid_params("schema", e.to_string()))?;
        let query = format!(
            "SELECT name FROM sys.objects WHERE type IN ('FN', 'IF', 'TF') AND schema_id = SCHEMA_ID('{}') ORDER BY name",
            safe_schema
        );
        let result = self
            .executor
            .execute_raw(&query)
            .await
            .map_err(|e| McpError::internal(format!("Failed to get functions: {}", e)))?;

        Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                row.columns.get("name").and_then(|v| match v {
                    SqlValue::String(s) => Some(s.clone()),
                    _ => None,
                })
            })
            .collect())
    }

    /// Get trigger names for a schema.
    async fn get_trigger_names(&self, schema: &str) -> Result<Vec<String>, McpError> {
        use crate::database::types::SqlValue;

        let safe_schema = safe_identifier(schema)
            .map_err(|e| McpError::invalid_params("schema", e.to_string()))?;
        let query = format!(
            "SELECT t.name FROM sys.triggers t
             JOIN sys.objects o ON t.parent_id = o.object_id
             WHERE o.schema_id = SCHEMA_ID('{}') ORDER BY t.name",
            safe_schema
        );
        let result = self
            .executor
            .execute_raw(&query)
            .await
            .map_err(|e| McpError::internal(format!("Failed to get triggers: {}", e)))?;

        Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                row.columns.get("name").and_then(|v| match v {
                    SqlValue::String(s) => Some(s.clone()),
                    _ => None,
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_encoding() {
        let offset = 100;
        let cursor = encode_cursor(offset);
        let decoded = decode_cursor(&cursor).unwrap();
        assert_eq!(offset, decoded);
    }

    #[test]
    fn test_cursor_encoding_large() {
        let offset = 10000;
        let cursor = encode_cursor(offset);
        let decoded = decode_cursor(&cursor).unwrap();
        assert_eq!(offset, decoded);
    }

    #[test]
    fn test_build_parameterized_query() {
        let mut params = std::collections::HashMap::new();
        params.insert("name".to_string(), serde_json::json!("test"));
        params.insert("id".to_string(), serde_json::json!(42));

        let (_query, decls, vals) = build_parameterized_query(
            "SELECT * FROM Users WHERE name = @name AND id = @id",
            &params,
        )
        .unwrap();

        assert!(!decls.is_empty());
        assert!(!vals.is_empty());
        assert!(decls.contains("NVARCHAR(MAX)"));
        assert!(decls.contains("BIGINT"));
    }

    #[test]
    fn test_format_parameter_value() {
        assert_eq!(format_parameter_value(&serde_json::json!(null)), "NULL");
        assert_eq!(format_parameter_value(&serde_json::json!(true)), "1");
        assert_eq!(format_parameter_value(&serde_json::json!(false)), "0");
        assert_eq!(format_parameter_value(&serde_json::json!(42)), "42");
        assert_eq!(
            format_parameter_value(&serde_json::json!("test")),
            "N'test'"
        );
        assert_eq!(
            format_parameter_value(&serde_json::json!("it's a test")),
            "N'it''s a test'"
        );
    }
}

// =========================================================================
// TaskHandler Implementation
// =========================================================================

use mcpkit::types::task::{Task, TaskId, TaskProgress, TaskStatus};
use mcpkit::TaskHandler;

/// Task handler for long-running SQL Server operations.
///
/// Maps our async query sessions to MCP tasks, providing:
/// - Task listing (list_tasks)
/// - Task status (get_task)
/// - Task cancellation (cancel_task)
///
/// This bridges our existing session system with the MCP task protocol,
/// allowing MCP clients to monitor and cancel long-running queries.
impl TaskHandler for MssqlMcpServer {
    /// List all tasks (async query sessions).
    ///
    /// Returns all active async query sessions as MCP tasks.
    async fn list_tasks(&self, _ctx: &Context<'_>) -> Result<Vec<Task>, McpError> {
        debug!("Listing all tasks");

        let state = self.state.read().await;
        let sessions = state.list_sessions();

        let tasks = sessions
            .into_iter()
            .map(|summary| {
                let status = match summary.status.as_str() {
                    "running" => TaskStatus::Running,
                    "completed" => TaskStatus::Completed,
                    "failed" => TaskStatus::Failed,
                    "cancelled" => TaskStatus::Cancelled,
                    _ => TaskStatus::Pending,
                };

                let mut task = Task::new(TaskId::new(&summary.id));
                task.status = status;
                task.tool = Some("execute_query_async".to_string());
                task.description = Some(summary.query_preview);

                // Add progress for running tasks
                if status == TaskStatus::Running {
                    task.progress = Some(TaskProgress::new(summary.progress as u64).total(100));
                }

                task
            })
            .collect();

        Ok(tasks)
    }

    /// Get a specific task by ID.
    ///
    /// Returns detailed information about an async query session.
    async fn get_task(
        &self,
        id: &TaskId,
        _ctx: &Context<'_>,
    ) -> Result<Option<Task>, McpError> {
        debug!("Getting task: {}", id);

        let state = self.state.read().await;
        let session = match state.get_session(id.as_str()) {
            Some(s) => s,
            None => return Ok(None),
        };

        let status = match session.status {
            crate::state::SessionStatus::Running => TaskStatus::Running,
            crate::state::SessionStatus::Completed => TaskStatus::Completed,
            crate::state::SessionStatus::Failed => TaskStatus::Failed,
            crate::state::SessionStatus::Cancelled => TaskStatus::Cancelled,
        };

        let mut task = Task::new(TaskId::new(&session.id));
        task.status = status;
        task.tool = Some("execute_query_async".to_string());
        task.description = Some(truncate_for_log(&session.query, 200));

        // Add progress for running tasks
        if status == TaskStatus::Running {
            task.progress = Some(
                TaskProgress::new(session.progress as u64)
                    .total(100)
                    .message(format!("Query in progress ({}%)", session.progress)),
            );
        }

        // Add result for completed tasks
        if status == TaskStatus::Completed {
            if let Some(ref result) = session.result {
                task.result = Some(json!({
                    "rows_affected": result.rows_affected,
                    "row_count": result.rows.len(),
                    "columns": result.columns,
                }));
            }
        }

        // Add error for failed tasks
        if status == TaskStatus::Failed {
            if let Some(ref error) = session.error {
                task.error = Some(mcpkit::types::task::TaskError::new(-1, error.clone()));
            }
        }

        Ok(Some(task))
    }

    /// Cancel a running task.
    ///
    /// Uses native SQL Server query cancellation via Attention packets
    /// when a CancelHandle is available.
    async fn cancel_task(&self, id: &TaskId, _ctx: &Context<'_>) -> Result<bool, McpError> {
        info!("Cancelling task: {}", id);

        let session_id = id.as_str();

        // First, check if the session exists and is running
        {
            let state = self.state.read().await;
            match state.get_session(session_id) {
                Some(session) if session.status == crate::state::SessionStatus::Running => {
                    // Session is running, proceed with cancellation
                }
                Some(_) => {
                    // Session exists but is not running
                    return Ok(false);
                }
                None => {
                    // Session doesn't exist
                    return Ok(false);
                }
            }
        }

        // Attempt native SQL Server cancellation via CancelHandle
        let mut state = self.state.write().await;

        // Try to get and use the cancel handle
        if let Some(handle) = state.get_cancel_handle(session_id) {
            // Clone the handle to use outside the borrow
            let handle = handle.clone();
            drop(state); // Release the lock before async operation

            // Send cancellation request via Attention packet
            match handle.cancel().await {
                Ok(()) => {
                    debug!("Native cancellation sent for task {}", id);
                }
                Err(e) => {
                    warn!("Failed to send native cancellation for task {}: {}", id, e);
                    // Continue to mark as cancelled anyway
                }
            }

            // Re-acquire lock to update state
            let mut state = self.state.write().await;

            // Mark the session as cancelled
            if let Some(session) = state.get_session_mut(session_id) {
                session.cancel();
            }

            // Clean up the cancel handle
            state.remove_cancel_handle(session_id);

            return Ok(true);
        }

        // No cancel handle - just mark as cancelled
        if let Some(session) = state.get_session_mut(session_id) {
            session.cancel();
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
