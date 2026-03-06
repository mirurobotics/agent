# Extract shared script library from duplicated agent and lint-tool scripts

This plan covers extracting the duplicated logic across `scripts/` and `tools/lint/scripts/` into a shared library at `scripts/lib/`, then converting the existing per-crate scripts into thin wrappers.

## Scope

| Repository | Access | Why touched |
|---|---|---|
| `agent` | read-write | All script changes live here |

The plan file lives in `agent/.ai/exec-plans/active/` because all changes are within the agent repository.

## Purpose / Big Picture

Today every script exists twice — once for the agent crate and once for the lint tool — with ~90% identical logic. The only differences are working directory, cargo package/feature flags, and path resolution style. When a bug is fixed or a feature is added to one copy, the other goes stale.

After this work, a single set of core scripts in `scripts/lib/` implements each operation (test, coverage, covgate, update-covgates, lint, dep-updates). The per-crate entry scripts (`scripts/*.sh` and `tools/lint/scripts/*.sh`) become 3–8 line wrappers that set environment variables and call the shared implementation. Adding a new crate to the repo means writing a handful of wrapper lines, not copying 100-line scripts.

The user can verify success by running every existing entry script and confirming identical behavior.

## Progress

- [ ] Milestone 1: Create `scripts/lib/` with shared core scripts
- [ ] Gate 1: User review + commit
- [ ] Milestone 2: Convert `scripts/*.sh` (agent wrappers) to call shared lib
- [ ] Gate 2: User review + commit
- [ ] Milestone 3: Convert `tools/lint/scripts/*.sh` (lint-tool wrappers) to call shared lib
- [ ] Gate 3: User review + commit
- [ ] Final: Run all entry scripts, verify behavior unchanged
- [ ] Gate 4: User review + commit

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- Decision: Use environment variables (not positional arguments) as the configuration interface between wrappers and core scripts.
  Rationale: Env vars are self-documenting, order-independent, and easy to default. Positional args become ambiguous at 4+ parameters. The variables are: `CRATE_DIR`, `CARGO_PKG`, `CARGO_FEATURES`, `CARGO_TEST_ARGS`, `SRC_DIR`, `RUST_LOG_OVERRIDE`.
  Date/Author: 2026-03-05

- Decision: Place shared scripts in `scripts/lib/` rather than a top-level `lib/` or `scripts/common/`.
  Rationale: `scripts/lib/` keeps the library co-located with the scripts that use it and is a well-understood Unix convention. `lib/` at repo root is ambiguous (Rust lib? shared objects?).
  Date/Author: 2026-03-05

- Decision: Core scripts `cd` into `CRATE_DIR` and run bare `cargo` commands, rather than using `--manifest-path` everywhere.
  Rationale: Simpler cargo invocations, consistent working directory, and matches how `cargo llvm-cov --json` reports file paths (relative to crate root). The agent wrappers set `CRATE_DIR` to repo root (since the workspace root is the crate root) and the lint wrappers set it to `tools/lint/`.
  Date/Author: 2026-03-05

- Decision: The `lint.sh` core script will NOT be shared — agent and lint-tool lint scripts differ substantially (nightly toolchain, udeps, diet, import lint paths, clippy flags).
  Rationale: lint.sh has the most divergence. Trying to parameterize all the differences would produce a more complex script than two separate ones. The other 5 scripts (test, coverage, covgate, update-covgates, dep-updates) are nearly identical and benefit from sharing.
  Date/Author: 2026-03-05

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

### Current file layout

    scripts/
      coverage.sh          # 20 lines — HTML coverage report for agent
      covgate.sh           # 100 lines — per-module coverage gate for agent
      dep-updates.sh       # 10 lines — cargo update for agent
      lint.sh              # 124 lines — full lint suite for agent
      test.sh              # 10 lines — run agent tests
      update-covgates.sh   # 89 lines — ratchet coverage thresholds for agent
    tools/lint/scripts/
      coverage.sh          # 20 lines — HTML coverage report for lint tool
      covgate.sh           # 105 lines — per-module coverage gate for lint tool
      dep-updates.sh       # 12 lines — cargo update for lint tool
      lint.sh              # 92 lines — full lint suite for lint tool
      test.sh              # 11 lines — run lint tool tests
      update-covgates.sh   # 94 lines — ratchet coverage thresholds for lint tool

### Parameterizable differences between the two copies

| Variable | Agent value | Lint-tool value | Purpose |
|---|---|---|---|
| `CRATE_DIR` | `$repo_root` | `$repo_root/tools/lint` | Working directory for cargo commands |
| `CARGO_PKG` | `--package miru-agent` | *(empty — single crate)* | Cargo package selector |
| `CARGO_FEATURES` | `--features test` | *(empty)* | Feature flags for test/coverage runs |
| `CARGO_TEST_ARGS` | `-- --test-threads=1` | *(empty)* | Extra args passed after `--` to test binary |
| `RUST_LOG_OVERRIDE` | `off` | *(empty)* | RUST_LOG value during test runs |
| `SRC_DIR` | `agent/src` (relative to repo root) | `src` (relative to crate dir) | Where to find `.covgate` files |

### Path resolution in covgate/update-covgates

The agent version uses absolute paths (`$git_repo_root_dir/$module_path/`) for jq matching because `cargo llvm-cov` reports absolute filenames when run from the workspace root. The lint-tool version uses relative paths (`$module_path/`) because it runs from the crate directory. The core script must handle both — the simplest approach is to always resolve `module_dir_abs` as an absolute path by prepending `$CRATE_DIR/` when the path isn't already absolute.

The lint-tool version also has a `module_display` fallback for root-level `.covgate` files (displays "src" instead of the raw path). The agent version lacks this. The shared script should include this fallback since it's strictly better.

## Plan of Work

### Milestone 1: Create `scripts/lib/` with 5 shared core scripts

Create these files:

**`scripts/lib/test.sh`** — Core test runner (~15 lines)
- Reads: `CRATE_DIR`, `CARGO_PKG`, `CARGO_FEATURES`, `CARGO_TEST_ARGS`, `RUST_LOG_OVERRIDE`
- `cd "$CRATE_DIR"`
- Runs: `${RUST_LOG_OVERRIDE:+RUST_LOG=$RUST_LOG_OVERRIDE} cargo test $CARGO_PKG $CARGO_FEATURES $CARGO_TEST_ARGS`

**`scripts/lib/coverage.sh`** — HTML coverage report (~20 lines)
- Reads: `CRATE_DIR`, `CARGO_PKG`, `CARGO_FEATURES`, `CARGO_TEST_ARGS`, `RUST_LOG_OVERRIDE`
- `cd "$CRATE_DIR"`
- Installs cargo-llvm-cov if missing
- Runs: `cargo llvm-cov --html --output-dir target/coverage $CARGO_PKG $CARGO_FEATURES $CARGO_TEST_ARGS`

**`scripts/lib/covgate.sh`** — Coverage gate checker (~80 lines)
- Reads: `CRATE_DIR`, `CARGO_PKG`, `CARGO_FEATURES`, `CARGO_TEST_ARGS`, `RUST_LOG_OVERRIDE`, `SRC_DIR`
- `cd "$CRATE_DIR"`
- Requires jq, installs cargo-llvm-cov if missing
- Runs coverage JSON, discovers `.covgate` files under `SRC_DIR`, checks per-module thresholds
- Uses absolute path resolution: `module_dir_abs="$(cd "$CRATE_DIR" && pwd)/$module_path/"`
- Includes lint-tool's root-level `.covgate` display fallback

**`scripts/lib/update-covgates.sh`** — Coverage threshold ratchet (~75 lines)
- Same parameters as covgate.sh
- Runs coverage JSON, discovers `.covgate` files, ratchets thresholds up

**`scripts/lib/dep-updates.sh`** — Dependency updater (~8 lines)
- Reads: `CRATE_DIR`
- `cd "$CRATE_DIR"` then `cargo update --verbose`

### Milestone 2: Convert agent wrappers in `scripts/`

Replace the body of each script with env var setup + exec/source of the lib script. Example for `scripts/test.sh`:

    #!/bin/sh
    set -e
    REPO_ROOT=$(git rev-parse --show-toplevel)
    export CRATE_DIR="$REPO_ROOT"
    export CARGO_PKG="--package miru-agent"
    export CARGO_FEATURES="--features test"
    export CARGO_TEST_ARGS="-- --test-threads=1"
    export RUST_LOG_OVERRIDE="off"
    exec "$REPO_ROOT/scripts/lib/test.sh"

Same pattern for coverage.sh, covgate.sh, update-covgates.sh, dep-updates.sh. Leave `lint.sh` unchanged (per decision log).

Add `SRC_DIR=agent/src` for covgate.sh and update-covgates.sh.

### Milestone 3: Convert lint-tool wrappers in `tools/lint/scripts/`

Same pattern, with lint-tool values:

    export CRATE_DIR="$REPO_ROOT/tools/lint"
    export CARGO_PKG=""
    export CARGO_FEATURES=""
    export CARGO_TEST_ARGS=""
    export RUST_LOG_OVERRIDE=""
    export SRC_DIR="src"

Leave `tools/lint/scripts/lint.sh` unchanged.

## Concrete Steps

### Milestone 1

Working directory: `agent/` (repo root)

    mkdir -p scripts/lib

Then create each file. After creating all 5:

    # Verify they're executable
    chmod +x scripts/lib/*.sh

    # Smoke test: run the simplest one directly
    CRATE_DIR=$(pwd) CARGO_PKG="--package miru-agent" CARGO_FEATURES="--features test" CARGO_TEST_ARGS="-- --test-threads=1" RUST_LOG_OVERRIDE="off" scripts/lib/test.sh

Expected output: cargo test runs and passes, same as `scripts/test.sh` today.

### Milestone 2

Working directory: `agent/`

Replace contents of `scripts/test.sh`, `scripts/coverage.sh`, `scripts/covgate.sh`, `scripts/update-covgates.sh`, `scripts/dep-updates.sh`.

Verify:

    ./scripts/test.sh
    ./scripts/covgate.sh

Expected output: identical behavior to before.

### Milestone 3

Working directory: `agent/`

Replace contents of `tools/lint/scripts/test.sh`, `tools/lint/scripts/coverage.sh`, `tools/lint/scripts/covgate.sh`, `tools/lint/scripts/update-covgates.sh`, `tools/lint/scripts/dep-updates.sh`.

Verify:

    ./tools/lint/scripts/test.sh
    ./tools/lint/scripts/covgate.sh

Expected output: identical behavior to before.

## Validation and Acceptance

1. `./scripts/test.sh` — agent tests pass (same output as before)
2. `./scripts/covgate.sh` — agent coverage gate passes with same module thresholds
3. `./scripts/coverage.sh` — HTML report generated at `target/coverage/html/index.html`
4. `./scripts/update-covgates.sh` — ratchets thresholds (or reports unchanged)
5. `./scripts/dep-updates.sh` — runs `cargo update --verbose`
6. `./tools/lint/scripts/test.sh` — lint tool tests pass
7. `./tools/lint/scripts/covgate.sh` — lint tool coverage gate passes
8. `./tools/lint/scripts/coverage.sh` — HTML report generated at `tools/lint/target/coverage/html/index.html`
9. `./tools/lint/scripts/update-covgates.sh` — ratchets thresholds
10. `./tools/lint/scripts/dep-updates.sh` — runs `cargo update --verbose` for lint tool
11. CI: existing `test` job (`./scripts/covgate.sh`) and `lint-tool` job (`./tools/lint/scripts/covgate.sh`) both pass

## Idempotence and Recovery

- All milestones are idempotent — rewriting script files is safe to repeat.
- If a milestone introduces a regression, `git checkout -- scripts/ tools/lint/scripts/` restores the originals.
- `scripts/lib/` is purely additive in milestone 1; deleting it reverts to the prior state since no wrapper depends on it until milestones 2–3.
- `lint.sh` is untouched in both locations, so lint behavior is unaffected throughout.
