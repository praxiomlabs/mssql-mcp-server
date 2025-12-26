# Releasing mssql-mcp-server

Comprehensive guide for releasing new versions of mssql-mcp-server to crates.io.

**Version:** 0.1.0 | **MSRV:** 1.88 | **Edition:** 2024

---

## Critical: Read Before Any Release

### The Cardinal Rules

1. **NEVER manually run `cargo publish`** — Always use the automated GitHub Actions workflow triggered by pushing a version tag. The `just publish` recipe exists only for disaster recovery.

2. **NEVER push a tag until CI passes on main** — Always run `gh run watch` or `just ci-status` to verify CI passed before creating a tag.

3. **ALWAYS run `just release-check` before tagging** — This validates ALL features, documentation, and publishing requirements.

4. **Publishing to crates.io is IRREVERSIBLE** — You can yank a version, but you cannot delete or re-upload it. A yanked version still counts as "used" forever.

### Pre-Release Verification Checklist

Before creating a tag, **always** verify:

```bash
# 1. Run the FULL release check (validates ALL features)
just release-check

# 2. Verify CI passed on main (blocking check)
just ci-status  # Must show "completed" with green check

# 3. Only then create the tag
just tag
git push origin vX.Y.Z
```

---

## Quick Start

For routine releases:

```bash
# 1. Validate everything is ready
just release-check

# 2. Bump version and update CHANGELOG (manual edit)
#    - Edit Cargo.toml: version = "X.Y.Z"
#    - Update CHANGELOG.md with release date

# 3. Commit, push, and wait for CI
git add Cargo.toml CHANGELOG.md
git commit -m "chore: release vX.Y.Z"
git push origin main
gh run watch  # Wait for CI to pass

# 4. Tag and release
just tag                    # Creates annotated tag vX.Y.Z
git push origin vX.Y.Z      # Triggers release workflow (if configured)

# 5. Publish (if not automated via CI)
just publish                # Will ask for confirmation
```

---

## Table of Contents

1. [CI Parity](#ci-parity)
2. [Version Numbering](#version-numbering)
3. [Pre-Release Checklist](#pre-release-checklist)
4. [Release Workflow](#release-workflow)
5. [Automated vs Manual Release](#automated-vs-manual-release)
6. [Post-Release Verification](#post-release-verification)
7. [CI Automation Coverage](#ci-automation-coverage)
8. [Manual Recovery Procedures](#manual-recovery-procedures)
9. [Justfile Recipe Reference](#justfile-recipe-reference)
10. [Troubleshooting](#troubleshooting)
11. [Platform-Specific Notes](#platform-specific-notes)
12. [Feature-Specific Testing](#feature-specific-testing)
13. [Security Incident Response](#security-incident-response)
14. [Lessons Learned](#lessons-learned)
15. [Release Checklist Template](#release-checklist-template)

---

## CI Parity

### Local Must Match CI

**The local `just ci` command must produce identical results to the GitHub Actions CI pipeline.** This is critical because:

1. If local passes but CI fails, you'll waste time waiting for CI feedback
2. If local fails but CI passes, you're testing against the wrong environment
3. Discrepancies hide bugs that only appear in production/release builds

### Local vs CI Command Mapping

| Check | Local Command | CI Job | Flags | Must Match |
|-------|---------------|--------|-------|------------|
| Format | `just fmt-check` | `fmt-check` | `+nightly --check` | ✅ Yes |
| Clippy | `just check` | `check` | `--all-targets --all-features -D warnings` | ✅ Yes |
| Tests | `just test` | `test` | `--all-features` | ✅ Yes |
| Doc build | `just doc-check` | `doc` | `-D warnings --all-features` | ✅ Yes |
| Deny | `just deny` | `deny` | (uses deny.toml) | ✅ Yes |
| Audit | `just audit` | `audit` | (advisory-db) | ✅ Yes |
| MSRV | `just msrv-check` | `msrv` | `+1.88 --all-features` | ✅ Yes |
| Semver | `just semver` | `semver` | (baseline: crates.io) | ✅ Yes (if published) |

### Key Commands

```bash
# Run the same checks as CI
just ci

# Run with locked Cargo.lock (same as CI)
just test-locked

# Run full release CI pipeline
just ci-release

# Verify CI status with SHA check
just ci-status
```

### Common Parity Issues

1. **Different Rust versions** — Use `rustup override set stable` in project directory
2. **Different Cargo.lock** — Always commit Cargo.lock, use `--locked` flag
3. **Missing tools** — Run `just setup` to install all required tools
4. **Feature flags** — Always use `--all-features` to match CI configuration
5. **Nightly formatter** — Ensure `rustup +nightly component add rustfmt` is installed
6. **Platform differences** — Some tests may behave differently on Linux vs macOS vs Windows

---

## Version Numbering

We follow [Semantic Versioning 2.0.0](https://semver.org/):

- **MAJOR.MINOR.PATCH** (e.g., 1.2.3)
- **Pre-1.0**: Minor bumps may contain breaking changes
- **Post-1.0**: Strictly follow semver

### Version Bump Guidelines

| Change Type | Version Bump | Example |
|-------------|--------------|---------|
| Breaking API change | MAJOR | 1.0.0 → 2.0.0 |
| Breaking MCP protocol change | MAJOR | 1.0.0 → 2.0.0 |
| New MCP tool/resource | MINOR | 0.1.0 → 0.2.0 |
| New feature (non-breaking) | MINOR | 0.1.0 → 0.2.0 |
| Bug fix, security patch | PATCH | 0.1.0 → 0.1.1 |
| Documentation only | PATCH | 0.1.0 → 0.1.1 |
| Performance improvement | PATCH | 0.1.0 → 0.1.1 |

---

## Pre-Release Checklist

### 0. Pre-flight Checks

```bash
just release-check  # Comprehensive validation
```

- [ ] Git working directory is clean (`git status`)
- [ ] On `main` branch
- [ ] CI is passing on main branch (`just ci-status`)
- [ ] `just release-check` completes successfully

### 1. Codebase Hygiene

```bash
just wip-check      # TODO/FIXME/XXX/HACK, todo!/unimplemented!
just panic-audit    # .unwrap()/.expect() audit
```

- [ ] No blocking `todo!()` or `unimplemented!()` in production code
- [ ] All `.unwrap()` and `.expect()` calls reviewed for safety
- [ ] WIP comments reviewed (TODO, FIXME, XXX, HACK)

### 2. Code Quality

```bash
just check          # Clippy with warnings-as-errors
just typos          # Spell checking
```

- [ ] No clippy warnings
- [ ] No typos in code or documentation

### 3. Testing

```bash
just test           # Unit tests
just test-features  # All feature combinations
just test-integration  # Integration tests (optional, requires Docker)
```

- [ ] All tests pass
- [ ] All feature combinations compile

### 4. Version Consistency

```bash
just version-sync   # Check README matches Cargo.toml
```

Verify version is consistent in:
- [ ] `Cargo.toml` version field
- [ ] README.md installation instructions (if version-specific)
- [ ] CHANGELOG.md has entry with correct date

### 5. Security & Dependencies

```bash
just deny    # Licenses, bans, advisories
just audit   # Security vulnerabilities
just machete # Unused dependencies
```

- [ ] No license violations
- [ ] No banned dependencies
- [ ] No unaddressed security advisories (or documented in `deny.toml`)
- [ ] No unused dependencies

### 6. Documentation

```bash
just doc-check   # Documentation builds without warnings
just link-check  # Markdown link validation (if lychee installed)
```

- [ ] Documentation builds without warnings
- [ ] CHANGELOG.md updated with new version section
- [ ] Breaking changes have migration notes
- [ ] All public APIs documented

### 7. API Compatibility

```bash
just semver    # Breaking change detection
```

- [ ] No unintended breaking changes
- [ ] Version bump accounts for any breaking changes
- [ ] Public API surface reviewed
- [ ] MCP tool/resource changes documented

### 8. MSRV Compliance

```bash
just msrv-check    # Compile with declared MSRV
```

- [ ] Compiles with MSRV (1.88)
- [ ] No features requiring newer Rust version

### 9. Build Verification

```bash
just ci-release    # Full CI simulation
```

- [ ] Full CI pipeline passes
- [ ] Release builds succeed

### 10. Publishing Preparation

```bash
just metadata-check   # Verify crates.io metadata
just publish-dry      # Dry-run publish
```

- [ ] Required metadata present (description, license, repository)
- [ ] Keywords and categories appropriate
- [ ] Dry-run publish succeeds

---

## Release Workflow

**Publishing to crates.io is IRREVERSIBLE.** Follow this exact sequence:

```
┌─────────────────────────────────────────────────────────────┐
│  1. PREPARE: just release-check                             │
│                         ↓                                   │
│  2. VERSION: Edit Cargo.toml + CHANGELOG                    │
│                         ↓                                   │
│  3. COMMIT: git commit -m "chore: release vX.Y.Z"           │
│                         ↓                                   │
│  4. PUSH: git push origin main                              │
│                         ↓                                   │
│  5. WAIT: gh run watch (CI must pass)                       │
│                         ↓                                   │
│  6. TAG: just tag (creates vX.Y.Z)                          │
│                         ↓                                   │
│  7. RELEASE: git push origin vX.Y.Z                         │
│              (triggers automated release if configured)     │
│                         ↓                                   │
│  8. PUBLISH: just publish (if not automated)                │
└─────────────────────────────────────────────────────────────┘
```

### Step-by-Step Commands

#### Step 1: Pre-release Validation

```bash
just release-check
```

#### Step 2: Prepare Version

Edit `Cargo.toml`:

```toml
[package]
version = "X.Y.Z"
```

Update `CHANGELOG.md`:

```markdown
## [Unreleased]

## [X.Y.Z] - YYYY-MM-DD

### Added
- ...

### Changed
- ...

### Fixed
- ...

[Unreleased]: https://github.com/jkindrix/mssql-mcp-server/compare/vX.Y.Z...HEAD
[X.Y.Z]: https://github.com/jkindrix/mssql-mcp-server/compare/vPREV...vX.Y.Z
```

#### Step 3-5: Commit, Push, and Wait

```bash
git add Cargo.toml CHANGELOG.md
git commit -m "chore: release vX.Y.Z"
git push origin main

# Wait for CI to pass
gh run watch                    # Interactive watch
# OR
gh run list --limit 1           # Check status
# OR
just ci-status                  # Quick check
```

#### Step 6-7: Tag and Release

```bash
# ONLY after CI passes on main!
just tag                        # Creates annotated tag vX.Y.Z
git push origin vX.Y.Z          # Triggers release workflow (if configured)
```

#### Step 8: Publish (if not automated)

```bash
just publish                    # Will ask for confirmation
```

---

## Automated vs Manual Release

| Aspect | Automated (tag push) | Manual (`just publish`) |
|--------|---------------------|------------------------|
| **Trigger** | Push `vX.Y.Z` tag | Run command locally |
| **CI checks** | Run automatically before publish | Must run manually first |
| **GitHub Release** | Created automatically (if configured) | Must create manually |
| **Recommended** | ✅ **Always use this** | ⚠️ Last resort only |

**Recommendation:** Always use automated release via tag push when available. Only use manual process for disaster recovery when CI is down or partially failed.

---

## Post-Release Verification

### Immediate Checks (within 5 minutes)

```bash
# Verify crate is on crates.io
cargo search mssql-mcp-server

# Verify crate is usable
cd /tmp && cargo new test-release && cd test-release
cargo add mssql-mcp-server@X.Y.Z
cargo check
cd - && rm -rf /tmp/test-release

# Verify GitHub release was created (if automated)
gh release view vX.Y.Z
```

- [ ] `cargo search mssql-mcp-server` shows correct version
- [ ] `cargo add` works in fresh project
- [ ] GitHub release exists (if automated workflow configured)

### Delayed Checks (15-30 minutes)

```bash
# Check docs.rs (takes time to build)
curl -I https://docs.rs/mssql-mcp-server/X.Y.Z

# Check badges
curl -I https://img.shields.io/crates/v/mssql-mcp-server.svg
```

- [ ] docs.rs documentation is built and accessible
- [ ] README badges show correct version (if present)

### Repository Cleanup

- [ ] Update `[Unreleased]` section in CHANGELOG for next cycle
- [ ] Close related milestones/issues
- [ ] Announce release (if applicable)

---

## CI Automation Coverage

The following checks are **automated in CI** (triggered on push/PR to main):

| Check | CI Job | Local Recipe | Trigger |
|-------|--------|--------------|---------|
| Format | `fmt-check` | `just fmt-check` | Push/PR |
| Clippy | `check` | `just check` | Push/PR |
| Tests | `test` | `just test` | Push/PR |
| Doc build | `doc` | `just doc-check` | Push/PR |
| License/deps | `deny` | `just deny` | Push/PR |
| Security audit | `audit` | `just audit` | Push/PR |
| MSRV | `msrv` | `just msrv-check` | Push/PR |
| Semver check | `semver` | `just semver` | Push/PR (if published) |

**Release workflow (on tag push, if configured):**

| Step | Automated |
|------|-----------|
| Create GitHub release | ✅ |
| Publish to crates.io | ✅ |

**Still requires manual verification:**
- Version string updates in documentation
- Post-release installation test
- docs.rs build verification
- Announcement/communication

---

## Manual Recovery Procedures

> **⚠️ WARNING: Manual publishing should be a LAST RESORT only.**
>
> The automated GitHub Actions workflow is the **only** sanctioned way to publish.
> Manual publishing bypasses CI checks and has historically caused broken releases.
>
> **Only use manual publishing when:**
> - GitHub Actions is completely down/unavailable
> - You have explicitly verified ALL checks pass locally with `just release-check`

### If Tag Was Pushed But Release Failed Completely

```bash
# 1. Delete the remote tag
git push --delete origin vX.Y.Z

# 2. Delete the local tag
git tag -d vX.Y.Z

# 3. Fix the issue

# 4. Recreate tag and push
just tag
git push origin vX.Y.Z
```

### If GitHub Release Was Created But Publish Failed

```bash
# 1. Delete the GitHub release (keeps tag)
gh release delete vX.Y.Z --yes

# 2. Fix the issue and re-trigger workflow
git push --delete origin vX.Y.Z
git push origin vX.Y.Z

# OR manually publish
just publish
```

### Rate Limited by crates.io

**Cause:** crates.io limits new crate publications.

**Fix:** Wait for the time specified in the error message, then retry:

```bash
just publish
```

---

## Justfile Recipe Reference

### Setup & Bootstrap

| Recipe | What It Does |
|--------|--------------|
| `just setup` | Full development setup (rust + tools) |
| `just setup-rust` | Install/update Rust toolchain |
| `just setup-tools` | Install cargo extensions |
| `just setup-hooks` | Install git pre-commit/pre-push hooks |
| `just bootstrap-apt` | Bootstrap system packages (Debian/Ubuntu) |
| `just bootstrap-brew` | Bootstrap system packages (macOS) |
| `just check-tools` | Check which tools are installed |

### Pre-Release Validation

| Recipe | What It Does |
|--------|--------------|
| `just release-check` | Complete release readiness check ⭐ |
| `just ci-status` | Check CI passed on main (with SHA verification) |
| `just wip-check` | Find TODO/FIXME/todo!/unimplemented! |
| `just panic-audit` | Find .unwrap()/.expect() |
| `just typos` | Spell check code and docs |

### CI Simulation

| Recipe | What It Does |
|--------|--------------|
| `just ci` | Match GitHub Actions CI |
| `just ci-fast` | Quick checks (no tests) |
| `just ci-full` | CI + MSRV + semver |
| `just ci-release` | Full release validation ⭐ |
| `just pre-commit` | Pre-commit hook checks |
| `just pre-push` | Pre-push hook checks |

### Testing

| Recipe | What It Does |
|--------|--------------|
| `just test` | Run all tests |
| `just test-nextest` | Run tests with nextest (faster) |
| `just test-miri` | Run tests under Miri (undefined behavior) |
| `just test-features` | Test all feature combinations |
| `just test-minimal` | Test with no features (minimal build) |
| `just test-http` | Test HTTP feature only |
| `just test-telemetry` | Test telemetry feature only |
| `just test-azure` | Test azure-auth feature only |
| `just test-combinations` | Test common feature combinations |
| `just test-integration` | Run integration tests (Docker required) |

### Security & Dependencies

| Recipe | What It Does |
|--------|--------------|
| `just security` | Run all security checks (bundled) ⭐ |
| `just deny` | License/ban/advisory check |
| `just audit` | Security vulnerability scan |
| `just vet` | Supply chain audit (cargo-vet) |
| `just geiger` | Scan for unsafe code |
| `just bloat` | Analyze binary size |
| `just machete` | Fast unused dependency check |
| `just tree-duplicates` | Show duplicate dependencies |
| `just outdated` | Check for outdated dependencies |

### Documentation

| Recipe | What It Does |
|--------|--------------|
| `just doc` | Generate documentation |
| `just doc-check` | Build docs without warnings |
| `just link-check` | Validate markdown links |
| `just version-sync` | Check version consistency |

### Semver & Compatibility

| Recipe | What It Does |
|--------|--------------|
| `just semver` | Breaking change detection |
| `just msrv-check` | Compile with MSRV |
| `just test-features` | Check feature combinations |

### Publishing

| Recipe | What It Does |
|--------|--------------|
| `just metadata-check` | Verify crates.io metadata |
| `just publish-dry` | Test publish without uploading |
| `just publish` | Publish to crates.io (**LAST RESORT**) |
| `just tag` | Create annotated version tag (verifies CI first) |
| `just yank <version>` | Yank a version (for security issues) |
| `just unyank <version>` | Unyank a previously yanked version |

### Development

| Recipe | What It Does |
|--------|--------------|
| `just run` | Run the server (stdio mode) |
| `just run-debug` | Run with debug logging |
| `just install` | Install to ~/.cargo/bin |
| `just uninstall` | Uninstall from ~/.cargo/bin |
| `just info` | Show version and environment info |

### Docker (SQL Server)

| Recipe | What It Does |
|--------|--------------|
| `just db-up` | Start all SQL Server containers |
| `just db-down` | Stop all containers |
| `just db-wait` | Wait for containers to be healthy |
| `just db-status` | Show container status |

---

## Troubleshooting

### "no matching package named X found"

**Cause:** Dependency not published to crates.io.

**Fix:** Ensure all dependencies are available on crates.io before publishing.

### Rate Limited (429 Too Many Requests)

**Cause:** crates.io limits new crate publications.

**Fix:** Wait for the time specified in the error message, then retry.

### docs.rs Build Failed

**Cause:** Documentation requires features or dependencies not available in docs.rs environment.

**Fix:**
1. Check docs.rs build logs
2. Add `[package.metadata.docs.rs]` configuration if needed:
   ```toml
   [package.metadata.docs.rs]
   all-features = true
   ```
3. Ensure all doc examples compile (`cargo test --doc`)

### GitHub Release Not Created

**Cause:** Release workflow failed or tag format incorrect.

**Fix:**
1. Verify tag format is `vX.Y.Z` (not `v.X.Y.Z` or other variants)
2. Check workflow logs in GitHub Actions
3. Manually create release if needed: `gh release create vX.Y.Z --generate-notes`

### Integration Tests Fail

**Cause:** SQL Server container issues or Docker not available.

**Fix:**
1. Ensure Docker is running: `docker ps`
2. Start SQL Server containers: `just db-up && just db-wait`
3. Check container logs: `just db-logs`

### Semver Check Fails

**Cause:** Unintended breaking changes detected.

**Fix:**
1. Review the semver report: `just semver`
2. If changes are intentional, bump MAJOR version
3. If changes are unintentional, fix the API to maintain compatibility

### Local CI Passes But Remote Fails

**Cause:** Environment differences between local and CI.

**Fix:**
1. Ensure Cargo.lock is committed
2. Use `just test-locked` instead of `just test`
3. Run `just setup` to install same tool versions
4. Check for platform-specific issues

### MSRV Check Fails

**Cause:** Code uses features from newer Rust versions.

**Fix:**
1. Check which feature requires newer Rust: `cargo +1.88 check --all-features 2>&1`
2. Either update MSRV or avoid the new feature
3. Ensure all developers have the MSRV toolchain: `rustup install 1.88`

---

## Platform-Specific Notes

### Linux

Linux is the primary development and CI platform. All features should work out of the box.

**Bootstrap:**
```bash
just bootstrap-apt  # Debian/Ubuntu
```

**Known Issues:**
- None currently

### macOS

macOS is a supported development platform. Docker Desktop is required for integration tests.

**Bootstrap:**
```bash
just bootstrap-brew
```

**Known Issues:**
- Docker Desktop must be running for `just db-up` and integration tests
- Ensure `docker compose` (v2) is available, not legacy `docker-compose`

### Windows

Windows is supported via WSL2 or native builds.

**WSL2 (Recommended):**
- Use the Linux instructions within WSL2
- Docker Desktop with WSL2 backend works well

**Native Windows:**
- Requires Visual Studio Build Tools or MSVC
- May need manual SSL/TLS configuration
- Some shell-based recipes may need PowerShell equivalents

**Known Issues:**
- Path separators in some recipes may cause issues on native Windows
- Consider using WSL2 for full compatibility

---

## Feature-Specific Testing

Before releasing, test all feature combinations to ensure correctness.

### Feature Matrix

| Feature | Description | Test Command | Dependencies | Notes |
|---------|-------------|--------------|--------------|-------|
| `default` | Base functionality (stdio transport) | `cargo test` | None | Core MCP server |
| `http` | HTTP/SSE transport | `cargo test --features http` | axum, hyper | Web-based transport |
| `telemetry` | OpenTelemetry tracing | `cargo test --features telemetry` | opentelemetry crates | Observability |
| `azure-auth` | Azure AD authentication | `cargo test --features azure-auth` | azure-identity | Cloud auth |
| `full` | All features combined | `cargo test --all-features` | All above | Comprehensive |

### Feature Combination Testing

```bash
# Test each feature in isolation
cargo check --no-default-features
cargo test --no-default-features --features http
cargo test --no-default-features --features telemetry
cargo test --no-default-features --features azure-auth

# Test common combinations
cargo test --features "http,telemetry"
cargo test --features "http,azure-auth"
cargo test --features "telemetry,azure-auth"

# Test full feature set (pre-release requirement)
cargo test --all-features
```

### Integration Tests

Integration tests require a running SQL Server instance:

```bash
# Start SQL Server container
just db-up && just db-wait

# Run integration tests
just test-integration

# With specific features
cargo test --features http --test integration
```

The `just test-features` recipe runs all feature combinations automatically.

---

## Security Incident Response

This section documents procedures for handling security vulnerabilities in released versions.

### Severity Assessment

| Severity | CVSS Score | Response Time | Examples |
|----------|------------|---------------|----------|
| **Critical** | 9.0-10.0 | Immediate (same day) | SQL injection, auth bypass, RCE |
| **High** | 7.0-8.9 | 24-48 hours | Credential exposure, privilege escalation |
| **Medium** | 4.0-6.9 | 1 week | Information disclosure, DoS |
| **Low** | 0.1-3.9 | Next release | Minor information disclosure |

### Security Release Process

1. **Assess and Confirm**
   - Verify the vulnerability is real and reproducible
   - Determine affected versions and severity
   - Check if actively exploited

2. **Develop Fix**
   - Create fix on private branch
   - Ensure fix doesn't introduce new issues
   - Prepare minimal, targeted patch

3. **Coordinate Disclosure** (for Critical/High)
   - Notify affected downstream users privately if known
   - Coordinate with security researchers if externally reported
   - Prepare security advisory

4. **Release Security Patch**
   - Follow standard release process with expedited timeline
   - Use PATCH version bump (e.g., 0.1.0 → 0.1.1)
   - Document as security fix in CHANGELOG

5. **Post-Release**
   - Publish GitHub Security Advisory
   - Request CVE if applicable
   - Update RustSec advisory database

### Security Advisory Template

```markdown
## Security Advisory: [Brief Description]

**Severity**: [Critical/High/Medium/Low]
**CVE**: [CVE-YYYY-NNNNN or "Pending"]
**Affected Versions**: [e.g., < 0.1.1]
**Fixed Versions**: [e.g., >= 0.1.1]

### Description

[Detailed description of the vulnerability]

### Impact

[What can an attacker do with this vulnerability]

### Mitigation

[Immediate steps users can take before updating]

### Resolution

Update to version X.Y.Z or later:
\`\`\`bash
cargo update -p mssql-mcp-server
\`\`\`

### Credits

[Acknowledge reporters if they consent]
```

### Yanking Considerations

For severe security issues, yank affected versions:

```bash
cargo yank --version 0.X.Y mssql-mcp-server
```

**Note:** Yanking prevents new installations but doesn't break existing `Cargo.lock` files.

---

## Lessons Learned

This section documents issues encountered in past releases and patterns to avoid.

### 1. CI Parity is Non-Negotiable

**Issue**: Local tests pass but CI fails due to environment differences.

**Solution**: Always run `just ci` and `just ci-status` before pushing. These commands are designed to mirror CI configuration exactly.

### 2. SQL Server Container Health

**Issue**: Integration tests fail sporadically due to container startup timing.

**Solution**: Always run `just db-wait` after `just db-up`. The wait command ensures SQL Server is accepting connections.

### 3. Feature Gate Testing

**Issue**: Code compiles with `--all-features` but fails with specific feature combinations.

**Solution**: Run `just test-features` which tests all feature combinations individually.

### 4. Azure Auth Testing

**Issue**: The `azure-auth` feature requires valid Azure credentials to test fully.

**Solution**: Unit tests with mocked credentials work in CI. Integration tests require manual verification with real Azure environment.

### 5. MSRV Compliance

**Issue**: Accidentally using newer Rust features breaks MSRV compatibility.

**Solution**: Run `just msrv-check` before every release. CI enforces this automatically.

### 6. Tag Format Consistency

**Issue**: Tags without `v` prefix don't trigger release workflow.

**Solution**: Always use `just tag` which enforces the `vX.Y.Z` format.

### 7. Docker Compose Version

**Issue**: Some systems have legacy `docker-compose` (v1) instead of `docker compose` (v2).

**Solution**: The justfile detects and uses the correct command. If issues persist, install Docker Compose v2.

### 8. Nightly Formatter

**Issue**: Formatting check fails because nightly rustfmt isn't installed.

**Solution**: Run `rustup +nightly component add rustfmt` to install the nightly formatter.

### 9. Platform Path Differences

**Issue**: Some justfile recipes fail on Windows due to path separators.

**Solution**: Use WSL2 on Windows for full compatibility, or contribute PowerShell equivalents.

### 10. Telemetry Dependencies

**Issue**: OpenTelemetry crates have strict version alignment requirements.

**Solution**: All otel crates must be version-aligned. Check `Cargo.toml` for version consistency.

---

## Release Checklist Template

Copy this for each release:

```markdown
## Release vX.Y.Z Checklist

### Pre-Release
- [ ] `just release-check` passes
- [ ] Version bumped in Cargo.toml
- [ ] CHANGELOG.md updated with date
- [ ] CI passing on main branch (`just ci-status`)

### Release Execution
- [ ] Release commit pushed to main
- [ ] CI passed on main (verified via `gh run watch`)
- [ ] Tag created with `just tag`
- [ ] Tag pushed
- [ ] Crate published (automated or `just publish`)

### Post-Release
- [ ] `cargo search mssql-mcp-server` shows new version
- [ ] `cargo add` works in fresh project
- [ ] GitHub release exists (if configured)
- [ ] docs.rs building/built (check after ~15 min)
- [ ] CHANGELOG [Unreleased] section reset
```

---

## Additional Resources

- [Publishing on crates.io](https://doc.rust-lang.org/cargo/reference/publishing.html) - Official documentation
- [Semver Specification](https://semver.org/) - Semantic Versioning 2.0.0
- [cargo-release](https://github.com/crate-ci/cargo-release) - Automated release workflow
- [release-plz](https://release-plz.ieni.dev/) - Automated release PRs
- [git-cliff](https://git-cliff.org/) - Changelog generation
