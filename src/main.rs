//! MSSQL MCP Server entry point.
//!
//! This binary starts the MCP server using stdio transport for integration
//! with Claude Desktop, Cursor, and other MCP clients.
//!
//! Features:
//! - Graceful shutdown with connection draining
//! - Signal handling (SIGTERM, SIGINT)
//! - Transaction rollback on shutdown
//! - Cache cleanup

use anyhow::Result;
use mssql_mcp_server::shutdown::{
    install_signal_handlers, new_shutdown_controller_with_timeouts, ShutdownConfig,
};
use mssql_mcp_server::{Config, MssqlMcpServer};
use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging to stderr (stdout is reserved for JSON-RPC)
    init_logging();

    // Log startup information to stderr
    let version = env!("CARGO_PKG_VERSION");
    eprintln!("MSSQL MCP Server v{version} starting...");
    eprintln!("Protocol: MCP 2025-03-26");
    eprintln!("Transport: stdio");

    // Set up panic hook for debugging
    std::panic::set_hook(Box::new(|info| {
        eprintln!("[PANIC] {}", info);
    }));

    // Load configuration from environment
    let config = Config::from_env()?;
    eprintln!("Configuration loaded successfully");

    // Load shutdown configuration
    let shutdown_config = ShutdownConfig::from_env();

    // Create the shutdown controller
    let shutdown_controller = new_shutdown_controller_with_timeouts(
        shutdown_config.drain_timeout,
        shutdown_config.force_timeout,
    );

    // Install signal handlers for graceful shutdown
    install_signal_handlers(shutdown_controller.clone()).await;

    // Create the MCP server
    let server = MssqlMcpServer::new(config).await?;
    let state = server.state().clone();
    eprintln!("Server initialized. Ready to accept requests...");

    // Start serving on stdio transport
    let transport = rmcp::transport::stdio();
    let service = server.serve(transport).await?;

    // Wait for shutdown signal or service completion
    let mut shutdown_signal = shutdown_controller.signal();

    tokio::select! {
        quit_reason = service.waiting() => {
            match quit_reason {
                Ok(reason) => eprintln!("Service stopped: {reason:?}"),
                Err(e) => eprintln!("Service error: {e}"),
            }
        }
        _ = shutdown_signal.recv() => {
            eprintln!("Shutdown signal received");
        }
    }

    // Perform graceful shutdown
    eprintln!("Initiating graceful shutdown...");
    shutdown_controller.graceful_shutdown(&state).await;
    eprintln!("Server shutdown complete");

    Ok(())
}

/// Initialize tracing subscriber with stderr output.
///
/// Logs MUST go to stderr because stdout is used for JSON-RPC communication.
fn init_logging() {
    let filter = std::env::var("RUST_LOG")
        .map(EnvFilter::new)
        .unwrap_or_else(|_| EnvFilter::new("warn,mssql_mcp_server=info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
}
