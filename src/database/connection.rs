//! Connection pool management for SQL Server.

use super::auth::create_config;
use crate::config::DatabaseConfig;
use crate::error::ServerError;
use mssql_driver_pool::{Pool, PoolBuilder, PooledConnection};
use tracing::{debug, info};

/// Type alias for the connection pool.
pub type ConnectionPool = Pool;

/// Type alias for a pooled connection.
pub type PooledConn = PooledConnection;

/// Create a connection pool from configuration.
pub async fn create_pool(config: &DatabaseConfig) -> Result<ConnectionPool, ServerError> {
    info!(
        "Creating connection pool for {}:{} (min: {}, max: {})",
        config.host, config.port, config.pool.min_connections, config.pool.max_connections
    );

    // Create base configuration
    let client_config = create_config(config).await?;

    // Build the pool with settings from our config
    let pool = PoolBuilder::new()
        .client_config(client_config)
        .min_connections(config.pool.min_connections)
        .max_connections(config.pool.max_connections)
        .idle_timeout(config.pool.idle_timeout)
        .connection_timeout(config.pool.connection_timeout)
        .sp_reset_connection(true) // Enable connection state cleanup
        .build()
        .await
        .map_err(|e| ServerError::connection_with_source("Failed to create connection pool", e))?;

    // Test the pool by getting a connection
    {
        let _conn = pool.get().await.map_err(|e| {
            ServerError::connection(format!("Failed to establish initial connection: {}", e))
        })?;
        debug!("Initial connection test successful");
        // Connection dropped here, releasing borrow and returning to pool
    }

    info!("Connection pool created successfully");
    Ok(pool)
}

/// Get pool health status.
pub fn pool_status(pool: &ConnectionPool) -> PoolStatus {
    let status = pool.status();
    PoolStatus {
        total_connections: status.total as usize,
        available_connections: status.available as usize,
        in_use_connections: status.in_use as usize,
        max_connections: status.max as usize,
    }
}

/// Pool status information.
#[derive(Debug, Clone)]
pub struct PoolStatus {
    /// Total number of connections in the pool.
    pub total_connections: usize,
    /// Number of connections available for checkout.
    pub available_connections: usize,
    /// Number of connections currently in use.
    pub in_use_connections: usize,
    /// Maximum allowed connections.
    pub max_connections: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuthConfig, PoolConfig, RetryConfig, TdsVersionConfig, TimeoutsConfig};

    fn test_config() -> DatabaseConfig {
        DatabaseConfig {
            host: "localhost".to_string(),
            port: 1433,
            instance: None,
            database: Some("master".to_string()),
            auth: AuthConfig::SqlServer {
                username: "sa".to_string(),
                password: "test".to_string(),
            },
            pool: PoolConfig::default(),
            timeouts: TimeoutsConfig::default(),
            encrypt: false,
            trust_server_certificate: true,
            application_name: "test".to_string(),
            mars: false,
            retry: RetryConfig::default(),
            tds_version: TdsVersionConfig::default(),
        }
    }

    #[test]
    fn test_pool_config() {
        let config = test_config();
        // Just verify config creation doesn't panic
        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 1433);
    }
}
