//! Connection pool management for SQL Server.

use crate::config::{AuthConfig, DatabaseConfig};
use crate::error::McpError;
use bb8::{Pool, PooledConnection};
use std::future::Future;
use std::sync::Arc;
use tiberius::{AuthMethod, Client, Config, EncryptionLevel};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};
use tracing::{debug, info};

/// Type alias for the connection pool.
pub type ConnectionPool = Pool<ConnectionManager>;

/// Type alias for a pooled connection.
pub type PooledConn<'a> = PooledConnection<'a, ConnectionManager>;

/// Connection manager for bb8 pool.
#[derive(Clone)]
pub struct ConnectionManager {
    config: Arc<Config>,
    host: String,
    port: u16,
}

impl ConnectionManager {
    /// Create a new connection manager from database configuration.
    pub fn new(db_config: &DatabaseConfig) -> Result<Self, McpError> {
        let mut config = Config::new();

        // Set host and port
        config.host(&db_config.host);
        config.port(db_config.port);

        // Set database if specified
        if let Some(ref database) = db_config.database {
            config.database(database);
        }

        // Configure authentication
        match &db_config.auth {
            AuthConfig::SqlServer { username, password } => {
                config.authentication(AuthMethod::sql_server(username, password));
            }
            #[cfg(windows)]
            AuthConfig::Windows => {
                config.authentication(AuthMethod::Integrated);
            }
            AuthConfig::AzureAd { .. } => {
                // Azure AD auth requires additional setup
                return Err(McpError::config(
                    "Azure AD authentication is not yet supported",
                ));
            }
        }

        // Configure encryption
        if db_config.encrypt {
            config.encryption(EncryptionLevel::Required);
        } else {
            config.encryption(EncryptionLevel::Off);
        }

        // Trust server certificate if requested
        if db_config.trust_server_certificate {
            config.trust_cert();
        }

        // Set application name
        config.application_name(&db_config.application_name);

        Ok(Self {
            config: Arc::new(config),
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
        let config = self.config.clone();
        let address = self.address();

        async move {
            debug!("Creating new database connection to {}", address);

            let tcp = TcpStream::connect(&address).await?;
            tcp.set_nodelay(true)?;

            let client = Client::connect((*config).clone(), tcp.compat_write()).await?;

            debug!("Database connection established");
            Ok(client)
        }
    }

    fn is_valid(&self, conn: &mut Self::Connection) -> impl Future<Output = Result<(), Self::Error>> + Send {
        // We need to take a mutable reference and return a future
        // Since we can't move conn into the async block, use a simple approach
        let query_future = conn.simple_query("SELECT 1");
        async move {
            query_future.await?;
            Ok(())
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
        let _conn = pool
            .get()
            .await
            .map_err(|e| McpError::connection(format!("Failed to establish initial connection: {}", e)))?;
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
