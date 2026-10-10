# Read every multipart part from one file handle

Linear: ENG-1538 (File Uploads Tech Debt, M0 data-loss fixes).

## Purpose

`Store::read_part_bytes` (`agent/src/s3/multipart.rs`) reopened the source file by path for every part. If logrotate renamed `robot.log` mid-upload and created a new `robot.log`, the later parts came from the new file. When the new file was long enough, `read_exact` succeeded, the multipart upload completed and was confirmed, and retention then deleted the local original, leaving a corrupt mix of two files as the only stored copy. Single PUT and GCS already hold one handle for the whole transfer.

## Progress

- [x] M1: open the source once in `upload_parts` and read every part through that handle.
- [x] M2: test, lint, clippy.

## The Change

- `upload_parts` opens the file lazily on the first part that needs uploading (so a resume whose parts all landed still never touches the file) and passes the handle to every later `read_part_bytes`.
- `read_part_bytes` takes the open handle instead of a path; the open moved into `open_source`, keeping the same `LocalIoErr` mapping.
- `upload_part` now takes the part's bytes, so the read and the S3 call stay separate and the argument count stays under clippy's limit.

## Validation

- `rotation::parts_read_after_rotation_come_from_the_original_file` (unix): opens the source, renames it away and writes a different file at the same path, then reads a part through the handle and gets the original bytes.
- Existing `put` and `resume` tests (missing, deleted, shrunk sources; landed-part reuse) pass unchanged.
- `cargo test --package miru-agent`, `cargo fmt`, `cargo clippy --all-targets --all-features -D warnings`, custom import linter.

## What does NOT change

- Part sizing, the part plan, abort-on-failure, and resume semantics.
- Single PUT and GCS transfers.
- Optional follow-up not done here: comparing dev/inode/mtime at start and complete.
