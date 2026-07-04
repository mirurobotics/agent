# ExecPlan: S3 module — use `File` instead of `Path`

## Goal

Refactor the S3 object-storage module (`agent/src/s3/`) to accept the internal
`File` type (`crate::filesys::file::File`) instead of `std::path::Path`. Finish
and correct the partial WIP already in the working tree.

## Scope (decided — do not revisit)

1. **Path → File** refactor across the s3 module (the main task).
2. **Keep** the WIP's threshold simplification: multipart threshold = 8 MiB,
   `PART_SIZE` = 8 MiB, and the old 5 GiB `SINGLE_PUT_MAX` removed from the
   production default.
3. **Re-add a `#[cfg(feature = "test")]` seam** so the multipart tests can force
   the multipart path with tiny fixtures (they call `set_single_put_threshold(0)`).
   Minimal test fix — do NOT rewrite tests to use >8 MiB fixtures.

## Files changed

### `agent/src/s3/mod.rs`

The WIP currently does not compile. Corrections:

- Keep `use crate::filesys::file::File;`. Update the module-level doc comment
  (~line 15) that references the now-removed `SINGLE_PUT_MAX`.
- **Test seam:** re-add field `single_put_threshold: u64` to `struct S3Store`.
  Add `const DEFAULT_MULTIPART_THRESHOLD: u64 = 8 * 1024 * 1024;` (8 MiB) and
  initialize the field to it in **both** constructors (`new`, `with_http_client`).
  Re-add:
  ```rust
  #[cfg(feature = "test")]
  pub fn set_single_put_threshold(&mut self, bytes: u64) {
      self.single_put_threshold = bytes;
  }
  ```
  Remove the WIP's local `const EIGHT_MIB` / `if size > EIGHT_MIB`.
- `put_file(&self, key: &str, file: File)`:
  - `let size = file.size().await?;` (propagates `FileSysErr` via new `From`).
  - `if size > self.single_put_threshold { return self.put_object_multipart(key, &file, size).await; }`
  - `ByteStream::from_path(file.path())`; error mapper takes `&file`.
- `put_object_multipart(&self, key, file: &File, size)` and
  `upload_parts_and_complete(&self, key, file: &File, size, part_size, upload_id)`:
  switch `&Path` → `&File`; use `file.path()` in `ByteStream::from_path` /
  `ByteStream::read_from().path(...)`; pass `file` through to `map_bytestream_err`.
- `get_object(&self, key, dest: &File)`: switch `&Path` → `&File`; use
  `dest.path()` in `tokio::fs::File::create(...)` and the io-error mapper.
- Mappers `map_body_io_err` / `map_bytestream_err`: change the `path: &Path`
  param to `file: &File`. `File` implements `Display`, so `path.display()` in the
  `format!` becomes `{file}`.
- Keep `PART_SIZE = 8 MiB`, `MIN_PART_SIZE = 5 MiB`, `MAX_PARTS = 10_000`,
  `part_size_for`, and its 3 unit tests unchanged.

### `agent/src/s3/errors.rs`

Follow the existing pattern in `agent/src/storage/errors.rs`:

- `use crate::filesys;`
- Add enum variant `#[error(transparent)] FileSysErr(filesys::FileSysErr),`.
- `impl From<filesys::FileSysErr> for S3Err { fn from(e) -> Self { Self::FileSysErr(e) } }`
- Add `FileSysErr` to the `crate::impl_error!(S3Err { ... })` list.

### `agent/tests/s3/mod.rs`

- Add import for `File` (confirm public path — `File` is `crate::filesys::file::File`
  in src; from the integration test crate use `miru_agent::filesys::file::File`.
  Grep `agent/src/lib.rs` + `agent/src/filesys/mod.rs` for `pub mod` / `pub use`
  and use whatever public route exists).
- `store.put_file(key, src.path())` → `store.put_file(key, File::new(src.path()))`
  (`src` is a `NamedTempFile`; `src.path(): &Path`; `File::new` takes `Into<PathBuf>`).
- `store.get_object(key, dest.path())` → `store.get_object(key, &File::new(dest.path()))`.
- `put_source_missing` test: a missing file now surfaces via `file.size().await?`
  → `FileSysErr` → `S3Err::FileSysErr` (not `InvalidResponseErr`). Wrap `missing`
  in `File::new(...)` and change the expected variant to `S3Err::FileSysErr(_)`.
- The 6 `set_single_put_threshold(0)` calls stay (seam restored).

## Conventions

- Import ordering: `// standard crates` / `// internal crates` / `// external crates`,
  blank-line separated.
- Error enums: `thiserror::Error` + `crate::errors::Error` + `impl_error!`.
- `#[cfg(feature = "test")]` gates the seam only — never a production path.

## Test steps

1. `put_streams_file_body_bytes` (single-PUT happy path) passes with `File`.
2. `large_file_uploads_in_parts` + multipart error/abort tests pass, multipart
   forced via `set_single_put_threshold(0)`.
3. `put_source_missing` now expects `S3Err::FileSysErr(_)`.
4. `get_object` round-trip + not-found tests pass with `&File`.
5. The 3 `part_size_for` unit tests unchanged and passing.

## Validation

- Build: `cargo build --features test -p miru-agent`.
- Tests: `./scripts/test.sh` (`RUST_LOG=off cargo test --features test`) — all pass.
- Lint: `scripts/lint.sh` (import linter, `cargo fmt`, machete/diet, audit,
  clippy `-D warnings`).
- Coverage: `scripts/covgate.sh` — the `agent/src/s3/.covgate` gate must pass.
- **Preflight must report `clean` before the changes are pushed.**
