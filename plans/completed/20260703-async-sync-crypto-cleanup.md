# Async/sync cleanup: offload CPU-bound RSA keygen and drop pointless async

## Scope

| Repo | Path | Access |
| --- | --- | --- |
| agent | /home/ben/miru/workbench4/repos/agent | read-write |

This plan file lives in the agent repo at `plans/backlog/20260703-async-sync-crypto-cleanup.md`.

Git note: the orchestrator owns branching. Do NOT create or switch branches. Commit from inside the agent repo's own git context (cwd `/home/ben/miru/workbench4/repos/agent`), never from the workbench root. Follow the repo's commit-message trailer convention (`Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`).

Generated code under `libs/backend-api/` and `libs/device-api/` is out of scope and MUST NOT be touched.


## Purpose / Big Picture

An async audit of the agent found two classes of async/sync misuse. This task fixes them as pure refactors — **no observable behavior changes**. Existing tests passing is therefore the primary success signal.

**Category A — offload CPU-bound RSA crypto to a blocking thread.** `agent/src/crypt/rsa.rs::gen_key_pair()` runs a 4096-bit `Rsa::generate()` (hundreds of ms of pure CPU) directly on a tokio async worker thread. On low-core robot devices this pins one of very few worker threads and stalls concurrent async tasks (the MQTT event loop, the sync poller, the local Unix-socket server). Wrapping the keygen in `tokio::task::spawn_blocking` moves it onto tokio's dedicated blocking-thread pool so it no longer starves the async runtime. This is the **priority, highest-value** change. The much cheaper (sub-ms) `sign`/`verify` functions are evaluated but, per the audit's own guidance, are expected to be **left as-is** (see Decision Log) because thread-move + `Send + 'static` overhead can exceed the work saved.

**Category B — drop `async` from functions that do no async work.** Five functions are `async` but contain no `.await` and perform only pure/in-memory/synchronous work; that needless `async` colors every caller (forcing `.await` and async contexts to propagate). Converting each to a plain `fn` and removing `.await` at call sites simplifies the call graph with zero behavior change. Each candidate is verified to do no async work before converting. Where dropping `async` would ripple across an unreasonable number of call sites or is ambiguous, the plan **leaves the function alone and documents why** rather than making a risky mechanical change (this applies to `create_temp_dir`, see below).


## Progress

- [ ] M1: Offload `gen_key_pair` RSA keygen to `spawn_blocking` (Category A priority)
- [ ] M2: Convert Category B pure-sync fns to `fn` + update call sites
- [ ] M3: Validate (build, test, preflight clean)


## Surprises & Discoveries

(To be filled in during implementation. Record anything that contradicts this plan here — e.g. a "pure sync" function that turns out to await, or a call site the enumeration below missed.)


## Decision Log

- 2026-07-03 (author): **`gen_key_pair` — wrap.** The CPU-bound `Rsa::generate(num_bits)` call is cleanly separable: its only input is `num_bits: u32` (a `Copy` value, trivially `Send + 'static`), and its output `Rsa<Private>` is `Send + 'static`. The surrounding async file writes stay outside the closure. This is the clean, high-value wrap.
- 2026-07-03 (author): **`sign` / `sign_rs256` / `sign_rs512` / `verify` — expected outcome: LEAVE, document.** These do sub-millisecond CPU work, and the CPU portion is interleaved with an async key-file read (`read_private_key(...).await` / `read_public_key(...).await`) that happens *first*, inside the same function. To wrap only the CPU tail in `spawn_blocking`, the closure would have to take ownership of the loaded `Rsa<Private>`/`Rsa<Public>` key plus the data bytes and move them across a thread boundary. For sub-ms work, that thread-move + allocation overhead can cost more than it saves, and `sign_rs512` is on the token-issuing hot path (`authn/issue.rs:84` → `mint_jwt`). Per the audit's explicit guidance ("wrap only if clean; otherwise leave and document — do NOT force it"), the default decision is to **leave these four unchanged**. The implementer may override only if a wrap turns out to be genuinely clean and measured to help; if so, record the reversal here.
- 2026-07-03 (author): **`Dir::create_temp_dir` — LEAVE (ripples too far).** It is genuinely pure-sync (`tempfile::Builder`), so it *qualifies* for Category B on the merits. But it has 2 production call sites (`main.rs:59`, `main.rs:112`) and **~250+ test call sites** across `agent/tests/` (it is the standard test-fixture temp-dir helper). Converting it to `fn` would require deleting `.await` at every one of those sites — a large, noisy, high-risk mechanical edit for a function whose cost (a single temp-dir creation) is trivial and never on a hot async path. Per the scope guard ("if dropping async ripples too far or is ambiguous, prefer to leave it and note why"), **do not convert `create_temp_dir`.** The other four Category B functions have small, bounded call-site sets and are converted.
- 2026-07-03 (author): **Reference pattern note.** The task brief cited an "existing correct `spawn_blocking` pattern at `agent/src/upload/uploader.rs:103`". That file/path does not exist in the current tree and there is **no existing `spawn_blocking` usage anywhere in `agent/src`** (verified: `grep -rn spawn_blocking agent/src` returns nothing). So there is no in-repo pattern to mirror; M1 introduces the first `spawn_blocking` use, following the canonical tokio idiom spelled out in Concrete Steps below.


## Outcomes & Retrospective

(To be filled in on completion: final validation results, commit hashes/messages, any deviations from plan.)


## Context and Orientation

Read this assuming no prior knowledge of the agent repo. All paths are relative to `/home/ben/miru/workbench4/repos/agent`.

### Repo conventions (from `AGENTS.md`)

- **Import ordering.** Every source file groups imports as: standard crates, then internal crates (`use crate::...`), then external crates — each group separated by a blank line and a `// <group>` comment. Any new `use` line goes in the correct group.
- **Error handling.** Error enums derive `thiserror::Error` and implement `crate::errors::Error`; aggregating enums use the `impl_error!` macro. This refactor introduces no new error variants (M1 keeps `CryptErr` and its existing variants; converting a fn from `async fn -> Result<..>` to `fn -> Result<..>` keeps the same error type).
- **Feature flags.** `#[cfg(feature = "test")]` gates test-only code. Not relevant here.
- **Testing.** Run `./scripts/test.sh` (it runs `RUST_LOG=off cargo test --package miru-agent --features test`). The `--features test` flag is MANDATORY — many mocks/helpers are behind `#[cfg(feature = "test")]` and tests fail with misleading errors without it. Tests run in parallel by default; tests binding shared OS resources are annotated `#[serial]`. Test files in `agent/tests/` mirror the `agent/src/` module structure.
- **Coverage gates.** Each module has a `.covgate` file with a minimum coverage %. `scripts/covgate.sh` enforces them. These refactors don't remove test coverage, so gates should stay satisfied.
- **Linting.** `./scripts/lint.sh` runs the custom import linter, `cargo fmt`, machete/diet unused-dep checks, security audit, and clippy `-D warnings`. Run `./scripts/update-deps.sh` first to refresh `Cargo.lock`.

### Category A target: `agent/src/crypt/rsa.rs`

- `gen_key_pair(num_bits: u32, private_key_file: &filesys::File, public_key_file: &filesys::File, overwrite: Overwrite) -> Result<(), CryptErr>` (currently `async fn`, defined at **line 40**). Body: line **47** does `let rsa = ssl_err!(GenerateRSAKeyPairErr, Rsa::generate(num_bits))?;` — this is the CPU-bound call to wrap. Everything after (PEM extraction + `write_bytes(...).await` + `set_permissions(...).await` for both keys) is async file I/O that must stay in the async body, *after* the awaited keygen result.
  - `ssl_err!` is a local `macro_rules!` (lines 19-28) that maps an `openssl::error::ErrorStack` to a `CryptErr` variant and attaches `trace!()`. It must run in the async context (it uses `trace!()` and `?`). Therefore: do the raw `Rsa::generate(num_bits)` *inside* the `spawn_blocking` closure, return its `Result<Rsa<Private>, openssl::error::ErrorStack>` out of the closure, then apply `ssl_err!` in the async body after joining. Do NOT move the `ssl_err!`/`trace!()`/`?` machinery into the closure.
  - `Rsa::generate` returns `Result<Rsa<Private>, ErrorStack>`. `Rsa<Private>` is `Send + 'static`; `ErrorStack` is `Send + 'static`. `num_bits: u32` is `Copy`. So `spawn_blocking(move || Rsa::generate(num_bits))` satisfies the `FnOnce() -> T + Send + 'static` / `T: Send + 'static` bounds.
  - `tokio::task::spawn_blocking` returns a `JoinHandle<T>` whose `.await` yields `Result<T, JoinError>`. A `JoinError` here only occurs if the blocking task panics. Map it to an existing/appropriate `CryptErr` variant (see Concrete Steps for the exact mapping decision) — do NOT `.unwrap()` it.
- Production callers of `gen_key_pair` (both pass `4096`, both already in async fns — they are unaffected because `gen_key_pair` stays `async fn`): `agent/src/provisioning/provision.rs:52`, `agent/src/provisioning/reprovision.rs:28`. No caller changes needed for M1.
- `sign` (line 117), `sign_rs256` (line 132), `sign_rs512` (line 140), `verify` (line 148): see Decision Log — expected to be left unchanged. `sign_rs512`'s only production caller is `agent/src/authn/issue.rs:84` inside `mint_jwt`.

### Category B targets and exact call-site enumeration

For each function: the definition, whether it is verified pure-sync, and the **complete** list of `.await` call sites to update. Line numbers are current as of writing; re-confirm with `grep` before editing (the plan gives greps).

**B1 — `agent/src/filesys/cached_file.rs::SingleThreadCachedFile::read()` (line 84).**
- Definition: `pub async fn read(&self) -> Arc<ContentT> { self.cache.clone() }` — pure `Arc::clone`, no `.await`. Verified pure-sync. Convert to `pub fn read(&self) -> Arc<ContentT>`.
- **CRITICAL — two different `read()` methods exist in this file.** Do NOT confuse them:
  - `SingleThreadCachedFile::read()` at line 84 returns a plain `Arc<ContentT>` (no `Result`). This is the one to convert.
  - `ConcurrentCachedFile::read()` at line 262 returns `Result<Arc<ContentT>, FileSysErr>` and is **genuinely async** — it sends a `Command::Read` over an mpsc channel and awaits a oneshot reply (`send_command(...).await`). It MUST stay `async` and MUST NOT be touched.
- Call sites of the SingleThread `read()` (remove `.await`). NOTE — `TokenFile` (`agent/src/authn/token_mngr.rs:25: pub type TokenFile = SingleThreadCachedFile<Token, token::Updates>`) is a `SingleThreadCachedFile`, so `token_file.read()` calls are ALSO SingleThread `read()` sites:
  - Production: `agent/src/filesys/cached_file.rs:165` — inside `Worker::run`, `self.file.read().await` (the `self.file` is a `SingleThreadCachedFile`). Change to `self.file.read()`. Note `Worker::run` stays `async` (it awaits the mpsc `recv()` and `self.file.write(...).await` / `patch(...).await`); only this one inner `.await` is removed.
  - Production: `agent/src/storage/device.rs:43` — `let token = token_file.read().await;` inside `resolve_device_id` (a `TokenFile`). Change to `token_file.read()`. `resolve_device_id` stays async (it awaits other I/O).
  - Production: `agent/src/authn/token_mngr.rs:63` — `self.token_file.read().await` inside `SingleThreadTokenManager::get_token(&self) -> Arc<Token>`. Change to `self.token_file.read()`. **Important:** this inner `get_token` (line 61) is an *inherent private* method, NOT the trait method. Leave `get_token` itself `async fn` — a sync `read()` call inside an async fn is legal, and this avoids cascading into `get_token`'s callers or anywhere near the trait. Do NOT convert `get_token`, and do NOT touch the trait `TokenManagerExt::get_token` (line 31, returns `Result<Arc<Token>, AuthnErr>`) — that is the genuinely-async actor-channel method and a trait signature (scope guard: leave async-trait impls alone).
  - Tests in `agent/tests/filesys/cached_file.rs`: the calls on a `SingleThreadTokenFile` value. These are the `cached_file.read().await.as_ref()` sites (no `?`/`.unwrap()` on `read` itself) at lines **59, 74, 90, 111, 126, 137, 172, 187, 206, 223, 231, 244, 256, 284, 306, 321, 337**. Change each `.read().await` → `.read()`.
  - Do NOT change the `.read().await.unwrap()` / `.read().await.unwrap_err()` sites at lines 384+ in that same test file — those are on `ConcurrentCachedFile` values (they return `Result`), i.e. the async variant that stays.
  - Do NOT change the Concurrent-variant production call sites: `agent/src/services/device/get.rs:7` (`device_stor.read().await?`), `agent/src/storage/mod.rs:184`, `agent/src/workers/mqtt.rs:324` — all are `storage::Device` = `ConcurrentCachedFile`, using `?`/match. Verify by the presence of `?` or `Result` handling.
- Disambiguation grep before editing:
  ```
  grep -rn "\.read()\.await" agent/src agent/tests
  ```
  SingleThread sites (convert): results used directly as an `Arc` — the `Worker::run` self.file site, the two `token_file.read()` sites (`storage/device.rs:43`, `token_mngr.rs:63`), and the test sites using `.as_ref()` without `.unwrap()`. Concurrent sites (leave): those on `storage::Device`/`ConcurrentCachedFile` values, which use `?`, `.unwrap()`, `.unwrap_err()`, or match on `Ok/Err` (e.g. `services/device/get.rs:7`, `storage/mod.rs:184`, `workers/mqtt.rs:324`, and the `.read().await.unwrap()` test sites at cached_file.rs:384+, plus `workers/{poller,mqtt}.rs` test `device_file.read().await.unwrap()` sites).

**B2 — `agent/src/storage/device.rs::assert_activated()` (line 14).**
- Definition: `pub async fn assert_activated(layout: &Layout) -> Result<(), StorageErr>` — body is two `if !auth_dir.private_key().exists()` / `public_key().exists()` checks returning an error. `exists()` is sync (`agent/src/filesys/path.rs:32: fn exists(&self) -> bool`). No `.await`. Verified pure-sync. Convert to `pub fn assert_activated(...)`.
- Re-exported at `agent/src/storage/mod.rs:19` (`pub use self::device::{assert_activated, ...}`) — the re-export line is unchanged (it re-exports the item, not its asyncness).
- Call sites (remove `.await`):
  - Production: `agent/src/provisioning/provision.rs:31` (`storage::assert_activated(layout).await.is_ok()`), `agent/src/app/await_activation.rs:24`, `agent/src/app/await_activation.rs:42`. Change each `assert_activated(layout).await` → `assert_activated(layout)`.
    - Note on `await_activation.rs`: line 42's call sits *inside* a `tokio::select!` branch, but it is not the `select!` itself — the enclosing `await_activation` fn legitimately stays `async` (it awaits `shutdown` and `sleep_fn`). Removing `.await` from the now-sync `assert_activated` call inside the branch is correct and does not violate the "do not touch fns using `tokio::select!`" guard (we are not converting `await_activation`, only removing a `.await` on a call that no longer needs it).
  - Tests: `agent/tests/storage/device.rs:25, 39, 53, 70` (`assert_activated(&layout).await...`). Change `.await` off each.
- Grep to confirm the full set:
  ```
  grep -rn "assert_activated(" agent/src agent/tests | grep -v "fn assert_activated" | grep -v "pub use"
  ```

**B3 — `agent/src/app/upgrade.rs::validate_layout()` (line 100).**
- Definition: `pub async fn validate_layout(layout: &Layout) -> Result<(), UpgradeErr>` — body calls `auth_dir.private_key().assert_exists()?` and `public_key().assert_exists()?`. `assert_exists()` is sync (`agent/src/filesys/path.rs:36: fn assert_exists(&self) -> Result<(), FileSysErr>`). No `.await`. Verified pure-sync. Convert to `pub fn validate_layout(...)`.
- Call sites (remove `.await`):
  - Production: `agent/src/app/upgrade.rs:35` — `validate_layout(layout).await?` inside `reconcile` (which stays async). Change to `validate_layout(layout)?`.
  - Tests: none found (`grep -rn validate_layout agent/tests` → no matches). Re-confirm before finishing.
- Grep:
  ```
  grep -rn "validate_layout" agent/src agent/tests | grep -v "fn validate_layout"
  ```

**B4 — `agent/src/mqtt/client.rs::Client::new()` (line 44).**
- Definition: `pub async fn new(options: &Options) -> (Self, EventLoop)` — body builds `MqttOptions`, calls setters, and `AsyncClient::new(mqtt_options, options.capacity)` (rumqttc's `AsyncClient::new` is **not** async — it returns `(AsyncClient, EventLoop)` synchronously). No `.await`. Verified pure-sync. Convert to `pub fn new(options: &Options) -> (Self, EventLoop)`.
- Note: this is an inherent method on `Client`, NOT a trait method. The trait `ClientI` (defined in the same file, lines 23-35) declares `publish`/`subscribe`/`unsubscribe`/`disconnect` — it does **not** declare `new`. So converting `new` does not touch any async-trait signature (scope guard respected).
- Call sites (remove `.await`), all of form `let (…, …) = …Client::new(&…).await;`:
  - Production: `agent/src/workers/mqtt.rs:178`.
  - Tests: `agent/tests/mqtt/errors.rs:326, 344`; `agent/tests/mqtt/client.rs:31, 70, 92, 112, 130, 143, 153, 169`; `agent/tests/workers/mqtt.rs:443, 502, 564`.
- Grep to confirm the full set:
  ```
  grep -rn "Client::new(" agent/src/workers/mqtt.rs agent/src/mqtt agent/tests | grep -i mqtt
  ```
  (Ensure you are matching the mqtt `Client::new`, not `AsyncClient::new` or an HTTP client.)

**B5 — `agent/src/filesys/dir.rs::create_temp_dir()` (line 67).** **LEAVE UNCHANGED.** See Decision Log — genuinely pure-sync but ~250+ test call sites make conversion a disproportionate, risky ripple. Do not convert. (Listed here so the implementer does not "helpfully" convert it.)

### Scope guards (MUST hold)

- Do NOT change functions that are `async` only to satisfy an async **trait** signature — impls of `SyncerExt`, `SingleThreadCache`, and anything in `cache/*` and `sync/syncer.rs`. (None of B1-B4 are such impls: B1/B2/B3 are free/inherent fns, B4 is an inherent `Client::new` not on `ClientI`.)
- Do NOT change axum route handlers in `agent/src/server/handlers.rs` — axum requires `async` handlers.
- Do NOT touch functions built around `tokio::select!` (`agent/src/workers/{mqtt,poller,uploads}.rs`, `agent/src/main.rs` shutdown). Removing a `.await` from a *call inside* such a fn (as in B2's `await_activation`) is allowed only because the callee became sync; the enclosing async fn is not converted.
- Do NOT edit generated code under `libs/backend-api/` or `libs/device-api/`.

### Validation tooling

- `./scripts/test.sh` → `RUST_LOG=off cargo test --package miru-agent --features test`. `--features test` mandatory.
- `./scripts/preflight.sh` → runs four checks in parallel (`scripts/lint.sh`, `scripts/covgate.sh`, `tools/lint/scripts/lint.sh`, `tools/lint/scripts/covgate.sh`) and prints `Preflight clean` on success (exit 0) or `Preflight FAILED (...)` on any non-zero (exit 1).
- `./scripts/lint.sh` → import linter + `cargo fmt` + machete/diet + audit + clippy `-D warnings`. Run `./scripts/update-deps.sh` first.
- `cargo build --package miru-agent --features test` / `cargo check` for a fast compile signal.


## Plan of Work

Three milestones, one commit each. Order matters: M1 (priority) first so it is independently reviewable/revertable.

**M1 — Offload `gen_key_pair` keygen to `spawn_blocking`.** Wrap only the `Rsa::generate(num_bits)` call. Keep the fn `async`, keep `ssl_err!`/error mapping in the async body, keep all file writes unchanged. No call-site changes. Commit.

**M2 — Convert Category B pure-sync fns and update call sites.** Convert B1-B4 (`SingleThreadCachedFile::read`, `assert_activated`, `validate_layout`, `mqtt::Client::new`) from `async fn` to `fn` and remove `.await` at every enumerated call site (production + tests). Leave B5 (`create_temp_dir`) and the four `sign`/`verify` fns untouched. Commit.

**M3 — Validate.** `cargo build --features test`, `./scripts/test.sh`, then `./scripts/update-deps.sh` + `./scripts/lint.sh`, then `./scripts/preflight.sh` until clean. If fmt/covgate force trivial fixups, commit them; otherwise M3 has no code commit.


## Concrete Steps

All commands list their working directory explicitly. cwd is always `/home/ben/miru/workbench4/repos/agent` unless noted.

### M1 — `spawn_blocking` around RSA keygen

Edit `agent/src/crypt/rsa.rs`. Replace line 47's single statement:

```rust
    // Generate the RSA key pair
    let rsa = ssl_err!(GenerateRSAKeyPairErr, Rsa::generate(num_bits))?;
```

with a `spawn_blocking` that runs the CPU-bound generate off the async runtime, then applies the existing error mapping in the async body:

```rust
    // Generate the RSA key pair on a blocking thread so the 4096-bit keygen
    // (hundreds of ms of pure CPU) does not pin an async worker thread and
    // stall concurrent tasks (MQTT loop, poller, local socket server).
    let rsa = tokio::task::spawn_blocking(move || Rsa::generate(num_bits))
        .await
        .map_err(|e| /* JoinError -> CryptErr; see mapping note */)?;
    let rsa = ssl_err!(GenerateRSAKeyPairErr, rsa)?;
```

Notes for the implementer:
- The closure is `move` and captures only `num_bits` (`Copy`), so it is `Send + 'static`. `Rsa<Private>` and `ErrorStack` are `Send + 'static`, satisfying `spawn_blocking`'s return bound.
- The inner `let rsa = ssl_err!(GenerateRSAKeyPairErr, rsa)?;` re-applies the existing openssl-error → `CryptErr::GenerateRSAKeyPairErr` mapping (with `trace!()`), exactly as before, so error behavior for a keygen failure is unchanged.
- **JoinError mapping:** a `JoinError` only happens if the blocking task panics (it cannot be cancelled here). Decide the mapping by inspecting `agent/src/crypt/errors.rs`:
  - Preferred: if there is a general/opaque `CryptErr` variant suitable for an internal task-join failure, map to it with `trace!()`.
  - If no suitable variant exists, the simplest behavior-preserving choice is to treat a panic as unrecoverable and let it propagate by `.expect("rsa keygen task panicked")` on the `JoinError` (a panic in keygen would have propagated before this change too, since the code ran inline). Prefer a mapped `CryptErr` if one fits; only fall back to `.expect` if adding a new error variant would be out of proportion. Record the choice in the Decision Log.
  - Do NOT introduce a new `CryptErr` variant unless clearly warranted; if you do, follow the `impl_error!` / `thiserror` conventions and add it to `agent/src/crypt/errors.rs`.
- `tokio::task::spawn_blocking` is referenced by full path, so no new `use` is required. If you prefer a `use tokio::task;` import, place it in the external-crates group per the import-ordering convention.

Confirm no other keygen call needs wrapping and that the four sign/verify fns are being left (per Decision Log):

```
grep -n "spawn_blocking" agent/src/crypt/rsa.rs   # expect: exactly the new call
```

Build and run the crypt tests:

```
cargo build --package miru-agent --features test
./scripts/test.sh 2>&1 | tail -40    # all pass; crypt/rsa tests exercise gen_key_pair
```

Commit M1:

```
git add agent/src/crypt/rsa.rs agent/src/crypt/errors.rs
git commit -m "perf(crypt): offload RSA keygen to spawn_blocking" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

### M2 — Convert Category B fns to sync + update call sites

Do each function fully (def + all call sites) before moving to the next, and re-grep to confirm you caught every site.

**B1 `SingleThreadCachedFile::read`:**
1. `agent/src/filesys/cached_file.rs:84`: `pub async fn read(&self) -> Arc<ContentT>` → `pub fn read(&self) -> Arc<ContentT>`.
2. `agent/src/filesys/cached_file.rs:165`: `self.file.read().await` → `self.file.read()`.
3. `agent/src/storage/device.rs:43`: `token_file.read().await` → `token_file.read()`.
4. `agent/src/authn/token_mngr.rs:63`: `self.token_file.read().await` → `self.token_file.read()`. Leave the enclosing inherent `get_token` async (do not convert it); do not touch the trait `TokenManagerExt::get_token`.
5. `agent/tests/filesys/cached_file.rs`: at each SingleThread site (lines 59, 74, 90, 111, 126, 137, 172, 187, 206, 223, 231, 244, 256, 284, 306, 321, 337) change `.read().await` → `.read()`. Leave the `ConcurrentCachedFile` sites (`.read().await.unwrap()` / `.unwrap_err()` at 384+) untouched.
6. Confirm: `grep -rn "\.read()\.await" agent/src agent/tests` — remaining hits must all be Concurrent-variant (the line-262 method: results handled with `?`/`.unwrap()`/`.unwrap_err()`/match), never the SingleThread `Arc`/`.as_ref()` form.

**B2 `assert_activated`:**
1. `agent/src/storage/device.rs:14`: drop `async`.
2. Remove `.await` at `agent/src/provisioning/provision.rs:31`, `agent/src/app/await_activation.rs:24`, `agent/src/app/await_activation.rs:42`, and tests `agent/tests/storage/device.rs:25, 39, 53, 70`.
3. Confirm: `grep -rn "assert_activated(" agent/src agent/tests | grep "\.await"` → no matches (except none).

**B3 `validate_layout`:**
1. `agent/src/app/upgrade.rs:100`: drop `async`.
2. `agent/src/app/upgrade.rs:35`: `validate_layout(layout).await?` → `validate_layout(layout)?`.
3. Confirm: `grep -rn "validate_layout" agent/src agent/tests | grep "\.await"` → no matches.

**B4 `mqtt::Client::new`:**
1. `agent/src/mqtt/client.rs:44`: `pub async fn new(...)` → `pub fn new(...)`.
2. Remove `.await` at `agent/src/workers/mqtt.rs:178` and tests `agent/tests/mqtt/errors.rs:326, 344`, `agent/tests/mqtt/client.rs:31, 70, 92, 112, 130, 143, 153, 169`, `agent/tests/workers/mqtt.rs:443, 502, 564`.
3. Confirm: `grep -rn "Client::new(" agent/src agent/tests | grep -i mqtt | grep "\.await"` → no matches.

**Do NOT touch:** `Dir::create_temp_dir` (B5) and `crypt::rsa::{sign,sign_rs256,sign_rs512,verify}`.

Compile after all four conversions (the compiler will flag any missed `.await` on a now-sync fn as a type error — `Arc`/`Result`/tuple has no `.await`; use that as a completeness check):

```
cargo build --package miru-agent --features test 2>&1 | tail -40   # expect: Finished, no errors
```

If the build complains about a stray `.await`, that is a missed call site — fix it. If clippy later flags an `unused` `async`/needless-return, address in M3.

Commit M2:

```
git add agent/src/filesys/cached_file.rs agent/src/authn/token_mngr.rs agent/src/storage/device.rs agent/src/app/upgrade.rs agent/src/mqtt/client.rs agent/src/provisioning/provision.rs agent/src/app/await_activation.rs agent/src/workers/mqtt.rs agent/tests/
git commit -m "refactor(async): drop needless async from pure-sync fns" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

### M3 — Validate

```
cargo build --package miru-agent --features test    # Finished, no errors
./scripts/test.sh                                   # all tests pass
./scripts/update-deps.sh                            # refresh Cargo.lock
./scripts/lint.sh                                   # import linter + fmt + clippy -D warnings clean
./scripts/preflight.sh                              # prints "Preflight clean" (exit 0)
```

If `cargo fmt` (via lint) or covgate rewrites files, review and commit:

```
git add -A
git commit -m "chore(async): apply fmt/coverage fixups" \
  -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

Otherwise M3 produces no commit.


## TEST

These are behavior-preserving refactors, so **the primary signal is that the existing suite passes unchanged.** No behavior changes → no new assertions of new behavior are required.

- **Run:** `./scripts/test.sh` (`RUST_LOG=off cargo test --package miru-agent --features test`). Must be green. Key files that exercise the touched code:
  - `agent/tests/crypt/rsa.rs` — exercises `gen_key_pair` (2048 and 4096 bit), `sign_rs256`, `sign_rs512`, `verify`. These must still pass after M1's `spawn_blocking` wrap; they are the main guard that keygen output and error paths are unchanged.
  - `agent/tests/filesys/cached_file.rs` — exercises both `SingleThreadCachedFile::read` (converted) and `ConcurrentCachedFile::read` (unchanged).
  - `agent/tests/storage/device.rs` — `assert_activated` success + not-activated error cases.
  - `agent/tests/app/upgrade.rs` — exercises `reconcile`/`validate_layout` path.
  - `agent/tests/mqtt/{client,errors}.rs`, `agent/tests/workers/mqtt.rs` — `Client::new`.
- **No new tests required.** Do not add tests asserting `spawn_blocking` is used or asserting a function is sync — those are implementation details, not behavior, and would be brittle. If, and only if, coverage gates (`scripts/covgate.sh`) drop below a module's `.covgate` threshold because a converted function changed which lines count as covered, restore coverage minimally (e.g. keep the existing test that already covers the path); do not invent behavioral tests. Record any such adjustment in Surprises.
- **Compile-time completeness check.** After M2, a clean `cargo build --features test` is itself a strong test: any call site where a `.await` was left on a now-sync function fails to compile (a plain `Arc`/`Result`/tuple has no `IntoFuture`). Treat a green build as confirmation that the call-site enumeration was complete.
- **Serial tests.** Some mqtt/socket tests are `#[serial]`; `./scripts/test.sh` handles this. Do not add or remove `#[serial]` — resource usage is unchanged.


## VALIDATION

Acceptance = verifiable behavior and a clean gate, not just compiling code.

1. **Keygen offloaded:** `grep -n "spawn_blocking" agent/src/crypt/rsa.rs` shows exactly the one new call wrapping `Rsa::generate`; the surrounding `ssl_err!(GenerateRSAKeyPairErr, ...)` mapping and the file writes are intact and still `.await`ed. The four `sign`/`verify` fns and `gen_key_pair`'s public signature (still `async fn ... -> Result<(), CryptErr>`) are unchanged.
2. **Category B converted, no stragglers:** each of `SingleThreadCachedFile::read`, `assert_activated`, `validate_layout`, `mqtt::Client::new` is now a plain `fn`, and `grep`-ing each name across `agent/src`+`agent/tests` shows no remaining `.await` on those calls. `create_temp_dir` and the sign/verify fns are demonstrably unchanged (`git diff` shows no edits to them).
3. **Scope respected:** `git diff --stat` touches only `agent/src/**` and `agent/tests/**` (plus `Cargo.lock` from `update-deps`). No changes under `libs/backend-api/` or `libs/device-api/`. No axum handler, no `SyncerExt`/`SingleThreadCache`/`sync/syncer.rs` impl, and no `tokio::select!` fn was converted.
4. **Build:** `cargo build --package miru-agent --features test` finishes with no errors (proves every call site was updated).
5. **Tests:** `./scripts/test.sh` reports all tests passing — the behavior-unchanged guarantee. The `crypt/rsa` and `filesys/cached_file` suites in particular pass.
6. **Preflight clean — REQUIRED before publishing.** `./scripts/preflight.sh` MUST print `Preflight clean` and exit 0 (all four sub-checks: lint, covgate/tests, tools lint, tools tests). Changes MUST NOT be published/opened as a PR until preflight reports clean. `./scripts/lint.sh` (run after `./scripts/update-deps.sh`) must also be clean as its own gate — clippy runs with `-D warnings`, so any needless-`async`/unused-import left behind will fail the gate and must be fixed.


## Idempotence and Recovery

- All edits are small, local, and reversible via `git checkout -- <file>` per file, or `git reset --soft HEAD~1` to amend a milestone commit. Milestones are independent: M1 (crypt) can ship or revert without M2, and vice versa.
- If M2's build fails on a stray `.await`, the compiler error points at the exact file:line — remove that `.await` and rebuild. Re-run the per-function greps in Concrete Steps to find any site the enumeration missed.
- If the `SingleThread` vs `Concurrent` `read()` distinction gets muddled and a Concurrent call site loses its `.await` by mistake, the build breaks immediately (Concurrent `read` returns a `Future<Output = Result<...>>`); restore the `.await` there.
- If M1's `spawn_blocking` wrap causes any `crypt/rsa` test to fail, the most likely cause is the JoinError mapping or a moved `ssl_err!` — re-check that `ssl_err!(GenerateRSAKeyPairErr, ...)` still runs in the async body (not inside the closure) and that only `Rsa::generate(num_bits)` moved into the closure.
- If covgate regresses, prefer restoring/keeping the existing test that covered the line rather than adding new behavioral tests; document in Surprises.
