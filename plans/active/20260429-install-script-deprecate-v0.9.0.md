# Deprecate install scripts for Miru Agent v0.9.0+

## Scope

Modify the three generated install scripts (`scripts/install/install.sh`, `scripts/install/staging-install.sh`, `scripts/install/uat-install.sh`) so they reject any attempt to install Miru Agent at version `v0.9.0` or greater, including pre-release tags (e.g. `v0.9.0-rc1`, `v0.9.0-beta`) and the unpinned "latest" path when `latest` resolves to v0.9.0+. Versions strictly less than `v0.9.0` (and at or above the existing `v0.6.0` floor) must continue to install normally.

In scope:

- `scripts/jinja/templates/scripts/install.j2` (the Jinja2 template that produces all three install variants).
- The three regenerated `.sh` files under `scripts/install/`.

Out of scope:

- The provision scripts (`scripts/install/provision.sh`, `staging-provision.sh`, `uat-provision.sh`) and their template `scripts/jinja/templates/scripts/provision.j2`.
- The shared partial `scripts/jinja/templates/partials/utils/version.sh`.
- Any documentation site, marketing copy, or apt-repo configuration.

## Purpose / Big Picture

Miru Agent v0.9.0 introduces a new apt-based provisioning workflow incompatible with the legacy curl-piped install scripts' tarball download path. This plan permanently deprecates the legacy scripts for v0.9.0+ via a hard refuse-and-redirect gate, while leaving v0.6.0..v0.8.x unchanged.

After this plan ships, running

    curl -sL https://install.miru... | sh -s -- --version=0.9.0

will fatal-exit before any download with:

    Version v0.9.0 cannot be installed with this script. Miru Agent v0.9.0 and later use a new apt-based provisioning workflow. See https://docs.mirurobotics.com/docs/developers/agent/install for instructions.

## Progress

One entry per milestone, marked `[ ]` when planned, `[x]` when committed.

- [ ] M1: Edit `scripts/jinja/templates/scripts/install.j2` to add the v0.9.0+ rejection gate.
- [ ] M2: Run `scripts/jinja/render.sh`; commit regenerated install scripts separately from the template change.
- [ ] M3: Run `scripts/preflight.sh`; resolve any findings.

## Surprises & Discoveries

Add entries as work proceeds.

## Decision Log

- Decision: Use Approach A (inline gate in `install.j2` after the `version.sh` include) for the v0.9.0+ rejection.
  Rationale: Self-contained, install-only; reuses the `MAJOR`/`MINOR` shell variables already exported by the partial. No new files; minimal surface.
  Date/Author: 2026-04-29 / authoring agent.
- Decision: Approach B (new wrapper partial `partials/utils/version-install.sh`) considered and deferred.
  Rationale: Cleaner separation but adds a new file for a single check. Reconsider if more install-only version logic is needed in the future.
  Date/Author: 2026-04-29 / authoring agent.
- 2026-04-29 — Pre-release tags and unpinned ("latest") path are handled implicitly. `version.sh`'s `PATCH=$(... | sed 's/[^0-9].*//')` strips suffixes like `-rc1`/`-beta`, and `MAJOR`/`MINOR` come from raw `cut -d '.' -f 1`/`-f 2` so `0.9.0-rc1` evaluates to `MAJOR=0 MINOR=9`. `version.sh` also resolves an unpinned `VERSION` from the GitHub releases API before the gate runs, so the gate sees the resolved value uniformly.

## Outcomes & Retrospective

Add entries as work proceeds.

## Context and Orientation

Repo root: `/home/ben/miru/workbench2/repos/agent`. Branch: `feat/install-script-deprecate-v0.9.0` (forked from `origin/main`). Read `AGENTS.md` at the repo root before editing.

### How install scripts are generated

The three install scripts at `scripts/install/install.sh`, `staging-install.sh`, and `uat-install.sh` are **generated** from Jinja2 templates. Always edit the template and rerun the renderer; direct edits to `.sh` files will be overwritten on next render.

Key files:

- `scripts/jinja/render.sh` — renderer entrypoint. Creates a `.venv`, installs `jinja2` and `pyyaml`, runs `python3 render.py --config install.yaml --output-dir ../install`. Run from inside `scripts/jinja/`.
- `scripts/jinja/install.yaml` — config defining six scripts (three install + three provision variants).
- `scripts/jinja/templates/scripts/install.j2` — install template; this is where the new gate goes.
- `scripts/jinja/templates/scripts/provision.j2` — provision template. Out of scope; do not edit.
- `scripts/jinja/templates/partials/utils/version.sh` — shared version-resolution/validation partial included by both templates. Do not edit.
- `scripts/jinja/templates/base/script.j2` — base template that `install.j2` extends.

Generated scripts contain a `# Build Timestamp:` line that updates on every render; a Build-Timestamp-only diff is expected and meaningful only as evidence the renderer ran.

### Existing version validation (reference — do not change)

From `scripts/jinja/templates/partials/utils/version.sh`, lines 16–25:

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

After this partial runs, `MAJOR`, `MINOR`, and `PATCH` are guaranteed numeric. The new gate relies on that.

### Existing structure of `install.j2` (reference)

    {#- Activate script template extending base -#}
    {% extends "script.j2" %}

    {% block utilities %}
    {% include 'partials/utils/checksum.sh' %}{{- "\n" -}}
    {% endblock %}

    {% from 'partials/arch.j2' import convert as convert_arch, convert_deb as convert_deb_arch %}

    {% block script_body %}
    {% include 'partials/os.j2' %}{{- "\n\n" -}}

    DEB_ARCH=$ARCH{{- "\n" -}}
    {{- convert_deb_arch(var='DEB_ARCH') -}}{{- "\n" -}}

    # USE PROVIDED PACKAGE #
    # -------------------- #
    {% include 'partials/utils/parse-from-pkg.sh' %}{{- "\n\n" -}}

    # DETERMINE THE VERSION #
    # --------------------- #
    {% include 'partials/utils/version.sh' %}{{- "\n\n" -}}

    # DOWNLOAD THE AGENT #
    # ------------------ #
    {% include 'partials/utils/download.sh' %}{{- "\n\n" -}}

    # ACTIVATE THE AGENT #
    # ------------------ #
    {% include 'partials/utils/activate.sh' %}{{- "\n" -}}

    {% endblock %}

The new gate goes between the `version.sh` and `download.sh` includes — after `VERSION` is resolved/validated, before any download.

### Shell dialect

Generated install scripts run as POSIX `sh`, not bash. Use `[ ... ]` not `[[ ... ]]`; no arrays, no `==`, no `local`. The existing `version.sh` partial is a good reference.

### Tooling

- Lint: `scripts/lint.sh`
- Preflight (lint + tests + tools lint + tools tests, in parallel): `scripts/preflight.sh`
- Install scripts have no automated test suite; validate by manual repro (see Validation and Acceptance).

## Plan of Work

Three milestones, executed in order. Each milestone ends with a commit so PR history is reviewable and bisectable.

1. **M1 — Add the rejection gate to the template.** Edit `scripts/jinja/templates/scripts/install.j2` to add the gate immediately after the `version.sh` include, using the `MAJOR`/`MINOR` shell variables. Provision template and `version.sh` partial are not modified.
2. **M2 — Regenerate the install scripts.** Run `scripts/jinja/render.sh` from inside `scripts/jinja/`. Verify by `git diff` that only the three install variants gained the new gate; provision variants must show only Build-Timestamp changes (or none). Split commits per milestone (M1 template-only, M2 generated `.sh`) for bisectable review.
3. **M3 — Preflight.** Run `scripts/preflight.sh` from the repo root. Resolve any findings and commit the cleanup; if preflight is already clean, no extra commit is needed.

## Concrete Steps

All paths are relative to repo root `/home/ben/miru/workbench2/repos/agent` unless otherwise noted.

### M1: Add the rejection gate to `install.j2`

In `scripts/jinja/templates/scripts/install.j2`, locate:

    # DETERMINE THE VERSION #
    # --------------------- #
    {% include 'partials/utils/version.sh' %}{{- "\n\n" -}}

Insert the new gate immediately after that include and before the `# DOWNLOAD THE AGENT #` block, so the region becomes:

    # DETERMINE THE VERSION #
    # --------------------- #
    {% include 'partials/utils/version.sh' %}{{- "\n\n" -}}

    # REJECT v0.9.0+ — INSTALL VIA APT INSTEAD #
    # ---------------------------------------- #
    if [ "$MAJOR" -gt 0 ] || { [ "$MAJOR" -eq 0 ] && [ "$MINOR" -ge 9 ]; }; then
        fatal "Version v$VERSION cannot be installed with this script. Miru Agent v0.9.0 and later use a new apt-based provisioning workflow. See https://docs.mirurobotics.com/docs/developers/agent/install for instructions."
    fi{{- "\n\n" -}}

    # DOWNLOAD THE AGENT #
    # ------------------ #
    {% include 'partials/utils/download.sh' %}{{- "\n\n" -}}

Notes:

- The condition matches every version ≥ v0.9.0 (any 1.x+, plus 0.9.x, 0.10.x, …); v0.6.0..v0.8.x fall through. Braces around the inner `[...] && [...]` are required to bind `&&` tighter than the outer `||` in POSIX `sh`. (See Context for why `MAJOR`/`MINOR` are safe numerics and why pre-releases / unpinned-latest are covered.)
- The `fatal "..."` is a single line and satisfies the three required properties: states this script does not support that version, names v0.9.0 as the boundary, and links to https://docs.mirurobotics.com/docs/developers/agent/install.
- Keep the surrounding `{{- "\n\n" -}}` Jinja whitespace markers consistent with other separators in the file.

Commit (template-only):

    git add scripts/jinja/templates/scripts/install.j2
    git commit -m "$(cat <<'EOF'
    feat(install-scripts): reject installs of agent v0.9.0+ in install.j2

    Miru Agent v0.9.0 introduces an apt-based provisioning workflow. The
    legacy install.sh family no longer supports v0.9.0+; the gate added in
    install.j2 fails fast with a link to the new docs. version.sh and
    provision.j2 are unchanged, so provision scripts keep their existing
    behavior.
    EOF
    )"

### M2: Regenerate the three install scripts

From the repo root:

    cd scripts/jinja
    ./render.sh

Then return to the repo root and restore the three provision scripts so they remain byte-identical to `origin/main`:

    cd "$(git rev-parse --show-toplevel)"
    git restore scripts/install/provision.sh scripts/install/staging-provision.sh scripts/install/uat-provision.sh

Verify:

    git status
    git diff scripts/install/install.sh
    git diff scripts/install/staging-install.sh
    git diff scripts/install/uat-install.sh

Each install variant should now contain the new `if [ "$MAJOR" -gt 0 ] ...` gate between the version-resolution and download blocks, plus an updated `# Build Timestamp:` line. Verify the provision variants did **not** gain the gate:

    git diff scripts/install/provision.sh
    git diff scripts/install/staging-provision.sh
    git diff scripts/install/uat-provision.sh

These three must show no diff after `git restore`. If any are still listed as modified, re-run the restore. If a provision script gained the new gate, the wrong template was edited — revert and re-do M1 against `install.j2` only.

Commit:

    git add scripts/install/install.sh scripts/install/staging-install.sh scripts/install/uat-install.sh
    git commit -m "chore(install): regenerate install scripts for v0.9.0+ rejection"

### M3: Preflight

From the repo root:

    scripts/preflight.sh

If clean, no further commit is needed. Otherwise:

    git add <fixed files>
    git commit -m "chore(install): address preflight findings"

## Validation and Acceptance

### Required gates

- preflight (`scripts/preflight.sh`) must report clean before changes are published.
- `scripts/lint.sh` must pass (part of preflight; called out for visibility).
- The three provision `.sh` files must be byte-identical to `origin/main`:

        git diff --exit-code origin/main -- \
            scripts/install/provision.sh \
            scripts/install/staging-provision.sh \
            scripts/install/uat-provision.sh

    Expected: exit code 0. Any output indicates a stray Build-Timestamp diff M2's `git restore` should have undone. The provision scripts must not contain the new fatal message or the new `if [ "$MAJOR" -gt 0 ]` block.

- The provision template and shared partial must be byte-identical to `origin/main`:

        git diff --exit-code origin/main -- \
            scripts/jinja/templates/scripts/provision.j2 \
            scripts/jinja/templates/partials/utils/version.sh

    Expected: exit code 0. Any diff indicates an accidental edit and must be reverted before merge.

### Manual test cases (run against the regenerated `scripts/install/install.sh` from M2)

For each case, run from the repo root:

    sh scripts/install/install.sh --version=<VERSION_UNDER_TEST>

The download step requires network and a real release artifact, so cases that "proceed past version check" may fail later in download — that is not the gate under test.

1. **`--version=0.8.0` → proceeds past the new gate.** Script prints `Version to install: 0.8.0` (or similar) and continues into download. Later download/network failure is acceptable. The new fatal message must **not** appear.
2. **`--version=0.9.0` → script exits with the new fatal message before any download.** Output contains: `Version v0.9.0 cannot be installed with this script. Miru Agent v0.9.0 and later use a new apt-based provisioning workflow. See https://docs.mirurobotics.com/docs/developers/agent/install for instructions.` Exit code non-zero. No `curl` to releases artifacts.
3. **`--version=0.9.0-rc1` → script exits with the new fatal message before any download.** Same expected output/exit-code as case 2 with `0.9.0-rc1` substituted.
4. **`--version=1.0.0` → script exits with the new fatal message before any download.** Same expected output/exit-code as case 2 with `1.0.0` substituted.
5. **Unpinned ("latest") path with no `--version` flag, when latest resolves to v0.9.0+.** Mock `curl` as below; do not skip. Stub `curl` returns a fake `releases/latest` JSON whose `tag_name` is `v0.9.0`. Script should exit with the new fatal message.

        mkdir -p /tmp/fakebin
        cat >/tmp/fakebin/curl <<'EOF'
        #!/bin/sh
        # Pretend the latest release is v0.9.0
        case "$*" in
            *releases/latest*) echo '"tag_name": "v0.9.0"' ;;
            *) echo "" ;;
        esac
        EOF
        chmod +x /tmp/fakebin/curl
        PATH="/tmp/fakebin:$PATH" sh scripts/install/install.sh

### Documentation pointer

Confirm https://docs.mirurobotics.com/docs/developers/agent/install resolves to a page documenting the new apt-based workflow. If it does not, raise it in Surprises & Discoveries; this plan does not block on doc-site availability, but the URL must be the right one.

## Idempotence and Recovery

### Idempotence

- Re-running `scripts/jinja/render.sh` is idempotent modulo the `# Build Timestamp:` line.
- The new gate is purely additive: re-running the install script with `--version=0.8.0` yields identical behavior; with `--version=0.9.0` it fatal-exits in the same place every time.
- Re-applying M1 on an already-edited template is a no-op when the gate text is identical; otherwise reconcile by hand.

### Recovery

- **Wrong template edited (e.g. `provision.j2`):** `git restore scripts/jinja/templates/scripts/provision.j2`, re-do M1 against `install.j2`, re-run M2.
- **Regenerated provision scripts gained the new gate (means `version.sh` was edited):** `git restore scripts/jinja/templates/partials/utils/version.sh`, re-do M1 in `install.j2`, re-run `render.sh`.
- **`render.sh` fails because Python or `python3-venv` is missing:** install prerequisites listed at the top of `scripts/jinja/render.sh` (or `AGENTS.md`), re-run. The renderer is safe to interrupt and rerun.
- **Preflight (M3) fails:** read the failing tool's output, fix the root cause, re-run. Do not bypass with `--no-verify`.
- **Abandon the change entirely:** `git checkout origin/main -- scripts/jinja/templates/scripts/install.j2 scripts/install/install.sh scripts/install/staging-install.sh scripts/install/uat-install.sh` and recommit; the branch may also be deleted.
