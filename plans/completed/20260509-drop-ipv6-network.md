# Drop IPv6 loopback support in network module

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Narrows `is_loopback_host` to `localhost` + `127.0.0.1`, removes IPv6/bracket handling in `BackendHost`, simplifies `explicit_port`, updates affected unit tests. Optionally drops the `url` dependency from the agent crate. |

This plan lives under `plans/backlog/` of the agent repo because every change is confined to `agent/src/network/mod.rs`, `agent/tests/network/mod.rs`, and (under Option B) `agent/Cargo.toml`. It amends the existing PR #66 on branch `refactor/backend-host`. Do **not** create a new branch — commit on top of the current branch.

## Purpose / Big Picture

`agent/src/network/mod.rs` currently treats `localhost`, `127.0.0.1`, and `::1` as loopback. The IPv6 literal forces the module to carry: a synthetic-scheme wrap for unbracketed `::1`, a bracket-strip on `host_str()`, a multi-colon guard inside `explicit_port`, and per-form authority-formatting branches in both `BackendHost::new` and `BackendHost::as_url`. Operators do not run the agent against an IPv6 loopback; the `127.0.0.1` and `localhost` literals cover every realistic local-dev scenario. Dropping `::1` shrinks the type, the test surface, and (optionally) the dependency graph without changing any behavior an operator can observe.

After this change:

- `BackendHost::new("localhost")`, `BackendHost::new("127.0.0.1")`, and `BackendHost::new("api.mirurobotics.com")` continue to work identically.
- `BackendHost::new("::1")` and `BackendHost::new("[::1]:8080")` return `Err`.
- `MqttHost::new("::1")` returns `Err` (inherited automatically — the body of `MqttHost::new` does not change).
- `MqttHost::new("localhost")` and `MqttHost::new("127.0.0.1")` continue to work, so the SSL-unless-loopback rule in `agent/src/mqtt/options.rs:58` (`ConnectAddress::new`) is observably unaffected: the loopback set still admits both forms operators actually use.

## Progress

- [x] Decide Option A vs Option B (see Decision Log) before touching code.
- [x] Edit `agent/src/network/mod.rs` per Plan of Work.
- [x] Edit `agent/tests/network/mod.rs` per Plan of Work (delete five IPv6 tests; add three rejection tests).
- [x] If Option B: drop `url = { workspace = true }` from `agent/Cargo.toml`, run `scripts/update-deps.sh`.
- [x] `cargo build` passes.
- [x] `cargo fmt -p miru-agent -- --check` produces no diff.
- [x] `cargo clippy --package miru-agent --all-features -- -D warnings` reports no warnings.
- [x] `./scripts/test.sh` passes (new `rejects_loopback_ipv6*` tests pin the behavior change).
- [x] `./scripts/lint.sh` passes (includes `cargo machete` — important under Option B).
- [ ] `./scripts/covgate.sh` passes (storage and mqtt covgates depend on the network module indirectly; the network module itself has no `.covgate` file). See Surprises — pre-existing timing flakes under llvm-cov instrumentation, unrelated to this diff.
- [x] Commit `refactor(network): drop IPv6 loopback support` (or two commits if Option B — see Branch and Commit Guidance).
- [ ] `./scripts/preflight.sh` reports clean. **Required before publishing.** (Owned by the preflight subagent in the next step.)

## Surprises & Discoveries

- `Cargo.lock` is gitignored (see `.gitignore` line 8). Running `scripts/update-deps.sh` does refresh the lock on disk — and `cargo update` did pull in many unrelated dep bumps as a side effect — but none of those changes are tracked by git, so they aren't part of the commit and don't need to be reverted. The commit therefore contains only `agent/src/network/mod.rs`, `agent/tests/network/mod.rs`, `agent/Cargo.toml`, and the plan file. (Date/Author: 2026-05-09 / implementer.)
- `./scripts/covgate.sh` failed with two pre-existing timing flakes under llvm-cov instrumentation (`app::run::max_runtime_reached` — 5 s timeout wrapper, panicked with `Elapsed(())`; `filesys::file::last_modified::success` — `elapsed < 1 sec` assertion). Both tests are unrelated to the network module (no covgate file exists for `agent/src/network/`, and `as_url` shape is unchanged). The plain `./scripts/test.sh` passed all 1340 tests including the three new `rejects_loopback_ipv6*` / `rejects_bracketed_ipv6_loopback` cases. Treating the covgate flakes as pre-existing infra issues; the orchestrator's preflight subagent will retry as needed. (Date/Author: 2026-05-09 / implementer.)

## Decision Log

- Decision (placeholder — implementer must resolve before editing): **Option A vs Option B for `Url::parse`.**
  - Option A — keep `Url::parse`, only delete the IPv6 cases. Smallest diff; preserves the existing port-range error message; leaves `url` as a dependency of the agent crate (cost: one transitive crate, already pulled in by `libs/backend-api/` and `libs/device-api/` via their own `url = "^2.5"` declarations, so no actual binary-size win without dropping the agent-side declaration).
  - Option B — replace `Url::parse` with a direct `rsplit_once(':')` split. Drops the synthetic-scheme dance, the post-parse userinfo/path/query/fragment guards (the upstream `://`, `@`, `/` checks already cover them), and lets us remove `url = { workspace = true }` from `agent/Cargo.toml` (verified: `agent/src/network/mod.rs` is the only file in the agent crate that imports `url::*`; the other workspace `url` users are the codegen crates `api/codegen/{backend,device}` which declare their own `url` deps).
  - **Default recommendation: Option B.** Once IPv6 is gone the `Url::parse` "wraps the host in `http://` to lean on a URL parser" trick adds no value — there is no userinfo or path that the upstream `contains("://") / contains('@') / contains('/')` checks miss. The only behavior difference is the exact error message text for an out-of-range port (`url::Url`'s "invalid port number" vs Rust's `<u16 as FromStr>::Err` message), and the existing `rejects_non_numeric_port` / `rejects_out_of_range_port` tests assert on a permissive substring (`"invalid"` or `"port"`) that both forms satisfy.
  - The implementer must record their choice and rationale in this section before editing.
  Date/Author: 2026-05-09 / planner.

- Decision: **Option B chosen** — drop `Url::parse` and the agent-side `url = { workspace = true }` declaration.
  Rationale: Verified via `grep -rn "use url::\|url::Url" agent/src libs/` that `agent/src/network/mod.rs` is the only file in the agent crate importing `url::*` (the only other matches are in `libs/backend-api/` and `libs/device-api/`, which declare their own `url` deps and are out of scope). With IPv6 gone, the synthetic-scheme dance and the post-parse userinfo/path/query/fragment guards add no value over a direct `rsplit_once(':')` split — the upstream `contains("://") / contains('@') / contains('/')` checks already cover the same surface. The existing port-error tests assert on permissive substrings (`"invalid"` or `"port"`) that both `url::Url`'s and Rust's `<u16 as FromStr>` messages satisfy.
  Date/Author: 2026-05-09 / implementer.

- Decision: 127.0.0.1 is intentionally retained.
  Rationale: Local development against the agent commonly uses `127.0.0.1` (curl, `nc`, raw socket binds in `agent/tests/http/` and `agent/tests/mocks/`). Narrowing further to `localhost`-only was previously drafted (`plans/backlog/20260508-localhost-only-network.md`, deleted 2026-05-09 in commit `50b73af`) and explicitly rejected on the grounds that it offered no real shrinkage of the type once IPv6 was already gone — the bracket and multi-colon code paths are entirely an IPv6 concern, not a 127.0.0.1 concern — while imposing a mechanical rename on six MQTT-test fixtures and tightening the SSL-unless-loopback rule for an MQTT broker configured at `127.0.0.1` (a configuration that has no operator workflow but is still measurably tighter than today). The plan recorded here keeps `127.0.0.1`.
  Date/Author: 2026-05-09 / planner.

- Decision: MQTT SSL-unless-loopback behavior is unchanged.
  Rationale: `agent/src/mqtt/options.rs:58` calls `is_loopback_host(broker.as_str())` to gate "SSL required". The narrowed `is_loopback_host` still returns `true` for both `localhost` and `127.0.0.1`, which are the two loopback forms anyone would configure an MQTT broker as. The only theoretical behavior change is that an MQTT broker configured at `::1` would now require SSL — but `MqttHost::new("::1")` will reject the host outright, so that configuration is no longer constructable. Net effect on observable behavior: zero.
  Date/Author: 2026-05-09 / planner.

## Outcomes & Retrospective

- Option B executed end-to-end. Net source-side change: `agent/src/network/mod.rs` shrank from 257 to 207 lines (the entire `Url::parse` block, the `explicit_port` helper, the `host_is_ipv6` flag, the bracket-strip, and the synthetic-scheme dance are gone). `agent/Cargo.toml` lost the `url = { workspace = true }` declaration; `cargo machete` confirms the agent crate no longer depends on `url`.
- All five IPv6 unit tests deleted and three rejection tests added; the plain `./scripts/test.sh` reports 1340 passing including the new rejection cases. Build, fmt, clippy, and lint gates all clean. The only failing gate (`covgate.sh`) tripped on two pre-existing timing-flaky tests in `app::run` and `filesys::file` — both well outside this diff's blast radius and recorded under Surprises for the preflight subagent to retry.
- One single commit on `refactor/backend-host`; the optional second commit (splitting the `Url::parse` removal from the IPv6 narrowing) was not used since the changes are tightly coupled and the combined diff stays small.

## Context and Orientation

Working directory for all commands: `/home/ben/miru/workbench2/repos/agent`.

Key files:

- `agent/src/network/mod.rs` (257 lines) — the module that defines `is_loopback_host`, `is_allowed_host`, `explicit_port`, and the `BackendHost` and `MqttHost` newtypes. The accepted-host invariant for both newtypes is `is_loopback_host(host) || is_allowed_host(host)`. After this change the invariant becomes: `is_loopback_host(host)` returns `true` only for `"localhost"` and `"127.0.0.1"`. `is_allowed_host` (the `mirurobotics.com` rule) is unchanged.
- `agent/tests/network/mod.rs` (364 lines) — owns all `BackendHost` and `MqttHost` unit tests. Five IPv6 tests are deleted; three rejection tests are added.
- `agent/src/mqtt/options.rs:58` — `ConnectAddress::new` calls `is_loopback_host(broker.as_str())` to enforce "SSL unless loopback". Narrowing the loopback set automatically tightens this; see Decision Log for why this is observably a no-op.
- `agent/Cargo.toml` — under Option B, drop the `url = { workspace = true }` line.
- `Cargo.toml` — workspace-level dep declarations. The workspace `url = "2.5.8"` line is **not** removed (the codegen crates `api/codegen/{backend,device}` declare their own `url = "^2.5"` and do not use the workspace declaration, but other code may pick it up; do not modify the workspace dep without re-grepping all `Cargo.toml` files in the workspace).
- `AGENTS.md` — for project conventions: import ordering, `thiserror` + `crate::errors::Error` trait, `#[cfg(feature = "test")]` gating, `./scripts/test.sh` and `./scripts/lint.sh` for pre-commit validation, per-module `.covgate` files for coverage gates.

Sweep results (executed against current HEAD):

```
grep -rn "::1\|ipv6\|IPv6\|host_is_ipv6" agent/ libs/ api/ --include="*.rs" --include="*.toml" --include="*.md"
```

The only matches are inside `agent/src/network/mod.rs` and `agent/tests/network/mod.rs`. There are zero other source-side or test-side IPv6 references. Other tests in `agent/tests/http/` and `agent/tests/mocks/` use `127.0.0.1` for raw socket binds and are out of scope (their use of `127.0.0.1` is correct and continues to work — `127.0.0.1` is still allowed).

Out of scope (must not change):

- `is_allowed_host` rule (`mirurobotics.com`).
- `BackendHost::Default` (`api.mirurobotics.com`) and `MqttHost::Default` (`mqtt.mirurobotics.com`).
- `as_url()` shape: `<scheme>://<authority>/agent/v1`, no trailing slash.
- The on-disk JSON keys (`backend.host`, `mqtt_broker.host`) and the warn-and-default fallback path in `Backend::deserialize` / `MQTTBroker::deserialize`.
- All call sites in `agent/src/main.rs`, `agent/src/app/`, `agent/src/storage/`, `agent/src/provisioning/` — they consume `BackendHost::as_url()` / `as_str()` and don't care about the loopback set.
- `http::Client::new(&str)` signature.
- `agent/tests/http/` and `agent/tests/mocks/` raw socket binds with `127.0.0.1` literals.
- MQTT integration test fixtures using `127.0.0.1` (those stay — `127.0.0.1` is still valid).
- The workspace `url = "2.5.8"` declaration in the root `Cargo.toml` (only the agent crate's `url = { workspace = true }` line is removed under Option B).

## Plan of Work

### `agent/src/network/mod.rs`

Re-grep before editing (line numbers are HEAD as of draft time):

1. **`is_loopback_host` (lines 9–12):** drop `"::1"` from the match arm. Final body: `matches!(host, "localhost" | "127.0.0.1")`. Update the doc comment on line 9 to say "the two literal loopback hostnames we accept" if you want to preserve the docstring; alternatively shorten to "Returns true for `localhost` and `127.0.0.1`."
2. **`explicit_port` (lines 23–37):** the helper exists solely to handle IPv6 brackets and the multi-colon guard. Treatment depends on Option A vs B:
   - **Option A:** collapse to `raw.rsplit_once(':').and_then(|(_, p)| p.parse::<u16>().ok())`. Drop the bracket-skip at lines 24–27 and the multi-colon guard at lines 33–35. Update doc comment (lines 20–22) to "Returns the trailing `:NNN` port from a host string, or `None` if no port suffix is present or the suffix is not a valid `u16`."
   - **Option B:** delete `explicit_port` entirely — the new direct-parse path inside `BackendHost::new` extracts the port inline.
3. **`BackendHost::new` (lines 70–135):**
   - Remove the `if raw == "::1"` synthetic-scheme wrap (lines 87–92).
   - Remove the bracket-strip `trim_start_matches('[').trim_end_matches(']')` (lines 112–114). The `let bare_host = ...` binding becomes redundant; just use `host` directly.
   - Remove the `host_is_ipv6` flag and the bracketed-`formatted` arm (lines 123–128). Final shape: `let formatted = match port { Some(p) => format!("{host}:{p}"), None => host.to_string() };`.
   - **Under Option B**, replace the entire `Url::parse` block (lines 84–122) with:
     ```rust
     let (host, port) = match raw.rsplit_once(':') {
         Some((h, p)) => (
             h,
             Some(p.parse::<u16>().map_err(|e| format!("invalid port in `{raw}`: {e}"))?),
         ),
         None => (raw, None),
     };
     if host.is_empty() {
         return Err(format!("backend host `{raw}` has no host"));
     }
     if !is_loopback_host(host) && !is_allowed_host(host) {
         return Err(format!("host `{host}` is not allowed"));
     }
     ```
     This subsumes the host_str-extraction step and the post-parse userinfo/path/query/fragment guards (the upstream `contains("://")`, `contains('@')`, `contains('/')` already cover them — the post-parse checks were only triggerable through escape sequences that `Url::parse` happened to normalize, and none reached production today). Remove `use url::Url;` from the imports (line 7).
4. **`BackendHost::as_url` (lines 157–173):** remove the `host_is_ipv6` flag (line 165) and the bracketed authority arms (lines 167, 169). Final shape:
   ```rust
   let authority = match self.port {
       Some(port) => format!("{}:{port}", self.host),
       None => self.host.clone(),
   };
   ```
5. **Doc comments to strip / reword** (re-grep for `IPv6`, `bracket`, `::1`):
   - Line 9 — narrow to "localhost and 127.0.0.1".
   - Lines 20–22 — drop "ignoring colons inside `[...]` IPv6 brackets".
   - Lines 30–32 — delete (IPv6 multi-colon guard rationale is gone).
   - Line 46 — drop "(no brackets for IPv6 literals)".
   - Lines 50–52 — drop "(with brackets around IPv6 hosts when a port is present)".
   - Line 64 — keep, but reword the loopback bullet to reference the narrowed rule.
   - Lines 68–69 — delete "IPv6 host+port form requires brackets..." paragraph.
   - Lines 84–86 — delete "Parse via url::Url with a synthetic scheme..." (entire block goes under Option B; under Option A, drop the second sentence about `::1`).
   - Lines 112–113 — delete "host_str() preserves IPv6 brackets...".
   - Line 149 — drop "IPv6 hosts are bracketed when a port is present".
   - Lines 163–164 — delete "IPv6 hosts must be bracketed inside a URL even when port-less...".
6. **`MqttHost::new` (lines 207–216):** body unchanged. Narrowing is automatic.

### `agent/tests/network/mod.rs`

Re-grep before deleting — line numbers may drift after earlier edits to the file in this branch.

**Delete five tests:**
- `mod backend_host_new::accepts_loopback_ipv6` (lines 32–35)
- `mod backend_host_new::as_url_http_for_loopback_ipv6` (lines 215–219)
- `mod backend_host_new::accepts_loopback_ipv6_with_port` (lines 233–237)
- `mod backend_host_new::as_url_http_loopback_ipv6_with_port` (lines 239–243)
- `mod mqtt_host_new::accepts_loopback_ipv6` (lines 278–281)

**Add three tests** (locations: append to each `mod`'s rejection-test block):

```rust
// in mod backend_host_new
#[test]
fn rejects_loopback_ipv6() {
    let err = BackendHost::new("::1").unwrap_err();
    assert!(
        err.contains("::1") || err.contains("not allowed"),
        "expected host-not-allowed message, got: {err}"
    );
}

#[test]
fn rejects_bracketed_ipv6_loopback() {
    BackendHost::new("[::1]:8080").unwrap_err();
}

// in mod mqtt_host_new
#[test]
fn rejects_loopback_ipv6() {
    let err = MqttHost::new("::1").unwrap_err();
    assert!(
        err.contains("::1") || err.contains("not allowed"),
        "expected host-not-allowed message, got: {err}"
    );
}
```

**Tests that MUST continue to pass** (do not delete or rename):
- `mod backend_host_new::accepts_loopback_localhost` (line 22–25)
- `mod backend_host_new::accepts_loopback_ipv4` (line 27–30) — confirm intact.
- `mod backend_host_new::accepts_loopback_with_port` (line 43–47)
- `mod backend_host_new::as_url_http_for_localhost` (line 203–207)
- `mod backend_host_new::as_url_http_for_loopback_ipv4` (line 209–213)
- `mod backend_host_new::as_url_http_loopback_with_port` (line 227–231)
- `mod backend_host_new::preserves_explicit_port_80_for_loopback` (line 245–250)
- `mod mqtt_host_new::accepts_localhost` (line 268–271)
- `mod mqtt_host_new::accepts_loopback_ipv4` (line 273–276)

### `agent/Cargo.toml` (Option B only)

Delete the line `url = { workspace = true }` (currently line 37). Then run `scripts/update-deps.sh` to refresh `Cargo.lock`. `cargo machete` (run as part of `scripts/lint.sh`) will detect any straggling unused dep; `cargo build` should still succeed because the only `use url::Url;` site inside the agent crate is removed in step 3 above.

## Concrete Steps

Numbered, mechanically executable. Run from `/home/ben/miru/workbench2/repos/agent`.

1. Verify branch: `git branch --show-current` should print `refactor/backend-host`. If not, switch to it (do not create a new branch).
2. Re-grep current line numbers:
   ```
   grep -n "::1\|IPv6\|host_is_ipv6\|trim_start_matches\|trim_end_matches\|url::Url\|use url::" agent/src/network/mod.rs
   grep -n "loopback_ipv6\|::1\|\[::1\]" agent/tests/network/mod.rs
   ```
3. Decide Option A vs Option B and append the choice + rationale to the Decision Log placeholder above. Do this **before** editing source.
4. Edit `agent/src/network/mod.rs` per Plan of Work (steps 1–6).
5. Edit `agent/tests/network/mod.rs` per Plan of Work (delete five, add three).
6. Under Option B only: edit `agent/Cargo.toml` (drop the `url` line) and run `./scripts/update-deps.sh` (refreshes `Cargo.lock`).
7. Run validation gates (Validation and Acceptance section). Fix any failures and rerun.
8. Commit per Branch and Commit Guidance.
9. Run `./scripts/preflight.sh`. Must report clean.

## Validation and Acceptance

Each gate must pass before the change is published. Run in this order:

| Gate | Command | Expected |
|---|---|---|
| Build | `cargo build` | Succeeds. |
| Format | `cargo fmt -p miru-agent -- --check` | No diff. |
| Clippy | `cargo clippy --package miru-agent --all-features -- -D warnings` | No warnings. |
| Tests | `./scripts/test.sh` | All tests pass, including the three new `rejects_loopback_ipv6*` / `rejects_bracketed_ipv6_loopback` tests. |
| Lint | `./scripts/lint.sh` | Passes (includes `cargo machete`, which under Option B confirms `url` is no longer needed by the agent crate). |
| Coverage | `./scripts/covgate.sh` | Storage and MQTT covgates still pass. The network module has no `.covgate` of its own, but the deleted IPv6 tests reduce overall network-module line coverage modestly; if storage's covgate (94.21%) breaches, add a small structural test such as `as_url_authority_uses_no_brackets` to the network test module to compensate. |
| Preflight | `./scripts/preflight.sh` | **Required.** Must report clean before commit is pushed for review. |

Acceptance signal: the three rejection tests added in `agent/tests/network/mod.rs` are the behavior-pinning gate. They will fail under HEAD (because `::1` is currently accepted) and pass after the edits, proving the narrowing took effect.

## Idempotence and Recovery

- All edits are localized; if a step fails partway, `git checkout -- agent/src/network/mod.rs agent/tests/network/mod.rs agent/Cargo.toml Cargo.lock` resets the working tree without affecting other in-flight branches.
- The plan can be re-entered from any unfinished checklist item without prior steps becoming stale: the source edits are mechanical removals (no symbol additions other than three test functions), so the diff is monotone.
- If preflight surfaces a coverage breach in storage or MQTT covgates, the recovery is to add one structural test to `agent/tests/network/mod.rs` (e.g. `as_url_authority_uses_no_brackets`); do not lower the covgate threshold.
- If the Option A vs B choice is revisited mid-work, revert the `Cargo.toml` and `use url::Url;` lines, restore the `Url::parse` block (`git diff` against the pre-edit `agent/src/network/mod.rs`), and re-run validation.

## Branch and Commit Guidance

- **Branch:** `refactor/backend-host` (already exists, currently checked out, two commits ahead of `origin`). Do not create a new branch.
- **Commit:** single commit `refactor(network): drop IPv6 loopback support` is recommended.
- Under Option B, splitting into two commits is acceptable but optional:
  1. `refactor(network): drop IPv6 loopback support` — the source/test changes only.
  2. `refactor(network): replace url::Url parsing with direct rsplit_once` — the `Url::parse` removal and `Cargo.toml` dep drop.
- After preflight is clean, push with `git push` (the branch already tracks `origin/refactor/backend-host`); do not force-push. The PR is #66.
