//! SQL Server type mapping to Rust types.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use mssql_client::Row;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A SQL value that can be serialized to JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SqlValue {
    Null,
    Bool(bool),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    String(String),
    Bytes(Vec<u8>),
    Decimal(Decimal),
    Uuid(Uuid),
    Date(NaiveDate),
    Time(NaiveTime),
    DateTime(NaiveDateTime),
    DateTimeUtc(DateTime<Utc>),
}

impl SqlValue {
    /// Check if this value is null.
    pub fn is_null(&self) -> bool {
        matches!(self, SqlValue::Null)
    }

    /// Convert to a display string.
    pub fn to_display_string(&self) -> String {
        match self {
            SqlValue::Null => "NULL".to_string(),
            SqlValue::Bool(v) => v.to_string(),
            SqlValue::I8(v) => v.to_string(),
            SqlValue::I16(v) => v.to_string(),
            SqlValue::I32(v) => v.to_string(),
            SqlValue::I64(v) => v.to_string(),
            SqlValue::F32(v) => v.to_string(),
            SqlValue::F64(v) => v.to_string(),
            SqlValue::String(v) => v.clone(),
            SqlValue::Bytes(v) => format!("0x{}", hex::encode(v)),
            SqlValue::Decimal(v) => v.to_string(),
            SqlValue::Uuid(v) => v.to_string(),
            SqlValue::Date(v) => v.to_string(),
            SqlValue::Time(v) => v.to_string(),
            SqlValue::DateTime(v) => v.to_string(),
            SqlValue::DateTimeUtc(v) => v.to_rfc3339(),
        }
    }
}

/// Type mapper for converting SQL Server types to Rust types.
pub struct TypeMapper;

impl TypeMapper {
    /// Extract a value from a row column.
    ///
    /// This method attempts to extract values by trying different types
    /// in order of likelihood, returning the first successful conversion.
    /// The try_get method returns None if the value is NULL or if the type
    /// conversion fails.
    pub fn extract_column(row: &Row, idx: usize) -> SqlValue {
        // Check if the column is null first
        if row.is_null(idx) {
            return SqlValue::Null;
        }

        // Try each type in order of likelihood
        // Strings (most common)
        if let Some(v) = row.try_get::<String>(idx) {
            return SqlValue::String(v);
        }

        // Integers
        if let Some(v) = row.try_get::<i32>(idx) {
            return SqlValue::I32(v);
        }
        if let Some(v) = row.try_get::<i64>(idx) {
            return SqlValue::I64(v);
        }
        if let Some(v) = row.try_get::<i16>(idx) {
            return SqlValue::I16(v);
        }
        // Note: i8/TINYINT is typically handled as u8
        if let Some(v) = row.try_get::<u8>(idx) {
            return SqlValue::I8(v as i8);
        }

        // Floating point
        if let Some(v) = row.try_get::<f64>(idx) {
            return SqlValue::F64(v);
        }
        if let Some(v) = row.try_get::<f32>(idx) {
            return SqlValue::F32(v);
        }

        // Decimal
        if let Some(v) = row.try_get::<Decimal>(idx) {
            return SqlValue::Decimal(v);
        }

        // Boolean
        if let Some(v) = row.try_get::<bool>(idx) {
            return SqlValue::Bool(v);
        }

        // UUID
        if let Some(v) = row.try_get::<Uuid>(idx) {
            return SqlValue::Uuid(v);
        }

        // Date/Time types
        if let Some(v) = row.try_get::<NaiveDateTime>(idx) {
            return SqlValue::DateTime(v);
        }
        if let Some(v) = row.try_get::<NaiveDate>(idx) {
            return SqlValue::Date(v);
        }
        if let Some(v) = row.try_get::<NaiveTime>(idx) {
            return SqlValue::Time(v);
        }

        // Binary
        if let Some(v) = row.try_get::<Vec<u8>>(idx) {
            return SqlValue::Bytes(v);
        }

        // Fall back to NULL for unsupported types
        SqlValue::Null
    }

    /// Get the SQL type name for a column based on the value.
    ///
    /// Note: This is a best-effort type detection based on the extracted value.
    /// For precise type information, query the column metadata from SQL Server.
    pub fn sql_type_name_from_value(value: &SqlValue) -> &'static str {
        match value {
            SqlValue::Null => "NULL",
            SqlValue::Bool(_) => "BIT",
            SqlValue::I8(_) => "TINYINT",
            SqlValue::I16(_) => "SMALLINT",
            SqlValue::I32(_) => "INT",
            SqlValue::I64(_) => "BIGINT",
            SqlValue::F32(_) => "REAL",
            SqlValue::F64(_) => "FLOAT",
            SqlValue::String(_) => "NVARCHAR",
            SqlValue::Bytes(_) => "VARBINARY",
            SqlValue::Decimal(_) => "DECIMAL",
            SqlValue::Uuid(_) => "UNIQUEIDENTIFIER",
            SqlValue::Date(_) => "DATE",
            SqlValue::Time(_) => "TIME",
            SqlValue::DateTime(_) => "DATETIME2",
            SqlValue::DateTimeUtc(_) => "DATETIMEOFFSET",
        }
    }
}

/// Hex encoding helper (minimal implementation to avoid extra dependency).
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02X}", b)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sql_value_display() {
        assert_eq!(SqlValue::Null.to_display_string(), "NULL");
        assert_eq!(SqlValue::I32(42).to_display_string(), "42");
        assert_eq!(
            SqlValue::String("hello".to_string()).to_display_string(),
            "hello"
        );
        assert_eq!(SqlValue::Bool(true).to_display_string(), "true");
    }

    #[test]
    fn test_sql_value_is_null() {
        assert!(SqlValue::Null.is_null());
        assert!(!SqlValue::I32(0).is_null());
    }

    #[test]
    fn test_hex_encode() {
        assert_eq!(hex::encode(&[0xDE, 0xAD, 0xBE, 0xEF]), "DEADBEEF");
        assert_eq!(hex::encode(&[]), "");
    }

    #[test]
    fn test_sql_type_name_from_value() {
        assert_eq!(
            TypeMapper::sql_type_name_from_value(&SqlValue::Null),
            "NULL"
        );
        assert_eq!(
            TypeMapper::sql_type_name_from_value(&SqlValue::I32(42)),
            "INT"
        );
        assert_eq!(
            TypeMapper::sql_type_name_from_value(&SqlValue::String("test".to_string())),
            "NVARCHAR"
        );
    }
}
