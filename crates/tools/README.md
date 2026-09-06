# `stump_tools`

## Purpose

Library maintenance tools as in-process pipeline modules: the Kavita "external
tools" feature set, re-implemented in Rust. Every tool is a `Tool` with a pure
`plan` (reads only, returns a printable `Plan`) and an `apply` that performs
exactly the planned actions, so a dry run is a truthful preview and an original
file is only ever touched when the caller applies.

Deliberately not here: job scheduling, database access, HTTP routes. A tool
takes paths plus a JSON options blob and returns data; `crates/cli`
(`stump tools list|plan|apply`) is the only front end today.

## Reference / upstream

| What | Where | How it is used |
| --- | --- | --- |
| Kavita external tools catalogue | <https://wiki.kavitareader.com/guides/external-tools/> | Behaviour list only (which tools exist, what each is for). No code from those (GPL/AGPL) projects is copied; every rule is re-derived and tested here, and the doc comment of each module cites what it mirrors. |
| ComicInfo v2.0 schema | <https://anansi-project.github.io/docs/comicinfo/schemas/v2.0> | Element names and order for generated `ComicInfo.xml` |
| EPUB 3.3 | <https://www.w3.org/TR/epub-33/> | Spine order and `href` resolution (`epub2cbz`); envelope/package conformance rules (`epub-check`) |
| calibre manual (9.14.0) | <https://manual.calibre-ebook.com/generated/en/cli-index.html> | `ebook-convert`/`ebook-meta` invocation and flags, and the `%prog (calibre <version>)` banner parsed by `calibre::locate`. calibre is GPL-3: it is executed as a separate process only, never linked, vendored, or bundled. See `docs/content/docs/developer/calibre-tooling.mdx` |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| A plan is self-contained — everything `apply` needs (page order, flags, thresholds) lives in `Action.detail` — and `apply` still re-checks `target.exists()` | `apply(&Plan, &mut dyn ProgressSink)` never sees `ToolInput`, so the dry run the user approved is exactly what runs; a plan can also be minutes old, and `overwrite` is the only license to replace a file | `src/lib.rs` `Tool`; `src/cbzit.rs` `PackDetail`, `existing_archives_are_never_clobbered_without_overwrite`; `src/epub2cbz.rs` `ConvertDetail` |
| Recoverable per-file problems are `Warning`s (plan) or `Report::skipped` (apply); `ToolError` is only for aborting the whole run | Bulk runs over a library must not die on one bad file | `src/epub2cbz.rs` `bulk_over_a_folder_skips_prose_epubs_with_a_warning` |
| Output is staged in a sibling `.stump-tools-*.tmp` and renamed | A crash or mid-write error never leaves a truncated archive, and the original is untouched until the new bytes are complete | `src/util.rs` `write_atomic`, `write_atomic_leaves_no_temp_file_and_keeps_the_original_on_error` |
| CBZ output matches the containers Stump already writes: `ComicInfo.xml` first, `CompressionMethod::Stored`, root-level pages `0001.ext` (1-based, 4-wide) | One page-naming convention across the codebase; pages are already compressed images | `src/util.rs` `write_cbz`; `crates/provider/src/host.rs:601`; `crates/media/src/transform/container.rs:58` |
| Page detection and page order reuse `stump_media::PathUtils::{is_img, is_hidden_file}` and `alphanumeric_sort` | Tools must see exactly the pages the scanner and processors see | `src/util.rs` `is_page_file`/`sorted_files`; `crates/media/src/media/utils.rs:31` |
| Sequence numbers are parsed only by `stump_scanner::{parse_identifier_of, clean_name}`; `cbzit` owns just the leftover title text | One filename grammar in one place (agreed with the `missing-sequence` work) | `src/cbzit.rs` `identify`/`leftover`, `names_round_trip_through_the_scanner_parser` |
| Generated `ComicInfo.xml` writes the language as `<LanguageISO>`, accepting that it does not round-trip into Stump's own metadata | `LanguageISO` is the schema element every other reader (Kavita, Komga, ComicRack) uses; `stump_media`'s reader only aliases `<Language>`, and fixing that means editing another crate's metadata contract — flagged rather than silently emitting a non-schema element | `src/util.rs` `ComicInfo::to_xml`, `generated_comic_info_parses_as_stump_media_metadata`; `crates/media/src/media/metadata.rs:97-98` |
| `missing-sequence` is report-only: `apply` writes nothing and re-emits the plan's actions as `applied` | The finding *is* the deliverable, and a report tool with a mutating apply would make `plan` a lie; the CLI still gets one uniform `plan`/`apply` shape | `src/missing_sequence.rs` `apply_writes_nothing_and_re_emits_the_findings` |
| Gaps are computed per folder over `PathUtils::is_default_ignored`-filtered files, never per file | A gap only exists relative to siblings, and covers/dotfiles/unsupported formats are not books | `src/missing_sequence.rs` `ignored_files_do_not_influence_the_sequence`, `collect_folders` |
| Front/back covers are entries named `!000-cover.<ext>`/`zzz-back.<ext>`, not Kavita's `cover.*` | Stump numbers archive pages purely by alphanumeric entry-name order and has no in-archive cover-name rule (`is_accepted_cover_name` only matches sidecar files in a series folder), so the name itself has to sort to the edge | `src/cbz_covers.rs` `FRONT_STEM`/`BACK_STEM`, `adds_front_and_back_covers_at_the_page_edges`; `crates/media/src/media/format/zip.rs:194-229`; `crates/media/src/common.rs:231-239` |
| Entries this tool adds are recorded in a hidden `.stump-covers.json` inside the archive, never in `ComicInfo.xml` | `remove_added` must strip exactly what the tool added and leave hand-made covers alone; the leading dot keeps the manifest out of the page count, and a metadata tool rewriting `ComicInfo.xml` cannot destroy it | `src/cbz_covers.rs` `MARKER_ENTRY`, `remove_added_restores_a_byte_identical_page_list`, `remove_added_leaves_pages_the_tool_did_not_add` |
| Pure additions append via `ZipWriter::new_append`; any removal, or any entry whose local header sets the data-descriptor bit, rewrites via `raw_copy_file` | Appending leaves every existing byte (extra fields, comments) alone, but it re-emits central headers whose general purpose flag is derived from the file name only — a bit-3 archive would come out self-contradictory. Neither path recompresses a retained page | `src/cbz_covers.rs` `apply_archive`/`append_entries`/`rewrite`, `an_archive_with_data_descriptors_is_rewritten_not_appended`; `zip-1.1.3/src/write.rs:1552-1558` |
| Cover precedence is manual > auto > global, and `auto_assign_dir` matches volume tokens with the ingest filename grammar's volume subset | Same precedence as the reference tool; one volume grammar in the codebase, so `v01`/`vol.7`/`volume 12` mean here what they mean during a scan | `src/cbz_covers.rs` `resolve_front`/`VOLUME_TOKEN`, `cover_precedence_is_manual_then_auto_then_global`, `volume_token_grammar`; `core/src/ingest/quality/filename.rs:272-275` |
| The calibre adapters carry their own `bin_dir` option instead of reading a `STUMP_CALIBRE_BIN_DIR` config key | `stump_tools` must stay free of `stump_core`, so there is no config to read; resolution is `bin_dir`, else `PATH`, else the macOS bundle dir, and a version below 7.0 is `ExternalToolMissing` with an install hint | `src/calibre.rs` `Calibre::locate`/`MIN_VERSION`, `locate_accepts_a_supported_version_and_rejects_an_old_one`, `a_missing_binary_is_reported_with_an_install_hint` |
| Child stdout/stderr go to temp files, not pipes, and every run has a finite timeout | A parent that polls `try_wait` and only reads pipes after exit deadlocks the moment a chatty conversion fills the pipe buffer; a hung `ebook-convert` must not hold a library run open forever | `src/calibre.rs` `run`, `apply_times_out_and_reports_it` |
| A failed, timed-out, or output-less conversion deletes the target file and becomes a `skipped` entry carrying the exit status and stderr tail | A truncated book in a library folder is worse than no book, and the planner already refuses pre-existing targets, so anything present after a failure is ours to remove | `src/calibre.rs` `CalibreConvert::apply`, `apply_captures_stderr_and_removes_a_broken_output` |
| `calibre-meta` is read-only and emits `ExternalMediaMetadata`-shaped JSON with only populated fields | Writing metadata mutates the original and needs the same explicit-apply story as the mutating tools; omitting empty fields means a later merge can never blank a value it has no opinion on | `src/calibre.rs` `CalibreMetadata`/`parse_opf`, `opf_is_mapped_onto_the_external_metadata_shape`; `crates/integrations/metadata/src/types/metadata.rs` |
| `epub-check --fix` relays out the whole archive — `mimetype` written first, stored, exact — and raw-copies every other entry, re-serialising only the package document | The OCF rules are about entry *order* and *compression*, which an in-place append cannot change; raw copy means a repair provably cannot alter a content document, image, font or style sheet | `src/epub_check.rs` `repair_epub`, `repaired_epub_is_clean_and_readable`, `compressed_and_padded_mimetype_is_normalized` (asserts `data_start() == 30 + 8`, i.e. no extra field) |
| Findings are convergent: nav/NCX/cover are judged over the items that survive a repair, and a dropped manifest item takes its spine `itemref` and `spine/@toc` with it | Re-checking a repaired file must report nothing new, or `fix` never terminates and the tool blames itself for its own edits | `src/epub_check.rs` `check_manifest`/`check_navigation`, `dropping_a_manifest_item_drops_its_spine_itemref`, `dropping_the_ncx_clears_the_spine_toc_pointer` |
| Only mechanical defects are `fixable` (entry layout, `dc:language` `und`, a `urn:uuid` v4 identifier under a collision-free id, dropping items with no file, adding `cover-image`); a missing title, nav document, NCX or non-empty spine is reported and never invented | Guessing a title or synthesising navigation means writing a content document, which this tool refuses to do; the OPF is rewritten event-by-event so declaration, comments, attribute order and indentation survive, and the inserted `dc:` prefix is copied from the document rather than assumed | `src/epub_check.rs` `rewrite_opf`/`DcNaming::detect`, `epub2_without_title_or_ncx_reports_unfixable_findings`, `a_spine_with_no_readable_document_is_reported`, `missing_identifier_and_cover_property_are_repaired` |

## Tools

Each tool owns one `### <id>` subsection: what it does and refuses to do, its
options (`Option | Type | Default | Effect`), then its action kinds and warning
codes. Subsections are sorted by tool id as ASCII (`-` sorts before digits and
letters, so `cbz-covers`, `cbzit`, `epub-check`, `epub2cbz`,
`missing-sequence`). This section is the one part of the crate README that is
exempt from the 120-line template budget: the tool contract requires one table
row per option, so the file grows with the tool count. Keep each subsection to
8-14 lines.

### `calibre-convert`

Converts e-books with the operator's own `ebook-convert`. calibre is GPL-3, so
it is only ever executed as a subprocess: nothing is linked, vendored, or
installed by Stump, and both calibre tools fail with an install hint when it is
absent or older than 7.0. It removes no DRM and never will; `ebook-convert`
refuses protected input and ingest rejects it earlier through `drm_protected`.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `to` | string | `epub` | Output format: `epub`, `kepub`, `azw3`, `mobi`, `pdf`; also the target extension `ebook-convert` selects its output plugin from |
| `output_dir` | path | the source's directory | Where converted files are written |
| `extra_args` | string[] | `[]` | Verbatim extra `ebook-convert` arguments, e.g. `--smarten-punctuation` |
| `bin_dir` | path | `PATH`, then the macOS bundle dir | Directory holding `ebook-convert` |
| `timeout_secs` | int | `600` | Per-file wall clock; the child is killed past it |
| `overwrite` | bool | `false` | Replace an existing target instead of skipping it |

`to: "kepub"` writes `Book.kepub`; Kobo sideloading wants it renamed to
`Book.kepub.epub`. Prefer the in-process `stump_kepub` converter for EPUB
sources.

Actions: `convert`. Warnings: `calibre-version`, `not-a-file`, `no-file-stem`,
`already-target-format`, `target-is-source`, `target-exists`,
`calibre-stderr`.

### `calibre-meta`

Reads embedded metadata with `ebook-meta --to-opf` and maps the OPF onto
`ExternalMediaMetadata`-shaped JSON in each applied action's `detail`
(`title`, `writers`, `summary`, `tags`, `year`/`month`/`day`, `isbn`/`isbn_13`,
`series_name`/`number`, plus `publisher` and `language`). Read-only: it never
writes metadata back into a book.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `bin_dir` | path | `PATH`, then the macOS bundle dir | Directory holding `ebook-meta` |
| `timeout_secs` | int | `120` | Per-file wall clock; the child is killed past it |

Actions: `read-metadata`. Warnings: `calibre-version`, `not-a-file`.

### `cbz-covers`

Sets a front and/or back cover on CBZ archives, drops the current first/last
page, bulk-assigns loose cover images by volume number, and undoes its own
earlier runs. It never deletes a page it did not add unless asked explicitly:
`remove_added` strips only the entries listed in the archive's
`.stump-covers.json`, and pages removed by `delete_first`/`delete_last` are
gone for good.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `front` | path | none | Image to insert as the first page (`!000-cover.<ext>`) |
| `back` | path | none | Image to insert as the last page (`zzz-back.<ext>`) |
| `delete_first` | bool | `false` | Drop the current first page (after `remove_added`) |
| `delete_last` | bool | `false` | Drop the current last page (after `remove_added`) |
| `remove_added` | bool | `false` | Strip the entries a previous run added, restoring the original pages byte for byte |
| `auto_assign_dir` | path | none | Directory of loose images matched to archives by volume token (`v01`, `vol.7`, `volume 12`); fronts only |

Within one archive the order is always `remove_added`, then the deletions, then
the additions. A single target path makes `front`/`back` a *manual*
assignment; the same option over several archives is the *global* fallback.

Actions: `remove-added`, `delete-page`, `add-front-cover`, `add-back-cover`.
Warnings: `missing-path`, `unsupported-path`, `no-archives`,
`unreadable-archive`, `no-pages`, `unsupported-cover-image`,
`cover-entry-replaced`, `cover-not-at-edge`, `auto-assign-no-volume`,
`auto-assign-ambiguous`, `auto-assign-duplicate-volume`,
`auto-assign-unmatched`.

### `cbzit`

Packs folders of page images into cleanly named CBZs; `merge` consolidates
chapter CBZs into per-volume ones. Names are
`Series v01 c003 - Title [en].cbz` (volume 2-wide, chapter 3-wide, decimals
kept) and parse back through the scanner's grammar.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `output_dir` | path | the input root | Where archives are written |
| `series` | string | detected | Override the detected series name |
| `language` | string | detected | Override the detected language tag |
| `overwrite` | bool | `false` | Replace existing archives instead of skipping |
| `comic_info` | bool | `true` | Write `ComicInfo.xml` for the detected identity |
| `min_pages` | int | `1` | Folders with fewer images are reported, not packed |
| `merge` | bool | `false` | Merge chapter `.cbz` files into per-volume archives |
| `max_merge_bytes` | int | `2147483648` | Skip a merge whose pages exceed this many bytes |

Actions: `pack`, `merge`. Warnings: `no-images`, `not-a-directory`,
`not-an-archive`, `no-archives`, `too-few-pages`, `mixed-folder`,
`ambiguous-series`, `ambiguous-number`, `conflicting-volume`,
`duplicate-chapter`, `target-exists`, `single-chapter`, `no-volume`,
`merge-too-large`, `target-is-source`, `unreadable`.

### `epub-check`

Validates the EPUB envelope and package document against EPUB 3.3: `mimetype`
entry, `container.xml` rootfile, `unique-identifier`/`dc:identifier`,
`dc:title`, `dc:language`, manifest `href` existence, a non-empty spine whose
`idref`s resolve, nav document (EPUB 3) or NCX (EPUB 2), `cover-image`
property. `fix` repairs only the mechanical violations — entry layout and the
package document; content documents, images, fonts and styles are copied
through as their original compressed bytes and are never rewritten.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `fix` | bool | `false` | Plan a repair for every `fixable` finding; without it the plan is findings-only and an apply writes nothing |

Actions: `repair-epub`. Warnings: `mimetype-{missing,not-first,compressed,content}`, `container-{missing,unparsable,rootfile-missing,rootfile-dangling}`, `opf-{unparsable,identifier-missing,unique-identifier-{missing,unresolved},title-missing,language-missing}`, `manifest-item-missing`, `spine-idref-unresolved`, `spine-empty`, `nav-document-missing`, `ncx-missing`, `cover-image-property-missing`.

### `epub2cbz`

Converts image-based EPUBs to CBZ in spine order, resolving `img@src` and
SVG `image@xlink:href` per document, de-duplicating repeated images, and
generating `ComicInfo.xml` from the package metadata. Text EPUBs are warned
about and skipped, so it is safe to point at a whole library.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `output_dir` | path | next to the source | Where CBZs are written |
| `recursive` | bool | `false` | Descend into subdirectories of a given folder |
| `overwrite` | bool | `false` | Replace an existing target CBZ |
| `comic_info` | bool | `true` | Write `ComicInfo.xml` from the EPUB metadata |
| `max_text_chars` | int | `200` | Visible text per spine document before it counts as prose |

Actions: `convert`. Warnings: `no-epubs`, `unreadable`, `no-images`,
`not-image-based`, `target-collision`, `target-exists`, `page-count-drift`.

### `missing-sequence`

Reports folders whose chapter/volume numbering skips a number, using the
shared `stump_scanner::sequence` grammar (identifiers, omnibus ranges,
decimals, `()`/`[]` stripping, changing-token fallback). Report-only: `apply`
writes nothing and re-emits the plan's findings as `applied`.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `detailed` | bool | `false` | Add `found_ranges` and `decimals` to each detail |
| `unidentified` | bool | `false` | List names with no chapter/volume schema, and report folders that only have such names |
| `all` | bool | `false` | Report gap-free folders too (default: only folders with gaps) |

Actions: `report-gaps`, `report-complete`, `report-unsequenced`,
`report-unidentified`. Warnings: `no-books`, `missing-path`, `span-exceeded`.

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | `Tool`, `ToolInput`, `ProgressSink`/`NoopProgress`, `registry()`, `find()` |
| `src/plan.rs` | `Plan`, `Action`, `Warning`, `Severity`, `Report` |
| `src/error.rs` | `ToolError`, `ToolResult` |
| `src/util.rs` | Page detection/naming, stored-CBZ writing, atomic write, `ComicInfo` generation, sorted walks |
| `src/<id>.rs` | One tool per file; module name is the id with underscores |
| `src/calibre.rs` | The one module holding two tools (`calibre-convert`, `calibre-meta`), because both share `Calibre::locate` and the timed subprocess runner |

## How to verify

```sh
cargo test -p stump_tools                     # unit + fixture tests (no binary fixtures)
cargo run -p cli --bin cli-bin -- tools list  # every registered tool
cargo run -p cli --bin cli-bin -- tools plan cbzit /books/Series --json
```

`plan` writes nothing: the fixture tests assert the target does not exist after
planning, and that a failed apply leaves the original bytes in place.

## Deep docs

- `crates/README.md` — crate index and README template
- `crates/media/README.md` — archive/image processing this crate builds on
- `crates/scanner/README.md` — the shared filename/sequence grammar
