# `stump_tools`

## Purpose

`stump_tools` provides library-maintenance operations behind the
`stump tools list|plan|apply` CLI. Each operation separates a read-only `plan`
from an explicit `apply`; a dry run is printable and does not mutate files.
This crate does not own job scheduling, database access, or HTTP routes.

Tools cover audiobook assembly, chapter/tag edits, and reports; Kindle and
Calibre conversion/metadata; CBZ covers and packing; EPUB validation, polish,
and image extraction; bulk metadata edits; sequence-gap reports; and source
catalogue import. The registry in `src/lib.rs` is the complete command list.

## Reference / upstream

- Kavita's external-tools catalogue is a behavior reference only; no upstream
  implementation code is copied.
- EPUB 3.3, ComicInfo, MP4/ID3 chapter, ffmetadata, and MOBI/KF8 format sources
  define serialization and parsing contracts. MOBI parsing belongs to
  `stump_media::MobiBook`.
- Calibre and boko are optional operator-installed command-line programs. They
  are launched as separate processes; this crate does not link, vendor, or
  bundle them. See the [Calibre tooling guide](../../docs/content/docs/developer/calibre-tooling.mdx).

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Planning reads only; apply consumes the complete plan and rechecks replacement targets. | The approved dry run must be what executes, even if it is applied later. | `src/lib.rs` (`Tool`); `src/plan.rs` |
| Bulk per-file failures are warnings or skipped reports; `ToolError` aborts a whole run. | One malformed book must not discard unrelated work. | `src/plan.rs`; tool tests |
| Generated outputs stage beside their destination and rename atomically. | A failed write must not truncate the original or leave a partial archive. | `src/util.rs` (`write_atomic`) |
| CBZ page detection/order uses shared media and scanner conventions. | Tools, scanner, and processors must agree on what a page and sequence are. | `src/util.rs`; `crates/media`; `crates/scanner` |
| Audio assembly verifies its output and keeps chapter-writing behavior distinct from in-place chapter edits. | Players differ in chapter support; mutating a published container has a narrower safety boundary than building a new one. | `src/audio_assemble/`; `src/audio_chapters.rs` |
| `sources-import` emits data-only source definitions. | Site-specific parsing data should not become executable Rust code. | `src/sources_import.rs`; [source definitions](../../docs/content/docs/developer/source-definitions.mdx) |

## Layout

| Path | Responsibility |
| --- | --- |
| `src/lib.rs`, `src/plan.rs`, `src/error.rs` | Tool registry, plan/action/report contract, and errors. |
| `src/util.rs`, `src/external.rs`, `src/ffmpeg.rs` | Safe file helpers and operator-installed process adapters. |
| `src/audio_*.rs`, `src/audio_assemble/` | Audio assembly, chapter editing, and reporting. |
| `src/calibre*.rs`, `src/boko.rs`, `src/mobi2epub.rs` | E-book conversion and metadata. |
| `src/cbz_*.rs`, `src/cbzit.rs`, `src/epub*.rs`, `src/webp_convert.rs` | Archive, EPUB, and image tools. |
| `src/meta_edit.rs`, `src/missing_sequence.rs`, `src/sources_import.rs` | Metadata, sequence, and source-definition operations. |

## How to verify

```sh
cargo test -p stump_tools
cargo test -p stump_tools audio
cargo test -p stump_tools boko
cargo test -p stump_tools mobi2epub
cargo test -p stump_tools sources_import
cargo run -p cli --bin cli-bin -- tools list
cargo run -p cli --bin cli-bin -- tools plan cbzit /books/Series --json
```

Planning is read-only; fixture tests verify that it creates no target and that
failed applies preserve original bytes. The MOBI and audio tools use checked-in
format fixtures; mutating tests work on temporary copies.

## Deep docs

- [Calibre and external tooling](../../docs/content/docs/developer/calibre-tooling.mdx)
- [Source definitions and imports](../../docs/content/docs/developer/source-definitions.mdx)
- [`stump_media`](../media/README.md) — archive, image, and audio processing
- [`stump_scanner`](../scanner/README.md) — shared filename and sequence rules
- [Crate index](../README.md)
