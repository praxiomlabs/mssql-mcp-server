//! Connection pool management for SQL Server.

use super::auth::{configure_auth, create_base_config};
use crate::config::{AuthConfig, DatabaseConfig};
use crate::error::McpError;
use bb8::{Pool, PooledConnection};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tiberius::{Client, Config};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};
use tracing::{debug, info, warn};

/// Type alias for the connection pool.
pub type ConnectionPool = Pool<ConnectionManager>;

/// Type alias for a pooled connection.
pub type PooledConn<'a> = PooledConnection<'a, ConnectionManager>;

/// Connection manager for bb8 pool.
#[derive(Clone)]
pub struct ConnectionManager {
    /// Base configuration (without auth - auth is configured per connection for Azure AD)
    base_config: Arc<Config>,
    /// Auth configuration for token refresh
    auth_config: Arc<AuthConfig>,
    host: String,
    port: u16,
}

impl ConnectionManager {
    /// Create a new connection manager from database configuration.
    ///
    /// Note: For Azure AD authentication, the token is acquired fresh for each
    /// new connection to handle token expiration properly.
    pub fn new(db_config: &DatabaseConfig) -> Result<Self, McpError> {
        let base_config = create_base_config(db_config);

        Ok(Self {
            base_config: Arc::new(base_config),
            auth_config: Arc::new(db_config.auth.clone()),
            host: db_config.host.clone(),
            port: db_config.port,
        })
    }

    /// Get the connection address.
    fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl bb8::ManageConnection for ConnectionManager {
    type Connection = Client<Compat<TcpStream>>;
    type Error = tiberius::error::Error;

    fn connect(&self) -> impl Future<Output = Result<Self::Connection, Self::Error>> + Send {
        let base_config = self.base_config.clone();
        let auth_config = self.auth_config.clone();
        let address = self.address();

        async move {
            debug!("Creating new database connection to {}", address);

            // Configure authentication (acquires fresh Azure AD token if needed)
            let config = configure_auth(&base_config, &auth_config)
                .await
                .map_err(|e| {
                    // Convert McpError to tiberius error for bb8 compatibility
                    tiberius::error::Error::Protocol(e.to_string().into())
                })?;

            let tcp = TcpStream::connect(&address).await?;
            tcp.set_nodelay(true)?;

            let client = Client::connect(config, tcp.compat_write()).await?;

            debug!("Database connection established");
            Ok(client)
        }
    }

    fn is_valid(
        &self,
        conn: &mut Self::Connection,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        // Validate connection with a lightweight query
        // Use a 5 second timeout to prevent hanging on dead connections
        const VALIDATION_TIMEOUT: Duration = Duration::from_secs(5);

        let query_future = conn.simple_query("SELECT 1");
        async move {
            match timeout(VALIDATION_TIMEOUT, async {
                // Execute validation query and consume results
                let stream = query_future.await?;
                let _ = stream.into_results().await?;
                Ok::<_, tiberius::error::Error>(())
            })
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    warn!(
                        "Connection validation timed out after {:?}",
                        VALIDATION_TIMEOUT
                    );
                    Err(tiberius::error::Error::Protocol(
                        "Connection validation timeout".into(),
                    ))
                }
            }
        }
    }

    fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
        // Tiberius doesn't provide a way to check if connection is broken
        // without executing a query, so we return false here and rely on
        // is_valid for actual validation
        false
    }
}

/// Create a connection pool from configuration.
pub async fn create_pool(config: &DatabaseConfig) -> Result<ConnectionPool, McpError> {
    info!(
        "Creating connection pool for {}:{} (min: {}, max: {})",
        config.host, config.port, config.pool.min_connections, config.pool.max_connections
    );

    let manager = ConnectionManager::new(config)?;

    let pool = Pool::builder()
        .min_idle(Some(config.pool.min_connections))
        .max_size(config.pool.max_connections)
        .connection_timeout(config.pool.connection_timeout)
        .idle_timeout(Some(config.pool.idle_timeout))
        .build(manager)
        .await
        .map_err(|e| McpError::connection_with_source("Failed to create connection pool", e))?;

    // Test the pool by getting a connection
    {
        let _conn = pool.get().await.map_err(|e| {
            McpError::connection(format!("Failed to establish initial connection: {}", e))
        })?;
        // Connection dropped here, releasing borrow
    }

    info!("Connection pool created successfully");
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;

    fn test_config() -> DatabaseConfig {
        DatabaseConfig {
            host: "localhost".to_string(),
            port: 1433,
            database: Some("master".to_string()),
            auth: AuthConfig::SqlServer {
                username: "sa".to_string(),
                password: "test".to_string(),
            },
            pool: PoolConfig::default(),
            encrypt: false,
            trust_server_certificate: true,
            application_name: "test".to_string(),
        }
    }

    #[test]
    fn test_connection_manager_creation() {
        let config = test_config();
        let manager = ConnectionManager::new(&config);
        assert!(manager.is_ok());
    }

    #[test]
    fn test_connection_address() {
        let config = test_config();
        let manager = ConnectionManager::new(&config).unwrap();
        assert_eq!(manager.address(), "localhost:1433");
    }
}
