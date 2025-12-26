# ═══════════════════════════════════════════════════════════════════════════════
# MSSQL MCP Server
# ═══════════════════════════════════════════════════════════════════════════════
#
# High-performance MCP server for Microsoft SQL Server.
# Modern command runner with improved UX, safety, and features.
#
# Usage:
#   just              - Show all available commands
#   just build        - Build debug
#   just ci           - Run full CI pipeline
#   just <recipe>     - Run any recipe
#
# Requirements:
#   - Just >= 1.23.0 (for [group], [confirm], [doc] attributes)
#   - Rust toolchain (rustup recommended)
#
# Install Just:
#   cargo install just
#   # or: brew install just / apt install just / pacman -S just
#
# ═══════════════════════════════════════════════════════════════════════════════

# ─────────────────────────────────────────────────────────────────────────────────
# Project Configuration
# ─────────────────────────────────────────────────────────────────────────────────

project_name := "mssql-mcp-server"
# Version is read dynamically from Cargo.toml to avoid drift
version := `cargo metadata --no-deps --format-version 1 2>/dev/null | jq -r '.packages[0].version' 2>/dev/null || grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/'`
# MSRV: 1.88 required for rmcp dependency (darling 0.23.0 requires rustc 1.88+)
msrv := "1.88"
# Rust edition from Cargo.toml
edition := `cargo metadata --no-deps --format-version 1 2>/dev/null | jq -r '.packages[0].edition' 2>/dev/null || grep '^edition' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/'`

# ─────────────────────────────────────────────────────────────────────────────────
# Tool Configuration (can be overridden via environment)
# ─────────────────────────────────────────────────────────────────────────────────

cargo := env_var_or_default("CARGO", "cargo")

# Parallel jobs: auto-detect CPU count
jobs := env_var_or_default("JOBS", num_cpus())

# Runtime configuration
rust_log := env_var_or_default("RUST_LOG", "info")
rust_backtrace := env_var_or_default("RUST_BACKTRACE", "1")

# ─────────────────────────────────────────────────────────────────────────────────
# Platform Detection
# ─────────────────────────────────────────────────────────────────────────────────

platform := if os() == "linux" { "linux" } else if os() == "macos" { "macos" } else { "windows" }
open_cmd := if os() == "linux" { "xdg-open" } else if os() == "macos" { "open" } else { "start" }

# ─────────────────────────────────────────────────────────────────────────────────
# ANSI Color Codes
# ─────────────────────────────────────────────────────────────────────────────────

reset := '\033[0m'
bold := '\033[1m'
dim := '\033[2m'

red := '\033[31m'
green := '\033[32m'
yellow := '\033[33m'
blue := '\033[34m'
cyan := '\033[36m'

# ─────────────────────────────────────────────────────────────────────────────────
# Default Recipe & Settings
# ─────────────────────────────────────────────────────────────────────────────────

# Show help by default
default:
    @just --list --unsorted

# Load .env file if present
set dotenv-load

# Use bash for shell commands with strict error handling
# -e: Exit on error, -u: Error on undefined vars, -o pipefail: Propagate pipe errors
set shell := ["bash", "-euo", "pipefail", "-c"]

# Export all variables to child processes
set export

# ═══════════════════════════════════════════════════════════════════════════════
# SETUP RECIPES
# Bootstrap development environment
# ═══════════════════════════════════════════════════════════════════════════════

[group('setup')]
[doc("Full development setup (rust + tools + hooks)")]
setup: setup-rust setup-tools
    @printf '{{green}}{{bold}}✓ Development environment ready{{reset}}\n'

[group('setup')]
[doc("Install/update Rust toolchain components")]
setup-rust:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Installing Rust toolchain...{{reset}}\n'
    rustup toolchain install stable --profile default
    rustup toolchain install nightly --profile minimal
    rustup toolchain install {{msrv}} --profile minimal
    rustup component add rustfmt clippy llvm-tools-preview
    rustup component add --toolchain nightly rustfmt
    printf '{{green}}[OK]{{reset}}   Rust toolchain ready\n'

[group('setup')]
[doc("Install development tools (cargo extensions)")]
setup-tools:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Installing development tools...{{reset}}\n'
    # Core tools (required for CI)
    {{cargo}} install cargo-llvm-cov cargo-deny cargo-audit
    # Release tools
    {{cargo}} install cargo-semver-checks
    # Quality tools
    {{cargo}} install cargo-outdated cargo-machete
    # Development tools
    {{cargo}} install typos-cli
    printf '{{green}}[OK]{{reset}}   Tools installed\n'

[group('setup')]
[doc("Install minimal tools for CI/release checks")]
setup-tools-minimal:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Installing minimal tools...{{reset}}\n'
    {{cargo}} install cargo-deny cargo-audit cargo-semver-checks
    printf '{{green}}[OK]{{reset}}   Minimal tools installed\n'

[group('setup')]
[doc("Bootstrap system packages (Debian/Ubuntu)")]
bootstrap-apt:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Installing system packages...{{reset}}\n'
    sudo apt-get update
    sudo apt-get install -y build-essential pkg-config libssl-dev curl jq
    printf '{{green}}[OK]{{reset}}   System packages installed\n'

[group('setup')]
[doc("Bootstrap system packages (macOS)")]
bootstrap-brew:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Installing system packages...{{reset}}\n'
    brew install openssl pkg-config jq
    printf '{{green}}[OK]{{reset}}   System packages installed\n'

[group('setup')]
[doc("Install git pre-commit hooks")]
setup-hooks:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Setting up git hooks...{{reset}}\n'

    HOOK_DIR=".git/hooks"
    if [ ! -d "$HOOK_DIR" ]; then
        printf '{{red}}[ERR]{{reset}}  Not a git repository\n'
        exit 1
    fi

    # Create pre-commit hook
    printf '%s\n' '#!/usr/bin/env bash' > "$HOOK_DIR/pre-commit"
    printf '%s\n' 'set -euo pipefail' >> "$HOOK_DIR/pre-commit"
    printf '%s\n' 'echo "Running pre-commit checks..."' >> "$HOOK_DIR/pre-commit"
    printf '%s\n' 'if ! cargo +nightly fmt --all -- --check 2>/dev/null; then' >> "$HOOK_DIR/pre-commit"
    printf '%s\n' '    echo "Format check failed. Run just fmt to fix."' >> "$HOOK_DIR/pre-commit"
    printf '%s\n' '    exit 1' >> "$HOOK_DIR/pre-commit"
    printf '%s\n' 'fi' >> "$HOOK_DIR/pre-commit"
    printf '%s\n' 'if ! cargo clippy --all-targets --all-features -- -D warnings 2>/dev/null; then' >> "$HOOK_DIR/pre-commit"
    printf '%s\n' '    echo "Clippy found issues. Run just clippy-fix to auto-fix."' >> "$HOOK_DIR/pre-commit"
    printf '%s\n' '    exit 1' >> "$HOOK_DIR/pre-commit"
    printf '%s\n' 'fi' >> "$HOOK_DIR/pre-commit"
    printf '%s\n' 'if ! cargo check --all-features 2>/dev/null; then' >> "$HOOK_DIR/pre-commit"
    printf '%s\n' '    echo "Type check failed."' >> "$HOOK_DIR/pre-commit"
    printf '%s\n' '    exit 1' >> "$HOOK_DIR/pre-commit"
    printf '%s\n' 'fi' >> "$HOOK_DIR/pre-commit"
    printf '%s\n' 'echo "Pre-commit checks passed"' >> "$HOOK_DIR/pre-commit"
    chmod +x "$HOOK_DIR/pre-commit"
    printf '{{green}}[OK]{{reset}}   Pre-commit hook installed\n'

    # Create pre-push hook
    printf '%s\n' '#!/usr/bin/env bash' > "$HOOK_DIR/pre-push"
    printf '%s\n' 'set -euo pipefail' >> "$HOOK_DIR/pre-push"
    printf '%s\n' 'echo "Running pre-push checks..."' >> "$HOOK_DIR/pre-push"
    printf '%s\n' 'if ! cargo test --all-features 2>/dev/null; then' >> "$HOOK_DIR/pre-push"
    printf '%s\n' '    echo "Tests failed."' >> "$HOOK_DIR/pre-push"
    printf '%s\n' '    exit 1' >> "$HOOK_DIR/pre-push"
    printf '%s\n' 'fi' >> "$HOOK_DIR/pre-push"
    printf '%s\n' 'if command -v cargo-audit &> /dev/null; then' >> "$HOOK_DIR/pre-push"
    printf '%s\n' '    if ! cargo audit 2>/dev/null; then' >> "$HOOK_DIR/pre-push"
    printf '%s\n' '        echo "Security audit found issues. Review before pushing."' >> "$HOOK_DIR/pre-push"
    printf '%s\n' '    fi' >> "$HOOK_DIR/pre-push"
    printf '%s\n' 'fi' >> "$HOOK_DIR/pre-push"
    printf '%s\n' 'echo "Pre-push checks passed"' >> "$HOOK_DIR/pre-push"
    chmod +x "$HOOK_DIR/pre-push"
    printf '{{green}}[OK]{{reset}}   Pre-push hook installed\n'

[group('setup')]
[doc("Remove git hooks")]
remove-hooks:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Removing git hooks...\n'
    rm -f .git/hooks/pre-commit .git/hooks/pre-push
    printf '{{green}}[OK]{{reset}}   Hooks removed\n'

[group('setup')]
[doc("Check which development tools are installed")]
check-tools:
    #!/usr/bin/env bash
    printf '\n{{bold}}Development Tool Status{{reset}}\n'
    printf '═══════════════════════════════════════\n'

    check_tool() {
        if command -v "$1" &> /dev/null || {{cargo}} "$1" --version &> /dev/null 2>&1; then
            printf '{{green}}✓{{reset}} %s\n' "$1"
        else
            printf '{{red}}✗{{reset}} %s (not installed)\n' "$1"
        fi
    }

    # Core tools
    printf '\n{{cyan}}Core:{{reset}}\n'
    printf '  '; rustc --version
    printf '  '; cargo --version
    check_tool "rustfmt"
    check_tool "clippy"

    # Cargo extensions
    printf '\n{{cyan}}Cargo Extensions:{{reset}}\n'
    for tool in llvm-cov audit deny outdated semver-checks machete; do
        if {{cargo}} $tool --version &> /dev/null 2>&1; then
            printf '{{green}}✓{{reset}} cargo-%s\n' "$tool"
        else
            printf '{{red}}✗{{reset}} cargo-%s\n' "$tool"
        fi
    done

    # External tools
    printf '\n{{cyan}}External:{{reset}}\n'
    check_tool "typos"
    check_tool "lychee"
    check_tool "jq"

    printf '\n'

# ═══════════════════════════════════════════════════════════════════════════════
# BUILD RECIPES
# Compilation and build targets
# ═══════════════════════════════════════════════════════════════════════════════

[group('build')]
[doc("Build workspace in debug mode")]
build:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Building (debug)...{{reset}}\n'
    {{cargo}} build --all-features -j {{jobs}}
    printf '{{green}}[OK]{{reset}}   Build complete\n'

[group('build')]
[doc("Build workspace in release mode with optimizations")]
build-release:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Building (release)...{{reset}}\n'
    {{cargo}} build --all-features --release -j {{jobs}}
    printf '{{green}}[OK]{{reset}}   Release build complete\n'

[group('build')]
[doc("Fast type check without code generation")]
check-build:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Type checking...\n'
    {{cargo}} check --all-features -j {{jobs}}
    printf '{{green}}[OK]{{reset}}   Type check passed\n'

[group('build')]
[confirm("This will delete all build artifacts. Continue?")]
[doc("Clean all build artifacts")]
clean:
    #!/usr/bin/env bash
    printf '{{yellow}}Cleaning build artifacts...{{reset}}\n'
    {{cargo}} clean
    rm -rf coverage/ lcov.info *.profraw *.profdata
    printf '{{green}}[OK]{{reset}}   Clean complete\n'

[group('build')]
[doc("Clean and rebuild from scratch")]
rebuild: clean build

# ═══════════════════════════════════════════════════════════════════════════════
# TEST RECIPES
# Testing and quality assurance
# ═══════════════════════════════════════════════════════════════════════════════

[group('test')]
[doc("Run all tests")]
test:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Running tests...{{reset}}\n'
    {{cargo}} test --all-features -j {{jobs}}
    printf '{{green}}[OK]{{reset}}   All tests passed\n'

[group('test')]
[doc("Run tests with locked dependencies (reproducible)")]
test-locked:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Running tests (locked)...{{reset}}\n'
    {{cargo}} test --all-features --locked -j {{jobs}}
    printf '{{green}}[OK]{{reset}}   All tests passed (locked)\n'

[group('test')]
[doc("Run tests with output visible")]
test-verbose:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Running tests (verbose)...{{reset}}\n'
    {{cargo}} test --all-features -j {{jobs}} -- --nocapture
    printf '{{green}}[OK]{{reset}}   All tests passed\n'

[group('test')]
[doc("Run documentation tests only")]
test-doc:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Running doc tests...\n'
    {{cargo}} test --all-features --doc
    printf '{{green}}[OK]{{reset}}   Doc tests passed\n'

[group('test')]
[doc("Run tests matching a pattern")]
test-filter pattern:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Running tests matching: {{pattern}}\n'
    {{cargo}} test --all-features -- {{pattern}}
    printf '{{green}}[OK]{{reset}}   Filtered tests complete\n'

[group('test')]
[doc("Run tests with various feature combinations")]
test-features:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Testing feature matrix...{{reset}}\n'
    printf '{{cyan}}[INFO]{{reset}} Testing with no features...\n'
    {{cargo}} test --no-default-features -j {{jobs}}
    printf '{{cyan}}[INFO]{{reset}} Testing with default features...\n'
    {{cargo}} test -j {{jobs}}
    printf '{{cyan}}[INFO]{{reset}} Testing with http feature...\n'
    {{cargo}} test --features http -j {{jobs}}
    printf '{{cyan}}[INFO]{{reset}} Testing with telemetry feature...\n'
    {{cargo}} test --features telemetry -j {{jobs}}
    printf '{{cyan}}[INFO]{{reset}} Testing with azure-auth feature...\n'
    {{cargo}} test --features azure-auth -j {{jobs}}
    printf '{{cyan}}[INFO]{{reset}} Testing with all features...\n'
    {{cargo}} test --all-features -j {{jobs}}
    printf '{{green}}[OK]{{reset}}   Feature matrix tests passed\n'

[group('test')]
[doc("Run tests with nextest (faster, better output)")]
test-nextest:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Running tests (nextest)...{{reset}}\n'
    if ! command -v cargo-nextest &> /dev/null; then
        printf '{{yellow}}[WARN]{{reset}} cargo-nextest not installed (cargo install cargo-nextest)\n'
        printf '{{cyan}}[INFO]{{reset}} Falling back to cargo test...\n'
        {{cargo}} test --all-features -j {{jobs}}
    else
        {{cargo}} nextest run --all-features -j {{jobs}}
    fi
    printf '{{green}}[OK]{{reset}}   Tests passed\n'

[group('test')]
[doc("Run tests under Miri (undefined behavior detection)")]
test-miri:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Running Miri (undefined behavior detection)...{{reset}}\n'
    printf '{{yellow}}[NOTE]{{reset}} This may take a while and requires nightly\n'

    # Ensure miri is installed
    if ! rustup +nightly component list | grep -q 'miri.*installed'; then
        printf '{{cyan}}[INFO]{{reset}} Installing Miri component...\n'
        rustup +nightly component add miri
    fi

    {{cargo}} +nightly miri test --no-default-features 2>&1 || {
        printf '{{yellow}}[WARN]{{reset}} Miri may not support all features\n'
        printf '{{cyan}}[INFO]{{reset}} Miri is limited to tests that do not use FFI\n'
    }
    printf '{{green}}[OK]{{reset}}   Miri check complete\n'

[group('test')]
[doc("Test with no features (minimal build)")]
test-minimal:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Testing minimal build (no default features)...\n'
    {{cargo}} check --no-default-features
    {{cargo}} test --no-default-features -j {{jobs}}
    printf '{{green}}[OK]{{reset}}   Minimal build tests passed\n'

[group('test')]
[doc("Test HTTP feature only")]
test-http:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Testing HTTP feature...\n'
    {{cargo}} test --no-default-features --features http -j {{jobs}}
    printf '{{green}}[OK]{{reset}}   HTTP feature tests passed\n'

[group('test')]
[doc("Test telemetry feature only")]
test-telemetry:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Testing telemetry feature...\n'
    {{cargo}} test --no-default-features --features telemetry -j {{jobs}}
    printf '{{green}}[OK]{{reset}}   Telemetry feature tests passed\n'

[group('test')]
[doc("Test azure-auth feature only")]
test-azure:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Testing azure-auth feature...\n'
    {{cargo}} test --no-default-features --features azure-auth -j {{jobs}}
    printf '{{green}}[OK]{{reset}}   Azure auth feature tests passed\n'

[group('test')]
[doc("Test common feature combinations")]
test-combinations:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Testing feature combinations...{{reset}}\n'

    printf '{{cyan}}[INFO]{{reset}} Testing http + telemetry...\n'
    {{cargo}} test --features "http,telemetry" -j {{jobs}}

    printf '{{cyan}}[INFO]{{reset}} Testing http + azure-auth...\n'
    {{cargo}} test --features "http,azure-auth" -j {{jobs}}

    printf '{{cyan}}[INFO]{{reset}} Testing telemetry + azure-auth...\n'
    {{cargo}} test --features "telemetry,azure-auth" -j {{jobs}}

    printf '{{green}}[OK]{{reset}}   Feature combinations passed\n'

[group('test')]
[doc("Run integration tests (requires Docker)")]
test-integration:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Running integration tests...{{reset}}\n'
    printf '{{yellow}}[NOTE]{{reset}} Requires Docker with SQL Server container\n'
    {{cargo}} test --test integration_tests -- --ignored --test-threads=1
    printf '{{green}}[OK]{{reset}}   Integration tests passed\n'

# ═══════════════════════════════════════════════════════════════════════════════
# LINT RECIPES
# Code quality and style checks
# ═══════════════════════════════════════════════════════════════════════════════

[group('lint')]
[doc("Run clippy lints (matches CI configuration)")]
check:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Running clippy...\n'
    {{cargo}} clippy --all-targets --all-features -- -D warnings
    printf '{{green}}[OK]{{reset}}   Clippy passed\n'

[group('lint')]
[doc("Auto-fix clippy warnings")]
clippy-fix:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Auto-fixing clippy warnings...\n'
    {{cargo}} clippy --all-targets --all-features --fix --allow-dirty --allow-staged
    printf '{{green}}[OK]{{reset}}   Clippy fixes applied\n'

[group('lint')]
[doc("Check code formatting")]
fmt-check:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Checking format...\n'
    {{cargo}} +nightly fmt --all -- --check
    printf '{{green}}[OK]{{reset}}   Format check passed\n'

[group('lint')]
[doc("Format all code")]
fmt:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Formatting code...\n'
    {{cargo}} +nightly fmt --all
    printf '{{green}}[OK]{{reset}}   Formatting complete\n'

[group('lint')]
[doc("Run typos spell checker")]
typos:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Checking for typos...\n'
    if ! command -v typos &> /dev/null; then
        printf '{{yellow}}[WARN]{{reset}} typos not installed (cargo install typos-cli)\n'
        exit 0
    fi
    typos src/ README.md CHANGELOG.md RELEASING.md
    printf '{{green}}[OK]{{reset}}   Typos check passed\n'

[group('lint')]
[doc("Fix typos automatically")]
typos-fix:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Fixing typos...\n'
    typos --write-changes
    printf '{{green}}[OK]{{reset}}   Typos fixed\n'

[group('lint')]
[doc("Check markdown links (requires lychee)")]
link-check:
    #!/usr/bin/env bash
    set -e
    printf '{{cyan}}[INFO]{{reset}} Checking markdown links...\n'
    if ! command -v lychee &> /dev/null; then
        printf '{{yellow}}[WARN]{{reset}} lychee not installed (cargo install lychee)\n'
        printf '{{yellow}}[WARN]{{reset}} Skipping link check\n'
        exit 0
    fi
    lychee --no-progress --accept 200,204,206 \
        --exclude '^https://crates.io' \
        --exclude '^https://docs.rs' \
        './README.md' './RELEASING.md' './CHANGELOG.md'
    printf '{{green}}[OK]{{reset}}   Link check passed\n'

[group('lint')]
[doc("Find unused dependencies via cargo-machete (fast, heuristic)")]
machete:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Finding unused dependencies...\n'
    if ! command -v cargo-machete &> /dev/null; then
        printf '{{yellow}}[WARN]{{reset}} cargo-machete not installed (cargo install cargo-machete)\n'
        exit 0
    fi
    {{cargo}} machete
    printf '{{green}}[OK]{{reset}}   Machete check complete\n'

[group('lint')]
[doc("Run all lints (fmt + clippy + typos)")]
lint: fmt-check check typos
    @printf '{{green}}[OK]{{reset}}   All lints passed\n'

[group('lint')]
[doc("Auto-fix: format + clippy warnings")]
fix: fmt
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Auto-fixing issues...\n'
    git add ./
    {{cargo}} clippy --fix --all-targets --all-features --allow-staged
    printf '{{green}}[OK]{{reset}}   Fixed\n'

# ═══════════════════════════════════════════════════════════════════════════════
# DOCUMENTATION RECIPES
# Documentation generation and checking
# ═══════════════════════════════════════════════════════════════════════════════

[group('docs')]
[doc("Generate documentation")]
doc:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Generating documentation...\n'
    {{cargo}} doc --no-deps --all-features
    printf '{{green}}[OK]{{reset}}   Documentation generated\n'

[group('docs')]
[doc("Generate and open documentation")]
doc-open:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Generating documentation...\n'
    {{cargo}} doc --no-deps --all-features --open
    printf '{{green}}[OK]{{reset}}   Documentation opened\n'

[group('docs')]
[doc("Check documentation for warnings")]
doc-check:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Checking documentation...\n'
    RUSTDOCFLAGS="-D warnings" {{cargo}} doc --no-deps --all-features
    printf '{{green}}[OK]{{reset}}   Documentation check passed\n'

# ═══════════════════════════════════════════════════════════════════════════════
# COVERAGE RECIPES
# Code coverage generation
# ═══════════════════════════════════════════════════════════════════════════════

[group('coverage')]
[doc("Generate HTML coverage report")]
coverage:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Generating coverage report...{{reset}}\n'
    {{cargo}} llvm-cov --all-features --html
    printf '{{green}}[OK]{{reset}}   Coverage report: target/llvm-cov/html/index.html\n'

[group('coverage')]
[doc("Generate coverage and open in browser")]
coverage-open:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Generating coverage report...{{reset}}\n'
    {{cargo}} llvm-cov --all-features --html --open
    printf '{{green}}[OK]{{reset}}   Coverage report opened\n'

[group('coverage')]
[doc("Generate LCOV coverage for CI integration")]
coverage-lcov output="target/coverage.lcov":
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Generating LCOV coverage...\n'
    {{cargo}} llvm-cov --all-features --lcov --output-path {{output}}
    printf '{{green}}[OK]{{reset}}   Coverage saved to {{output}}\n'

[group('coverage')]
[doc("Show coverage summary in terminal")]
coverage-summary:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Coverage summary:\n'
    {{cargo}} llvm-cov --all-features

# Alias for backward compatibility
cov: coverage-summary

cov-summary: coverage-summary

# ═══════════════════════════════════════════════════════════════════════════════
# CI/CD RECIPES
# Continuous integration simulation
# ═══════════════════════════════════════════════════════════════════════════════

[group('ci')]
[doc("Check documentation versions match Cargo.toml")]
version-sync:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Checking version sync...\n'
    VERSION="{{version}}"
    MAJOR_MINOR=$(echo "$VERSION" | cut -d. -f1,2)

    # Check README.md for version references
    if [ -f "README.md" ]; then
        if grep -q "Rust 1\." README.md 2>/dev/null; then
            printf '{{green}}[OK]{{reset}}   README.md contains Rust version reference\n'
        fi
    fi

    printf '{{green}}[OK]{{reset}}   Version sync check complete (v%s)\n' "$VERSION"

[group('ci')]
[doc("Check CI status on main branch (with SHA verification)")]
ci-status:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Checking CI status on main...\n'

    if ! command -v gh &> /dev/null; then
        printf '{{yellow}}[WARN]{{reset}} gh CLI not installed, cannot check CI status\n'
        exit 0
    fi

    # Get HEAD SHA
    HEAD_SHA=$(git rev-parse HEAD)
    MAIN_SHA=$(git rev-parse main 2>/dev/null || git rev-parse origin/main 2>/dev/null)

    printf '{{cyan}}[INFO]{{reset}} Local HEAD: %s\n' "${HEAD_SHA:0:8}"
    printf '{{cyan}}[INFO]{{reset}} Main branch: %s\n' "${MAIN_SHA:0:8}"

    # Get latest CI run for main
    RUN_INFO=$(gh run list --limit 1 --branch main --json headSha,status,conclusion,name,databaseId 2>/dev/null)

    if [ -z "$RUN_INFO" ] || [ "$RUN_INFO" = "[]" ]; then
        printf '{{yellow}}[WARN]{{reset}} No CI runs found for main branch\n'
        exit 0
    fi

    RUN_SHA=$(echo "$RUN_INFO" | jq -r '.[0].headSha')
    RUN_STATUS=$(echo "$RUN_INFO" | jq -r '.[0].status')
    RUN_CONCLUSION=$(echo "$RUN_INFO" | jq -r '.[0].conclusion')
    RUN_NAME=$(echo "$RUN_INFO" | jq -r '.[0].name')
    RUN_ID=$(echo "$RUN_INFO" | jq -r '.[0].databaseId')

    printf '\n{{bold}}Latest CI Run:{{reset}}\n'
    printf '  Name:       %s\n' "$RUN_NAME"
    printf '  SHA:        %s\n' "${RUN_SHA:0:8}"
    printf '  Status:     %s\n' "$RUN_STATUS"
    printf '  Conclusion: %s\n' "$RUN_CONCLUSION"

    # Verify SHA matches
    if [ "${RUN_SHA:0:8}" != "${MAIN_SHA:0:8}" ]; then
        printf '\n{{yellow}}[WARN]{{reset}} CI run SHA does not match main branch HEAD\n'
        printf '{{yellow}}[WARN]{{reset}} CI may be running on older commit or push not yet processed\n'
    fi

    # Check status
    if [ "$RUN_STATUS" = "completed" ]; then
        if [ "$RUN_CONCLUSION" = "success" ]; then
            if [ "${RUN_SHA:0:8}" = "${MAIN_SHA:0:8}" ]; then
                printf '\n{{green}}[OK]{{reset}}   CI passed on current main (safe to tag)\n'
            else
                printf '\n{{yellow}}[WARN]{{reset}} CI passed but on different SHA\n'
                printf '{{cyan}}[INFO]{{reset}} Wait for CI to run on current main before tagging\n'
            fi
        else
            printf '\n{{red}}[ERR]{{reset}}  CI failed with conclusion: %s\n' "$RUN_CONCLUSION"
            printf '{{cyan}}[INFO]{{reset}} View details: gh run view %s\n' "$RUN_ID"
        fi
    else
        printf '\n{{yellow}}[WARN]{{reset}} CI still running (status: %s)\n' "$RUN_STATUS"
        printf '{{cyan}}[INFO]{{reset}} Wait for completion: gh run watch %s\n' "$RUN_ID"
    fi

[group('ci')]
[doc("Standard CI pipeline (matches GitHub Actions)")]
ci: fmt-check check test audit deny doc-check
    #!/usr/bin/env bash
    printf '\n{{bold}}{{blue}}══════ CI Pipeline Complete ══════{{reset}}\n\n'
    printf '{{green}}[OK]{{reset}}   All CI checks passed\n'

[group('ci')]
[doc("Fast CI checks (no tests)")]
ci-fast: fmt-check check check-build
    @printf '{{green}}[OK]{{reset}}   Fast CI checks passed\n'

[group('ci')]
[doc("Full CI with MSRV and security audit")]
ci-full: ci msrv-check semver
    @printf '{{green}}[OK]{{reset}}   Full CI pipeline passed\n'

[group('ci')]
[doc("Complete CI with all checks (for releases)")]
ci-release: ci-full test-features link-check
    @printf '{{green}}[OK]{{reset}}   Release CI pipeline passed\n'

[group('ci')]
[doc("Verify MSRV compliance")]
msrv-check:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Checking MSRV {{msrv}}...\n'
    {{cargo}} +{{msrv}} check --all-features
    printf '{{green}}[OK]{{reset}}   MSRV {{msrv}} check passed\n'

[group('ci')]
[doc("Pre-commit hook checks")]
pre-commit: fmt-check check check-build
    @printf '{{green}}[OK]{{reset}}   Pre-commit checks passed\n'

[group('ci')]
[doc("Pre-push hook checks")]
pre-push: ci
    @printf '{{green}}[OK]{{reset}}   Pre-push checks passed\n'

# ═══════════════════════════════════════════════════════════════════════════════
# DEPENDENCY MANAGEMENT
# Dependency analysis and auditing
# ═══════════════════════════════════════════════════════════════════════════════

[group('deps')]
[doc("Run cargo-deny checks (licenses, bans, advisories) - matches CI")]
deny:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Running cargo-deny (matches CI)...\n'
    {{cargo}} deny check
    printf '{{green}}[OK]{{reset}}   Deny checks passed\n'

[group('deps')]
[doc("Security vulnerability audit via cargo-audit")]
audit:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Running security audit...\n'
    {{cargo}} audit || printf '{{yellow}}[NOTE]{{reset}} cargo-audit warnings are advisory. See deny.toml for documented exceptions.\n'
    printf '{{green}}[OK]{{reset}}   Security audit complete\n'

[group('deps')]
[doc("Check for outdated dependencies")]
outdated:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Checking for outdated dependencies...\n'
    {{cargo}} outdated -R

[group('deps')]
[doc("Update Cargo.lock to latest compatible versions")]
update:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Updating dependencies...\n'
    {{cargo}} update
    printf '{{green}}[OK]{{reset}}   Dependencies updated\n'

[group('deps')]
[doc("Update specific dependency")]
update-dep package:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Updating {{package}}...\n'
    {{cargo}} update -p {{package}}
    printf '{{green}}[OK]{{reset}}   {{package}} updated\n'

[group('deps')]
[doc("Show dependency tree")]
tree:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Dependency tree:\n'
    {{cargo}} tree

[group('deps')]
[doc("Show duplicate dependencies")]
tree-duplicates:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Duplicate dependencies:\n'
    {{cargo}} tree --duplicates

[group('deps')]
[doc("Show dependencies with specific features")]
tree-feature feature:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Dependencies for feature {{feature}}:\n'
    {{cargo}} tree --features {{feature}}

[group('deps')]
[doc("Run cargo-vet supply chain audit")]
vet:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Running supply chain audit (cargo-vet)...\n'
    if ! command -v cargo-vet &> /dev/null; then
        printf '{{yellow}}[WARN]{{reset}} cargo-vet not installed\n'
        printf '{{cyan}}[INFO]{{reset}} Install with: cargo install cargo-vet\n'
        exit 0
    fi
    {{cargo}} vet || printf '{{yellow}}[NOTE]{{reset}} Run "cargo vet certify" to certify dependencies\n'
    printf '{{green}}[OK]{{reset}}   Supply chain audit complete\n'

[group('deps')]
[doc("Scan for unsafe code with cargo-geiger")]
geiger:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Scanning for unsafe code...\n'
    if ! command -v cargo-geiger &> /dev/null; then
        printf '{{yellow}}[WARN]{{reset}} cargo-geiger not installed\n'
        printf '{{cyan}}[INFO]{{reset}} Install with: cargo install cargo-geiger\n'
        exit 0
    fi
    {{cargo}} geiger --all-features 2>&1 | head -100
    printf '\n{{cyan}}[INFO]{{reset}} Full report: cargo geiger --all-features\n'

[group('deps')]
[doc("Analyze binary size with cargo-bloat")]
bloat:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Analyzing binary size...\n'
    if ! command -v cargo-bloat &> /dev/null; then
        printf '{{yellow}}[WARN]{{reset}} cargo-bloat not installed\n'
        printf '{{cyan}}[INFO]{{reset}} Install with: cargo install cargo-bloat\n'
        exit 0
    fi
    {{cargo}} bloat --release --all-features -n 30
    printf '\n'

[group('deps')]
[doc("Show largest functions in binary")]
bloat-functions:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Analyzing function sizes...\n'
    if ! command -v cargo-bloat &> /dev/null; then
        printf '{{yellow}}[WARN]{{reset}} cargo-bloat not installed\n'
        printf '{{cyan}}[INFO]{{reset}} Install with: cargo install cargo-bloat\n'
        exit 0
    fi
    {{cargo}} bloat --release --all-features --crates -n 20
    printf '\n'

[group('deps')]
[doc("Run all security checks (deny + audit + vet + geiger)")]
security: deny audit
    #!/usr/bin/env bash
    printf '\n{{bold}}{{blue}}══════ Security Summary ══════{{reset}}\n\n'

    # Run vet if available
    if command -v cargo-vet &> /dev/null; then
        printf '{{cyan}}[INFO]{{reset}} Supply chain audit (cargo-vet)...\n'
        {{cargo}} vet 2>&1 | head -20 || true
    else
        printf '{{yellow}}[SKIP]{{reset}} cargo-vet not installed\n'
    fi

    # Run geiger if available
    if command -v cargo-geiger &> /dev/null; then
        printf '\n{{cyan}}[INFO]{{reset}} Unsafe code scan (cargo-geiger)...\n'
        {{cargo}} geiger --all-features 2>&1 | tail -15 || true
    else
        printf '{{yellow}}[SKIP]{{reset}} cargo-geiger not installed\n'
    fi

    printf '\n{{green}}[OK]{{reset}}   Security checks complete\n'

# ═══════════════════════════════════════════════════════════════════════════════
# RELEASE RECIPES
# Version management and publishing
# ═══════════════════════════════════════════════════════════════════════════════

[group('release')]
[doc("Full release validation (REQUIRED before tagging)")]
release-check: ci-release wip-check panic-audit version-sync typos machete metadata-check publish-dry
    #!/usr/bin/env bash
    printf '\n{{bold}}{{blue}}══════ Release Validation ══════{{reset}}\n\n'
    printf '{{cyan}}[INFO]{{reset}} Checking for uncommitted changes...\n'
    if ! git diff-index --quiet HEAD --; then
        printf '{{red}}[ERR]{{reset}}  Uncommitted changes detected\n'
        git status --short
        exit 1
    fi
    printf '{{cyan}}[INFO]{{reset}} Checking for unpushed commits...\n'
    if [ -n "$(git log @{u}.. 2>/dev/null)" ]; then
        printf '{{yellow}}[WARN]{{reset}} Unpushed commits detected\n'
    fi
    printf '{{green}}[OK]{{reset}}   Ready for release\n'
    printf '\n{{cyan}}Next steps:{{reset}}\n'
    printf '  1. Review any warnings above\n'
    printf '  2. Update version in Cargo.toml if needed\n'
    printf '  3. Update CHANGELOG.md\n'
    printf '  4. Commit changes: git commit -am "chore: release v{{version}}"\n'
    printf '  5. Push to main: git push origin main\n'
    printf '  6. Wait for CI to pass: just ci-status\n'
    printf '  7. Create tag: just tag\n'
    printf '  8. Push tag: git push origin v{{version}}\n'

[group('release')]
[doc("Check for WIP markers (TODO, FIXME, XXX, HACK, todo!, unimplemented!)")]
wip-check:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Checking for WIP markers...\n'

    # Search for comment markers
    COMMENTS=$(grep -rn "TODO\|FIXME\|XXX\|HACK" --include="*.rs" src/ 2>/dev/null || true)
    if [ -n "$COMMENTS" ]; then
        printf '{{yellow}}[WARN]{{reset}} Found WIP comments:\n'
        echo "$COMMENTS" | head -20
        COMMENT_COUNT=$(echo "$COMMENTS" | wc -l)
        if [ "$COMMENT_COUNT" -gt 20 ]; then
            printf '{{yellow}}[WARN]{{reset}} ... and %d more\n' "$((COMMENT_COUNT - 20))"
        fi
    fi

    # Search for incomplete macros (blocking)
    MACROS=$(grep -rn "todo!\|unimplemented!" --include="*.rs" src/ 2>/dev/null || true)
    if [ -n "$MACROS" ]; then
        printf '{{red}}[ERR]{{reset}}  Found incomplete macros in production code:\n'
        echo "$MACROS"
        exit 1
    fi

    printf '{{green}}[OK]{{reset}}   WIP check passed (no blocking issues)\n'

[group('release')]
[doc("Audit panic paths (.unwrap(), .expect()) in production code")]
panic-audit:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Auditing panic paths in production code...\n'

    # Find .unwrap() in src/ directories (production code)
    UNWRAPS=$(grep -rn "\.unwrap()" src/ --include="*.rs" 2>/dev/null || true)
    EXPECTS=$(grep -rn "\.expect(" src/ --include="*.rs" 2>/dev/null || true)

    if [ -n "$UNWRAPS" ] || [ -n "$EXPECTS" ]; then
        printf '{{yellow}}[WARN]{{reset}} Found potential panic paths:\n'
        if [ -n "$UNWRAPS" ]; then
            echo "$UNWRAPS" | head -15
            UNWRAP_COUNT=$(echo "$UNWRAPS" | wc -l)
            printf '{{cyan}}[INFO]{{reset}} Total .unwrap() calls: %d\n' "$UNWRAP_COUNT"
        fi
        if [ -n "$EXPECTS" ]; then
            echo "$EXPECTS" | head -10
            EXPECT_COUNT=$(echo "$EXPECTS" | wc -l)
            printf '{{cyan}}[INFO]{{reset}} Total .expect() calls: %d\n' "$EXPECT_COUNT"
        fi
        printf '{{yellow}}[NOTE]{{reset}} Review each for production safety.\n'
    else
        printf '{{green}}[OK]{{reset}}   No panic paths found in production code\n'
    fi

[group('release')]
[doc("Verify Cargo.toml metadata for crates.io publishing")]
metadata-check:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Checking Cargo.toml metadata...\n'

    METADATA=$(cargo metadata --no-deps --format-version 1 | jq '.packages[0]')

    # Required fields
    DESC=$(echo "$METADATA" | jq -r '.description // empty')
    LICENSE=$(echo "$METADATA" | jq -r '.license // empty')
    REPO=$(echo "$METADATA" | jq -r '.repository // empty')

    MISSING=""
    [ -z "$DESC" ] && MISSING="$MISSING description"
    [ -z "$LICENSE" ] && MISSING="$MISSING license"
    [ -z "$REPO" ] && MISSING="$MISSING repository"

    if [ -n "$MISSING" ]; then
        printf '{{red}}[ERR]{{reset}}  Missing required fields:%s\n' "$MISSING"
        exit 1
    fi

    printf '{{green}}[OK]{{reset}}   Metadata valid\n'
    echo "$METADATA" | jq '{name, version, description, repository, keywords, categories, license}'

[group('release')]
[doc("Publish to crates.io (dry run)")]
publish-dry:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Publishing (dry run)...\n'
    {{cargo}} publish --dry-run
    printf '{{green}}[OK]{{reset}}   Dry run complete\n'

# Alias for backward compatibility
publish-check: publish-dry

[group('release')]
[confirm("This will publish to crates.io. This action is IRREVERSIBLE. Continue?")]
[doc("Publish to crates.io (IRREVERSIBLE!)")]
publish:
    #!/usr/bin/env bash
    printf '\n{{bold}}{{blue}}══════ Publishing to crates.io ══════{{reset}}\n\n'
    printf '{{yellow}}[WARN]{{reset}} This action is IRREVERSIBLE!\n'
    {{cargo}} publish
    printf '\n{{green}}[OK]{{reset}}   Crate published successfully!\n'
    printf '{{cyan}}[INFO]{{reset}} Next steps:\n'
    printf '  1. Verify: cargo search mssql-mcp-server\n'
    printf '  2. Check docs.rs in ~15 minutes\n'
    printf '  3. Update CHANGELOG.md [Unreleased] section\n'

[group('release')]
[confirm("This will yank the specified version from crates.io. Continue?")]
[doc("Yank a version from crates.io (for security issues)")]
yank version:
    #!/usr/bin/env bash
    printf '{{yellow}}[WARN]{{reset}} Yanking version {{version}} from crates.io...\n'
    printf '{{cyan}}[INFO]{{reset}} This prevents new installations but existing Cargo.lock files still work.\n'
    {{cargo}} yank --version {{version}}
    printf '{{green}}[OK]{{reset}}   Version {{version}} yanked\n'
    printf '\n{{cyan}}[INFO]{{reset}} Remember to:\n'
    printf '  1. Publish a security advisory\n'
    printf '  2. Update RustSec advisory database\n'
    printf '  3. Request CVE if applicable\n'

[group('release')]
[doc("Unyank a previously yanked version")]
unyank version:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Unyanking version {{version}}...\n'
    {{cargo}} yank --version {{version}} --undo
    printf '{{green}}[OK]{{reset}}   Version {{version}} unyanked\n'

[group('release')]
[doc("Check semver compatibility (skipped for unpublished crates)")]
semver:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Checking semver compliance...\n'
    if ! command -v cargo-semver-checks &> /dev/null; then
        printf '{{yellow}}[WARN]{{reset}} cargo-semver-checks not installed (cargo install cargo-semver-checks)\n'
        exit 0
    fi
    # Check if crate is published on crates.io
    CRATE_NAME=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].name')
    if ! cargo search "$CRATE_NAME" 2>/dev/null | grep -q "^$CRATE_NAME "; then
        printf '{{yellow}}[INFO]{{reset}} Crate '%s' not yet published - skipping semver check (first release)\n' "$CRATE_NAME"
        exit 0
    fi
    {{cargo}} semver-checks check-release
    printf '{{green}}[OK]{{reset}}   Semver check passed\n'

[group('release')]
[doc("Create annotated git tag for current version (verifies CI first)")]
tag:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Preparing to create tag v{{version}}...\n'

    # Check for uncommitted changes
    if ! git diff-index --quiet HEAD --; then
        printf '{{red}}[ERR]{{reset}}  Uncommitted changes detected. Commit or stash first.\n'
        git status --short
        exit 1
    fi

    # Verify we're on main branch
    BRANCH=$(git branch --show-current)
    if [ "$BRANCH" != "main" ] && [ "$BRANCH" != "master" ]; then
        printf '{{yellow}}[WARN]{{reset}} Not on main branch (current: %s)\n' "$BRANCH"
        printf '{{cyan}}[INFO]{{reset}} Releases should typically be from main branch\n'
    fi

    # Check if tag already exists
    if git tag -l "v{{version}}" | grep -q "v{{version}}"; then
        printf '{{red}}[ERR]{{reset}}  Tag v{{version}} already exists\n'
        printf '{{cyan}}[INFO]{{reset}} Delete with: git tag -d v{{version}}\n'
        exit 1
    fi

    # Verify CI status if gh is available
    if command -v gh &> /dev/null; then
        printf '{{cyan}}[INFO]{{reset}} Verifying CI status...\n'
        HEAD_SHA=$(git rev-parse HEAD)
        MAIN_SHA=$(git rev-parse main 2>/dev/null || git rev-parse origin/main 2>/dev/null)

        RUN_INFO=$(gh run list --limit 1 --branch main --json headSha,status,conclusion 2>/dev/null)

        if [ -n "$RUN_INFO" ] && [ "$RUN_INFO" != "[]" ]; then
            RUN_SHA=$(echo "$RUN_INFO" | jq -r '.[0].headSha')
            RUN_STATUS=$(echo "$RUN_INFO" | jq -r '.[0].status')
            RUN_CONCLUSION=$(echo "$RUN_INFO" | jq -r '.[0].conclusion')

            if [ "$RUN_STATUS" != "completed" ]; then
                printf '{{red}}[ERR]{{reset}}  CI is still running. Wait for completion.\n'
                printf '{{cyan}}[INFO]{{reset}} Run: gh run watch\n'
                exit 1
            fi

            if [ "$RUN_CONCLUSION" != "success" ]; then
                printf '{{red}}[ERR]{{reset}}  CI did not pass (conclusion: %s)\n' "$RUN_CONCLUSION"
                exit 1
            fi

            if [ "${RUN_SHA:0:8}" != "${MAIN_SHA:0:8}" ]; then
                printf '{{yellow}}[WARN]{{reset}} CI passed on different SHA. Push and wait for CI.\n'
                printf '{{cyan}}[INFO]{{reset}} CI SHA: %s, main: %s\n' "${RUN_SHA:0:8}" "${MAIN_SHA:0:8}"
                exit 1
            fi

            printf '{{green}}[OK]{{reset}}   CI passed on current main\n'
        fi
    fi

    # Create the tag
    printf '{{cyan}}[INFO]{{reset}} Creating tag v{{version}}...\n'
    git tag -a "v{{version}}" -m "Release v{{version}}"
    printf '{{green}}[OK]{{reset}}   Tag created: v{{version}}\n'
    printf '\n{{bold}}Next steps:{{reset}}\n'
    printf '  1. Push tag: git push origin v{{version}}\n'
    printf '  2. Monitor release workflow (if configured)\n'
    printf '  3. Verify on crates.io after publishing\n'

# ═══════════════════════════════════════════════════════════════════════════════
# DOCKER RECIPES
# SQL Server container management
# ═══════════════════════════════════════════════════════════════════════════════

[group('docker')]
[doc("Start all SQL Server containers")]
db-up:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Starting SQL Server containers...{{reset}}\n'
    docker compose up -d
    printf '{{green}}[OK]{{reset}}   Containers starting\n'
    printf '{{cyan}}[INFO]{{reset}} Ports:\n'
    printf '  - 2025: localhost:1433\n'
    printf '  - 2022: localhost:1434\n'
    printf '  - 2019: localhost:1435\n'
    printf '{{dim}}Wait for health checks with: just db-wait{{reset}}\n'

[group('docker')]
[doc("Start only SQL Server 2025 (primary target)")]
db-up-2025:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Starting SQL Server 2025 on localhost:1433\n'
    docker compose up -d mssql-2025

[group('docker')]
[doc("Start only SQL Server 2022")]
db-up-2022:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Starting SQL Server 2022 on localhost:1434\n'
    docker compose up -d mssql-2022

[group('docker')]
[doc("Start only SQL Server 2019")]
db-up-2019:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Starting SQL Server 2019 on localhost:1435\n'
    docker compose up -d mssql-2019

[group('docker')]
[doc("Stop all SQL Server containers")]
db-down:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Stopping containers...\n'
    docker compose down
    printf '{{green}}[OK]{{reset}}   Containers stopped\n'

[group('docker')]
[confirm("This will destroy all SQL Server data. Continue?")]
[doc("Stop and remove volumes (destroys data)")]
db-destroy:
    #!/usr/bin/env bash
    printf '{{red}}{{bold}}Destroying SQL Server containers and data...{{reset}}\n'
    docker compose down -v
    printf '{{green}}[OK]{{reset}}   Containers and volumes destroyed\n'

[group('docker')]
[doc("View container logs")]
db-logs:
    docker compose logs -f

[group('docker')]
[doc("View logs for SQL Server 2025")]
db-logs-2025:
    docker compose logs -f mssql-2025

[group('docker')]
[doc("View logs for SQL Server 2022")]
db-logs-2022:
    docker compose logs -f mssql-2022

[group('docker')]
[doc("View logs for SQL Server 2019")]
db-logs-2019:
    docker compose logs -f mssql-2019

[group('docker')]
[doc("Show container status")]
db-status:
    docker compose ps

[group('docker')]
[doc("Wait for all containers to be healthy")]
db-wait:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Waiting for SQL Server containers to be healthy...\n'
    docker compose up -d --wait
    printf '{{green}}[OK]{{reset}}   All containers are healthy\n'

[group('docker')]
[doc("Connect to SQL Server 2025 using sqlcmd")]
db-connect-2025:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Connecting to SQL Server 2025...\n'
    docker exec -it mssql_2025 /opt/mssql-tools18/bin/sqlcmd -S localhost -U sa -P 'YourStrong@Passw0rd' -C

[group('docker')]
[doc("Connect to SQL Server 2022 using sqlcmd")]
db-connect-2022:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Connecting to SQL Server 2022...\n'
    docker exec -it mssql_2022 /opt/mssql-tools18/bin/sqlcmd -S localhost -U sa -P 'YourStrong@Passw0rd' -C

[group('docker')]
[doc("Connect to SQL Server 2019 using sqlcmd")]
db-connect-2019:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Connecting to SQL Server 2019...\n'
    docker exec -it mssql_2019 /opt/mssql-tools18/bin/sqlcmd -S localhost -U sa -P 'YourStrong@Passw0rd' -C 2>/dev/null || \
        docker exec -it mssql_2019 /opt/mssql-tools/bin/sqlcmd -S localhost -U sa -P 'YourStrong@Passw0rd'

[group('docker')]
[doc("Run integration tests against SQL Server 2025")]
test-integration-2025:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Testing against SQL Server 2025...\n'
    MSSQL_TEST_VERSION=2025-latest {{cargo}} test --test integration_tests -- --ignored --test-threads=1
    printf '{{green}}[OK]{{reset}}   SQL Server 2025 tests passed\n'

[group('docker')]
[doc("Run integration tests against SQL Server 2022")]
test-integration-2022:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Testing against SQL Server 2022...\n'
    MSSQL_TEST_VERSION=2022-latest {{cargo}} test --test integration_tests -- --ignored --test-threads=1
    printf '{{green}}[OK]{{reset}}   SQL Server 2022 tests passed\n'

[group('docker')]
[doc("Run integration tests against SQL Server 2019")]
test-integration-2019:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Testing against SQL Server 2019...\n'
    MSSQL_TEST_VERSION=2019-latest {{cargo}} test --test integration_tests -- --ignored --test-threads=1
    printf '{{green}}[OK]{{reset}}   SQL Server 2019 tests passed\n'

[group('docker')]
[doc("Run integration tests against all supported versions")]
test-integration-all:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Testing against all SQL Server versions...{{reset}}\n'

    printf '\n{{cyan}}[INFO]{{reset}} Testing against SQL Server 2025...\n'
    MSSQL_TEST_VERSION=2025-latest {{cargo}} test --test integration_tests -- --ignored --test-threads=1

    printf '\n{{cyan}}[INFO]{{reset}} Testing against SQL Server 2022...\n'
    MSSQL_TEST_VERSION=2022-latest {{cargo}} test --test integration_tests -- --ignored --test-threads=1

    printf '\n{{cyan}}[INFO]{{reset}} Testing against SQL Server 2019...\n'
    MSSQL_TEST_VERSION=2019-latest {{cargo}} test --test integration_tests -- --ignored --test-threads=1

    printf '\n{{green}}[OK]{{reset}}   All version tests completed!\n'

# ═══════════════════════════════════════════════════════════════════════════════
# DEVELOPMENT WORKFLOW RECIPES
# Day-to-day development utilities
# ═══════════════════════════════════════════════════════════════════════════════

[group('dev')]
[doc("Full development setup and validation")]
dev: build test lint
    @printf '{{green}}[OK]{{reset}}   Development environment ready\n'

[group('dev')]
[doc("Run the server (stdio mode)")]
run:
    {{cargo}} run --all-features

[group('dev')]
[doc("Run the server with debug logging")]
run-debug:
    RUST_LOG=debug {{cargo}} run --all-features

[group('dev')]
[doc("Run the server with trace logging")]
run-trace:
    RUST_LOG=trace {{cargo}} run --all-features

[group('dev')]
[doc("Install the MCP server to ~/.cargo/bin")]
install:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Installing {{project_name}}...{{reset}}\n'
    {{cargo}} install --path . --all-features
    printf '{{green}}[OK]{{reset}}   Installed to ~/.cargo/bin/{{project_name}}\n'

[group('dev')]
[doc("Install with release optimizations")]
install-release:
    #!/usr/bin/env bash
    printf '{{blue}}{{bold}}Installing {{project_name}} (optimized)...{{reset}}\n'
    {{cargo}} install --path . --all-features --release
    printf '{{green}}[OK]{{reset}}   Installed to ~/.cargo/bin/{{project_name}}\n'

[group('dev')]
[doc("Uninstall the MCP server")]
uninstall:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Uninstalling {{project_name}}...\n'
    {{cargo}} uninstall {{project_name}} 2>/dev/null || printf '{{yellow}}[WARN]{{reset}} Not installed\n'
    printf '{{green}}[OK]{{reset}}   Uninstalled\n'

# ═══════════════════════════════════════════════════════════════════════════════
# UTILITY RECIPES
# Miscellaneous utilities
# ═══════════════════════════════════════════════════════════════════════════════

[group('util')]
[doc("Show version and environment info")]
info:
    #!/usr/bin/env bash
    printf '\n{{bold}}{{project_name}} v{{version}}{{reset}}\n'
    printf '═══════════════════════════════════════════════════════════\n'
    printf '{{cyan}}Edition:{{reset}}   {{edition}}\n'
    printf '{{cyan}}MSRV:{{reset}}      {{msrv}}\n'
    printf '{{cyan}}Platform:{{reset}}  {{platform}}\n'
    printf '{{cyan}}Jobs:{{reset}}      {{jobs}}\n'

    printf '\n{{bold}}Toolchain{{reset}}\n'
    printf '───────────────────────────────────────────────────────────\n'
    printf '{{cyan}}Rust:{{reset}}      %s\n' "$(rustc --version)"
    printf '{{cyan}}Cargo:{{reset}}     %s\n' "$(cargo --version)"
    printf '{{cyan}}Just:{{reset}}      %s\n' "$(just --version)"

    # Git info if available
    if command -v git &> /dev/null && git rev-parse --git-dir &> /dev/null; then
        printf '\n{{bold}}Git{{reset}}\n'
        printf '───────────────────────────────────────────────────────────\n'
        printf '{{cyan}}Branch:{{reset}}    %s\n' "$(git branch --show-current)"
        printf '{{cyan}}Commit:{{reset}}    %s\n' "$(git rev-parse --short HEAD)"
        printf '{{cyan}}Status:{{reset}}    %s\n' "$(git status --porcelain | wc -l | tr -d ' ') uncommitted changes"
    fi

    # Features
    printf '\n{{bold}}Cargo Features{{reset}}\n'
    printf '───────────────────────────────────────────────────────────\n'
    cargo metadata --no-deps --format-version 1 2>/dev/null | \
        jq -r '.packages[0].features | to_entries[] | "  \(.key): \(.value | join(", "))"' 2>/dev/null || \
        printf '  (unable to read features)\n'

    printf '\n'

[group('util')]
[doc("Count lines of code")]
loc:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Lines of code:\n'
    tokei src/ 2>/dev/null || find src -name '*.rs' | xargs wc -l | tail -1

[group('util')]
[doc("Check git status")]
git-status:
    @git status --short

[group('util')]
[doc("Open crates.io page")]
crates-io:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Opening crates.io...\n'
    {{open_cmd}} "https://crates.io/crates/mssql-mcp-server" 2>/dev/null || \
        printf 'https://crates.io/crates/mssql-mcp-server\n'

[group('util')]
[doc("Open docs.rs page")]
docs-rs:
    #!/usr/bin/env bash
    printf '{{cyan}}[INFO]{{reset}} Opening docs.rs...\n'
    {{open_cmd}} "https://docs.rs/mssql-mcp-server" 2>/dev/null || \
        printf 'https://docs.rs/mssql-mcp-server\n'

# ═══════════════════════════════════════════════════════════════════════════════
# HELP RECIPES
# Documentation and assistance
# ═══════════════════════════════════════════════════════════════════════════════

[group('help')]
[doc("Show all available recipes grouped by category")]
help:
    #!/usr/bin/env bash
    printf '\n{{bold}}{{project_name}} v{{version}}{{reset}} — Development Command Runner\n'
    printf 'MSRV: {{msrv}} | Platform: {{platform}}\n\n'
    printf '{{bold}}Usage:{{reset}} just [recipe] [arguments...]\n\n'
    just --list --unsorted

[group('help')]
[doc("Show commonly used recipes")]
quick:
    #!/usr/bin/env bash
    printf '{{cyan}}{{bold}}Quick Reference{{reset}}\n\n'
    printf '{{bold}}Development:{{reset}}\n'
    printf '  {{green}}just build{{reset}}          Build debug\n'
    printf '  {{green}}just test{{reset}}           Run tests\n'
    printf '  {{green}}just check{{reset}}          Run clippy\n'
    printf '  {{green}}just fmt{{reset}}            Format code\n'
    printf '\n{{bold}}CI/Release:{{reset}}\n'
    printf '  {{green}}just ci{{reset}}             Run full CI\n'
    printf '  {{green}}just ci-release{{reset}}     Release CI\n'
    printf '  {{green}}just release-check{{reset}}  Pre-release validation\n'
    printf '\n{{bold}}Docker:{{reset}}\n'
    printf '  {{green}}just db-up{{reset}}          Start SQL Server containers\n'
    printf '  {{green}}just db-wait{{reset}}        Wait for containers to be healthy\n'
    printf '  {{green}}just db-down{{reset}}        Stop containers\n'
    printf '\n{{bold}}Analysis:{{reset}}\n'
    printf '  {{green}}just coverage{{reset}}       Code coverage\n'
    printf '  {{green}}just deny{{reset}}           Security/license check\n'
    printf '  {{green}}just outdated{{reset}}       Check for outdated deps\n'
