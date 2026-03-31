---
name: reliability
description: Assess and improve runtime reliability by hardening error handling, resource lifecycle management, cancellation/timeout behavior, and recovery characteristics. Use when building or reviewing code that touches I/O, concurrency, retries, long-running jobs, cleanup paths, or failure handling.
---

# Reliability Workflow

## Inputs
- `scope`: files, module, package, service, or subsystem.
- `mode` (optional): `review-only` or `fix` (default).
- `risk_level` (optional): `high`, `medium`, `low`.

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
1. Identify reliability-sensitive paths in scope.
2. Audit failure handling, cleanup, cancellation/timeouts, and retry strategy.
3. Identify reliability risks and rank by severity.
4. In `fix` mode, apply minimal safe changes to eliminate high/medium risks.
5. Validate with targeted tests and failure-path checks.
6. Summarize resolved and residual risks.

## Reliability Focus Areas
- Resource acquisition/release symmetry.
- Explicit error propagation and context.
- Bounded waits, retries, and backoff.
- Cancellation propagation and graceful shutdown.
- Idempotency/recovery for retries/restarts.

## Output Contract
1. Reliability findings (severity + file evidence).
2. Fix summary (when `mode=fix`).
3. Validation evidence and remaining risks.
