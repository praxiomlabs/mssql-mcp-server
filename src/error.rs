//! Error types for the MSSQL MCP Server.
//!
//! This module defines semantic error types with SQL Server error code mapping
//! for user-friendly error messages.

use rmcp::ErrorData;
use thiserror::Error;

/// Domain-specific errors for the MSSQL MCP Server.
#[derive(Debug, Error)]
pub enum McpError {
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Connection error
    #[error("Connection error: {message}")]
    Connection {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Authentication error
    #[error("Authentication failed: {0}")]
    Authentication(String),

    /// Database not found
    #[error("Database not found: {0}")]
    DatabaseNotFound(String),

    /// Object not found (table, view, procedure, etc.)
    #[error("{object_type} not found: {name}")]
    ObjectNotFound { object_type: String, name: String },

    /// Permission denied
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    /// Query validation error
    #[error("Query validation failed: {0}")]
    ValidationFailed(String),

    /// SQL injection detected
    #[error("Potential SQL injection detected: {0}")]
    InjectionDetected(String),

    /// Query execution error
    #[error("Query execution error: {message}")]
    QueryExecution {
        message: String,
        sql_error_code: Option<i32>,
        sql_state: Option<String>,
    },

    /// Query timeout
    #[error("Query timeout: operation exceeded {timeout_seconds} seconds")]
    Timeout { timeout_seconds: u64 },

    /// Circuit breaker open
    #[error(
        "Circuit breaker open: service unavailable, retry after {retry_after_seconds} seconds"
    )]
    CircuitOpen { retry_after_seconds: u64 },

    /// Constraint violation
    #[error("Constraint violation: {0}")]
    ConstraintViolation(String),

    /// Data truncation
    #[error("Data truncation: {0}")]
    DataTruncation(String),

    /// Session error
    #[error("Session error: {0}")]
    Session(String),

    /// Session not found
    #[error("Session not found: {0}")]
    SessionNotFound(String),

    /// Resource not found
    #[error("Resource not found: {0}")]
    ResourceNotFound(String),

    /// Invalid input
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

impl McpError {
    /// Create a configuration error.
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    /// Create a connection error.
    pub fn connection(msg: impl Into<String>) -> Self {
        Self::Connection {
            message: msg.into(),
            source: None,
        }
    }

    /// Create a connection error with a source.
    pub fn connection_with_source(
        msg: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Connection {
            message: msg.into(),
            source: Some(Box::new(source)),
        }
    }

    /// Create an authentication error.
    pub fn auth(msg: impl Into<String>) -> Self {
        Self::Authentication(msg.into())
    }

    /// Create an object not found error.
    pub fn object_not_found(object_type: impl Into<String>, name: impl Into<String>) -> Self {
        Self::ObjectNotFound {
            object_type: object_type.into(),
            name: name.into(),
        }
    }

    /// Create a permission denied error.
    pub fn permission_denied(msg: impl Into<String>) -> Self {
        Self::PermissionDenied(msg.into())
    }

    /// Create a validation error.
    pub fn validation(msg: impl Into<String>) -> Self {
        Self::ValidationFailed(msg.into())
    }

    /// Create an injection detection error.
    pub fn injection(msg: impl Into<String>) -> Self {
        Self::InjectionDetected(msg.into())
    }

    /// Create a query execution error.
    pub fn query_error(msg: impl Into<String>) -> Self {
        Self::QueryExecution {
            message: msg.into(),
            sql_error_code: None,
            sql_state: None,
        }
    }

    /// Create a query execution error with SQL error details.
    pub fn query_error_with_code(msg: impl Into<String>, code: i32, state: Option<String>) -> Self {
        Self::QueryExecution {
            message: msg.into(),
            sql_error_code: Some(code),
            sql_state: state,
        }
    }

    /// Create a timeout error.
    pub fn timeout(seconds: u64) -> Self {
        Self::Timeout {
            timeout_seconds: seconds,
        }
    }

    /// Create a circuit breaker open error.
    pub fn circuit_open(retry_after_seconds: u64) -> Self {
        Self::CircuitOpen {
            retry_after_seconds,
        }
    }

    /// Create a session not found error.
    pub fn session_not_found(id: impl Into<String>) -> Self {
        Self::SessionNotFound(id.into())
    }

    /// Create a resource not found error.
    pub fn resource_not_found(uri: impl Into<String>) -> Self {
        Self::ResourceNotFound(uri.into())
    }

    /// Create an invalid input error.
    pub fn invalid_input(msg: impl Into<String>) -> Self {
        Self::InvalidInput(msg.into())
    }

    /// Create an internal error.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    /// Check if this error is transient and may succeed on retry.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Connection { .. } => true,
            Self::Timeout { .. } => true,
            Self::CircuitOpen { .. } => true,
            Self::QueryExecution {
                sql_error_code: Some(code),
                ..
            } => is_transient_sql_error(*code),
            _ => false,
        }
    }

    /// Get a user-friendly suggestion for how to fix this error.
    pub fn suggestion(&self) -> Option<&'static str> {
        match self {
            Self::Config(_) => Some("Check your environment variables and configuration"),
            Self::Connection { .. } => {
                Some("Check server hostname, port, and network connectivity")
            }
            Self::Authentication(_) => Some("Verify your username and password are correct"),
            Self::DatabaseNotFound(_) => Some("Check the database name and ensure it exists"),
            Self::ObjectNotFound { .. } => Some("Check the object name and schema"),
            Self::PermissionDenied(_) => {
                Some("Request appropriate permissions from your database administrator")
            }
            Self::ValidationFailed(_) => Some("Review your query against the validation rules"),
            Self::InjectionDetected(_) => {
                Some("Use parameterized queries instead of string concatenation")
            }
            Self::Timeout { .. } => Some("Try a simpler query or increase the timeout limit"),
            Self::CircuitOpen { .. } => {
                Some("Service is temporarily unavailable due to repeated failures. Wait and retry.")
            }
            Self::ConstraintViolation(_) => {
                Some("Check the constraint definition and your data values")
            }
            _ => None,
        }
    }
}

/// Map SQL Server error codes to semantic McpError types.
pub fn from_sql_error(code: i32, message: &str) -> McpError {
    match code {
        // Authentication errors
        18456 => McpError::auth(format!("Login failed: {}", message)),

        // Database errors
        4060 => McpError::DatabaseNotFound(message.to_string()),

        // Object not found errors
        208 => McpError::object_not_found("Object", message),
        2812 => McpError::object_not_found("Stored procedure", message),

        // Permission errors
        229 | 230 => McpError::permission_denied(message),
        262 => McpError::permission_denied(format!("CREATE permission denied: {}", message)),

        // Timeout
        -2 => McpError::timeout(0),

        // Connection errors
        -1 => McpError::connection("Connection broken"),
        53 => McpError::connection("Server not found or not accessible"),

        // Constraint violations
        547 => McpError::ConstraintViolation(message.to_string()),
        2601 | 2627 => McpError::ConstraintViolation(format!("Duplicate key: {}", message)),

        // Data errors
        8115 => McpError::QueryExecution {
            message: format!("Arithmetic overflow: {}", message),
            sql_error_code: Some(code),
            sql_state: None,
        },
        8152 => McpError::DataTruncation(message.to_string()),

        // Syntax errors
        102 => McpError::query_error_with_code(format!("Syntax error: {}", message), code, None),

        // Invalid column/object
        207 => McpError::query_error_with_code(format!("Invalid column: {}", message), code, None),
        201 => McpError::query_error_with_code(format!("Invalid object: {}", message), code, None),

        // Deadlock
        1205 => McpError::QueryExecution {
            message: "Transaction was deadlocked and has been rolled back".to_string(),
            sql_error_code: Some(code),
            sql_state: None,
        },

        // Default: generic query error
        _ => McpError::query_error_with_code(message, code, None),
    }
}

/// Check if a SQL Server error code indicates a transient error.
fn is_transient_sql_error(code: i32) -> bool {
    matches!(
        code,
        -2      // Timeout
        | -1    // Connection broken
        | 1205  // Deadlock
        | 10053 // Connection forcibly closed
        | 10054 // Connection reset
        | 10060 // Connection timed out
        | 40197 // Azure: service error
        | 40501 // Azure: service busy
        | 40613 // Azure: database unavailable
        | 49918 // Azure: not enough resources
        | 49919 // Azure: too many requests
        | 49920 // Azure: too busy
    )
}

/// Convert McpError to rmcp's ErrorData for protocol responses.
///
/// Note: Tool errors should generally return `CallToolResult` with `is_error: true`
/// instead of using this conversion. This is primarily for protocol-level errors.
impl From<McpError> for ErrorData {
    fn from(e: McpError) -> Self {
        match e {
            McpError::Config(msg) => ErrorData::invalid_request(msg, None),
            McpError::InvalidInput(msg) => ErrorData::invalid_params(msg, None),
            McpError::ValidationFailed(msg) => ErrorData::invalid_params(msg, None),
            McpError::InjectionDetected(msg) => ErrorData::invalid_params(msg, None),
            McpError::ResourceNotFound(msg) => {
                ErrorData::invalid_params(format!("Resource not found: {}", msg), None)
            }
            McpError::SessionNotFound(msg) => {
                ErrorData::invalid_params(format!("Session not found: {}", msg), None)
            }
            McpError::ObjectNotFound { object_type, name } => {
                ErrorData::invalid_params(format!("{} not found: {}", object_type, name), None)
            }
            McpError::Connection { message, .. }
            | McpError::Authentication(message)
            | McpError::DatabaseNotFound(message)
            | McpError::PermissionDenied(message)
            | McpError::Session(message)
            | McpError::Internal(message) => ErrorData::internal_error(message, None),
            McpError::QueryExecution { message, .. } => ErrorData::internal_error(message, None),
            McpError::Timeout { timeout_seconds } => ErrorData::internal_error(
                format!("Query timeout after {} seconds", timeout_seconds),
                None,
            ),
            McpError::CircuitOpen {
                retry_after_seconds,
            } => ErrorData::internal_error(
                format!(
                    "Service unavailable, circuit breaker open. Retry after {} seconds",
                    retry_after_seconds
                ),
                None,
            ),
            McpError::ConstraintViolation(msg) | McpError::DataTruncation(msg) => {
                ErrorData::internal_error(msg, None)
            }
        }
    }
}

impl From<mssql_client::Error> for McpError {
    fn from(e: mssql_client::Error) -> Self {
        use mssql_client::Error;

        match &e {
            Error::Server {
                number, message, ..
            } => from_sql_error(*number, message),
            Error::Io(_) => McpError::connection(format!("IO error: {}", e)),
            Error::Tls(_) => McpError::connection(format!("TLS error: {}", e)),
            Error::Protocol(_) => McpError::connection(format!("Protocol error: {}", e)),
            Error::Authentication(_) => McpError::auth(e.to_string()),
            Error::Connection(_) => McpError::connection(e.to_string()),
            Error::ConnectionClosed => McpError::connection("Connection closed"),
            Error::ConnectTimeout | Error::ConnectionTimeout | Error::CommandTimeout => {
                McpError::timeout(0)
            }
            Error::Type(_) => McpError::query_error(format!("Type conversion error: {}", e)),
            Error::Codec(_) => McpError::query_error(format!("Codec error: {}", e)),
            Error::Query(_) => McpError::query_error(e.to_string()),
            Error::Transaction(_) => McpError::query_error(format!("Transaction error: {}", e)),
            Error::Config(_) => McpError::config(e.to_string()),
            Error::PoolExhausted => McpError::connection("Connection pool exhausted"),
            Error::Cancelled => McpError::query_error("Query was cancelled"),
            _ => McpError::internal(e.to_string()),
        }
    }
}

impl From<mssql_driver_pool::PoolError> for McpError {
    fn from(e: mssql_driver_pool::PoolError) -> Self {
        McpError::connection(format!("Pool error: {}", e))
    }
}

impl From<std::io::ErrorKind> for McpError {
    fn from(kind: std::io::ErrorKind) -> Self {
        use std::io::ErrorKind;
        match kind {
            ErrorKind::ConnectionRefused => McpError::connection("Connection refused"),
            ErrorKind::ConnectionReset => McpError::connection("Connection reset"),
            ErrorKind::ConnectionAborted => McpError::connection("Connection aborted"),
            ErrorKind::NotConnected => McpError::connection("Not connected"),
            ErrorKind::TimedOut => McpError::timeout(0),
            _ => McpError::connection(format!("IO error: {:?}", kind)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sql_error_mapping() {
        let err = from_sql_error(18456, "Login failed for user 'test'");
        assert!(matches!(err, McpError::Authentication(_)));

        let err = from_sql_error(208, "Invalid object name 'foo'");
        assert!(matches!(err, McpError::ObjectNotFound { .. }));

        let err = from_sql_error(229, "SELECT permission denied");
        assert!(matches!(err, McpError::PermissionDenied(_)));
    }

    #[test]
    fn test_transient_errors() {
        let err = McpError::timeout(30);
        assert!(err.is_transient());

        let err = McpError::connection("test");
        assert!(err.is_transient());

        let err = McpError::auth("test");
        assert!(!err.is_transient());
    }

    #[test]
    fn test_error_suggestions() {
        let err = McpError::auth("Login failed");
        assert!(err.suggestion().is_some());

        let err = McpError::Internal("unknown".to_string());
        assert!(err.suggestion().is_none());
    }
}
