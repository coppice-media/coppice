# stump_media

## Purpose

File and image processing for Stump: the per-format `FileProcessor`
implementations (`ZipProcessor`, `EpubProcessor`, `MobiProcessor`,
`PdfProcessor` behind `pdf`, `RarProcessor` behind `rar`), `ContentType`
detection, the unified `FileError`,
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
| PDFium | `pdfium-render 0.9.1` (`Cargo.toml:28`), library loaded from `MediaConfig.pdfium_path` (`PDFIUM_PATH`) | Only PDF backend; see `docs/content/docs/developer/server-architecture.mdx` (§ "Metadata and PDF caveats" → "PDF") |
| DRM/encryption markers | EPUB 3.3 §4.2.6.3.2 / §4.4.5 <https://www.w3.org/TR/epub-33/>; epubcheck `OCFEncryptionFileHandler` <https://github.com/w3c/epubcheck>; MobileRead <https://wiki.mobileread.com/wiki/MOBI> and <https://wiki.mobileread.com/wiki/PDB>; ISO 32000-1 §7.5.5; DeDRM_tools `v10.0.3` (`epubtest.py` is Unlicense) | Documented byte/path markers only, reimplemented in `src/drm.rs`; no GPL code is copied and nothing here decrypts anything. Prose in `docs/content/docs/developer/calibre-tooling.mdx` |
| MOBI / KF8 | MobileRead <https://wiki.mobileread.com/wiki/MOBI>, <https://wiki.mobileread.com/wiki/PDB>, <https://wiki.mobileread.com/wiki/PalmDOC>, <https://wiki.mobileread.com/wiki/KF8>; HUFF/CDIC and KF8 skeleton/fragment notes from Kindling's `README.md` (MIT, <https://github.com/ciscoriordan/kindling>, linked from the MobileRead MOBI page as further format documentation) | `src/media/format/mobi.rs` is clean-room from those descriptions plus the record bytes of the committed fixtures; nothing is derived from calibre, KindleUnpack, libmobi or boko, and every structure cites the page it comes from |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| `TransformProfile` carries an `audio: AudioProfile` section whose `output` is `Passthrough` (default) or `Opus { bitrate }`, with its own `AudioProfile::digest()` | Audio delivery is per device like page delivery, so it belongs on the same profile; a separate digest lets two devices whose page settings differ but whose audio matches share one transcode. Adding the field changed `TransformProfile::digest()`, so existing comic cache entries miss once and rebuild | `src/transform/profile.rs`; `TransformCache::audio_path_for`; tests `audio_defaults_to_passthrough_everywhere`, `digest_separates_audio_outputs` |
| Opus is a delivery preset only — never a value `STUMP_AUDIO_CANONICAL` accepts — and its bitrate is parsed and bounded to 500..=512000 bit/s in `from_device_profile` | An unparseable `-b:a` would otherwise surface as a failed `ffmpeg` child once per track request, long after the profile was stored; a stored-Opus option would make the archive copy depend on which device asked first | `AudioProfile::validate`, `parse_bitrate`; `core/src/config/audio.rs` `validate_audio_canonical`; test `device_profile_rejects_an_unusable_opus_bitrate` |
| Processors take an immutable `MediaConfig` snapshot instead of `StumpConfig`/`Ctx` | Paths (`cache`, `thumbnails`, `cache/pdf_pages`) are resolved once in `from_values`; per-call code neither clones config nor rebuilds paths; crate compiles without core | `src/lib.rs:137-203`; `docs/content/docs/developer/server-architecture.mdx` (§ "Package seams" → `stump_media`) |
| `pdf` and `rar` are Cargo features (default on) rather than runtime switches | They remove `pdfium-render`/`unrar` from the build; core and server forward them (`stump_core/pdf`, `stump_media/pdf`, ...) | `Cargo.toml:5-9,28,43`; `core/Cargo.toml:13-14`; `apps/server/Cargo.toml:13-15` |
| Feature-off dispatch returns typed errors, not panics | `Rar` → `FileError::UnsupportedFileType("RAR support is disabled")`, `Pdf` → `FileError::PdfConfigurationError`; verified by `feature_gate_tests` | `src/media/process.rs:190-220`; `src/media/process.rs` `feature_gate_tests`; `src/error.rs:24-56` |
| `models` with `default-features = false` | `models` only has the optional `graphql` feature; a leaf crate must not pull `async-graphql` | `Cargo.toml:26`; `crates/models/Cargo.toml:5-10` |
| Stump hash samples the file (4 × 10 000 B + tail) once it exceeds 40 000 B | Cheap identity for large archives; full digest for small files | `src/hash.rs:13-14,32-60` |
| KOReader hash is a literal port of the Lua loop (1 KiB reads at `1024 << i`, `i = -1..=10`) | KOReader identifies documents by this digest; sync only works if both sides agree | `src/hash.rs:62-99`; vectors `src/hash.rs:117-130`; caveat for short final reads `docs/content/docs/developer/provider-status.mdx:140-146` |
| `DUPLICATE_PAGE_TOLERANCE = 4` lives next to `hamming`, not in a consumer | The ingest quality check (`stump_ingest`), the duplicate-page candidate aggregation (GraphQL) and the serve-time page skip (`stump_core::filesystem::media::visible_pages`) must agree on what "the same page" is; `stump_ingest` cannot depend on `stump_core`, so a shared constant here is the only way to keep one value | `src/hash.rs` (below `hamming`); consumers `crates/ingest/src/quality/duplicate_pages_across_books.rs`, `crates/graphql/src/query/duplicate_page.rs`, `core/src/filesystem/media/visible_pages.rs` |
| `move_file` (rename, falling back to copy + remove) lives in `common` | Library roots, staging roots and drop folders are routinely on different filesystems, where `rename` fails with `EXDEV`; `stump_ingest` and `stump_library` both need it and only one of them can see `stump_core`, so a single implementation belongs in the file-handling crate | `src/common.rs::move_file`; consumers `crates/ingest/src/store.rs::approve`, `crates/library/src/series.rs::apply_moves` |
| `ContentType::from_bytes_with_fallback` prefers a more specific extension subtype (EPUB/CBZ over generic ZIP) before `infer` | Byte sniffing cannot distinguish EPUB/CBZ from ZIP | `src/content_type.rs:137-185`; tests `src/content_type.rs` |
| EPUB search is bounded and uncached (query 2–128 chars, limit 20/max 50, ≤128 spine items, 32 MiB total / 16 MiB per item, 20 matches per spine item) | Keeps a single request's memory and CPU predictable without an index | `src/media/epub_search.rs:1-5,26-35` |
| Image crate limited to GIF/JPEG/PNG/WebP; colours via `kmeans_colors` (7 + average), ThumbHash ≤100 px, WebP encoder quality 100 | Smaller build; placeholder metadata computed once per thumbnail | `Cargo.toml:18,22,38,46`; `src/image/placeholder.rs:90-104,142-190,216-230`; `src/image/webp.rs:17-83` |
| PDF page cache/prerender lives here, keyed under `MediaConfig.pdf_cache_dir` | Rendering is the expensive step; `pdf_cache_pages`, `pdf_prerender_range`, `pdf_render_dpi`, `pdf_max_dimension`, `pdf_render_format` are plain config values | `src/lib.rs:143-149,225-243`; `src/media/format/pdf.rs:360-389,432-467` |
| `transform` module: `TransformProfile` + device presets (`clara`, `libra`, `sage`, `elipsa`, `nia`, `*-colour`, `koreader`, `phone`), decode → tall-page split → resize → tone LUT → encode on `concurrency` threads with in-order output; sources are `Iterator<Item = TransformResult<Vec<u8>>>` so a bad page fails alone | One CPU pipeline for every device; never upscales; Kobo presets are grayscale JPEG because Kobo renders no WebP | `src/transform/{profile,pipeline,source}.rs`; tests `transform::pipeline::tests`, `transform::source::tests` |
| Features `fast-resize` (`fast_image_resize` Lanczos3), `turbojpeg` (libjpeg-turbo), `kepub` (`stump_kepub` content transform), default on; feature-off falls back to `image` CatmullRom / `image` JPEG, and `build_kepub` returns `FeatureDisabled("KEPUB")` | They remove native code from the build; server forwards them as `transform` | `Cargo.toml:6-12`; `src/transform/pipeline.rs` (`cfg(feature = ...)`); `src/transform/container.rs` `kepub_is_disabled_without_the_feature` |
| Containers stream: CBZ stored with `ComicInfo.xml` first; KEPUB writes `mimetype` (stored), pages as they arrive, `content.opf` last | A whole comic is never held in memory | `src/transform/container.rs` `build_cbz`/`build_kepub`; tests `kepub_container_is_structurally_valid` |
| `TransformCache`: `<media-id>-<mtime ns>-<profile digest>.<ext>`, UUID `.tmp` → rename, hits touch mtime, LRU sweep to a byte budget skipping `.tmp` | Same discipline as the KEPUB cache; profile changes never serve stale bytes | `src/transform/cache.rs`; `src/transform/profile.rs` `digest` |
| Fixtures `scrambled.cbr` (10/2/1.jpg + `ComicInfo.xml` + `__MACOSX` shadow, scrambled order) and an in-test scrambled CBZ prove natural page order and hidden-entry skipping | The shipped RAR/ZIP fixtures hold only `.ico` files | `src/transform/source.rs` tests; `integration-tests/data/scrambled.cbr` |
| `src/drm.rs` detects protection but never removes it, and treats the two font-obfuscation algorithms as *not* DRM | `META-INF/encryption.xml` is also how EPUB font obfuscation is declared, so flagging its mere presence would fail every obfuscated-font book; epubcheck classifies exactly `http://www.idpf.org/2008/embedding` and `http://ns.adobe.com/pdf/enc#RC` as font mangling and everything else as encryption it cannot decrypt | `src/drm.rs` `FONT_OBFUSCATION_ALGORITHMS`, `font_obfuscation_only_is_not_drm`, `content_encryption_without_a_vendor_marker_is_reported` |
| A MOBI/AZW verdict comes from the PalmDOC encryption type (and the MOBI DRM key block), never from EXTH `209` alone | EXTH 209/1/2/3 survive in DRM-free Amazon files, while encryption type `0` proves the text records are plaintext; the EXTH types are still recorded as evidence | `src/drm.rs` `detect_mobi_drm`/`mobi_exth_drm_records`, `unencrypted_mobi_is_not_drm_even_with_tamper_proof_keys` |
| Containers are sniffed by magic bytes, and a `{adept}user` UUID is deliberately never recorded | A MOBI named `.epub` must still be classified; the ADEPT user UUID identifies the purchaser's account and quality reports are persisted evidence | `src/drm.rs` `detect_drm`/`adept_encrypted_key`, `detect_drm_dispatches_on_container_magic_not_extension` |
| One `MobiProcessor` reads MOBI 6 and KF8, and a dual-format file is read through its KF8 half | Both are the same Palm database and `infer` reports both as `application/x-mobipocket-ebook`; EXTH `121` names the KF8 boundary, and every record number in that part's header is relative to it | `src/media/format/mobi.rs` `MobiBook::open`, `a_dual_format_file_reads_its_kf8_half` |
| A Kindle page count is the section count, not a synthetic byte budget like an EPUB's | A Kindle book is one compressed text stream: there is no per-document compressed size to divide, and the reassembled sections already *are* the reading unit | `src/media/format/mobi.rs` `get_page_count`; contrast `src/media/format/epub.rs` `spine_page_budget` |
| The PalmDOC header's `text length` is recorded but never used to truncate | A KF8 producer may count only the markup flow, leaving the style-sheet flow past the declared end; `FDST` delimits KF8 flows and `</body>` delimits MOBI 6 content | `src/media/format/mobi.rs` `read_text`, `kf8_sections_are_reassembled_from_the_skeleton_and_fragment_indices` |
| MOBI 6 splitting and link rewriting work on raw bytes, not on decoded text | A `filepos` link is a *byte* offset into the text; decoding CP1252 first moves every offset past the first non-ASCII character | `src/media/format/mobi.rs` `build_mobi6_sections`, `mobi6_recindex_images_and_filepos_links_are_rewritten`, `cp1252_text_is_decoded_through_the_windows_high_block` |
| A protected Kindle file is refused with `FileError::DrmProtected` carrying `detect_mobi_drm`'s own reason, and the KF8 half's encryption type is checked separately | Its text records are ciphertext, so decompressing them yields garbage rather than a book; the detector only reads record 0 | `src/media/format/mobi.rs` `MobiBook::open`, `a_drm_protected_book_is_refused_with_the_detectors_reason` |
| Audio probing takes two libraries: `symphonia` for every fact, `mp4ameta` for MP4 chapters | `symphonia` demuxes every container the audio module accepts and is the single source of duration, codec, sample rate, channels, tags and cover, and it reads ID3v2 `CHAP`/`CTOC` plus `CHAPTERxxx` Vorbis chapters — but `symphonia-format-isomp4` 0.6 reads neither a Nero `chpl` atom nor a QuickTime chapter track, and `mp4ameta` is the only library that tells those two mechanisms apart, which is exactly the `chapter_source` provenance Stump stores | `src/audio/mod.rs` ("Why two libraries"); `src/audio/probe.rs` `mp4_chapters`/`container_chapters`; `src/audio/tests.rs` `audio_chapter_source_names_the_container_mechanism`; fixtures `integration-tests/data/audio/chapters-{chpl,track}.m4b` |
| An audiobook is processed by `AudioProcessor` (`media/format/audio.rs`), a `FileProcessor` like every other format; **page 1 is its cover** (embedded art, else a `cover.*`/`folder.*` sidecar beside a folder book), every other page-oriented call returns `FileError::UnsupportedFileType`, and `get_page_count` returns `Ok(-1)` | `process()` is the scanner's only entry point, so a shape that cannot be processed cannot be scanned. Page 1 is what `get_media_thumbnail` and every profile's cover route (Komga, Kavita, OPDS, ABS) ask for when a book has no stored thumbnail — the same contract the EPUB processor honours — and the ABS `GET /api/items/{id}/cover` returned 500 on every audiobook before it did. A count is a fact about a book with no pages, another page is not, and a plausible wrong page would hide a routing bug | `src/media/format/audio.rs` `cover`, `AudioProcessor::get_page`/`get_page_content_types`; tests `audio_processor_serves_the_cover_as_page_one_and_nothing_else`, `audio_processor_reads_a_folder_cover_sidecar`; `apps/server/src/routers/api/v2/media.rs::get_media_thumbnail` |
| `determine_processor` routes to `Audio` on `ContentType::from_path().is_audio()` *or* `PathUtils::dir_is_audio_book(&GlobSet::empty())` | A folder book has no mime of its own, and reusing the walker's predicate keeps one definition of "audiobook folder"; ignore rules decide what the walker *offers* the processor, never what a folder physically is | `src/media/process.rs:173-185`; tests `media::process::tests::test_determine_processor_*` |
| `ProcessedFile.audio: Option<AudioFacts>` and `pages == -1` for audio | The write path takes the facts straight to `models::services::audio::replace_in`, and `media.pages` is already documented as `-1` for media with no pages | `core/src/filesystem/media/builder.rs:39-40`; `crates/models/src/entity/media.rs:39`; `src/media/format/audio.rs` tests |
| A folder book hashes its **first part** in natural filename order, for both the Stump and the KOReader hash | A directory has no bytes to sample, and the filename order is a pure function of the folder's contents — unlike playback order, which a complete `TRCK` numbering can permute — so the same folder always samples the same file at no probe cost | `src/media/format/audio.rs` `AudioProcessor::hashed_file`/`generate_hashes`; `src/audio/probe.rs` `audio_files_in` |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | Module tree, re-exports, async `resize_image`, `MediaConfig` |
| `src/media/process.rs` | `FileProcessor` / `FileConverter` traits, `FileProcessorOptions`, `ProcessedFile`, `ProcessedFileHashes`, MIME+extension dispatch, sync/async wrappers |
| `src/media/format/{zip,epub,mobi,pdf,rar}.rs` | Per-format processors; `pdf`/`rar` are `cfg`-gated in `format/mod.rs` |
| `src/media/format/mobi.rs` | `MobiProcessor` plus the `MobiBook` model (PDB records, MOBI/EXTH headers, PalmDOC and HUFF/CDIC decompression, trailing entries, INDX/TAGX, KF8 `FDST`/skeleton/fragment reassembly, `kindle:` link rewriting). Consumed by `stump_tools`' `mobi2epub` |
| `src/media/format/audio.rs` | `AudioProcessor`: the `FileProcessor` for a single audio container or a folder book, its hashed-file rule, the cover-as-page-1 rule, and the typed refusals for every other page route |
| `src/audio/{mod,probe}.rs` | `probe`/`probe_file`/`probe_folder`/`audio_files_in`, `ProbedAudio`/`ProbedTrack`/`ProbedChapter`, `ChapterSource` provenance, `AudioFacts` conversion. Fixtures in `integration-tests/data/audio/`; user-facing prose in `docs/content/docs/guides/fundamentals/audiobooks.mdx` |
| `src/media/metadata.rs`, `src/media/utils.rs`, `src/serde.rs` | ComicInfo / OPF / PDF metadata normalisation, cover-name rules, PDF date parsing, custom deserialisers |
| `src/media/readium.rs` | `ReadiumManifestGenerator` (manifest, positions, resource URLs) |
| `src/media/epub_search.rs` | `search_epub` with cursor, limits, Readium locators |
| `src/content_type.rs`, `src/error.rs` | `ContentType` detection/predicates, `FileError` |
| `src/drm.rs` | `detect_drm` container sniffing plus the EPUB/MOBI/PDF/Topaz/KFX marker rules, `DrmReport`; detection only, never removal |
| `src/hash.rs` | `generate` (sampled SHA-256), `generate_koreader_hash` |
| `src/common.rs`, `src/directory_listing.rs`, `src/archive.rs`, `src/series_metadata.rs` | Thumbnail lookup, `PathUtils`/`FileParts`, directory listing DTOs, ZIP creation, `series.json` |
| `src/image/*` | `ImageProcessor` trait, `GenericImageProcessor` (JPEG/PNG), `WebpProcessor`, placeholder metadata (colours, ThumbHash), thumbnail file helpers |
| `integration-tests/data/` | Fixtures: `book.{zip,rar,epub}`, `book-complex-tree.{zip,rar}`, `book-zip-mime.epub`, `book-image-format-test.zip`, `leaves.epub`, `kindle-formats.azw3` (13 KB, `boko convert` from a purpose-built EPUB: 4 KF8 sections, 2 PNG resources, 1 style flow, 3 NCX entries), `tall.pdf`, `rust_book.pdf`, `science_comics_001.cbz`, `nested-macos-compressed.cbz`, `book.opf`, `calibre*.opf`, `example.{avif,jpeg,jxl,png,webp}`, `mock-stump.toml` (helper map `src/lib.rs:246-303`) |
| `cache/` | Untracked, empty, not gitignored. `MediaConfig::default()` resolves `cache_dir` to the relative `cache` (`src/lib.rs:157-171,186`), so tests using the default config from the crate directory can create it; not a source tree |

Consumers (all `default-features = false`, forwarding `pdf`/`rar`):
`core/Cargo.toml:43-45`, `apps/server/Cargo.toml:80-81`,
`crates/graphql/Cargo.toml:38-40`, `crates/scanner/Cargo.toml:11-14`.

## How to verify

```text
cargo test -p stump_media                          # default features: pdf, rar, turbojpeg, fast-resize, kepub
cargo test -p stump_media --no-default-features    # feature_gate_tests assert the typed errors
cargo test -p stump_media --no-default-features --features pdf
cargo test -p stump_media --no-default-features --features rar
cargo test -p stump_media --no-default-features --lib drm   # 23 DRM-detector tests, synthetic fixtures only
cargo test -p stump_media --no-default-features --lib mobi  # Kindle reader; the AZW3 fixture plus byte-built MOBI 6/KF8 headers
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
- `docs/content/docs/developer/state.mdx` (§ "Gate at this commit" — workspace gate totals), `sync-platforms.mdx:32-34,129` and `provider-status.mdx:140-146` (KOReader hash compatibility).
- `docs/content/docs/guides/fundamentals/thumbnails.mdx` (ThumbHash/colour placeholders).
- `docs/content/docs/guides/fundamentals/audiobooks.mdx` (accepted extensions, folder-book detection, track order, the `chapter_source` provenance table, publication-relative milliseconds).
- Upstream source at the pinned base: `https://github.com/stumpapp/stump/tree/37fdb7d7/core/src/filesystem`.
