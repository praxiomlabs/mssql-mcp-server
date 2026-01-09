//! Native Bulk Copy Program (BCP) support for high-performance data loading.
//!
//! This module provides the infrastructure for native BCP protocol support.
//!
//! **Current Status**: The mssql-client library (v0.5.2) has BCP packet building
//! infrastructure but doesn't yet expose a `Client.bulk_insert()` method. Until
//! that's available, this module provides the options structure for future
//! compatibility while falling back to batched INSERT statements.
//!
//! When native BCP is fully supported, it will offer:
//! - Direct TDS bulk load protocol (packet type 0x07)
//! - Minimal logging (with simple recovery model)
//! - Batch commits to reduce lock contention
//! - Optional table lock for maximum throughput

use crate::config::DatabaseConfig;
use std::sync::Arc;

/// Options for native bulk insert operations.
///
/// These options map to SQL Server's BULK INSERT hints. Currently used
/// for future compatibility - actual BCP support pending mssql-client
/// library updates.
#[derive(Debug, Clone)]
pub struct NativeBulkOptions {
    /// Number of rows per batch.
    pub batch_size: usize,
    /// Acquire table lock during insert (improves performance, blocks other access).
    pub table_lock: bool,
    /// Fire INSERT triggers on the target table.
    pub fire_triggers: bool,
    /// Check constraints during insert.
    pub check_constraints: bool,
    /// Preserve NULL values (if false, NULLs replaced with column defaults).
    pub keep_nulls: bool,
    /// Preserve identity values from source data.
    pub keep_identity: bool,
}

impl Default for NativeBulkOptions {
    fn default() -> Self {
        Self {
            batch_size: 1000,
            table_lock: false,
            fire_triggers: false,
            check_constraints: true,
            keep_nulls: true,
            keep_identity: false,
        }
    }
}

/// Result of a native bulk insert operation.
#[derive(Debug, Clone)]
pub struct NativeBulkResult {
    /// Total rows inserted.
    pub rows_inserted: u64,
    /// Execution time in milliseconds.
    pub execution_time_ms: u64,
    /// Whether the operation completed successfully.
    pub success: bool,
    /// Error message if the operation failed.
    pub error: Option<String>,
    /// The method used for insertion.
    pub method: BulkInsertMethod,
}

/// The method used for bulk insertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkInsertMethod {
    /// Native TDS BCP protocol (not yet available).
    NativeBcp,
    /// Batched INSERT statements (current fallback).
    InsertStatements,
}

impl std::fmt::Display for BulkInsertMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BulkInsertMethod::NativeBcp => write!(f, "native_bcp"),
            BulkInsertMethod::InsertStatements => write!(f, "insert_statements"),
        }
    }
}

/// Bulk insert manager for native BCP operations.
///
/// **Note**: Native BCP is not yet supported by the mssql-client library.
/// The `bulk_insert` method will return an error indicating that native
/// BCP is not available, allowing the caller to fall back to INSERT statements.
#[allow(dead_code)]
pub struct BulkInsertManager {
    db_config: Arc<DatabaseConfig>,
}

impl BulkInsertManager {
    /// Create a new bulk insert manager.
    pub fn new(db_config: Arc<DatabaseConfig>) -> Self {
        Self { db_config }
    }

    /// Check if native BCP is available.
    ///
    /// Currently always returns `false` as mssql-client v0.5.2 doesn't
    /// expose the Client.bulk_insert() method yet.
    pub fn is_native_bcp_available(&self) -> bool {
        // Native BCP requires mssql-client to expose Client.bulk_insert()
        // This is tracked as a future enhancement
        false
    }

    /// Get the database configuration.
    #[allow(dead_code)]
    pub fn db_config(&self) -> &DatabaseConfig {
        &self.db_config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_options() {
        let opts = NativeBulkOptions::default();
        assert_eq!(opts.batch_size, 1000);
        assert!(!opts.table_lock);
        assert!(!opts.fire_triggers);
        assert!(opts.check_constraints);
        assert!(opts.keep_nulls);
        assert!(!opts.keep_identity);
    }

    #[test]
    fn test_bulk_insert_method_display() {
        assert_eq!(BulkInsertMethod::NativeBcp.to_string(), "native_bcp");
        assert_eq!(
            BulkInsertMethod::InsertStatements.to_string(),
            "insert_statements"
        );
    }
}
