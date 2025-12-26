# Contributing to mssql-mcp-server

Thank you for your interest in contributing to the SQL Server MCP Server! This document provides guidelines and instructions for contributing.

## Code of Conduct

This project follows the [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct). Please be respectful and constructive in all interactions.

## Getting Started

### Prerequisites

- Rust 1.85 or later
- Git
- Docker (for running SQL Server locally)
- [Just](https://github.com/casey/just) command runner (recommended)

### Setup

1. Fork the repository
2. Clone your fork:
   ```bash
   git clone https://github.com/YOUR_USERNAME/mssql-mcp-server.git
   cd mssql-mcp-server
   ```
3. Add upstream remote:
   ```bash
   git remote add upstream https://github.com/praxiomlabs/mssql-mcp-server.git
   ```
4. Start a local SQL Server instance:
   ```bash
   docker-compose up -d
   ```
5. Copy and configure environment:
   ```bash
   cp .env.example .env
   # Edit .env with your connection settings
   ```
6. Build the project:
   ```bash
   cargo build
   ```
7. Run tests:
   ```bash
   cargo test
   ```

## Development Workflow

### Creating a Branch

```bash
git checkout -b feature/your-feature-name
# or
git checkout -b fix/your-bug-fix
```

### Making Changes

1. Write your code
2. Add tests for new functionality
3. Ensure all tests pass: `cargo test`
4. Check formatting: `cargo fmt --check`
5. Run clippy: `cargo clippy --all-features`
6. Update documentation if needed

### Commit Messages

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
type(scope): description

[optional body]

[optional footer]
```

Types:
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation only
- `style`: Formatting, no code change
- `refactor`: Code change that neither fixes a bug nor adds a feature
- `perf`: Performance improvement
- `test`: Adding tests
- `chore`: Maintenance tasks

Examples:
```
feat(tools): add support for parameterized queries
fix(session): handle connection timeout gracefully
docs(readme): update installation instructions
```

### Pull Requests

1. Push your branch to your fork
2. Create a Pull Request against `main`
3. Fill in the PR template
4. Wait for CI to pass
5. Address review feedback

## Testing

### Running Tests

```bash
# All tests
cargo test

# With output
cargo test -- --nocapture

# Integration tests (requires SQL Server)
cargo test --test '*'
```

### Test Database

Integration tests require a running SQL Server instance. Use Docker:

```bash
docker-compose up -d
```

The default connection string is configured in `.env.example`.

## Code Style

### Formatting

We use `rustfmt` with default settings:

```bash
cargo fmt
```

### Linting

We use strict clippy settings:

```bash
cargo clippy --all-features -- -D warnings
```

## Security

This project handles database connections and SQL queries. Please:

- Never log sensitive data (passwords, connection strings)
- Validate all inputs before constructing queries
- Use parameterized queries to prevent SQL injection
- Report security vulnerabilities privately (see [SECURITY.md](SECURITY.md))

## Getting Help

- **Questions**: Open a [Discussion](https://github.com/praxiomlabs/mssql-mcp-server/discussions)
- **Bugs**: Open an [Issue](https://github.com/praxiomlabs/mssql-mcp-server/issues)
- **Security**: See [SECURITY.md](SECURITY.md)

## Recognition

Contributors are recognized in:
- Git commit history
- Release notes

Thank you for contributing!
