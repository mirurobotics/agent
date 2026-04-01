
# Python Security Standards

## Secrets and sensitive data
- [ ] Sensitive data (tokens, keys, PII) is handled with care and not logged or stored unnecessarily
- [ ] Sensitive data is redacted or omitted from logs, error messages, and exception output
- [ ] No hardcoded secrets; use environment variables or a secret manager
- [ ] Secrets and credentials are not committed to the repository

## Cryptography and randomness
- [ ] The `secrets` module is used for security-sensitive randomness (tokens, nonces, keys); never `random` for these
- [ ] Cryptographic operations use `cryptography` or standard-library APIs appropriately; no custom crypto
- [ ] TLS is used for connections that carry sensitive data; certificate verification is not disabled in production

## Input and injection
- [ ] User and external input is validated and sanitized before use
- [ ] Database queries use parameterized statements or the ORM’s safe APIs; no string formatting or concatenation for SQL
- [ ] Shell or command execution avoids unsanitized user input; use `shlex` or allowlists when necessary
- [ ] Deserialization of untrusted data avoids `pickle`; use JSON or other safe formats with validation
- [ ] Paths derived from user input are validated to prevent path traversal (e.g. `os.path.abspath`, check prefix)

## Authentication and authorization
- [ ] Authentication and authorization are checked before performing sensitive or privileged operations
- [ ] Default is deny; access is granted only when explicitly authorized
- [ ] Session and token handling follows best practices (secure storage, expiry, CSRF protection where applicable)

## Dependencies and supply chain
- [ ] Dependencies are kept up to date; known vulnerabilities are addressed promptly
- [ ] Vulnerability scanning (e.g. `pip-audit`, `safety`, Snyk) is used as part of the workflow
- [ ] Requirements are pinned and reviewed; virtual environments or containers are used to isolate dependencies

## General
- [ ] No obvious security vulnerabilities in new or modified code
- [ ] Error responses to clients or users do not leak internal details, stack traces, or sensitive data
- [ ] Debug and development-only code paths (e.g. `DEBUG=True`) are not enabled or exposed in production
