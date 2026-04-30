# Deprecate install scripts for Miru Agent v0.9.0+

## Scope

Modify the three generated install scripts (`scripts/install/install.sh`, `scripts/install/staging-install.sh`, `scripts/install/uat-install.sh`) so that they reject any attempt to install Miru Agent at version `v0.9.0` or greater, including pre-release tags (e.g. `v0.9.0-rc1`, `v0.9.0-beta`) and the unpinned "latest" path when `latest` resolves to v0.9.0+. Versions strictly less than `v0.9.0` (and at or above the existing `v0.6.0` floor) must continue to install normally.

In scope:

- `scripts/jinja/templates/scripts/install.j2` (the Jinja2 template that produces all three install variants).
- The three regenerated `.sh` files under `scripts/install/`.

Out of scope:

- The provision scripts (`scripts/install/provision.sh`, `staging-provision.sh`, `uat-provision.sh`) and their template `scripts/jinja/templates/scripts/provision.j2`. They share the `version.sh` partial but should not gain the new gate.
- The shared partial `scripts/jinja/templates/partials/utils/version.sh` itself — it stays as-is so provision scripts are unaffected.
- Any documentation site, marketing copy, or apt-repo configuration. The error message simply links to the existing docs page.

## Purpose / Big Picture

Miru Agent v0.9.0 introduces a new apt-based provisioning workflow. The legacy curl-piped install scripts at `scripts/install/install.sh` (and its staging/UAT siblings) cannot install v0.9.0+ correctly — they assume a tarball download path that no longer matches the v0.9.0 release artifacts. Rather than try to make the legacy scripts handle both worlds, we permanently deprecate them for v0.9.0+: the script must refuse to proceed and tell the user where to find the new instructions.

Concretely, after this plan ships, a user who runs

    curl -sL https://install.miru... | sh -s -- --version=0.9.0

will see a clear fatal error:

    Version v0.9.0 cannot be installed with this script. Miru Agent v0.9.0 and later use a new apt-based provisioning workflow. See https://docs.mirurobotics.com/docs/developers/agent/install for instructions.

…and the script will exit non-zero before any download is attempted. Users on `v0.6.0` through `v0.8.x` continue to be served by the same script unchanged.

The big picture is: the install script becomes a one-way deprecation gate for v0.9.0+ while remaining fully functional for older versions. There is no version-skew or capability negotiation — the gate is a hard refuse-and-redirect.

## Progress

Add entries as work proceeds. One entry per milestone, marked `[ ]` when planned, `[x]` when committed.

- [ ] M1: Edit `scripts/jinja/templates/scripts/install.j2` to add the v0.9.0+ rejection gate.
- [ ] M2: Run `scripts/jinja/render.sh` to regenerate the three install scripts; commit regenerated install scripts in a separate commit from the template change.
- [ ] M3: Run `scripts/preflight.sh`; resolve any findings.

## Surprises & Discoveries

Add entries as work proceeds.

## Decision Log

- Decision: Use Approach A (inline gate in `install.j2` after the `version.sh` include) for the v0.9.0+ rejection.
  Rationale: Self-contained, install-only; reuses the MAJOR/MINOR shell variables already exported by the partial. No new files; minimal surface.
  Date/Author: 2026-04-29 / authoring agent.
- Decision: Approach B (new wrapper partial `partials/utils/version-install.sh`) considered and deferred.
  Rationale: Approach B has cleaner separation but adds a new file for a single check. Reconsider if more install-only version logic is needed in the future.
  Date/Author: 2026-04-29 / authoring agent.
- **2026-04-29 — Pre-release tags handled implicitly:** The existing `PATCH=$(echo "$VERSION" | cut -d '.' -f 3 | sed 's/[^0-9].*//')` line in `version.sh` already strips suffixes like `-rc1` and `-beta`, so `0.9.0-rc1` parses to `MAJOR=0 MINOR=9 PATCH=0`. The new gate only needs to compare on `MAJOR` and `MINOR`; pre-release detection is not required.
- **2026-04-29 — Unpinned ("latest") path covered uniformly:** `version.sh` resolves `VERSION` from the GitHub releases API when `--version` is not provided. Because the new gate runs after the include, it sees the resolved value regardless of how it was obtained.

## Outcomes & Retrospective

Add entries as work proceeds.

## Context and Orientation

This section orients a novice reader who has only this plan and the current working tree.

### Repo

Repo root: `/home/ben/miru/workbench2/repos/agent`. Already-checked-out branch: `feat/install-script-deprecate-v0.9.0` (forked from `origin/main`). Read `AGENTS.md` at the repo root before editing — it documents repo-wide conventions.

### How install scripts are generated (this is the source of truth)

The three install scripts at `scripts/install/install.sh`, `scripts/install/staging-install.sh`, and `scripts/install/uat-install.sh` are **generated** from Jinja2 templates. Editing the generated `.sh` files directly is wrong; the next render will overwrite them. Always edit the template and rerun the renderer.

Key files:

- `scripts/jinja/render.sh` — renderer entrypoint. Creates a `.venv`, installs `jinja2` and `pyyaml`, then runs `python3 render.py --config install.yaml --output-dir ../install`. Run from inside `scripts/jinja/`.
- `scripts/jinja/install.yaml` — top-level config. Defines six scripts: three install variants (which use `install.j2`) and three provision variants (which use `provision.j2`).
- `scripts/jinja/templates/scripts/install.j2` — the install template. This is where the new gate goes.
- `scripts/jinja/templates/scripts/provision.j2` — the provision template. **Out of scope.** Do not edit.
- `scripts/jinja/templates/partials/utils/version.sh` — shared version-resolution and validation partial included by both `install.j2` and `provision.j2`. **Do not edit** — that would also affect provision scripts.
- `scripts/jinja/templates/base/script.j2` — the base script template that `install.j2` extends.

Generated scripts contain a `# Build Timestamp:` line near the top that updates on every render. A diff that shows only Build Timestamp changes is expected and meaningful only as evidence that the renderer ran.

### Existing version validation (reference — do not change)

Excerpt from `scripts/jinja/templates/partials/utils/version.sh`, lines 16–25:

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

After this partial runs, `MAJOR`, `MINOR`, and `PATCH` are guaranteed to be numeric (the parse-validation has already errored out otherwise). The new install-side gate can rely on those variables existing and being numbers.

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

The new gate goes between the `version.sh` include and the `download.sh` include — i.e. after `VERSION` is resolved and validated, and before any download begins.

### Shell dialect

The generated install scripts run as POSIX `sh`, not bash. Keep new shell logic POSIX-compatible: use `[ ... ]` rather than `[[ ... ]]`, no arrays, no `==` operator, no `local` keyword, etc. The existing `version.sh` partial is already POSIX-compatible and is a good reference.

### Tooling

- Lint: `scripts/lint.sh`
- Preflight (lint + tests + tools lint + tools tests, in parallel): `scripts/preflight.sh`
- The install scripts have no automated test suite. Validation is by manual repro commands (see Validation and Acceptance).

## Plan of Work

Three milestones, executed in order. Each milestone ends with a commit so the PR history is reviewable and bisectable.

1. **M1 — Add the rejection gate to the template.** Edit `scripts/jinja/templates/scripts/install.j2` to add the new gate immediately after the `version.sh` include. The gate uses the `MAJOR` and `MINOR` shell variables already set by `version.sh` and emits a fatal error matching the wording in Concrete Steps below. The provision template and the shared `version.sh` partial are not modified.

2. **M2 — Regenerate the install scripts.** Run `scripts/jinja/render.sh` from inside `scripts/jinja/`. This regenerates all six scripts under `scripts/install/`. Verify by `git diff` that only the three install variants gained the new gate; the three provision variants must show only Build Timestamp changes (or no changes if they were not re-rendered). **This plan uses split commits per milestone (M1 template only, M2 regenerated scripts)** so reviewers can see the template diff and the resulting `.sh` diff as distinct, bisectable units.

3. **M3 — Preflight.** Run `scripts/preflight.sh` from the repo root. Resolve any lint/test findings. Commit any cleanup needed to make preflight clean. If preflight is clean with no further edits, no additional commit is needed for this milestone.

## Concrete Steps

All paths are relative to the repo root `/home/ben/miru/workbench2/repos/agent` unless otherwise noted.

### M1: Add the rejection gate to `install.j2`

In `scripts/jinja/templates/scripts/install.j2`, locate this block:

    # DETERMINE THE VERSION #
    # --------------------- #
    {% include 'partials/utils/version.sh' %}{{- "\n\n" -}}

Insert the new gate immediately after that include and before the `# DOWNLOAD THE AGENT #` block, so the relevant region becomes:

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

Notes on the shell logic:

- `MAJOR` and `MINOR` are guaranteed numeric here because `version.sh` already parsed and validated them (and called `fatal` otherwise) before this point.
- The condition `[ "$MAJOR" -gt 0 ] || { [ "$MAJOR" -eq 0 ] && [ "$MINOR" -ge 9 ]; }` matches every version ≥ v0.9.0 (any 1.x, 2.x, …, plus 0.9.x, 0.10.x, etc.). Versions v0.6.0..v0.8.x fall through unchanged. The braces around the inner `[...] && [...]` are required to bind the `&&` tighter than the outer `||` in POSIX `sh`.
- Pre-release tags (e.g. `0.9.0-rc1`) are normalized by `version.sh`'s `sed 's/[^0-9].*//'` on `PATCH`, but `MAJOR` and `MINOR` are already pure numerics from `cut -d '.' -f 1` / `-f 2`, so `0.9.0-rc1` evaluates to `MAJOR=0 MINOR=9` and is rejected by this gate. No extra logic for pre-releases is needed.
- The error message must be a single line (single `fatal "..."` call). It satisfies the three required properties: it states this script does not support that version, it names v0.9.0 as the boundary, and it links to https://docs.mirurobotics.com/docs/developers/agent/install.
- Keep the surrounding `{{- "\n\n" -}}` Jinja whitespace-control markers consistent with the other section separators in this file.

Then commit with a message like:

    feat(install): reject v0.9.0+ in legacy install script

    The legacy curl-piped install script does not support the new
    apt-based provisioning workflow introduced in v0.9.0. Add a
    template-level gate so install.sh, staging-install.sh, and
    uat-install.sh fatal-exit with a pointer to the new docs page.

(Render is performed in M2; this commit is template-only on purpose so reviewers see the source-of-truth diff as one unit. M2 then commits the generated scripts.)

Commit (M1 only, template-only):

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

The renderer will create or reuse `.venv`, install `jinja2` and `pyyaml`, and write the regenerated scripts to `scripts/install/`. Then return to the repo root and immediately restore the three provision scripts so they remain byte-identical to `origin/main`:

    cd "$(git rev-parse --show-toplevel)"
    git restore scripts/install/provision.sh scripts/install/staging-provision.sh scripts/install/uat-provision.sh

render.sh regenerates all six scripts; restoring the three provision scripts keeps them byte-identical to origin/main, satisfying the UNCHANGED requirement.

    git status
    git diff scripts/install/install.sh
    git diff scripts/install/staging-install.sh
    git diff scripts/install/uat-install.sh

Each install variant should now contain the new `if [ "$MAJOR" -gt 0 ] ...` gate between the version-resolution block and the download block, plus an updated `# Build Timestamp:` line. Verify also that the provision variants did **not** gain the new gate:

    git diff scripts/install/provision.sh
    git diff scripts/install/staging-provision.sh
    git diff scripts/install/uat-provision.sh

These three should show only `# Build Timestamp:` changes (or be unchanged) and **must not** contain the new fatal message or the new `if [ "$MAJOR" -gt 0 ]` block.

If the provision scripts somehow gained the new gate, that means the wrong template was edited — revert and re-do M1 against `install.j2` only.

Commit the regenerated install scripts:

    git add scripts/install/install.sh scripts/install/staging-install.sh scripts/install/uat-install.sh
    git commit -m "chore(install): regenerate install scripts for v0.9.0+ rejection"

The provision scripts must be byte-identical to origin/main after this milestone — `git restore` undoes the timestamp-only diff that render.sh introduced. After the `git restore` step above, `git status` should show no pending changes for `scripts/install/provision.sh`, `staging-provision.sh`, or `uat-provision.sh`. If any are still listed as modified, re-run the restore command before committing.

### M3: Preflight

From the repo root:

    scripts/preflight.sh

Resolve any findings. If preflight is clean, no further commit is needed. If you had to fix anything, commit it:

    git add <fixed files>
    git commit -m "chore(install): address preflight findings"

## Validation and Acceptance

### Required gates

- preflight (`scripts/preflight.sh`) must report clean before changes are published.
- `scripts/lint.sh` must pass (it is part of preflight; calling it out separately for visibility).
- The three provision scripts must be byte-identical to `origin/main`. Run, from the repo root:

        git diff --exit-code origin/main -- \
            scripts/install/provision.sh \
            scripts/install/staging-provision.sh \
            scripts/install/uat-provision.sh

    Expected: exit code 0 (no diff). Any output indicates a stray Build-Timestamp diff that M2's `git restore` step should have undone. The provision scripts must not contain the new fatal message or the new `if [ "$MAJOR" -gt 0 ]` block.

- The shared partials and the provision template must also remain byte-identical to `origin/main`. Run, from the repo root:

        git diff --exit-code origin/main -- \
            scripts/jinja/templates/scripts/provision.j2 \
            scripts/jinja/templates/partials/utils/version.sh

    Expected: exit code 0. These files are out-of-scope; any diff indicates an accidental edit and must be reverted before the PR is merged.

### Manual test cases (run against the regenerated `scripts/install/install.sh` from M2)

These tests run the install script with `sh` and a flag that exits the script after the version gate. There is no automated test harness for these scripts, so validation is by manual invocation. For each test case, run from the repo root:

    sh scripts/install/install.sh --version=<VERSION_UNDER_TEST>

…and observe the exit code and output. The download step requires network and a real release artifact, so cases that "proceed past version check" are allowed to fail later in download — that failure is not the gate we are testing.

Test cases:

1. **`--version=0.8.0` → proceeds past the new gate.** The script should print `Version to install: 0.8.0` (or similar) and continue into the download step. It is acceptable for the script to fail later on download/network — that is not the gate under test. The new fatal message must **not** appear.
2. **`--version=0.9.0` → script exits with the new fatal message before any download.** Expected output contains: `Version v0.9.0 cannot be installed with this script. Miru Agent v0.9.0 and later use a new apt-based provisioning workflow. See https://docs.mirurobotics.com/docs/developers/agent/install for instructions.` Exit code is non-zero. No download attempt is made (no `curl` to the releases artifacts).
3. **`--version=0.9.0-rc1` → script exits with the new fatal message before any download.** Same expected output and exit-code as case 2, with `0.9.0-rc1` substituted into the version string.
4. **`--version=1.0.0` → script exits with the new fatal message before any download.** Same expected output and exit-code as case 2, with `1.0.0` substituted into the version string.
5. **Unpinned ("latest") path with no `--version` flag, when latest resolves to v0.9.0+.** Mock `curl` as below; do not skip this case. Create a temporary directory with a stub `curl` script that returns a fake `releases/latest` JSON whose `tag_name` is `v0.9.0`. Prepend that directory to `PATH` and re-invoke the install script with no `--version` flag. The script should exit with the new fatal message. Sketch:

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

Confirm that the URL https://docs.mirurobotics.com/docs/developers/agent/install resolves to a page that documents the new apt-based provisioning workflow. If it does not, raise it in Surprises & Discoveries — this plan does not block on doc-site availability, but the URL must at least be the right one.

## Idempotence and Recovery

### Idempotence

- **Re-running `scripts/jinja/render.sh` is idempotent modulo the `# Build Timestamp:` line.** The renderer rewrites all six scripts under `scripts/install/` on every invocation. The only differences between successive renders, given an unchanged template, are the embedded build timestamps. Implementers should not interpret a Build-Timestamp-only diff as evidence that something is wrong — it is expected.
- The new gate is purely additive. Running the install script twice (e.g. after a prior failed download) with `--version=0.8.0` produces the same behavior on each run. With `--version=0.9.0`, the script fatal-exits in the same place every time.
- Re-applying M1 (editing `install.j2`) on an already-edited template is a no-op if the gate text is identical; otherwise the implementer should reconcile by hand.

### Recovery

- **If the wrong template was edited (e.g. `provision.j2`):** `git restore scripts/jinja/templates/scripts/provision.j2`, then re-do M1 against `install.j2`. Re-run M2 to regenerate.
- **If the regenerated provision scripts accidentally got the new gate (which would mean `version.sh` was edited):** `git restore scripts/jinja/templates/partials/utils/version.sh`, re-do M1 in `install.j2` instead, then re-run `render.sh`.
- **If `render.sh` fails because Python or `python3-venv` is missing:** install the system prerequisites listed at the top of `scripts/jinja/render.sh` (or in `AGENTS.md`), then re-run. The renderer is safe to interrupt and rerun.
- **If preflight (M3) fails:** read the failing tool's output, fix the root cause, and re-run. Do not bypass with `--no-verify` or similar; preflight must report clean before publication.
- **To abandon the change entirely:** `git checkout origin/main -- scripts/jinja/templates/scripts/install.j2 scripts/install/install.sh scripts/install/staging-install.sh scripts/install/uat-install.sh` and recommit. The branch can also be deleted.
