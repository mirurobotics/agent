
# JavaScript/TypeScript Security Standards

## Secrets and sensitive data
- [ ] Sensitive data (tokens, keys, PII) is handled with care and not logged or stored unnecessarily
- [ ] Sensitive data is redacted or omitted from logs, error messages, and client-side state
- [ ] No hardcoded secrets; use environment variables or a secret manager (and never expose server secrets to the client)
- [ ] Secrets and credentials are not committed to the repository

## Cryptography and randomness
- [ ] `crypto.getRandomValues()` (or equivalent) is used for security-sensitive randomness (tokens, nonces, keys); never `Math.random()` for these
- [ ] Cryptographic operations use the Web Crypto API or well-audited libraries; no custom crypto
- [ ] HTTPS/TLS is used for all requests that carry sensitive data; certificate verification is not disabled in production

## Input and injection
- [ ] User and external input is validated and sanitized before use (including URL params, form data, and headers)
- [ ] `eval()`, `new Function()` with user input, and dangerous `innerHTML`/`document.write` with unsanitized data are avoided
- [ ] Database or API queries use parameterized statements or safe APIs; no string concatenation for SQL or NoSQL injection
- [ ] Output is escaped or sanitized for the context (HTML, URL, JS) to prevent XSS

## Authentication and authorization
- [ ] Authentication and authorization are checked before performing sensitive or privileged operations (client and server)
- [ ] Default is deny; access is granted only when explicitly authorized
- [ ] Tokens and session data are stored and transmitted securely (e.g. httpOnly cookies for session cookies); sensitive tokens are not in localStorage when XSS is a concern

## Dependencies and supply chain
- [ ] Dependencies are kept up to date; known vulnerabilities are addressed promptly
- [ ] Vulnerability scanning (e.g. `npm audit`, Snyk) is used as part of the workflow
- [ ] Package integrity is verified (lockfiles, optional integrity checks)

## General
- [ ] No obvious security vulnerabilities in new or modified code
- [ ] Error responses to clients do not leak internal details, stack traces, or sensitive data
- [ ] CORS and CSP are configured appropriately for the application
