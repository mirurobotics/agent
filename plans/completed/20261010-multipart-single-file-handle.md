# Upload every S3 object from one open file handle

Linear: ENG-1538 (File Uploads Tech Debt, M0 data-loss fixes).

## Purpose

`Store::read_part_bytes` (`agent/src/s3/multipart.rs`) reopened the source file by path for every part. If logrotate renamed `robot.log` mid-upload and created a new `robot.log`, the later parts came from the new file. When the new file was long enough, `read_exact` succeeded, the multipart upload completed and was confirmed, and retention then deleted the local original, leaving a corrupt mix of two files as the only stored copy.

Two smaller windows had the same shape:

- `Store::put` sized the file by path, and the multipart path opened it later (after `CreateMultipartUpload`), so the part plan and the bytes could describe different files.
- `put_singlepart` used `ByteStream::from_path`, whose body reopens the path on every SDK retry.

GCS already opens the file once and hands the handle to the SDK.

## Progress

- [x] M1: read every multipart part through one handle.
- [x] M2: introduce `s3::Source`, opened once in `put`, carrying the handle and the size read from it; route single PUT and multipart through it.
- [x] M3: tests, lint, clippy.

## The Change

- `s3::Source { file, handle, size }` is built only by `Source::open`, which opens the path once and takes `size` from the handle's metadata. A missing file is `FileSysErr::PathDoesNotExistErr` (as in GCS); other failures are `LocalIoErr`. `Source::read` seeks and reads a range from the handle.
- `Store::put` opens the `Source` before routing by size, so a missing file fails before any request.
- `put_singlepart` reads the whole file (at most `PART_SIZE`) from the handle into memory, so the SDK can still retry the body without reopening the path.
- `put_multipart`, `resume_multipart_upload` and `upload_parts` take the `Source`; `upload_part` takes the part's bytes. `open_source`, `read_part_bytes` and `errors::map_bytestream_err` are gone.
- Resume now requires an openable source even when every part has landed (the caller must build a `Source`). `resume_multipart_upload` has no production caller.

## Validation

- `put::path_replaced_after_open_uploads_the_opened_file` (multipart, unix): opens a two-part source, renames it away, writes a longer, different file at the path, then runs `put_multipart` and asserts both part bodies are the original bytes. The replacement would plan three parts if sized by path.
- `put::single::path_replaced_after_open_uploads_the_opened_file` (unix): the same for `put_singlepart`.
- Both tests fail when `Source::read` is changed to reopen the path.
- `put::source_truncated_after_open_maps_to_local_io_err`: truncating the opened file in place fails the part read and aborts.
- `put_missing_source_maps_to_filesys_err`: asserts `PathDoesNotExistErr` and no requests.
- Single-part tests now assert the exact request body.
- `cargo test --package miru-agent`, `scripts/lint.sh` (fmt, clippy, import and assert lints).

## What does NOT change

- Part sizing, the part plan, abort-on-failure.
- GCS transfers.
- A retry or queued attempt still reopens the path, so a file rotated between scan and upload is uploaded under the scanned digest. Tracked separately: ENG-1308 (scanned digest matches uploaded digest), ENG-1562 (backend verifies digest and size on confirm).
