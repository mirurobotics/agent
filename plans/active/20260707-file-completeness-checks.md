# Cheap file-completeness (EOF/finalization) checks for the data-upload subsystem

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | New top-level module `agent/src/fileformats/` with per-format completeness checks; two new read helpers in `agent/src/filesys/files.rs`; module registration in `agent/src/lib.rs` and `agent/tests/mod.rs`; new test directory `agent/tests/fileformats/`; new `.covgate` file. No other repos are touched. |

This plan lives in `agent/plans/` because all code changes are inside the agent repo. Work happens on branch `feat/file-finalization-checks` (base `main`). Commit from the agent repo root (`/home/ben/miru/workbench4/repos/agent` or wherever the repo is checked out), never from the workbench root.

## Purpose / Big Picture

The agent is gaining a file-upload subsystem: upload rules (already persisted via `agent/src/disk/upload_rules.rs`, model in `agent/src/models/upload_rule.rs`) match device files by glob, and the future uploader must decide whether a matched file is *complete* — i.e. its writer finished and finalized it — before uploading. Relying only on a stability window (file unchanged for N seconds, see `UploadRuleSource::stability_window_secs`) is slow and can be wrong for paused writers. The backend API spec already promises the direct approach — `stability_window_secs` is documented as "Files in a format with a finalization marker (e.g. MCAP, parquet) are detected directly; this window is the fallback for other files" (`libs/backend-api/src/models/upload_rule_source.rs`), and `create_upload_request.rs` carries an `incomplete: true` flag for files collected without a clean finalization. Many formats carry an explicit finalization marker (a footer, end-of-stream record, or self-describing size) that can be verified by reading only a few bytes; this module is that direct check.

After this change, the crate exposes:

    miru_agent::fileformats::check(&File) -> Result<Completeness, FileFormatsErr>

where `Completeness` is `Complete` (finalization marker present and consistent), `Incomplete` (format recognized, marker absent/inconsistent — writer likely still busy or the file is truncated/garbage), or `Unknown` (extension not recognized, or the format cannot be cheaply verified). Format detection is by file extension. Every check is cheap: it reads a small fixed-size head and/or tail, plus at most a bounded header-hop walk (seek over payloads reading only chunk/box headers) for container formats. No check ever reads or hashes a whole file.

There is no uploader yet; this lands as a self-contained module with no call sites outside tests. Observable outcome: the new unit tests demonstrate each format's verdicts, and `./scripts/preflight.sh` prints `Preflight clean`.

## Progress

- [ ] M1: `filesys::files` gains `read_tail` and `read_range` helpers + tests; committed.
- [ ] M2: `fileformats` module scaffold (enum, dispatch, errors, registration, `.covgate`) + dispatch tests; committed.
- [ ] M3: tail/head-marker formats (MCAP, Parquet, PNG, JPEG, tar, ZIP, gzip) + tests; committed.
- [ ] M4: header-parse formats (ROS1 bag, SQLite, HDF5, AVI) + tests; committed.
- [ ] M5: bounded-walk formats (MP4/MOV, MKV/WebM, zstd) + tests; committed.
- [ ] M6: coverage gate finalized, lint clean, `./scripts/preflight.sh` prints `Preflight clean`; committed.

Use timestamps when completing steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

(Add entries as you go.)

- Observation: …
  Evidence: …

## Decision Log

- Decision: New top-level module `agent/src/fileformats/`, not a `filesys` submodule.
  Rationale: `ARCHITECTURE.md` defines `filesys` as generic "file, directory, and path abstractions"; format-specific byte-layout knowledge (MCAP, MP4, …) is a different concern. `AGENTS.md` documents "Adding a new module" as a standard 4-step task. Date/Author: 2026-07-07 / ben@miruml.com.
- Decision: Files smaller than a format's minimum finalized size (including empty files) return `Incomplete`; unrecognized extensions and not-cheaply-verifiable cases return `Unknown`; I/O failures propagate as `Err`, never as a verdict.
  Rationale: A too-small file cannot be a finalized instance of its claimed format, so `Incomplete` is the honest verdict; `Unknown` is reserved for "we cannot tell", and errors must not masquerade as verdicts or the caller will endlessly re-poll a file it cannot read. Date/Author: 2026-07-07 / ben@miruml.com.
- Decision: gzip never returns `Complete` in this version (returns `Unknown` after magic + minimum-size sanity, `Incomplete` otherwise).
  Rationale: validating the gzip trailer requires decompressing the stream, which violates the cheapness constraint. Best-effort `Unknown` lets the caller fall back to the stability window. Date/Author: 2026-07-07 / ben@miruml.com.
- Decision: Test fixtures are hand-crafted byte vectors built inline in test code and written to temp dirs — no binary files committed to `testdata/`.
  Rationale: fixtures are tens of bytes, and inline builders are reviewable and self-documenting; `testdata/` (used today only for crypt PEM keys) stays free of opaque binaries. Date/Author: 2026-07-07 / ben@miruml.com.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

Key existing pieces (all paths relative to the agent repo root):

- `agent/src/filesys/` — filesystem primitives. `file.rs` defines `File` (a validated `PathBuf` wrapper with inherent `name()` / `parent()`, plus `path()` and `exists()` via the `PathExt` trait in `path.rs`). `files.rs` holds async free functions (`read_bytes`, `hash`, `glob`, `metadata`, `size`, …) built on `tokio::fs`, each mapping `std::io::Error` into a `FileSysErr` variant (see `map_io_err_for_open`). `errors.rs` defines one thiserror struct per failure (each with a `trace: Box<Trace>` field populated by the `crate::trace!()` macro) aggregated into `pub enum FileSysErr` registered with `crate::impl_error!`.
- `agent/src/errors/mod.rs` — the `Error` trait (default `code()`, `http_status()`, `params()`, `is_network_conn_err()`), the `Trace` struct, and the `trace!` / `impl_error!` macros every module's `errors.rs` uses.
- `agent/src/lib.rs` — alphabetical `pub mod` list; new modules must be inserted in order (`fileformats` sorts between `events` and `filesys`).
- `agent/tests/` — mirrors `agent/src/` (e.g. `agent/tests/filesys/files.rs` tests `agent/src/filesys/files.rs`); registered in `agent/tests/mod.rs` (also alphabetical). Tests use `#[tokio::test]`, create scratch space with `dirs::create_temp("testing")`, and write fixtures with `files::write_bytes`. The custom lint rejects 4+ `assert_eq!` calls on fields of one variable in a test (prefer whole-value assertions — here, assert the whole `Completeness` value).
- `agent/src/<module>/.covgate` — per-module minimum coverage percentage, enforced by `./scripts/covgate.sh` (e.g. `agent/src/filesys/.covgate` contains `81.69`). New modules need their own `.covgate` (AGENTS.md, "Adding a new module").
- Import ordering convention: every file groups imports as `// standard crates`, `// internal crates`, `// external crates`, enforced by the import linter run in `./scripts/lint.sh`.
- Gates: `./scripts/test.sh` (cargo test `--features test`), `./scripts/lint.sh` (import linter + fmt + machete/diet + audit + clippy `-D warnings`), `./scripts/covgate.sh` (coverage gates), `./scripts/preflight.sh` (all of the above plus the lint tool's own gates; final line `Preflight clean` on success).

Terms: a *finalization marker* is any byte structure a writer emits only on clean close (footer record, end-of-archive blocks, filled-in size field). A *header-hop walk* seeks from one chunk/box header to the next using the size declared in each header, reading only the headers (8–16 bytes each), never the payloads.

## Interfaces and Dependencies

No new crate dependencies — all parsing is hand-rolled over byte slices (the point of the module). `Cargo.toml` is untouched; `cargo machete` would flag an unused format crate anyway.

New public API in `agent/src/fileformats/mod.rs`:

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Completeness {
        Complete,   // finalization marker present and consistent with file size
        Incomplete, // format recognized; marker missing/inconsistent (truncated, still being written, or garbage)
        Unknown,    // extension not recognized, or format not cheaply verifiable
    }

    pub async fn check(file: &crate::filesys::File) -> Result<Completeness, FileFormatsErr>

`FileFormatsErr` (in `agent/src/fileformats/errors.rs`) follows the repo error pattern: a `FileSysErr(FileSysErr)` variant (with `From<FileSysErr>`), `#[error(transparent)]` per variant, and `crate::impl_error!(FileFormatsErr { FileSysErr });`. Parse anomalies are verdicts, not errors — only real I/O failures become `Err`.

New helpers in `agent/src/filesys/files.rs` (generic primitives, so they belong in `filesys`, mirroring `read_bytes`):

    /// Reads up to `n` bytes from the end of the file (fewer if the file is shorter).
    pub async fn read_tail(file: &File, n: u64) -> Result<Vec<u8>, FileSysErr>
    /// Reads up to `len` bytes starting at `offset` (fewer if EOF intervenes).
    pub async fn read_range(file: &File, offset: u64, len: u64) -> Result<Vec<u8>, FileSysErr>

Both open with `tokio::fs::File`, use `AsyncSeekExt::seek` + bounded `read` loop, and map errors exactly like `read_bytes` (reuse `map_io_err_for_open`; read failures → `FileSysErr::ReadFileErr`). No new `FileSysErr` variants needed.

## Plan of Work

### Module layout

    agent/src/fileformats/
        .covgate      — coverage gate (conservative placeholder 50.00 at scaffold so intermediate milestones pass; raised to the measured value in M6)
        mod.rs        — Completeness enum, extension→format dispatch, pub async fn check
        errors.rs     — FileFormatsErr
        mcap.rs  parquet.rs  rosbag.rs  sqlite.rs  mp4.rs  jpeg.rs  png.rs
        zip.rs  gzip.rs  tar.rs  zstd.rs  hdf5.rs  matroska.rs  avi.rs
    agent/tests/fileformats/
        mod.rs        — registers the test files below
        check.rs      — dispatch-level tests (extensions, unknown ext, empty, missing file)
        mcap.rs … avi.rs — one test file per format file above

Each format file exposes one `pub(crate) async fn check(file: &File, size: u64) -> Result<Completeness, FileFormatsErr>` (the dispatcher fetches `size` once via `filesys::files::size` so every check gets it for free). `mod.rs` lowercases the extension (`file.path().extension()`) and routes:

| Extension(s) | Format fn | Verdict logic (v1) |
|---|---|---|
| `mcap` | `mcap::check` | tail footer record + trailing magic |
| `parquet` | `parquet::check` | 8-byte tail: footer length + `PAR1` |
| `bag` | `rosbag::check` | leading magic + file-header `index_pos` field |
| `db` `db3` `sqlite` `sqlite3` | `sqlite::check` | header page math + no `-wal`/`-journal` sidecar |
| `mp4` `mov` `m4v` | `mp4::check` | bounded top-level box walk, `moov` required |
| `jpg` `jpeg` | `jpeg::check` | `FF D9` in last bytes, tolerating trailing NUL padding |
| `png` | `png::check` | leading signature + 12-byte IEND tail |
| `zip` | `zip::check` | EOCD signature scan in last 65,557 bytes |
| `gz` `tgz` | `gzip::check` | magic + min size → `Unknown` (never `Complete`) |
| `tar` | `tar::check` | size multiple of 512 + last 1024 bytes zero |
| `zst` | `zstd::check` | magic + capped block-header walk |
| `h5` `hdf5` | `hdf5::check` | superblock EOF address == file size |
| `mkv` `webm` | `matroska::check` | EBML header + finite Segment size tiling to EOF |
| `avi` | `avi::check` | RIFF size field + 8 == file size |
| anything else / no extension | — | `Unknown` |

Shared edge-case policy, applied uniformly inside each format fn: `size == 0` or `size <` the format's minimum finalized size → `Incomplete`; any `FileSysErr` from the read helpers → `Err` (bubbles via `?`); verdicts otherwise per format below.

### Per-format check specifications

Byte layouts below are the authoritative spec for implementation; all multi-byte integers state their endianness. "head(n)" = `read_range(file, 0, n)`, "tail(n)" = `read_tail(file, n)`.

**MCAP** (`mcap.rs`). Magic is the 8 bytes `\x89 M C A P 0 \r \n`. A finalized file starts with the magic and ends with a Footer record followed by the magic again. Records are `opcode: u8` + `length: u64 LE` + payload; the Footer has opcode `0x02` and a fixed 20-byte payload, so the trailing region is 1+8+20+8 = 37 bytes. Minimum finalized size: 45 (leading magic + trailing region). Check: head(8) == magic, and in tail(37): bytes 29..37 == magic, byte 0 == 0x02, u64 LE at 1..9 == 20. All hold → `Complete`, else `Incomplete`.

**Parquet** (`parquet.rs`). tail(8) = `footer_len: u32 LE` + ASCII `PAR1`. Minimum size 12 (leading `PAR1` + footer_len + trailing `PAR1`). Check: tail bytes 4..8 == `PAR1` and `footer_len as u64 + 8 <= size` → `Complete`, else `Incomplete`. (Optionally also head(4) == `PAR1`; include it — it is one cheap read and rejects garbage.)

**ROS 1 bag** (`rosbag.rs`). Leading magic is the 13 ASCII bytes `#ROSBAG V2.0\n`. Immediately after it sits the file-header record: `header_len: u32 LE`, then `header_len` bytes of fields, each field being `field_len: u32 LE` + `name=value` bytes. The file-header's `index_pos` field holds a `u64 LE` — the offset of the index data section, written only on clean close (0 while recording). Check: head(13) == magic else `Incomplete`; read `header_len` from bytes 13..17; if `header_len > 8192` → `Unknown` (implausible, not cheaply verifiable); read the header block, scan fields for name `index_pos`; found with 8-byte value: `Complete` iff `0 < index_pos <= size`, else `Incomplete`; field missing or field parse runs off the block → `Incomplete`.

**SQLite / ROS 2 bag** (`sqlite.rs`). head(100) is the database header: bytes 0..16 == `SQLite format 3\0`, `page_size: u16 BE` at offset 16 (value 1 means 65536), `page_count: u32 BE` at offset 28. Check: magic mismatch or `size < 100` → `Incomplete`. Sidecars: build sibling paths by appending `-wal` and `-journal` to the full filename (e.g. `data.db3-wal`); if either exists (`File::new(...).exists()`, no I/O beyond a stat) → `Incomplete` (a hot journal means unflushed state). Else `Complete` iff `page_size_bytes * page_count == size`, else `Incomplete`.

**MP4/MOV** (`mp4.rs`). The file is a sequence of top-level boxes: `size: u32 BE` + `fourcc: [u8;4]`; `size == 1` means a `largesize: u64 BE` follows in bytes 8..16 (header 16 bytes, largesize must be >= 16); `size == 0` means "box extends to EOF" (legal only for the final box); otherwise `size >= 8`. Walk from offset 0: `read_range(file, cursor, 16)` per box, record whether any fourcc == `moov`, advance `cursor += box_size`. Terminate: `cursor == size` exactly and `moov` seen → `Complete`; `size == 0` box → treat as ending at EOF (tiles by definition) and apply the same `moov` rule; malformed size (< 8 and not 0/1), or `cursor` overrunning `size` → `Incomplete`; more than 10,000 boxes → `Unknown` (cap keeps the walk bounded; real files have well under 100 top-level boxes).

**JPEG** (`jpeg.rs`). A finalized JPEG ends with the EOI marker `FF D9`, but some writers append a few NUL padding bytes. Check: head(2) == `FF D8` (SOI) else `Incomplete`; in tail(16), strip trailing `0x00` bytes, then the last two remaining bytes must be `FF D9` → `Complete`, else `Incomplete`. Minimum size 4.

**PNG** (`png.rs`). head(8) == signature `89 50 4E 47 0D 0A 1A 0A`; tail(12) == the complete IEND chunk `00 00 00 00` + `IEND` + CRC `AE 42 60 82` (IEND's CRC is constant since it has no data). Both → `Complete`, else `Incomplete`. Minimum size 20.

**ZIP** (`zip.rs`). The End of Central Directory (EOCD) record (22 bytes minimum, signature `50 4B 05 06`, `comment_len: u16 LE` at record offset 20) sits within the last 22+65535 = 65,557 bytes. Check: `size < 22` → `Incomplete`; read tail(min(size, 65557)); scan backwards for the signature; for each candidate at tail-offset `p`, accept iff `p + 22 <= tail_len` and `p + 22 + comment_len == tail_len` (the record + comment must end exactly at EOF); first accepted candidate → `Complete`; none → `Incomplete`.

**gzip** (`gzip.rs`). head(2) == `1F 8B` and `size >= 18` (10-byte header + 8-byte trailer) → `Unknown` (the CRC32/ISIZE trailer cannot be validated without decompressing — disallowed as too expensive — so completeness is not cheaply determinable); otherwise `Incomplete`. `.tgz` is treated as gzip (the tar layer is inside the compressed stream). This format never returns `Complete` in v1; callers fall back to the stability window.

**tar** (`tar.rs`). A finalized tar ends with two 512-byte zero blocks (and any further padding is also zeros). Check: `size >= 1024`, `size % 512 == 0`, and tail(1024) is all `0x00` → `Complete`, else `Incomplete`. (No leading-magic check: pre-POSIX v7 tars have none at a fixed offset.)

**zstd** (`zstd.rs`). Frames start with magic `28 B5 2F FD` (u32 LE `0xFD2FB528`); skippable frames use magic `50 2A 4D 18`..`5F 2A 4D 18` (u32 LE `0x184D2A50 + x`) followed by `frame_size: u32 LE`. Check: head(4) == zstd or skippable magic, else `Incomplete`. Walk frames from offset 0 with a global cap of 1,024 header reads: skippable frame → hop `8 + frame_size`. Normal frame: read the frame header — after the magic, `descriptor: u8` with `fcs_flag = descriptor >> 6`, `single_segment = (descriptor >> 5) & 1`, `dict_flag = descriptor & 3`; a 1-byte window descriptor follows iff `single_segment == 0`; dictionary-ID length is `[0,1,2,4][dict_flag]`; frame-content-size length is `[0,2,4,8][fcs_flag]`, except 1 when `fcs_flag == 0 && single_segment == 1`. Then hop blocks: each block header is 3 bytes LE — `last_block = bit 0`, `block_type = bits 1..3`, `block_size = bits 3..24`; payload length is `block_size` for raw (0) and compressed (2) blocks, 1 for RLE (1); type 3 is reserved → `Incomplete`. After the `last_block`, skip a 4-byte checksum iff the descriptor's checksum bit (`(descriptor >> 2) & 1`) is set. Walk ends exactly at `size` → `Complete`; overruns `size` or malformed → `Incomplete`; cap exhausted → `Unknown` (acceptable per the cheapness constraint — large multi-block files are simply not cheaply determinable).

**HDF5** (`hdf5.rs`). v1 scope: only a superblock at offset 0 (files with userblocks put it at 512/1024/…; those → `Incomplete` since the signature check at offset 0 fails — acceptable, documented here). head(64): signature `89 48 44 46 0D 0A 1A 0A` (`\x89HDF\r\n\x1a\n`), else `Incomplete`. `version: u8` at offset 8. The superblock stores an *end-of-file address* — the first byte past all HDF5 data; for base address 0 it equals the file size. Field offsets by version (`so` = "size of offsets", require `so == 8` else `Unknown`): v0: `so` at byte 13, EOF address (u64 LE) at `24 + 2*so`; v1: `so` at byte 13, EOF address at `28 + 2*so`; v2/v3: `so` at byte 9, EOF address at `12 + 2*so`. Other versions → `Unknown`. Also read the base address (the first of the address fields); base != 0 → `Unknown`. `Complete` iff EOF address == `size`, else `Incomplete`.

**MKV/WebM** (`matroska.rs`). EBML documents are element trees; an element is `ID` (1–4 bytes) + `data size` (a "vint": the count of leading zero bits of the first byte determines total width 1–8 bytes; the remaining bits after the marker bit are the value; all-value-bits-set means "unknown size") + payload. Check: head(4) == EBML header ID `1A 45 DF A3`, else `Incomplete`. Parse the EBML header's size vint (payload is ~40 bytes; if unknown-size or > 4,096 → `Unknown`), skip its payload; the next element must be Segment, ID `18 53 80 67` (else `Unknown` — inconclusive parse). Parse the Segment's size vint: unknown size (the all-ones vint, e.g. first byte `0xFF` or `01 FF FF FF FF FF FF FF`) → `Incomplete` (this is exactly the still-recording state); finite size → `Complete` iff `segment_payload_start + segment_size == size`, else `Incomplete`. Any other parse anomaly (truncated vint, zero-width vint) → `Unknown`.

**AVI** (`avi.rs`). head(12): bytes 0..4 == `RIFF`, `riff_size: u32 LE` at 4..8, bytes 8..12 == `AVI ` (trailing space), else `Incomplete`. `Complete` iff `riff_size as u64 + 8 == size`, else `Incomplete` (writers fill the size field on close; a recording AVI has 0 or a stale value). Known caveat to document in the module: OpenDML (AVI 2.0) files > ~1 GiB append additional `RIFF AVIX` chunks, so this check reports them `Incomplete`; extending to a bounded top-level RIFF-chunk walk (like MP4) is deliberate future work, not v1.

### Edits, in milestone order

1. **M1 — `agent/src/filesys/files.rs`**: add `read_tail` and `read_range` after `read_bytes`, per the signatures above. Tests in `agent/tests/filesys/files.rs`: new `pub mod read_tail` / `pub mod read_range` blocks covering exact read, shorter-than-requested file, offset past EOF (empty vec), and missing file → `FileSysErr::PathDoesNotExistErr`.
2. **M2 — scaffold**: create `agent/src/fileformats/mod.rs` (enum + dispatch returning `Unknown` for every recognized format initially, or wire formats as stubs returning `Unknown`), `errors.rs`, `.covgate` containing `50.00`; add `pub mod fileformats;` to `agent/src/lib.rs` (between `events` and `filesys`) and to `agent/tests/mod.rs`; create `agent/tests/fileformats/{mod.rs,check.rs}`. `check.rs` tests: unknown extension → `Unknown`, no extension → `Unknown`, uppercase extension (`FILE.PNG`) routes case-insensitively, empty recognized file → `Incomplete` (once formats land; while stubs, assert `Unknown` and tighten in M3), missing file → `Err` matching `FileFormatsErr::FileSysErr(_)`.
3. **M3 — tail/head formats**: implement `mcap.rs`, `parquet.rs`, `png.rs`, `jpeg.rs`, `tar.rs`, `zip.rs`, `gzip.rs` + their test files.
4. **M4 — header-parse formats**: `rosbag.rs`, `sqlite.rs`, `hdf5.rs`, `avi.rs` + tests (sqlite tests cover the `-wal` sidecar case).
5. **M5 — bounded walks**: `mp4.rs`, `matroska.rs`, `zstd.rs` + tests (include cap-exceeded → `Unknown` tests with fixture files declaring many tiny boxes/blocks).
6. **M6 — gates**: measure the module's coverage, set `agent/src/fileformats/.covgate` to the measured value (expect ≥ 90 — this is pure byte-math with total test control; do not touch other modules' `.covgate` files), run the full gate suite until `Preflight clean`, update this plan's living sections, and move the plan file toward `plans/completed/` when done.

### Test design (applies to M3–M5)

Each `agent/tests/fileformats/<format>.rs` follows the repo's nested-`pub mod` style (see `agent/tests/filesys/files.rs`) and contains small fixture-builder functions returning `Vec<u8>` — e.g. `fn valid_png() -> Vec<u8>` assembling signature + minimal IHDR + IEND by concatenating byte literals (tens of bytes, never real assets). Each test writes the fixture with `files::write_bytes(&dir.file("f.png"), &bytes, WriteOptions::default())` into `dirs::create_temp("testing")` and asserts the whole verdict:

    assert_eq!(fileformats::check(&file).await.unwrap(), Completeness::Complete);

Required cases per format: valid finalized bytes → `Complete` (for gzip: → `Unknown`); truncated valid bytes (chop the tail marker) → `Incomplete`; garbage bytes of plausible length → `Incomplete`; empty file → `Incomplete`; below-minimum-size file → `Incomplete`. Format-specific extras: sqlite valid + existing `-wal` sidecar → `Incomplete`; sqlite page math mismatch → `Incomplete`; mp4 tiling boxes without `moov` → `Incomplete`; mp4/zstd cap exceeded → `Unknown`; matroska unknown-size Segment vint → `Incomplete`; jpeg with 1–8 trailing NULs → `Complete`; zip with a non-empty comment → `Complete`; hdf5 EOF-address mismatch → `Incomplete`; parquet footer_len ≥ size → `Incomplete`; rosbag `index_pos == 0` → `Incomplete`; avi zero size field → `Incomplete`. No `#[serial]` needed (every test uses its own temp dir; no shared OS resources). Mind the field-by-field-assert lint: assert whole values, not 4+ fields of one struct.

## Concrete Steps

All commands run from the agent repo root (the directory containing `AGENTS.md` and `Cargo.toml`; on this machine `/home/ben/miru/workbench4/repos/agent`). Create the branch if it does not exist yet, then confirm it:

    git checkout -b feat/file-finalization-checks main   # skip if the branch already exists
    git rev-parse --abbrev-ref HEAD   # expect: feat/file-finalization-checks

Per milestone M1–M5:

1. Make the edits listed for the milestone in Plan of Work.
2. Build and run the focused tests, then the full suite:

       cargo build -p miru-agent
       ./scripts/test.sh              # full suite; all tests pass, new tests included

   Expected: exit 0; the summary line shows the new tests (e.g. `test fileformats::png::check::valid ... ok`). A failure names the offending test — fix before proceeding.
3. Lint:

       ./scripts/lint.sh

   Expected: exit 0. Most likely complaints: import-group ordering in new files, clippy `-D warnings`, `cargo fmt`.
4. Commit the milestone (one commit per milestone so the PR is reviewable and bisectable):

       git add -A
       git commit -m "<type>(fileformats): <milestone summary>"

   Suggested messages: M1 `feat(filesys): add read_tail and read_range helpers`; M2 `feat(fileformats): scaffold completeness-check module`; M3 `feat(fileformats): tail/head finalization checks (mcap, parquet, png, jpeg, tar, zip, gzip)`; M4 `feat(fileformats): header-parse checks (rosbag, sqlite, hdf5, avi)`; M5 `feat(fileformats): bounded-walk checks (mp4, matroska, zstd)`.

M6 (final gates):

    ./scripts/update-deps.sh          # refresh Cargo.lock before linting (repo convention)
    ./scripts/coverage.sh             # inspect fileformats coverage
    # set agent/src/fileformats/.covgate to the measured value (edit the file by hand;
    # do NOT run ./scripts/update-covgates.sh — it ratchets every module's gate, not just this one)
    ./scripts/covgate.sh              # expect: all gates pass
    ./scripts/preflight.sh            # expect final line: "Preflight clean"
    git add -A && git commit -m "chore(fileformats): finalize coverage gate"

If `preflight.sh` fails, its output names the failing gate (lint / tests / tools lint / tools tests); fix and re-run — it is idempotent. Only after `Preflight clean` prints may the branch be pushed / a PR opened (base `main`).

## Validation and Acceptance

Acceptance is behavioral:

1. `./scripts/test.sh` passes with the new tests present. Spot-check by name:

       RUST_LOG=off cargo test -p miru-agent --features test fileformats

   Expected: only `fileformats::…` tests run, all pass, covering every format listed in the dispatch table (14 format files + dispatch tests). Each format's `Complete`/`Incomplete`/`Unknown` cases from Test design are present; e.g. the truncated-MCAP test fails if the footer check is removed and passes with it (checks are the only thing distinguishing the fixtures).
2. Cheapness holds by construction: every check reads at most head(≤100) + tail(≤65,557 for zip, ≤1,024 for tar, ≤37 otherwise) plus capped header hops (10,000 boxes / 1,024 zstd headers / 2 EBML elements / one 8 KiB rosbag header). No code path calls `read_bytes` (whole file) or `hash` — verify with:

       grep -rn "read_bytes\|files::hash" agent/src/fileformats/    # expect: no matches
3. Coverage: `./scripts/covgate.sh` passes with `agent/src/fileformats/.covgate` set to the measured value (target ≥ 90); no other module's gate was edited.
4. Lint: `./scripts/lint.sh` exits 0.
5. **Publication gate: `./scripts/preflight.sh` must print `Preflight clean` (exit 0) before the changes are published — no push, no PR, no promotion of this plan out of the working branch until that line prints.**

Operator-visible behavior today: none (no caller yet). The demonstrable behavior is the API contract exercised by the tests above.

## Idempotence and Recovery

All steps are local, additive, and re-runnable: rebuilding, re-testing, and re-running the gate scripts are idempotent; re-editing a format file and re-running its tests is the whole dev loop. Nothing here migrates data or touches the wire.

- A broken milestone can be dropped with `git reset --hard HEAD~1` (milestone == commit) or the working tree cleaned with `git checkout -- .`; new untracked files can be removed with `git clean -fd agent/src/fileformats agent/tests/fileformats`.
- The only deletion risk is none — no existing files are removed; `agent/src/lib.rs` / `agent/tests/mod.rs` each gain exactly one line, trivially revertable.
- If `covgate.sh` fails on an unrelated module after these changes, coverage percentages shifted globally is *not* expected (changes are additive in a new module); investigate rather than lowering any gate.
- If a walk-based check misbehaves on a hand-crafted fixture, add the fixture bytes to the test as a regression case; fixtures are deterministic, so failures reproduce exactly.

---

Revision note (2026-07-07, pre-implementation review): added `-p miru-agent` to the validation spot-check command (the repo root is a virtual cargo workspace, and `scripts/test.sh` itself pins `--package miru-agent`); added the branch-creation command before the branch assertion; warned against `./scripts/update-covgates.sh` in M6 (it ratchets every module's gate, conflicting with the do-not-touch-other-gates constraint); clarified that `File::path()`/`exists()` come from the `PathExt` trait. No changes to per-format byte specifications.
