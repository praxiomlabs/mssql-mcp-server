//! Configuration management for the MSSQL MCP Server.
//!
//! Configuration is loaded from environment variables following the 12-factor app pattern.

use crate::constants::{
    DEFAULT_CACHE_MAX_ENTRIES, DEFAULT_CACHE_MAX_SIZE_MB, DEFAULT_CACHE_TTL,
    DEFAULT_CACHE_TTL_SECS, DEFAULT_CLEANUP_INTERVAL, DEFAULT_CONNECTION_TIMEOUT,
    DEFAULT_CONNECTION_TIMEOUT_SECS, DEFAULT_MAX_CONNECTIONS, DEFAULT_MAX_RESULT_ROWS,
    DEFAULT_MIN_CONNECTIONS, DEFAULT_QUERY_TIMEOUT, DEFAULT_QUERY_TIMEOUT_SECS,
};
use crate::error::McpError;
use crate::security::ValidationMode;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Server configuration loaded from environment variables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Database connection configuration
    pub database: DatabaseConfig,

    /// Security configuration
    pub security: SecurityConfig,

    /// Query execution configuration
    pub query: QueryConfig,

    /// Session management configuration
    pub session: SessionConfig,
}

/// Database connection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// SQL Server hostname or IP address
    pub host: String,

    /// SQL Server port (default: 1433)
    pub port: u16,

    /// Database name (optional, enables database mode vs server mode)
    pub database: Option<String>,

    /// Authentication configuration
    pub auth: AuthConfig,

    /// Connection pool configuration
    pub pool: PoolConfig,

    /// Enable TLS encryption
    pub encrypt: bool,

    /// Trust server certificate (for self-signed certs)
    pub trust_server_certificate: bool,

    /// Application name sent to SQL Server
    pub application_name: String,
}

/// Authentication configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthConfig {
    /// SQL Server authentication (username/password)
    SqlServer { username: String, password: String },

    /// Windows authentication (Integrated Security)
    #[cfg(windows)]
    Windows,

    /// Azure Active Directory authentication
    AzureAd {
        /// Client ID for Azure AD application
        client_id: String,
        /// Client secret or certificate
        client_secret: String,
        /// Azure AD tenant ID
        tenant_id: String,
    },
}

/// Connection pool configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolConfig {
    /// Minimum number of connections in the pool
    pub min_connections: u32,

    /// Maximum number of connections in the pool
    pub max_connections: u32,

    /// Connection timeout
    pub connection_timeout: Duration,

    /// Idle connection timeout
    pub idle_timeout: Duration,
}

/// Security configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Query validation mode
    pub validation_mode: ValidationMode,

    /// Enable SQL injection detection
    pub injection_detection: bool,

    /// Maximum query length (bytes)
    pub max_query_length: usize,

    /// Maximum result rows per query
    pub max_result_rows: usize,
}

/// Query execution configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryConfig {
    /// Default query timeout
    pub default_timeout: Duration,

    /// Maximum query timeout
    pub max_timeout: Duration,

    /// Enable query result caching
    pub enable_caching: bool,

    /// Cache TTL for query results
    pub cache_ttl: Duration,

    /// Maximum cache size in MB
    pub cache_max_size_mb: usize,

    /// Maximum number of cached entries
    pub cache_max_entries: usize,
}

/// Session management configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// Maximum concurrent async sessions
    pub max_sessions: usize,

    /// Session cleanup interval
    pub cleanup_interval: Duration,

    /// Session result retention time
    pub result_retention: Duration,
}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// # Environment Variables
    ///
    /// ## Required
    /// - `MSSQL_HOST`: SQL Server hostname
    /// - `MSSQL_USER`: SQL Server username (for SQL auth)
    /// - `MSSQL_PASSWORD`: SQL Server password (for SQL auth)
    ///
    /// ## Optional
    /// - `MSSQL_PORT`: Port number (default: 1433)
    /// - `MSSQL_DATABASE`: Database name (omit for server mode)
    /// - `MSSQL_ENCRYPT`: Enable TLS (default: true)
    /// - `MSSQL_TRUST_CERT`: Trust server certificate (default: false)
    /// - `MSSQL_POOL_MIN`: Minimum pool connections (default: 1)
    /// - `MSSQL_POOL_MAX`: Maximum pool connections (default: 10)
    /// - `MSSQL_CONNECT_TIMEOUT`: Connection timeout in seconds (default: 30)
    /// - `MSSQL_QUERY_TIMEOUT`: Default query timeout in seconds (default: 30)
    /// - `MSSQL_VALIDATION_MODE`: Query validation mode (readonly, standard, unrestricted)
    /// - `MSSQL_MAX_ROWS`: Maximum result rows (default: 10000)
    pub fn from_env() -> Result<Self, McpError> {
        // Required: Host
        let host = std::env::var("MSSQL_HOST")
            .map_err(|_| McpError::config("MSSQL_HOST environment variable is required"))?;

        // Determine authentication type
        let auth_type = std::env::var("MSSQL_AUTH_TYPE")
            .ok()
            .map(|s| s.to_lowercase());

        let auth = match auth_type.as_deref() {
            Some("azuread") | Some("azure") | Some("aad") => {
                // Azure AD Authentication
                let client_id = std::env::var("MSSQL_AZURE_CLIENT_ID").map_err(|_| {
                    McpError::config(
                        "MSSQL_AZURE_CLIENT_ID is required for Azure AD authentication",
                    )
                })?;
                let client_secret = std::env::var("MSSQL_AZURE_CLIENT_SECRET").map_err(|_| {
                    McpError::config(
                        "MSSQL_AZURE_CLIENT_SECRET is required for Azure AD authentication",
                    )
                })?;
                let tenant_id = std::env::var("MSSQL_AZURE_TENANT_ID").map_err(|_| {
                    McpError::config(
                        "MSSQL_AZURE_TENANT_ID is required for Azure AD authentication",
                    )
                })?;

                AuthConfig::AzureAd {
                    client_id,
                    client_secret,
                    tenant_id,
                }
            }
            _ => {
                // SQL Server Authentication (default)
                let username = std::env::var("MSSQL_USER").ok();
                let password = std::env::var("MSSQL_PASSWORD").ok();

                match (username, password) {
                    (Some(u), Some(p)) => AuthConfig::SqlServer {
                        username: u,
                        password: p,
                    },
                    (Some(_), None) => {
                        return Err(McpError::config(
                            "MSSQL_PASSWORD is required when MSSQL_USER is set",
                        ))
                    }
                    (None, Some(_)) => {
                        return Err(McpError::config(
                            "MSSQL_USER is required when MSSQL_PASSWORD is set",
                        ))
                    }
                    (None, None) => {
                        return Err(McpError::config(
                            "Authentication required: set MSSQL_USER and MSSQL_PASSWORD, or use MSSQL_AUTH_TYPE=azuread",
                        ))
                    }
                }
            }
        };

        // Optional: Port
        let port = std::env::var("MSSQL_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(1433);

        // Optional: Database (None = server mode)
        let database = std::env::var("MSSQL_DATABASE").ok();

        // Optional: Encryption settings
        let encrypt = std::env::var("MSSQL_ENCRYPT")
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(true);

        let trust_server_certificate = std::env::var("MSSQL_TRUST_CERT")
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        // Optional: Pool settings
        let min_connections = std::env::var("MSSQL_POOL_MIN")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_MIN_CONNECTIONS);

        let max_connections = std::env::var("MSSQL_POOL_MAX")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_MAX_CONNECTIONS);

        let connection_timeout_secs = std::env::var("MSSQL_CONNECT_TIMEOUT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_CONNECTION_TIMEOUT_SECS);

        let idle_timeout_secs: u64 = std::env::var("MSSQL_IDLE_TIMEOUT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(300);

        // Optional: Query settings
        let default_timeout_secs = std::env::var("MSSQL_QUERY_TIMEOUT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_QUERY_TIMEOUT_SECS);

        let max_timeout_secs = std::env::var("MSSQL_MAX_QUERY_TIMEOUT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(300);

        // Optional: Security settings
        let validation_mode = std::env::var("MSSQL_VALIDATION_MODE")
            .ok()
            .and_then(|m| match m.to_lowercase().as_str() {
                "readonly" | "read_only" | "read-only" => Some(ValidationMode::ReadOnly),
                "standard" => Some(ValidationMode::Standard),
                "unrestricted" => Some(ValidationMode::Unrestricted),
                _ => None,
            })
            .unwrap_or(ValidationMode::Standard);

        let max_query_length = std::env::var("MSSQL_MAX_QUERY_LENGTH")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(1_000_000); // 1MB default

        let max_result_rows = std::env::var("MSSQL_MAX_ROWS")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_MAX_RESULT_ROWS);

        let injection_detection = std::env::var("MSSQL_INJECTION_DETECTION")
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        // Optional: Session settings
        let max_sessions = std::env::var("MSSQL_MAX_SESSIONS")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(10);

        // Optional: Cache settings
        let enable_caching = std::env::var("MSSQL_ENABLE_CACHE")
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        let cache_ttl_secs = std::env::var("MSSQL_CACHE_TTL")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_CACHE_TTL_SECS);

        let cache_max_size_mb = std::env::var("MSSQL_CACHE_SIZE_MB")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_CACHE_MAX_SIZE_MB);

        let cache_max_entries = std::env::var("MSSQL_CACHE_MAX_ENTRIES")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_CACHE_MAX_ENTRIES);

        Ok(Config {
            database: DatabaseConfig {
                host,
                port,
                database,
                auth,
                pool: PoolConfig {
                    min_connections,
                    max_connections,
                    connection_timeout: Duration::from_secs(connection_timeout_secs),
                    idle_timeout: Duration::from_secs(idle_timeout_secs),
                },
                encrypt,
                trust_server_certificate,
                application_name: "mssql-mcp-server".to_string(),
            },
            security: SecurityConfig {
                validation_mode,
                injection_detection,
                max_query_length,
                max_result_rows,
            },
            query: QueryConfig {
                default_timeout: Duration::from_secs(default_timeout_secs),
                max_timeout: Duration::from_secs(max_timeout_secs),
                enable_caching,
                cache_ttl: Duration::from_secs(cache_ttl_secs),
                cache_max_size_mb,
                cache_max_entries,
            },
            session: SessionConfig {
                max_sessions,
                cleanup_interval: DEFAULT_CLEANUP_INTERVAL,
                result_retention: Duration::from_secs(3600),
            },
        })
    }

    /// Check if running in database mode (specific database) vs server mode (instance-wide).
    pub fn is_database_mode(&self) -> bool {
        self.database.database.is_some()
    }

    /// Get the current database name, if in database mode.
    pub fn current_database(&self) -> Option<&str> {
        self.database.database.as_deref()
    }
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_connections: DEFAULT_MIN_CONNECTIONS,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            connection_timeout: DEFAULT_CONNECTION_TIMEOUT,
            idle_timeout: Duration::from_secs(300),
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            validation_mode: ValidationMode::Standard,
            injection_detection: true,
            max_query_length: 1_000_000,
            max_result_rows: DEFAULT_MAX_RESULT_ROWS,
        }
    }
}

impl Default for QueryConfig {
    fn default() -> Self {
        Self {
            default_timeout: DEFAULT_QUERY_TIMEOUT,
            max_timeout: Duration::from_secs(300),
            enable_caching: false,
            cache_ttl: DEFAULT_CACHE_TTL,
            cache_max_size_mb: DEFAULT_CACHE_MAX_SIZE_MB,
            cache_max_entries: DEFAULT_CACHE_MAX_ENTRIES,
        }
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_sessions: 10,
            cleanup_interval: DEFAULT_CLEANUP_INTERVAL,
            result_retention: Duration::from_secs(3600),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_config_defaults() {
        let config = PoolConfig::default();
        assert_eq!(config.min_connections, 1);
        assert_eq!(config.max_connections, 10);
    }
}
