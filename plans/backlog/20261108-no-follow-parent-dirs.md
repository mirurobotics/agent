# Refuse symlinked parent directories after a scan

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Open, hash, upload and delete scanned files beneath their rule's glob root |

This plan lives in `agent/plans/` because all the code is in the agent repo.

## Purpose / Big Picture

Part of Linear ENG-1554 (stop the agent following symlinks). Two earlier PRs stop the scanner from walking into symlinks (#437) and open a scanned file with `O_NOFOLLOW` (#437, #438). `O_NOFOLLOW` only covers the last path component. After a scan, anyone who can write a watched directory can `rename(2)` a symlink such as `sub -> /var/log` over `<watched>/sub`. The agent would then hash, upload or delete `/var/log/<name>` (review thread on #438: https://github.com/mirurobotics/agent/pull/438#discussion_r4508086659).

After this change, on Unix, hashing (scanner and deleter), uploading and deleting a scanned file refuse a symlink at every path component below the rule's glob root, the directory the scanner walked from. The root itself may still be a symlink, because an admin chose it. With `sub` swapped for a symlink, the hash and upload fail with an open error and the delete fails and is retried, and nothing outside the watched tree is read or removed. Windows keeps its current behavior.

## Decisions

### Names

| New name | Alternatives | Why this one |
|----------|--------------|--------------|
| `files::glob_root` | `files::glob_prefix`, `FileRuleSource::root` | `glob`'s docs already call it the walk's start; a `files` function keeps the glob syntax in one module |
| `files::delete_beneath` | `files::unlink_at`, `files::delete_regular_file` | says how it differs from `files::delete`: it resolves the path beneath a root |
| `glob_root` field on the upload and delete `Job` | `root`, `scan_root` | matches `files::glob_root`, which computes it |
| `Job::root()` on both jobs | `open_root`, `glob_root_or_parent` | the directory every open and delete of the job starts from |
| `root: &Dir` parameter (`open_regular_file`, `sha256`, `sha256_and_crc32c`, `Source::open`, `ObjectTransfer` methods) | `beneath`, `prefix` | short; the docs say what it is |

### Precedent

No new files. `delete_beneath` mirrors `files::delete` (`NotFound` is success). The Unix and Windows halves of `open_beneath` and `delete_beneath` follow the `#[cfg(unix)]`/`#[cfg(windows)]` pairs already in `agent/src/filesys/files.rs` (`mode_open_options`). The `#[serde(default)] Option` field follows `retention` on the upload `Job`.

### Left out

- The scanner's and deleter's `files::metadata` stat still follows links: it only reads size and mtime, and the hash and unlink that act on the file are refused.
- Holding one handle from the deleter's identity check to its unlink: the unlink re-walks from the root, so a swap between them can only reach a file inside the watched tree, which the swapper could already write.
- `openat2(RESOLVE_BENEATH)`: Linux 5.6+ only; a per-component `openat` walk works on every Unix.
- Windows: no cheap `openat` equivalent; behavior unchanged, as #437 left it.

### Open questions

- Should a job queued by an older agent (no `glob_root`) be dropped instead of opened from its own directory? Default: open it from the file's own directory, which is exactly today's protection, so an upgrade loses no queued work.

## Progress

- [ ] Source written
- [ ] Tests written

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Thread the glob root through the jobs instead of canonicalizing scanned paths.
  Rationale: Canonical paths would change the `file_path` users see and the ledger keys, re-uploading files under a symlinked prefix after an upgrade. Upload and delete jobs are persisted and outlive rule changes, so they must carry the root themselves.
  Date/Author: 2026-11-08, Claude.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

All paths are relative to the agent repo root.

The scanner (`agent/src/data_uploads/scan/rule.rs`) lists a file rule's matches with `files::glob(&rule.source.glob)` (`agent/src/filesys/files.rs`). `glob` walks from the pattern's literal prefix (the directories before the first wildcard, computed by `parse_glob`) and never follows a symlink below it. When a candidate's stability window passes, `differs_from_previous` hashes it with `files::sha256`.

Stable files go to two sinks. `agent/src/data_uploads/upload/sink.rs` builds an upload `Job` (`agent/src/data_uploads/upload/job.rs`); `agent/src/data_uploads/retention/sink.rs` builds a delete `Job` (`agent/src/data_uploads/retention/job.rs`). Both jobs are persisted in queue snapshots, and released agents (v0.12.3) already wrote them, so a new field must deserialize from old snapshots. After a confirmed upload, `enqueue_delete_job` in `agent/src/data_uploads/upload/uploader_impl.rs` turns the upload job into a delete job.

Uploads: `LiveExecutor` (`agent/src/data_uploads/upload/executor.rs`) calls the `ObjectTransfer` methods in `agent/src/data_uploads/upload/transfer.rs`, which open the file with `s3::Source::open` (`agent/src/s3/source.rs`). `Source::open` uses `files::open_regular_file` and reads only from that handle.

Deletes: `DeleterImpl::sweep_entry` (`agent/src/data_uploads/retention/deleter_impl.rs`) stats the file, compares size, mtime and, if the mtime moved, the digest from `files::sha256`, then removes the path with `files::delete`.

`openat(dirfd, name, O_NOFOLLOW | O_DIRECTORY)` opens one directory entry relative to an open directory and fails with `ELOOP` (or `ENOTDIR`) when the entry is a symlink. Walking a path one component at a time this way, then `openat` or `unlinkat` the last name from the final directory handle, resolves the path without following any symlink below the starting directory. The `nix` crate wraps both calls behind its `fs` feature; the workspace `Cargo.toml` enables only `user` today.

## Plan of Work

Milestone 1, source.

`Cargo.toml` (workspace): add `"fs"` to the `nix` features.

`agent/src/filesys/dir.rs`: derive `PartialEq, Serialize, Deserialize` on `Dir` with `#[serde(transparent)]`, as `File` does, so jobs can hold one.

`agent/src/filesys/files.rs`:
- Add `pub fn glob_root(pattern: &str) -> Dir`, the literal prefix `glob` walks from. Move the prefix count out of `parse_glob` into a private `literal_prefix_len(&[Component]) -> usize` that both use.
- Change `open_regular_file(file)` to `open_regular_file(root: &Dir, file: &File)`. It calls a private `open_beneath`. On Unix, `open_beneath` runs on `spawn_blocking`: `open_parent_beneath` strips `root` from `file` (anything but plain names below it is `InvalidInput`), opens `root` with `O_DIRECTORY` (following links), then each directory below it with `openat(O_NOFOLLOW | O_DIRECTORY)`; `open_beneath` then opens the last name with `openat(O_NOFOLLOW | O_NONBLOCK)`. On Windows it opens the path as before. The regular-file check stays.
- `sha256`, `sha256_and_crc32c` and `read_chunks` take `root: &Dir` and pass it on.
- Add `pub async fn delete_beneath(root: &Dir, file: &File)`: on Unix, `unlinkat` the last name from `open_parent_beneath`'s directory; on Windows, `files::delete`. `NotFound` is success and other errors are `DeleteFileErr`, as in `files::delete`.

`agent/src/data_uploads/upload/job.rs` and `agent/src/data_uploads/retention/job.rs`: add `#[serde(default)] pub glob_root: Option<Dir>` and `pub fn root(&self) -> Dir`, which returns the glob root or, for a job from an older agent, the file's own directory.

`agent/src/data_uploads/upload/sink.rs` and `agent/src/data_uploads/retention/sink.rs`: set `glob_root: Some(files::glob_root(&rule.source.glob))`. `uploader_impl.rs`'s `enqueue_delete_job` copies it to the delete job.

`agent/src/data_uploads/scan/rule.rs`: `differs_from_previous` hashes with `files::sha256(&files::glob_root(&state.cfg.rule.source.glob), &candidate.file)`.

`agent/src/s3/source.rs`: `Source::open(root: &Dir, file: File, dst: &Object)`.

`agent/src/data_uploads/upload/transfer.rs`: `ObjectTransfer::reuse_or_create_multipart` and `transfer`, and their private helpers, take `root: &Dir` before `file` and pass it to `Source::open`. `executor.rs` passes `&job.root()`.

`agent/src/data_uploads/retention/deleter_impl.rs`: `check_digest_mismatch` hashes with `files::sha256(&entry.root(), &entry.file)`; `delete_file` removes with `files::delete_beneath(&entry.root(), &entry.file)`.

Milestone 2, tests (the `ready` stage).

Update the existing callers and job literals for the new signatures and field: `agent/tests/filesys/files.rs`, `agent/tests/s3/harnesses.rs`, the `s3` unit tests in `source.rs`, `store.rs`, `multipart.rs` and `gcs.rs`, the `ObjectTransfer` mocks and job fixtures under `agent/tests/data_uploads/` and `agent/tests/test_utils/` (`upload.rs`, `retention.rs`), and the inline tests in `rule.rs`, `deleter_impl.rs`, `job.rs` and `queue/file.rs`. Then add, next to #437's `refuses_symlink` in `agent/tests/filesys/files.rs` (all `#[cfg(unix)]`):

- `sha256` refuses a file whose parent directory below the root is a symlink (`OpenFileErr`), and reads a file whose root is itself a symlink.
- `sha256` refuses a file outside its root (`OpenFileErr`).
- `delete_beneath` refuses to unlink through a symlinked parent and leaves the target file; deletes a plain file; treats a missing file as success.
- `glob_root` returns the literal prefix (`/a/b/*.log` → `/a/b`, `/a/**/x` → `/a`, `/a/b.log` → `/a`).

In `deleter_impl.rs`'s inline tests: a job whose parent directory was swapped for a symlink after enqueue leaves the target file and counts a failed attempt. In `agent/tests/data_uploads/upload/queue.rs` and `agent/tests/data_uploads/retention/queue.rs`: a job serialized without `glob_root` deserializes with `None`, and `root()` is the file's directory.

## Concrete Steps

From the agent repo root, after each milestone:

    git add <the files changed>
    git commit -m "fix(filesys): open and delete scanned files beneath their glob root"

    git add <the test files>
    git commit -m "test(filesys): cover symlinked parent directories"

Push with `git push -u origin fix/eng-1554-no-follow-parent-dirs`. The PR stacks on #438, so `ci.yml` runs only when triggered on the branch (`workflow_dispatch`).

## Validation and Acceptance

`ci.yml` jobs `lint`, `test` and `windows-check` pass on the pushed head, and `$preflight` reports `CLEAN` before the PR leaves draft. The new tests show: with `<root>/sub` replaced by a symlink to a directory holding `x.log`, `sha256(root, <root>/sub/x.log)` fails with `OpenFileErr`, `delete_beneath` leaves the target, and a delete job for it is retried rather than deleting; with `root` itself a symlink, hashing still succeeds.

## Idempotence and Recovery

All edits are source changes; re-running a step re-applies the same edit. Old queue snapshots still load because the new field is `#[serde(default)]`. To roll back, revert the commits.
