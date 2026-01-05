//! Transaction connection management.
//!
//! This module provides dedicated connections for database transactions.
//! Unlike pooled connections, transaction connections are held for the
//! entire lifetime of a transaction to maintain transaction state.

use super::auth::{create_connection, truncate_for_log, RawConnection};
use crate::config::DatabaseConfig;
use crate::database::query::{ColumnInfo, QueryResult, ResultRow};
use crate::database::types::TypeMapper;
use crate::error::ServerError;
use futures_util::TryStreamExt;
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
    async fn create_txn_connection(&self) -> Result<RawConnection, ServerError> {
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
    ) -> Result<(), ServerError> {
        // Create a dedicated connection for this transaction
        let mut conn = self.create_txn_connection().await?;

        // Set isolation level and begin transaction
        let set_isolation = format!("SET TRANSACTION ISOLATION LEVEL {}", isolation_level);
        let begin_tx = match name {
            Some(n) => format!("BEGIN TRANSACTION [{}]", n.replace(']', "]]")),
            None => "BEGIN TRANSACTION".to_string(),
        };

        // Execute as separate statements on the same connection
        conn.execute(&set_isolation, &[])
            .await
            .map_err(|e| ServerError::query_error(format!("Failed to set isolation level: {}", e)))?;

        conn.execute(&begin_tx, &[])
            .await
            .map_err(|e| ServerError::query_error(format!("Failed to begin transaction: {}", e)))?;

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
    ) -> Result<QueryResult, ServerError> {
        let start = Instant::now();

        let mut connections = self.connections.lock().await;
        let conn = connections.get_mut(transaction_id).ok_or_else(|| {
            ServerError::Session(format!(
                "Transaction connection not found: {}",
                transaction_id
            ))
        })?;

        debug!(
            "Executing in transaction {}: {}",
            transaction_id,
            truncate_for_log(query, 100)
        );

        // Execute query and collect stream
        let stream = conn
            .query(query, &[])
            .await
            .map_err(|e| ServerError::query_error(format!("Query execution failed: {}", e)))?;

        let rows: Vec<mssql_client::Row> = stream.try_collect().await.map_err(|e| {
            ServerError::query_error(format!("Failed to collect query results: {}", e))
        })?;

        // Process results
        let result = self.process_rows(rows, self.max_rows, start)?;

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
    ) -> Result<(), ServerError> {
        let mut connections = self.connections.lock().await;
        let mut conn = connections.remove(transaction_id).ok_or_else(|| {
            ServerError::Session(format!(
                "Transaction connection not found: {}",
                transaction_id
            ))
        })?;

        let commit_sql = match name {
            Some(n) => format!("COMMIT TRANSACTION [{}]", n.replace(']', "]]")),
            None => "COMMIT TRANSACTION".to_string(),
        };

        conn.execute(&commit_sql, &[])
            .await
            .map_err(|e| ServerError::query_error(format!("Failed to commit transaction: {}", e)))?;

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
    ) -> Result<bool, ServerError> {
        let mut connections = self.connections.lock().await;

        // For savepoint rollback, we don't remove the connection
        if let Some(sp) = savepoint {
            let conn = connections.get_mut(transaction_id).ok_or_else(|| {
                ServerError::Session(format!(
                    "Transaction connection not found: {}",
                    transaction_id
                ))
            })?;

            let rollback_sql = format!("ROLLBACK TRANSACTION [{}]", sp.replace(']', "]]"));
            conn.execute(&rollback_sql, &[]).await.map_err(|e| {
                ServerError::query_error(format!("Failed to rollback to savepoint: {}", e))
            })?;

            debug!(
                "Transaction {} rolled back to savepoint {}",
                transaction_id, sp
            );
            return Ok(false); // Transaction still active
        }

        // Full rollback - remove and close connection
        let mut conn = connections.remove(transaction_id).ok_or_else(|| {
            ServerError::Session(format!(
                "Transaction connection not found: {}",
                transaction_id
            ))
        })?;

        let rollback_sql = match name {
            Some(n) => format!("ROLLBACK TRANSACTION [{}]", n.replace(']', "]]")),
            None => "ROLLBACK TRANSACTION".to_string(),
        };

        conn.execute(&rollback_sql, &[])
            .await
            .map_err(|e| ServerError::query_error(format!("Failed to rollback transaction: {}", e)))?;

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
                // Try to rollback before dropping
                let _ = conn
                    .execute("IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION", &[])
                    .await;
            }
        }
    }
}
