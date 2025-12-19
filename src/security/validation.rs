//! Query validation for different security modes.

use crate::error::McpError;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Query validation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ValidationMode {
    /// Read-only mode: Only SELECT queries allowed.
    /// Blocks: INSERT, UPDATE, DELETE, DROP, CREATE, ALTER, TRUNCATE, EXEC, etc.
    ReadOnly,

    /// Standard mode: DML allowed, DDL blocked.
    /// Allows: SELECT, INSERT, UPDATE, DELETE
    /// Blocks: DROP, CREATE, ALTER, TRUNCATE, EXEC (dangerous stored procs)
    #[default]
    Standard,

    /// Unrestricted mode: All queries allowed.
    /// Warning: Use only with trusted inputs or proper database permissions.
    Unrestricted,
}

/// Result of query validation.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether the query is valid
    pub valid: bool,
    /// Detected query type
    pub query_type: QueryType,
    /// Validation message (error description if invalid)
    pub message: Option<String>,
}

/// Type of SQL query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryType {
    Select,
    Insert,
    Update,
    Delete,
    Create,
    Alter,
    Drop,
    Truncate,
    Execute,
    Merge,
    Grant,
    Revoke,
    Other,
}

impl QueryType {
    /// Check if this is a read operation.
    pub fn is_read(&self) -> bool {
        matches!(self, QueryType::Select)
    }

    /// Check if this is a DML operation (data modification).
    pub fn is_dml(&self) -> bool {
        matches!(
            self,
            QueryType::Select | QueryType::Insert | QueryType::Update | QueryType::Delete
        )
    }

    /// Check if this is a DDL operation (schema modification).
    pub fn is_ddl(&self) -> bool {
        matches!(
            self,
            QueryType::Create | QueryType::Alter | QueryType::Drop | QueryType::Truncate
        )
    }
}

/// Regex patterns for query type detection.
///
/// These patterns are compiled once at first use (lazy static) for performance.
/// All patterns are hardcoded constants that have been verified to be valid regex.
static QUERY_TYPE_PATTERNS: Lazy<Vec<(Regex, QueryType)>> = Lazy::new(|| {
    // Helper to compile regex with better error message on failure.
    // These should never fail since patterns are hardcoded and tested.
    fn compile(pattern: &str) -> Regex {
        Regex::new(pattern).unwrap_or_else(|e| {
            panic!("Internal error: invalid regex pattern '{}': {}", pattern, e)
        })
    }

    vec![
        (compile(r"(?i)^\s*SELECT\b"), QueryType::Select),
        (compile(r"(?i)^\s*WITH\b"), QueryType::Select), // CTEs are SELECT
        (compile(r"(?i)^\s*INSERT\b"), QueryType::Insert),
        (compile(r"(?i)^\s*UPDATE\b"), QueryType::Update),
        (compile(r"(?i)^\s*DELETE\b"), QueryType::Delete),
        (compile(r"(?i)^\s*CREATE\b"), QueryType::Create),
        (compile(r"(?i)^\s*ALTER\b"), QueryType::Alter),
        (compile(r"(?i)^\s*DROP\b"), QueryType::Drop),
        (compile(r"(?i)^\s*TRUNCATE\b"), QueryType::Truncate),
        (compile(r"(?i)^\s*EXEC\b"), QueryType::Execute),
        (compile(r"(?i)^\s*EXECUTE\b"), QueryType::Execute),
        (compile(r"(?i)^\s*MERGE\b"), QueryType::Merge),
        (compile(r"(?i)^\s*GRANT\b"), QueryType::Grant),
        (compile(r"(?i)^\s*REVOKE\b"), QueryType::Revoke),
    ]
});

/// Dangerous keywords that are blocked in Standard mode.
///
/// These patterns detect potentially dangerous SQL Server operations.
/// All patterns are hardcoded constants that have been verified to be valid regex.
static DANGEROUS_KEYWORDS: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    // Helper to compile regex with better error message on failure.
    fn compile(pattern: &str) -> Regex {
        Regex::new(pattern).unwrap_or_else(|e| {
            panic!("Internal error: invalid regex pattern '{}': {}", pattern, e)
        })
    }

    vec![
        // Extended stored procedures
        (compile(r"(?i)\bxp_\w+"), "xp_ extended stored procedure"),
        (compile(r"(?i)\bsp_oa\w+"), "sp_oa OLE automation procedure"),
        // Dangerous system procedures
        (compile(r"(?i)\bsp_configure\b"), "sp_configure"),
        (compile(r"(?i)\bsp_addlogin\b"), "sp_addlogin"),
        (compile(r"(?i)\bsp_droplogin\b"), "sp_droplogin"),
        (
            compile(r"(?i)\bsp_addsrvrolemember\b"),
            "sp_addsrvrolemember",
        ),
        // Bulk operations
        (compile(r"(?i)\bBULK\s+INSERT\b"), "BULK INSERT"),
        (compile(r"(?i)\bOPENROWSET\b"), "OPENROWSET"),
        (compile(r"(?i)\bOPENDATASOURCE\b"), "OPENDATASOURCE"),
        (compile(r"(?i)\bOPENQUERY\b"), "OPENQUERY"),
        // Backup/restore (server-level operations)
        (compile(r"(?i)\bBACKUP\b"), "BACKUP"),
        (compile(r"(?i)\bRESTORE\b"), "RESTORE"),
        // Shutdown
        (compile(r"(?i)\bSHUTDOWN\b"), "SHUTDOWN"),
    ]
});

/// Pattern for safe EXEC commands (metadata procedures).
///
/// These procedures are allowed in standard mode as they only read metadata.
static SAFE_EXEC_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*EXEC(UTE)?\s+(sp_help|sp_columns|sp_tables|sp_stored_procedures|sp_fkeys|sp_pkeys)\b")
        .unwrap_or_else(|e| panic!("Internal error: invalid safe exec pattern: {}", e))
});

/// Query validator.
#[derive(Debug, Clone)]
pub struct QueryValidator {
    mode: ValidationMode,
    max_length: usize,
}

impl QueryValidator {
    /// Create a new query validator.
    pub fn new(mode: ValidationMode, max_length: usize) -> Self {
        Self { mode, max_length }
    }

    /// Validate a query against the current mode.
    pub fn validate(&self, query: &str) -> Result<ValidationResult, McpError> {
        // Check query length
        if query.len() > self.max_length {
            return Err(McpError::validation(format!(
                "Query exceeds maximum length of {} bytes",
                self.max_length
            )));
        }

        // Detect query type
        let query_type = detect_query_type(query);

        // Validate based on mode
        match self.mode {
            ValidationMode::ReadOnly => self.validate_read_only(query, query_type),
            ValidationMode::Standard => self.validate_standard(query, query_type),
            ValidationMode::Unrestricted => Ok(ValidationResult {
                valid: true,
                query_type,
                message: None,
            }),
        }
    }

    /// Validate in read-only mode.
    fn validate_read_only(
        &self,
        _query: &str,
        query_type: QueryType,
    ) -> Result<ValidationResult, McpError> {
        if query_type.is_read() {
            Ok(ValidationResult {
                valid: true,
                query_type,
                message: None,
            })
        } else {
            Err(McpError::validation(format!(
                "Query type {:?} is not allowed in read-only mode. Only SELECT queries are permitted.",
                query_type
            )))
        }
    }

    /// Validate in standard mode.
    fn validate_standard(
        &self,
        query: &str,
        query_type: QueryType,
    ) -> Result<ValidationResult, McpError> {
        // Block DDL operations
        if query_type.is_ddl() {
            return Err(McpError::validation(format!(
                "Query type {:?} is not allowed in standard mode. DDL operations are blocked.",
                query_type
            )));
        }

        // Block permission operations
        if matches!(query_type, QueryType::Grant | QueryType::Revoke) {
            return Err(McpError::validation(
                "Permission operations (GRANT/REVOKE) are not allowed in standard mode",
            ));
        }

        // Block EXECUTE unless it's a known safe pattern
        if query_type == QueryType::Execute {
            // Allow sp_help, sp_columns, sp_tables (metadata procedures)
            if !SAFE_EXEC_PATTERN.is_match(query) {
                return Err(McpError::validation(
                    "Arbitrary EXEC/EXECUTE is not allowed in standard mode",
                ));
            }
        }

        // Check for dangerous keywords
        for (pattern, keyword) in DANGEROUS_KEYWORDS.iter() {
            if pattern.is_match(query) {
                return Err(McpError::validation(format!(
                    "Dangerous keyword '{}' is not allowed in standard mode",
                    keyword
                )));
            }
        }

        Ok(ValidationResult {
            valid: true,
            query_type,
            message: None,
        })
    }

    /// Get the current validation mode.
    pub fn mode(&self) -> ValidationMode {
        self.mode
    }
}

/// Detect the type of a SQL query.
fn detect_query_type(query: &str) -> QueryType {
    // Remove leading comments
    let trimmed = remove_leading_comments(query);

    for (pattern, query_type) in QUERY_TYPE_PATTERNS.iter() {
        if pattern.is_match(&trimmed) {
            return *query_type;
        }
    }

    QueryType::Other
}

/// Remove leading SQL comments from a query.
fn remove_leading_comments(query: &str) -> String {
    let mut result = query.to_string();

    loop {
        let trimmed = result.trim_start();

        // Remove line comments
        if trimmed.starts_with("--") {
            if let Some(newline_pos) = trimmed.find('\n') {
                result = trimmed[newline_pos + 1..].to_string();
                continue;
            } else {
                // Entire query is a comment
                return String::new();
            }
        }

        // Remove block comments
        if trimmed.starts_with("/*") {
            if let Some(end_pos) = trimmed.find("*/") {
                result = trimmed[end_pos + 2..].to_string();
                continue;
            } else {
                // Unclosed comment
                return String::new();
            }
        }

        break;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_only_validator() -> QueryValidator {
        QueryValidator::new(ValidationMode::ReadOnly, 1_000_000)
    }

    fn standard_validator() -> QueryValidator {
        QueryValidator::new(ValidationMode::Standard, 1_000_000)
    }

    #[test]
    fn test_detect_query_type() {
        assert_eq!(detect_query_type("SELECT * FROM Users"), QueryType::Select);
        assert_eq!(
            detect_query_type("  SELECT * FROM Users"),
            QueryType::Select
        );
        assert_eq!(
            detect_query_type("INSERT INTO Users VALUES (1)"),
            QueryType::Insert
        );
        assert_eq!(
            detect_query_type("UPDATE Users SET name = 'foo'"),
            QueryType::Update
        );
        assert_eq!(
            detect_query_type("DELETE FROM Users WHERE id = 1"),
            QueryType::Delete
        );
        assert_eq!(detect_query_type("DROP TABLE Users"), QueryType::Drop);
        assert_eq!(
            detect_query_type("CREATE TABLE Users (id INT)"),
            QueryType::Create
        );
        assert_eq!(
            detect_query_type("WITH cte AS (SELECT 1) SELECT * FROM cte"),
            QueryType::Select
        );
    }

    #[test]
    fn test_detect_with_comments() {
        assert_eq!(
            detect_query_type("-- comment\nSELECT * FROM Users"),
            QueryType::Select
        );
        assert_eq!(
            detect_query_type("/* comment */ SELECT * FROM Users"),
            QueryType::Select
        );
    }

    #[test]
    fn test_read_only_validation() {
        let v = read_only_validator();

        assert!(v.validate("SELECT * FROM Users").is_ok());
        assert!(v.validate("INSERT INTO Users VALUES (1)").is_err());
        assert!(v.validate("UPDATE Users SET name = 'foo'").is_err());
        assert!(v.validate("DELETE FROM Users").is_err());
        assert!(v.validate("DROP TABLE Users").is_err());
    }

    #[test]
    fn test_standard_validation() {
        let v = standard_validator();

        // DML allowed
        assert!(v.validate("SELECT * FROM Users").is_ok());
        assert!(v.validate("INSERT INTO Users VALUES (1)").is_ok());
        assert!(v.validate("UPDATE Users SET name = 'foo'").is_ok());
        assert!(v.validate("DELETE FROM Users WHERE id = 1").is_ok());

        // DDL blocked
        assert!(v.validate("DROP TABLE Users").is_err());
        assert!(v.validate("CREATE TABLE Users (id INT)").is_err());
        assert!(v.validate("ALTER TABLE Users ADD col INT").is_err());
        assert!(v.validate("TRUNCATE TABLE Users").is_err());
    }

    #[test]
    fn test_dangerous_keywords() {
        let v = standard_validator();

        assert!(v.validate("EXEC xp_cmdshell 'dir'").is_err());
        assert!(v.validate("SELECT * FROM OPENROWSET(...)").is_err());
        assert!(v.validate("BACKUP DATABASE foo").is_err());
    }

    #[test]
    fn test_safe_exec() {
        let v = standard_validator();

        // Safe metadata procedures
        assert!(v.validate("EXEC sp_help 'Users'").is_ok());
        assert!(v.validate("EXEC sp_columns 'Users'").is_ok());

        // Arbitrary EXEC blocked
        assert!(v.validate("EXEC my_dangerous_proc").is_err());
    }

    #[test]
    fn test_query_length_limit() {
        let v = QueryValidator::new(ValidationMode::ReadOnly, 100);
        let long_query = "SELECT ".to_string() + &"x".repeat(100);
        assert!(v.validate(&long_query).is_err());
    }
}
