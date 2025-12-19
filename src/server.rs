//! MCP server struct definition and initialization.

use crate::config::Config;
use crate::database::{
    create_pool, ConnectionPool, MetadataQueries, QueryExecutor, SessionManager, TransactionManager,
};
use crate::error::McpError;
use crate::security::QueryValidator;
use crate::state::{new_shared_state, SharedState};
use crate::telemetry::{new_shared_metrics, SharedMetrics};
use rmcp::handler::server::router::tool::ToolRouter;
use std::sync::Arc;

/// The MSSQL MCP Server instance.
///
/// This struct is cloned for each request, but the inner state
/// is shared via Arc. The server provides:
///
/// - **Resources**: Database metadata (tables, views, procedures)
/// - **Tools**: Query execution and stored procedure invocation
/// - **Prompts**: AI-assisted query templates
#[derive(Clone)]
pub struct MssqlMcpServer {
    /// Thread-safe session state for async queries.
    pub(crate) state: SharedState,

    /// Database connection pool.
    pub(crate) pool: ConnectionPool,

    /// Configuration.
    pub(crate) config: Arc<Config>,

    /// Query executor.
    pub(crate) executor: Arc<QueryExecutor>,

    /// Metadata query builder.
    pub(crate) metadata: Arc<MetadataQueries>,

    /// Query validator for security.
    pub(crate) validator: Arc<QueryValidator>,

    /// Tool router for dispatching tool calls.
    pub(crate) tool_router: ToolRouter<Self>,

    /// Server metrics for telemetry.
    pub(crate) metrics: SharedMetrics,

    /// Transaction manager for dedicated transaction connections.
    pub(crate) transaction_manager: Arc<TransactionManager>,

    /// Session manager for pinned connections (temp tables, session state).
    pub(crate) session_manager: Arc<SessionManager>,
}

impl MssqlMcpServer {
    /// Create a new server instance with the given configuration.
    ///
    /// This performs async initialization including:
    /// - Creating the connection pool
    /// - Validating the database connection
    /// - Setting up the tool router
    pub async fn new(config: Config) -> Result<Self, McpError> {
        // Create connection pool
        let pool = create_pool(&config.database).await?;

        // Create shared state
        let state = new_shared_state();

        // Mark as initialized
        {
            let mut s = state.write().await;
            s.mark_initialized();
            s.set_default_timeout(config.query.default_timeout.as_secs());
        }

        // Create query executor
        let executor = Arc::new(QueryExecutor::new(
            pool.clone(),
            config.security.max_result_rows,
        ));

        // Create metadata queries
        let metadata = Arc::new(MetadataQueries::new(
            pool.clone(),
            config.security.max_result_rows,
        ));

        // Create query validator
        let validator = Arc::new(QueryValidator::new(
            config.security.validation_mode,
            config.security.max_query_length,
        ));

        // Create tool router (will be implemented in tools module)
        let tool_router = crate::tools::create_tool_router();

        // Create metrics collector
        let metrics = new_shared_metrics();

        // Create transaction manager with database config
        let db_config = Arc::new(config.database.clone());
        let transaction_manager = Arc::new(TransactionManager::new(
            db_config.clone(),
            config.security.max_result_rows,
        ));

        // Create session manager for pinned connections
        let session_manager = Arc::new(SessionManager::new(
            db_config,
            config.security.max_result_rows,
            config.session.result_retention, // Use result retention as session timeout
        ));

        Ok(Self {
            state,
            pool,
            config: Arc::new(config),
            executor,
            metadata,
            validator,
            tool_router,
            metrics,
            transaction_manager,
            session_manager,
        })
    }

    /// Create a server from environment variables.
    ///
    /// This is the standard way to create a server for production use.
    pub async fn from_env() -> Result<Self, McpError> {
        let config = Config::from_env()?;
        Self::new(config).await
    }

    /// Get a reference to the configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &ConnectionPool {
        &self.pool
    }

    /// Get a reference to the query executor.
    pub fn executor(&self) -> &QueryExecutor {
        &self.executor
    }

    /// Get a reference to the metadata queries.
    pub fn metadata(&self) -> &MetadataQueries {
        &self.metadata
    }

    /// Get a reference to the query validator.
    pub fn validator(&self) -> &QueryValidator {
        &self.validator
    }

    /// Get a reference to the session state.
    pub fn state(&self) -> &SharedState {
        &self.state
    }

    /// Get a reference to the metrics collector.
    pub fn metrics(&self) -> &SharedMetrics {
        &self.metrics
    }

    /// Get a reference to the transaction manager.
    pub fn transaction_manager(&self) -> &TransactionManager {
        &self.transaction_manager
    }

    /// Get a reference to the session manager.
    pub fn session_manager(&self) -> &SessionManager {
        &self.session_manager
    }

    /// Check if the server is in database mode (connected to specific database).
    pub fn is_database_mode(&self) -> bool {
        self.config.is_database_mode()
    }

    /// Get the current database name if in database mode.
    pub fn current_database(&self) -> Option<&str> {
        self.config.current_database()
    }

    /// Validate a query using the configured security settings.
    pub fn validate_query(&self, query: &str) -> Result<(), McpError> {
        // Validate using security module (it also checks query length)
        let result = self.validator.validate(query)?;

        // ValidationResult.valid should be true if no error was returned
        if !result.valid {
            return Err(McpError::validation(
                result
                    .message
                    .unwrap_or_else(|| "Query validation failed".to_string()),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AuthConfig, DatabaseConfig, PoolConfig, QueryConfig, SecurityConfig, SessionConfig,
    };
    use crate::security::ValidationMode;
    use std::time::Duration;

    fn test_config() -> Config {
        Config {
            database: DatabaseConfig {
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
            },
            security: SecurityConfig {
                validation_mode: ValidationMode::Standard,
                injection_detection: true,
                max_query_length: 100_000,
                max_result_rows: 1000,
            },
            query: QueryConfig {
                default_timeout: Duration::from_secs(30),
                max_timeout: Duration::from_secs(300),
                enable_caching: false,
                cache_ttl: Duration::from_secs(60),
                cache_max_size_mb: 100,
                cache_max_entries: 1000,
            },
            session: SessionConfig::default(),
        }
    }

    #[test]
    fn test_config_mode_detection() {
        let config = test_config();
        assert!(config.is_database_mode());
        assert_eq!(config.current_database(), Some("master"));

        let mut server_mode_config = config;
        server_mode_config.database.database = None;
        assert!(!server_mode_config.is_database_mode());
        assert_eq!(server_mode_config.current_database(), None);
    }
}
