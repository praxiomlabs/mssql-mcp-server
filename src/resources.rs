//! MCP Resources for SQL Server metadata.
//!
//! Resources provide passive read-only access to database schema information.
//! Following MCP semantics, resources are like GET endpoints - they don't modify data.
//!
//! ## URI Scheme
//!
//! Resources use the `mssql://` URI scheme:
//!
//! - `mssql://server/info` - Server information
//! - `mssql://databases` - List of databases
//! - `mssql://schemas` - List of schemas in current database
//! - `mssql://tables` - List of tables
//! - `mssql://tables/{schema}/{table}` - Table details with columns
//! - `mssql://views` - List of views
//! - `mssql://views/{schema}/{view}` - View details with definition
//! - `mssql://procedures` - List of stored procedures
//! - `mssql://procedures/{schema}/{procedure}` - Procedure details with parameters
//! - `mssql://functions` - List of user-defined functions
//! - `mssql://functions/{schema}/{function}` - Function details with parameters
//! - `mssql://triggers` - List of database triggers
//! - `mssql://triggers/{schema}/{trigger}` - Trigger details with definition

use crate::security::{parse_qualified_name, validate_identifier};
use crate::server::MssqlMcpServer;
use rmcp::model::{
    AnnotateAble, RawResource, RawResourceTemplate, ReadResourceResult, Resource, ResourceContents,
    ResourceTemplate,
};
use serde_json::json;

/// Build the list of available resources based on server state.
pub fn build_resource_list(server: &MssqlMcpServer) -> Vec<Resource> {
    let mut resources = vec![
        create_resource(
            "mssql://server/info",
            "Server Information",
            "SQL Server version, edition, and configuration details",
            "application/json",
        ),
        create_resource(
            "mssql://databases",
            "Databases",
            "List of all databases on the server",
            "application/json",
        ),
    ];

    // If in database mode, add database-specific resources
    if server.is_database_mode() {
        resources.extend(vec![
            create_resource(
                "mssql://schemas",
                "Schemas",
                "List of schemas in the current database",
                "application/json",
            ),
            create_resource(
                "mssql://tables",
                "Tables",
                "List of all tables with row counts and sizes",
                "application/json",
            ),
            create_resource(
                "mssql://views",
                "Views",
                "List of all views in the database",
                "application/json",
            ),
            create_resource(
                "mssql://procedures",
                "Stored Procedures",
                "List of stored procedures",
                "application/json",
            ),
            create_resource(
                "mssql://functions",
                "Functions",
                "List of user-defined functions (scalar and table-valued)",
                "application/json",
            ),
            create_resource(
                "mssql://triggers",
                "Triggers",
                "List of database triggers",
                "application/json",
            ),
        ]);
    }

    resources
}

/// Build resource templates for dynamic resources.
pub fn build_resource_templates(server: &MssqlMcpServer) -> Vec<ResourceTemplate> {
    let mut templates = Vec::new();

    // Only add templates if in database mode
    if server.is_database_mode() {
        templates.extend(vec![
            create_resource_template(
                "mssql://tables/{schema}/{table}",
                "Table Details",
                "Get detailed information about a specific table",
                "application/json",
            ),
            create_resource_template(
                "mssql://views/{schema}/{view}",
                "View Details",
                "Get detailed information about a specific view",
                "application/json",
            ),
            create_resource_template(
                "mssql://procedures/{schema}/{procedure}",
                "Procedure Details",
                "Get detailed information about a stored procedure",
                "application/json",
            ),
            create_resource_template(
                "mssql://functions/{schema}/{function}",
                "Function Details",
                "Get detailed information about a user-defined function",
                "application/json",
            ),
            create_resource_template(
                "mssql://triggers/{schema}/{trigger}",
                "Trigger Details",
                "Get detailed information about a trigger",
                "application/json",
            ),
        ]);
    }

    templates
}

/// Read a resource by URI.
pub async fn read_resource(
    server: &MssqlMcpServer,
    uri: &str,
) -> Result<ReadResourceResult, String> {
    // Parse the URI and dispatch to appropriate handler
    let parsed = parse_resource_uri(uri).map_err(|e| e.to_string())?;

    let content = match parsed {
        ResourceUri::ServerInfo => read_server_info(server).await?,
        ResourceUri::Databases => read_databases(server).await?,
        ResourceUri::Schemas => read_schemas(server).await?,
        ResourceUri::Tables => read_tables(server).await?,
        ResourceUri::TableDetails { schema, table } => {
            read_table_details(server, &schema, &table).await?
        }
        ResourceUri::Views => read_views(server).await?,
        ResourceUri::ViewDetails { schema, view } => {
            read_view_details(server, &schema, &view).await?
        }
        ResourceUri::Procedures => read_procedures(server).await?,
        ResourceUri::ProcedureDetails { schema, procedure } => {
            read_procedure_details(server, &schema, &procedure).await?
        }
        ResourceUri::Functions => read_functions(server).await?,
        ResourceUri::FunctionDetails { schema, function } => {
            read_function_details(server, &schema, &function).await?
        }
        ResourceUri::Triggers => read_triggers(server).await?,
        ResourceUri::TriggerDetails { schema, trigger } => {
            read_trigger_details(server, &schema, &trigger).await?
        }
    };

    Ok(ReadResourceResult {
        contents: vec![ResourceContents::text(content, uri.to_string())],
    })
}

// =========================================================================
// Resource URI Parsing
// =========================================================================

/// Parsed resource URI variants.
#[derive(Debug)]
enum ResourceUri {
    ServerInfo,
    Databases,
    Schemas,
    Tables,
    TableDetails { schema: String, table: String },
    Views,
    ViewDetails { schema: String, view: String },
    Procedures,
    ProcedureDetails { schema: String, procedure: String },
    Functions,
    FunctionDetails { schema: String, function: String },
    Triggers,
    TriggerDetails { schema: String, trigger: String },
}

/// Error type for resource URI parsing with detailed context.
#[derive(Debug)]
struct ResourceParseError {
    uri: String,
    reason: ParseErrorReason,
}

/// Specific reasons why a resource URI parse failed.
#[derive(Debug)]
enum ParseErrorReason {
    /// URI does not start with mssql:// scheme
    InvalidScheme,
    /// URI path is empty or contains no segments
    EmptyPath,
    /// Unknown resource type (first segment not recognized)
    UnknownResourceType { segment: String },
    /// Invalid identifier in path (contains SQL injection patterns, etc.)
    InvalidIdentifier { identifier: String, reason: String },
    /// Missing required path component
    MissingComponent { expected: &'static str },
    /// Too many path segments
    TooManySegments { expected: usize, got: usize },
}

impl std::fmt::Display for ResourceParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid resource URI '{}': ", self.uri)?;
        match &self.reason {
            ParseErrorReason::InvalidScheme => {
                write!(f, "URI must start with 'mssql://' scheme")
            }
            ParseErrorReason::EmptyPath => {
                write!(f, "URI path is empty")
            }
            ParseErrorReason::UnknownResourceType { segment } => {
                write!(
                    f,
                    "unknown resource type '{}'. Valid types: server, databases, schemas, tables, views, procedures, functions, triggers",
                    segment
                )
            }
            ParseErrorReason::InvalidIdentifier { identifier, reason } => {
                write!(
                    f,
                    "invalid identifier '{}': {}",
                    identifier, reason
                )
            }
            ParseErrorReason::MissingComponent { expected } => {
                write!(f, "missing required component: {}", expected)
            }
            ParseErrorReason::TooManySegments { expected, got } => {
                write!(
                    f,
                    "too many path segments (expected {}, got {})",
                    expected, got
                )
            }
        }
    }
}

impl std::error::Error for ResourceParseError {}

fn parse_resource_uri(uri: &str) -> Result<ResourceUri, ResourceParseError> {
    let path = match uri.strip_prefix("mssql://") {
        Some(p) => p,
        None => {
            return Err(ResourceParseError {
                uri: uri.to_string(),
                reason: ParseErrorReason::InvalidScheme,
            });
        }
    };

    // Split path into segments
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if segments.is_empty() {
        return Err(ResourceParseError {
            uri: uri.to_string(),
            reason: ParseErrorReason::EmptyPath,
        });
    }

    match segments.as_slice() {
        ["server", "info"] => Ok(ResourceUri::ServerInfo),
        ["server", ..] => Err(ResourceParseError {
            uri: uri.to_string(),
            reason: ParseErrorReason::MissingComponent {
                expected: "info (use mssql://server/info)",
            },
        }),
        ["databases"] => Ok(ResourceUri::Databases),
        ["schemas"] => Ok(ResourceUri::Schemas),
        ["tables"] => Ok(ResourceUri::Tables),
        // Support both mssql://tables/schema/table and mssql://tables/schema.table formats
        ["tables", qualified] => {
            parse_and_validate_qualified_name(qualified, uri)
                .map(|(schema, table)| ResourceUri::TableDetails { schema, table })
        }
        ["tables", schema, table] => {
            validate_schema_and_name(schema, table, uri)
                .map(|(s, t)| ResourceUri::TableDetails { schema: s, table: t })
        }
        ["tables", ..] => Err(ResourceParseError {
            uri: uri.to_string(),
            reason: ParseErrorReason::TooManySegments {
                expected: 3,
                got: segments.len(),
            },
        }),
        ["views"] => Ok(ResourceUri::Views),
        ["views", qualified] => {
            parse_and_validate_qualified_name(qualified, uri)
                .map(|(schema, view)| ResourceUri::ViewDetails { schema, view })
        }
        ["views", schema, view] => {
            validate_schema_and_name(schema, view, uri)
                .map(|(s, v)| ResourceUri::ViewDetails { schema: s, view: v })
        }
        ["views", ..] => Err(ResourceParseError {
            uri: uri.to_string(),
            reason: ParseErrorReason::TooManySegments {
                expected: 3,
                got: segments.len(),
            },
        }),
        ["procedures"] => Ok(ResourceUri::Procedures),
        ["procedures", qualified] => {
            parse_and_validate_qualified_name(qualified, uri)
                .map(|(schema, procedure)| ResourceUri::ProcedureDetails { schema, procedure })
        }
        ["procedures", schema, procedure] => {
            validate_schema_and_name(schema, procedure, uri)
                .map(|(s, p)| ResourceUri::ProcedureDetails { schema: s, procedure: p })
        }
        ["procedures", ..] => Err(ResourceParseError {
            uri: uri.to_string(),
            reason: ParseErrorReason::TooManySegments {
                expected: 3,
                got: segments.len(),
            },
        }),
        ["functions"] => Ok(ResourceUri::Functions),
        ["functions", qualified] => {
            parse_and_validate_qualified_name(qualified, uri)
                .map(|(schema, function)| ResourceUri::FunctionDetails { schema, function })
        }
        ["functions", schema, function] => {
            validate_schema_and_name(schema, function, uri)
                .map(|(s, f)| ResourceUri::FunctionDetails { schema: s, function: f })
        }
        ["functions", ..] => Err(ResourceParseError {
            uri: uri.to_string(),
            reason: ParseErrorReason::TooManySegments {
                expected: 3,
                got: segments.len(),
            },
        }),
        ["triggers"] => Ok(ResourceUri::Triggers),
        ["triggers", qualified] => {
            parse_and_validate_qualified_name(qualified, uri)
                .map(|(schema, trigger)| ResourceUri::TriggerDetails { schema, trigger })
        }
        ["triggers", schema, trigger] => {
            validate_schema_and_name(schema, trigger, uri)
                .map(|(s, t)| ResourceUri::TriggerDetails { schema: s, trigger: t })
        }
        ["triggers", ..] => Err(ResourceParseError {
            uri: uri.to_string(),
            reason: ParseErrorReason::TooManySegments {
                expected: 3,
                got: segments.len(),
            },
        }),
        [unknown, ..] => Err(ResourceParseError {
            uri: uri.to_string(),
            reason: ParseErrorReason::UnknownResourceType {
                segment: (*unknown).to_string(),
            },
        }),
        // Empty slice is already handled above, but required for exhaustive match
        [] => unreachable!("Empty segments already handled above"),
    }
}

/// Parse and validate a qualified name like "schema.table".
fn parse_and_validate_qualified_name(
    qualified: &str,
    uri: &str,
) -> Result<(String, String), ResourceParseError> {
    match parse_qualified_name(qualified) {
        Ok((Some(schema), name)) => {
            // Validate schema
            if let Err(e) = validate_identifier(&schema) {
                return Err(ResourceParseError {
                    uri: uri.to_string(),
                    reason: ParseErrorReason::InvalidIdentifier {
                        identifier: schema,
                        reason: e.to_string(),
                    },
                });
            }
            // Validate name
            if let Err(e) = validate_identifier(&name) {
                return Err(ResourceParseError {
                    uri: uri.to_string(),
                    reason: ParseErrorReason::InvalidIdentifier {
                        identifier: name,
                        reason: e.to_string(),
                    },
                });
            }
            Ok((schema, name))
        }
        Ok((None, name)) => {
            // No schema specified, use dbo as default
            if let Err(e) = validate_identifier(&name) {
                return Err(ResourceParseError {
                    uri: uri.to_string(),
                    reason: ParseErrorReason::InvalidIdentifier {
                        identifier: name,
                        reason: e.to_string(),
                    },
                });
            }
            Ok(("dbo".to_string(), name))
        }
        Err(e) => Err(ResourceParseError {
            uri: uri.to_string(),
            reason: ParseErrorReason::InvalidIdentifier {
                identifier: qualified.to_string(),
                reason: e.to_string(),
            },
        }),
    }
}

/// Validate schema and name identifiers.
fn validate_schema_and_name(
    schema: &str,
    name: &str,
    uri: &str,
) -> Result<(String, String), ResourceParseError> {
    // Validate schema
    if let Err(e) = validate_identifier(schema) {
        return Err(ResourceParseError {
            uri: uri.to_string(),
            reason: ParseErrorReason::InvalidIdentifier {
                identifier: schema.to_string(),
                reason: e.to_string(),
            },
        });
    }
    // Validate name
    if let Err(e) = validate_identifier(name) {
        return Err(ResourceParseError {
            uri: uri.to_string(),
            reason: ParseErrorReason::InvalidIdentifier {
                identifier: name.to_string(),
                reason: e.to_string(),
            },
        });
    }
    Ok((schema.to_string(), name.to_string()))
}

// =========================================================================
// Resource Handlers
// =========================================================================

async fn read_server_info(server: &MssqlMcpServer) -> Result<String, String> {
    let info = server
        .metadata
        .get_server_info()
        .await
        .map_err(|e| e.to_string())?;

    serde_json::to_string_pretty(&info).map_err(|e| e.to_string())
}

async fn read_databases(server: &MssqlMcpServer) -> Result<String, String> {
    let databases = server
        .metadata
        .list_databases()
        .await
        .map_err(|e| e.to_string())?;

    let response = json!({
        "count": databases.len(),
        "databases": databases,
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

async fn read_schemas(server: &MssqlMcpServer) -> Result<String, String> {
    let schemas = server
        .metadata
        .list_schemas()
        .await
        .map_err(|e| e.to_string())?;

    let response = json!({
        "count": schemas.len(),
        "schemas": schemas,
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

async fn read_tables(server: &MssqlMcpServer) -> Result<String, String> {
    let tables = server
        .metadata
        .list_tables(None)
        .await
        .map_err(|e| e.to_string())?;

    let response = json!({
        "count": tables.len(),
        "tables": tables,
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

async fn read_table_details(
    server: &MssqlMcpServer,
    schema: &str,
    table: &str,
) -> Result<String, String> {
    // Get columns for the table
    let columns = server
        .metadata
        .get_table_columns(schema, table)
        .await
        .map_err(|e| e.to_string())?;

    if columns.is_empty() {
        return Err(format!("Table not found: {}.{}", schema, table));
    }

    let response = json!({
        "schema": schema,
        "table": table,
        "column_count": columns.len(),
        "columns": columns,
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

async fn read_views(server: &MssqlMcpServer) -> Result<String, String> {
    let views = server
        .metadata
        .list_views(None)
        .await
        .map_err(|e| e.to_string())?;

    let response = json!({
        "count": views.len(),
        "views": views,
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

async fn read_view_details(
    server: &MssqlMcpServer,
    schema: &str,
    view: &str,
) -> Result<String, String> {
    // Get views filtered by name
    let views = server
        .metadata
        .list_views(Some(schema))
        .await
        .map_err(|e| e.to_string())?;

    let view_info = views
        .iter()
        .find(|v| v.view_name.eq_ignore_ascii_case(view))
        .ok_or_else(|| format!("View not found: {}.{}", schema, view))?;

    let response = json!({
        "schema": view_info.schema_name,
        "view": view_info.view_name,
        "definition": view_info.definition,
        "is_updatable": view_info.is_updatable,
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

async fn read_procedures(server: &MssqlMcpServer) -> Result<String, String> {
    let procedures = server
        .metadata
        .list_procedures(None)
        .await
        .map_err(|e| e.to_string())?;

    let response = json!({
        "count": procedures.len(),
        "procedures": procedures,
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

async fn read_procedure_details(
    server: &MssqlMcpServer,
    schema: &str,
    procedure: &str,
) -> Result<String, String> {
    // Get procedure definition
    let definition = server
        .metadata
        .get_procedure_definition(schema, procedure)
        .await
        .map_err(|e| e.to_string())?;

    // Get procedure parameters
    let parameters = server
        .metadata
        .get_procedure_parameters(schema, procedure)
        .await
        .map_err(|e| e.to_string())?;

    let response = json!({
        "schema": schema,
        "procedure": procedure,
        "definition": definition,
        "parameter_count": parameters.len(),
        "parameters": parameters,
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

async fn read_functions(server: &MssqlMcpServer) -> Result<String, String> {
    let functions = server
        .metadata
        .list_functions(None)
        .await
        .map_err(|e| e.to_string())?;

    let response = json!({
        "count": functions.len(),
        "functions": functions,
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

async fn read_function_details(
    server: &MssqlMcpServer,
    schema: &str,
    function: &str,
) -> Result<String, String> {
    // Get functions filtered by schema
    let functions = server
        .metadata
        .list_functions(Some(schema))
        .await
        .map_err(|e| e.to_string())?;

    let func_info = functions
        .iter()
        .find(|f| f.function_name.eq_ignore_ascii_case(function))
        .ok_or_else(|| format!("Function not found: {}.{}", schema, function))?;

    // Get function parameters
    let parameters = server
        .metadata
        .get_function_parameters(schema, function)
        .await
        .map_err(|e| e.to_string())?;

    let response = json!({
        "schema": func_info.schema_name,
        "function": func_info.function_name,
        "type": func_info.function_type,
        "return_type": func_info.return_type,
        "created": func_info.create_date,
        "modified": func_info.modify_date,
        "parameter_count": parameters.len(),
        "parameters": parameters,
        "definition": func_info.definition,
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

async fn read_triggers(server: &MssqlMcpServer) -> Result<String, String> {
    let triggers = server
        .metadata
        .list_triggers(None)
        .await
        .map_err(|e| e.to_string())?;

    let response = json!({
        "count": triggers.len(),
        "triggers": triggers,
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

async fn read_trigger_details(
    server: &MssqlMcpServer,
    schema: &str,
    trigger: &str,
) -> Result<String, String> {
    // Get triggers filtered by schema
    let triggers = server
        .metadata
        .list_triggers(Some(schema))
        .await
        .map_err(|e| e.to_string())?;

    let trigger_info = triggers
        .iter()
        .find(|t| t.trigger_name.eq_ignore_ascii_case(trigger))
        .ok_or_else(|| format!("Trigger not found: {}.{}", schema, trigger))?;

    let response = json!({
        "schema": trigger_info.schema_name,
        "trigger": trigger_info.trigger_name,
        "parent_object": trigger_info.parent_object,
        "type": trigger_info.trigger_type,
        "status": if trigger_info.is_disabled { "Disabled" } else { "Enabled" },
        "events": trigger_info.trigger_events,
        "created": trigger_info.create_date,
        "modified": trigger_info.modify_date,
        "definition": trigger_info.definition,
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

// =========================================================================
// Helpers
// =========================================================================

/// Create a resource definition.
fn create_resource(uri: &str, name: &str, description: &str, mime_type: &str) -> Resource {
    let mut resource = RawResource::new(uri, name);
    resource.description = Some(description.to_string());
    resource.mime_type = Some(mime_type.to_string());
    resource.no_annotation()
}

/// Create a resource template definition.
fn create_resource_template(
    uri_template: &str,
    name: &str,
    description: &str,
    mime_type: &str,
) -> ResourceTemplate {
    RawResourceTemplate {
        uri_template: uri_template.to_string(),
        name: name.to_string(),
        title: None,
        description: Some(description.to_string()),
        mime_type: Some(mime_type.to_string()),
    }
    .no_annotation()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_resource_uri() {
        // Basic resource URIs - now returns Result
        assert!(matches!(
            parse_resource_uri("mssql://server/info"),
            Ok(ResourceUri::ServerInfo)
        ));
        assert!(matches!(
            parse_resource_uri("mssql://databases"),
            Ok(ResourceUri::Databases)
        ));
        // Unknown resource type returns error with context
        let result = parse_resource_uri("mssql://unknown");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err.reason, ParseErrorReason::UnknownResourceType { .. }));
        assert!(err.to_string().contains("unknown resource type"));
    }

    #[test]
    fn test_parse_table_uri_with_schema_path() {
        // Traditional path format: mssql://tables/schema/table
        match parse_resource_uri("mssql://tables/dbo/Users") {
            Ok(ResourceUri::TableDetails { schema, table }) => {
                assert_eq!(schema, "dbo");
                assert_eq!(table, "Users");
            }
            other => panic!("Expected Ok(TableDetails), got {:?}", other),
        }
    }

    #[test]
    fn test_parse_table_uri_with_qualified_name() {
        // Qualified name format: mssql://tables/schema.table
        match parse_resource_uri("mssql://tables/dbo.Users") {
            Ok(ResourceUri::TableDetails { schema, table }) => {
                assert_eq!(schema, "dbo");
                assert_eq!(table, "Users");
            }
            other => panic!("Expected Ok(TableDetails), got {:?}", other),
        }
    }

    #[test]
    fn test_parse_table_uri_default_schema() {
        // Just table name defaults to dbo
        match parse_resource_uri("mssql://tables/Users") {
            Ok(ResourceUri::TableDetails { schema, table }) => {
                assert_eq!(schema, "dbo");
                assert_eq!(table, "Users");
            }
            other => panic!("Expected Ok(TableDetails), got {:?}", other),
        }
    }

    #[test]
    fn test_parse_uri_rejects_injection() {
        // SQL injection attempts should result in Err with specific reason
        let result = parse_resource_uri("mssql://tables/dbo/Users--");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err.reason, ParseErrorReason::InvalidIdentifier { .. }));

        let result = parse_resource_uri("mssql://tables/dbo/Users;DROP");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err.reason, ParseErrorReason::InvalidIdentifier { .. }));

        let result = parse_resource_uri("mssql://tables/dbo'/Users");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err.reason, ParseErrorReason::InvalidIdentifier { .. }));
    }

    #[test]
    fn test_parse_qualified_name_helper() {
        let uri = "mssql://tables/test";

        // Valid qualified names
        let result = parse_and_validate_qualified_name("dbo.Users", uri);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ("dbo".to_string(), "Users".to_string()));

        let result = parse_and_validate_qualified_name("Users", uri);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ("dbo".to_string(), "Users".to_string()));

        // Invalid names with injection patterns - now returns Err with context
        let result = parse_and_validate_qualified_name("dbo.Users--", uri);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Users--"));

        let result = parse_and_validate_qualified_name("dbo';DROP", uri);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_uri_error_messages() {
        // Test that error messages are descriptive

        // Invalid scheme
        let err = parse_resource_uri("http://server/info").unwrap_err();
        assert!(err.to_string().contains("mssql://"));

        // Empty path
        let err = parse_resource_uri("mssql://").unwrap_err();
        assert!(err.to_string().contains("empty"));

        // Too many segments
        let err = parse_resource_uri("mssql://tables/a/b/c/d").unwrap_err();
        assert!(err.to_string().contains("too many"));
    }
}
