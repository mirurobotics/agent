# Add config instance endpoints to the device API

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Add config instance service layer, handler functions, route registrations, response types, error variants, and tests. |

This plan lives in `agent/plans/backlog/` because all code changes happen inside the agent repo.

## Purpose / Big Picture

On-device applications currently access config data only by reading files from disk. After this change, four new HTTP endpoints on the agent's local Unix socket API let on-device apps fetch config instance metadata, raw file content, individual parameters by key path, and flat lists of all parameters -- all via standard HTTP. This eliminates the need for apps to know the agent's file layout or parse config files themselves.

User-visible behavior after this change:

- `GET /v0.2/config_instances/{id}` returns JSON metadata for a config instance. Adding `?expand=content` includes the raw file content inline.
- `GET /v0.2/config_instances/{id}/content` returns the raw config file with the correct Content-Type (`application/json` or `application/yaml`).
- `GET /v0.2/config_instances/{id}/parameters/{key}` returns a single parameter by dot-separated key path as a JSON object.
- `GET /v0.2/config_instances/{id}/parameters` returns all leaf parameters flattened into dot-separated key paths. Optional `?prefix=` filters by key prefix.

## Progress

- [ ] M1 -- Dependencies: add `serde_yml` to workspace and agent Cargo.toml.
- [ ] M2 -- Response types and errors: create response structs and config-instance error variants.
- [ ] M3 -- Service layer: create `services/config_instance/` with get, content, and parameter logic.
- [ ] M4 -- Handlers and routes: add handler functions and route registrations.
- [ ] M5 -- Tests: add service and handler tests.
- [ ] M6 -- Preflight: run `./scripts/preflight.sh` and fix any issues.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Define response types directly in agent source code rather than modifying `libs/device-api/`.
  Rationale: `libs/device-api/` is auto-generated from the OpenAPI spec in the `mirurobotics/openapi` repo. Per AGENTS.md: "Do not edit by hand." The existing `handle()` function accepts any `T: Serialize`, so custom response types work cleanly without touching generated code.
  Date/Author: 2026-05-24 / plan author.

- Decision: Use `serde_yml` (not `serde_yaml`) for YAML parsing.
  Rationale: `serde_yaml` is deprecated/unmaintained. `serde_yml` is the maintained successor with the same API surface. Neither crate is currently in the dependency tree.
  Date/Author: 2026-05-24 / plan author.

- Decision: No backend fallback for config instance endpoints. If a config instance is not in the local cache, return 404.
  Rationale: Config instance content is synced to the agent during deployment sync. Unlike deployments/releases/git_commits, there is no backend endpoint for fetching individual config instance metadata on demand. The content exists only after a successful deployment sync.
  Date/Author: 2026-05-24 / plan author.

- Decision: Create a new `agent/src/server/responses/` module for config instance response types rather than adding them to `agent/src/server/response.rs`.
  Rationale: `response.rs` contains only `From` impls that convert agent models to generated `device_server` types. Config instance response types are standalone `Serialize` structs with no generated counterpart. A separate module keeps the concerns distinct and avoids bloating the existing file.
  Date/Author: 2026-05-24 / plan author.

- Decision: The raw content endpoint (`GET .../content`) bypasses the `handle()` wrapper and returns a custom Axum response directly.
  Rationale: `handle()` always wraps the response in `Json(json!(...))`. The content endpoint must return raw text with `Content-Type: application/json` or `application/yaml` and a `Content-Disposition` header. A custom `impl IntoResponse` is the idiomatic Axum approach.
  Date/Author: 2026-05-24 / plan author.

- Decision: Add config-instance errors at the service layer (`CfgInstServiceErr`) rather than at the server layer.
  Rationale: The existing pattern routes domain errors through `ServiceErr` which `ServerErr` already wraps via `From<ServiceErr>`. Adding config-instance-specific errors at the service layer keeps the error hierarchy consistent with deployment/release/git_commit.
  Date/Author: 2026-05-24 / plan author.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

### Repository layout

The agent repo is at `/home/ben/miru/workbench2/repos/agent`. The branch `feat/device-api-config-instances` is checked out with no changes from main.

### Storage (already exists -- no changes needed)

Config instance data is split into two caches for performance:

- **Metadata cache** (`CfgInsts`): `cache::FileCache<CfgInstID, ConfigInstance>` -- all metadata entries in a single JSON file.
- **Content cache** (`CfgInstContent`): `cache::DirCache<CfgInstID, String>` -- each content entry in its own file under a directory.
- **Composite** (`CfgInstStor`): holds `Arc<CfgInsts>` and `Arc<CfgInstContent>`.
- Accessed via `state.storage.cfg_insts.meta` and `state.storage.cfg_insts.content`.

### ConfigInstance model (agent/src/models/config_instance.rs)

```rust
pub struct ConfigInstance {
    pub id: String,
    pub config_type_name: String,
    pub filepath: String,
    pub created_at: DateTime<Utc>,
    pub config_schema_id: String,
    pub config_type_id: String,
}
```

### Handler pattern

All existing handlers use the `handle()` utility which wraps service calls and returns `(StatusCode, Json<Value>)`.

### Error pattern

Custom error types derive `thiserror::Error` and implement `crate::errors::Error`. For 404 errors, override `code()` to return `Code::ResourceNotFound` and `http_status()` to return `HTTPCode::NOT_FOUND`.

## Plan of Work

### M1 -- Dependencies

Add `serde_yml` to workspace `Cargo.toml` and `agent/Cargo.toml`.

### M2 -- Response types and errors

**New files:**
- `agent/src/server/responses/mod.rs` -- module declarations
- `agent/src/server/responses/config_instance.rs` -- ConfigInstanceResponse, ContentField, ParameterResponse, ParameterListResponse

**New file:**
- `agent/src/services/config_instance/errors.rs` -- ConfigInstanceNotFoundErr, ContentNotFoundErr, ParameterNotFoundErr, ContentParseErr, CfgInstServiceErr enum

**Edit:**
- `agent/src/server/mod.rs` -- add `pub mod responses;`
- `agent/src/services/errors.rs` -- add CfgInstServiceErr variant to ServiceErr

### M3 -- Service layer

**New files:**
- `agent/src/services/config_instance/mod.rs`
- `agent/src/services/config_instance/get.rs` -- get(), get_with_content()
- `agent/src/services/config_instance/content.rs` -- get_raw_content(), infer_format(), content_type_for_format()
- `agent/src/services/config_instance/parameters.rs` -- get_parameter(), list_parameters()

**Edit:**
- `agent/src/services/mod.rs` -- add `pub mod config_instance;`

### M4 -- Handlers and routes

**Edit:**
- `agent/src/server/handlers.rs` -- add get_config_instance, get_config_instance_content, get_config_instance_parameter, list_config_instance_parameters
- `agent/src/server/serve.rs` -- register 4 new routes

### M5 -- Tests

**New files:**
- `agent/tests/services/config_instance/mod.rs`
- `agent/tests/services/config_instance/get.rs`
- `agent/tests/services/config_instance/content.rs`
- `agent/tests/services/config_instance/parameters.rs`

**Edit:**
- `agent/tests/services/mod.rs`
- `agent/tests/server/handlers.rs`

### M6 -- Preflight

Run `./scripts/test.sh` and `./scripts/lint.sh`. Fix any issues.

## Validation and Acceptance

- `./scripts/test.sh` -- all tests pass
- `./scripts/lint.sh` -- all lints pass
- Preflight must report clean before changes are published
