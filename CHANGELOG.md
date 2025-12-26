# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- **BREAKING**: Migrated SQL Server driver from `tiberius` to `mssql-client` v0.3.0
  - Resolves dependency conflicts with duplicate crate versions
  - Improved type support including proper handling of all MAX/LOB types
  - Better DATE, XML, SQL_VARIANT, and high-precision DECIMAL support
- Migrated connection pooling from `bb8` to `mssql-driver-pool` v0.3.0
  - Integrated pooling designed specifically for mssql-client
  - Simplified connection management with Arc<Pool> pattern
- Updated Azure SDK dependencies to v0.25 for Azure AD authentication
- Updated `deny.toml` with additional approved licenses (OpenSSL, Zlib, CDLA-Permissive-2.0)

### Fixed
- Connection pool now properly wrapped in Arc for thread-safe sharing
- Row column access updated from Result<Option<T>> to Option<T> pattern
- QueryStream collection using TryStreamExt::try_collect()

## [0.1.0] - 2025-12-18

### Added

#### Core Features
- Initial MCP server implementation using the rmcp SDK
- Full MCP protocol compliance with stdio transport
- Comprehensive SQL Server connectivity via TDS protocol (mssql-client)
- Connection pooling with mssql-driver-pool for high-performance concurrent access

#### Tools
- `execute_query` - Execute read-only SQL queries with result limiting
- `execute_query_async` - Execute queries with session affinity and per-query timeout support
- `execute_ddl` - Execute DDL statements (CREATE, ALTER, DROP) with batch-first support
- `execute_script` - Execute multi-batch T-SQL scripts with GO separator parsing
- `list_databases` - List all accessible databases
- `list_tables` - List tables with optional schema filtering
- `describe_table` - Get detailed table metadata including columns, constraints, indexes
- `get_table_sample` - Preview table data with configurable row limits

#### Session Management
- `create_session` - Create pinned database sessions for temp tables and state
- `execute_in_session` - Execute queries within pinned sessions
- `close_session` - Clean up session resources
- Session timeout with automatic cleanup
- Transaction support within sessions

#### Resources (MCP Resources Protocol)
- Database schemas as browsable resources
- Table metadata resources with column details
- View definitions with source SQL
- Stored procedure metadata and parameters
- Function metadata (scalar and table-valued)
- Trigger metadata and definitions

#### Security
- SQL injection prevention with comprehensive validation
- Reserved keyword detection and warnings
- Identifier validation and escaping
- Query complexity analysis
- Dangerous operation blocking (configurable)
- Read-only mode enforcement option

#### Resilience
- Circuit breaker pattern for fault tolerance
- Configurable retry with exponential backoff
- Per-query timeout overrides
- Connection validation with timeout protection

#### Configuration
- Environment variable configuration
- Connection string support
- Configurable query limits (max rows, timeout)
- Feature flags for optional capabilities
- Azure AD authentication support (optional feature)

#### Observability
- Structured logging with tracing
- Query execution timing metrics
- Connection pool statistics
- Optional OpenTelemetry integration (telemetry feature)

#### Developer Experience
- Justfile with common development commands
- Comprehensive test suite (111+ tests)
- Example environment configuration
- Full documentation with rustdoc

### Security Notes
- All queries are validated before execution
- Identifiers are properly escaped to prevent injection
- Dangerous operations (DROP DATABASE, TRUNCATE) blocked by default
- Connection credentials never logged

### Optional Features
- `http` - HTTP/SSE transport support with axum
- `telemetry` - OpenTelemetry metrics and tracing export
- `azure-auth` - Azure Active Directory authentication

[Unreleased]: https://github.com/jkindrix/mssql-mcp-server/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jkindrix/mssql-mcp-server/releases/tag/v0.1.0
