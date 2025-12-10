//! SQL Server identifier escaping utilities.
//!
//! Uses SQL Server's bracket notation `[identifier]` to safely escape identifiers.

use crate::error::McpError;

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
pub fn escape_identifier(identifier: &str) -> Result<String, McpError> {
    if identifier.is_empty() {
        return Err(McpError::invalid_input("Identifier cannot be empty"));
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
fn escape_single_identifier(identifier: &str) -> Result<String, McpError> {
    let trimmed = identifier.trim();

    if trimmed.is_empty() {
        return Err(McpError::invalid_input("Identifier cannot be empty"));
    }

    if trimmed.len() > MAX_IDENTIFIER_LENGTH {
        return Err(McpError::invalid_input(format!(
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
pub fn validate_identifier(identifier: &str) -> Result<(), McpError> {
    if identifier.is_empty() {
        return Err(McpError::invalid_input("Identifier cannot be empty"));
    }

    if identifier.len() > MAX_IDENTIFIER_LENGTH {
        return Err(McpError::invalid_input(format!(
            "Identifier exceeds maximum length of {} characters",
            MAX_IDENTIFIER_LENGTH
        )));
    }

    // Check for SQL injection patterns
    let dangerous_patterns = [
        "--",  // SQL comment
        "/*",  // Multi-line comment start
        "*/",  // Multi-line comment end
        ";",   // Statement separator
        "'",   // String delimiter
        "\"",  // Quoted identifier delimiter (we use brackets instead)
        "\\",  // Escape character
        "\x00", // Null byte
    ];

    for pattern in &dangerous_patterns {
        if identifier.contains(pattern) {
            return Err(McpError::invalid_input(format!(
                "Identifier contains forbidden character sequence: {}",
                pattern
            )));
        }
    }

    Ok(())
}

/// Validate and escape an identifier for safe use in SQL.
///
/// This combines validation and escaping for convenience.
pub fn safe_identifier(identifier: &str) -> Result<String, McpError> {
    validate_identifier(identifier)?;
    escape_identifier(identifier)
}

/// Parse a potentially schema-qualified identifier.
///
/// Returns (schema, name) tuple. Schema is None if not specified.
pub fn parse_qualified_name(identifier: &str) -> Result<(Option<String>, String), McpError> {
    if identifier.is_empty() {
        return Err(McpError::invalid_input("Identifier cannot be empty"));
    }

    if identifier.contains('.') {
        let parts: Vec<&str> = identifier.splitn(2, '.').collect();
        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            return Ok((Some(parts[0].to_string()), parts[1].to_string()));
        }
    }

    Ok((None, identifier.to_string()))
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
}
