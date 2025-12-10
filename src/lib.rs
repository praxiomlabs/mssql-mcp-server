//! # MSSQL MCP Server
//!
//! A high-performance Model Context Protocol (MCP) server for Microsoft SQL Server.
//!
//! This crate provides:
//! - **Resources**: Browse database metadata (tables, views, procedures, schemas)
//! - **Tools**: Execute queries and stored procedures
//! - **Prompts**: AI-assisted query generation
//! - **Caching**: In-memory query result caching with configurable TTL
//!
//! ## Architecture
//!
//! The server follows MCP protocol semantics:
//! - Resources for passive data access (schema discovery)
//! - Tools for active operations (query execution)
//! - Prompts for templated AI interactions

pub mod cache;
pub mod config;
pub mod constants;
pub mod database;
pub mod error;
pub mod handlers;
pub mod prompts;
pub mod resources;
pub mod security;
pub mod server;
pub mod shutdown;
pub mod state;
pub mod telemetry;
pub mod tools;
pub mod transport;

pub use config::Config;
pub use error::McpError;
pub use server::MssqlMcpServer;
