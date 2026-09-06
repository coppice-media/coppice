# stump_kepub

## Purpose

Pure-Rust conversion of an EPUB package into Kobo's KEPUB form: the
`div#book-columns > div#book-inner` wrapper, the `kobostylehacks` style
element, sentence-level `span.koboSpan` anchors numbered `kobo.N.M`, the
polyglot XHTML serialisation, and the OPF rewrite (cover property, Calibre
metadata removal, 4-space re-indent, dummy title page). The crate owns the
transform, its own streaming ZIP writer, and the cache-key derivation
(`cache_file_name`). It deliberately does **not** own delivery: the cache
directory, eviction, warming queue, HTTP serving, and the `KOBO_KEPUB_*`
switches live in `apps/server/src/routers/kobo_backend/{kepub,kepub_cache}.rs`
and `core/src/config/protocols.rs`. It has no database, config, or
`stump_core` dependency.

## Reference / upstream

| Reference | Pin | Used for |
| --- | --- | --- |
| kepubify `kepub` package | https://github.com/pgaskin/kepubify@9546034bc023891af5ce30709de6ae2dcf264628 | Transform semantics, Go test tables, default deflate level 6 |
| kepubify HTML parser fork | `pgaskin/kepubify/_/html@6ee2cc632cdc` | Lenient self-closing repair, BOM/XML-declaration handling, polyglot renderer (`tests/fork_parse_render.rs`) |
| kepubify docs snapshot | https://github.com/pgaskin/kepubify@448534694374e195f8a915c5caf16cdf22ec7792 | Feature description cited in `docs/content/docs/developer/kobo-sync-capabilities.mdx` |
| html5ever 0.26 / markup5ever 0.11 | `Cargo.toml` | HTML5 tree building through a custom `TreeSink` (`src/dom.rs`) |
| libdeflater 1.26 | `Cargo.toml` | Deflate/inflate for rewritten entries |

Parity contract: for every ZIP entry, Stump's decompressed bytes equal
kepubify's. Entry order and compression are Stump's own because kepubify's
ZIP writer is not byte-stable across runs (`tests/transform.rs:104-109`,
`tests/real_books.rs:138-139`).

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| 1:1 port instead of shelling out to a kepubify binary | Removes an external binary and `KOBO_KEPUBIFY_PATH`; conversion parallelises in-process | `apps/server/src/routers/kobo_backend/kepub.rs:226-229`; `docs/content/docs/developer/state.mdx` (§ "Media, scanner, and lifecycle crates", `stump_kepub` row) |
| Entry-level parity, verified 35/35 on a 36-book corpus | kepubify's archive bytes vary per run; one corpus book is rejected by kepubify itself (below) | `tests/real_books.rs:16-19,72-77`; `docs/content/docs/developer/kobo-sync-capabilities.mdx:181-192` |
| Convert OPFs declaring `<?xml version="1.1"?>` (deliberate deviation) | kepubify parses the OPF with Go `encoding/xml`, which rejects XML 1.1 and aborts the book. Stump's OPF pass uses `quick-xml`, which does not validate the version, and re-emits the declaration verbatim; the real-book test reports kepubify's refusal and validates Stump's output structurally instead of skipping the book | `src/lib.rs:1378-1393` (`Event::Decl` → `OpfNode::Declaration`); `src/lib.rs:874-889` (content-document declaration preserved); `tests/real_books.rs:72-80`; `kobo-sync-capabilities.mdx:194-195` |
| Go test tables ported verbatim | Same fixtures as upstream catch drift in any single stage | `tests/transform_content.rs`, `tests/transform_opf_and_files.rs`, `tests/split_sentences.rs`, `tests/convert.rs`; per-stage `parts` API `src/lib.rs:691-752` |
| Arena DOM (`NodeId` indices) with a custom html5ever `TreeSink`; `koboSpan` emitted during serialisation, never materialised as nodes | Avoids `Rc<RefCell>` rcdom traffic and a second tree walk | `src/dom.rs:9-56`; `src/lib.rs:667-683` |
| Lenient self-closing repair done as a byte pre-pass before html5ever | html5ever cannot be configured to accept the fork's `<p/>`, `<div/>`, `<a/>`, `<span/>`, `<title/>`, `<script/>` behaviour | `src/lib.rs:891-899`; `tests/fork_parse_render.rs:16-75` |
| Own streaming ZIP writer instead of the `zip` crate writer | `zip` cannot add pre-compressed entries, which the parallel pipeline needs; untouched entries are sliced from the source by offset and never inflated; only the central directory is retained until `finish` | `src/archive.rs:1-9,84-92,147-148` |
| Bounded rayon batches (`2 × threads`) written in source order | Peak memory is independent of book size; output streams into any `Write` sink (the server writes straight into the cache temp file) | `src/lib.rs:207-213,273-307`; `apps/server/src/routers/kobo_backend/kepub.rs:122-131` |
| libdeflate, default level 6, clamp `1..=12` | Level 6 matches kepubify's default; `1..=12` is libdeflate's supported range and the same bound is validated on the config key | `src/lib.rs:321-322`; `src/lib.rs:98-101,220`; `core/src/config/protocols.rs:36-40,102-105` |
| Compression level is part of the cache key but not of `TransformOptions` | Level changes bytes but not content; callers append `-d<level>` so a level change never serves stale output | `src/lib.rs:93-101`; `apps/server/src/routers/kobo_backend/kepub.rs:75-90` |
| Cache key = `<media_id>-<mtime_ns>-<sha256(options)[..8]>.kepub.epub` | Any byte-affecting option or source replacement selects a new entry; the original EPUB is never mutated | `src/lib.rs:126-168`; `src/lib.rs:1757-1766`; `tests/transform.rs:85-102` |
| Non-content entries copied verbatim (compressed bytes, timestamps, Unix mode) | Preserves images/fonts/CSS byte-for-byte and skips inflate/deflate work | `src/archive.rs:28-42`; `tests/transform.rs:51-52`; `kobo-sync-capabilities.mdx:217-218` |
| kepubify file filter (`__MACOSX`, `calibre_bookmarks.txt`, `iTunes*.plist`, `.DS_STORE`, `thumbs.db`) | Parity with kepubify's entry set | `src/lib.rs:391-413`; `tests/transform_opf_and_files.rs:163-178` |

### Measured speed

Release build, AMD Ryzen 7 5800X, 36-book / 37 MB corpus, best-of-3
process wall-clock including startup; recorded in
`docs/content/docs/developer/kobo-sync-capabilities.mdx:197-215`:

| | Stump | kepubify | ratio | peak RSS Stump / kepubify |
| --- | --- | --- | --- | --- |
| pinned to one core | 1160 ms | 1742 ms | 1.50× | 14 MB / 22 MB |
| 8 cores | 386 ms | 1060 ms | 2.75× | 29 MB / 34 MB |

Server-side observation (debug build, fixture, 5 h uptime): 14.5 MB RSS
idle, 110 MB peak during a parallel KEPUB batch
(`docs/content/docs/developer/client-verification.mdx:35-36`). Re-measure
with `examples/bench_loop.rs` (in-process) or `examples/convert_file.rs`
(streaming to disk).

### Server integration (`KOBO_KEPUB_*`)

| Key | Default | Effect | Evidence |
| --- | --- | --- | --- |
| `KOBO_KEPUB_CONVERSION` | `false` | Convert EPUB at the existing `/kobo/{api_key}/v1/books/{id}/file/epub`; metadata reports `KEPUB`; warms cache on `library/sync` entitlements | `core/src/config/protocols.rs:42-45`; `apps/server/src/routers/kobo_backend/router.rs:246-249,522-525` |
| `KOBO_KEPUB_PRECONVERT` | `false` | Enqueue every EPUB of a library on scan completion | `core/src/config/protocols.rs:27-29`; `kobo_backend/kepub_cache.rs:63-66` |
| `KOBO_KEPUB_CACHE_MAX_AGE_DAYS` | `90` | Age sweep at startup and every 24 h; delivery hits touch the file | `core/src/config/protocols.rs:31-34`; `kobo_backend/kepub_cache.rs:198-199` |
| `KOBO_KEPUB_DEFLATE_LEVEL` | `6` | libdeflate level, validated `1..=12`, appended to the cache name | `core/src/config/protocols.rs:36-40,102-124`; `kobo_backend/kepub.rs:75-90` |

Cache location: `<config_dir>/cache/kepub/<media_id>-<mtime_ns>-<digest>-d<level>.kepub.epub`,
written to a UUID temp name and renamed into place, served with `ServeFile`
(range requests) — `apps/server/src/routers/kobo_backend/kepub.rs:62-139`.

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | Public API (`TransformOptions`, `WriteOptions`, `KepubError`, `transform_epub`, `transform_epub_to`, `convert`, `transform_content`, `transform_opf`, `transform_dummy_titlepage`, `split_sentences`, `cache_file_name`/`cache_key`, `parts`), package discovery, manifest/spine parsing, content pipeline, OPF tree rewrite, dummy title-page heuristic |
| `src/dom.rs` | Arena HTML tree + html5ever `TreeSink`, whitespace normalisation, div wrapper, style insertion, Adept/MS Word cleanup, polyglot serialiser with inline `koboSpan` emission and smartypants |
| `src/archive.rs` | Streaming ZIP writer: stored/deflated/pre-compressed/verbatim-copied entries, `EntryMeta`, `EntryLocation`, central directory |
| `examples/convert_file.rs` | Stream one EPUB to a `.kepub.epub` file |
| `examples/bench_loop.rs` | In-process timing loop |
| `examples/dump_entry.rs` | Print one transformed entry |
| `tests/transform_content.rs`, `tests/transform_opf_and_files.rs`, `tests/split_sentences.rs`, `tests/convert.rs` | kepubify Go test tables |
| `tests/fork_parse_render.rs` | Parser-fork behaviours (self-closing, BOM, escaping, polyglot output) |
| `tests/transform.rs` | Bundled fixture: determinism, raw-copy of non-content entries, cache-key behaviour, `kepubify_corpus_matches_reference` against `tests/fixtures/kepubify/` |
| `tests/real_books.rs` | Ignored differential test over a local corpus (`KEPUB_REAL_INPUT`, optional `KEPUBIFY_BIN`) |

## How to verify

```text
cargo test -p stump_kepub --lib --tests                       # gate line in .omp/PROJECT_STATE.md
KEPUB_REAL_INPUT=<dir> KEPUBIFY_BIN=<kepubify@9546034> \
  cargo test -p stump_kepub --test real_books -- --ignored --nocapture
cargo run --release -p stump_kepub --example convert_file -- in.epub out.kepub.epub
cargo run --release -p stump_kepub --example bench_loop -- in.epub 20
```

Live probe (fixture launcher exports `KOBO_KEPUB_CONVERSION=true`,
`scripts/dev-fixture-server.sh:112`): `GET $BASE/kobo/$KEY/v1/books/<id>/file/epub`
with a `Range` header returns `application/epub+zip`, a `.kepub.epub`
attachment name, and a `206`; the file lands under `<config>/cache/kepub/`.
The Kobo state round trip is scripted in
`docs/content/docs/developer/kobo-sync-capabilities.mdx:250-296`. There is no
Hurl target in `../komga-compat` for KEPUB delivery; the harness covers Komga,
Kavita, Komf, Mihon, Readium, and liseur-sync only.

## Deep docs

- `docs/content/docs/developer/kobo-sync-capabilities.mdx` §3 "Stump's optional converter and cache" (parity, performance, design, cache lifecycle).
- `docs/content/docs/developer/state.mdx` (crate table), `provider-status.mdx` (Kobo/KEPUB rows), `client-verification.mdx` (device row, memory).
- `docs/content/docs/guides/configuration/server-config.mdx` (`KOBO_KEPUB_*`).
- `docs/content/docs/developer/hardcover-integration.mdx` (why `koboSpan` IDs are not a portable locator).
