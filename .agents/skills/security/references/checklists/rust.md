
# Rust Security Standards

## Secrets and sensitive data
- [ ] Sensitive data (tokens, keys, PII) is handled with care and not logged or stored unnecessarily
- [ ] Sensitive data is redacted or omitted from logs, error messages, and metrics
- [ ] No hardcoded secrets; use environment variables or a secret manager
- [ ] Secrets and credentials are not committed to the repository
- [ ] Sensitive buffers (e.g. key material) are zeroed or cleared when no longer needed where feasible (e.g. `Zeroize`)

## Cryptography and randomness
- [ ] `getrandom` or the standard library’s secure RNG is used for security-sensitive randomness (tokens, nonces, keys); never non-crypto RNGs for these
- [ ] Cryptographic operations use well-audited crates (e.g. `rand`, `ring`, `rustls`); no custom crypto
- [ ] TLS is used for connections that carry sensitive data; certificate verification is not disabled in production (e.g. `rustls` with proper root store)

## Input and injection
- [ ] User and external input is validated and sanitized before use
- [ ] Database queries use parameterized statements or a safe query builder; no string concatenation for SQL
- [ ] Shell or command execution avoids unsanitized user input; use allowlists and safe APIs when necessary
- [ ] Paths derived from user input are validated to prevent path traversal (e.g. `canonicalize`, check prefix)

## Authentication and authorization
- [ ] Authentication and authorization are checked before performing sensitive or privileged operations
- [ ] Default is deny; access is granted only when explicitly authorized
- [ ] Session or token handling follows best practices (secure storage, expiry, revocation where applicable)

## Unsafe and memory safety
- [ ] `unsafe` is used only when necessary; invariants are documented and upheld
- [ ] No undefined behavior; no use-after-free or data races in safe or unsafe code

## Dependencies and supply chain
- [ ] Dependencies are kept up to date; known vulnerabilities are addressed promptly
- [ ] Vulnerability scanning (e.g. `cargo audit`) is used as part of the workflow
- [ ] Crate sources and versions are reviewed; avoid unnecessary or unmaintained dependencies for security-sensitive paths

## General
- [ ] No obvious security vulnerabilities in new or modified code
- [ ] Error responses to clients do not leak internal details, stack traces, or sensitive data
