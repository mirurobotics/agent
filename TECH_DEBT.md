# Tech Debt — Agent

Items are ordered by ID. Gaps in IDs are expected — never renumber.

| ID | Title | Category | Scope |
|----|-------|----------|-------|
| TD-001 | Import comment labels inconsistent with AGENTS.md convention | `inconsistency` | `M` |
| TD-005 | Deployment and device model enum conversion boilerplate | `complexity` | `S` |
| TD-006 | Cache actor worker dispatch boilerplate | `complexity` | `S` |
| TD-007 | ShutdownManager repetitive with_*_handle methods | `complexity` | `XS` |

---

### TD-001: Import comment labels inconsistent with AGENTS.md convention `inconsistency` `M`

**Location:** `agent/src/**/*.rs` (~50+ files with import group comments)

**Current state:** AGENTS.md specifies import group comments as `// standard library`, `// internal`, `// external`. The vast majority of files use `// standard crates`, `// internal crates`, `// external crates` instead. Only `main.rs` and a handful of files (`http/client.rs`, `mqtt/client.rs`, `authn/token_mngr.rs`, `filesys/file.rs`) use the documented labels. Additionally, some files place `backend_api`/`device_api` imports under `// internal crates` while others place them under `// external crates`, creating ambiguity about where generated-code imports belong.

**Desired state:** Either update AGENTS.md to match the `// xxx crates` labels the codebase actually uses, or normalize all files to match the current AGENTS.md convention. Clarify whether generated-lib imports (`backend_api`, `device_api`) belong in the internal or external group.

### TD-005: Deployment and device model enum conversion boilerplate `complexity` `S`

**Location:** `agent/src/models/deployment.rs` (lines 15-414), `agent/src/models/device.rs` (lines 11-53)

**Current state:** Five enums (`DplTarget`, `DplActivity`, `DplErrStatus`, `DplStatus`, `DeviceStatus`) each repeat the same pattern:
1. Custom `Deserialize` impl — match string variants, warn + default on unknown (identical structure, ~20 lines each)
2. `variants()` method — returns a Vec of all variants (identical structure)
3. `From<&Self>` for `agent_server::*` — match each variant to its generated-API counterpart
4. `From<&Self>` for `backend_client::*` — same but for backend client types (deployment enums only)
5. `From<&backend_client::*>` for Self — reverse conversion (deployment enums only)

This produces ~350 lines of near-identical match-statement boilerplate across the deployment enums alone. Each new variant added to any enum requires updating 3-5 match blocks.

**Desired state:** Introduce a declarative macro (e.g., `impl_enum_conversion!`) that generates the Deserialize impl, variants(), and From impls from a single enum definition with variant mappings. This would reduce ~350 lines to ~40 lines and make adding new variants a single-line change.

### TD-006: Cache actor worker dispatch boilerplate `complexity` `S`

**Location:** `agent/src/cache/concurrent.rs` (lines 144-305)

**Current state:** The `Worker::run()` method contains a `match cmd` block with 21 arms. Of these, 20 follow an identical pattern:
```rust
WorkerCommand::Xxx { ..., respond_to } => {
    let result = self.cache.xxx(...).await;
    if respond_to.send(result).is_err() {
        error!("Actor failed to ...");
    }
}
```
The only variation is the method name, parameters, and error message string. This is ~160 lines of code where a macro or generic dispatch helper could reduce it significantly.

**Desired state:** Extract the response-send-or-log pattern into a helper (macro or function) so each arm reduces to one or two lines. The `Shutdown` arm remains special-cased since it breaks the loop.

### TD-007: ShutdownManager repetitive with_*_handle methods `complexity` `XS`

**Location:** `agent/src/app/run.rs` (lines 359-421)

**Current state:** `ShutdownManager` has four nearly identical methods: `with_token_refresh_worker_handle()`, `with_poller_worker_handle()`, `with_mqtt_worker_handle()`, `with_socket_server_handle()`. Each checks if the corresponding `Option` field is already `Some`, returns a `ShutdownMngrDuplicateArgErr` if so, and sets it otherwise. The only differences are the field name and error arg_name string.

**Desired state:** Replace the four methods with a single generic `register_handle()` method that takes a field selector and name string, or use a `Vec<(&str, JoinHandle<()>)>` instead of named Option fields. This would reduce ~60 lines to ~15 and eliminate the possibility of copy-paste errors.

**Notes:** `with_app_state()` and `with_socket_server_handle()` have different parameter types (`Arc<AppState> + Pin<Box<...>>` and `JoinHandle<Result<(), ServerErr>>` respectively), so they may need to remain as separate methods. The three worker handle methods (`token_refresh`, `poller`, `mqtt`) are the strongest candidates for consolidation since they all take `JoinHandle<()>`.
