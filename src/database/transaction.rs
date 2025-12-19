//! Transaction connection management.
//!
//! This module provides dedicated connections for database transactions.
//! Unlike pooled connections, transaction connections are held for the
//! entire lifetime of a transaction to maintain transaction state.

use super::auth::{create_connection, truncate_for_log, RawConnection};
use crate::config::DatabaseConfig;
use crate::database::query::{ColumnInfo, QueryResult, ResultRow};
use crate::database::types::TypeMapper;
use crate::error::McpError;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// Manager for transaction-dedicated connections.
///
/// This struct manages connections that are held for the duration of a transaction.
/// Each transaction gets its own dedicated connection that is not returned to the
/// pool until the transaction is committed or rolled back.
pub struct TransactionManager {
    /// Database configuration for creating new connections.
    db_config: Arc<DatabaseConfig>,

    /// Active transaction connections keyed by transaction ID.
    connections: Mutex<HashMap<String, RawConnection>>,

    /// Maximum rows to return from queries.
    max_rows: usize,
}

impl TransactionManager {
    /// Create a new transaction manager.
    pub fn new(db_config: Arc<DatabaseConfig>, max_rows: usize) -> Self {
        Self {
            db_config,
            connections: Mutex::new(HashMap::new()),
            max_rows,
        }
    }

    /// Create a new raw connection using the database configuration.
    async fn create_txn_connection(&self) -> Result<RawConnection, McpError> {
        create_connection(&self.db_config, Some("txn")).await
    }

    /// Begin a new transaction and store its dedicated connection.
    ///
    /// Returns the transaction ID if successful.
    pub async fn begin_transaction(
        &self,
        transaction_id: &str,
        isolation_level: &str,
        name: Option<&str>,
    ) -> Result<(), McpError> {
        // Create a dedicated connection for this transaction
        let mut conn = self.create_txn_connection().await?;

        // Set isolation level and begin transaction
        let set_isolation = format!("SET TRANSACTION ISOLATION LEVEL {}", isolation_level);
        let begin_tx = match name {
            Some(n) => format!("BEGIN TRANSACTION [{}]", n.replace(']', "]]")),
            None => "BEGIN TRANSACTION".to_string(),
        };

        // Execute as separate statements on the same connection
        // We must consume the results from simple_query to complete the operation
        conn.simple_query(&set_isolation)
            .await
            .map_err(|e| McpError::query_error(format!("Failed to set isolation level: {}", e)))?
            .into_results()
            .await
            .map_err(|e| McpError::query_error(format!("Failed to set isolation level: {}", e)))?;

        conn.simple_query(&begin_tx)
            .await
            .map_err(|e| McpError::query_error(format!("Failed to begin transaction: {}", e)))?
            .into_results()
            .await
            .map_err(|e| McpError::query_error(format!("Failed to begin transaction: {}", e)))?;

        // Store the connection
        let mut connections = self.connections.lock().await;
        connections.insert(transaction_id.to_string(), conn);

        debug!(
            "Transaction {} started with dedicated connection",
            transaction_id
        );
        Ok(())
    }

    /// Execute a query within an existing transaction.
    pub async fn execute_in_transaction(
        &self,
        transaction_id: &str,
        query: &str,
    ) -> Result<QueryResult, McpError> {
        let start = Instant::now();

        let mut connections = self.connections.lock().await;
        let conn = connections.get_mut(transaction_id).ok_or_else(|| {
            McpError::Session(format!(
                "Transaction connection not found: {}",
                transaction_id
            ))
        })?;

        debug!(
            "Executing in transaction {}: {}",
            transaction_id,
            truncate_for_log(query, 100)
        );

        // Execute query
        let stream = conn
            .query(query, &[])
            .await
            .map_err(|e| McpError::query_error(format!("Query execution failed: {}", e)))?;

        // Process results
        let result = self.process_stream(stream, self.max_rows, start).await?;

        debug!(
            "Transaction query completed: {} rows in {} ms",
            result.rows.len(),
            result.execution_time_ms
        );

        Ok(result)
    }

    /// Commit a transaction and release its connection.
    pub async fn commit_transaction(
        &self,
        transaction_id: &str,
        name: Option<&str>,
    ) -> Result<(), McpError> {
        let mut connections = self.connections.lock().await;
        let mut conn = connections.remove(transaction_id).ok_or_else(|| {
            McpError::Session(format!(
                "Transaction connection not found: {}",
                transaction_id
            ))
        })?;

        let commit_sql = match name {
            Some(n) => format!("COMMIT TRANSACTION [{}]", n.replace(']', "]]")),
            None => "COMMIT TRANSACTION".to_string(),
        };

        conn.simple_query(&commit_sql)
            .await
            .map_err(|e| McpError::query_error(format!("Failed to commit transaction: {}", e)))?
            .into_results()
            .await
            .map_err(|e| McpError::query_error(format!("Failed to commit transaction: {}", e)))?;

        debug!(
            "Transaction {} committed, connection released",
            transaction_id
        );
        // Connection is dropped here when it goes out of scope
        Ok(())
    }

    /// Rollback a transaction and release its connection.
    pub async fn rollback_transaction(
        &self,
        transaction_id: &str,
        name: Option<&str>,
        savepoint: Option<&str>,
    ) -> Result<bool, McpError> {
        let mut connections = self.connections.lock().await;

        // For savepoint rollback, we don't remove the connection
        if let Some(sp) = savepoint {
            let conn = connections.get_mut(transaction_id).ok_or_else(|| {
                McpError::Session(format!(
                    "Transaction connection not found: {}",
                    transaction_id
                ))
            })?;

            let rollback_sql = format!("ROLLBACK TRANSACTION [{}]", sp.replace(']', "]]"));
            conn.simple_query(&rollback_sql)
                .await
                .map_err(|e| {
                    McpError::query_error(format!("Failed to rollback to savepoint: {}", e))
                })?
                .into_results()
                .await
                .map_err(|e| {
                    McpError::query_error(format!("Failed to rollback to savepoint: {}", e))
                })?;

            debug!(
                "Transaction {} rolled back to savepoint {}",
                transaction_id, sp
            );
            return Ok(false); // Transaction still active
        }

        // Full rollback - remove and close connection
        let mut conn = connections.remove(transaction_id).ok_or_else(|| {
            McpError::Session(format!(
                "Transaction connection not found: {}",
                transaction_id
            ))
        })?;

        let rollback_sql = match name {
            Some(n) => format!("ROLLBACK TRANSACTION [{}]", n.replace(']', "]]")),
            None => "ROLLBACK TRANSACTION".to_string(),
        };

        conn.simple_query(&rollback_sql)
            .await
            .map_err(|e| McpError::query_error(format!("Failed to rollback transaction: {}", e)))?
            .into_results()
            .await
            .map_err(|e| McpError::query_error(format!("Failed to rollback transaction: {}", e)))?;

        debug!(
            "Transaction {} rolled back, connection released",
            transaction_id
        );
        // Connection is dropped here
        Ok(true) // Transaction ended
    }

    /// Check if a transaction connection exists.
    pub async fn has_connection(&self, transaction_id: &str) -> bool {
        let connections = self.connections.lock().await;
        connections.contains_key(transaction_id)
    }

    /// Get the count of active transaction connections.
    pub async fn active_count(&self) -> usize {
        let connections = self.connections.lock().await;
        connections.len()
    }

    /// Process a query stream into a QueryResult.
    async fn process_stream(
        &self,
        mut stream: tiberius::QueryStream<'_>,
        max_rows: usize,
        start: Instant,
    ) -> Result<QueryResult, McpError> {
        use futures_util::stream::TryStreamExt;

        let mut columns: Vec<ColumnInfo> = Vec::new();
        let mut rows: Vec<ResultRow> = Vec::new();
        let rows_affected: u64 = 0;
        let mut truncated = false;

        while let Some(item) = stream
            .try_next()
            .await
            .map_err(|e| McpError::query_error(e.to_string()))?
        {
            match item {
                tiberius::QueryItem::Metadata(meta) => {
                    columns = meta
                        .columns()
                        .iter()
                        .map(|col| ColumnInfo {
                            name: col.name().to_string(),
                            sql_type: TypeMapper::sql_type_name(col).to_string(),
                            nullable: true,
                        })
                        .collect();
                }
                tiberius::QueryItem::Row(row) => {
                    if rows.len() >= max_rows {
                        truncated = true;
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

        Ok(QueryResult {
            columns,
            rows,
            rows_affected,
            execution_time_ms: start.elapsed().as_millis() as u64,
            truncated,
        })
    }

    /// Clean up orphaned transaction connections.
    ///
    /// This should be called periodically or when transactions are cleaned up.
    pub async fn cleanup_orphaned(&self, valid_transaction_ids: &[String]) {
        let mut connections = self.connections.lock().await;
        let orphaned: Vec<String> = connections
            .keys()
            .filter(|id| !valid_transaction_ids.contains(id))
            .cloned()
            .collect();

        for id in orphaned {
            warn!("Cleaning up orphaned transaction connection: {}", id);
            if let Some(mut conn) = connections.remove(&id) {
                // Try to rollback before dropping - consume results for proper cleanup
                if let Ok(stream) = conn
                    .simple_query("IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION")
                    .await
                {
                    let _ = stream.into_results().await;
                }
            }
        }
    }
}
