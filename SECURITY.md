# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.x.x   | :white_check_mark: |

## Reporting a Vulnerability

We take security vulnerabilities seriously. If you discover a security issue, please report it responsibly.

### How to Report

1. **Do NOT** open a public GitHub issue for security vulnerabilities
2. Email security concerns to: security@praxiomlabs.org
3. Or use GitHub's private vulnerability reporting feature

### What to Include

Please include the following in your report:

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Any suggested fixes (optional)

### Response Timeline

- **Initial Response**: Within 48 hours
- **Status Update**: Within 7 days
- **Resolution Target**: Within 90 days (may vary based on severity)

### Disclosure Policy

- We follow coordinated disclosure practices
- We will credit reporters (unless they prefer to remain anonymous)
- We aim to release fixes before public disclosure

## Security Considerations

### Database Connections

- **Connection Strings**: Never log or expose connection strings
- **Credentials**: Use environment variables or secure vaults for credentials
- **TLS**: Always use encrypted connections in production

### SQL Injection Prevention

This server implements multiple layers of SQL injection protection:

1. **Query Analysis**: Static analysis of SQL patterns
2. **Parameterized Queries**: All user inputs are parameterized
3. **Allowlist Validation**: Only permitted statement types are executed
4. **Input Sanitization**: Special characters are properly escaped

### Session Security

- Sessions are isolated per connection
- Transactions are scoped to individual sessions
- Pinned sessions have timeout protection
- Connection pools are properly managed

### Best Practices for Deployment

1. **Network Security**: Run behind a firewall, limit network access
2. **Least Privilege**: Use database accounts with minimal required permissions
3. **Audit Logging**: Enable SQL Server audit logging
4. **TLS**: Always use TDS 8.0 or encrypted connections
5. **Monitoring**: Monitor for unusual query patterns

## Security Features

### Type Safety

The server uses Rust's type system to prevent common vulnerabilities:

- No unsafe code
- Strong typing prevents type confusion
- Ownership system prevents use-after-free

### Error Handling

- Errors never expose sensitive information
- Stack traces are not sent to clients
- Database errors are sanitized before returning

### Dependencies

- Minimal dependency tree
- Regular dependency audits via `cargo audit`
- CI includes security scanning

## Security Checklist for Contributors

Before submitting PRs:

- [ ] No new `unsafe` code
- [ ] Input validation for all user-provided data
- [ ] No hardcoded credentials or secrets
- [ ] Error messages don't leak sensitive info
- [ ] SQL queries use parameterization
- [ ] No new security warnings from `cargo audit`

## References

- [MCP Security Specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/security)
- [OWASP SQL Injection Prevention](https://cheatsheetseries.owasp.org/cheatsheets/SQL_Injection_Prevention_Cheat_Sheet.html)
- [SQL Server Security Best Practices](https://docs.microsoft.com/en-us/sql/relational-databases/security/security-center-for-sql-server-database-engine-and-azure-sql-database)
