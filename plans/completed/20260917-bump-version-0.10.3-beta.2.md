# Bump workspace version to 0.10.3-beta.2

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent) | read-write | Change the Cargo workspace version in `Cargo.toml` and the three matching `Cargo.lock` entries. |

This plan lives in `agent/plans/` because the only change is in this repo. All paths below are relative to the agent repo root (`/home/ben/miru/workbench3/repos/agent`). Work happens on branch `chore/bump-version-0.10.3-beta.2`, which targets `main`.

## Purpose / Big Picture

The next prerelease tag will be `v0.10.3-beta.2`. `agent/build.rs` refuses to build a tagged commit whose tag does not match the Cargo version, and the binary reports `v` + `CARGO_PKG_VERSION` as its version. After this change, `cargo metadata` reports `0.10.3-beta.2` for the three workspace crates, and a build tagged `v0.10.3-beta.2` passes the tag check.

## Progress

- [x] Milestone 1: bump `Cargo.toml`, refresh `Cargo.lock`, verify, commit.
- [x] Preflight reports `CLEAN` (CI green on PR #247 at 488d1ab).

## Surprises & Discoveries

- None. `cargo update --workspace --offline` changed exactly the three workspace entries; `cargo check --workspace --locked --offline` passed.

## Decision Log

- Commit message uses `0.10.3-beta.2` (no `v` prefix), matching the Cargo version rather than the tag name.
- Separate refine and test-writing passes were skipped: the diff is four version lines, and the existing `agent/tests/version/mod.rs` covers the runtime version string.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

The version is set in one place, `Cargo.toml` line 13, under `[workspace.package]`: `version = "0.10.2"`. The workspace crates `agent/` (package `miru-agent`), `libs/backend-api/` (package `backend-api`), and `libs/device-api/` (package `device-api`) inherit it with `version = { workspace = true }`. `tools/lint` sets its own `0.1.0` and is not part of this change. `Cargo.lock` records the version once for each workspace crate, at the `name = "miru-agent"`, `name = "backend-api"`, and `name = "device-api"` entries. The other `0.10.2` strings in `Cargo.lock` belong to third-party crates (for example `rand` and `const-oid`) and must not change. CI builds with `--locked` (see `.github/workflows/ci.yml`, `windows-package` job), so a stale lockfile fails CI.

Other code that uses the version was checked, and none of it needs edits:

- `agent/build.rs`: when HEAD has an exact tag, it asserts that the tag without the leading `v` equals `CARGO_PKG_VERSION`. `v0.10.3-beta.2` matches `0.10.3-beta.2`.
- `agent/src/version/mod.rs` and `agent/tests/version/mod.rs`: both derive the version from `env!("CARGO_PKG_VERSION")`. No version literal is hardcoded.
- `build/.goreleaser.yaml`: uses `{{ .Version }}` from the git tag and sets `release.prerelease: auto`, so a `-beta.N` tag is published as a prerelease. This has been done before: commit 2714881 set `version = "0.10.2-beta.1"` for tag `v0.10.2-beta.1`. That commit left `Cargo.lock` stale, which is why this plan refreshes it.
- Windows MSI (`build/windows/miru-agent.wixproj`, `build/windows/miru-agent.wxs`): `Version` is passed explicitly with `-p:Version=...` and must match `^[0-9]+\.[0-9]+\.[0-9]+$`. A prerelease value fails with `MIRUMSI1002`. Nothing derives this value from `Cargo.toml`. `build/windows/tests/package-tests.ps1` uses fixed test versions (`1.2.0`, `1.2.3`, and `1.2.3-beta.1` as the expected-failure case), and `release.yml` does not build an MSI. The `-beta.2` suffix does not break CI or the release. Any future step that feeds the Cargo or tag version into the MSI build must remove the prerelease suffix first, for example by mapping it to `0.10.3`.

## Plan of Work

In `Cargo.toml`, `[workspace.package]`, change `version = "0.10.2"` to `version = "0.10.3-beta.2"`. Then run `cargo update --workspace --offline`. This updates only the workspace crates' entries in `Cargo.lock`, and `--offline` keeps it from resolving newer third-party versions. No other files change.

## Concrete Steps

Run all commands from `/home/ben/miru/workbench3/repos/agent`.

1. Edit the version:

        sed -i 's/^version = "0.10.2"$/version = "0.10.3-beta.2"/' Cargo.toml
        grep -n '^version' Cargo.toml
        # 13:version = "0.10.3-beta.2"

2. Refresh the lockfile:

        cargo update --workspace --offline
        # Updating backend-api v0.10.2 (...) -> v0.10.3-beta.2
        # Updating device-api v0.10.2 (...) -> v0.10.3-beta.2
        # Updating miru-agent v0.10.2 (...) -> v0.10.3-beta.2

3. Confirm that the diff contains only the four version lines:

        git diff --stat
        # Cargo.lock | 6 +++---
        # Cargo.toml | 2 +-

   `git diff Cargo.lock` must show exactly three `-version = "0.10.2"` / `+version = "0.10.3-beta.2"` pairs, one each under `backend-api`, `device-api`, and `miru-agent`.

4. Commit (Milestone 1):

        git add Cargo.toml Cargo.lock
        git commit -S -m "chore: bump version to v0.10.3-beta.2"

## Validation and Acceptance

Run from the repo root:

    cargo metadata --no-deps --format-version 1 --offline | jq -r '.packages[] | "\(.name) \(.version)"'
    # miru-agent 0.10.3-beta.2
    # backend-api 0.10.3-beta.2
    # device-api 0.10.3-beta.2
    # (tools/lint is not a workspace member and does not appear)

    cargo check --workspace --locked --offline

`cargo check` must succeed. Because of `--locked`, it also fails if `Cargo.lock` is out of date. `agent/tests/version/mod.rs` covers the runtime version string in CI.

The task is complete only when preflight reports `CLEAN`, meaning CI is green on the pushed branch head SHA. Keep the PR in draft until then, and do not report the task complete until then.

## Idempotence and Recovery

Every step can be repeated safely. The `sed` does nothing after the first run, and `cargo update --workspace --offline` does nothing when the lockfile is already current. If step 3 shows any third-party version change, run `git checkout -- Cargo.lock` and repeat step 2. If an offline update fails because a crate is missing from the cache, run `cargo fetch --locked` first and retry. To roll back before the commit, run `git checkout -- Cargo.toml Cargo.lock`.
