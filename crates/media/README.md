# stump_media

## Purpose

File and image processing for Stump: the per-format `FileProcessor`
implementations (`ZipProcessor`, `EpubProcessor`, `PdfProcessor` behind `pdf`,
`RarProcessor` behind `rar`), `ContentType` detection, the unified `FileError`,
Stump's sampled SHA-256 and the KOReader partial-MD5 hashes, Readium Web
Publication Manifest generation, bounded EPUB text search, image resize /
thumbnail / colour / ThumbHash primitives, `series.json` parsing, and the
`MediaConfig` snapshot every processor takes. It deliberately does not depend
on `stump_core`, the job runtime, events, or the database (`src/lib.rs:1-6`);
orchestration (scan jobs, thumbnail/analysis jobs, metadata coordination)
stays in `core/src/filesystem/`, and the scanner walker lives in
`crates/scanner`.

## Reference / upstream

| Reference | Pin | Notes |
| --- | --- | --- |
| Upstream Stump `core/src/filesystem/*` | `stumpapp/stump` nightly merge `37fdb7d7` (PR #1361) | Every `src/**/*.rs` here is an R100 move of `core/src/filesystem/<same relative path>` in `b068deb9` (`git log --follow --name-status -- crates/media/src/<file>`); `27764cf5` added `lib.rs`/`MediaConfig`, the features, and `integration-tests/data`; `859ac022` (Komf) touched it last |
| KOReader partial MD5 | https://github.com/koreader/koreader/blob/009367df7ac7142bc0d1eb5782d061260b11baa6/frontend/util.lua#L1109-L1125; test vectors `spec/unit/util_spec.lua#L338-L345` at the same commit | `generate_koreader_hash` must keep producing KOReader's document identity (`src/hash.rs:62-99,117-130`) |
| Readium Web Publication Manifest | `src/media/readium.rs` | Manifest, positions, `/resource/` URL encoding consumed by `apps/server` readium routes and the Komga/Kavita adapters |
| PDFium | `pdfium-render 0.9.1` (`Cargo.toml:28`), library loaded from `MediaConfig.pdfium_path` (`PDFIUM_PATH`) | Only PDF backend; see `docs/content/docs/developer/server-architecture.mdx:239-243` |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Processors take an immutable `MediaConfig` snapshot instead of `StumpConfig`/`Ctx` | Paths (`cache`, `thumbnails`, `cache/pdf_pages`) are resolved once in `from_values`; per-call code neither clones config nor rebuilds paths; crate compiles without core | `src/lib.rs:137-203`; `docs/content/docs/developer/server-architecture.mdx:133` |
| `pdf` and `rar` are Cargo features (default on) rather than runtime switches | They remove `pdfium-render`/`unrar` from the build; core and server forward them (`stump_core/pdf`, `stump_media/pdf`, ...) | `Cargo.toml:5-9,28,43`; `core/Cargo.toml:13-14`; `apps/server/Cargo.toml:13-15` |
| Feature-off dispatch returns typed errors, not panics | `Rar` → `FileError::UnsupportedFileType("RAR support is disabled")`, `Pdf` → `FileError::PdfConfigurationError`; verified by `feature_gate_tests` | `src/media/process.rs:190-220`; `src/media/process.rs` `feature_gate_tests`; `src/error.rs:24-56` |
| `models` with `default-features = false` | `models` only has the optional `graphql` feature; a leaf crate must not pull `async-graphql` | `Cargo.toml:26`; `crates/models/Cargo.toml:5-10` |
| Stump hash samples the file (4 × 10 000 B + tail) once it exceeds 40 000 B | Cheap identity for large archives; full digest for small files | `src/hash.rs:13-14,32-60` |
| KOReader hash is a literal port of the Lua loop (1 KiB reads at `1024 << i`, `i = -1..=10`) | KOReader identifies documents by this digest; sync only works if both sides agree | `src/hash.rs:62-99`; vectors `src/hash.rs:117-130`; caveat for short final reads `docs/content/docs/developer/provider-status.mdx:140-146` |
| `ContentType::from_bytes_with_fallback` prefers a more specific extension subtype (EPUB/CBZ over generic ZIP) before `infer` | Byte sniffing cannot distinguish EPUB/CBZ from ZIP | `src/content_type.rs:137-185`; tests `src/content_type.rs` |
| EPUB search is bounded and uncached (query 2–128 chars, limit 20/max 50, ≤128 spine items, 32 MiB total / 16 MiB per item, 20 matches per spine item) | Keeps a single request's memory and CPU predictable without an index | `src/media/epub_search.rs:1-5,26-35` |
| Image crate limited to GIF/JPEG/PNG/WebP; colours via `kmeans_colors` (7 + average), ThumbHash ≤100 px, WebP encoder quality 100 | Smaller build; placeholder metadata computed once per thumbnail | `Cargo.toml:18,22,38,46`; `src/image/placeholder.rs:90-104,142-190,216-230`; `src/image/webp.rs:17-83` |
| PDF page cache/prerender lives here, keyed under `MediaConfig.pdf_cache_dir` | Rendering is the expensive step; `pdf_cache_pages`, `pdf_prerender_range`, `pdf_render_dpi`, `pdf_max_dimension`, `pdf_render_format` are plain config values | `src/lib.rs:143-149,225-243`; `src/media/format/pdf.rs:360-389,432-467` |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | Module tree, re-exports, async `resize_image`, `MediaConfig` |
| `src/media/process.rs` | `FileProcessor` / `FileConverter` traits, `FileProcessorOptions`, `ProcessedFile`, `ProcessedFileHashes`, MIME+extension dispatch, sync/async wrappers |
| `src/media/format/{zip,epub,pdf,rar}.rs` | Per-format processors; `pdf`/`rar` are `cfg`-gated in `format/mod.rs` |
| `src/media/metadata.rs`, `src/media/utils.rs`, `src/serde.rs` | ComicInfo / OPF / PDF metadata normalisation, cover-name rules, PDF date parsing, custom deserialisers |
| `src/media/readium.rs` | `ReadiumManifestGenerator` (manifest, positions, resource URLs) |
| `src/media/epub_search.rs` | `search_epub` with cursor, limits, Readium locators |
| `src/content_type.rs`, `src/error.rs` | `ContentType` detection/predicates, `FileError` |
| `src/hash.rs` | `generate` (sampled SHA-256), `generate_koreader_hash` |
| `src/common.rs`, `src/directory_listing.rs`, `src/archive.rs`, `src/series_metadata.rs` | Thumbnail lookup, `PathUtils`/`FileParts`, directory listing DTOs, ZIP creation, `series.json` |
| `src/image/*` | `ImageProcessor` trait, `GenericImageProcessor` (JPEG/PNG), `WebpProcessor`, placeholder metadata (colours, ThumbHash), thumbnail file helpers |
| `integration-tests/data/` | Fixtures: `book.{zip,rar,epub}`, `book-complex-tree.{zip,rar}`, `book-zip-mime.epub`, `book-image-format-test.zip`, `leaves.epub`, `tall.pdf`, `rust_book.pdf`, `science_comics_001.cbz`, `nested-macos-compressed.cbz`, `book.opf`, `calibre*.opf`, `example.{avif,jpeg,jxl,png,webp}`, `mock-stump.toml` (helper map `src/lib.rs:246-303`) |
| `cache/` | Untracked, empty, not gitignored. `MediaConfig::default()` resolves `cache_dir` to the relative `cache` (`src/lib.rs:157-171,186`), so tests using the default config from the crate directory can create it; not a source tree |

Consumers (all `default-features = false`, forwarding `pdf`/`rar`):
`core/Cargo.toml:43-45`, `apps/server/Cargo.toml:80-81`,
`crates/graphql/Cargo.toml:38-40`, `crates/scanner/Cargo.toml:11-14`.

## How to verify

```text
cargo test -p stump_media                          # 134 tests with default features (state.mdx:44)
cargo test -p stump_media --no-default-features    # 121 tests; feature_gate_tests assert the typed errors
cargo test -p stump_media --no-default-features --features pdf
cargo test -p stump_media --no-default-features --features rar
cargo check -p stump_server --no-default-features --features minimal   # structural change gate (.omp/PROJECT_STATE.md)
```

PDF tests need `PDFIUM_PATH` (fixture launcher exports `/tmp/libpdfium.so`,
`scripts/dev-fixture-server.sh:113`). Live probe: on the fixture
(`http://127.0.0.1:25600`) fetch a book page (`GET /api/v2/media/{id}/page/1`,
`apps/server/src/routers/api/v2/media.rs:34`) and the Readium manifest
(`GET /api/v2/epub/{id}/manifest.json`, `apps/server/src/routers/api/v2/epub.rs:44-49`),
then run `make replay` and `make replay-readium` from `../komga-compat`, which
exercise page/thumbnail/manifest delivery through the Komga adapter.

## Deep docs

- `docs/content/docs/developer/server-architecture.mdx` (crate boundary, PDFium notes).
- `docs/content/docs/developer/state.mdx:44` (test counts), `sync-platforms.mdx:32-34,129` and `provider-status.mdx:140-146` (KOReader hash compatibility).
- `docs/content/docs/guides/fundamentals/thumbnails.mdx` (ThumbHash/colour placeholders).
- Upstream source at the pinned base: `https://github.com/stumpapp/stump/tree/37fdb7d7/core/src/filesystem`.
