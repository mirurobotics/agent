# Auto-generate backend forward-compat tests in `impl_status_enum!`

This ExecPlan is a living document. Sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `/home/ben/miru/workbench1/repos/agent` (workspace root) | read-write | Add a new arm to the `impl_status_enum!` macro that auto-generates the backend forward-compat tests; migrate the four backend-form call sites in `agent/src/models/deployment.rs`; delete now-redundant hand-written trios. Add `paste` as a dev-dep. |

Extends the open PR #75 ("Make backend-api OpenAPI enums forward-compatible") on the existing branch `claude/hunt-agent-repo-190Ta`. Do NOT create a new branch; push to the same branch to update PR #75.

## Purpose

PR #75 made the backend-api enums forward-compatible: `From<&$backend_type> for $name` in `impl_status_enum!` has a wildcard arm that maps unknown backend values to the declared default (with a `log` macro call), and `Deserialize for $name` does the same for unknown wire strings. To prove that contract per enum, four hand-written test trios were added to `agent/src/models/deployment.rs` (`mod backend_unknown_mapping_tests`). They are mechanical boilerplate: unknown→default, known→exact, unknown-wire→default — only the enum name, backend sentinel, and default variant change.

The macro already owns the contract. It should own the proof too. Adding `unknown_backend: <sentinel>` to a new backend-form arm lets the macro emit `#[cfg(test)] mod <name>_mapping_tests` with the three tests per call site, so new `impl_status_enum!` users get coverage for free.

After this change:
- The four call sites in `deployment.rs` pass `unknown_backend:` and the macro emits per-enum test modules.
- The hand-written `mod backend_unknown_mapping_tests` is gone, except for `deployment_payload_with_unknown_activity_status_still_deserializes` — that test exercises `Deployment` JSON composition (backend-api serde → `Deployment::from_backend`), not the macro contract, and is preserved in a renamed module.

Observable outcome: `cargo test -p miru-agent --features test` runs the auto-generated tests (one trio per backend enum, plus the preserved composition test) and they pass. Removing the hand-written trios loses no coverage.

## Progress

- [ ] Confirm `paste` is not currently a dependency (workspace + agent + libs).
- [ ] Add the new arm `unknown_backend: $unknown_backend:path` to `agent/src/models/status.rs`'s `impl_status_enum!` macro. The arm delegates to the existing backend-form arm and additionally emits `#[cfg(test)] mod <snake_name>_mapping_tests { ... }` via `paste::paste!`.
- [ ] Add `paste = "1"` to `agent/Cargo.toml` `[dev-dependencies]`.
- [ ] Migrate the four backend-form `impl_status_enum!` call sites in `agent/src/models/deployment.rs` to pass `unknown_backend:`.
- [ ] Delete the per-enum trios under `mod backend_unknown_mapping_tests` in `agent/src/models/deployment.rs`.
- [ ] Preserve `deployment_payload_with_unknown_activity_status_still_deserializes` in a renamed `mod deployment_payload_forward_compat_tests`.
- [ ] Confirm `agent/src/models/device.rs` is untouched (base-form macro, no `backend_type`).
- [ ] Run `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test -p miru-agent --features test`; confirm pass.
- [ ] Run `scripts/preflight.sh`; confirm `Preflight clean`.

## Surprises & Discoveries

- Observation: `paste` is NOT currently anywhere in `Cargo.lock` (`grep '^name = "paste"' Cargo.lock` empty). Adopting it requires a new dev-dep entry and `Cargo.lock` refresh.
- Observation: The hand-written `mod backend_unknown_mapping_tests` (≈ lines 389-508) contains FOUR per-enum trios PLUS ONE composition-level `deployment_payload_with_unknown_activity_status_still_deserializes`. The composition test exercises the chain backend-api serde → `From<&backend_client::DeploymentActivityStatus> for DplActivity` → `Deployment::from_backend` — must be preserved.
- Observation: `agent/src/models/device.rs::DeviceStatus` uses the BASE form (no `backend_type`). Out of scope.
- Observation: Backend "unknown" sentinels follow `<EnumPascal>UnknownValue`:
  - `backend_client::DeploymentTargetStatus::DeploymentTargetStatusUnknownValue`
  - `backend_client::DeploymentActivityStatus::DeploymentActivityStatusUnknownValue`
  - `backend_client::DeploymentErrorStatus::DeploymentErrorStatusUnknownValue`
  - `backend_client::DeploymentStatus::DeploymentStatusUnknownValue`
- Observation: `agent/Cargo.toml` lints config doesn't allow `non_snake_case`. Bare `mod DplTarget_mapping_tests` would lint-fail under `clippy -- -D warnings` unless `#[allow(non_snake_case)]` is sprinkled in every expansion.

## Decision Log

- Decision: Use `paste = "1"` dev-dep to lowercase the enum ident into a snake_case module name.
  Rationale: `paste` is a tiny well-known proc-macro crate, compile-time only, used solely inside `#[cfg(test)]`. The alternative (bare `<Name>_mapping_tests` with `#[allow(non_snake_case)]`) puts visible lint suppression into the macro expansion. `paste` removes the noise.
  Date/Author: 2026-05-15 / Claude.

- Decision: Make the new arm a SUPERSET of the existing backend-form arm — a NEW arm that delegates to the existing one and additionally emits the test module. Do NOT modify the existing backend-form arm.
  Rationale: Backward compatibility; opt-in via `unknown_backend:`; smaller macro diff.
  Date/Author: 2026-05-15 / Claude.

- Decision: Use literal `"__impl_status_enum_unknown_sentinel__"` as the unknown wire string in the auto-generated `unknown_wire_string_deserializes_to_default` test.
  Rationale: Guarantees it can never accidentally collide with a real wire value; documents intent. Any unique non-wire string works.
  Date/Author: 2026-05-15 / Claude.

- Decision: Preserve `deployment_payload_with_unknown_activity_status_still_deserializes` by moving it into `mod deployment_payload_forward_compat_tests` and deleting the rest of `mod backend_unknown_mapping_tests`.
  Rationale: It tests `Deployment` composition, not the macro contract; rename avoids collision with the macro-generated `dpl_activity_mapping_tests`.
  Date/Author: 2026-05-15 / Claude.

## Outcomes & Retrospective

(Fill in at completion.)

## Context and Orientation

Rust workspace at `/home/ben/miru/workbench1/repos/agent`. Relevant files:

- `agent/src/models/status.rs` — declares `impl_status_enum!`. Two existing public arms: base form (no `backend_type:`) used by `device.rs`; backend form used by `deployment.rs`. Internal `@base` arm shared.
- `agent/src/models/deployment.rs` — four backend-form `impl_status_enum!` invocations for `DplTarget` (default Staged), `DplActivity` (default Drifted), `DplErrStatus` (default None), `DplStatus` (default Drifted). At the bottom, `mod backend_unknown_mapping_tests` holds the 13 hand-written tests.
- `agent/src/models/device.rs` — BASE form; out of scope.
- `agent/Cargo.toml` — add `paste = "1"` under `[dev-dependencies]`.
- `Cargo.lock` — refreshed by adding the dep.

`paste::paste!` supports identifier transformation tokens like `[<$name:snake _mapping_tests>]`, which lowercases `$name` and joins with `_mapping_tests`. Compile-time only.

Validation tooling:
- `scripts/preflight.sh` — runs `scripts/lint.sh`, `scripts/covgate.sh`, tools-crate lint/test. Prints `Preflight clean` on success.
- `scripts/test.sh` — `cargo test --features test`.
- `scripts/lint.sh` — fmt, clippy `-D warnings`, machete, custom import linter.

## Plan of Work

### M1 — Add the new macro arm

Edit `agent/src/models/status.rs`. Insert a NEW arm BEFORE the existing backend-form arm (macro arms match in declaration order; the new arm is a strict superset of the existing one).

Signature:

```rust
(
    enum $name:ident,
    default: $default:ident,
    label: $label:expr,
    log: $log_macro:ident,
    agent_type: $agent_type:ty,
    backend_type: $backend_type:ty,
    unknown_backend: $unknown_backend:path,
    mappings: [
        $(
            $variant:ident => $wire:literal =>
                $agent_value:expr =>
                $backend_value:path
        ),+ $(,)?
    ]
)
```

Body:

1. Delegate to the existing backend-form arm to emit `Deserialize`, `From<&$name> for $agent_type`, `From<&$name> for $backend_type`, and `From<&$backend_type> for $name`. Re-invokes `impl_status_enum!` with the same fields minus `unknown_backend:`.
2. Emit one `#[cfg(test)] mod <snake_name>_mapping_tests { ... }` block via `paste::paste!`, importing `super::*` and defining three tests:
   - `unknown_backend_maps_to_default`: `let d: $name = (&$unknown_backend).into(); assert_eq!(d, $name::$default);`
   - `unknown_wire_string_deserializes_to_default`: `let d: $name = serde_json::from_str("\"__impl_status_enum_unknown_sentinel__\"").unwrap(); assert_eq!(d, $name::$default);`
   - `known_backend_values_map_exactly`: per-variant repetition asserting `(&$backend_value).into()` equals `$name::$variant`.

Sketch:

```rust
(
    enum $name:ident,
    default: $default:ident,
    label: $label:expr,
    log: $log_macro:ident,
    agent_type: $agent_type:ty,
    backend_type: $backend_type:ty,
    unknown_backend: $unknown_backend:path,
    mappings: [
        $( $variant:ident => $wire:literal => $agent_value:expr => $backend_value:path ),+ $(,)?
    ]
) => {
    impl_status_enum!(
        enum $name,
        default: $default,
        label: $label,
        log: $log_macro,
        agent_type: $agent_type,
        backend_type: $backend_type,
        mappings: [
            $( $variant => $wire => $agent_value => $backend_value ),+
        ]
    );

    paste::paste! {
        #[cfg(test)]
        mod [<$name:snake _mapping_tests>] {
            use super::*;

            #[test]
            fn unknown_backend_maps_to_default() {
                let d: $name = (&$unknown_backend).into();
                assert_eq!(d, $name::$default);
            }

            #[test]
            fn unknown_wire_string_deserializes_to_default() {
                let d: $name =
                    serde_json::from_str("\"__impl_status_enum_unknown_sentinel__\"").unwrap();
                assert_eq!(d, $name::$default);
            }

            #[test]
            fn known_backend_values_map_exactly() {
                $(
                    let d: $name = (&$backend_value).into();
                    assert_eq!(d, $name::$variant);
                )+
            }
        }
    }
};
```

`paste::paste!` passes through non-`[<...>]` tokens unchanged, so `$name`, `$variant`, etc. expand normally inside it. If the version of `paste` resolved by Cargo does not behave this way, fall back to: define the generated ident via `paste::paste!{}` first, then write the `mod $ident { ... }` block in normal scope. Document any fallback in Decision Log.

### M2 — Add `paste` dev-dep

Edit `agent/Cargo.toml`. Under `[dev-dependencies]` add `paste = "1"`. Refresh `Cargo.lock` via `cargo build -p miru-agent --tests --features test`.

### M3 — Migrate the four call sites in `deployment.rs`

Add one line `unknown_backend: backend_client::<Enum>::<Enum>UnknownValue,` between `backend_type:` and `mappings:` for each of the four `impl_status_enum!` invocations:

- `DplTarget` (≈ line 24): `unknown_backend: backend_client::DeploymentTargetStatus::DeploymentTargetStatusUnknownValue,`
- `DplActivity` (≈ line 57): `unknown_backend: backend_client::DeploymentActivityStatus::DeploymentActivityStatusUnknownValue,`
- `DplErrStatus` (≈ line 96): `unknown_backend: backend_client::DeploymentErrorStatus::DeploymentErrorStatusUnknownValue,`
- `DplStatus` (≈ line 151): `unknown_backend: backend_client::DeploymentStatus::DeploymentStatusUnknownValue,`

No `mappings: [...]` edits. No imports change (sentinels are already used by the hand-written tests below).

### M4 — Delete hand-written trios; preserve the composition test

Delete the four `// ---- <Name> ----` trio sections under `mod backend_unknown_mapping_tests`. Move `deployment_payload_with_unknown_activity_status_still_deserializes` into a renamed module to avoid collision with the macro-generated `dpl_activity_mapping_tests`:

```rust
#[cfg(test)]
mod deployment_payload_forward_compat_tests {
    use super::*;

    #[test]
    fn deployment_payload_with_unknown_activity_status_still_deserializes() {
        // body unchanged from current
    }
}
```

The old `mod backend_unknown_mapping_tests` is fully removed.

### M5 — Validate

```
cd /home/ben/miru/workbench1/repos/agent
cargo fmt -p miru-agent -- --check
cargo clippy --package miru-agent --all-features -- -D warnings
cargo test -p miru-agent --features test
scripts/preflight.sh
```

Expected `cargo test` output: 12 auto-generated tests (3 each × 4 enums) plus the preserved composition test (13 total, matching the count of hand-written tests removed).

## Concrete Steps

Working directory: `/home/ben/miru/workbench1/repos/agent`.

### Step A — Add the new macro arm

Edit `agent/src/models/status.rs`. Insert the new arm (M1 §Sketch) before the existing backend-form arm.

```
grep -n 'unknown_backend:' agent/src/models/status.rs   # expect 1
grep -n 'macro_rules! impl_status_enum' agent/src/models/status.rs   # expect 1
```

### Step B — Add `paste` dev-dep

```
# Append paste = "1" under [dev-dependencies] in agent/Cargo.toml
cargo build -p miru-agent --tests --features test 2>&1 | tail -5
grep '^name = "paste"' Cargo.lock   # expect 1
```

### Step C — Migrate the four call sites

```
grep -c 'unknown_backend:' agent/src/models/deployment.rs   # expect 4
```

### Step D — Delete trios; preserve composition test

```
grep -n 'mod backend_unknown_mapping_tests' agent/src/models/deployment.rs   # expect 0
grep -n 'deployment_payload_with_unknown_activity_status_still_deserializes' agent/src/models/deployment.rs   # expect 1
grep -n 'mod deployment_payload_forward_compat_tests' agent/src/models/deployment.rs   # expect 1
```

### Step E — Validate

```
cargo fmt -p miru-agent -- --check
cargo clippy --package miru-agent --all-features -- -D warnings
cargo test -p miru-agent --features test
```

Expect 13 forward-compat tests passing total.

### Step F — Preflight gate

```
scripts/preflight.sh
```

Required final line: `Preflight clean`. **Do NOT push until preflight reports `clean`.**

### Step G — Push to update PR #75

Commit and push (orchestrator handles delivery — push mode rebases onto `main` then pushes with `--force-with-lease`).

## Validation and Acceptance

1. **Auto-generated trios cover the macro contract.** Each `<enum>_mapping_tests` module's three tests pass.
2. **Composition coverage preserved.** `deployment_payload_with_unknown_activity_status_still_deserializes` still passes in its renamed module.
3. **No regression.** `cargo clippy --package miru-agent --all-features -- -D warnings` succeeds.
4. **`agent/src/models/device.rs` is untouched.**
5. **Preflight gate.** `scripts/preflight.sh` MUST report `Preflight clean` before publishing.
6. **PR #75 updated, not replaced.**

## Idempotence and Recovery

- Static source edits are idempotent.
- If `paste`'s `[<...>]` syntax interaction with declarative macro substitution fails in practice, fall back to bare `mod $name_mapping_tests` with `#[allow(non_snake_case)]` inside the generated module; document in Decision Log.
- Rollback: `git checkout -- agent/src/models/status.rs agent/src/models/deployment.rs agent/Cargo.toml Cargo.lock`.
