# Releasing mssql-mcp-server

Comprehensive guide for releasing new versions of mssql-mcp-server to crates.io.

---

## Quick Start

For routine releases, use the standard workflow:

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

# 5. Publish (if not automated)
cargo publish
```

---

## Table of Contents

1. [Version Numbering](#version-numbering)
2. [Pre-Release Checklist](#pre-release-checklist)
3. [Release Workflow](#release-workflow)
4. [Post-Release Verification](#post-release-verification)
5. [CI Automation Coverage](#ci-automation-coverage)
6. [Justfile Recipe Reference](#justfile-recipe-reference)
7. [Troubleshooting](#troubleshooting)

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
| New MCP tool/resource | MINOR | 0.1.0 → 0.2.0 |
| Bug fix, security patch | PATCH | 0.1.0 → 0.1.1 |
| Documentation only | PATCH | 0.1.0 → 0.1.1 |

---

## Pre-Release Checklist

### 0. Pre-flight Checks

```bash
just release-check  # Comprehensive validation
```

- [ ] Git working directory is clean
- [ ] CI is passing on main branch
- [ ] `just release-check` completes successfully

### 1. Codebase Hygiene & Safety

```bash
just wip-check      # TODO/FIXME/XXX/HACK, todo!/unimplemented!
just panic-audit    # .unwrap()/.expect() audit
just check          # Clippy warnings-as-errors
```

- [ ] No blocking `todo!()` or `unimplemented!()` in production code
- [ ] All `.unwrap()` and `.expect()` calls reviewed for safety
- [ ] No clippy warnings

### 2. Version Consistency

```bash
just version-sync   # Check README matches Cargo.toml
```

Verify version is consistent in:
- [ ] `Cargo.toml` version field
- [ ] README.md installation instructions (if version-specific)
- [ ] CHANGELOG.md has entry with correct date

### 3. Security & Dependency Audit

```bash
just deny    # Licenses, bans, advisories
just audit   # Security vulnerabilities
```

- [ ] No license violations
- [ ] No banned dependencies
- [ ] No unaddressed security advisories (or documented in `deny.toml`)

### 4. Documentation Integrity

```bash
just doc-check      # Documentation builds without warnings
```

- [ ] Documentation builds without warnings
- [ ] CHANGELOG.md updated with new version section
- [ ] Breaking changes have migration notes

### 5. API Compatibility

```bash
just semver    # Breaking change detection (if installed)
```

- [ ] No unintended breaking changes (or version bump accounts for them)
- [ ] Public API surface reviewed
- [ ] MCP tool/resource changes documented

### 6. Final Build Verification

```bash
just ci-release    # Full CI + MSRV + feature testing
```

- [ ] All tests pass
- [ ] All feature combinations compile
- [ ] Integration tests pass against SQL Server

### 7. Publishing Preparation

```bash
just publish-dry      # Dry-run publish
just metadata-check   # Verify crates.io metadata
```

- [ ] Dry-run publish succeeds
- [ ] Required metadata present (description, license, repository)
- [ ] Keywords and categories appropriate

---

## Release Workflow

**Publishing to crates.io is IRREVERSIBLE.** Follow this exact sequence:

```
┌─────────────────────────────────────────────────────────────┐
│  1. PREPARE: Version bump + CHANGELOG + commit              │
│                         ↓                                   │
│  2. PUSH: git push origin main                              │
│                         ↓                                   │
│  3. WAIT: CI must pass on main (watch with `gh run watch`)  │
│                         ↓                                   │
│  4. TAG: just tag (creates vX.Y.Z)                          │
│                         ↓                                   │
│  5. RELEASE: git push origin vX.Y.Z                         │
│                         ↓                                   │
│  6. PUBLISH: cargo publish (or automated via CI)            │
└─────────────────────────────────────────────────────────────┘
```

### Step-by-Step Commands

#### Step 1: Prepare Version

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

#### Step 2-3: Commit and Push

```bash
git add Cargo.toml CHANGELOG.md
git commit -m "chore: release vX.Y.Z"
git push origin main

# Wait for CI to pass
gh run watch                    # Interactive watch
# OR
gh run list --limit 1           # Check status
```

#### Step 4-5: Tag and Release

```bash
# ONLY after CI passes on main!
just tag                        # Creates annotated tag vX.Y.Z
git push origin vX.Y.Z          # Triggers release workflow (if configured)
```

#### Step 6: Publish

```bash
cargo publish
```

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

# Verify GitHub release was created (if automated)
gh release view vX.Y.Z
```

- [ ] `cargo search mssql-mcp-server` shows correct version
- [ ] `cargo add` works in fresh project
- [ ] GitHub release exists with changelog content

### Delayed Checks (15-30 minutes)

```bash
# Check docs.rs (takes time to build)
curl -I https://docs.rs/mssql-mcp-server/X.Y.Z
```

- [ ] docs.rs documentation is built and accessible
- [ ] README badges show correct version (if present)

### Repository Cleanup

- [ ] Update `[Unreleased]` section in CHANGELOG for next cycle
- [ ] Close related milestones/issues

---

## CI Automation Coverage

The following checks are **automated in CI**:

| Check | CI Job | Local Recipe | Trigger |
|-------|--------|--------------|---------|
| Format | `fmt-check` | `just fmt-check` | Push/PR |
| Linting | `check` | `just check` | Push/PR |
| Tests | `test` | `just test` | Push/PR |
| Security audit | `audit` | `just audit` | Push/PR |
| License/deps | `deny` | `just deny` | Push/PR |
| Doc build | `doc` | `just doc-check` | Push/PR |

**Still requires manual verification:**
- Version string updates in documentation
- MSRV compliance (add `msrv-check` when MSRV is declared)
- Post-release installation test
- Announcement/communication

---

## Justfile Recipe Reference

| Checklist Section | Recipe | What It Does |
|-------------------|--------|--------------|
| Pre-flight | `just release-check` | Full validation + git state |
| Code hygiene | `just wip-check` | TODO/FIXME/todo!/unimplemented! |
| Code hygiene | `just panic-audit` | .unwrap()/.expect() audit |
| Version sync | `just version-sync` | README version matches Cargo.toml |
| Security | `just deny` | Licenses, bans, advisories |
| Security | `just audit` | Vulnerability scan |
| Documentation | `just doc-check` | Docs build without warnings |
| Semver | `just semver` | Breaking change detection |
| Publishing | `just publish-dry` | Dry-run publish |
| Publishing | `just metadata-check` | crates.io metadata |
| Git | `just tag` | Create annotated version tag |
| Full CI | `just ci-release` | ci + audit + deny + doc-check |

---

## Troubleshooting

### "no matching package named X found"

**Cause**: Dependency not published to crates.io.

**Fix**: Ensure all dependencies are available on crates.io before publishing.

### Rate Limited (429 Too Many Requests)

**Cause**: crates.io limits new crate publications.

**Fix**: Wait for the time specified in the error message, then retry.

### docs.rs Build Failed

**Cause**: Documentation requires features or dependencies not available in docs.rs environment.

**Fix**:
1. Check docs.rs build logs
2. Add `[package.metadata.docs.rs]` configuration if needed:
   ```toml
   [package.metadata.docs.rs]
   all-features = true
   ```
3. Ensure all doc examples compile (`cargo test --doc`)

### GitHub Release Not Created

**Cause**: Release workflow failed or tag format incorrect.

**Fix**:
1. Verify tag format is `vX.Y.Z` (not `v.X.Y.Z` or other variants)
2. Check workflow logs in GitHub Actions
3. Manually create release if needed: `gh release create vX.Y.Z --generate-notes`

### Integration Tests Fail

**Cause**: SQL Server container issues or Docker not available.

**Fix**:
1. Ensure Docker is running: `docker ps`
2. Start SQL Server containers: `just db-up && just db-wait`
3. Check container logs: `just db-logs`

---

## Feature-Specific Testing

Before releasing, test all optional features:

```bash
# Default (no optional features)
cargo test

# HTTP transport
cargo test --features http

# OpenTelemetry
cargo test --features telemetry

# Azure AD authentication
cargo test --features azure-auth

# All features
cargo test --all-features
```

---

## Release Checklist Template

Copy this for each release:

```markdown
## Release vX.Y.Z Checklist

### Pre-Release
- [ ] `just release-check` passes
- [ ] Version bumped in Cargo.toml
- [ ] CHANGELOG.md updated with date
- [ ] CI passing on main branch

### Release Execution
- [ ] Release commit pushed to main
- [ ] CI passed on main (verified via `gh run watch`)
- [ ] Tag created with `just tag`
- [ ] Tag pushed
- [ ] `cargo publish` succeeded

### Post-Release
- [ ] `cargo search mssql-mcp-server` shows new version
- [ ] `cargo add` works in fresh project
- [ ] GitHub release exists (if configured)
- [ ] docs.rs building/built
- [ ] CHANGELOG [Unreleased] section reset
```

---

## Additional Resources

- [cargo-release](https://github.com/crate-ci/cargo-release) - Automated release workflow
- [release-plz](https://crates.io/crates/release-plz) - Semantic versioning with conventional commits
- [Publishing on crates.io](https://doc.rust-lang.org/cargo/reference/publishing.html) - Official documentation
