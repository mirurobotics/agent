
# Go Security Standards

## Secrets and sensitive data
- [ ] Sensitive data (tokens, keys, PII) is handled with care and not logged or stored unnecessarily
- [ ] Sensitive data is redacted or omitted from logs, error messages, and metrics
- [ ] No hardcoded secrets; use environment variables or a secret manager
- [ ] Secrets and credentials are not committed to the repository

## Cryptography and randomness
- [ ] `crypto/rand` is used for security-sensitive randomness (tokens, nonces, keys); never `math/rand` for these
- [ ] Cryptographic operations use the standard library or well-audited packages (e.g. `golang.org/x/crypto`); no custom crypto
- [ ] TLS is used for connections that carry sensitive data; certificate verification is not disabled in production

## Input and injection
- [ ] User and external input is validated and sanitized before use
- [ ] Database queries use parameterized statements or prepared statements; no string concatenation for SQL
- [ ] Shell or command execution avoids unsanitized user input; if unavoidable, use safe APIs and strict allowlists

## Authentication and authorization
- [ ] Authentication and authorization are checked before performing sensitive or privileged operations
- [ ] Default is deny; access is granted only when explicitly authorized
- [ ] Session or token handling follows best practices (secure storage, expiry, revocation where applicable)

## Dependencies and supply chain
- [ ] Dependencies are kept up to date; known vulnerabilities are addressed promptly
- [ ] `go mod verify` and vulnerability scanning (e.g. `govulncheck`) are used as part of the workflow

## General
- [ ] No obvious security vulnerabilities in new or modified code
- [ ] Error responses to clients do not leak internal details, stack traces, or sensitive data
