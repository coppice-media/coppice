# Ebook pipeline

Use this reference for the EPUB side of a read-aloud fixture. Durable source
material is `docs/content/docs/developer/calibre-tooling.mdx`,
`crates/tools/src/epub_check.rs`, `epub_polish.rs`, `mobi2epub.rs`,
`boko.rs`, `calibre.rs`, and `calibre_polish.rs`. The lab must preserve the
source EPUB and make every repair or conversion a named derived artifact.

## EPUB order of operations

1. Detect DRM and stop; Stump never removes or bypasses protection.
2. Run `epub-check` against the source and save its JSON/action report.
3. Use `epub-polish` only for requested mechanical repairs; output separately.
4. Use `meta-edit` for package metadata when the fixture needs stable fields.
5. Re-run `epub-check` on the exact bytes passed to alignment.
6. Slice with `read-aloud-lab.py` only after the source is structurally usable.
7. Run `read-aloud-lab.py inspect` on every generated overlay EPUB.

`epub-check` follows the OCF rules that matter here: `mimetype` is first and
stored, `META-INF/container.xml` resolves one package document, manifest and
spine references resolve, and navigation/cover declarations remain coherent.
`epub-polish` is the in-process repair path for EPUB 2→3 structure,
navigation, punctuation, jackets, and related mechanical changes. Do not use a
repair merely to make a bad alignment look valid.

## Kindle and Kobo paths

| Input or output                                     | Preferred path                                                                                                   | Fallback or limit                                  |
| --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- | -------------------------------------------------- |
| MOBI/PRC/AZW/AZW3 read                              | Stump native MOBI reader.                                                                                        | DRM-protected input is refused.                    |
| MOBI/Kindle → EPUB 3                                | `mobi2epub` in `crates/tools/src/mobi2epub.rs`.                                                                  | No DRM removal; report unreadable input.           |
| EPUB → KEPUB for Kobo                               | In-process `stump_kepub` / `crates/kepub`; server adapter under `apps/server/src/routers/kobo_backend/kepub.rs`. | Keep KEPUB as a separate delivery derivative.      |
| EPUB/AZW3/KFX → Kindle formats                      | External `boko-convert`; adapter `crates/tools/src/boko.rs`.                                                     | Operator-installed, subprocess-only.               |
| MOBI output, PDF/DOCX/FB2/LIT/PDB, broad conversion | External calibre `ebook-convert`; adapter `crates/tools/src/calibre.rs`.                                         | Operator-installed, subprocess-only.               |
| Metadata read from broad formats                    | `calibre-meta` / `ebook-meta` subprocess.                                                                        | Stump metadata model remains authoritative.        |
| Minimal EPUB/AZW3/KEPUB repair                      | `calibre-polish` only when needed.                                                                               | Prefer native `epub-polish` for supported repairs. |

`stump_kepub` is a conversion library, not an alignment engine. KEPUB
pagination wrappers and `koboSpan` anchors are not sentence timing targets;
compare the canonical EPUB before device conversion.

## License boundary

Calibre and boko are external executables. NEVER copy, import, vendor, bundle,
or link their GPL code; exchange only documented argv/files and record the
external tool name/version in provenance. `ghost-story` and other reference
components follow the same boundary. The skill contains behavior and commands,
not source from those projects.

## OPF spine discipline

A narrative start is a structural decision, not a guessed chapter number. Use a
unique `bodymatter` landmark when present; otherwise pass an explicit idref or
href. The lab helper truncates only the derived OPF spine after a selected whole
chapter. It preserves the original archive and fails on ambiguous or partial
chapter/spine matching. `inspect` must then confirm that every SMIL text target
resolves to one XHTML element id and every audio target resolves to an archive
member.

Keep content identity evidence as hashes, spine ids, ordinals, and counts. Do
not paste prose, transcript text, cover art, or audio bytes into reports.
