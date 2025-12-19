# MSSQL MCP Server

A high-performance [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) server for Microsoft SQL Server, enabling AI assistants like Claude to interact with your SQL Server databases securely and efficiently.

## Features

### Core Capabilities

- **Query Execution**: Execute SQL queries with comprehensive result formatting
- **Stored Procedures**: Call stored procedures with parameter support
- **Transactions**: Full ACID transaction support with isolation levels
- **Pinned Sessions**: Persistent connections for temp tables and session state
- **Pagination**: Cursor-based pagination for large result sets
- **Bulk Operations**: Efficient batch inserts for large datasets
- **Data Sampling**: Statistical sampling methods for data exploration

### Schema Discovery (Resources)

Browse database metadata via MCP resources:

- `mssql://server/info` - Server version, edition, and configuration
- `mssql://databases` - List all databases
- `mssql://schemas` - List schemas in current database
- `mssql://tables` - List tables with row counts and sizes
- `mssql://tables/{schema}/{table}` - Table details with columns
- `mssql://views` - List views
- `mssql://views/{schema}/{view}` - View definition
- `mssql://procedures` - List stored procedures
- `mssql://procedures/{schema}/{procedure}` - Procedure parameters
- `mssql://functions` - List user-defined functions
- `mssql://triggers` - List database triggers

### Security

- **SQL Injection Protection**: Multi-layer defense against injection attacks
- **Query Validation**: Configurable validation modes (read-only, standard, unrestricted)
- **Identifier Escaping**: Safe handling of object names
- **Parameterized Queries**: Full support for parameterized execution

### Performance

- **Connection Pooling**: Efficient bb8-based connection pooling
- **Query Caching**: In-memory caching with configurable TTL
- **Async I/O**: Full async/await support with Tokio runtime
- **Result Streaming**: Efficient memory usage for large results

## Installation

### Prerequisites

- Rust 1.70 or later
- SQL Server 2016 or later (including Azure SQL Database)

### Build from Source

```bash
git clone https://github.com/jkindrix/mssql-mcp-server.git
cd mssql-mcp-server
cargo build --release
```

### Optional Features

Enable optional features during build:

```bash
# Enable HTTP transport with SSE support
cargo build --release --features http

# Enable OpenTelemetry metrics and tracing
cargo build --release --features telemetry

# Enable Azure AD authentication
cargo build --release --features azure-auth

# Enable all features
cargo build --release --features "http,telemetry,azure-auth"
```

## Configuration

Create a `.env` file (see `.env.example` for all options):

```bash
# Required
MSSQL_HOST=localhost
MSSQL_USER=sa
MSSQL_PASSWORD=your_password

# Optional
MSSQL_PORT=1433
MSSQL_DATABASE=mydb  # Omit for server mode
MSSQL_ENCRYPT=true
MSSQL_TRUST_CERT=false
```

### Authentication Methods

**SQL Server Authentication:**
```bash
MSSQL_USER=sa
MSSQL_PASSWORD=your_password
```

**Azure AD Authentication (requires `azure-auth` feature):**
```bash
MSSQL_AUTH_TYPE=azuread
MSSQL_AZURE_CLIENT_ID=your_client_id
MSSQL_AZURE_CLIENT_SECRET=your_client_secret
MSSQL_AZURE_TENANT_ID=your_tenant_id
```

### Connection Pool Settings

```bash
MSSQL_MIN_CONNECTIONS=2
MSSQL_MAX_CONNECTIONS=10
MSSQL_CONNECTION_TIMEOUT=30
MSSQL_IDLE_TIMEOUT=600
```

### Query Execution

```bash
MSSQL_MAX_ROWS=1000
MSSQL_QUERY_TIMEOUT=30
MSSQL_VALIDATION_MODE=standard  # read_only, standard, unrestricted
```

### Security Settings

```bash
MSSQL_INJECTION_DETECTION=true
```

## Usage

### With Claude Desktop

Add to your Claude Desktop configuration (`~/.config/claude-desktop/config.json` on Linux, `~/Library/Application Support/Claude/claude_desktop_config.json` on macOS):

```json
{
  "mcpServers": {
    "mssql": {
      "command": "/path/to/mssql-mcp-server",
      "env": {
        "MSSQL_HOST": "localhost",
        "MSSQL_USER": "sa",
        "MSSQL_PASSWORD": "your_password",
        "MSSQL_DATABASE": "mydb"
      }
    }
  }
}
```

### Running Standalone

```bash
# With environment variables
MSSQL_HOST=localhost MSSQL_USER=sa MSSQL_PASSWORD=pass ./mssql-mcp-server

# With .env file
./mssql-mcp-server
```

## Available Tools

### Query Execution

| Tool | Description |
|------|-------------|
| `execute_query` | Execute a read-only SQL query and return results |
| `execute_parameterized` | Execute query with parameterized values |
| `execute_procedure` | Execute a stored procedure with parameters |
| `execute_async` | Execute query with session affinity and timeout override |
| `execute_paginated` | Execute query with cursor-based pagination |
| `explain_query` | Get execution plan for a query |
| `analyze_query` | Analyze query for performance issues |

### Transactions

| Tool | Description |
|------|-------------|
| `begin_transaction` | Start a new database transaction |
| `execute_in_transaction` | Execute query within a transaction |
| `commit_transaction` | Commit an open transaction |
| `rollback_transaction` | Rollback a transaction (optionally to savepoint) |

### Pinned Sessions

| Tool | Description |
|------|-------------|
| `begin_pinned_session` | Start a session with dedicated connection |
| `execute_in_pinned_session` | Execute query in pinned session |
| `end_pinned_session` | Close a pinned session |
| `list_pinned_sessions` | List active pinned sessions |

### Async Session Management

| Tool | Description |
|------|-------------|
| `get_session_status` | Get status of an async query session |
| `get_session_results` | Get results from a completed async session |
| `cancel_session` | Cancel a running async session |
| `list_sessions` | List all active async sessions |

### Data Operations

| Tool | Description |
|------|-------------|
| `sample_data` | Sample data from a table (TOP N, RANDOM, TABLESAMPLE) |
| `bulk_insert` | Insert multiple rows in batches |
| `export_data` | Export query results in various formats |

### Schema Tools

| Tool | Description |
|------|-------------|
| `switch_database` | Switch the active database context |
| `compare_schemas` | Compare schemas between databases |
| `compare_tables` | Compare table structures |
| `recommend_indexes` | Get index recommendations for a query |

### Server Management

| Tool | Description |
|------|-------------|
| `health_check` | Check server connectivity and health |
| `set_timeout` | Set query timeout for the session |
| `get_timeout` | Get current query timeout setting |
| `get_metrics` | Get server performance metrics |
| `get_pool_metrics` | Get connection pool statistics |
| `get_internal_metrics` | Get internal server metrics |

## API Examples

### Execute a Query

```json
{
  "method": "tools/call",
  "params": {
    "name": "execute_query",
    "arguments": {
      "query": "SELECT TOP 10 * FROM Customers"
    }
  }
}
```

### Parameterized Query

```json
{
  "method": "tools/call",
  "params": {
    "name": "execute_parameterized",
    "arguments": {
      "query": "SELECT * FROM Orders WHERE CustomerID = @customerId AND Status = @status",
      "params": {
        "customerId": "ALFKI",
        "status": "Shipped"
      }
    }
  }
}
```

### Transaction Example

```json
// Begin transaction
{
  "method": "tools/call",
  "params": {
    "name": "begin_transaction",
    "arguments": {
      "isolation_level": "read_committed"
    }
  }
}
// Returns: { "transaction_id": "txn_abc123" }

// Execute in transaction
{
  "method": "tools/call",
  "params": {
    "name": "execute_in_transaction",
    "arguments": {
      "transaction_id": "txn_abc123",
      "query": "UPDATE Accounts SET Balance = Balance - 100 WHERE AccountID = 1"
    }
  }
}

// Commit
{
  "method": "tools/call",
  "params": {
    "name": "commit_transaction",
    "arguments": {
      "transaction_id": "txn_abc123"
    }
  }
}
```

### Pinned Session Example

```json
// Start session (for temp tables, session variables)
{
  "method": "tools/call",
  "params": {
    "name": "begin_pinned_session",
    "arguments": {
      "session_id": "analysis_session"
    }
  }
}

// Create temp table
{
  "method": "tools/call",
  "params": {
    "name": "execute_in_pinned_session",
    "arguments": {
      "session_id": "analysis_session",
      "query": "CREATE TABLE #TempResults (ID INT, Value DECIMAL(18,2))"
    }
  }
}

// Use temp table in subsequent queries
{
  "method": "tools/call",
  "params": {
    "name": "execute_in_pinned_session",
    "arguments": {
      "session_id": "analysis_session",
      "query": "INSERT INTO #TempResults SELECT ID, SUM(Amount) FROM Orders GROUP BY ID"
    }
  }
}
```

## Architecture

```
                    ┌─────────────────────────────────────────────┐
                    │              MCP Client                     │
                    │    (Claude Desktop, Cursor, etc.)          │
                    └────────────────────┬────────────────────────┘
                                        │ JSON-RPC over stdio
                    ┌────────────────────▼────────────────────────┐
                    │           MSSQL MCP Server                 │
                    │  ┌─────────────────────────────────────┐   │
                    │  │         MCP Handler Layer           │   │
                    │  │   (Resources, Tools, Prompts)       │   │
                    │  └─────────────────┬───────────────────┘   │
                    │  ┌─────────────────▼───────────────────┐   │
                    │  │        Security Layer               │   │
                    │  │  (Validation, Injection Detection)  │   │
                    │  └─────────────────┬───────────────────┘   │
                    │  ┌─────────────────▼───────────────────┐   │
                    │  │        Query Executor               │   │
                    │  │   (Caching, Session Management)     │   │
                    │  └─────────────────┬───────────────────┘   │
                    │  ┌─────────────────▼───────────────────┐   │
                    │  │       Connection Pool (bb8)         │   │
                    │  │   ┌─────────┐  ┌─────────────────┐  │   │
                    │  │   │  Pool   │  │ Transaction &   │  │   │
                    │  │   │Connects │  │Session Connects │  │   │
                    │  │   └────┬────┘  └────────┬────────┘  │   │
                    │  └────────┼────────────────┼───────────┘   │
                    └───────────┼────────────────┼───────────────┘
                                │                │
                    ┌───────────▼────────────────▼───────────────┐
                    │           SQL Server (TDS)                 │
                    │      (On-prem or Azure SQL Database)       │
                    └────────────────────────────────────────────┘
```

## Development

This project uses [just](https://github.com/casey/just) as a command runner. Run `just --list` to see all available commands.

### Prerequisites

- Rust 1.70+
- Docker (for integration tests and local development)
- [just](https://github.com/casey/just) command runner

### Local SQL Server Setup

For local development and testing, use Docker Compose to run SQL Server containers:

```bash
# Start all SQL Server versions (2025, 2022, 2019)
just db-up

# Or start just SQL Server 2025 (primary target)
just db-up-2025

# Wait for containers to be healthy
just db-wait

# View container status
just db-status

# Stop containers
just db-down
```

**Port Mapping:**
| Version | Port | Password |
|---------|------|----------|
| SQL Server 2025 | 1433 | `YourStrong@Passw0rd` |
| SQL Server 2022 | 1434 | `YourStrong@Passw0rd` |
| SQL Server 2019 | 1435 | `YourStrong@Passw0rd` |

**Connect via sqlcmd:**
```bash
just db-connect-2025  # Connect to SQL Server 2025
just db-connect-2022  # Connect to SQL Server 2022
```

### Common Commands

```bash
# Format code
just fmt

# Run linter (clippy)
just check

# Run all tests (unit tests only, no Docker required)
just test

# Run all CI checks locally
just ci

# Build release binary
just build-release

# Generate documentation
just doc

# Run security audit
just audit
```

### Integration Testing

Integration tests use [testcontainers](https://testcontainers.com/) to automatically manage SQL Server containers:

```bash
# Run integration tests (requires Docker)
just test-integration

# Test against a specific SQL Server version
just test-integration-2025
just test-integration-2022
just test-integration-2019

# Test against all supported versions
just test-integration-all
```

**Version Matrix (as of December 2025):**
| Version | Status | Recommended |
|---------|--------|-------------|
| 2025-latest | Current GA | ✅ Primary target |
| 2022-latest | Supported | ✅ Yes |
| 2019-latest | Extended support only | ⚠️ Legacy |
| 2017-latest | Extended support (ends Oct 2027) | ❌ Not recommended |

### Running Tests Directly

```bash
# Unit tests (no database required)
cargo test

# With all features
cargo test --all-features

# Verbose output
cargo test --all-features -- --nocapture

# Integration tests with specific version
MSSQL_TEST_VERSION=2022-latest cargo test --test integration_tests -- --ignored
```

### Linting

```bash
cargo clippy --all-features -- -D warnings
```

### Building Documentation

```bash
cargo doc --open
```

## Troubleshooting

### Connection Issues

- Verify SQL Server is accepting TCP connections on the configured port
- Check firewall rules allow connections from your host
- For Azure SQL, ensure your IP is in the firewall allowlist
- If using encryption, ensure `MSSQL_TRUST_CERT=true` for self-signed certificates

### Authentication Failures

- SQL Auth: Verify username and password are correct
- Azure AD: Ensure the service principal has the correct database permissions
- Check the application name in SQL Server logs: `mssql-mcp-server`

### Performance

- Increase `MSSQL_MAX_CONNECTIONS` for high-concurrency workloads
- Enable caching with appropriate `MSSQL_CACHE_TTL` values
- Use `execute_paginated` for large result sets
- Consider `MSSQL_MAX_ROWS` to limit result sizes

### Logging

Enable detailed logging:

```bash
RUST_LOG=debug ./mssql-mcp-server
```

Log levels: `error`, `warn`, `info`, `debug`, `trace`

## License

MIT License - see [LICENSE](LICENSE) for details.

## Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes and add tests
4. Run `just ci` to ensure all checks pass
5. Commit with conventional commit messages (`feat:`, `fix:`, `docs:`, etc.)
6. Push and open a Pull Request
