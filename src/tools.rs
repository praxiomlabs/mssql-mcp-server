//! MCP Tools for SQL Server operations.
//!
//! Tools are action-oriented operations that execute queries and procedures:
//!
//! - `execute_query`: Execute arbitrary SQL queries
//! - `execute_parameterized`: Execute parameterized queries (SQL injection safe)
//! - `execute_procedure`: Execute stored procedures
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

mod inputs;

pub use inputs::*;

use crate::security::{parse_qualified_name, safe_identifier, validate_identifier};
use crate::server::MssqlMcpServer;
use crate::state::{IsolationLevel, SessionStatus, TransactionStatus};
use mcpkit::prelude::*;
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
    #[tool(description = "Execute a SQL query and return results. Supports SELECT, INSERT, UPDATE, DELETE based on security mode.")]
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
        let result = if QueryExecutor::contains_go_separator(&input.query) {
            // Multi-batch query with GO separators
            // Pass database context so each batch gets the USE prefix
            debug!("Using multi-batch execution for script with GO separators");
            match self
                .executor
                .execute_multi_batch_with_db(&input.query, current_db.as_deref())
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("Multi-batch execution failed: {}", e);
                    return Ok(ToolOutput::error(format!("Query execution failed: {}", e)));
                }
            }
        } else if QueryExecutor::requires_raw_execution(&input.query) {
            // Batch-first DDL statements (CREATE VIEW/PROC/FUNC/TRIGGER/SCHEMA)
            // must be executed using simple_query to avoid sp_executesql wrapper
            debug!("Using raw execution for batch-first DDL statement");
            let effective_query = match &current_db {
                Some(db) => format!("USE [{}];\n{}", db, input.query),
                None => input.query.clone(),
            };
            match self.executor.execute_raw(&effective_query).await {
                Ok(r) => r,
                Err(e) => {
                    warn!("Raw query execution failed: {}", e);
                    return Ok(ToolOutput::error(format!("Query execution failed: {}", e)));
                }
            }
        } else {
            // Standard execution with optional database context
            let effective_query = match &current_db {
                Some(db) => format!("USE [{}];\n{}", db, input.query),
                None => input.query.clone(),
            };
            match self
                .executor
                .execute_with_options(&effective_query, max_rows, input.timeout_seconds)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("Query execution failed: {}", e);
                    return Ok(ToolOutput::error(format!("Query execution failed: {}", e)));
                }
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
    #[tool(description = "Get the execution plan for a SQL query. Useful for query optimization.")]
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
    #[tool(description = "Execute a stored procedure with parameters. Returns result sets and output parameters.")]
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

    // =========================================================================
    // Async Session Tools
    // =========================================================================

    /// Start an asynchronous query execution.
    ///
    /// Returns a session ID that can be used to check status and retrieve results.
    #[tool(description = "Start an asynchronous query execution. Returns a session ID to check status later.")]
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

        // Spawn the async execution task
        let executor = self.executor.clone();
        let state = self.state.clone();
        let max_rows = input
            .max_rows
            .unwrap_or(self.config.security.max_result_rows);
        let timeout_seconds = input.timeout_seconds;
        let query = input.query;
        let sid = session_id.clone();

        tokio::spawn(async move {
            let result = executor
                .execute_with_options(&query, max_rows, timeout_seconds)
                .await;

            let mut state = state.write().await;
            if let Some(session) = state.get_session_mut(&sid) {
                match result {
                    Ok(r) => {
                        info!("Async query {} completed successfully", sid);
                        session.complete(r);
                    }
                    Err(e) => {
                        warn!("Async query {} failed: {}", sid, e);
                        session.fail(e.to_string());
                    }
                }
            }
        });

        let response = json!({
            "session_id": session_id,
            "status": "running",
            "message": "Query execution started. Use get_session_status to check progress."
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| format!("Session ID: {}", session_id)),
        ))
    }

    /// Get the status of an async query session.
    #[tool(description = "Get the status and results of an async query session.")]
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
    #[tool(description = "Cancel a running async query session.")]
    pub async fn cancel_session(
        &self,
        input: CancelSessionInput,
    ) -> Result<ToolOutput, McpError> {
        let mut state = self.state.write().await;

        let session = match state.get_session_mut(&input.session_id) {
            Some(s) => s,
            None => {
                return Ok(ToolOutput::error(format!(
                    "Session not found: {}",
                    input.session_id
                )));
            }
        };

        if !session.is_running() {
            return Ok(ToolOutput::error(format!(
                "Session {} is not running (status: {})",
                input.session_id, session.status
            )));
        }

        session.cancel();
        info!("Session {} cancelled", input.session_id);

        let response = json!({
            "session_id": input.session_id,
            "status": "cancelled",
            "message": "Session cancelled successfully"
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| "Session cancelled".to_string()),
        ))
    }

    /// List all async query sessions.
    #[tool(description = "List all async query sessions with optional status filter.")]
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
    #[tool(description = "Get the results of a completed async query session with formatting options.")]
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
    #[tool(description = "Test database connectivity and return health status with optional diagnostics.")]
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
    #[tool(description = "Set the default query timeout in seconds. Affects subsequent query executions.")]
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
    #[tool(description = "Get the current default query timeout and related configuration.")]
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
    #[tool(description = "Execute a SQL query with parameters. Safer than raw queries as parameters are bound separately.")]
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
    #[tool(description = "Start a new database transaction with optional name and isolation level.")]
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
                &isolation_level.to_string(),
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
    #[tool(description = "Commit a transaction, making all changes permanent.")]
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
    #[tool(description = "Rollback a transaction, undoing all changes.")]
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
    #[tool(description = "Execute a SQL statement within an active transaction.")]
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
    #[tool(description = "Start a pinned session with a dedicated connection. Use for temp tables (#tables) and session-scoped state that needs to persist across queries.")]
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
    #[tool(description = "Execute a SQL statement within a pinned session. Temp tables and session state persist across calls.")]
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
    #[tool(description = "End a pinned session and release its dedicated connection. Any temp tables will be automatically dropped.")]
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
    #[tool(description = "List all active pinned sessions with their statistics.")]
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
    #[tool(description = "Execute a SQL query with pagination support. Query must include ORDER BY for consistent results.")]
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
    #[tool(description = "Switch the connection to a different database on the same server.")]
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
    #[tool(description = "Analyze a SQL query and recommend indexes for better performance.")]
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
    #[tool(description = "Compare objects between two schemas in the same database.")]
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
    #[tool(description = "Compare the structure of two tables, including columns, indexes, and constraints.")]
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
    #[tool(description = "Get a random or stratified sample of data from a table.")]
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
    #[tool(description = "Insert multiple rows into a table efficiently using batched inserts.")]
    pub async fn bulk_insert(
        &self,
        input: BulkInsertInput,
    ) -> Result<ToolOutput, McpError> {
        debug!(
            "Bulk inserting {} rows into {}",
            input.rows.len(),
            input.table
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
        let mut total_inserted = 0;
        let mut errors: Vec<String> = Vec::new();

        // Process in batches
        for chunk in input.rows.chunks(batch_size) {
            let values: Vec<String> = chunk
                .iter()
                .map(|row| {
                    let formatted_values: Vec<String> =
                        row.iter().map(format_parameter_value).collect();
                    format!("({})", formatted_values.join(", "))
                })
                .collect();

            let insert_query = format!(
                "INSERT INTO {} ({}) VALUES {}",
                escaped_table,
                escaped_columns.join(", "),
                values.join(", ")
            );

            match self.executor.execute(&insert_query).await {
                Ok(result) => {
                    total_inserted += result.rows_affected as usize;
                }
                Err(e) => {
                    errors.push(format!("Batch error: {}", e));
                }
            }
        }

        let response = json!({
            "table": input.table,
            "rows_requested": input.rows.len(),
            "rows_inserted": total_inserted,
            "batch_size": batch_size,
            "batches": input.rows.len().div_ceil(batch_size),
            "errors": errors,
            "status": if errors.is_empty() { "success" } else { "partial" },
        });

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| format!("Inserted {} rows", total_inserted)),
        ))
    }

    /// Export query results to various formats.
    #[tool(description = "Export query results in CSV, JSON, or JSON Lines format.")]
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
    #[tool(description = "Get SQL Server performance metrics including connections, queries, and memory usage.")]
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
    #[tool(description = "Analyze a SQL query for performance issues and optimization opportunities.")]
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
    #[tool(description = "Get connection pool metrics including active connections, idle connections, and pool health.")]
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
    #[tool(description = "Get internal server metrics including query counts, latency, cache stats, and transaction counts.")]
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
}

// =========================================================================
// Helper Functions
// =========================================================================

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
