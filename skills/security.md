# Skill: Security Audit Checklist

**Trigger:** Use when auditing code for security vulnerabilities.

## P0 — Always Check (Never Skip)

### Input Injection
- [ ] All user input is validated and sanitized before use.
- [ ] Shell commands use argument arrays, not string interpolation: `Command::new("git").arg(user_input)` not `format!("git {user_input}")`.
- [ ] SQL queries use parameterized statements, never string concatenation.
- [ ] HTML output is escaped to prevent XSS.

### Path Traversal
- [ ] File paths from untrusted sources reject `..` components.
- [ ] Paths are canonicalized and verified to stay within the expected directory.
- [ ] Symlinks in user-controlled paths are resolved before access checks.

### Authentication & Authorization
- [ ] Every protected endpoint verifies authentication.
- [ ] Authorization checks use allow-lists, not deny-lists.
- [ ] Session tokens are cryptographically random and expire.
- [ ] Password comparison uses constant-time equality.

### LLM Output Injection
- [ ] Model output is never passed directly to shell interpreters or code evaluation functions.
- [ ] Model output used in file paths is sanitized (no `..`, no absolute paths).
- [ ] Model-generated code is sandboxed or reviewed before execution.

## P1 — Check for All Changes

### Data Handling
- [ ] Secrets (API keys, tokens, passwords) are not hardcoded or logged.
- [ ] Sensitive data is zeroed from memory after use where possible.
- [ ] Error messages do not leak internal paths, stack traces, or schema details.

### Dependency Security
- [ ] New dependencies are from reputable sources with active maintenance.
- [ ] No known CVEs in current dependency versions (`cargo audit` / `npm audit`).
- [ ] Dependency permissions are minimal (no unnecessary filesystem/network access).

### Cryptography
- [ ] Using well-known libraries (ring, openssl, libsodium) — never hand-rolled crypto.
- [ ] TLS verification is enabled for all outbound HTTPS connections.
- [ ] Random values for security purposes use CSPRNG, not a standard random number generator.

## How to Report

- Severity: Critical / High / Medium / Low.
- Proof of concept: minimal reproduction showing the vulnerability.
- Remediation: specific code change or pattern to apply.
- Scope: list all affected files and entry points.
