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
| EPUB 3.3 | <https://www.w3.org/TR/epub-33/> | Spine order and `href` resolution (`epub2cbz`); envelope/package conformance rules (`epub-check`); the EPUB 3 package (`#attrdef-package-version`, `#last-modified-date`, `#sec-cover-image`), navigation document (`#sec-nav`, `#sec-nav-{toc,pagelist,landmarks,prop}`), XHTML5 content documents (`#sec-xhtml-req`) and the legacy `guide` (`#sec-opf2-guide`) that `epub-polish` upgrades to |
| calibre manual (9.14.0) | <https://manual.calibre-ebook.com/generated/en/cli-index.html> | `ebook-convert`/`ebook-meta` invocation and flags, and the `%prog (calibre <version>)` banner parsed by `calibre::locate`. calibre is GPL-3: it is executed as a separate process only, never linked, vendored, or bundled. See `docs/content/docs/developer/calibre-tooling.mdx` |
| `ebook-polish` manual (9.14.0) | <https://manual.calibre-ebook.com/generated/en/ebook-polish.html> | The seven polish flags `calibre-polish` maps onto, its `input_file [output_file]` calling convention, and the AZW3/EPUB/KEPUB-only input rule; each option's doc comment cites the flag's own anchor |
| boko 0.5.0 | <https://github.com/zacharydenton/boko> (README format matrix), plus `boko --help` / `boko convert --help` of a real install | `boko convert --quiet <in> <out>` invocation, the `epub`/`azw3`/`kfx`/`mobi` format list, the `boko <version>` banner parsed by `boko::locate`, and the `MOBI output is not supported; use .azw3 instead` refusal. boko is GPL-3.0-or-later: it is executed as a separate process only, never linked, vendored, bundled, or added to a `Cargo.toml`. See `docs/content/docs/developer/calibre-tooling.mdx` |
| OPF/OPS 2.0.1 | <https://idpf.org/epub/20/spec/OPF_2.0.1_draft.htm#Section2.4.1>, <https://idpf.org/epub/20/spec/OPF_2.0.1_draft.htm#Section2.6>, <https://idpf.org/epub/20/spec/OPS_2.0.1_draft.htm#Section2.2> | The NCX `epub-polish` generates a nav document from, the `guide` it maps onto `landmarks`, and the XHTML 1.1 content-document doctype it rewrites |
| EPUB 3 structural semantics vocabulary | <https://www.w3.org/TR/epub-ssv-11/> | The `epub:type` terms `guide/@type` values are mapped to in a generated `landmarks` nav |
| SmartyPants | <https://daringfireball.net/projects/smartypants/> | The published punctuation rules `smarten_punctuation` implements, including the "old school" dashes (`--` en dash, `---` em dash) and the opening/closing quote heuristic |
| CLDR delimiters (LDML) | <https://www.unicode.org/reports/tr35/tr35-general.html#Delimiter_Elements>, `common/main/{en,de,fr}.xml` | The `quotationStart`/`quotationEnd`/`alternateQuotation*` glyphs per language: `en` `“”`/`‘’`, `de` `„“`/`‚‘`, `fr` `«»` |
| MOBI / KF8 | MobileRead <https://wiki.mobileread.com/wiki/MOBI>, <https://wiki.mobileread.com/wiki/KF8> | Read only as the *format* behind `mobi2epub`; the parsing itself lives in `stump_media::MobiBook` and the citations are on its structures. Neither calibre's, KindleUnpack's nor boko's source is used |
| ID3v2 chapter frames 1.0 | <https://id3.org/id3v2-chapters-1.0> | The `CHAP`/`CTOC` frames `audio-chapters` writes: `TIT2` inside a `CHAP` as the chapter name, the all-`0xFF` start/end byte offsets that mean "address me by time", and the one top-level ordered `CTOC` that lists the chapters |
| QuickTime chapter lists | <https://developer.apple.com/documentation/quicktime-file-format/chapter_lists> | Why `audio-chapters` writes a Nero `chpl` list and never a chapter track — a chapter track is a media *track*, and a player uses the first chapter list it finds, which is also the precedence `stump_media::audio` probes with |

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
| The calibre adapters carry their own `bin_dir` option instead of reading a `STUMP_CALIBRE_BIN_DIR` config key | `stump_tools` must stay free of `stump_core`, so there is no config to read; resolution is `bin_dir`, else `PATH`, else the macOS bundle dir, and a version below 7.0 is `ExternalToolMissing` with an install hint | `src/calibre.rs` `CALIBRE_TOOL`/`Calibre::locate`/`MIN_VERSION`, `src/external.rs` `ExternalTool::resolve_dir`, `locate_accepts_a_supported_version_and_rejects_an_old_one`, `a_missing_binary_is_reported_with_an_install_hint` |
| Child stdout/stderr go to temp files, not pipes, stdin is `/dev/null`, and every run has a finite timeout | A parent that polls `try_wait` and only reads pipes after exit deadlocks the moment a chatty conversion fills the pipe buffer; a child that asks a question must be killed by the timeout instead of blocking on the server's stdin, and a hung `ebook-convert` must not hold a library run open forever | `src/external.rs` `ExternalTool::run`, `apply_times_out_and_reports_it` |
| A failed, timed-out, or output-less conversion deletes the target file and becomes a `skipped` entry carrying the exit status and stderr tail | A truncated book in a library folder is worse than no book, and the planner already refuses pre-existing targets, so anything present after a failure is ours to remove | `src/calibre.rs` `CalibreConvert::apply`, `apply_captures_stderr_and_removes_a_broken_output` |
| `calibre-meta` is read-only and emits `ExternalMediaMetadata`-shaped JSON with only populated fields | Writing metadata mutates the original and needs the same explicit-apply story as the mutating tools; omitting empty fields means a later merge can never blank a value it has no opinion on | `src/calibre.rs` `CalibreMetadata`/`parse_opf`, `opf_is_mapped_onto_the_external_metadata_shape`; `crates/integrations/metadata/src/types/metadata.rs` |
| `calibre-polish` always passes an explicit output file, stages an `in_place` run into a temp dir beside the source, and refuses a run with no flag selected | The manual makes `output_file` optional, i.e. calibre would rewrite the input it is reading; a staged output plus a `.bak` and a rename is the only way an in-place polish can fail without damaging the book, and a flagless polish is a rewrite with no effect | `src/calibre_polish.rs` `stage`/`clean_up`, `in_place_backs_up_the_original_and_replaces_it_with_the_polish`, `a_failed_in_place_polish_leaves_the_original_byte_identical`, `a_polish_with_no_action_selected_is_refused` |
| The seven polish flags are emitted in one fixed order out of a `FLAGS` table, and `Calibre::locate`'s `ebook-convert` probe is followed by an `ebook-polish` executability check | A deterministic command line is testable without a real calibre, and an installation that has `ebook-convert` but not `ebook-polish` must fail once at plan time instead of skipping every book at apply time | `src/calibre_polish.rs` `FLAGS`/`PolishOptions::flags`, `selected_flags_are_emitted_in_the_documented_order`, `a_calibre_without_ebook_polish_is_reported_as_missing` |
| `epub-check --fix` relays out the whole archive — `mimetype` written first, stored, exact — and raw-copies every other entry, re-serialising only the package document | The OCF rules are about entry *order* and *compression*, which an in-place append cannot change; raw copy means a repair provably cannot alter a content document, image, font or style sheet | `src/epub_check.rs` `repair_epub`, `repaired_epub_is_clean_and_readable`, `compressed_and_padded_mimetype_is_normalized` (asserts `data_start() == 30 + 8`, i.e. no extra field) |
| Findings are convergent: nav/NCX/cover are judged over the items that survive a repair, and a dropped manifest item takes its spine `itemref` and `spine/@toc` with it | Re-checking a repaired file must report nothing new, or `fix` never terminates and the tool blames itself for its own edits | `src/epub_check.rs` `check_manifest`/`check_navigation`, `dropping_a_manifest_item_drops_its_spine_itemref`, `dropping_the_ncx_clears_the_spine_toc_pointer` |
| Only mechanical defects are `fixable` (entry layout, `dc:language` `und`, a `urn:uuid` v4 identifier under a collision-free id, dropping items with no file, adding `cover-image`); a missing title, nav document, NCX or non-empty spine is reported and never invented | Guessing a title or synthesising navigation means writing a content document, which this tool refuses to do; the OPF is rewritten event-by-event so declaration, comments, attribute order and indentation survive, and the inserted `dc:` prefix is copied from the document rather than assumed | `src/epub_check.rs` `rewrite_opf`/`DcNaming::detect`, `epub2_without_title_or_ncx_reports_unfixable_findings`, `a_spine_with_no_readable_document_is_reported`, `missing_identifier_and_cover_property_are_repaired` |
| `meta-edit` rewrites `ComicInfo.xml` and the OPF with one quick-xml event pass and puts the bytes back with `util::replace_archive_entry`; missing elements are inserted before the container's closing tag with the observed indentation, and a repeated owned element is dropped after its first occurrence | A hand-made metadata file carries a declaration, comments, unknown elements and an attribute order the tool has no opinion about, and raw-copying every other member means a metadata edit provably cannot touch a page, a content document or a font; a field stored twice is a file two readers disagree about | `src/meta_edit.rs` `rewrite`/`write_values`, `an_edit_keeps_every_unknown_comic_info_byte_and_every_page`, `an_epub_edit_reads_back_through_the_epub_processor` |
| A field's `from` value is read from the element that stores it, never from a normalised model; `set` only ever sets (an empty value is `ToolError::Options`) and `language` also patches an existing non-schema `<Language>` without ever introducing one | A diff has to be about the bytes in the file, so `no-changes` means nothing would move; clearing metadata in bulk is a data-loss button nobody asked for, and writing `LanguageISO` while leaving a stale `<Language>` would make one file name two languages | `src/meta_edit.rs` `Values`/`comic_info_edits`, `plan_writes_nothing_and_reports_only_the_changed_fields`, `unusable_options_are_rejected_before_a_file_is_touched` |
| A rename renders the *post-edit* values, drops any whitespace chunk holding a value-less placeholder, and claims its target inside the plan | `v{volume:02}` must not leave an orphan `v` when a book has no volume, and two books in one run that render to one name must not overwrite each other before any `exists` check could see it | `src/meta_edit.rs` `Pattern::render`/`plan_file`, `a_rename_from_metadata_round_trips_through_the_scanner_parser`, `a_placeholder_with_no_value_drops_its_whole_chunk`, `two_books_rendering_to_one_name_never_overwrite_each_other` |
| Page pixels are re-encoded by `stump_media::transform::transform_page_bytes`, never by an image dependency of this crate | One decoder, one Lanczos/CatmullRom resize kernel, one never-upscale rule and one WebP encoder across delivery and maintenance — the code path the device presets are already validated against | `src/webp_convert.rs` `ConvertDetail::profile`, `pages_become_webp_and_the_archive_shrinks`, `max_dimensions_downscale_and_never_upscale` |
| A converted page keeps its entry name and only swaps the extension; an already-WebP page is raw-copied, and an extension collision (`001.jpg` + `001.png`) skips the archive | Renumbering entries would move every reader's page positions, re-encoding WebP again is generational loss for no gain, and renaming one side of a collision would silently reorder the book | `src/webp_convert.rs` `webp_name`/`plan_archive`, `an_already_webp_page_is_copied_not_re_encoded`, `an_extension_collision_leaves_the_archive_alone` |
| A derived sibling output is named through `util::derived_target` with a bracketed or parenthesised suffix (` [webp]`, ` (polished)`), and `in_place` instead keeps a `<file name>.bak` it refuses to clobber | The output must never be its own source, and `stump_scanner::clean_name` strips `[...]`/`(...)` spans, so the derived file still scans as the same series, volume and chapter | `src/util.rs` `derived_target`; `src/webp_convert.rs` `converted_name_still_parses_as_the_same_volume`, `in_place_keeps_a_backup_of_the_original` |
| Every test that writes a fake external binary and executes it takes ONE crate-wide lock, `test_support::fake_binary_lock`, held for the fixture's whole lifetime | `execve` returns `ETXTBSY` while any process still holds the image open for writing, and a per-module mutex only serializes a module against itself: once a second tool shelled out, the suite started failing in whichever module lost the race | `src/test_support.rs`; `src/calibre/tests.rs` `FakeCalibre::new`, `src/calibre_polish.rs` `Fake::with_binaries` |
| boko is the preferred converter for the Kindle formats; `calibre-convert` is the fallback | boko writes KFX without Amazon's Kindle Previewer (calibre's KFX Output plugin only drives Previewer, which needs Wine on Linux and does not run in Flatpak/Snap), needs no plugin, and is native — the fixture EPUB became AZW3 in ~60 ms. calibre keeps what boko cannot do at all: MOBI output, PDF, DOCX, FB2, LIT, PDB, CBZ | `src/boko.rs` module docs, `SourceFormat::ALL`; `src/calibre.rs` `TargetFormat` |
| `to: "mobi"` deserializes and is then refused by `plan`, with a `calibre-convert` pointer, before boko is even located | boko reads MOBI but `boko convert in.epub out.mobi` exits 1 with "MOBI output is not supported; use .azw3 instead": refusing the *request* once beats one failed child per book, and rejecting the word in serde would answer with a variant list instead of the fix | `src/boko.rs` `TargetFormat::writable`/`BokoConvert::plan`, `mobi_output_is_refused_with_a_pointer_to_calibre` |
| `boko convert` is always passed `--quiet`, so a non-empty stderr is a real diagnostic and becomes a `boko-stderr` warning | boko prints its progress (`Converting <in> -> <out>`, `Done.`) on **stderr**, so without `--quiet` every successful conversion would come back carrying captured "error output"; `--quiet` suppresses only those lines, an `error: ...` still arrives | `src/boko.rs` `QUIET`, `apply_invokes_boko_convert_with_source_target_and_extra_args` |
| `boko::locate` searches `bin_dir`, then `PATH`, then `$CARGO_HOME/bin` and `~/.cargo/bin` | `cargo install boko` is boko's only distribution channel, and a server started by a service manager rarely carries that directory in `PATH` — the same reasoning that puts the macOS bundle dir in calibre's search order | `src/boko.rs` `cargo_bin_dirs`/`BOKO_TOOL`, `src/external.rs` `ExternalTool::resolve_dir`, `a_missing_binary_is_reported_with_an_install_hint` |
| Every external binary is described once as an `external::ExternalTool` (name, probe, `--version` arg, banner anchor, `MinVersion`, extra dirs, install hint) and located, version-checked, run and reported through that one implementation | Locating, banner parsing, the timed child runner and the `ExternalToolMissing` shape are the hard parts; per-tool copies of them would be several implementations of the hard part in order to change a name in one error string. Only what genuinely differs stays per adapter: calibre's floor is a `major.minor` (its CLI is stable across patches), boko's is the whole triple (a pre-1.0 crate can move a subcommand in a minor release) | `src/external.rs` `ExternalTool`/`MinVersion`/`Output`; `src/calibre.rs` `CALIBRE_TOOL`, `src/boko.rs` `BOKO_TOOL`, `src/calibre_polish.rs` (locates and runs through `CALIBRE_TOOL`); `locate_accepts_a_supported_version_and_rejects_an_old_one` and `a_pre_release_patch_is_below_the_minimum` in both adapters' tests |
| `epub-polish` refuses an upgrade outright — no NCX to build navigation from, or a package missing the identifier, title or language EPUB 3 requires — instead of upgrading half-way | An EPUB 3 with no `nav` document is invalid, and synthesising navigation or minting a title is `epub-check --fix`'s job, not a polisher's; a refused upgrade leaves a book that still opens | `src/epub_polish.rs` `navigation`/`plan_upgrade`, `an_epub2_without_an_ncx_is_never_upgraded` |
| The upgrade also adds `cover-image` to the cover item (same `<meta name="cover">`-then-`cover`-id heuristic as `epub-check`) and rewrites an XHTML 1.1 prolog, while keeping the NCX, `guide` and `spine/@toc` | Those two are EPUB 3 requirements the version bump *creates*: an upgrade that leaves new `epub-check` findings behind blames the next tool for its own edits. The EPUB 2 navigation is kept because EPUB 2 readers still need it, and `prefix`/`unique-identifier` survive because the OPF is rewritten event-by-event | `src/epub_polish.rs` `cover_image_id`/`xhtml5_prolog`/`rewrite_opf`, `epub2_becomes_an_epub3_that_epub_check_finds_nothing_wrong_with`, `an_upgraded_book_is_reported_as_epub3_and_not_upgraded_again` |
| Punctuation is substituted in the *raw escaped bytes* of text nodes only, never in attributes, `pre`, `code`, `script`, `style`, or an entity reference | Every character this tool inserts is non-ASCII and every character it consumes (`"`, `'`, `-`, `.`) is impossible inside an entity reference, so transforming the escaped text preserves `&amp;`/`&#8212;` byte-for-byte instead of re-escaping a whole document; authored text is not prose | `src/epub_polish.rs` `smarten_document`/`smarten_text`, `punctuation_is_educated_in_prose_and_nowhere_else`, `quote_style_follows_dc_language` |
| The generated navigation document is carried in the plan's `detail`, while the content-document transforms are carried as flags | The nav is *invented*, so the dry run has to show the exact document that will be written; the text transforms are pure functions of the archived bytes, so replaying flags cannot drift, and a plan does not have to carry a whole book's prose | `src/epub_polish.rs` `NavDocument`/`content_transforms`, `the_generated_nav_lists_every_ncx_entry`, `plan_writes_nothing` |
| `mobi2epub` converts in process through `stump_media::MobiBook` rather than shelling out to `boko` or `ebook-convert` | The Kindle reader already exists in the tree for scanning, so a conversion needs no external binary, no temp copy of the whole book and no GPL process boundary; the external converters stay for the formats Stump cannot read (KFX) and for output formats it does not write | `src/mobi2epub.rs` `convert`, `the_produced_epub_passes_epub_check_with_no_findings`, `the_produced_epub_opens_with_the_epub_processor` |
| `mobi2epub` writes the envelope with the same rules `epub-check` validates, and its own test asserts zero findings | Two modules that disagree about the OCF layout would mean `epub-check --fix` rewriting every file the converter just wrote | `src/mobi2epub.rs` `convert`/`Package`, `the_epub_envelope_follows_the_ocf_rules` |
| `sources-import` reads the extension repository as text with a hand-rolled recogniser, and rejects a whole source the moment one member is not a literal instead of emitting a partial definition | A half-derived definition is a source that silently misbehaves at runtime; a rejected one is a row in `unsupported.json` naming the member, which is a work item. There is no Kotlin runtime here, so a "mostly parsed" class has nothing to fall back to | `src/sources_import.rs` `parse_class_body`/`Reject`, `behaviour_overrides_are_unsupported_with_the_member_named`, `a_source_without_a_theme_is_never_masked_by_its_overrides` |
| Only data is derived: no upstream Kotlin is copied into this crate, every definition records `upstream.{repo,commit,path}`, and the output directory gets a `NOTICE` | keiyoushi/extensions-source is Apache-2.0, so a derived data set has to carry attribution and has to be re-derivable from the exact commit it came from — which is also what makes two runs diffable | `src/sources_import.rs` `NOTICE_TEMPLATE`/`Upstream`, `provenance_is_read_from_the_checkout_without_running_git`, `apply_writes_the_definitions_index_unsupported_and_notice` |
| A definition's `version` is the extension's own `versionCode`; the theme's `baseVersionCode` is deliberately not folded in | Upstream adds the two to version an *APK* built from a Kotlin theme, but here the theme is Rust and ships with Stump — folding it in would bump every definition whenever the upstream counterpart of our own engine changed | `src/sources_import.rs` `Definition::version`, `docs/content/docs/developer/source-definitions.mdx` schema table |
| Provenance is read out of `.git/HEAD`, `refs/` and `packed-refs` directly instead of shelling out to `git` | This crate has exactly one process-spawning path (`src/external.rs`, for operator-installed converters) and reading a commit id is not a conversion; it also keeps the tool working on an exported tree with no `git` on `PATH` | `src/sources_import.rs` `resolve_commit`/`resolve_repo`, `provenance_is_read_from_the_checkout_without_running_git` |
| `audio-report` and `audio-chapters` resolve their targets through one function, `audio_report::publications` | "Audio publication" has to mean the same thing in the report and in the writer, or the mutating tool could name a file the report never listed; the walk is one level deep because `PathUtils::dir_is_audio_book` already refuses a directory holding subdirectories, so a deeper walk could only find folders that are not books | `src/audio_report.rs` `publications`, `a_library_folder_is_walked_one_level_into_its_publications`; `src/audio_chapters.rs` `apply_never_touches_a_file_the_plan_did_not_name` |
| `audio-report` keeps the action row of a broken publication and attaches an `Error` warning instead of dropping the action | A report tool's action *is* its finding, so honouring `Severity::Error`'s "the related action was dropped" would hide exactly the books a librarian has to fix | `src/audio_report.rs` `warn_about`; `src/plan.rs` `Severity` |
| `audio-chapters` writes only the MP4 `chpl` list, never the chapter track, and refuses Ogg/Opus/FLAC outright | A chapter track is a media track: rewriting or removing one is structural surgery, while the marks written are the ones just read from it, so the two mechanisms cannot disagree. Vorbis-comment marks live in the container's comment header, and rewriting that means rewriting every following page's granule bookkeeping — a metadata tool that gets it wrong produces a file no player opens | `src/audio_chapters.rs` `write_chpl`/`container_of`, `a_chapter_track_becomes_a_portable_chpl_list`, `an_unsupported_container_is_refused_with_an_error` |
| A folder book's *per-file* provenance is derived from the publication's only for `per_track`, and re-probed per file otherwise | `probe_folder` records `per_track` only when no part carried a mark at all, so that one case is exact and free; any other aggregate names whichever part carried marks first and would license overwriting a part that has its own | `src/audio_chapters.rs` `existing_source` |
| An in-place tag edit stages a *copy* of the original into `write_atomic`'s temp file and restores the file's mode afterwards | Both taggers rewrite a container in place — they need the media bytes they are not changing — so there is no "write a fresh file" path to use; and `write_atomic` stages through a 0600 temp file, which a library file must not inherit | `src/audio_chapters.rs` `edit_atomic`; `src/util.rs` `replace_archive_entry` |

## Tools

Each tool owns one `### <id>` subsection: what it does and refuses to do, its
options (`Option | Type | Default | Effect`), then its action kinds and warning
codes. Subsections are sorted by tool id as ASCII (`-` sorts before digits and
letters, so `cbz-covers`, `cbzit`, `epub-check`, `epub-polish`, `epub2cbz`,
`meta-edit`, `missing-sequence`, `sources-import`, `webp-convert`). This
section is the one part of the crate README that is exempt from the 120-line
template budget: the tool contract requires one table row per option, so the
file grows with the tool count. Keep each subsection to
8-14 lines.

### `audio-chapters`

Embeds an audiobook's chapter marks into its own files, so a chapter list Stump
only derived becomes a property of the book: a Nero `chpl` list for
`.m4b`/`.m4a`, ID3v2 `CHAP` frames plus one top-level `CTOC` for `.mp3`. Every
other container is refused, including Ogg/Opus/FLAC. On MP4 only the `chpl`
list is written, never the chapter track.

Marks a publisher authored (`mp4_chpl`, `mp4_chapter_track`, `id3_chap`,
`vorbis_comment`) are reported as `keep-chapters` and left alone unless `force`
is set. The unit is the *file*: a folder book is many containers, each carrying
only the marks that fall inside it, rebased to file-relative milliseconds.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `force` | bool | `false` | Rewrite marks a publisher already authored |
| `titles_from_filename` | bool | `false` | Name a per-file mark after the file stem even when the file carries a title tag; the default prefers the tag and falls back to the stem |

Actions: `write-chapters`, `keep-chapters`. Warnings: `no-chapters`,
`unsupported-container`, `probe-failed`, `not-audio`, `no-audio`,
`missing-path`.

### `audio-report`

Reports one row per audio publication: duration (`h:mm:ss` and milliseconds),
codec, sample rate, channels, bitrate, chapter provenance, chapter and track
counts, cover presence, title, author and narrator. A single container, an
audiobook folder, and a library folder holding both are all accepted.

The warnings are the conditions that change what a client can do: no marks at
all (`no-chapters`), marks Stump synthesized per file rather than a publisher
authored (`synthesized-chapters`), parts that do not share one codec
(`mixed-codec`), an undeterminable duration (`zero-duration`), and no embedded
artwork (`no-cover`). Report-only: `apply` writes nothing and re-emits the
findings, exactly like `missing-sequence`.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `detailed` | bool | `false` | Add the chapter marks (`0:00:00 Opening`) and the file list, in playback order, to every detail |

Actions: `report-audio`. Warnings: `no-chapters`, `synthesized-chapters`,
`mixed-codec`, `zero-duration`, `no-cover`, `probe-failed`, `not-audio`,
`no-audio`, `missing-path`.

### `boko-convert`

Converts between EPUB, AZW3/KF8 and KFX with the operator's own `boko`, and is
the **preferred** converter for the Kindle formats — it writes KFX with no
Kindle Previewer, no plugin and no Wine, so it works on a headless server;
`calibre-convert` is the fallback for everything boko cannot read or write.
boko is GPL-3.0-or-later and is only ever executed as a subprocess: nothing is
linked, vendored, or installed by Stump, and the tool fails with an install
hint when boko is absent or older than 0.5.0. It removes no DRM.

Reads `.epub`, `.azw3`, `.mobi`, `.kfx` (Markdown and plain text are write-only
for boko); writes EPUB, AZW3, KFX. `to: "mobi"` is refused with a pointer to
`calibre-convert`, because boko reads MOBI but never writes it.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `to` | string | `epub` | Output format: `epub`, `azw3`, `kfx`; also the target extension boko infers its writer from. `mobi` parses and is then refused |
| `output_dir` | path | the source's directory | Where converted files are written |
| `extra_args` | string[] | `[]` | Verbatim extra `boko convert` arguments, e.g. `--optimize` |
| `bin_dir` | path | `PATH`, then `$CARGO_HOME/bin`, then `~/.cargo/bin` | Directory holding `boko` |
| `timeout_secs` | int | `300` | Per-file wall clock; the child is killed past it |
| `overwrite` | bool | `false` | Replace an existing target instead of skipping it |

The exact invocation is `boko convert --quiet <source> <target>
[extra_args...]`, verified against boko **0.5.0**; the version probe is
`boko --version`, whose banner is `boko 0.5.0`.

`to: "azw3"` from an EPUB source is the *send-to-Kindle* path (`KindleExport`):
AZW3 is what every Kindle since the Paperwhite 1 accepts as a sideload over USB
or by mail. Each such action also gets an informational `kindle-export`
warning. `to: "kfx"` reads better on a current Kindle (hyphenation, kerning,
ligatures) but is the newer container.

Actions: `convert`. Warnings: `boko-version`, `kindle-export`, `not-a-file`,
`unsupported-source`, `no-file-stem`, `already-target-format`,
`target-is-source`, `target-exists`, `boko-stderr`.

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

### `calibre-polish`

Runs the operator's own `ebook-polish` per book, always with an explicit output
file so calibre never rewrites the file it is reading. `ebook-polish` only
reads AZW3, EPUB and KEPUB, so anything else is reported and never planned, and
a run with no flag at all is refused rather than rewriting books for nothing.
Each applied action's `detail` carries `before_bytes`/`after_bytes`.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `subset_fonts` | bool | `false` | `--subset-fonts`: reduce embedded fonts to the glyphs used |
| `smarten_punctuation` | bool | `false` | `--smarten-punctuation`: typographic quotes, dashes, ellipses |
| `jacket` | bool | `false` | `--jacket`: insert a metadata jacket page, replacing any previous one |
| `remove_jacket` | bool | `false` | `--remove-jacket`: remove a previously inserted jacket |
| `upgrade_book` | bool | `false` | `--upgrade-book`: upgrade internal structures, e.g. EPUB 2 to 3 |
| `compress_images` | bool | `false` | `--compress-images`: lossless image recompression |
| `add_soft_hyphens` | bool | `false` | `--add-soft-hyphens`: soft hyphens for readers without hyphenation |
| `output_dir` | path | the source's directory | Where polished copies go; refused with `in_place` |
| `suffix` | string | `" (polished)"` | Stem suffix used when the output lands in the source's own directory |
| `in_place` | bool | `false` | Polish the original, keeping `<file name>.bak` |
| `recursive` | bool | `false` | Descend into subdirectories of a given folder |
| `bin_dir` | path | `PATH`, then the macOS bundle dir | Directory holding `ebook-polish` |
| `timeout_secs` | int | `600` | Per-file wall clock; the child is killed past it |
| `overwrite` | bool | `false` | Replace an existing target or backup instead of skipping |

Flags are emitted in the table's order, whatever order they were given in.

Actions: `polish`. Warnings: `calibre-version`, `not-a-file`,
`unsupported-format`, `target-is-source`, `target-exists`, `backup-exists`,
`no-books`, `calibre-stderr`.

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

### `epub-polish`

Polishes EPUBs in process, with no external binary — the subset of
`ebook-polish` that is mechanical enough to re-derive from the specifications.
`upgrade` takes an EPUB 2 package to EPUB 3.3: `version="3.0"`, a
`dcterms:modified` property, a `nav.xhtml` generated from the NCX (toc,
`page-list`, and `landmarks` mapped from the `guide`) added to the manifest
with `properties="nav"`, and `cover-image` on the cover item. The NCX,
`guide` and `spine/@toc` stay for EPUB 2 readers, and `prefix`,
`unique-identifier`, comments and indentation survive. Content documents are
only ever touched where asked: an XHTML 1.1 prolog becomes `<!DOCTYPE html>`
(prolog only), and text nodes get typographic punctuation. A run with no
option selected is refused.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `upgrade` | bool | `false` | EPUB 2 → EPUB 3.3: package version, `dcterms:modified`, generated `nav.xhtml`, `cover-image`, XHTML5 prologs |
| `smarten_punctuation` | bool | `false` | `"`/`'` → typographic quotes, `--`/`---` → en/em dash, `...` → `…`, in text nodes only (never attributes, `pre`, `code`, `script`, `style`); quote style from `dc:language` (`en` `“”`, `de` `„“`, `fr` `«»`, else `en`) |
| `remove_jacket` | bool | `false` | Drop a calibre jacket page (`calibre_jacket` class or `jacket` id) plus its manifest item, spine `itemref` and `guide` reference |

Actions: `polish-epub`. Warnings: `unreadable`, `incomplete-package`,
`already-epub3`, `ncx-{missing,unparsable,empty}`,
`jacket-{missing,in-toc}`, `language-{missing,unsupported}`,
`unparsable-document`, `nothing-to-do`.

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

### `meta-edit`

Bulk-sets metadata in CBZ `ComicInfo.xml` and EPUB package documents, then
renames files from the values it just wrote. Values are set, never cleared, and
travel as strings so `3.5` and `007` survive; `volume`/`year` must parse as
integers and `number` as a decimal. Unknown elements, comments, the
declaration, attribute order and indentation all survive, and every other
archive member is raw-copied. A plan lists the per-field `from`/`to` diff and
writes nothing.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `set.series` | string | none | `Series` / `calibre:series` |
| `set.number` | string | none | `Number` / `calibre:series_index` (decimal) |
| `set.volume` | string | none | `Volume` (integer); unsupported in an EPUB |
| `set.title` | string | none | `Title` / `dc:title` |
| `set.writer` | string | none | `Writer` / `dc:creator` |
| `set.publisher` | string | none | `Publisher` / `dc:publisher` |
| `set.year` | string | none | `Year` / `dc:date` (integer) |
| `set.language` | string | none | `LanguageISO` / `dc:language` |
| `set.tags` | string[] | `[]` | `Tags` (comma-separated) / one `dc:subject` each |
| `rename_from_metadata` | bool | `false` | Rename each file from `pattern` |
| `pattern` | string | `"{series} v{volume:02} c{chapter:03} - {title}"` | Rename pattern; see below |
| `recursive` | bool | `false` | Descend into subdirectories of a given folder |
| `overwrite` | bool | `false` | Let a rename replace an existing file |

Pattern placeholders are `{series}`, `{number}`/`{chapter}`, `{volume}`,
`{title}`, `{writer}`, `{publisher}`, `{year}`, `{language}`, each optionally
`:0N` zero-padded (integer part padded, decimal kept). The pattern is split on
whitespace: a chunk holding a value-less placeholder is dropped whole, so
`v{volume:02}` leaves no orphan `v`, and a chunk with no letters or digits
survives only between two that have some. The stem goes through
`sanitize_file_stem`, keeps the extension, and stays in its own directory.

Actions: `edit-metadata`, `rename`. Warnings: `no-books`, `not-a-file`,
`unsupported-format`, `unreadable`, `opf-missing`, `opf-unparsable`,
`field-unsupported`, `rename-target-exists`, `no-changes`.

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

### `mobi2epub`

Converts Kindle/Mobipocket books (`.mobi`, `.prc`, `.azw`, `.azw3`) to EPUB 3
in process, with no external converter: `stump_media`'s `MobiBook` reassembles
a KF8 part from its flow table plus its skeleton and fragment indices, splits a
MOBI 6 part on `<mbp:pagebreak>`, and rewrites every `kindle:pos:fid`,
`kindle:flow` and `kindle:embed` link (MOBI 6: `filepos` and `recindex`) onto
the names it writes. Metadata comes from EXTH and navigation from the NCX
index. It removes no DRM: a protected book is an `unreadable` warning carrying
the DRM detector's own verdict.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `output_dir` | path | next to the source | Where EPUBs are written |
| `recursive` | bool | `false` | Descend into subdirectories of a given folder |
| `overwrite` | bool | `false` | Replace an existing target EPUB |
| `suffix` | string | `""` | Stem suffix used when the output lands in the source's own directory; the extension already differs, so the default adds nothing |

The container is the one `epub-check` validates: `mimetype` first and stored,
one `container.xml` rootfile, a package document with a resolving
`unique-identifier`, `dc:title`, `dc:language`, a `nav` document and a
`cover-image` property, plus an EPUB 2 NCX under `spine@toc`.

Actions: `convert`. Warnings: `no-books`, `unreadable`, `no-sections`,
`target-is-source`, `target-collision`, `target-exists`,
`section-count-drift`.

### `sources-import`

Derives data-only source definitions from a checkout of
[keiyoushi/extensions-source](https://github.com/keiyoushi/extensions-source):
one JSON file per source carrying base URL, language, theme, version, NSFW flag
and the per-site knobs that theme needs, plus `index.json`, `unsupported.json`
and a `NOTICE`. Stump runs no Kotlin, so the repository is read as *text* by a
hand-rolled recogniser — the Gradle `keiyoushi {}`/`source {}` DSL, base-class
constructor arguments, and `override val`/`override fun` members whose body is
a single literal expression — and the theme engines in `crates/provider-themes`
execute the result. Behaviour is never guessed at: the whole source lands in
`unsupported.json` naming every member that blocked it. Schema v1, the knob
vocabulary and the reason codes are frozen in
`docs/content/docs/developer/source-definitions.mdx`.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `extensions_source` | path | first input path | Checkout root; must contain `src/` |
| `output_dir` | path | required | Existing directory the definitions are written into |
| `themes` | string[] | all | Only derive these themes; everything else is counted as filtered |
| `langs` | string[] | all | Only derive these source languages |

Actions: `write-definition`, `write-index`, `write-unsupported`,
`write-notice`, `report-stats` (the per-theme derived/unsupported table, which
`apply` reports without writing a file). Warnings: `no-themes`,
`unresolved-upstream-commit`, `duplicate-id`. Unsupported reason codes:
`no-source-class`, `multiple-source-classes`, `no-theme`,
`unknown-base-class`, `extra-supertype`, `no-base-url`, `unparsed-gradle`,
`ctor-args`, `unparsed-knob`, `custom-override`, `custom-client`,
`init-block`.

### `webp-convert`

Re-encodes the pages of a CBZ as WebP through Stump's own page pipeline
(`stump_media::transform`), so the encoder, the resize kernel and the
never-upscale rule are the ones the device presets use. Pages keep their entry
names (only the extension changes), already-WebP pages are copied verbatim, and
`ComicInfo.xml`, `.stump-covers.json` and every other member are raw-copied, so
metadata written by the other tools survives. Each applied action reports
`before_bytes`/`after_bytes`/`saved_bytes` for that file.

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `quality` | int | `80` | WebP quality, `1..=100` |
| `max_width` | int | none | Largest page width; pages are only downscaled |
| `max_height` | int | none | Largest page height; pages are only downscaled |
| `output_dir` | path | next to the source | Where converted archives are written |
| `suffix` | string | `" [webp]"` | Stem suffix used when the output lands in the source's own directory |
| `recursive` | bool | `false` | Descend into subdirectories of a given folder |
| `overwrite` | bool | `false` | Replace an existing target or backup instead of skipping |
| `in_place` | bool | `false` | Replace the source, keeping `<file name>.bak` |

Actions: `convert-pages`. Warnings: `no-archives`, `missing-path`,
`not-an-archive`, `unreadable-archive`, `no-pages`, `already-webp`,
`page-name-collision`, `target-is-source`, `target-exists`, `backup-exists`.

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | `Tool`, `ToolInput`, `ProgressSink`/`NoopProgress`, `registry()`, `find()` |
| `src/plan.rs` | `Plan`, `Action`, `Warning`, `Severity`, `Report` |
| `src/error.rs` | `ToolError`, `ToolResult` |
| `src/util.rs` | Page detection/naming, stored-CBZ writing, atomic write, derived output naming, single-entry archive rewrite, `ComicInfo` generation, sorted walks |
| `src/external.rs` | `ExternalTool`, `MinVersion`, `Install`, `Output`, `is_executable`: the crate's only locate/version-check/timed-child-run path for an operator-installed binary |
| `src/<id>.rs` | One tool per file; module name is the id with underscores |
| `src/calibre.rs` | The one module holding two tools (`calibre-convert`, `calibre-meta`), because both share `Calibre::locate` and the `CALIBRE_TOOL` description of the installation; `src/calibre_polish.rs` reuses both |
| `src/boko.rs` | `boko-convert` plus the `KindleExport` convenience; `BOKO_TOOL` is its whole `src/external.rs` description — one binary, clap's banner, the `cargo install` bin dir |
| `src/epub_polish.rs` | `epub-polish`; imports `epub_check`'s package/zip helpers (`parse_opf`, `parse_container`, `resolve_href`, `set_attribute`, `read_entry_text`, `collect_epubs`, …) instead of re-deriving them, so both tools read one OPF model |
| `src/mobi2epub.rs` | `mobi2epub`; the parsing is `stump_media::MobiBook`'s, this module only builds the OCF container (package document, nav, NCX) to `epub_check`'s rules |
| `src/sources_import.rs` | `sources-import`; the crate's one tool that reads a source *repository* instead of books — no `ToolInput` paths beyond the checkout root, and the only writer of the definition schema |
| `src/audio_report.rs` | `audio-report`, plus `publications()`: the one "what is an audio publication" walk both audio tools resolve their targets through |
| `src/audio_chapters.rs` | `audio-chapters`; the crate's only in-place *container* edit — `edit_atomic` stages a copy of the original in `util::write_atomic`'s temp file, because a tagger rewrites a container in place |
| `src/test_support.rs` | `cfg(test)` only: the crate-wide `fake_binary_lock` every subprocess test takes |

## How to verify

```sh
cargo test -p stump_tools                     # unit + fixture tests; every fixture is built in code except `mobi2epub`'s and the audio tools'
cargo test -p stump_tools audio               # both audio tools against the real ffmpeg-muxed fixtures
cargo test -p stump_tools boko                # boko adapter against a fake `boko`
cargo test -p stump_tools mobi2epub           # Kindle -> EPUB 3, checked with `epub-check` and re-opened with `EpubProcessor`
cargo test -p stump_tools sources_import      # the Gradle/Kotlin recogniser, on synthetic checkouts built in code
cargo run -p cli --bin cli-bin -- tools list  # every registered tool
cargo run -p cli --bin cli-bin -- tools plan cbzit /books/Series --json
```

`plan` writes nothing: the fixture tests assert the target does not exist after
planning, and that a failed apply leaves the original bytes in place.

Two tools are the exception to "no binary fixtures". `mobi2epub` reads
`crates/media/integration-tests/data/kindle-formats.azw3`, because a real KF8
skeleton/fragment table cannot be synthesised in a test and no converter that
writes one is installable in this tree. The `audio-*` tools read
`crates/media/integration-tests/data/audio/`, because the whole point of an
audio tool is that it reads and writes what real muxers write — including the
two MP4 chapter mechanisms, which no synthetic blob distinguishes. Those
fixtures are shared with `stump_media`'s own probe tests, so every mutating
test copies one into a temp dir first.

## Deep docs

- `crates/README.md` — crate index and README template
- `crates/media/README.md` — archive/image processing this crate builds on
- `crates/scanner/README.md` — the shared filename/sequence grammar
- `docs/content/docs/developer/source-definitions.mdx` — the frozen schema v1 `sources-import` writes
