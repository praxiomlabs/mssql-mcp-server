//! SQL Server metadata queries for schema introspection.

use crate::database::types::SqlValue;
use crate::database::{ConnectionPool, QueryExecutor, QueryResult, ResultRow};
use crate::error::McpError;
use serde::{Deserialize, Serialize};

/// Database metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseInfo {
    pub name: String,
    pub database_id: i32,
    pub create_date: String,
    pub collation_name: Option<String>,
    pub state_desc: String,
    pub recovery_model_desc: String,
    pub compatibility_level: i32,
}

/// Table metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub schema_name: String,
    pub table_name: String,
    pub table_type: String,
    pub row_count: Option<i64>,
    pub data_size_kb: Option<i64>,
    pub index_size_kb: Option<i64>,
}

/// Column metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub column_name: String,
    pub ordinal_position: i32,
    pub data_type: String,
    pub max_length: Option<i32>,
    pub precision: Option<i32>,
    pub scale: Option<i32>,
    pub is_nullable: bool,
    pub default_value: Option<String>,
    pub is_identity: bool,
    pub is_computed: bool,
}

/// View metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewInfo {
    pub schema_name: String,
    pub view_name: String,
    pub definition: Option<String>,
    pub is_updatable: bool,
}

/// Stored procedure metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedureInfo {
    pub schema_name: String,
    pub procedure_name: String,
    pub create_date: String,
    pub modify_date: String,
}

/// Stored procedure parameter metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedureParameter {
    pub parameter_name: String,
    pub ordinal_position: i32,
    pub data_type: String,
    pub max_length: Option<i32>,
    pub precision: Option<i32>,
    pub scale: Option<i32>,
    pub is_output: bool,
    pub has_default: bool,
    pub default_value: Option<String>,
}

/// Server information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub product_version: String,
    pub product_level: String,
    pub edition: String,
    pub engine_edition: i32,
    pub server_name: String,
    pub is_clustered: bool,
    pub collation: String,
}

/// Metadata query builder.
pub struct MetadataQueries {
    executor: QueryExecutor,
}

impl MetadataQueries {
    /// Create a new metadata query builder.
    pub fn new(pool: ConnectionPool, max_rows: usize) -> Self {
        Self {
            executor: QueryExecutor::new(pool, max_rows),
        }
    }

    /// Get server information.
    pub async fn get_server_info(&self) -> Result<ServerInfo, McpError> {
        let query = r#"
            SELECT
                SERVERPROPERTY('ProductVersion') AS product_version,
                SERVERPROPERTY('ProductLevel') AS product_level,
                SERVERPROPERTY('Edition') AS edition,
                SERVERPROPERTY('EngineEdition') AS engine_edition,
                @@SERVERNAME AS server_name,
                SERVERPROPERTY('IsClustered') AS is_clustered,
                SERVERPROPERTY('Collation') AS collation
        "#;

        let result = self.executor.execute(query).await?;

        if result.rows.is_empty() {
            return Err(McpError::internal("Failed to get server info"));
        }

        let row = &result.rows[0];

        Ok(ServerInfo {
            product_version: extract_string(row, "product_version").unwrap_or_default(),
            product_level: extract_string(row, "product_level").unwrap_or_default(),
            edition: extract_string(row, "edition").unwrap_or_default(),
            engine_edition: extract_i32(row, "engine_edition").unwrap_or(0),
            server_name: extract_string(row, "server_name").unwrap_or_default(),
            is_clustered: extract_bool(row, "is_clustered").unwrap_or(false),
            collation: extract_string(row, "collation").unwrap_or_default(),
        })
    }

    /// List all databases on the server.
    pub async fn list_databases(&self) -> Result<Vec<DatabaseInfo>, McpError> {
        let query = r#"
            SELECT
                name,
                database_id,
                CONVERT(VARCHAR(23), create_date, 121) AS create_date,
                collation_name,
                state_desc,
                recovery_model_desc,
                compatibility_level
            FROM sys.databases
            WHERE state_desc = 'ONLINE'
            ORDER BY name
        "#;

        let result = self.executor.execute(query).await?;

        Ok(result
            .rows
            .iter()
            .map(|row| DatabaseInfo {
                name: extract_string(row, "name").unwrap_or_default(),
                database_id: extract_i32(row, "database_id").unwrap_or(0),
                create_date: extract_string(row, "create_date").unwrap_or_default(),
                collation_name: extract_string(row, "collation_name"),
                state_desc: extract_string(row, "state_desc").unwrap_or_default(),
                recovery_model_desc: extract_string(row, "recovery_model_desc").unwrap_or_default(),
                compatibility_level: extract_i32(row, "compatibility_level").unwrap_or(0),
            })
            .collect())
    }

    /// List all schemas in the current database.
    pub async fn list_schemas(&self) -> Result<Vec<String>, McpError> {
        let query = r#"
            SELECT schema_name
            FROM INFORMATION_SCHEMA.SCHEMATA
            WHERE schema_name NOT IN ('guest', 'INFORMATION_SCHEMA', 'sys')
            ORDER BY schema_name
        "#;

        let result = self.executor.execute(query).await?;

        Ok(result
            .rows
            .iter()
            .filter_map(|row| extract_string(row, "schema_name"))
            .collect())
    }

    /// List tables in a schema.
    pub async fn list_tables(&self, schema: Option<&str>) -> Result<Vec<TableInfo>, McpError> {
        let query = format!(
            r#"
            SELECT
                s.name AS schema_name,
                t.name AS table_name,
                'TABLE' AS table_type,
                SUM(p.rows) AS row_count,
                SUM(a.data_pages) * 8 AS data_size_kb,
                SUM(a.used_pages - a.data_pages) * 8 AS index_size_kb
            FROM sys.tables t
            INNER JOIN sys.schemas s ON t.schema_id = s.schema_id
            INNER JOIN sys.indexes i ON t.object_id = i.object_id
            INNER JOIN sys.partitions p ON i.object_id = p.object_id AND i.index_id = p.index_id
            INNER JOIN sys.allocation_units a ON p.partition_id = a.container_id
            WHERE t.is_ms_shipped = 0
            {}
            GROUP BY s.name, t.name
            ORDER BY s.name, t.name
        "#,
            schema
                .map(|s| format!("AND s.name = '{}'", s.replace('\'', "''")))
                .unwrap_or_default()
        );

        let result = self.executor.execute(&query).await?;

        Ok(result
            .rows
            .iter()
            .map(|row| TableInfo {
                schema_name: extract_string(row, "schema_name").unwrap_or_default(),
                table_name: extract_string(row, "table_name").unwrap_or_default(),
                table_type: extract_string(row, "table_type").unwrap_or_default(),
                row_count: extract_i64(row, "row_count"),
                data_size_kb: extract_i64(row, "data_size_kb"),
                index_size_kb: extract_i64(row, "index_size_kb"),
            })
            .collect())
    }

    /// Get columns for a table.
    pub async fn get_table_columns(
        &self,
        schema: &str,
        table: &str,
    ) -> Result<Vec<ColumnInfo>, McpError> {
        let query = format!(
            r#"
            SELECT
                c.COLUMN_NAME AS column_name,
                c.ORDINAL_POSITION AS ordinal_position,
                c.DATA_TYPE AS data_type,
                c.CHARACTER_MAXIMUM_LENGTH AS max_length,
                c.NUMERIC_PRECISION AS precision,
                c.NUMERIC_SCALE AS scale,
                CASE WHEN c.IS_NULLABLE = 'YES' THEN 1 ELSE 0 END AS is_nullable,
                c.COLUMN_DEFAULT AS default_value,
                COLUMNPROPERTY(OBJECT_ID(c.TABLE_SCHEMA + '.' + c.TABLE_NAME), c.COLUMN_NAME, 'IsIdentity') AS is_identity,
                COLUMNPROPERTY(OBJECT_ID(c.TABLE_SCHEMA + '.' + c.TABLE_NAME), c.COLUMN_NAME, 'IsComputed') AS is_computed
            FROM INFORMATION_SCHEMA.COLUMNS c
            WHERE c.TABLE_SCHEMA = '{}'
            AND c.TABLE_NAME = '{}'
            ORDER BY c.ORDINAL_POSITION
        "#,
            schema.replace('\'', "''"),
            table.replace('\'', "''")
        );

        let result = self.executor.execute(&query).await?;

        Ok(result
            .rows
            .iter()
            .map(|row| ColumnInfo {
                column_name: extract_string(row, "column_name").unwrap_or_default(),
                ordinal_position: extract_i32(row, "ordinal_position").unwrap_or(0),
                data_type: extract_string(row, "data_type").unwrap_or_default(),
                max_length: extract_i32(row, "max_length"),
                precision: extract_i32(row, "precision"),
                scale: extract_i32(row, "scale"),
                is_nullable: extract_bool(row, "is_nullable").unwrap_or(true),
                default_value: extract_string(row, "default_value"),
                is_identity: extract_bool(row, "is_identity").unwrap_or(false),
                is_computed: extract_bool(row, "is_computed").unwrap_or(false),
            })
            .collect())
    }

    /// List views in a schema.
    pub async fn list_views(&self, schema: Option<&str>) -> Result<Vec<ViewInfo>, McpError> {
        let query = format!(
            r#"
            SELECT
                s.name AS schema_name,
                v.name AS view_name,
                m.definition AS definition,
                OBJECTPROPERTY(v.object_id, 'IsUpdatable') AS is_updatable
            FROM sys.views v
            INNER JOIN sys.schemas s ON v.schema_id = s.schema_id
            LEFT JOIN sys.sql_modules m ON v.object_id = m.object_id
            WHERE v.is_ms_shipped = 0
            {}
            ORDER BY s.name, v.name
        "#,
            schema
                .map(|s| format!("AND s.name = '{}'", s.replace('\'', "''")))
                .unwrap_or_default()
        );

        let result = self.executor.execute(&query).await?;

        Ok(result
            .rows
            .iter()
            .map(|row| ViewInfo {
                schema_name: extract_string(row, "schema_name").unwrap_or_default(),
                view_name: extract_string(row, "view_name").unwrap_or_default(),
                definition: extract_string(row, "definition"),
                is_updatable: extract_bool(row, "is_updatable").unwrap_or(false),
            })
            .collect())
    }

    /// List stored procedures in a schema.
    pub async fn list_procedures(
        &self,
        schema: Option<&str>,
    ) -> Result<Vec<ProcedureInfo>, McpError> {
        let query = format!(
            r#"
            SELECT
                s.name AS schema_name,
                p.name AS procedure_name,
                CONVERT(VARCHAR(23), p.create_date, 121) AS create_date,
                CONVERT(VARCHAR(23), p.modify_date, 121) AS modify_date
            FROM sys.procedures p
            INNER JOIN sys.schemas s ON p.schema_id = s.schema_id
            WHERE p.is_ms_shipped = 0
            {}
            ORDER BY s.name, p.name
        "#,
            schema
                .map(|s| format!("AND s.name = '{}'", s.replace('\'', "''")))
                .unwrap_or_default()
        );

        let result = self.executor.execute(&query).await?;

        Ok(result
            .rows
            .iter()
            .map(|row| ProcedureInfo {
                schema_name: extract_string(row, "schema_name").unwrap_or_default(),
                procedure_name: extract_string(row, "procedure_name").unwrap_or_default(),
                create_date: extract_string(row, "create_date").unwrap_or_default(),
                modify_date: extract_string(row, "modify_date").unwrap_or_default(),
            })
            .collect())
    }

    /// Get procedure definition.
    pub async fn get_procedure_definition(
        &self,
        schema: &str,
        procedure: &str,
    ) -> Result<Option<String>, McpError> {
        let query = format!(
            r#"
            SELECT m.definition
            FROM sys.procedures p
            INNER JOIN sys.schemas s ON p.schema_id = s.schema_id
            INNER JOIN sys.sql_modules m ON p.object_id = m.object_id
            WHERE s.name = '{}'
            AND p.name = '{}'
        "#,
            schema.replace('\'', "''"),
            procedure.replace('\'', "''")
        );

        let result = self.executor.execute(&query).await?;

        if result.rows.is_empty() {
            return Ok(None);
        }

        Ok(extract_string(&result.rows[0], "definition"))
    }

    /// Get procedure parameters.
    pub async fn get_procedure_parameters(
        &self,
        schema: &str,
        procedure: &str,
    ) -> Result<Vec<ProcedureParameter>, McpError> {
        let query = format!(
            r#"
            SELECT
                par.name AS parameter_name,
                par.parameter_id AS ordinal_position,
                TYPE_NAME(par.user_type_id) AS data_type,
                par.max_length AS max_length,
                par.precision AS precision,
                par.scale AS scale,
                par.is_output AS is_output,
                par.has_default_value AS has_default,
                par.default_value AS default_value
            FROM sys.parameters par
            INNER JOIN sys.procedures p ON par.object_id = p.object_id
            INNER JOIN sys.schemas s ON p.schema_id = s.schema_id
            WHERE s.name = '{}'
            AND p.name = '{}'
            ORDER BY par.parameter_id
        "#,
            schema.replace('\'', "''"),
            procedure.replace('\'', "''")
        );

        let result = self.executor.execute(&query).await?;

        Ok(result
            .rows
            .iter()
            .map(|row| ProcedureParameter {
                parameter_name: extract_string(row, "parameter_name").unwrap_or_default(),
                ordinal_position: extract_i32(row, "ordinal_position").unwrap_or(0),
                data_type: extract_string(row, "data_type").unwrap_or_default(),
                max_length: extract_i32(row, "max_length"),
                precision: extract_i32(row, "precision"),
                scale: extract_i32(row, "scale"),
                is_output: extract_bool(row, "is_output").unwrap_or(false),
                has_default: extract_bool(row, "has_default").unwrap_or(false),
                default_value: extract_string(row, "default_value"),
            })
            .collect())
    }

    /// Execute a raw query (passthrough for QueryExecutor).
    pub async fn execute_query(&self, query: &str) -> Result<QueryResult, McpError> {
        self.executor.execute(query).await
    }
}

// Helper functions to extract values from result rows
fn extract_string(row: &ResultRow, column: &str) -> Option<String> {
    match row.get(column)? {
        SqlValue::String(s) => Some(s.clone()),
        SqlValue::Null => None,
        other => Some(other.to_display_string()),
    }
}

fn extract_i32(row: &ResultRow, column: &str) -> Option<i32> {
    match row.get(column)? {
        SqlValue::I32(v) => Some(*v),
        SqlValue::I16(v) => Some(*v as i32),
        SqlValue::I8(v) => Some(*v as i32),
        SqlValue::I64(v) => Some(*v as i32),
        _ => None,
    }
}

fn extract_i64(row: &ResultRow, column: &str) -> Option<i64> {
    match row.get(column)? {
        SqlValue::I64(v) => Some(*v),
        SqlValue::I32(v) => Some(*v as i64),
        SqlValue::I16(v) => Some(*v as i64),
        SqlValue::I8(v) => Some(*v as i64),
        _ => None,
    }
}

fn extract_bool(row: &ResultRow, column: &str) -> Option<bool> {
    match row.get(column)? {
        SqlValue::Bool(v) => Some(*v),
        SqlValue::I32(v) => Some(*v != 0),
        SqlValue::I16(v) => Some(*v != 0),
        SqlValue::I8(v) => Some(*v != 0),
        SqlValue::I64(v) => Some(*v != 0),
        _ => None,
    }
}
