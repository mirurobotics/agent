
## Principle

When you check that input satisfies an invariant (e.g. “non-empty”, “valid format”, “in range”), **parse** it into a type that encodes that invariant instead of **validating** and returning success/fail. Validation discards the information you just learned, so every downstream caller must either re-check or handle “impossible” cases. Parsing preserves the invariant in the type system so it can’t be forgotten.

**Validate:** check, then return `void` or `bool`. Callers get no type-level guarantee.  
**Parse:** check, then return a refined value (e.g. `NonEmpty T`, `ValidatedEmail`, `UserId`). Callers receive a type that can only represent valid data.

## Why it helps

- **Single check at the boundary.** You establish the invariant once (e.g. when reading config, parsing JSON, or accepting user input). After that, functions take the refined type and don’t need to re-validate.
- **Compiler enforces the invariant.** If the boundary stops doing the check, its return type must change, so call sites fail to typecheck instead of failing at runtime.
- **No shotgun parsing.** When validation is scattered (“hope something catches bad input”), invalid data can be partially processed before a check fails. Parse up front so invalid input can only fail in one phase; keep processing separate.

## In practice

- **Use types that make illegal states unrepresentable.** Prefer a type that can’t express “empty list” or “duplicate keys” over a general type plus runtime checks. If the natural encoding makes invariants hard to enforce, consider a different representation.
- **Parse at the boundary.** Get external data (env, network, CLI, DB) into the most precise representation you need as soon as it enters your system, before any business logic acts on it.
- **Write functions on the data you wish you had.** Define the ideal argument types for your core logic, then add a thin parsing layer that turns raw input into those types. The design exercise is bridging the gap between “what we get” and “what we need.”
- **Treat “validate and return void/bool” with suspicion.** If the main point of a function is to enforce an invariant, its result should be the refined data, not just success/fail. That way the check can’t be omitted and the type carries the proof.
- **When “illegal state unrepresentable” isn’t practical,** use an abstract type plus a smart constructor (or factory) so the only way to obtain the type is through the check. Callers then depend on the refined type, not on remembering to validate.

## Scope

These are ideals to aim for, not strict rules. Sometimes a single “impossible” branch or a local validation is acceptable; document the invariant and handle it with care. Prefer parsing at boundaries and refined types where the cost is low and the benefit is clear.

**Reference:** [Parse, don’t validate](https://lexi-lambda.github.io/blog/2019/11/05/parse-don-t-validate/) (Alexis King, 2019).
