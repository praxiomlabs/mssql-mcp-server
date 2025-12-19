//! ServerHandler implementation for the MSSQL MCP Server.
//!
//! This module implements the rmcp `ServerHandler` trait which defines how
//! the server responds to MCP protocol requests.

use crate::prompts::{build_prompt_list, get_prompt};
use crate::resources::{build_resource_list, build_resource_templates, read_resource};
use crate::server::MssqlMcpServer;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    GetPromptRequestParam, GetPromptResult, Implementation, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, Meta, PaginatedRequestParam, ProtocolVersion,
    ReadResourceRequestParam, ReadResourceResult, ServerCapabilities, ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{tool_handler, ErrorData};
use tracing::info;

/// The `#[tool_handler]` macro wires up tool routing automatically.
/// It generates the `list_tools` and `call_tool` method implementations.
#[tool_handler]
impl ServerHandler for MssqlMcpServer {
    /// Server identification - called during initialization handshake.
    fn get_info(&self) -> ServerInfo {
        info!("MCP client requesting server info");

        ServerInfo {
            // Use the latest protocol version
            protocol_version: ProtocolVersion::LATEST,

            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),

            server_info: Implementation {
                name: env!("CARGO_PKG_NAME").to_string(),
                title: Some("MSSQL MCP Server".to_string()),
                version: env!("CARGO_PKG_VERSION").to_string(),
                icons: None,
                website_url: None,
            },

            instructions: Some(build_instructions(self)),
        }
    }

    /// List available resources.
    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let resources = build_resource_list(self);

        Ok(ListResourcesResult {
            resources,
            next_cursor: None,
            meta: Some(Meta(
                serde_json::json!({
                    "database_mode": self.is_database_mode(),
                    "current_database": self.current_database(),
                })
                .as_object()
                .cloned()
                .unwrap_or_default(),
            )),
        })
    }

    /// List resource templates for dynamic resources.
    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        let templates = build_resource_templates(self);

        Ok(ListResourceTemplatesResult {
            resource_templates: templates,
            next_cursor: None,
            meta: None,
        })
    }

    /// Read a specific resource.
    async fn read_resource(
        &self,
        request: ReadResourceRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        read_resource(self, &request.uri)
            .await
            .map_err(|e| ErrorData::invalid_params(e, None))
    }

    /// List available prompts.
    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        let prompts = build_prompt_list();

        Ok(ListPromptsResult {
            prompts,
            next_cursor: None,
            meta: None,
        })
    }

    /// Get a specific prompt.
    async fn get_prompt(
        &self,
        request: GetPromptRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, ErrorData> {
        // Convert Map<String, Value> to HashMap<String, String>
        let arguments: Option<std::collections::HashMap<String, String>> =
            request.arguments.map(|map| {
                map.into_iter()
                    .map(|(k, v)| {
                        // Convert Value to String (handle strings and other types)
                        match v {
                            serde_json::Value::String(s) => (k, s),
                            other => (k, other.to_string()),
                        }
                    })
                    .collect()
            });

        get_prompt(self, &request.name, arguments.as_ref())
            .await
            .map_err(|e| ErrorData::invalid_params(e, None))
    }
}

/// Build server instructions based on current state.
fn build_instructions(server: &MssqlMcpServer) -> String {
    let mut instructions = String::new();

    instructions.push_str("# MSSQL MCP Server\n\n");
    instructions.push_str("This server provides access to Microsoft SQL Server databases.\n\n");

    // Mode-specific instructions
    if server.is_database_mode() {
        if let Some(db) = server.current_database() {
            instructions.push_str(&format!("**Connected to database:** `{}`\n\n", db));
        }
        instructions.push_str("## Available Operations\n\n");
        instructions.push_str("### Resources (Read-only metadata access)\n");
        instructions.push_str("- Browse tables, views, and stored procedures\n");
        instructions.push_str("- View column definitions and data types\n");
        instructions.push_str("- Inspect stored procedure parameters\n\n");
    } else {
        instructions.push_str("**Running in server mode** (no specific database selected)\n\n");
        instructions.push_str("### Available Operations\n");
        instructions.push_str("- List databases on the server\n");
        instructions.push_str("- View server configuration\n\n");
    }

    // Security mode instructions
    instructions.push_str("### Tools (Query execution)\n");
    match server.config.security.validation_mode {
        crate::security::ValidationMode::ReadOnly => {
            instructions.push_str("- **Read-only mode**: Only SELECT queries are allowed\n");
        }
        crate::security::ValidationMode::Standard => {
            instructions.push_str("- **Standard mode**: SELECT, INSERT, UPDATE, DELETE allowed\n");
            instructions.push_str("- DDL operations (CREATE, DROP, ALTER) are blocked\n");
        }
        crate::security::ValidationMode::Unrestricted => {
            instructions.push_str("- **Unrestricted mode**: All SQL operations allowed\n");
            instructions.push_str("- ⚠️ Use with caution - DDL operations permitted\n");
        }
    }

    instructions.push_str("\n### Best Practices\n");
    instructions.push_str("1. Use resources to explore schema before writing queries\n");
    instructions.push_str("2. Use `execute_async` for long-running queries\n");
    instructions.push_str("3. Use `explain_query` to analyze query performance\n");
    instructions.push_str("4. Use parameterized stored procedures when available\n");

    instructions
}
