//! SQL Server identifier escaping utilities.
//!
//! Uses SQL Server's bracket notation `[identifier]` to safely escape identifiers.
//! Also provides validation against SQL reserved keywords.

use crate::error::ServerError;
use once_cell::sync::Lazy;
use std::collections::HashSet;

/// Maximum length for SQL Server identifiers.
pub const MAX_IDENTIFIER_LENGTH: usize = 128;

/// Escape a SQL Server identifier using bracket notation.
///
/// This function handles:
/// - Schema-qualified names (`dbo.Users` -> `[dbo].[Users]`)
/// - Simple names (`Users` -> `[Users]`)
/// - Names with special characters
/// - Names that contain brackets (escaped as `]]`)
///
/// # Examples
///
/// ```
/// use mssql_mcp_server::security::escape_identifier;
///
/// assert_eq!(escape_identifier("Users").unwrap(), "[Users]");
/// assert_eq!(escape_identifier("dbo.Users").unwrap(), "[dbo].[Users]");
/// assert_eq!(escape_identifier("My Table").unwrap(), "[My Table]");
/// ```
pub fn escape_identifier(identifier: &str) -> Result<String, ServerError> {
    if identifier.is_empty() {
        return Err(ServerError::invalid_input("Identifier cannot be empty"));
    }

    // Handle schema-qualified names (e.g., "dbo.Users")
    if identifier.contains('.') {
        let parts: Vec<&str> = identifier.splitn(2, '.').collect();
        if parts.len() == 2 {
            let schema = escape_single_identifier(parts[0])?;
            let name = escape_single_identifier(parts[1])?;
            return Ok(format!("{}.{}", schema, name));
        }
    }

    escape_single_identifier(identifier)
}

/// Escape a single identifier (no dots).
fn escape_single_identifier(identifier: &str) -> Result<String, ServerError> {
    let trimmed = identifier.trim();

    if trimmed.is_empty() {
        return Err(ServerError::invalid_input("Identifier cannot be empty"));
    }

    if trimmed.len() > MAX_IDENTIFIER_LENGTH {
        return Err(ServerError::invalid_input(format!(
            "Identifier exceeds maximum length of {} characters",
            MAX_IDENTIFIER_LENGTH
        )));
    }

    // Check if already wrapped in brackets - only strip if both outer brackets exist
    let clean = if trimmed.starts_with('[') && trimmed.ends_with(']') {
        // Remove outer brackets only
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    // Escape any embedded right brackets by doubling them
    let escaped = clean.replace(']', "]]");

    Ok(format!("[{}]", escaped))
}

/// Validate that an identifier contains only allowed characters.
///
/// This is a stricter validation for use cases where we want to ensure
/// the identifier doesn't contain any potentially dangerous characters.
pub fn validate_identifier(identifier: &str) -> Result<(), ServerError> {
    if identifier.is_empty() {
        return Err(ServerError::invalid_input("Identifier cannot be empty"));
    }

    if identifier.len() > MAX_IDENTIFIER_LENGTH {
        return Err(ServerError::invalid_input(format!(
            "Identifier exceeds maximum length of {} characters",
            MAX_IDENTIFIER_LENGTH
        )));
    }

    // Check for SQL injection patterns
    let dangerous_patterns = [
        "--",   // SQL comment
        "/*",   // Multi-line comment start
        "*/",   // Multi-line comment end
        ";",    // Statement separator
        "'",    // String delimiter
        "\"",   // Quoted identifier delimiter (we use brackets instead)
        "\\",   // Escape character
        "\x00", // Null byte
    ];

    for pattern in &dangerous_patterns {
        if identifier.contains(pattern) {
            return Err(ServerError::invalid_input(format!(
                "Identifier contains forbidden character sequence: {}",
                pattern
            )));
        }
    }

    Ok(())
}

/// Validate and escape an identifier for safe use in SQL.
///
/// This combines validation, escaping, and reserved keyword warnings.
/// If the identifier is a SQL reserved keyword, a warning is logged
/// but the operation proceeds (bracket escaping makes it safe to use).
pub fn safe_identifier(identifier: &str) -> Result<String, ServerError> {
    validate_identifier(identifier)?;
    // Warn about reserved keywords (but allow them since escaping handles it)
    warn_if_reserved(identifier, "identifier");
    escape_identifier(identifier)
}

/// Parse a potentially schema-qualified identifier.
///
/// Returns (schema, name) tuple. Schema is None if not specified.
pub fn parse_qualified_name(identifier: &str) -> Result<(Option<String>, String), ServerError> {
    if identifier.is_empty() {
        return Err(ServerError::invalid_input("Identifier cannot be empty"));
    }

    if identifier.contains('.') {
        let parts: Vec<&str> = identifier.splitn(2, '.').collect();
        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            return Ok((Some(parts[0].to_string()), parts[1].to_string()));
        }
    }

    Ok((None, identifier.to_string()))
}

/// SQL Server reserved keywords (T-SQL 2019+).
///
/// Using a reserved keyword as an identifier requires bracket escaping.
/// This list includes both ANSI SQL and T-SQL specific reserved words.
static SQL_RESERVED_KEYWORDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        // ANSI SQL reserved words
        "ADD",
        "ALL",
        "ALTER",
        "AND",
        "ANY",
        "AS",
        "ASC",
        "AUTHORIZATION",
        "BACKUP",
        "BEGIN",
        "BETWEEN",
        "BREAK",
        "BROWSE",
        "BULK",
        "BY",
        "CASCADE",
        "CASE",
        "CHECK",
        "CHECKPOINT",
        "CLOSE",
        "CLUSTERED",
        "COALESCE",
        "COLLATE",
        "COLUMN",
        "COMMIT",
        "COMPUTE",
        "CONSTRAINT",
        "CONTAINS",
        "CONTAINSTABLE",
        "CONTINUE",
        "CONVERT",
        "CREATE",
        "CROSS",
        "CURRENT",
        "CURRENT_DATE",
        "CURRENT_TIME",
        "CURRENT_TIMESTAMP",
        "CURRENT_USER",
        "CURSOR",
        "DATABASE",
        "DBCC",
        "DEALLOCATE",
        "DECLARE",
        "DEFAULT",
        "DELETE",
        "DENY",
        "DESC",
        "DISK",
        "DISTINCT",
        "DISTRIBUTED",
        "DOUBLE",
        "DROP",
        "DUMP",
        "ELSE",
        "END",
        "ERRLVL",
        "ESCAPE",
        "EXCEPT",
        "EXEC",
        "EXECUTE",
        "EXISTS",
        "EXIT",
        "EXTERNAL",
        "FETCH",
        "FILE",
        "FILLFACTOR",
        "FOR",
        "FOREIGN",
        "FREETEXT",
        "FREETEXTTABLE",
        "FROM",
        "FULL",
        "FUNCTION",
        "GOTO",
        "GRANT",
        "GROUP",
        "HAVING",
        "HOLDLOCK",
        "IDENTITY",
        "IDENTITY_INSERT",
        "IDENTITYCOL",
        "IF",
        "IN",
        "INDEX",
        "INNER",
        "INSERT",
        "INTERSECT",
        "INTO",
        "IS",
        "JOIN",
        "KEY",
        "KILL",
        "LEFT",
        "LIKE",
        "LINENO",
        "LOAD",
        "MERGE",
        "NATIONAL",
        "NOCHECK",
        "NONCLUSTERED",
        "NOT",
        "NULL",
        "NULLIF",
        "OF",
        "OFF",
        "OFFSETS",
        "ON",
        "OPEN",
        "OPENDATASOURCE",
        "OPENQUERY",
        "OPENROWSET",
        "OPENXML",
        "OPTION",
        "OR",
        "ORDER",
        "OUTER",
        "OVER",
        "PERCENT",
        "PIVOT",
        "PLAN",
        "PRECISION",
        "PRIMARY",
        "PRINT",
        "PROC",
        "PROCEDURE",
        "PUBLIC",
        "RAISERROR",
        "READ",
        "READTEXT",
        "RECONFIGURE",
        "REFERENCES",
        "REPLICATION",
        "RESTORE",
        "RESTRICT",
        "RETURN",
        "REVERT",
        "REVOKE",
        "RIGHT",
        "ROLLBACK",
        "ROWCOUNT",
        "ROWGUIDCOL",
        "RULE",
        "SAVE",
        "SCHEMA",
        "SECURITYAUDIT",
        "SELECT",
        "SEMANTICKEYPHRASETABLE",
        "SEMANTICSIMILARITYDETAILSTABLE",
        "SEMANTICSIMILARITYTABLE",
        "SESSION_USER",
        "SET",
        "SETUSER",
        "SHUTDOWN",
        "SOME",
        "STATISTICS",
        "SYSTEM_USER",
        "TABLE",
        "TABLESAMPLE",
        "TEXTSIZE",
        "THEN",
        "TO",
        "TOP",
        "TRAN",
        "TRANSACTION",
        "TRIGGER",
        "TRUNCATE",
        "TRY_CONVERT",
        "TSEQUAL",
        "UNION",
        "UNIQUE",
        "UNPIVOT",
        "UPDATE",
        "UPDATETEXT",
        "USE",
        "USER",
        "VALUES",
        "VARYING",
        "VIEW",
        "WAITFOR",
        "WHEN",
        "WHERE",
        "WHILE",
        "WITH",
        "WITHIN GROUP",
        "WRITETEXT",
        // Additional T-SQL keywords
        "ABS",
        "ACOS",
        "APP_NAME",
        "ASCII",
        "ASIN",
        "ATAN",
        "ATN2",
        "AVG",
        "BINARY",
        "BIT",
        "CAST",
        "CEILING",
        "CHAR",
        "CHARINDEX",
        "CHOOSE",
        "COS",
        "COT",
        "COUNT",
        "COUNT_BIG",
        "DATALENGTH",
        "DATEADD",
        "DATEDIFF",
        "DATEFROMPARTS",
        "DATENAME",
        "DATEPART",
        "DATETIME",
        "DATETIME2",
        "DATETIMEOFFSET",
        "DAY",
        "DB_ID",
        "DB_NAME",
        "DECIMAL",
        "EXP",
        "FLOAT",
        "FLOOR",
        "FORMAT",
        "GETDATE",
        "GETUTCDATE",
        "GO",
        "HOST_ID",
        "HOST_NAME",
        "IIF",
        "IMAGE",
        "INT",
        "ISNULL",
        "ISNUMERIC",
        "LEN",
        "LOG",
        "LOG10",
        "LOWER",
        "LTRIM",
        "MAX",
        "MIN",
        "MONEY",
        "MONTH",
        "NCHAR",
        "NEWID",
        "NTEXT",
        "NUMERIC",
        "NVARCHAR",
        "OBJECT_ID",
        "OBJECT_NAME",
        "PARSE",
        "PATINDEX",
        "PERMISSIONS",
        "PI",
        "POWER",
        "QUOTENAME",
        "RAND",
        "REAL",
        "REPLACE",
        "REPLICATE",
        "REVERSE",
        "ROUND",
        "ROW_NUMBER",
        "RTRIM",
        "SCOPE_IDENTITY",
        "SIGN",
        "SIN",
        "SMALLDATETIME",
        "SMALLINT",
        "SMALLMONEY",
        "SOUNDEX",
        "SPACE",
        "SQL_VARIANT",
        "SQRT",
        "STR",
        "STRING_SPLIT",
        "STUFF",
        "SUBSTRING",
        "SUM",
        "SUSER_NAME",
        "TAN",
        "TEXT",
        "TIME",
        "TIMESTAMP",
        "TINYINT",
        "TRANSLATE",
        "TRIM",
        "TYPE_ID",
        "TYPE_NAME",
        "UNICODE",
        "UNIQUEIDENTIFIER",
        "UPPER",
        "USER_ID",
        "USER_NAME",
        "VARBINARY",
        "VARCHAR",
        "XACT_STATE",
        "XML",
        "YEAR",
    ]
    .iter()
    .copied()
    .collect()
});

/// Check if an identifier is a SQL reserved keyword.
///
/// Returns true if the identifier (case-insensitive) matches a SQL Server reserved keyword.
pub fn is_reserved_keyword(identifier: &str) -> bool {
    SQL_RESERVED_KEYWORDS.contains(identifier.to_uppercase().as_str())
}

/// Validate that an identifier is not a reserved keyword.
///
/// Returns an error if the identifier is a reserved keyword without bracket escaping.
pub fn validate_not_reserved(identifier: &str) -> Result<(), ServerError> {
    // Extract the actual name from bracket-escaped identifiers
    let name = if identifier.starts_with('[') && identifier.ends_with(']') {
        // Already escaped, so it's safe to use
        return Ok(());
    } else if identifier.contains('.') {
        // For qualified names, check each part
        for part in identifier.split('.') {
            let clean_part = if part.starts_with('[') && part.ends_with(']') {
                continue; // Already escaped
            } else {
                part.trim()
            };

            if is_reserved_keyword(clean_part) {
                return Err(ServerError::invalid_input(format!(
                    "'{}' is a SQL reserved keyword. Consider using bracket escaping: [{}]",
                    clean_part, clean_part
                )));
            }
        }
        return Ok(());
    } else {
        identifier.trim()
    };

    if is_reserved_keyword(name) {
        return Err(ServerError::invalid_input(format!(
            "'{}' is a SQL reserved keyword. Consider using bracket escaping: [{}]",
            name, name
        )));
    }

    Ok(())
}

/// Warn (log) if an identifier is a reserved keyword.
///
/// Unlike `validate_not_reserved`, this doesn't return an error, just logs a warning.
/// This is useful for soft validation where we want to notify but not block.
pub fn warn_if_reserved(identifier: &str, context: &str) {
    let check_name = |name: &str| {
        if is_reserved_keyword(name) {
            tracing::warn!(
                "{}: '{}' is a SQL reserved keyword. This may cause issues in some contexts.",
                context,
                name
            );
        }
    };

    if identifier.starts_with('[') && identifier.ends_with(']') {
        return; // Already escaped
    }

    if identifier.contains('.') {
        for part in identifier.split('.') {
            if !(part.starts_with('[') && part.ends_with(']')) {
                check_name(part.trim());
            }
        }
    } else {
        check_name(identifier.trim());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_simple_identifier() {
        assert_eq!(escape_identifier("Users").unwrap(), "[Users]");
        assert_eq!(escape_identifier("MyTable").unwrap(), "[MyTable]");
    }

    #[test]
    fn test_escape_qualified_identifier() {
        assert_eq!(escape_identifier("dbo.Users").unwrap(), "[dbo].[Users]");
        assert_eq!(
            escape_identifier("schema.table").unwrap(),
            "[schema].[table]"
        );
    }

    #[test]
    fn test_escape_with_spaces() {
        assert_eq!(escape_identifier("My Table").unwrap(), "[My Table]");
        assert_eq!(
            escape_identifier("dbo.My Table").unwrap(),
            "[dbo].[My Table]"
        );
    }

    #[test]
    fn test_escape_with_brackets() {
        assert_eq!(escape_identifier("Table[1]").unwrap(), "[Table[1]]]");
    }

    #[test]
    fn test_escape_already_escaped() {
        assert_eq!(escape_identifier("[Users]").unwrap(), "[Users]");
    }

    #[test]
    fn test_empty_identifier() {
        assert!(escape_identifier("").is_err());
    }

    #[test]
    fn test_validate_identifier() {
        assert!(validate_identifier("Users").is_ok());
        assert!(validate_identifier("my_table").is_ok());
        assert!(validate_identifier("Table123").is_ok());

        // Dangerous patterns
        assert!(validate_identifier("Users--").is_err());
        assert!(validate_identifier("Users/*comment").is_err());
        assert!(validate_identifier("Users;DROP").is_err());
        assert!(validate_identifier("Users'").is_err());
    }

    #[test]
    fn test_parse_qualified_name() {
        let (schema, name) = parse_qualified_name("dbo.Users").unwrap();
        assert_eq!(schema, Some("dbo".to_string()));
        assert_eq!(name, "Users");

        let (schema, name) = parse_qualified_name("Users").unwrap();
        assert_eq!(schema, None);
        assert_eq!(name, "Users");
    }

    #[test]
    fn test_is_reserved_keyword() {
        // Common reserved keywords
        assert!(is_reserved_keyword("SELECT"));
        assert!(is_reserved_keyword("select")); // case insensitive
        assert!(is_reserved_keyword("INSERT"));
        assert!(is_reserved_keyword("UPDATE"));
        assert!(is_reserved_keyword("DELETE"));
        assert!(is_reserved_keyword("DROP"));
        assert!(is_reserved_keyword("TABLE"));
        assert!(is_reserved_keyword("FROM"));
        assert!(is_reserved_keyword("WHERE"));

        // T-SQL specific
        assert!(is_reserved_keyword("EXEC"));
        assert!(is_reserved_keyword("DECLARE"));
        assert!(is_reserved_keyword("GETDATE"));

        // Not reserved
        assert!(!is_reserved_keyword("Users"));
        assert!(!is_reserved_keyword("MyTable"));
        assert!(!is_reserved_keyword("CustomerName"));
    }

    #[test]
    fn test_validate_not_reserved() {
        // Normal identifiers should pass
        assert!(validate_not_reserved("Users").is_ok());
        assert!(validate_not_reserved("my_table").is_ok());
        assert!(validate_not_reserved("dbo.Users").is_ok());

        // Reserved keywords should fail
        assert!(validate_not_reserved("select").is_err());
        assert!(validate_not_reserved("TABLE").is_err());
        assert!(validate_not_reserved("from").is_err());

        // Escaped identifiers should pass even if they're keywords
        assert!(validate_not_reserved("[SELECT]").is_ok());
        assert!(validate_not_reserved("[TABLE]").is_ok());
        assert!(validate_not_reserved("dbo.[SELECT]").is_ok());
        assert!(validate_not_reserved("[dbo].[TABLE]").is_ok());

        // Qualified names with reserved keywords
        assert!(validate_not_reserved("dbo.SELECT").is_err());
    }

    #[test]
    fn test_reserved_keyword_count() {
        // Sanity check that we have a reasonable number of keywords
        assert!(SQL_RESERVED_KEYWORDS.len() > 200);
        assert!(SQL_RESERVED_KEYWORDS.len() < 400);
    }
}
