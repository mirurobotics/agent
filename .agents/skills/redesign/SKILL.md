---
name: redesign
description: Refactor and improve existing code while preserving behavior, using incremental, verifiable changes to reduce complexity and improve maintainability. Use when updating existing implementations, simplifying architecture, or restructuring legacy code without changing external contracts unless explicitly requested.
---

# Redesign Workflow

## Inputs
- `scope`: existing file(s), module, package, or subsystem to refactor.
- `constraints` (optional): compatibility, performance, rollout, or deadline constraints.
- `aggressiveness` (optional): `conservative` (default), `moderate`, or `broad`.

## References
For design implementation, invoke the `$design` skill

## Procedure
1. Establish baseline behavior (tests, interfaces, invariants, runtime expectations).
2. Identify design debt in existing code (complexity, coupling, unclear boundaries, naming issues).
3. Build a refactor sequence of small reversible steps.
4. Apply steps incrementally while preserving behavior.
5. Validate after each step (tests/lint and focused runtime checks).
6. Update docs/comments for changed structure or rationale.
7. Summarize improvements and remaining debt.

## Refactor Priorities
- Reduce nesting and mixed abstraction levels.
- Split oversized functions/modules.
- Clarify boundaries and ownership of side effects.
- Improve naming consistency and intent clarity.
- Remove dead code and redundant indirection.

## Rules
- Preserve external behavior unless change is explicitly requested.
- Prefer incremental, verifiable refactors over broad rewrites.
- Keep compatibility constraints explicit.
- Stop and call out risks when safe migration path is unclear.

## Output Contract
1. Refactor plan (baseline + sequence of steps).
2. Structural changes made and why.
3. Validation evidence and compatibility status.
4. Remaining technical debt or next-step recommendations.
