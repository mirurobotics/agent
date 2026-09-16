# Add the Windows (x86_64-pc-windows-msvc) release lane to GoReleaser


This ExecPlan is a living document. Keep Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective up to date during implementation.

## Scope


| Repository | Access | Work |
| --- | --- | --- |
| `/home/ben/miru/workbench4/repos/agent` | Read-write | `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `build/.goreleaser.yaml`, `.gitignore`, `build/windows/README.md`, `plans/active/20260910-windows-support.md`, and this plan. |
| `/home/ben/miru/workbench4` | Read-only | Workspace instructions and shared skill policies. Independent Git checkout, not a submodule. |

This plan lives in the agent repository because it owns the workflows and the GoReleaser config. Work happens on branch `feat/windows-release-lane` (based on `main` at `cd770ef0`). It is PR 8 of the roadmap in `plans/active/20260910-windows-support.md`.

Out of scope, deliberately: the MSI `windows-package` job and everything under `build/windows/` except one README paragraph; the Windows service work (roadmap PR 6, tracked as #242); Authenticode signing (roadmap PR 10); WinGet; any production Rust code, `Cargo.toml` profile, `.cargo/config.toml`, or `build/Dockerfile.builder` change (the Linux builder image is untouched).

## Purpose / Big Picture


Today a tag on `main` produces Linux artifacts only (`agent_Linux_x86_64.tar.gz`, `agent_Linux_arm64.tar.gz`, `.deb` packages, SBOMs, checksums). After this plan, the same tag additionally publishes `agent_Windows_x86_64.zip` containing `miru-agent.exe` built natively with the MSVC toolchain through `cargo auditable` (so the executable carries a `.dep-v0` dependency section like the Linux binaries), a matching SPDX SBOM, and the executable's debug symbols `miru_agent.pdb` as a separate release asset. Nothing is signed yet.

A reviewer can see it working before any tag exists: CI on the pull request builds the Windows executable, hands it to a Linux job, runs GoReleaser in snapshot mode, and uploads the resulting `dist/` directory as a workflow artifact whose contents prove the zip, the SBOM and the PDB exist.

## Progress


- [ ] M1: `windows-release-build` job in `ci.yml` uploads `miru-agent.exe` + `miru_agent.pdb`.
- [ ] M2: `build/.goreleaser.yaml` gains the `agent-windows` prebuilt build, archive/nfpm id filters, and the PDB extra file; `goreleaser check` passes locally.
- [ ] M3: `goreleaser-snapshot` dry-run job in `ci.yml` proves ingestion on the PR.
- [ ] M4: `release.yml` downloads the Windows artifact before `build/release.sh`.
- [ ] M5: docs (`build/windows/README.md`, roadmap PR 8 entry) updated.
- [ ] Final: preflight `CLEAN`, dry-run artifact inspected, PR leaves draft.

## Surprises & Discoveries


(Add entries as you go.)

## Decision Log


(Authoring decisions are in Context and Plan of Work; add implementation-time entries here.)

## Outcomes & Retrospective


(Summarize at completion.)

## Context and Orientation


How a release is produced today. `.github/workflows/release.yml` runs on every pushed tag: `check-main` verifies the tag is an ancestor of `main` or a `release/*` branch (output `on_main`), `ci` calls `.github/workflows/ci.yml` as a reusable workflow (`workflow_call`) gated on `on_main == 'true'`, and `release` (`needs: [ci, check-main]`, same gate) checks out with `fetch-depth: 0` and runs `build/release.sh`. That script sources `build/git-tags.sh` to compute the previous `vX.Y.Z` tag, then runs `docker build -f build/Dockerfile` with `GORELEASER_ARGS=""`; `build/Dockerfile` starts from the prebuilt builder image `ghcr.io/mirurobotics/agent-builder:43e2c5b` (Rust 1.93.0, Zig, cargo-zigbuild, cargo-auditable, GoReleaser 2.13.3, syft 1.46.0; defined by `build/Dockerfile.builder`, built by `builder.yml`), does `COPY . .` of the whole checkout into `/workspace`, and runs `cd build && goreleaser release ${GORELEASER_ARGS} --clean`. Artifacts land in `build/dist/`. `build/build.sh` is the same thing with the Dockerfile's default `GORELEASER_ARGS="--snapshot"`, i.e. a local dry run. There is no `.dockerignore`, so anything present in the checkout (including files a workflow step drops there) is inside the Docker build context.

`build/.goreleaser.yaml` (GoReleaser Pro, `version: 2`, `pro: true`) has one build id `agent` (`builder: rust`, `tool: cargo-auditable-zigbuild`, targets `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, `dir: ..`), an `nfpms` entry producing `.deb`, one `archives` entry (`id: agent`, `ids: ["agent"]`, `formats: [tar.gz]`, name template `agent_<Os>_<x86_64|arm64>`, and a `format_overrides` zip for `goos: windows` that nothing produces yet), `sboms` for `archive` and `binary` artifacts (syft, SPDX JSON), a changelog, and a `release` stanza. GoReleaser's `.Version` is the tag without the leading `v`; in snapshot mode it is `{{ incpatch .Version }}-next` (e.g. tag `v0.10.2` on `main` gives snapshot version `0.10.3-next`). Release archives do not carry the version in their file name; SBOM documents and `.deb` files do.

GoReleaser Pro's `prebuilt` builder (docs: https://goreleaser.com/customization/prebuilt/) imports binaries built elsewhere: `builder: prebuilt`, explicit `goos`/`goarch` (no defaults), `prebuilt.path` as a template to the external file, and `binary` as the name used inside archives. Quoting the docs: "The other steps of the pipeline will act as if those were built by GoReleaser itself. There is no difference in how the binaries are handled", and "GoReleaser will try to stat the final path, if any error happens while doing that (e.g. file does not exist or permission issues), GoReleaser will fail." The `dist` directory is removed before the run, so the prebuilt file must live elsewhere. Template fields available on build-related paths include `.Os`, `.Arch`, `.Target` (https://goreleaser.com/customization/templates/, "Single-artifact extra fields"); GoReleaser's `internal/tmpl/tmpl.go` also defines a build key `Ext`, but whether the Pro prebuilt builder fills it for Windows is undocumented, so this plan does not rely on it.

How Windows builds work in CI today. `ci.yml` has `windows-check` (test suite on `blacksmith-4vcpu-windows-2025`; `dtolnay/rust-toolchain@631a55b1... # stable`, `Swatinem/rust-cache@6323deb1... # v2`, NASM via `choco install nasm` for `aws-lc-sys`), `windows_package_scope` (job name `windows-package-scope`; on pull requests it runs `dorny/paths-filter` over `build/windows/**` and both workflow files and exposes output `windows_package`, which is always `true` for non-PR events, including the `workflow_call` from `release.yml`), and `windows-package` (MSI build and installer tests on `windows-latest`, gated on that output). The repository's Actions policy (`gh api repos/mirurobotics/agent/actions/permissions/selected-actions`) is `allowed_actions: selected` with `github_owned_allowed: true`, `verified_allowed: true` and patterns `taiki-e/install-action@*`, `Swatinem/rust-cache@*`, `cargo-bins/cargo-binstall@*`, `dtolnay/rust-toolchain@*`, `dorny/paths-filter@*`, plus a few others; `sha_pinning_required: true`, so every `uses:` must be a full SHA with a version comment, as the file already does.

Facts that shape the design (all verified while authoring, 2026-09-16):

- cargo-auditable supports Windows: its README says "Linux, Windows and Mac OS are officially supported", and `auditable-extract/src/lib.rs` has a `Format::PE` arm reading section `.dep-v0`. syft's Rust "cargo-auditable-binary-cataloger" (`syft/pkg/cataloger/rust/parse_audit_binary.go`) delegates to `github.com/rust-secure-code/go-rustaudit`, whose `rustaudit.go` opens files with `debug/pe` and reads `.dep-v0`; syft's `ExecutableMIMETypeSet` includes `application/vnd.microsoft.portable-executable`. So a syft scan of `miru-agent.exe` is expected to enumerate crates exactly like the ELF scan does. The dry run in M3 proves it; if it does not, SBOM scope is narrowed rather than guessed at (see Plan of Work, M3).
- `taiki-e/install-action` is allowlisted and already used (`@21eb0b62... # cargo-llvm-cov`, which is a commit of the moving per-tool tag `cargo-llvm-cov`). Its `TOOLS.md` lists `cargo-auditable` for Linux, macOS and Windows; the per-tool tag `cargo-auditable` currently resolves to `47345a21ef01b2d0e9b59e2e41d51f17780e1b0d` (`gh api repos/taiki-e/install-action/git/ref/tags/cargo-auditable`), and the action's `tool:` input accepts `cargo-auditable@0.7.6` (latest release `v0.7.6`). Prefer it over `cargo install cargo-auditable --locked` (which compiles from source, minutes on a 4 vCPU runner); the `cargo install` form is the fallback if the prebuilt download fails.
- `actions/download-artifact` v4 tag `v4.3.0` resolves to commit `d3f86a106a0bac45b974a628896c90dbdf5c8093` (`gh api repos/actions/download-artifact/git/ref/tags/v4.3.0`; the `v4` moving tag points at the same commit). The repo already pins `actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4.6.2`. Both are `actions/*`, allowed by `github_owned_allowed`. Artifacts uploaded by a job of a called reusable workflow belong to the caller's workflow run, so `release.yml`'s `release` job can download what `ci.yml`'s job uploaded in the same run.
- PDB. Rust's MSVC linker driver passes `/DEBUG` unconditionally (`compiler/rustc_codegen_ssa/src/back/linker.rs`, `MsvcLinker::debuginfo`: "This will cause the Microsoft linker to generate a PDB file from the CodeView line tables in the object files"), so `cargo build --release` on MSVC always writes a `.pdb` even with the workspace's `[profile.release] debug = false` (which is why `.gitignore` already lists `*.pdb`). With `debug = false` that PDB holds only public symbols and no line tables. Cargo names the file after the crate name with underscores, `target/x86_64-pc-windows-msvc/release/miru_agent.pdb`, next to `miru-agent.exe` (`src/cargo/core/compiler/build_context/target_info.rs`: `.pdb` uses `should_replace_hyphens: true`, with the comment that debuggers "will look in the same directory of the exe with the original pdb filename"). To make the PDB useful for backtraces with file and line numbers, the Windows job sets the job-level environment variable `CARGO_PROFILE_RELEASE_DEBUG: 1` (Cargo config reference: every `[profile.<name>]` key is settable as `CARGO_PROFILE_<NAME>_<KEY>`; `1`/`limited` is "debug info without type or variable-level information", https://doc.rust-lang.org/cargo/reference/profiles.html#debug). On MSVC `split-debuginfo` defaults to `packed`, i.e. the debug information goes into the `.pdb`, not the `.exe`, so the shipped executable is unchanged apart from its existing debug-directory entry. This is job-scoped YAML; `Cargo.toml` is not edited and the Linux lane is unaffected. `windows-check` already uses the same mechanism (`CARGO_PROFILE_DEV_DEBUG: 0`).
- GoReleaser `release.extra_files` (https://goreleaser.com/customization/release/) attaches pre-existing files to the GitHub release: `glob` (templated) and optional `name_template` (templated too, `internal/extrafiles/extra_files.go` applies `tmpl` to both; `name_template` requires the glob to match exactly one file). Extra files are processed by the publish stage only, so a `--snapshot` run does not exercise them.
- `goreleaser release --snapshot` "Generate[s] an unversioned snapshot release, skipping all validations and without publishing any artifacts (implies --skip=announce,publish,validate)" (`goreleaser release --help`, v2.18.1 locally; the same flag exists in the builder image's 2.13.3 and is what `build/build.sh` already uses). `goreleaser check -f build/.goreleaser.yaml` validates the config locally and, with `GORELEASER_KEY` set, reports Pro features correctly.
- Runner. `blacksmith-4vcpu-windows-2025` is where `windows-check` runs (warm compile about 42 s, cold about 2 min measured for the test build); a plain `cargo build` needs no interactive session, so the release build uses the same runner. `windows-package` stays on `windows-latest` and is not touched.
- The executable prints its version with `miru-agent version` (`agent/src/cli/mod.rs`), which the Windows job uses as a smoke test that the built file runs.

Terms. "Prebuilt builder": the GoReleaser build type that copies an externally built file into the pipeline. "Artifact" (GitHub Actions sense): a file set uploaded from one job and downloadable by another job of the same workflow run. "PDB": Program Database, the MSVC debug-symbol file a debugger loads next to an `.exe`. "SBOM": Software Bill of Materials, here an SPDX JSON document produced by syft.

## Plan of Work


All paths are relative to `/home/ben/miru/workbench4/repos/agent`. One commit per milestone.

M1, Windows build job. In `.github/workflows/ci.yml`, extend the `windows_package_scope` job's filter with a second key and output so the new job runs on PRs only when release-relevant files change, and always for `push` and `workflow_call`:

    windows_package_scope:
      name: windows-package-scope
      ...
      outputs:
        windows_package: ${{ github.event_name != 'pull_request' || steps.filter.outputs.package == 'true' }}
        windows_release: ${{ github.event_name != 'pull_request' || steps.filter.outputs.release == 'true' }}
      steps:
        - name: Classify changes
          ...
          with:
            filters: |
              package:
                - 'build/windows/**'
                - '.github/workflows/ci.yml'
                - '.github/workflows/release.yml'
              release:
                - 'build/**'
                - '.github/workflows/ci.yml'
                - '.github/workflows/release.yml'
                - 'Cargo.toml'
                - 'Cargo.lock'

Add the job after `windows-package`:

    windows-release-build:
      needs: windows_package_scope
      if: needs.windows_package_scope.outputs.windows_release == 'true'
      runs-on: blacksmith-4vcpu-windows-2025
      timeout-minutes: 45
      env:
        # Line/module-level debug info so the shipped PDB supports symbolized
        # backtraces. On MSVC debug info is "packed" into the .pdb, so the .exe
        # is unchanged. Job-scoped on purpose: the Linux lane keeps
        # [profile.release] debug = false from Cargo.toml.
        CARGO_PROFILE_RELEASE_DEBUG: 1
      steps:
        - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1

        - name: Install Rust Toolchain
          uses: dtolnay/rust-toolchain@631a55b12751854ce901bb631d5902ceb48146f7 # stable
          with:
            targets: x86_64-pc-windows-msvc

        - name: Cache Rust dependencies
          uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2
          with:
            key: release-msvc
            cache-on-failure: true

        - name: Install NASM (aws-lc-sys)
          run: |
            choco install nasm -y
            echo "C:\Program Files\NASM" >> $env:GITHUB_PATH

        - name: Install cargo-auditable
          uses: taiki-e/install-action@47345a21ef01b2d0e9b59e2e41d51f17780e1b0d # cargo-auditable
          with:
            tool: cargo-auditable@0.7.6

        - name: Build Windows Release (cargo auditable)
          run: cargo auditable build --release --target x86_64-pc-windows-msvc -p miru-agent --locked

        - name: Stage prebuilt artifacts
          shell: pwsh
          run: |
            $out = "build\prebuilt\windows_amd64"
            New-Item -ItemType Directory -Force $out | Out-Null
            Copy-Item target\x86_64-pc-windows-msvc\release\miru-agent.exe $out
            Copy-Item target\x86_64-pc-windows-msvc\release\miru_agent.pdb $out
            Get-ChildItem $out
            & "$out\miru-agent.exe" version

        - name: Upload prebuilt artifacts
          uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4.6.2
          with:
            name: agent-windows-amd64-msvc
            path: build/prebuilt/windows_amd64/
            if-no-files-found: error

The artifact contains exactly two files at its root, `miru-agent.exe` and `miru_agent.pdb`; the file names carry no version so the Windows job needs no knowledge of the tag. Add `build/prebuilt/` to `.gitignore` (the directory only ever exists on a runner or during a local dry run). If the copy of `miru_agent.pdb` fails because Cargo named it differently on the runner's toolchain, the `Get-ChildItem` output of `target\x86_64-pc-windows-msvc\release` in the log shows the actual name; adjust the copy and record it in Surprises. If `taiki-e/install-action` cannot download cargo-auditable, replace that step with `run: cargo install cargo-auditable --locked --version 0.7.6` and record the timing.

M2, GoReleaser config. In `build/.goreleaser.yaml`:

1. After the `agent` build entry add:

        - id: "agent-windows"
          # Built natively on a Windows runner (aws-lc-sys cannot cross-compile
          # from Linux) through `cargo auditable`, so the PE also carries a
          # `.dep-v0` section. The workflow downloads the runner's artifact into
          # build/prebuilt/windows_amd64/ before GoReleaser runs; the prebuilt
          # builder fails loudly if the file is missing.
          builder: prebuilt
          goos: [windows]
          goarch: [amd64]
          prebuilt:
            path: prebuilt/{{ .Os }}_{{ .Arch }}/miru-agent.exe
          binary: miru-agent

   `prebuilt.path` is relative to GoReleaser's working directory, which is `build/` (the Dockerfile runs `cd build && goreleaser ...`), so it resolves to `build/prebuilt/windows_amd64/miru-agent.exe`.

2. In `archives`, change `ids: ["agent"]` to `ids: ["agent", "agent-windows"]`. The existing `format_overrides` (`goos: windows` -> `zip`) and the name template then yield `agent_Windows_x86_64.zip`.

3. In `nfpms`, add `ids: ["agent"]` directly under `id: agent` so the `.deb` packaging is explicitly limited to the Linux build and can never pick up the Windows binary.

4. Under `release`, add:

        extra_files:
          # Debug symbols for the Windows executable, attached as their own asset
          # so the zip stays a clean install artifact. The file keeps the name
          # recorded in miru-agent.exe's debug directory (Cargo writes
          # miru_agent.pdb) because debuggers look for exactly that name next
          # to the executable.
          - glob: ./prebuilt/windows_amd64/miru_agent.pdb

5. Update the comment above `sboms` to say the binary scan also covers the Windows PE (`.dep-v0` is read by syft's cargo-auditable cataloger for ELF, PE and Mach-O). Leave the `sboms` stanza itself unchanged; the `binary` SBOM will be produced for all three binaries and the `archive` SBOM for all three archives.

Run `goreleaser check -f build/.goreleaser.yaml` locally (M2 step below) before committing.

M3, dry run on the PR. Add to `ci.yml`, after `windows-release-build`:

    goreleaser-snapshot:
      name: goreleaser-snapshot
      needs: [windows_package_scope, windows-release-build]
      # Pull requests only: on tags the real release job in release.yml consumes
      # the same artifact. Same-repo PRs can read GORELEASER_KEY; forks cannot.
      if: github.event_name == 'pull_request' && needs.windows_package_scope.outputs.windows_release == 'true'
      runs-on: blacksmith-4vcpu-ubuntu-2404
      timeout-minutes: 30
      steps:
        - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
          with:
            fetch-depth: 0

        - name: Download Windows prebuilt artifacts
          uses: actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093 # v4.3.0
          with:
            name: agent-windows-amd64-msvc
            path: build/prebuilt/windows_amd64

        - name: GoReleaser snapshot (no publish)
          run: ./build/build.sh
          env:
            GORELEASER_KEY: ${{ secrets.GORELEASER_KEY }}

        - name: Verify Windows artifacts
          run: |
            set -euxo pipefail
            ls -l build/dist
            test -f build/dist/agent_Windows_x86_64.zip
            unzip -l build/dist/agent_Windows_x86_64.zip | grep -E ' miru-agent\.exe$'
            ls build/dist/miru-agent_*_windows_amd64.sbom.json
            grep -c '"name": "tokio"' build/dist/miru-agent_*_windows_amd64.sbom.json
            test -f build/dist/agent_Windows_x86_64.zip.sbom.json
            ! ls build/dist/*.deb | grep -i windows
            ls -l build/prebuilt/windows_amd64/miru_agent.pdb

        - name: Upload snapshot dist
          uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4.6.2
          with:
            name: goreleaser-snapshot-dist
            path: build/dist/
            if-no-files-found: error

`build/build.sh` is the existing local snapshot entry point (Docker, `GORELEASER_ARGS="--snapshot"`), so the PR exercises the exact container and GoReleaser version the tag release will use; `--snapshot` implies `--skip=publish`, so nothing reaches GitHub. Docker is available on the Blacksmith Ubuntu runners (`builder.yml` already builds there) and the builder image pull needs no login (the `release` job pulls it today with no login step). The `grep -c '"name": "tokio"'` line is the SBOM proof: it is non-zero only if syft read the `.dep-v0` section of the PE. If that check fails while the ELF SBOMs are fine, do not guess: add `ids: ["agent"]` to the `binary` SBOM entry so the Windows binary SBOM is skipped, keep the archive SBOM only if `agent_Windows_x86_64.zip.sbom.json` lists crates, drop the failing assertion(s), and record the syft output in the Decision Log. The `.deb` check guards the `nfpms` filter. The zip-listing check fails if GoReleaser names the entry `miru-agent` without `.exe`; in that case inspect the listing, adjust `binary` accordingly, and record the observed behaviour.

M4, wire the tag release. In `.github/workflows/release.yml`, in the `release` job, insert between "Checkout Code" and "Release":

      - name: Download Windows prebuilt artifacts
        uses: actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093 # v4.3.0
        with:
          name: agent-windows-amd64-msvc
          path: build/prebuilt/windows_amd64

      - name: Show Windows prebuilt artifacts
        run: ls -l build/prebuilt/windows_amd64

No `needs:` change is required: `release` already `needs: [ci, check-main]`, `windows-release-build` is a job of the called `ci` workflow (its filter output is `true` for `workflow_call`), and `ci` itself only runs when `on_main == 'true'`. Because the `release` job needs the whole `ci` workflow, a red or missing Windows build blocks the release, and a missing artifact makes GoReleaser fail at the prebuilt `stat` (strict failure, no silent Linux-only release). Also add a comment in `release.yml` next to the `permissions` block noting that the Windows artifact is produced by `ci.yml`'s `windows-release-build` job.

M5, docs. In `build/windows/README.md`, "Validation" section, add one sentence that pull requests touching `build/**` or the workflows also run `windows-release-build` and `goreleaser-snapshot`, and in the last paragraph remove "the GoReleaser/PDB release lane" and "artifact ... publication" from the deferred list (keep Authenticode, WinGet, service lifecycle, account and recovery handling, live-backend provisioning, Windows Server certification). In `plans/active/20260910-windows-support.md`, append to the PR 8 paragraph: "(implemented by this plan's PR; `plans/backlog/20260916-windows-release-lane.md`)" and note the PDB is attached as a separate release asset.

## Concrete Steps


All commands run from `/home/ben/miru/workbench4/repos/agent`. Heavy validation runs only in GitHub Actions; locally: edit, `goreleaser check`, `git diff --check`, commit, push.

Setup once:

    git fetch origin main
    git checkout feat/windows-release-lane
    git log --oneline origin/main..HEAD
    gh pr create --draft --base main --title "build(windows): add msvc release lane via GoReleaser prebuilt build" \
      --body "See plans/backlog/20260916-windows-release-lane.md"

M1:

    $EDITOR .github/workflows/ci.yml .gitignore
    git diff --check
    git add .github/workflows/ci.yml .gitignore
    git commit -m "ci(windows): build msvc release binary and pdb with cargo-auditable"
    git push

M2:

    $EDITOR build/.goreleaser.yaml
    goreleaser check -f build/.goreleaser.yaml
    # expected (tail): "1 configuration file(s) validated" and "thanks for using GoReleaser Pro!"
    # (needs GORELEASER_KEY in the environment; without it the check reports the
    # Pro-only prebuilt builder as an error, which is not a config defect)
    git add build/.goreleaser.yaml
    git commit -m "build(goreleaser): ingest windows msvc prebuilt binary and attach pdb"
    git push

M3:

    $EDITOR .github/workflows/ci.yml
    git diff --check
    git add .github/workflows/ci.yml
    git commit -m "ci(release): dry-run goreleaser snapshot with the windows prebuilt on pull requests"
    git push

M4:

    $EDITOR .github/workflows/release.yml
    git add .github/workflows/release.yml
    git commit -m "ci(release): download windows prebuilt artifacts before goreleaser"
    git push

M5:

    $EDITOR build/windows/README.md plans/active/20260910-windows-support.md
    git add build/windows/README.md plans/active/20260910-windows-support.md
    git commit -m "docs(windows): record the goreleaser release lane"
    git push

After each push:

    gh run list --branch feat/windows-release-lane --workflow CI --limit 1 --json databaseId,url,status
    gh run watch <RUN_ID> --exit-status
    gh run view <RUN_ID> --json jobs -q '.jobs[] | "\(.name) \(.conclusion)"'

Expected job list once M3 is in: `lint`, `windows-check`, `windows-package-scope`, `windows-package`, `windows-release-build`, `goreleaser-snapshot`, `test`, `tools`, all `success`. To read a job's log:

    gh run view <RUN_ID> --job <JOB_ID> --log | sed 's/\x1b\[[0-9;]*m//g' | grep -E 'miru-agent|miru_agent|Finished|sbom|zip'

## Validation and Acceptance


CI is the test. On the PR's pushed head:

1. `windows-release-build` is green. Its "Stage prebuilt artifacts" step log lists `miru-agent.exe` and `miru_agent.pdb` (PDB size in the tens of MB with `CARGO_PROFILE_RELEASE_DEBUG: 1`, versus low MB without it) and prints the version string from `miru-agent.exe version`. The "Build Windows Release" step's Cargo invocation line shows `cargo auditable build`. The run's artifacts page lists `agent-windows-amd64-msvc`.
2. `goreleaser-snapshot` is green. Its "Verify Windows artifacts" step shows `agent_Windows_x86_64.zip` in `build/dist`, the `unzip -l` line ending in `miru-agent.exe`, a non-zero count from the `tokio` grep on `miru-agent_<version>_windows_amd64.sbom.json` (proving syft read the PE's `.dep-v0` section), the presence of `agent_Windows_x86_64.zip.sbom.json`, and no Windows `.deb`. The `goreleaser-snapshot-dist` artifact can be downloaded (`gh run download <RUN_ID> -n goreleaser-snapshot-dist`) and inspected with `unzip -l agent_Windows_x86_64.zip`.
3. `windows-check`, `windows-package`, `lint`, `test`, `tools` are green and unchanged.
4. `goreleaser check -f build/.goreleaser.yaml` passes locally (M2 step).
5. `release.yml` is not executable on the PR (tags only); the reviewer verifies M4 by reading the diff: the download step names the same artifact (`agent-windows-amd64-msvc`) and path (`build/prebuilt/windows_amd64`) as the dry-run job, and `release` still `needs: [ci, check-main]`.

Preflight (the `$preflight` workflow: CI on the pushed branch head, draft PR) must report exactly `CLEAN` before the PR leaves draft or the task is reported complete; a later commit needs its own green run.

Post-merge proof of the publish-only pieces (`extra_files`), outside this PR but part of the roadmap item: the first tag after merge (a `v0.10.x-beta.N` prerelease tag is fine; `release.prerelease: auto` marks it) must show `agent_Windows_x86_64.zip`, `agent_Windows_x86_64.zip.sbom.json`, `miru-agent_<version>_windows_amd64.sbom.json` and `miru_agent.pdb` among the release assets (`gh release view <tag> --json assets -q '.assets[].name'`), and the `release` job log must show the "Show Windows prebuilt artifacts" listing before GoReleaser starts. If `miru_agent.pdb` is missing, the `extra_files` glob is wrong relative to `build/`; fix and re-tag a beta.

## Idempotence and Recovery


Every milestone is one commit of YAML or Markdown; repeating a push re-runs the same checks, and reverting is `git revert <sha>`. `build/prebuilt/` is gitignored and recreated by each run; delete it locally if a local `./build/build.sh` complains about a stale file. The rust-cache key `release-msvc` is new, so the first `windows-release-build` run is cold (expect several minutes); measure from the second run. The `goreleaser-snapshot` job only runs on PRs, so a mistake in it can never publish anything. If a tag release ever fails at the prebuilt `stat`, the fix is to make the Windows artifact present (re-run the failed workflow from the failed job), never to remove the `agent-windows` build; the Linux-only release must not silently ship. Nothing in this plan touches production code, so no rollback beyond the workflow and config files is ever needed.
