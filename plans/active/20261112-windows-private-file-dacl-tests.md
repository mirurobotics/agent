# Test the Windows ACL on the device API discovery file

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

> **Design settled after review (2026-11-12).** The tests moved to `agent/tests/filesys/files.rs` as Windows siblings of `honors_mode_atomic` (`inherits_folder_dacl_atomic`, `replace_drops_old_dacl_atomic`), and the helpers shrank to one, `assert_dacl_matches_new_file`, which compares a file's DACL with a control file's in the default temp dir. `make_private`, `current_user_sid`, the SID checks and the `device_api.rs` tests are gone, and so are the per-resource follow-ups. See the last two Decision Log entries; sections below that describe the old design are superseded by them.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (mirurobotics/agent) | read-write | Two Windows-only test helpers in `agent/tests/test_utils/filesys/access.rs` and two Windows-only tests in `agent/tests/disk/device_api.rs`. Test-only; no production change. |

This plan lives in `agent/plans/` because all edits are in the agent repo. Branch: `test/windows-private-file-access`, based on `main` at `e9f2c46`.

## Purpose / Big Picture

On Unix the agent creates its private files with an explicit mode, and Unix-only tests check those modes. On Windows the mode is dropped, so a written file is protected only by the ACL (access control list) it inherits from its folder, which the installer locks down. The installer's own tests prove that a file created directly in those folders is locked down. Nothing proves that the agent's write path, an atomic write through a temp file that is then renamed into place, ends up with the same protection.

After this change, the `windows-check` CI job fails if the device API discovery file the agent writes, first written or replacing an older file, carries any ACL other than exactly what its folder gives a new file. This test is the exemplar for the pattern. The private key and other private files follow in their own PRs once it is approved.

## Decisions

### Names

| New name | Alternatives | Why this one |
|----------|--------------|--------------|
| `make_private` (fn, `test_utils::filesys::access`) | `protect_dir`, `make_agent_private` | Matches the `make_unreadable` / `make_read_only` verbs in the same file and says what state the folder ends up in. `protect` sounds like the installer's action, and "agent private" names an installer descriptor the reader may not know. |
| `assert_inherits_private_dacl` (fn, same module) | `assert_inherits_only`, `assert_dacl_matches_new_file` | Says both checks: every entry is inherited, and the entries are the private set `make_private` grants. `assert_inherits_only` leaves out which grants it expects. No `assert_*` helper exists under `agent/tests/test_utils/` yet, so there is no clash. |
| `inherits_private_dir_dacl` (test, `disk::device_api::write`) | `sets_private_dacl`, `dacl_is_inherited` | Follows the sibling `sets_mode_0640` in saying what the file ends up with. The agent sets nothing on Windows, so a `sets_*` name would mislead. |
| `replace_drops_old_dacl` (test, same module) | `replace_takes_dir_dacl`, `replaces_existing_dacl` | Names what can go wrong on replace: the old file's ACL is carried over. Follows the sibling `replaces_existing_file_atomically`. |

Private helpers inside the Windows `os` module (`dacl`, `current_user_sid`, `icacls_ok`) are not public names.

### Precedent

No new source files. The helpers go into `agent/tests/test_utils/filesys/access.rs` and copy `make_unreadable` / `make_read_only` (PRs #472, #474, #475): a documented public function at the top of the file that calls into the per-OS `os` module, with the Windows side shelling out to `icacls`. The new public functions are gated `#[cfg(windows)]` because they mean nothing on Unix. The tests sit in `agent/tests/disk/device_api.rs` module `write`, right after their Unix sibling `sets_mode_0640`, and copy its shape: a temp dir from `test_dirs::temp`, one `device_api::write`, then one check. That file already has a `#[cfg(windows)]` module (`windows_sharing`), so Windows-only tests in it are not new.

### Left out

- Private key test in `agent/tests/crypt/rsa.rs` (Windows sibling of `file_permissions`): this is the second resource for the same pattern. Under the pattern-first rule it follows once this exemplar is approved, and it will be one test calling the same two helpers.
- Other private files (public key, `configs`, the token and device files in `Agent\auth`): follow-ups after the exemplar.
- Production change that sets an explicit ACL on Windows when `WriteOptions.mode` is set: a follow-up. This PR only checks today's behavior.
- A stand-in read entry for `Miru Agent Users` on the test `device-api` folder: the control-file comparison already proves the write path passes on every inheritable entry the folder has. A reader entry would only be worth adding with an assertion on it, and the installer integration tests already cover that.
- Non-atomic write path (`write_bytes_direct`): the discovery file uses the atomic path, and a direct create inherits the same way the control file does.
- Restoring the folder's old ACL: `make_private` is used only on temp dirs, which are deleted when the test ends, so it returns no guard.
- Changes to `Cargo.toml`: `std::process::Command` and `tempfile` (already a dependency) are enough.

### Open questions

- Should the private key test ship in this PR too? The task brief listed it, but the pattern-first rule says to land one exemplar first. Default: discovery file only, private key as the first follow-up.
- Should the DACL be read with `icacls /save` or with PowerShell `(Get-Acl).Sddl`? Default: `icacls /save`, the only tool the file already uses, and it returns just the DACL. `Get-Acl` adds a second tool and a slow PowerShell start, and its `.Sddl` includes the owner and group.

## Progress

- [x] Milestone 1: helpers in `access.rs`, tests in `device_api.rs`, one commit.
- [x] Source written
- [ ] Push, open a draft PR, and run `$preflight` until it reports `CLEAN`.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- Decision: Test only the discovery file in this PR. The private key test is a follow-up.
  Rationale: The pattern-first rule (one exemplar, then siblings) applies, since the same Windows ACL check repeats for each private file the agent writes. The discovery file is the more representative exemplar: the agent rewrites it on every start, so it covers both a first write and a replace, and the replace case is where carrying over the wrong ACL could happen. The private key copy is one test calling the same helpers.
  Date/Author: 2026-11-12, plan author.
- Decision: Compare the written file's DACL with a control file created directly (`std::fs::write`) in the same folder, and check the trustees (the accounts the entries name) and the inherited flag, not exact rights masks.
  Rationale: The rights strings Windows writes can't be checked without a Windows machine. A control file shows exactly what the folder gives a new file, which is the property under test.
  Date/Author: 2026-11-12, plan author.
- Decision: Read the DACL with `icacls <path> /save <file>` and parse line 2 (SDDL, UTF-16LE). PowerShell `Get-Acl` was considered and rejected.
  Rationale: `icacls` is already the file's tool, the output holds only the DACL, and the decoding needs only `std`.
  Date/Author: 2026-11-12, plan author.
- Decision: Stand in for the service account with this process's user, whose SID comes from `whoami /user /fo csv /nh`.
  Rationale: A SID is exact and independent of locale and domain. Account names passed to `icacls` are neither.
  Date/Author: 2026-11-12, plan author.
- Decision: `icacls_ok` returns nothing instead of `Output`, and its failure message names the arguments (`icacls [..] on <path>`) instead of the fixed `/deny`.
  Rationale: No caller reads the output after the success check, and the helper now runs `/deny`, `/inheritance:r /grant:r` and `/save`, so a fixed `/deny` in the message would mislead.
  Date/Author: 2026-11-12, implementer.

- Decision: Test the atomic write path in `filesys` instead of the discovery file, and drop the per-resource follow-ups (private key, public key, configs, auth files).
  Rationale: Design review (AI review D10 on the PR, and `$design-review`): on Windows the mode is dropped, so the only behavior under test is `files::write_bytes` with `Atomic::Yes`, which every private-file writer (discovery file, keys) uses. One pair of tests next to `honors_mode_atomic` covers every caller. The non-atomic path keeps an existing file's DACL on truncate; no private file uses it, so it gets no test.
  Date/Author: 2026-11-12, author settling the design review.
- Decision: Drop `make_private`, `current_user_sid` and the SDDL/SID checks; keep one helper, `assert_dacl_matches_new_file`, run in the default temp dir.
  Rationale: Design review (AI review D2, `$design-review` finding 2): the control-file comparison is the property the agent owns (its write adds, drops and carries over no ACL entry). The SID check only re-checked the fixture `make_private` had set, and whether a new file in the installer's folders is private is owned by the installer integration tests (`Assert-InheritedProtection` and the non-admin probe). A non-empty check stops the comparison passing on two empty DACLs.
  Date/Author: 2026-11-12, author settling the design review.
- Decision: Give the test folder an explicit inheritable read entry for Users (`add_inheritable_entry`) before writing.
  Rationale: AI review on the PR: a plain temp dir passes on the same DACL as `%TEMP%`, so without a distinct entry the test would still pass if the write staged its temp file in another folder and moved it in.
  Date/Author: 2026-11-12, author.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

The agent is a Rust workspace. The binary package is `agent/`, its library is `miru_agent`, and its integration tests live under `agent/tests/`, mirroring `agent/src/`.

Write path. `agent/src/filesys/files.rs` `write_bytes` creates the parent folder if it is missing, then calls `write_bytes_atomic` when `WriteOptions.atomic` is `Atomic::Yes`. That function uses the `atomicwrites` 0.4.4 crate. `AtomicFile::write_with_options` creates a temp subfolder named `.atomicwrite*` inside the target's parent folder, writes `tmpfile.tmp` there, and moves it onto the target with `MoveFileExW`, which replaces the target when overwriting is allowed. On Windows a move within one volume keeps the moved file's own security descriptor. It does not inherit again from the new folder. So the final file carries the ACL it got inside the temp subfolder. That ACL matches a direct child of the parent only if the parent's inheritable entries reach a grandchild unchanged. A replace never keeps the old file's ACL. On Windows, `mode_open_options` and `apply_mode` in the same file drop `WriteOptions.mode`, so no explicit ACL is set.

Discovery file. `agent/src/disk/device_api.rs` `write(file, port, token)` writes `{"port":…,"token":…}` atomically with `Overwrite::Allow` and `mode: Some(0o640)`. The agent rewrites it on every start. Its tests are in `agent/tests/disk/device_api.rs`. There, `discovery_file(&dir)` returns `<dir>/device-api/device-api.json`, the Unix test `sets_mode_0640` checks the mode, and a `#[cfg(windows)] pub mod windows_sharing` already holds Windows-only tests.

Installer. `build/windows/miru-agent.wxs` gives the `device-api` folder the protected descriptor `AgentPrivateSddl` = `O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;;FRFWFX;;;<service>)(A;OICIIO;FA;;;<service>)`, plus a `Miru Agent Users` read entry. In SDDL (the Windows text form of a security descriptor), `D:` starts the DACL (discretionary ACL: the allow and deny entries), `P` marks it protected (inherits nothing), each `(type;flags;rights;;;sid)` is an ACE (access control entry), `OI`/`CI` pass the entry on to files and folders, `IO` applies it only to children, `ID` marks an entry as inherited, `FA` is full access, `SY` is SYSTEM (`S-1-5-18`), and `BA` is Administrators (`S-1-5-32-544`). `build/windows/tests/integration-lib.ps1` `Assert-InheritedProtection` checks that a file created directly in each folder has only inherited ACEs for the trusted SIDs.

Test helpers. `agent/tests/test_utils/filesys/access.rs` holds `make_unreadable(&File)` and `make_read_only(&Dir)`. Each is a documented portable function that builds a `Denied` guard in a per-OS `mod os`. The Windows `os` adds deny ACEs with `icacls` (`fn icacls(path, args) -> io::Result<Output>`) and removes them on drop. This file is also mounted into the library's unit-test build through `agent/tests/test_utils/unit.rs`, so it may reach sibling fixtures only through `super::` (for example `super::super::dirs` from inside `mod os`), never `crate::`. `agent/tests/test_utils/filesys/dirs.rs` `temp(prefix)` returns a `TempDir` that is deleted on drop. Tests that use it don't need `#[serial]`.

CI. `.github/workflows/ci.yml` job `windows-check` runs `cargo test --package miru-agent --locked` on a Windows runner for every pull request to `main`. It is the only place these tests run. The Linux jobs `lint` and `test` compile without them, because everything new is `#[cfg(windows)]`.

## Plan of Work

All edits are in the agent repo.

In `agent/tests/test_utils/filesys/access.rs`, add two public functions after `make_read_only` and before `enum Access`, each gated `#[cfg(windows)]` and with a doc comment in the style of its neighbors:

    /// Replaces `dir`'s ACL with a protected one like the installer's private
    /// folders: full control for SYSTEM, Administrators and this process's user,
    /// passed on to new entries, with nothing inherited. Use it on a temp dir,
    /// since nothing restores the old ACL.
    #[cfg(windows)]
    pub fn make_private(dir: &Dir) {
        os::make_private(dir.path())
    }

    /// Asserts `file` has exactly the ACL a file created directly in its folder
    /// gets, and that the ACL allows only SYSTEM, Administrators and this
    /// process's user, all through inherited entries. Pair it with
    /// [`make_private`].
    #[cfg(windows)]
    pub fn assert_inherits_private_dacl(file: &File) {
        os::assert_inherits_private_dacl(file.path())
    }

In the Windows `mod os` of the same file, add these after `impl Drop for Denied`, in call order:

- Constants `SYSTEM: &str = "*S-1-5-18"` and `ADMINISTRATORS: &str = "*S-1-5-32-544"`, next to `EVERYONE`.
- `pub(super) fn make_private(dir: &Path)` runs `icacls <dir> /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F *<user SID>:(OI)(CI)F` through `icacls_ok`.
- `pub(super) fn assert_inherits_private_dacl(file: &Path)` does four things. It writes an empty control file `dacl-control` next to `file` with `std::fs::write`, reads its DACL with `dacl`, and removes it. It asserts that `dacl(file)` equals the control's DACL, with a message naming the file. It splits the DACL after the `D:` flags into ACEs (strip the outer parentheses, split on `)(`, split each ACE on `;`) and asserts each has type `A` and flags containing `ID`. And it asserts that the sorted, deduplicated SIDs (field 6) equal the sorted, deduplicated `["SY", "BA", user]`, where `user` is `"SY"` when the SID is `S-1-5-18` and the raw SID otherwise. SDDL abbreviates only well-known SIDs.
- `fn dacl(path: &Path) -> String` makes a temp dir with `super::super::dirs::temp("dacl")`, runs `icacls <path> /save <tmp>\dacl.txt` through `icacls_ok`, reads the bytes, decodes them as UTF-16LE (`chunks_exact(2)`, `u16::from_le_bytes`, `String::from_utf16`), strips a leading `\u{feff}` if one is there, and returns line 2 trimmed. That line is the SDDL, starting with `D:`.
- `fn current_user_sid() -> String` runs `whoami /user /fo csv /nh`, asserts success, and returns the last comma-separated field of stdout with the quotes and whitespace trimmed. The output looks like `"host\runneradmin","S-1-5-21-…"`.
- `fn icacls_ok<S: AsRef<OsStr>>(path: &Path, args: &[S]) -> Output` calls `icacls` and asserts success with the same message `Denied::new` builds today. Then change `Denied::new` to call `icacls_ok` instead of repeating that assert. `Drop` keeps calling `icacls` and ignoring the result.

Add the imports the new code needs inside `mod os` (`miru_agent::filesys::PathExt` for `TempDir::path`), keeping the `// standard crates` / `// internal crates` grouping.

In `agent/tests/disk/device_api.rs` module `write`, add two tests right after `sets_mode_0640`. Each is gated `#[cfg(windows)]`, with the imports inside the function as `sets_mode_0640` does:

    #[cfg(windows)]
    #[tokio::test]
    async fn inherits_private_dir_dacl() {
        use crate::test_utils::filesys::access::{assert_inherits_private_dacl, make_private};

        let dir = test_dirs::temp("discovery_inherits_private_dir_dacl").unwrap();
        let file = discovery_file(&dir);
        let api_dir = file.parent().unwrap();
        std::fs::create_dir(api_dir.path()).unwrap();
        make_private(&api_dir);

        let token = SecretString::from("test-token");
        device_api::write(&file, 6478, &token).await.unwrap();
        assert_inherits_private_dacl(&file);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn replace_drops_old_dacl() {
        use crate::test_utils::filesys::access::{
            assert_inherits_private_dacl, make_private, make_unreadable,
        };

        let dir = test_dirs::temp("discovery_replace_drops_old_dacl").unwrap();
        let file = discovery_file(&dir);
        let api_dir = file.parent().unwrap();
        std::fs::create_dir(api_dir.path()).unwrap();
        make_private(&api_dir);
        let old = SecretString::from("old-token");
        device_api::write(&file, 1234, &old).await.unwrap();
        // an explicit entry on the old file that the replacement must not keep
        let _unreadable = make_unreadable(&file);

        let token = SecretString::from("test-token");
        device_api::write(&file, 6478, &token).await.unwrap();
        assert_inherits_private_dacl(&file);
    }

`make_unreadable` adds a deny-read ACE for Everyone. The replace still succeeds, because it needs delete access, which the folder's inherited full control gives. The guard's drop then removes deny entries from the new file, which has none, so that does nothing. Neither test needs `#[serial]`, because each uses its own temp dir.

## Concrete Steps

From the agent repo root (`/home/user/agent`):

1. Confirm the branch: `git status` shows `On branch test/windows-private-file-access`.
2. Edit `agent/tests/test_utils/filesys/access.rs` and `agent/tests/disk/device_api.rs` as described in Plan of Work.
3. Static check only, with no build or test run: `git diff` shows the new code only under `#[cfg(windows)]` (apart from the `Denied::new` switch to `icacls_ok`), imports keep their group comments, and `grep -n "make_private\|assert_inherits_private_dacl" -r agent/tests` lists the definitions and the two tests.
4. Commit milestone 1:

       git add agent/tests/test_utils/filesys/access.rs agent/tests/disk/device_api.rs
       git commit -m "test(disk): check the Windows ACL on the device API discovery file"

5. Push the branch, open a draft PR, and run `$preflight`. If `windows-check` fails, read `gh run view <run-id> --log-failed`, fix the helper (the likely spots are listed under Idempotence and Recovery), commit, and push again.

## Validation and Acceptance

CI is the only validation. Nothing is built, linted or tested locally. The work is done only when `$preflight` reports `CLEAN`, meaning CI is green on the pushed branch head. The PR must not leave draft, and the task must not be reported complete, before that.

In `.github/workflows/ci.yml`, job `windows-check` (step "Run Windows Tests", `cargo test --package miru-agent --locked`) must pass on the pushed head and must list these tests as passed:

- `disk::device_api::write::inherits_private_dir_dacl`: in a folder with a protected ACL, a discovery file written by `device_api::write` has only inherited ACEs, granting only SYSTEM, Administrators and the current user, identical to a file created directly in that folder.
- `disk::device_api::write::replace_drops_old_dacl`: when the existing discovery file carries an explicit deny entry, the file after `device_api::write` has none, and its DACL again equals the control file's.

Jobs `lint` and `test` (Linux) must also pass, showing that the Unix build and the existing `sets_mode_0640` test are unaffected.

To see that the tests can fail, a reviewer can temporarily change the second test to skip its second `write`. `replace_drops_old_dacl` then fails, because the file keeps its explicit deny ACE. Revert before merge.

## Idempotence and Recovery

The edits are confined to two test files and can be reapplied or reverted with `git checkout -- <file>`. The tests create everything under temp dirs that are deleted on drop, so reruns leave no state behind. Likely first-run failures, with their fixes:

- `icacls` rejects `/inheritance:r` and `/grant:r` together: split them into two `icacls_ok` calls, `/inheritance:r` first.
- The `/save` output has an unexpected line layout: find the SDDL as the first line starting with `D:` instead of taking line 2.
- The CI runner runs as SYSTEM: this is already handled by mapping `S-1-5-18` to `SY` and deduplicating.
- The two DACLs differ on CI: this is a real finding, not a test bug. Record the two SDDL strings under Surprises & Discoveries and raise it on the PR before changing any assertion.
