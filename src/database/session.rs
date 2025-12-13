//! Session-pinned connection management.
//!
//! This module provides dedicated connections for stateful database sessions.
//! Unlike pooled connections, session connections are held for the entire
//! lifetime of a session, allowing temp tables, session variables, and
//! SET options to persist across queries.

use crate::config::DatabaseConfig;
use crate::database::query::{ColumnInfo, QueryResult, ResultRow};
use crate::database::types::TypeMapper;
use crate::error::McpError;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tiberius::{AuthMethod, Client, Config, EncryptionLevel};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};
use tracing::{debug, warn};

/// Type alias for a raw tiberius connection.
pub type RawConnection = Client<Compat<TcpStream>>;

/// Metadata about a pinned session.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Session ID.
    pub id: String,
    /// When the session was created.
    pub created_at: Instant,
    /// Last activity time.
    pub last_activity: Instant,
    /// Number of queries executed in this session.
    pub query_count: u64,
}

/// Manager for session-pinned connections.
///
/// This struct manages connections that are held for the duration of a session.
/// Each session gets its own dedicated connection that is not returned to the
/// pool until the session is explicitly ended.
///
/// Use cases:
/// - Temp tables (#tables) that need to persist across queries
/// - Session variables that need to be set once and used multiple times
/// - SET options that should persist for multiple queries
pub struct SessionManager {
    /// Database configuration for creating new connections.
    db_config: Arc<DatabaseConfig>,

    /// Active session connections keyed by session ID.
    connections: Mutex<HashMap<String, (RawConnection, SessionInfo)>>,

    /// Maximum rows to return from queries.
    max_rows: usize,

    /// Session timeout duration.
    session_timeout: Duration,
}

impl SessionManager {
    /// Create a new session manager.
    pub fn new(db_config: Arc<DatabaseConfig>, max_rows: usize, session_timeout: Duration) -> Self {
        Self {
            db_config,
            connections: Mutex::new(HashMap::new()),
            max_rows,
            session_timeout,
        }
    }

    /// Create a new raw connection using the database configuration.
    async fn create_connection(&self) -> Result<RawConnection, McpError> {
        let mut config = Config::new();

        // Set host and port
        config.host(&self.db_config.host);
        config.port(self.db_config.port);

        // Set database if specified
        if let Some(ref database) = self.db_config.database {
            config.database(database);
        }

        // Configure authentication
        match &self.db_config.auth {
            crate::config::AuthConfig::SqlServer { username, password } => {
                config.authentication(AuthMethod::sql_server(username, password));
            }
            #[cfg(windows)]
            crate::config::AuthConfig::Windows => {
                config.authentication(AuthMethod::Integrated);
            }
            crate::config::AuthConfig::AzureAd { .. } => {
                return Err(McpError::config(
                    "Azure AD authentication is not yet supported",
                ));
            }
        }

        // Configure encryption
        if self.db_config.encrypt {
            config.encryption(EncryptionLevel::Required);
        } else {
            config.encryption(EncryptionLevel::Off);
        }

        // Trust server certificate if requested
        if self.db_config.trust_server_certificate {
            config.trust_cert();
        }

        // Set application name with session indicator
        config.application_name(&format!("{}-session", self.db_config.application_name));

        let address = format!("{}:{}", self.db_config.host, self.db_config.port);
        debug!("Creating session connection to {}", address);

        let tcp = TcpStream::connect(&address)
            .await
            .map_err(|e| McpError::connection(format!("Failed to connect: {}", e)))?;

        tcp.set_nodelay(true)
            .map_err(|e| McpError::connection(format!("Failed to set TCP_NODELAY: {}", e)))?;

        let client = Client::connect(config, tcp.compat_write())
            .await
            .map_err(|e| McpError::connection(format!("Failed to connect to SQL Server: {}", e)))?;

        debug!("Session connection established");
        Ok(client)
    }

    /// Begin a new pinned session.
    ///
    /// Returns the session ID if successful.
    pub async fn begin_session(&self, session_id: &str) -> Result<SessionInfo, McpError> {
        // Check if session already exists
        {
            let connections = self.connections.lock().await;
            if connections.contains_key(session_id) {
                return Err(McpError::Session(format!(
                    "Session already exists: {}",
                    session_id
                )));
            }
        }

        // Create a dedicated connection for this session
        let conn = self.create_connection().await?;

        let info = SessionInfo {
            id: session_id.to_string(),
            created_at: Instant::now(),
            last_activity: Instant::now(),
            query_count: 0,
        };

        // Store the connection
        let mut connections = self.connections.lock().await;
        connections.insert(session_id.to_string(), (conn, info.clone()));

        debug!(
            "Session {} started with dedicated connection",
            session_id
        );
        Ok(info)
    }

    /// Execute a query within an existing session.
    pub async fn execute_in_session(
        &self,
        session_id: &str,
        query: &str,
    ) -> Result<QueryResult, McpError> {
        let start = Instant::now();

        let mut connections = self.connections.lock().await;
        let (conn, info) = connections.get_mut(session_id).ok_or_else(|| {
            McpError::Session(format!("Session not found: {}", session_id))
        })?;

        // Update last activity and query count
        info.last_activity = Instant::now();
        info.query_count += 1;

        debug!(
            "Executing in session {}: {}",
            session_id,
            truncate_for_log(query, 100)
        );

        // Execute query using simple_query for full flexibility (supports all SQL)
        let results = conn
            .simple_query(query)
            .await
            .map_err(|e| McpError::query_error(format!("Query execution failed: {}", e)))?
            .into_results()
            .await
            .map_err(|e| McpError::query_error(format!("Failed to get results: {}", e)))?;

        // Convert results
        let result = self.convert_results(results, start);

        debug!(
            "Session query completed: {} rows in {} ms",
            result.rows.len(),
            result.execution_time_ms
        );

        Ok(result)
    }

    /// End a session and release its connection.
    pub async fn end_session(&self, session_id: &str) -> Result<SessionInfo, McpError> {
        let mut connections = self.connections.lock().await;
        let (mut conn, info) = connections.remove(session_id).ok_or_else(|| {
            McpError::Session(format!("Session not found: {}", session_id))
        })?;

        // Clean up any temp tables or transactions before closing
        // This is best-effort - we don't fail if cleanup fails
        if let Ok(stream) = conn
            .simple_query("IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION")
            .await
        {
            let _ = stream.into_results().await;
        }

        debug!(
            "Session {} ended after {} queries, connection released",
            session_id, info.query_count
        );
        // Connection is dropped here when it goes out of scope
        Ok(info)
    }

    /// Get information about a session.
    pub async fn get_session_info(&self, session_id: &str) -> Option<SessionInfo> {
        let connections = self.connections.lock().await;
        connections.get(session_id).map(|(_, info)| info.clone())
    }

    /// List all active sessions.
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        let connections = self.connections.lock().await;
        connections
            .values()
            .map(|(_, info)| info.clone())
            .collect()
    }

    /// Check if a session exists.
    pub async fn has_session(&self, session_id: &str) -> bool {
        let connections = self.connections.lock().await;
        connections.contains_key(session_id)
    }

    /// Get the count of active sessions.
    pub async fn active_count(&self) -> usize {
        let connections = self.connections.lock().await;
        connections.len()
    }

    /// Clean up expired sessions.
    ///
    /// This should be called periodically to release idle sessions.
    pub async fn cleanup_expired(&self) -> Vec<String> {
        let mut connections = self.connections.lock().await;
        let now = Instant::now();

        let expired: Vec<String> = connections
            .iter()
            .filter(|(_, (_, info))| now.duration_since(info.last_activity) > self.session_timeout)
            .map(|(id, _)| id.clone())
            .collect();

        let mut cleaned = Vec::new();
        for id in expired {
            if let Some((mut conn, info)) = connections.remove(&id) {
                warn!(
                    "Cleaning up expired session {} (idle for {:?})",
                    id,
                    now.duration_since(info.last_activity)
                );
                // Try to clean up before dropping
                if let Ok(stream) = conn
                    .simple_query("IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION")
                    .await
                {
                    let _ = stream.into_results().await;
                }
                cleaned.push(id);
            }
        }

        cleaned
    }

    /// Convert simple_query results to QueryResult.
    fn convert_results(&self, results: Vec<Vec<tiberius::Row>>, start: Instant) -> QueryResult {
        let mut columns: Vec<ColumnInfo> = Vec::new();
        let mut rows: Vec<ResultRow> = Vec::new();
        let mut truncated = false;

        for result_set in results {
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

                // Check row limit
                if rows.len() >= self.max_rows {
                    truncated = true;
                    continue;
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
            truncated,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_for_log() {
        assert_eq!(truncate_for_log("short", 10), "short");
        assert_eq!(
            truncate_for_log("this is a long string", 10),
            "this is a ..."
        );
    }
}
