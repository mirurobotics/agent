
# Security

**Security is non-negotiable.** At Miru we treat it as a first-class requirement, not an afterthought. One vulnerability can compromise user trust, data, and systems. Every change is reviewed with security in mind.

## Philosophy

- **Secure by default.** Choose safe options first (e.g. parameterized queries, principle of least privilege). Require an explicit reason to weaken a control, not to add one.
- **Defense in depth.** Don’t rely on a single control. Validate input, enforce auth and authorization, protect data in transit and at rest, and limit blast radius.
- **Assume breach.** Sensitive data must be handled so that exposure in logs, errors, or caches does not become a full compromise. Redact, minimize retention, and avoid logging secrets.
- **Don’t roll your own.** Use the platform’s or language’s standard or well-audited libraries for crypto, auth, and secrets. Custom crypto and ad-hoc auth are high-risk.

Checklist items are in `security/checklists/` (Go, JavaScript/TypeScript, Python, Rust). Use the checklist for your language when reviewing or writing code.
