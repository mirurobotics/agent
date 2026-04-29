# Deprecate `scripts/install/` for agent v0.9.0+

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Add a deprecation gate to the six legacy install scripts in `scripts/install/` and to the Jinja partial that generates them. |

This plan lives in `agent/plans/backlog/` because all changes are to scripts owned by this repo. No other repo is read or modified.

## Purpose / Big Picture

The six shell scripts under `scripts/install/` (`install.sh`, `provision.sh`, `staging-install.sh`, `staging-provision.sh`, `uat-install.sh`, `uat-provision.sh`) are being deprecated as of agent **v0.9.0**. They must continue to work for any release strictly before `v0.9.0` (i.e. `v0.6.0` through `v0.8.x`) but must refuse to run for `v0.9.0` or later.

After this change, a customer running

    curl -fsSL https://raw.githubusercontent.com/mirurobotics/agent/main/scripts/install/install.sh | sh -s -- --version=v0.9.0

will see a clear, customer-facing error explaining the deprecation and exit with a non-zero status. The same invocation with `--version=v0.8.5` continues to pass the version-validation gate (and proceeds toward download/install as before). Invocations that omit `--version` and would otherwise resolve to "latest" — which, as of today, points at `v0.9.0-alpha.1` (see `Cargo.toml`) — must also be blocked once "latest" resolves to v0.9.0+.

User-visible behavior to verify:

- `scripts/install/install.sh --version=v0.8.5 --debug` — passes the new gate (still fails downstream because no checksum download in a sandbox, but the gate itself prints "Version to install: 0.8.5" and proceeds past the validation block).
- `scripts/install/install.sh --version=v0.9.0` — exits non-zero with the deprecation message before any download or `dpkg` call.
- `scripts/install/install.sh --version=0.9.0` (no `v` prefix) — same deprecation behavior as above.
- `scripts/install/install.sh --version=v1.0.0` — same deprecation behavior.
- `scripts/install/install.sh` (no `--version` and no `--prerelease`) — exits non-zero with the deprecation message because `latest` is now ambiguous and may resolve to v0.9.0+.

## Progress

- [ ] M1: Confirm the rendering pipeline, baseline behavior, and the file that owns the version gate.
- [ ] M2: Update `scripts/jinja/templates/partials/utils/version.sh` to add the v0.9.0 deprecation gate (semver compare on `MAJOR`/`MINOR`).
- [ ] M3: Update `scripts/jinja/templates/partials/utils/version.sh` (or add a new partial) to reject the case where the user supplied no `--version` (i.e. the scripts would have to resolve "latest" / "latest pre-release" themselves), printing the deprecation error instead of fetching from GitHub.
- [ ] M4: Re-render the six install scripts via `scripts/jinja/render.sh` and commit the regenerated `scripts/install/*.sh` alongside the template change.
- [ ] M5: Add the test scenarios from "Validation and Acceptance" (run them manually; record transcripts under "Artifacts and Notes").
- [ ] Final: preflight clean (formatting, lint, tests). Preflight MUST report `clean` before changes are published.

Use timestamps when you complete steps.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Put the version gate in the shared Jinja partial `scripts/jinja/templates/partials/utils/version.sh` rather than in each of the six rendered scripts.
  Rationale: `scripts/install/*.sh` are generated, not hand-edited (the file headers say "Jinja Template: install.j2 / provision.j2" and `scripts/jinja/render.sh` writes them via `python3 render.py --config install.yaml --output-dir ../install`). Editing the rendered scripts directly would be overwritten on the next render. The same partial is included by both `install.j2` and `provision.j2` (`{% include 'partials/utils/version.sh' %}`), so a single edit covers all six rendered scripts. The partial already contains a comparable pre-existing gate (rejecting `< v0.6.0`); the new check is a natural extension of that block.
  Date/Author: 2026-04-29 / plan author.

- Decision: Reject "latest" resolution outright when a customer omits `--version`. Do not let the script resolve `latest` and then check the resolved version.
  Rationale: The task brief says "handle the special case of latest if the scripts support it — likely needs to resolve latest to a concrete version first, or otherwise reject latest as ambiguous now that the scripts are deprecated." Today the scripts call the GitHub releases API to discover latest. Once v0.9.0 ships, "latest" will return v0.9.0+, which must be blocked. Rather than fetch-then-check (which still hits the GitHub API and leaks the agent's release cadence to anyone running the deprecated script), refuse the operation early when `--version` is empty AND `--from-pkg` is empty. Customers who genuinely want a pre-v0.9.0 install must pass `--version=vX.Y.Z` explicitly.
  Date/Author: 2026-04-29 / plan author.

- Decision: Allow `--from-pkg=...` to bypass the "no --version supplied" early-reject, but still apply the v0.9.0 semver gate on the version extracted from the `.deb` package via `dpkg -f "$FROM_PKG" Version`.
  Rationale: The `parse-from-pkg.sh` partial sets `VERSION=$(dpkg -f "$FROM_PKG" Version)` before `version.sh` runs. The semver gate inside `version.sh` will then evaluate the package's actual version. This is the correct behavior: a customer who hand-supplies a pre-v0.9.0 `.deb` should still be able to install it from local disk; one who hand-supplies a v0.9.0+ `.deb` should be blocked with the same deprecation error, because the deprecated scripts are no longer the supported install path.
  Date/Author: 2026-04-29 / plan author.

- Decision: Use the existing shell-only semver compare style (`MAJOR=$(echo "$VERSION" | cut -d '.' -f 1)`, `MINOR=$(... -f 2)`) rather than introducing `sort -V` or any new helper.
  Rationale: Consistency with the pre-existing pre-v0.6.0 gate immediately above; no new external commands; works in `/bin/sh` (the scripts use `#!/bin/sh`, not bash). The comparison "version >= v0.9.0" is `MAJOR > 0 OR (MAJOR == 0 AND MINOR >= 9)`. For now there are no `>= v1.0.0` releases either, so the simpler form `MAJOR -ge 1 OR (MAJOR -eq 0 AND MINOR -ge 9)` is correct.
  Date/Author: 2026-04-29 / plan author.

- Decision: Reference the official documentation URL `https://docs.mirurobotics.com/docs/agent-sdk` (already cited in `README.md`) as the pointer to the new install method, with the placeholder note that the exact new install path is **not currently documented inside this repo**. The error message will say "see https://docs.mirurobotics.com/docs/agent-sdk for the supported installation method." If a more specific URL is published before merge, update the message.
  Rationale: Task requirement #3 says "if you can find it documented in the repo, reference it; if not, leave a placeholder and note this in the plan." A grep across `README.md`, `ARCHITECTURE.md`, `AGENTS.md`, `Cargo.toml`, and `scripts/` for the strings `v0.9`, `deprecat`, and "new install" produces only the existing pre-v0.6.0 gate text and no forward-looking documentation. The docs site URL is the safest pointer; flag this as a placeholder so a reviewer (or the implementer at execution time) can substitute a more specific URL.
  Date/Author: 2026-04-29 / plan author.

- Decision: Do not delete or move the six install scripts in this plan. Keep them in place and let the gate enforce the deprecation.
  Rationale: Customers who pinned a curl-pipe URL in their provisioning automation (a common pattern with `install.sh | sh`) need a graceful, message-bearing failure, not a 404. Removing the scripts would silently break those pipelines instead of telling the operator what to do. A future plan can remove them once telemetry shows no v0.9.0+ traffic.
  Date/Author: 2026-04-29 / plan author.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

**Repository under change:** `agent/` (the Miru Agent Rust project). Current `Cargo.toml` workspace version is `0.9.0-alpha.1`, so the v0.9.0 cutover is imminent.

**The six scripts being gated** (all under `agent/scripts/install/`):

- `install.sh` — production install.
- `staging-install.sh` — install pointing at `https://staging.api.mirurobotics.com`.
- `uat-install.sh` — install pointing at `https://uat.api.mirurobotics.com`.
- `provision.sh` — production provision (creates a device row, then installs).
- `staging-provision.sh` — staging provision.
- `uat-provision.sh` — UAT provision.

Each script is `/bin/sh` (POSIX, not bash) and accepts these arguments via `--key=value` flags (parsed by a `for arg in "$@"; do case $arg in ...; esac done` loop, see `scripts/jinja/templates/partials/args.j2`):

- `--version=vX.Y.Z` — explicit version. Optional. Default empty.
- `--prerelease` / `--prerelease=true` — when `--version` is empty, fetch the most recent prerelease instead of the latest stable. Default `false`.
- `--from-pkg=/path/to/.deb` — install from a local file. When provided, `VERSION` is read from `dpkg -f`. The deb path takes precedence over `--version`.
- Other flags (`--device-name`, `--backend-host`, `--mqtt-broker-host`, `--debug`, `--allow-reactivation` for provision) — not relevant to the gate.

**Generation pipeline.** The six scripts are NOT hand-edited:

- `agent/scripts/jinja/render.sh` activates a Python venv and runs `python3 render.py --config install.yaml --output-dir ../install`.
- `agent/scripts/jinja/render.py` loads templates from `agent/scripts/jinja/templates/{base,partials,scripts}/`, merges per-script variables from `agent/scripts/jinja/install.yaml`, and writes one `.sh` per entry under `scripts:` in the YAML. Output paths are `chmod 0o755`.
- The template chain is: `install.j2` (or `provision.j2`) `extends` `base/script.j2`, which yields header / display / arguments / utilities / variables / main / footer blocks. The "DETERMINE THE VERSION" section comes from `partials/utils/version.sh` via `{% include 'partials/utils/version.sh' %}` in both `install.j2` and `provision.j2`.

**The current version-gate file** — `agent/scripts/jinja/templates/partials/utils/version.sh` — already enforces `>= v0.6.0`:

    if [ -z "$VERSION" ]; then
        if [ "$PRERELEASE" = true ]; then
            log "Fetching latest pre-release version..."
            VERSION=$(curl -sL "https://api.github.com/repos/${GITHUB_REPO}/releases" |
                jq -r '.[] | select(.prerelease==true) | .tag_name' | head -n 1) || fatal "Failed to fetch latest pre-release version"
        else
            log "Fetching latest stable version..."
            VERSION=$(curl -sL "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" |
                grep "tag_name" | cut -d '"' -f 4) || fatal "Failed to fetch latest version"
        fi
    fi
    VERSION=$(echo "$VERSION" | cut -d 'v' -f 2)
    [ -z "$VERSION" ] && fatal "Could not determine latest version"

    # Validate the version is supported
    MAJOR=$(echo "$VERSION" | cut -d '.' -f 1)
    MINOR=$(echo "$VERSION" | cut -d '.' -f 2)
    PATCH=$(echo "$VERSION" | cut -d '.' -f 3 | sed 's/[^0-9].*//')
    if ! echo "$MAJOR" | grep -q '^[0-9]\+$' || ! echo "$MINOR" | grep -q '^[0-9]\+$' || ! echo "$PATCH" | grep -q '^[0-9]\+$'; then
        fatal "Could not parse version '$VERSION' to determine if it is supported"
    else
        if [ "$MAJOR" -lt 0 ] || [ "$MAJOR" -eq 0 ] && [ "$MINOR" -lt 6 ]; then
            fatal "Version v$VERSION has been deprecated, please install v0.6.0 or greater"
        fi
    fi
    log "Version to install: ${VERSION}"

The new gate sits inside the same `else` branch (or chained to it), checks the upper bound, and prints the deprecation message.

**Helper functions available in scope.** `fatal()` (from `partials/display.sh`) prints `${RED}Error:${NO_COLOR} <msg>` on stderr-equivalent and `exit 1`. `log()` prints `${GREEN}==>${NO_COLOR} <msg>` informationally. Both are defined at the top of every rendered script.

**Argument-flow ordering inside each rendered script** (matters because the gate must run before any irreversible action):

1. `# ARGUMENTS #` — flags parsed.
2. `# UTILITIES #` — `cmd_exists`, then `for cmd in curl grep cut jq` requirement check.
3. `# VARIABLES #` — `ARCH`, `DOWNLOAD_DIR`, `AGENT_DEB_PKG_NAME`, `GITHUB_REPO`, `CHECKSUMS_FILE`, `DEB_PKG_MIME_TYPE`.
4. `# MAIN LOGIC #` — OS check, arch normalization, then per-template body:
   - `install.j2`: USE PROVIDED PACKAGE → DETERMINE THE VERSION → DOWNLOAD THE AGENT → ACTIVATE THE AGENT.
   - `provision.j2`: USE PROVIDED PACKAGE → PROVISION THE DEVICE (HTTP POSTs to the backend!) → DETERMINE THE VERSION → DOWNLOAD → ACTIVATE.

Critical observation: `provision.j2` calls `POST /v1/devices` and `POST /v1/devices/<id>/activation_token` BEFORE the "DETERMINE THE VERSION" block runs. Putting the v0.9.0 gate inside `version.sh` is therefore too late for `provision.sh` — by the time the gate fires, a device row may have been created in the customer's account. **The gate must be hoisted to run BEFORE the provision step.** See "Plan of Work" for how this is handled.

## Plan of Work

The work is one logical change spread across two Jinja partial edits (so the gate fires before any side-effects in both `install.j2` and `provision.j2`), then a re-render.

### Step 1: Add an early `--version` deprecation gate that runs before any side-effects

Create a new partial: `agent/scripts/jinja/templates/partials/utils/deprecation-gate.sh`. Contents:

    # DEPRECATION GATE (v0.9.0) #
    # ------------------------- #
    # These scripts are being phased out as of agent v0.9.0. Reject any request
    # that targets v0.9.0 or later, and reject the "latest" path (--version
    # unset and --from-pkg unset) because "latest" now resolves to v0.9.0+.
    # Customers must use the installation method documented at
    # https://docs.mirurobotics.com/docs/agent-sdk.

    if [ -z "$FROM_PKG" ] && [ -z "$VERSION" ]; then
        error "These installation scripts are deprecated as of agent v0.9.0."
        error "Resolving the 'latest' release is no longer supported here because"
        error "the latest release is v0.9.0 or newer."
        error "If you need a pre-v0.9.0 release, pass an explicit --version=vX.Y.Z (where X.Y.Z is < 0.9.0)."
        fatal "Otherwise, see https://docs.mirurobotics.com/docs/agent-sdk for the supported installation method."
    fi

    if [ -n "$VERSION" ]; then
        # Strip leading 'v' so users can pass either 'v0.8.5' or '0.8.5'.
        DEPRECATION_VERSION=$(echo "$VERSION" | sed 's/^v//')
        DEPRECATION_MAJOR=$(echo "$DEPRECATION_VERSION" | cut -d '.' -f 1)
        DEPRECATION_MINOR=$(echo "$DEPRECATION_VERSION" | cut -d '.' -f 2)
        if echo "$DEPRECATION_MAJOR" | grep -q '^[0-9]\+$' && echo "$DEPRECATION_MINOR" | grep -q '^[0-9]\+$'; then
            if [ "$DEPRECATION_MAJOR" -ge 1 ] || { [ "$DEPRECATION_MAJOR" -eq 0 ] && [ "$DEPRECATION_MINOR" -ge 9 ]; }; then
                error "These installation scripts are deprecated as of agent v0.9.0."
                error "You requested v$DEPRECATION_VERSION, which is v0.9.0 or newer."
                error "These scripts can only be used to install pre-v0.9.0 releases."
                fatal "See https://docs.mirurobotics.com/docs/agent-sdk for the supported installation method."
            fi
        fi
        unset DEPRECATION_VERSION DEPRECATION_MAJOR DEPRECATION_MINOR
    fi

Notes on this partial:

- The first block fires when both `--version` and `--from-pkg` are empty (i.e. the only path that would otherwise resolve "latest"). It runs before the GitHub API is contacted and before any backend HTTP call in `provision.j2`.
- The second block validates the user-supplied `--version` only when present. When `--from-pkg` is supplied, `VERSION` is initially empty here (it is set later by `parse-from-pkg.sh`); the second block harmlessly skips. The `from-pkg` case is then re-checked by Step 2 below.
- We use shell-only string ops (`sed 's/^v//'`, `cut -d '.' -f N`) to avoid introducing new tool dependencies. The check rejects exactly `MAJOR >= 1` OR `(MAJOR == 0 AND MINOR >= 9)`, i.e. semver `>= 0.9.0`.

### Step 2: Hoist the deprecation gate ahead of provision side-effects in both templates

Edit `agent/scripts/jinja/templates/scripts/install.j2`. Insert the new partial **after** `# USE PROVIDED PACKAGE #` (so that `--from-pkg` has already populated `VERSION` and the gate evaluates the package's version) and **before** `# DETERMINE THE VERSION #`:

    # USE PROVIDED PACKAGE #
    # -------------------- #
    {% include 'partials/utils/parse-from-pkg.sh' %}{{- "\n\n" -}}

    # DEPRECATION GATE (v0.9.0) #
    # ------------------------- #
    {% include 'partials/utils/deprecation-gate.sh' %}{{- "\n\n" -}}

    # DETERMINE THE VERSION #
    ...

Edit `agent/scripts/jinja/templates/scripts/provision.j2` similarly, but place the new include **before** `# PROVISION THE DEVICE #` so the device-creation HTTP call never happens for a v0.9.0+ request:

    # USE PROVIDED PACKAGE #
    # -------------------- #
    {% include 'partials/utils/parse-from-pkg.sh' %}{{- "\n\n" -}}

    # DEPRECATION GATE (v0.9.0) #
    # ------------------------- #
    {% include 'partials/utils/deprecation-gate.sh' %}{{- "\n\n" -}}

    # PROVISION THE DEVICE #
    # --------------------- #
    {% include 'partials/utils/provision.sh' %}{{- "\n\n" -}}

    # DETERMINE THE VERSION #
    ...

### Step 3: Leave `partials/utils/version.sh` mostly alone

The pre-existing pre-v0.6.0 gate inside `version.sh` stays. It still serves as a defense-in-depth check for the case where (in some future) `--version` validation needs to handle a resolved-from-API version differently. We do **not** add the v0.9.0 gate here, because by the time `version.sh` runs in `provision.j2`, the device row has already been created — see the critical observation in "Context and Orientation."

### Step 4: Re-render the six install scripts

From `agent/scripts/jinja/`:

    ./render.sh

This regenerates `agent/scripts/install/{install,staging-install,uat-install,provision,staging-provision,uat-provision}.sh`. Diff each against its previous content; the only changes should be the inserted "DEPRECATION GATE (v0.9.0)" block plus the updated `Build Timestamp:` header.

### Step 5: Verify the six rendered scripts contain the gate

Run:

    grep -c "DEPRECATION GATE" agent/scripts/install/*.sh

Expected output: each of the six files prints `1`.

### Step 6: Commit the regenerated scripts

Commit the template changes (`agent/scripts/jinja/templates/scripts/install.j2`, `provision.j2`, and the new `partials/utils/deprecation-gate.sh`) **together with** the regenerated `agent/scripts/install/*.sh`. Do not split these into separate commits — the rendered scripts are derived artifacts and must stay in lockstep with the templates.

## Concrete Steps

All commands run from the agent repo root: `agent/`.

### Set up

    cd agent

### Edit templates

Use your editor of choice:

    $EDITOR scripts/jinja/templates/partials/utils/deprecation-gate.sh   # new file
    $EDITOR scripts/jinja/templates/scripts/install.j2                   # add the include
    $EDITOR scripts/jinja/templates/scripts/provision.j2                 # add the include (BEFORE provision.sh)

### Render

    cd scripts/jinja
    ./render.sh
    cd ../..

Expected transcript:

    ==> Building scripts from Jinja2 templates...
    Rendering Miru shell scripts with Jinja2...
    ...
    Rendering install.sh from install.j2...
    Generated: ../install/install.sh
    ...
    Render complete: 6/6 scripts rendered successfully

### Confirm gate is present

    grep -c "DEPRECATION GATE" scripts/install/*.sh

Expected:

    scripts/install/install.sh:1
    scripts/install/provision.sh:1
    scripts/install/staging-install.sh:1
    scripts/install/staging-provision.sh:1
    scripts/install/uat-install.sh:1
    scripts/install/uat-provision.sh:1

### Run the test scenarios

See "Validation and Acceptance" for the exact invocations and expected outputs. Capture the stdout/stderr of each into "Artifacts and Notes" below as evidence.

### Preflight

    ./scripts/lint.sh
    ./scripts/test.sh

These must report `clean` (no errors, no failing tests). If preflight is not clean, do not publish the change.

## Validation and Acceptance

The new gate is verified by running the six test scenarios below. Each scenario invokes the rendered script directly (no `curl | sh`) so we can read the exit code and message. Use `bash -c` to wrap them and read the exit code separately.

**T1 — Pre-v0.9.0 explicit version passes the gate.**

    sh scripts/install/install.sh --version=v0.8.5 --debug; echo "exit=$?"

Expected: prints the parsed argument debug lines, then `==> Version to install: 0.8.5` (or similar), then fails later (because we're not actually running on a Linux host with the deb tooling, OR with `MIRU_ACTIVATION_TOKEN` set). The point is the gate does not block; observable evidence is the absence of the deprecation error and the appearance of `Version to install: 0.8.5`. Exit code may be non-zero from later steps; that is acceptable.

**T2 — `v0.9.0` is rejected.**

    sh scripts/install/install.sh --version=v0.9.0; echo "exit=$?"

Expected: prints (in red) lines beginning with `Error:` containing "deprecated as of agent v0.9.0" and "You requested v0.9.0", then "see https://docs.mirurobotics.com/docs/agent-sdk". Exit code is 1.

**T3 — `0.9.0` (no `v` prefix) is rejected.**

    sh scripts/install/install.sh --version=0.9.0; echo "exit=$?"

Expected: same as T2.

**T4 — `v1.0.0` is rejected.**

    sh scripts/install/install.sh --version=v1.0.0; echo "exit=$?"

Expected: same as T2 (with "You requested v1.0.0").

**T5 — Missing `--version` (would resolve "latest") is rejected.**

    sh scripts/install/install.sh; echo "exit=$?"

Expected: prints "Resolving the 'latest' release is no longer supported here", followed by the docs URL. Exit code is 1. **Crucially: no curl call to `api.github.com` is made.** Verify by running with `--debug` and confirming no "Fetching latest stable version..." log appears.

**T6 — `provision.sh` rejects v0.9.0 BEFORE creating a device.**

    MIRU_API_KEY=fake sh scripts/install/provision.sh --version=v0.9.0 --device-name=test-host; echo "exit=$?"

Expected: prints the deprecation error and exits 1. **Crucially: no `POST` to `/v1/devices` is made.** This is the key correctness property of placing the gate before `partials/utils/provision.sh`. Verify by running on a host whose DNS does not resolve `api.mirurobotics.com` (or by capturing network traffic) — there should be no outbound request.

**Acceptance summary.** The change is acceptable when:

- T1 demonstrates a pre-v0.9.0 install version is allowed past the gate.
- T2, T3, T4 demonstrate that v0.9.0+ versions are rejected, in both `vX.Y.Z` and `X.Y.Z` form.
- T5 demonstrates that "latest" (no `--version`) is rejected without contacting the GitHub API.
- T6 demonstrates that the `provision.sh` flow rejects before creating a device.
- All six rendered scripts contain the gate (Step 5 above).
- `./scripts/lint.sh` and `./scripts/test.sh` both report `clean`.

**Preflight gate.** Per task requirement 5: preflight must report `clean` before changes are published. If lint or tests fail, do not merge.

## Idempotence and Recovery

Every step is idempotent:

- Editing the templates is a normal text edit; re-running has no effect.
- `scripts/jinja/render.sh` always overwrites the six output files — re-running it produces the same content (modulo the `Build Timestamp:` header, which changes). If you want a deterministic diff, set the build time manually in `install.yaml` `global_variables.build_time` before rendering.
- The deprecation gate itself is read-only — it inspects `$VERSION` and `$FROM_PKG` and either calls `fatal` (exit 1) or falls through. It cannot leave the system in a partial state.

**Rollback.** To revert, `git revert` the commit. The rendered scripts will fall back to the pre-change versions; templates likewise. Customers running the old scripts will see the pre-v0.6.0 gate as the only version check, as before.

## Interfaces and Dependencies

- POSIX shell (`/bin/sh`) — no bashisms in the new partial.
- `cut`, `sed`, `grep`, `echo` — all already required by `partials/utils/version.sh`. No new tool dependencies.
- Jinja2 + PyYAML — installed by `scripts/jinja/render.sh` into `.venv`. Already required for any change to the install scripts.

## Artifacts and Notes

(Populate during implementation with the actual transcripts of T1–T6.)
