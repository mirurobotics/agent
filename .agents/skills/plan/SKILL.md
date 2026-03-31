---
name: plan
description: Create, maintain, and execute ExecPlans for complex or multi-step work that spans many files or repositories. Use when work needs a self-contained living plan with milestones, decision tracking, concrete commands, validation criteria, and restartable context.
---

# ExecPlan Workflow

## Required Policy Load
- Read `references/policy.md` in full on every invocation before writing, revising, or implementing an ExecPlan.
- Treat `references/policy.md` as authoritative and complete; do not replace it with summaries.

## Inputs
- `goal`: target behavior or outcome.
- `mode` (optional): `author`, `implement`, or `revise`.
- `scope` (optional): affected repos/modules and constraints.

## References
- `references/policy.md`: full-fidelity ExecPlan policy and template requirements (must read fully each invocation).

## Procedure
1. Load `references/policy.md` fully.
3. Execute the selected mode (`author`, `implement`, or `revise`) by following the policy requirements exactly.
