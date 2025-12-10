//! SQL Server type mapping to Rust types.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tiberius::Row;
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
    /// Extract a value from a Tiberius row column.
    pub fn extract_column(row: &Row, idx: usize) -> SqlValue {
        let col = row.columns().get(idx);
        if col.is_none() {
            return SqlValue::Null;
        }

        // Try each type in order of likelihood
        // Strings (most common)
        if let Some(v) = row.try_get::<&str, _>(idx).ok().flatten() {
            return SqlValue::String(v.to_string());
        }

        // Integers
        if let Some(v) = row.try_get::<i32, _>(idx).ok().flatten() {
            return SqlValue::I32(v);
        }
        if let Some(v) = row.try_get::<i64, _>(idx).ok().flatten() {
            return SqlValue::I64(v);
        }
        if let Some(v) = row.try_get::<i16, _>(idx).ok().flatten() {
            return SqlValue::I16(v);
        }
        // Note: i8/TINYINT is handled as u8 in tiberius
        if let Some(v) = row.try_get::<u8, _>(idx).ok().flatten() {
            return SqlValue::I8(v as i8);
        }

        // Floating point
        if let Some(v) = row.try_get::<f64, _>(idx).ok().flatten() {
            return SqlValue::F64(v);
        }
        if let Some(v) = row.try_get::<f32, _>(idx).ok().flatten() {
            return SqlValue::F32(v);
        }

        // Decimal
        if let Some(v) = row.try_get::<Decimal, _>(idx).ok().flatten() {
            return SqlValue::Decimal(v);
        }

        // Boolean
        if let Some(v) = row.try_get::<bool, _>(idx).ok().flatten() {
            return SqlValue::Bool(v);
        }

        // UUID
        if let Some(v) = row.try_get::<Uuid, _>(idx).ok().flatten() {
            return SqlValue::Uuid(v);
        }

        // Date/Time types
        if let Some(v) = row.try_get::<NaiveDateTime, _>(idx).ok().flatten() {
            return SqlValue::DateTime(v);
        }
        if let Some(v) = row.try_get::<NaiveDate, _>(idx).ok().flatten() {
            return SqlValue::Date(v);
        }
        if let Some(v) = row.try_get::<NaiveTime, _>(idx).ok().flatten() {
            return SqlValue::Time(v);
        }

        // Binary
        if let Some(v) = row.try_get::<&[u8], _>(idx).ok().flatten() {
            return SqlValue::Bytes(v.to_vec());
        }

        // Fall back to NULL for unsupported types
        SqlValue::Null
    }

    /// Get the SQL type name for a column.
    pub fn sql_type_name(col: &tiberius::Column) -> &'static str {
        use tiberius::ColumnType;

        match col.column_type() {
            ColumnType::Null => "NULL",
            ColumnType::Int1 => "TINYINT",
            ColumnType::Int2 => "SMALLINT",
            ColumnType::Int4 => "INT",
            ColumnType::Int8 => "BIGINT",
            ColumnType::Float4 => "REAL",
            ColumnType::Float8 => "FLOAT",
            ColumnType::Money => "MONEY",
            ColumnType::Money4 => "SMALLMONEY",
            ColumnType::Datetime => "DATETIME",
            ColumnType::Datetime4 => "SMALLDATETIME",
            ColumnType::Bit => "BIT",
            ColumnType::Guid => "UNIQUEIDENTIFIER",
            ColumnType::Decimaln => "DECIMAL",
            ColumnType::Numericn => "NUMERIC",
            ColumnType::Bitn => "BIT",
            ColumnType::Intn => "INT",
            ColumnType::Floatn => "FLOAT",
            ColumnType::Datetimen => "DATETIME",
            ColumnType::Daten => "DATE",
            ColumnType::Timen => "TIME",
            ColumnType::Datetime2 => "DATETIME2",
            ColumnType::DatetimeOffsetn => "DATETIMEOFFSET",
            ColumnType::BigVarBin => "VARBINARY",
            ColumnType::BigVarChar => "VARCHAR",
            ColumnType::BigBinary => "BINARY",
            ColumnType::BigChar => "CHAR",
            ColumnType::NVarchar => "NVARCHAR",
            ColumnType::NChar => "NCHAR",
            ColumnType::Xml => "XML",
            ColumnType::Text => "TEXT",
            ColumnType::Image => "IMAGE",
            ColumnType::NText => "NTEXT",
            ColumnType::SSVariant => "SQL_VARIANT",
            _ => "UNKNOWN",
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
}
