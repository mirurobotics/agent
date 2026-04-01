---
name: security
description: Assess and improve security posture of code changes by addressing input handling, authentication/authorization boundaries, secret management, cryptography usage, and dependency/supply-chain risk. Use when code processes untrusted input, handles credentials/tokens, accesses protected resources, or changes external interfaces.
---

# Security Workflow

## Inputs
- `scope`: files, module, package, service, or subsystem.
- `mode` (optional): `review-only` or `fix` (default).
- `threat_model` (optional): constraints, assumptions, and attacker model.

## References
Use selective loading:
- Always start with `references/philosophy/overview.md`.
- Load only active-language checklists:
- `references/checklists/go.md`
- `references/checklists/javascript-typescript.md`
- `references/checklists/python.md`
- `references/checklists/rust.md`
- In polyglot changes, load only the checklists for touched languages.

## Procedure
1. Map trust boundaries and sensitive data flows in scope.
2. Audit input validation/sanitization and injection surfaces.
3. Audit authn/authz, secret handling, crypto, and dependency risk.
4. Rank findings by exploitability and impact.
5. In `fix` mode, apply least-risk mitigations and safe defaults.
6. Validate security-critical paths with tests and negative cases.
7. Summarize fixes and remaining risk assumptions.

## Security Focus Areas
- Untrusted input and injection prevention.
- Authentication and authorization checks.
- Secret/token storage, transport, and logging hygiene.
- Safe crypto primitives and randomness.
- Dependency and supply-chain hygiene.

## Output Contract
1. Security findings (severity + attack path + evidence).
2. Mitigation summary (when `mode=fix`).
3. Validation evidence and residual risk assumptions.
