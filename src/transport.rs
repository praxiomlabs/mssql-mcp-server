//! Transport layer abstraction for MCP server.
//!
//! Supports multiple transport mechanisms:
//! - stdio: Standard input/output (default, for CLI tools)
//! - http: HTTP with Server-Sent Events (SSE) for web integrations
//!
//! The HTTP transport is optional and requires the `http` feature flag.

/// Transport configuration.
#[derive(Debug, Clone)]
pub struct TransportConfig {
    /// Transport type to use.
    pub transport_type: TransportType,

    /// HTTP server configuration (only used for HTTP transport).
    pub http: HttpConfig,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            transport_type: TransportType::Stdio,
            http: HttpConfig::default(),
        }
    }
}

/// Available transport types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportType {
    /// Standard input/output transport (default).
    Stdio,

    /// HTTP transport with SSE support.
    #[cfg(feature = "http")]
    Http,
}

/// Error returned when parsing a transport type fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseTransportTypeError(String);

impl std::fmt::Display for ParseTransportTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid transport type: '{}'", self.0)
    }
}

impl std::error::Error for ParseTransportTypeError {}

impl std::str::FromStr for TransportType {
    type Err = ParseTransportTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "stdio" | "standard" | "io" => Ok(TransportType::Stdio),
            #[cfg(feature = "http")]
            "http" | "sse" | "web" => Ok(TransportType::Http),
            _ => Err(ParseTransportTypeError(s.to_string())),
        }
    }
}

impl std::fmt::Display for TransportType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportType::Stdio => write!(f, "stdio"),
            #[cfg(feature = "http")]
            TransportType::Http => write!(f, "http"),
        }
    }
}

/// HTTP transport configuration.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Host to bind to.
    pub host: String,

    /// Port to listen on.
    pub port: u16,

    /// Enable CORS.
    pub enable_cors: bool,

    /// Allowed origins for CORS (empty means all).
    pub cors_origins: Vec<String>,

    /// Request timeout in seconds.
    pub request_timeout_seconds: u64,

    /// Maximum request body size in bytes.
    pub max_body_size: usize,

    /// Enable rate limiting.
    pub rate_limit_enabled: bool,

    /// Rate limit requests per minute per client.
    pub rate_limit_rpm: u32,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3000,
            enable_cors: true,
            cors_origins: Vec::new(),
            request_timeout_seconds: 30,
            max_body_size: 10 * 1024 * 1024, // 10MB
            rate_limit_enabled: true,
            rate_limit_rpm: 100,
        }
    }
}

impl HttpConfig {
    /// Create configuration from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(host) = std::env::var("MSSQL_HTTP_HOST") {
            config.host = host;
        }

        if let Ok(port) = std::env::var("MSSQL_HTTP_PORT") {
            if let Ok(p) = port.parse() {
                config.port = p;
            }
        }

        if let Ok(cors) = std::env::var("MSSQL_HTTP_CORS") {
            config.enable_cors = cors.to_lowercase() == "true" || cors == "1";
        }

        if let Ok(origins) = std::env::var("MSSQL_HTTP_CORS_ORIGINS") {
            config.cors_origins = origins.split(',').map(|s| s.trim().to_string()).collect();
        }

        if let Ok(timeout) = std::env::var("MSSQL_HTTP_TIMEOUT") {
            if let Ok(t) = timeout.parse() {
                config.request_timeout_seconds = t;
            }
        }

        if let Ok(rate_limit) = std::env::var("MSSQL_HTTP_RATE_LIMIT") {
            config.rate_limit_enabled = rate_limit.to_lowercase() == "true" || rate_limit == "1";
        }

        if let Ok(rpm) = std::env::var("MSSQL_HTTP_RATE_LIMIT_RPM") {
            if let Ok(r) = rpm.parse() {
                config.rate_limit_rpm = r;
            }
        }

        config
    }
}

/// HTTP server implementation (only available with `http` feature).
#[cfg(feature = "http")]
pub mod http_server {
    use super::*;
    use crate::shutdown::SharedShutdownController;
    use crate::MssqlMcpServer;
    use axum::{
        extract::State,
        http::{header, Method},
        response::{sse::Event, IntoResponse, Sse},
        routing::{get, post},
        Json, Router,
    };
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::RwLock;
    use tower_http::cors::{Any, CorsLayer};
    use tracing::info;

    /// Shared server state for HTTP handlers.
    pub struct HttpServerState {
        pub mcp_server: Arc<RwLock<MssqlMcpServer>>,
        pub config: HttpConfig,
        pub shutdown_controller: Option<SharedShutdownController>,
    }

    /// Start the HTTP server.
    pub async fn start_http_server(
        mcp_server: MssqlMcpServer,
        config: HttpConfig,
    ) -> Result<(), anyhow::Error> {
        start_http_server_with_shutdown(mcp_server, config, None).await
    }

    /// Start the HTTP server with graceful shutdown support.
    pub async fn start_http_server_with_shutdown(
        mcp_server: MssqlMcpServer,
        config: HttpConfig,
        shutdown_controller: Option<SharedShutdownController>,
    ) -> Result<(), anyhow::Error> {
        let state = Arc::new(HttpServerState {
            mcp_server: Arc::new(RwLock::new(mcp_server)),
            config: config.clone(),
            shutdown_controller: shutdown_controller.clone(),
        });

        // Build CORS layer
        let cors = if config.enable_cors {
            if config.cors_origins.is_empty() {
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                    .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
            } else {
                let origins: Vec<_> = config
                    .cors_origins
                    .iter()
                    .filter_map(|o| o.parse().ok())
                    .collect();
                CorsLayer::new()
                    .allow_origin(origins)
                    .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                    .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
            }
        } else {
            CorsLayer::new()
        };

        // Build router
        let app = Router::new()
            .route("/", get(health_handler))
            .route("/health", get(health_handler))
            .route("/sse", get(sse_handler))
            .route("/message", post(message_handler))
            .layer(cors)
            .with_state(state);

        let addr = format!("{}:{}", config.host, config.port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;

        info!("HTTP server listening on http://{}", addr);
        info!("SSE endpoint: http://{}/sse", addr);
        info!("Message endpoint: http://{}/message", addr);

        // Start server with graceful shutdown if controller is provided
        if let Some(controller) = shutdown_controller {
            let mut shutdown_signal = controller.signal();
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    shutdown_signal.recv().await;
                    info!("HTTP server received shutdown signal");
                })
                .await?;
        } else {
            axum::serve(listener, app).await?;
        }

        Ok(())
    }

    /// Health check handler.
    async fn health_handler() -> impl IntoResponse {
        Json(serde_json::json!({
            "status": "healthy",
            "server": "mssql-mcp-server",
            "version": env!("CARGO_PKG_VERSION"),
            "transport": "http",
        }))
    }

    /// SSE connection handler for streaming responses.
    async fn sse_handler(
        State(_state): State<Arc<HttpServerState>>,
    ) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
        use futures_util::stream;

        info!("New SSE connection established");

        // Create a simple ping stream to keep connection alive
        let stream = stream::repeat_with(|| {
            Ok(Event::default().event("ping").data(
                serde_json::json!({"timestamp": chrono::Utc::now().to_rfc3339()}).to_string(),
            ))
        });

        Sse::new(stream).keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive"),
        )
    }

    /// Message handler for JSON-RPC requests.
    ///
    /// This handler provides basic MCP protocol support over HTTP.
    /// Note: Full MCP functionality is best used via stdio transport.
    /// The HTTP transport provides a simplified interface for web integrations.
    async fn message_handler(
        State(state): State<Arc<HttpServerState>>,
        Json(request): Json<serde_json::Value>,
    ) -> impl IntoResponse {
        use rmcp::handler::server::ServerHandler;

        info!("Received HTTP message: {:?}", request.get("method"));

        let method = request.get("method").and_then(|m| m.as_str());
        let id = request.get("id").cloned();
        let _params = request.get("params").cloned();

        let response = match method {
            Some("initialize") => {
                let server = state.mcp_server.read().await;
                let info = server.get_info();
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "serverInfo": {
                            "name": info.server_info.name,
                            "version": info.server_info.version,
                        },
                        "capabilities": {
                            "tools": {},
                            "resources": {},
                            "prompts": {},
                        },
                        "instructions": info.instructions
                    }
                })
            }

            Some("initialized") | Some("notifications/initialized") => {
                // Client acknowledging initialization
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {}
                })
            }

            Some("ping") => {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {}
                })
            }

            // Note: Full MCP method routing would require rmcp's internal context handling.
            // For production HTTP transport, consider using the stdio transport via a subprocess
            // or implementing a full JSON-RPC router that matches rmcp's expectations.
            Some(method) => {
                info!(
                    "Method {} received - HTTP transport has limited support",
                    method
                );
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32601,
                        "message": format!(
                            "Method '{}' is not fully supported over HTTP transport. \
                             For full MCP functionality, use the stdio transport.",
                            method
                        )
                    }
                })
            }

            None => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32600,
                    "message": "Invalid request: missing method"
                }
            }),
        };

        Json(response)
    }
}
