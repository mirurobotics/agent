
# Design philosophy

Short overview of each design philosophy in this folder. **Language-specific checklists** (Go, JavaScript/TypeScript, Python, Rust) live in `design/checklists/` and contain the full checklist items; use the checklist for your language when reviewing or writing code.

---

## Nesting

Prefer early returns and guard clauses so the main path stays at shallow indentation; avoid deep nesting of conditionals and loops.

**Full explanation:** [`never-nest.mdc`](never-nest.mdc). Checklist: apply the patterns there; conditionals appear in language checklists where applicable.

---

## Names

Names should reveal intent and stay consistent: one word per concept, booleans as questions, no disinformation or noise.

**Full explanation:** [`names.mdc`](names.mdc). Checklist: **checklists** → Naming (Design Principles).

---

## Functions

Functions do one thing, stay small, take few arguments, and avoid hidden side effects; command and query are separated.

**Full explanation:** [`functions.mdc`](functions.mdc). Checklist: **checklists** → Functions.

---

## Parse, don’t validate

At boundaries, parse input into refined types that encode invariants instead of validating and returning success/fail, so the type system preserves what you learned.

**Full explanation:** [`parse-not-validate.mdc`](parse-not-validate.mdc). No dedicated checklist section; apply at boundaries.

---
