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

    /// Enable request tracing via tower-http TraceLayer.
    ///
    /// When enabled, all HTTP requests are traced with structured logging
    /// including request method, path, status code, and latency.
    pub enable_tracing: bool,

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
            enable_tracing: true, // Enabled by default for observability
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

        if let Ok(tracing) = std::env::var("MSSQL_HTTP_TRACING") {
            config.enable_tracing = tracing.to_lowercase() == "true" || tracing == "1";
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

/// HTTP server implementation using mcpkit-axum (only available with `http` feature).
///
/// This provides full MCP functionality over HTTP, including:
/// - JSON-RPC message handling at `/mcp`
/// - Server-Sent Events streaming at `/mcp/sse`
/// - All tools, resources, and prompts from the MssqlMcpServer
#[cfg(feature = "http")]
pub mod http_server {
    use super::*;
    use crate::shutdown::SharedShutdownController;
    use crate::MssqlMcpServer;
    use axum::{response::IntoResponse, routing::get, Json, Router};
    use mcpkit_axum::McpRouter;
    use tracing::info;

    /// Start the HTTP server with full MCP support.
    pub async fn start_http_server(
        mcp_server: MssqlMcpServer,
        config: HttpConfig,
    ) -> Result<(), anyhow::Error> {
        start_http_server_with_shutdown(mcp_server, config, None).await
    }

    /// Start the HTTP server with graceful shutdown support.
    ///
    /// Uses mcpkit-axum's `McpRouter` for full MCP protocol support:
    /// - All 32+ tools are available over HTTP
    /// - All resources are accessible
    /// - All prompts work correctly
    ///
    /// Custom endpoints:
    /// - `/health` - Health check endpoint
    /// - `/` - Also serves health check
    pub async fn start_http_server_with_shutdown(
        mcp_server: MssqlMcpServer,
        config: HttpConfig,
        shutdown_controller: Option<SharedShutdownController>,
    ) -> Result<(), anyhow::Error> {
        // Build MCP router with mcpkit-axum for full protocol support
        let mut mcp_router = McpRouter::new(mcp_server)
            .post_path("/mcp")
            .sse_path("/mcp/sse");

        // Enable CORS if configured
        if config.enable_cors {
            mcp_router = mcp_router.with_cors();
        }

        // Enable request tracing if configured
        // This adds tower-http TraceLayer for structured request/response logging
        if config.enable_tracing {
            mcp_router = mcp_router.with_tracing();
        }

        // Merge with custom health endpoints
        let app = Router::new()
            .route("/health", get(health_handler))
            .route("/", get(health_handler))
            .merge(mcp_router.into_router());

        let addr = format!("{}:{}", config.host, config.port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;

        info!("HTTP server listening on http://{}", addr);
        info!("MCP endpoint: http://{}/mcp", addr);
        info!("SSE endpoint: http://{}/mcp/sse", addr);
        info!("Health endpoint: http://{}/health", addr);
        if config.enable_tracing {
            info!("Request tracing enabled");
        }

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
            "endpoints": {
                "mcp": "/mcp",
                "sse": "/mcp/sse",
                "health": "/health"
            }
        }))
    }
}
