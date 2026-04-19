# Fix rustls-webpki Security Advisories (RUSTSEC-2026-0098 / RUSTSEC-2026-0099)

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent` (this repo) | read-write | All changes land here: workspace `Cargo.toml`, `agent/Cargo.toml`, `agent/src/mqtt/client.rs`, `.cargo/audit.toml`, and `scripts/` invocations. |
| `bytebeamio/rumqtt` (crates.io) | read-only | Source of the transitive `rustls-webpki 0.102.8` import. No newer release exists; we must work around it. |
| `rustls/webpki` (crates.io) | read-only | Upstream of the advisory. Fix exists only in 0.103.10+ (for -0099) and 0.103.11+ (for -0098). No 0.102.x backport on crates.io. |

This plan lives in `agent/plans/` because all code changes are made inside the `agent` repo.

## Purpose / Big Picture

`cargo audit` (invoked via `./scripts/lint.sh`) fails on PR #42 with two rustls-webpki 0.102.8 advisories:

- **RUSTSEC-2026-0098**: Name constraints for URI names were incorrectly accepted.
- **RUSTSEC-2026-0099**: Name constraints were accepted for certificates asserting a wildcard name.

Both require upgrading to `rustls-webpki >=0.103.12, <0.104.0-alpha.1` or `>=0.104.0-alpha.6`. Neither `rumqttc 0.25.1` nor `rumqttd 0.20.0` (the transitive consumers) has a release with this upgrade; the crates.io version of both still pins `rustls-webpki 0.102.x`. There is no 0.102.9+ patch release on crates.io.

After this change, a developer runs `./scripts/lint.sh` (or CI runs `rustsec/audit-check`) and `cargo audit` reports no RUSTSEC-2026-0098 or RUSTSEC-2026-0099 findings against the workspace. The MQTT client and test broker still build, all existing tests still pass, and production TLS-to-cloud-MQTT behaviour is preserved (or consciously replaced, per the chosen route).

The plan selects a route during Milestone 0, then executes it. Two routes are prepared; the chosen one becomes authoritative and the other is recorded as rejected in the Decision Log.

## Progress

- [ ] Milestone 0: Route selection & spike.
- [ ] Milestone 1: Implement chosen route.
- [ ] Milestone 2: Validate — run `./scripts/lint.sh`, `cargo build --workspace`, `cargo test --workspace --features test`.
- [ ] Milestone 3: Preflight (`./scripts/preflight.sh`) clean.
- [ ] Milestone 4: Commit and push.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Investigate two routes in Milestone 0 before coding.
  Rationale: No drop-in semver-compatible upgrade exists for `rustls-webpki 0.102.x`; each route has trade-offs.
  Date/Author: 2026-04-19 / plan author.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

**Repository layout (relevant parts).**

- `Cargo.toml` (workspace root) — declares `[workspace.dependencies]` including `rumqttc = "0.25.1"` and `rumqttd = "0.20.0"`. Also declares `openssl = { version = "0.10.64", features = ["vendored"] }` already used for HTTP/TLS in `reqwest`.
- `agent/Cargo.toml` — the `miru-agent` binary crate. Depends on `rumqttc` (runtime) and has `rumqttd` as a dev-dependency (used only by the in-process test broker in `agent/tests/mocks/mqtt_client.rs`).
- `agent/src/mqtt/client.rs` — MQTT client wiring. Uses `rumqttc::Transport::Tls(Default::default())` at line 59 when `Protocol::SSL` is selected. `Default::default()` yields a rustls-backed `TlsConfiguration`.
- `agent/src/workers/mqtt.rs` — the worker that drives the client.
- `agent/tests/mocks/mqtt_client.rs` — dev-dep-only broker used by integration tests. Broker config sets `tls: None` (plain TCP); does not use websocket or TLS features.
- `.cargo/audit.toml` — `cargo audit` ignore list. Already ignores `RUSTSEC-2026-0049` with a reference to `https://github.com/mirurobotics/agent/issues/2` because of the same `rumqttd`/`rumqttc` version pinning.
- `scripts/lint.sh` → `scripts/lib/lint.sh` — runs (in order) custom import linter, `cargo fmt`, `cargo machete` (unused deps), `cargo audit`, and `cargo clippy`. `cargo audit` is the step currently failing.
- `scripts/update-deps.sh` — runs `cargo update --verbose`; refreshes the uncommitted `Cargo.lock`. Cargo.lock is not committed in this repo; the CI job regenerates it via `update-deps.sh` before auditing.
- `scripts/preflight.sh` — runs lint + tests + tools-lint + tools-tests in parallel; must report "Preflight clean".

**Terms.**

- *RUSTSEC-*  identifiers are advisory IDs from `rustsec.org`. `cargo audit` consumes them.
- *Transitive dependency*: a crate you do not directly depend on; you inherit it from a direct dep's `Cargo.toml`. `rustls-webpki` is pulled in by `rumqttc` (`use-rustls` feature) and `rumqttd` (`use-rustls` feature).
- *`[patch.crates-io]`*: Cargo's mechanism for replacing a crates.io package at lock-resolution time with a git source or local path. The replacement must advertise a **semver-compatible** version to the consumer — patching 0.102.x with 0.103.x fails because the consumer's `rustls-webpki = "0.102"` requirement does not accept 0.103.
- *`use-native-tls` feature*: a feature on both `rumqttc` and `rumqttd` that swaps rustls for `native-tls`/`tokio-native-tls`, which on Linux defaults to OpenSSL. Enabling it while disabling default features removes the `rustls-webpki` transitive dep entirely.

**Key upstream facts (verified during research; see Surprises if they change).**

- `rustls-webpki` on crates.io: latest 0.102.x is `0.102.8`; no `0.102.9` exists. 0.103.12 is the latest 0.103.x.
- `rumqttc 0.25.1` `Cargo.toml`: `rustls-webpki = { version = "0.102.8", optional = true }`. No newer release.
- `rumqttd 0.20.0` `Cargo.toml`: `rustls-webpki = { version = "0.102.2", optional = true }`. No newer release.
- `bytebeamio/rumqtt` main branch: unchanged; no open tag bumps `rustls-webpki` to 0.103.
- `rumqttc 0.25.1` `Transport` enum exposes variants `Tcp`, `Tls(TlsConfiguration)`, `Wss(TlsConfiguration)`. `TlsConfiguration::Default` (selected by `Default::default()`) uses rustls when `use-rustls` is on; the native-tls path uses `TlsConfiguration::NativeConnector(tokio_native_tls::TlsConnector)` via `From<TlsConnector>`.
- `rustls-pemfile 2.2.0` unmaintained warning (`RUSTSEC-2025-0134`) is a warning-level finding, not an error. Treated as informational and left alone by this plan unless the chosen route incidentally removes `rustls-pemfile`.

## Plan of Work

### Milestone 0 — Route selection (spike, ~30 min)

Evaluate the two viable routes and pick one. Record the decision in the Decision Log.

**Route A — Switch MQTT to `use-native-tls` (recommended primary).**

- Idea: disable `rumqttc` and `rumqttd` default features (which include `use-rustls`) and enable `use-native-tls`. This removes `rustls-webpki` from the dependency graph entirely, making both advisories vanish. `native-tls` on Linux uses system OpenSSL; the workspace already vendors OpenSSL via `openssl = { version = "0.10.64", features = ["vendored"] }`, which `native-tls` can also use through its `vendored` feature.
- Code impact: in `agent/src/mqtt/client.rs` line 59, `Transport::Tls(Default::default())` currently constructs a rustls config. Under `use-native-tls` (without `use-rustls`), `TlsConfiguration::Default` does not exist / resolves to a different variant depending on feature gating. Construct a `native_tls::TlsConnector::new()` explicitly and wrap with `tokio_native_tls::TlsConnector::from(...)`, then `Transport::tls_with_config(TlsConfiguration::NativeConnector(connector))`.
- Dev-dep impact: `rumqttd` default features include `websocket`; disabling defaults drops websocket. The existing test broker (`agent/tests/mocks/mqtt_client.rs`) uses plain TCP only (`ServerSettings.tls = None`, no `ws_*` settings). Confirm during the spike by reading every use site of `rumqttd` types.
- Lock file impact: `rustls-webpki`, `rustls`, `tokio-rustls`, `rustls-pemfile`, `rustls-native-certs` all drop out. `cargo audit` clears both -0098 and -0099 plus incidentally the -2025-0134 warning.
- Risks: production MQTT in prod uses `Protocol::SSL` against a real broker — switching the TLS stack changes certificate validation semantics (system trust store instead of webpki-roots). Enumerate cert handling by reading current `Transport::Tls(Default::default())` semantics: in rumqttc 0.25.1, `TlsConfiguration::Default` loads native roots via `rustls-native-certs`, so both stacks already consult the OS trust store. Risk is small but must be called out and tested.
- Validation signal for the spike: `cargo tree --workspace --target x86_64-unknown-linux-gnu | grep rustls-webpki` returns nothing after the feature flip.

**Route B — `[patch.crates-io]` with a mirurobotics fork of `bytebeamio/rumqtt`.**

- Idea: fork `bytebeamio/rumqtt` to `mirurobotics/rumqtt`, on a branch `miru-rustls-webpki-0.103`, bump `rumqttc/Cargo.toml` to `rustls-webpki = "0.103.12"` and `tokio-rustls = "0.26"`, bump `rumqttd/Cargo.toml` similarly, fix any API breakage (rustls 0.22 → 0.23 migration inside rumqttc), publish nothing. Reference the branch via `[patch.crates-io]` in workspace `Cargo.toml`:

      [patch.crates-io]
      rumqttc = { git = "https://github.com/mirurobotics/rumqtt", branch = "miru-rustls-webpki-0.103" }
      rumqttd = { git = "https://github.com/mirurobotics/rumqtt", branch = "miru-rustls-webpki-0.103" }

- Code impact on this repo: none beyond the `[patch.crates-io]` block.
- Risks: rustls 0.22 → 0.23 has breaking API changes; the fork will be non-trivial and ongoing maintenance burden. Cannot release the agent binary without vendoring or publishing the fork. Larger attack surface for drift.
- When to pick: only if Route A spike reveals a blocker (e.g., native-tls breaks an undocumented TLS behaviour the agent relies on).

**Spike steps (read-only; no code changes yet).**

1. From `/home/ben/miru/workbench2/repos/agent`, run:

       cargo tree --workspace -e normal --target x86_64-unknown-linux-gnu | grep -E "rustls-webpki|rustls |tokio-rustls|native-tls"

   Expected transcript shows `rustls-webpki 0.102.8` today.

2. Read `rumqttc 0.25.1` source for `TlsConfiguration` / `Transport::tls_with_config` to confirm the constructor works with `use-native-tls` standalone. Source: `https://docs.rs/rumqttc/0.25.1/src/rumqttc/`.

3. Read `agent/src/mqtt/client.rs`, `agent/src/workers/mqtt.rs`, and every file in `agent/tests/mqtt/` and `agent/tests/mocks/mqtt_client.rs` to catalog rumqttc/rumqttd API surface in use.

4. Decide. Record the decision, rationale, and rejected-route summary in the Decision Log.

### Milestone 1 — Implement chosen route

**If Route A (native-tls):**

1. Edit `Cargo.toml` (workspace root):

   Change:

       rumqttc = "0.25.1"
       rumqttd = "0.20.0"

   to:

       rumqttc = { version = "0.25.1", default-features = false, features = ["use-native-tls"] }
       rumqttd = { version = "0.20.0", default-features = false, features = ["use-native-tls"] }

   Leave other `[workspace.dependencies]` entries alone.

2. Ensure `native-tls` uses the vendored OpenSSL the workspace already builds. Add directly under the `openssl` entry:

       native-tls = { version = "0.2.12", features = ["vendored"] }
       tokio-native-tls = "0.3.1"

   And add `native-tls = { workspace = true }` + `tokio-native-tls = { workspace = true }` to `agent/Cargo.toml` `[dependencies]`.

3. Edit `agent/src/mqtt/client.rs`. Replace the `Protocol::SSL` arm so it builds a native-tls transport explicitly:

       Protocol::SSL => {
           let connector = native_tls::TlsConnector::new()
               .map_err(|e| /* map into MQTTError; follow existing error patterns in agent/src/mqtt/errors.rs */)?;
           let connector = tokio_native_tls::TlsConnector::from(connector);
           mqtt_options.set_transport(
               rumqttc::Transport::tls_with_config(
                   rumqttc::TlsConfiguration::NativeConnector(connector)
               )
           );
       }

   Update the `use` block at the top of `client.rs` to import `native_tls` if the repo convention requires top-level imports (verify against repo import-ordering rule in `AGENTS.md`).

4. Adjust `Client::new`'s signature if `TlsConnector::new()` returns `Result`. Today `Client::new` returns `(Self, EventLoop)` without `Result`; either unwrap with a clear panic (production-safe because `TlsConnector::new()` only fails on system TLS init) or thread a `Result` through. Follow existing error-handling patterns in `agent/src/mqtt/errors.rs` (`impl_error!` macro, `crate::errors::Error` trait).

5. Verify `agent/tests/mocks/mqtt_client.rs` still compiles. The test broker uses `ServerSettings { tls: None, ... }` and no websocket — dropping `rumqttd`'s default features should leave it intact. If the code references `rumqttd::protocol::Protocol::WebSocket` or similar, bring back the `websocket` feature only where needed.

6. Run `./scripts/update-deps.sh` from the repo root to regenerate `Cargo.lock` with the new feature selection.

**If Route B (`[patch.crates-io]`):**

1. Create the fork (`mirurobotics/rumqtt`), push branch `miru-rustls-webpki-0.103` with the minimal `rustls-webpki = "0.103.12"` + `tokio-rustls = "0.26"` bump. Verify the fork's own CI passes.

2. Edit workspace `Cargo.toml` at the bottom (after `[profile.release]`):

       [patch.crates-io]
       rumqttc = { git = "https://github.com/mirurobotics/rumqtt", branch = "miru-rustls-webpki-0.103" }
       rumqttd = { git = "https://github.com/mirurobotics/rumqtt", branch = "miru-rustls-webpki-0.103" }

3. Run `./scripts/update-deps.sh`. If cargo emits a `[patch] unused` warning, adjust the patch entry's declared package name/version to match what the lock resolver expects.

4. Commit note: link the fork URL in a comment inside `Cargo.toml` so future readers know the provenance.

### Milestone 2 — Validation

(See "Validation and Acceptance" below for expected transcripts.)

### Milestone 3 — Preflight

Run `./scripts/preflight.sh`; must report `Preflight clean`. If any of `Lint`, `Tests`, `Tools Lint`, `Tools Tests` fails, fix the root cause — do not bypass.

### Milestone 4 — Commit

Commit the changes on branch `fix/rustls-webpki-security-advisories` with a Conventional Commit message:

    fix(deps): bump rustls-webpki via <route A: native-tls switch | route B: rumqtt patch>

    Resolves RUSTSEC-2026-0098 and RUSTSEC-2026-0099.
    Upstream rumqttc 0.25.1 / rumqttd 0.20.0 still pin rustls-webpki 0.102.x;
    no 0.102.9 backport exists. <Route rationale, one sentence.>

Then push and let CI re-run on PR #42.

## Concrete Steps

All commands assume working directory `/home/ben/miru/workbench2/repos/agent` unless stated.

**M0 — Spike.**

    git status                                   # verify branch is fix/rustls-webpki-security-advisories, tree clean
    ./scripts/update-deps.sh                     # refresh Cargo.lock on the current (pre-fix) state
    cargo tree --workspace -e normal | grep -E "rustls-webpki|rustls |tokio-rustls"
    # Expected: shows rustls-webpki v0.102.8 (once).

Read in full: `agent/src/mqtt/client.rs`, `agent/src/workers/mqtt.rs`, `agent/tests/mocks/mqtt_client.rs`, `agent/tests/mqtt/client.rs`. Record the decision in the Decision Log.

**M1 — Implement (Route A example).**

    # Edit Cargo.toml, agent/Cargo.toml, agent/src/mqtt/client.rs per Plan of Work.
    ./scripts/update-deps.sh
    cargo tree --workspace -e normal | grep -E "rustls-webpki|native-tls"
    # Expected: no rustls-webpki line; native-tls v0.2.x present.

**M2 — Validation.**

    cargo build --workspace
    # Expected: "Finished `dev` profile [unoptimized + debuginfo] target(s) in X.YZs"

    cargo audit
    # Expected: "Success No vulnerable packages found" for the errors list.
    # RUSTSEC-2025-0134 may still appear as a warning (allowed).

    ./scripts/test.sh
    # Expected: "test result: ok. <N> passed; 0 failed" for the miru-agent crate.

    ./scripts/lint.sh
    # Expected: ends with "Lint complete" and exit 0.

**M3 — Preflight.**

    ./scripts/preflight.sh
    # Expected: ends with "Preflight clean".

**M4 — Commit and push.**

    git add Cargo.toml agent/Cargo.toml agent/src/mqtt/client.rs .cargo/audit.toml
    # Cargo.lock is not tracked; do not add it.
    git status                                    # verify what will be committed
    git commit -m "fix(deps): bump rustls-webpki via native-tls switch"
    git push origin fix/rustls-webpki-security-advisories

## Validation and Acceptance

Acceptance is behavioural and checked by these observations:

1. **`cargo audit` clears both advisories.** From repo root:

       cargo audit 2>&1 | grep -E "RUSTSEC-2026-0098|RUSTSEC-2026-0099"

   Expected: no output, exit status 1 on the grep (pattern not found). Before the fix, the same command prints both advisory lines and `cargo audit` itself exits non-zero.

2. **`./scripts/lint.sh` succeeds end-to-end.** From repo root:

       ./scripts/lint.sh
       echo "exit=$?"

   Expected: final lines include `Lint complete` and `exit=0`. The `Security vulnerabilities` section prints "No vulnerable packages found" (warnings permitted).

3. **The agent behavioural suite still passes.** From repo root:

       ./scripts/test.sh

   Expected: final line matches `test result: ok. <N> passed; 0 failed; ...` for the miru-agent crate. The MQTT integration tests in `agent/tests/mqtt/` exercise connection, publish, subscribe, and reconnect paths against the in-process `rumqttd` broker — they must pass unmodified, proving the `use-native-tls` switch did not regress behaviour.

4. **Preflight clean.** From repo root:

       ./scripts/preflight.sh

   Expected: trailing line is `Preflight clean`. Implementation is not considered done until this reports clean.

## Idempotence and Recovery

- `./scripts/update-deps.sh`, `cargo build`, `cargo audit`, `./scripts/lint.sh`, `./scripts/test.sh`, `./scripts/preflight.sh` are all idempotent — re-run freely.
- Editing `Cargo.toml` is reversible with `git checkout -- Cargo.toml agent/Cargo.toml`.
- If Route A reveals a blocker mid-implementation (e.g., rumqttd with `default-features = false` fails to compile because an undocumented item is gated behind a default feature), revert the `Cargo.toml` edits and proceed with Route B. Record the blocker in Surprises & Discoveries and the route pivot in the Decision Log.
- If Route B is chosen and the fork URL in `[patch.crates-io]` is wrong or unreachable, `cargo` errors immediately at resolution time with a clear message; fix the URL and re-run `./scripts/update-deps.sh`. No partial state is written.
- Cargo.lock is not checked in, so a broken lock file cannot contaminate the repo; `./scripts/update-deps.sh` regenerates it on every run.
- Rollback path for a merged-then-regretted fix: revert the single commit with `git revert <sha>` and reopen the advisory-tracking issue. The `.cargo/audit.toml` precedent (`RUSTSEC-2026-0049`) shows the fallback pattern — ignore with a tracking-issue comment — which must only be used if both routes fail their spikes.
