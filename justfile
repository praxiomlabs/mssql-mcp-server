# MSSQL MCP Server - Development Commands
# Run `just --list` to see all available commands

# Format code using nightly rustfmt
fmt:
    cargo +nightly fmt --all

# Check formatting without making changes
fmt-check:
    cargo +nightly fmt --all -- --check

# Run clippy with all features and strict warnings
check:
    cargo clippy --all-targets --all-features -- -D warnings

# Auto-fix clippy warnings and format code
fix: fmt
    git add ./
    cargo clippy --fix --all-targets --all-features --allow-staged

# Run all tests
test:
    cargo test --all-features

# Run tests with verbose output
test-verbose:
    cargo test --all-features -- --nocapture

# Run integration tests (requires Docker)
test-integration:
    @echo "Running integration tests (requires Docker)..."
    cargo test --test integration_tests -- --ignored --test-threads=1

# Run security audit (cargo-audit) - advisory, see cargo-deny for blocking checks
audit:
    cargo audit || echo "Note: cargo-audit warnings are advisory. See deny.toml for documented exceptions."

# Run comprehensive security and license check (cargo-deny)
deny:
    cargo deny check

# Generate documentation
doc:
    cargo doc --no-deps --all-features

# Open documentation in browser
doc-open:
    cargo doc --no-deps --all-features --open

# Build release binary
build-release:
    cargo build --release --all-features

# Build debug binary
build:
    cargo build --all-features

# Clean build artifacts
clean:
    cargo clean

# Run all CI checks locally (format, clippy, test, audit, deny, doc)
ci: fmt-check check test audit deny doc
    @echo "All CI checks passed!"

# Generate code coverage report (requires cargo-llvm-cov)
cov:
    cargo llvm-cov --all-features --lcov --output-path {{justfile_directory()}}/target/coverage.lcov

# Generate and display code coverage summary
cov-summary:
    cargo llvm-cov --all-features

# Run the server (stdio mode)
run:
    cargo run --all-features

# Run the server with debug logging
run-debug:
    RUST_LOG=debug cargo run --all-features

# Check for outdated dependencies
outdated:
    cargo outdated

# Update dependencies
update:
    cargo update

# Verify the package is ready for publishing
publish-check:
    cargo publish --dry-run

# Show dependency tree
tree:
    cargo tree

# Show dependency tree for a specific feature
tree-feature feature:
    cargo tree --features {{feature}}

# =============================================================================
# Docker Container Management (for local development)
# =============================================================================
# Note: Use `docker compose` (space) not `docker-compose` (hyphen) - Compose v1 is deprecated.

# Start all SQL Server containers
db-up:
    docker compose up -d
    @echo "SQL Server containers starting..."
    @echo "  - 2025: localhost:1433"
    @echo "  - 2022: localhost:1434"
    @echo "  - 2019: localhost:1435"
    @echo "Wait for health checks with: just db-wait"

# Start only SQL Server 2025 (primary target)
db-up-2025:
    docker compose up -d mssql-2025
    @echo "SQL Server 2025 starting on localhost:1433"

# Start only SQL Server 2022
db-up-2022:
    docker compose up -d mssql-2022
    @echo "SQL Server 2022 starting on localhost:1434"

# Start only SQL Server 2019
db-up-2019:
    docker compose up -d mssql-2019
    @echo "SQL Server 2019 starting on localhost:1435"

# Stop all SQL Server containers
db-down:
    docker compose down

# Stop and remove volumes (WARNING: destroys data)
db-destroy:
    @echo "WARNING: This will destroy all SQL Server data!"
    @read -p "Are you sure? (y/N) " confirm && [ "$$confirm" = "y" ]
    docker compose down -v

# View container logs
db-logs:
    docker compose logs -f

# View logs for specific container
db-logs-2025:
    docker compose logs -f mssql-2025

db-logs-2022:
    docker compose logs -f mssql-2022

db-logs-2019:
    docker compose logs -f mssql-2019

# Show container status
db-status:
    docker compose ps

# Wait for all containers to be healthy
db-wait:
    @echo "Waiting for SQL Server containers to be healthy..."
    @docker compose up -d --wait
    @echo "All containers are healthy!"

# Connect to SQL Server 2025 using sqlcmd (requires mssql-tools)
db-connect-2025:
    @echo "Connecting to SQL Server 2025..."
    docker exec -it mssql_2025 /opt/mssql-tools18/bin/sqlcmd -S localhost -U sa -P 'YourStrong@Passw0rd' -C

# Connect to SQL Server 2022 using sqlcmd
db-connect-2022:
    @echo "Connecting to SQL Server 2022..."
    docker exec -it mssql_2022 /opt/mssql-tools18/bin/sqlcmd -S localhost -U sa -P 'YourStrong@Passw0rd' -C

# Connect to SQL Server 2019 using sqlcmd
db-connect-2019:
    @echo "Connecting to SQL Server 2019..."
    docker exec -it mssql_2019 /opt/mssql-tools18/bin/sqlcmd -S localhost -U sa -P 'YourStrong@Passw0rd' -C 2>/dev/null || docker exec -it mssql_2019 /opt/mssql-tools/bin/sqlcmd -S localhost -U sa -P 'YourStrong@Passw0rd'

# Run integration tests against specific SQL Server version
test-integration-2025:
    MSSQL_TEST_VERSION=2025-latest cargo test --test integration_tests -- --ignored --test-threads=1

test-integration-2022:
    MSSQL_TEST_VERSION=2022-latest cargo test --test integration_tests -- --ignored --test-threads=1

test-integration-2019:
    MSSQL_TEST_VERSION=2019-latest cargo test --test integration_tests -- --ignored --test-threads=1

# Run integration tests against all supported versions
test-integration-all:
    @echo "Testing against SQL Server 2025..."
    MSSQL_TEST_VERSION=2025-latest cargo test --test integration_tests -- --ignored --test-threads=1
    @echo ""
    @echo "Testing against SQL Server 2022..."
    MSSQL_TEST_VERSION=2022-latest cargo test --test integration_tests -- --ignored --test-threads=1
    @echo ""
    @echo "Testing against SQL Server 2019..."
    MSSQL_TEST_VERSION=2019-latest cargo test --test integration_tests -- --ignored --test-threads=1
    @echo ""
    @echo "All version tests completed!"

# =============================================================================
# Release Readiness Recipes
# =============================================================================

# Check for WIP markers (TODO, FIXME, XXX, HACK, todo!, unimplemented!)
wip-check:
    @echo "Checking for WIP markers..."
    @! grep -rn "TODO\|FIXME\|XXX\|HACK" --include="*.rs" src/ || echo "No TODO/FIXME/XXX/HACK found"
    @! grep -rn "todo!\|unimplemented!" --include="*.rs" src/ || echo "No todo!/unimplemented! found"

# Audit panic paths (.unwrap(), .expect() in production code)
panic-audit:
    @echo "Auditing panic paths in production code..."
    @grep -rn "\.unwrap()" src/ --include="*.rs" | grep -v "test\|#\[cfg(test)\]" || echo "No .unwrap() found in production code"
    @grep -rn "\.expect(" src/ --include="*.rs" | grep -v "test\|#\[cfg(test)\]" || echo "No .expect() found in production code"

# Check documentation builds without warnings
doc-check:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features

# Run typos spell checker
typos:
    typos src/ README.md CHANGELOG.md

# Check for unused dependencies (requires cargo-machete)
machete:
    cargo machete

# Verify Cargo.toml metadata for crates.io
metadata-check:
    @echo "Checking Cargo.toml metadata..."
    @cargo metadata --no-deps --format-version 1 | jq '.packages[0] | {name, version, description, repository, keywords, categories, license}'

# Create an annotated git tag from Cargo.toml version
tag:
    #!/usr/bin/env bash
    set -euo pipefail
    VERSION=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')
    echo "Creating tag v${VERSION}..."
    git tag -a "v${VERSION}" -m "Release v${VERSION}"
    echo "Tag v${VERSION} created. Push with: git push origin v${VERSION}"

# Full release readiness check
release-check: ci wip-check panic-audit doc-check typos machete deny metadata-check
    @echo ""
    @echo "============================================"
    @echo "Release readiness check completed!"
    @echo "============================================"
    @echo ""
    @echo "Next steps:"
    @echo "1. Review any warnings above"
    @echo "2. Update version in Cargo.toml if needed"
    @echo "3. Update CHANGELOG.md"
    @echo "4. Commit changes: git commit -am 'chore: prepare release vX.Y.Z'"
    @echo "5. Push to main: git push origin main"
    @echo "6. Wait for CI to pass"
    @echo "7. Create tag: just tag"
    @echo "8. Push tag: git push origin vX.Y.Z"
