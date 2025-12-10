//! SQL injection detection.
//!
//! Detects common SQL injection patterns in queries.

use crate::error::McpError;
use once_cell::sync::Lazy;
use regex::Regex;

/// Compiled regex patterns for SQL injection detection.
///
/// These patterns are compiled once at first use (lazy static) for performance.
/// All patterns are hardcoded constants that have been verified to be valid regex.
static INJECTION_PATTERNS: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    // Helper to compile regex with better error message on failure.
    // These should never fail since patterns are hardcoded and tested.
    fn compile(pattern: &str) -> Regex {
        Regex::new(pattern).unwrap_or_else(|e| {
            panic!("Internal error: invalid regex pattern '{}': {}", pattern, e)
        })
    }

    vec![
        // Comment injection
        (compile(r"--\s*$"), "SQL line comment at end of input"),
        (compile(r"/\*.*\*/"), "SQL block comment"),
        // Union-based injection
        (compile(r"(?i)\bUNION\s+(ALL\s+)?SELECT\b"), "UNION SELECT injection"),
        // OR-based injection (tautology)
        (compile(r"(?i)'\s*OR\s+'[^']*'\s*=\s*'"), "OR tautology injection"),
        (compile(r"(?i)'\s*OR\s+1\s*=\s*1"), "OR 1=1 injection (string context)"),
        (compile(r"(?i)\bOR\s+1\s*=\s*1\b"), "OR 1=1 injection"),
        // AND-based injection
        (compile(r"(?i)'\s*AND\s+'[^']*'\s*=\s*'"), "AND tautology injection"),
        // Stacked queries (multiple statements)
        (compile(r";\s*(?i)(SELECT|INSERT|UPDATE|DELETE|DROP|EXEC|EXECUTE|CREATE|ALTER|TRUNCATE)\b"), "Stacked query injection"),
        // Time-based blind injection
        (compile(r"(?i)\bWAITFOR\s+DELAY\b"), "Time-based blind injection (WAITFOR)"),
        // Extended stored procedures (common attack vectors)
        (compile(r"(?i)\bxp_cmdshell\b"), "xp_cmdshell execution attempt"),
        (compile(r"(?i)\bxp_reg\w+\b"), "Registry access attempt"),
        (compile(r"(?i)\bsp_oacreate\b"), "OLE automation attempt"),
        // Information disclosure
        (compile(r"(?i)\bINFORMATION_SCHEMA\b.*\bWHERE\b.*="), "Schema enumeration with filter"),
        // Hex-encoded injection
        (compile(r"0x[0-9a-fA-F]{10,}"), "Long hex-encoded string"),
        // CHAR() obfuscation
        (compile(r"(?i)CHAR\s*\(\s*\d+\s*\)(\s*\+\s*CHAR\s*\(\s*\d+\s*\)){3,}"), "CHAR() obfuscation"),
    ]
});

/// SQL injection detector.
#[derive(Debug, Clone, Default)]
pub struct InjectionDetector {
    /// Whether detection is enabled
    enabled: bool,
}

impl InjectionDetector {
    /// Create a new injection detector.
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Check a query for SQL injection patterns.
    ///
    /// Returns `Ok(())` if no injection is detected, or an error describing
    /// the detected pattern.
    pub fn check(&self, query: &str) -> Result<(), McpError> {
        if !self.enabled {
            return Ok(());
        }

        for (pattern, description) in INJECTION_PATTERNS.iter() {
            if pattern.is_match(query) {
                return Err(McpError::injection(*description));
            }
        }

        Ok(())
    }

    /// Check if a string value might contain injection.
    ///
    /// This is for checking parameter values, not full queries.
    pub fn check_value(&self, value: &str) -> Result<(), McpError> {
        if !self.enabled {
            return Ok(());
        }

        // Check for obvious injection attempts in values
        let dangerous_patterns = [
            ("'--", "Comment injection in value"),
            ("';", "Statement terminator in value"),
            ("' OR ", "OR injection in value"),
            ("' AND ", "AND injection in value"),
            ("UNION SELECT", "UNION injection in value"),
        ];

        let upper = value.to_uppercase();
        for (pattern, description) in dangerous_patterns {
            if upper.contains(pattern) {
                return Err(McpError::injection(description));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detector() -> InjectionDetector {
        InjectionDetector::new(true)
    }

    #[test]
    fn test_clean_query() {
        let d = detector();
        assert!(d.check("SELECT * FROM Users WHERE id = @id").is_ok());
        assert!(d.check("SELECT name, email FROM Users").is_ok());
    }

    #[test]
    fn test_union_injection() {
        let d = detector();
        assert!(d
            .check("SELECT * FROM Users WHERE id = 1 UNION SELECT * FROM Passwords")
            .is_err());
        assert!(d
            .check("SELECT * FROM Users UNION ALL SELECT * FROM Admin")
            .is_err());
    }

    #[test]
    fn test_or_injection() {
        let d = detector();
        assert!(d.check("SELECT * FROM Users WHERE name = '' OR '1'='1'").is_err());
        assert!(d.check("SELECT * FROM Users WHERE id = 1 OR 1=1").is_err());
    }

    #[test]
    fn test_stacked_queries() {
        let d = detector();
        assert!(d.check("SELECT * FROM Users; DROP TABLE Users").is_err());
        assert!(d
            .check("SELECT * FROM Users; DELETE FROM Users")
            .is_err());
    }

    #[test]
    fn test_xp_cmdshell() {
        let d = detector();
        assert!(d.check("EXEC xp_cmdshell 'dir'").is_err());
        assert!(d.check("EXECUTE xp_cmdshell 'whoami'").is_err());
    }

    #[test]
    fn test_waitfor_injection() {
        let d = detector();
        assert!(d.check("SELECT * FROM Users; WAITFOR DELAY '0:0:5'").is_err());
    }

    #[test]
    fn test_disabled_detector() {
        let d = InjectionDetector::new(false);
        // Should not detect anything when disabled
        assert!(d.check("SELECT * FROM Users; DROP TABLE Users").is_ok());
    }

    #[test]
    fn test_value_injection() {
        let d = detector();
        assert!(d.check_value("normal value").is_ok());
        assert!(d.check_value("John's value").is_ok()); // Single quote alone is fine
        assert!(d.check_value("value'--").is_err());
        assert!(d.check_value("value'; DROP TABLE").is_err());
    }
}
