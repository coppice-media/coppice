# annotation-sync

| | |
| --- | --- |
| **Package** | `stump_annotation_sync` (`crates/annotation-sync`) |
| **Purpose** | The canonical per-user annotation export model (`ExportBatch`/`ExportBook` built from `media_annotations`, `bookmarks`, the liseur-sync CAS, and reading heads/sessions), the `Sink` trait, and the `markdown` (Obsidian-friendly, byte-stable) and `git` (libgit2 commit/push, rebase-once) sinks. Host wiring — config group, debounce, `AnnotationSyncJob`, encryption of secret settings, GraphQL — lives in `stump_core::annotation_sync` / `stump_core::job::annotation_sync`; this crate only ever sees a `DatabaseConnection`. |
| **Reference / upstream** | liseur-sync CAS tables from `crates/migrations/src/m20260904_000000_add_liseur_sync.rs` (read-only here); Readium locator shape `models::shared::readium::ReadiumLocator`; Obsidian block ids / properties (<https://help.obsidian.md/>); `git2` 0.21 (libgit2, `default-features = false`). |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Native books are always rebuilt in full and fold every linked liseur work; only standalone liseur works are incremental by CAS `seq` | Native rows are hard-deleted, so a delta would miss deletions; a partial rebuild of a native book would flip its file between including and omitting linked liseur rows | `model.rs::build_export_batch`, `build_native_book`; test `builds_native_and_liseur_books_in_canonical_order` |
| One batch per run, built from the oldest sink cursor; every successful sink advances to the batch high-water mark | Sinks never diverge on what they saw; a failed sink keeps its old cursor and re-receives the delta next run | `stump_core::job::annotation_sync` (`init`, `execute_task`) |
| Canonical ordering fixed in code: books by key (`liseur:<work>` < `native:<media>`), annotations by `(created_at, id)` with native rows first, identifiers by `(scheme, value)` | Byte-identical re-exports for unchanged data | `model.rs` sorts; `markdown.rs` module doc |
| Tombstones stay in the model, are skipped by the renderer | Sinks that diff or mirror deletions can act on them; markdown readers never see them | `ExportAnnotation::deleted`; `render_book` filters |
| Markdown: YAML frontmatter with JSON-quoted scalars, `# <title>`, `## Highlights` / `## Bookmarks` lists, one `↗` meta line per row ending in an Obsidian block id (`^<row id>`) | Obsidian properties + block links work out of the box; ids survive round trips; `serde_json` escaping is valid YAML double-quote style | `markdown.rs`; golden test `renders_obsidian_layout_and_skips_tombstones` |
| Reader links only when the `base_url` setting is set (native books only) | The web reader has no locator deep link yet and a relative link would be an Obsidian vault path; liseur-only works have no Stump route | `markdown.rs::render_book`; `base_url_setting` shared with the git sink |
| Files are named `<key with ':'→'-'>.md` (`native-<media_id>.md`); unchanged files are not rewritten | Stable names for links; mtimes stay untouched on re-export | `markdown.rs::file_name`, `write_books` |
| Git sink: init `<root>/<user>` as a work tree, `add_all`, commit with the configured author, push `refs/heads/<branch>`; on `NotFastForward` fetch + rebase once, then push again; a rebase conflict aborts and surfaces `GitConflict` with the local branch intact | Two writers (another device, a human editing the vault) must not lose data silently; the job records the error on the row for the user to resolve | `git.rs::push_with_retry`, `rebase_onto_remote`; test `publishes_then_rebases_once_and_surfaces_conflicts` |
| Push runs whenever the branch has commits, even with an unchanged batch | A previous run may have committed and then failed to push | `git.rs::publish` |
| `git2` with `default-features = false` (no vendored openssl/libssh2); token offered as `x-access-token` user/pass | Keeps the headless build small; HTTPS token auth is what forges accept | `Cargo.toml`; `git.rs::auth_callbacks` |
| Secret settings are encrypted by the host before storage; sinks receive plaintext | Encryption key lives in `server_config`; one place to decrypt | `stump_core::annotation_sync::{encrypt,decrypt}_sink_values` |

## Layout

| File | Responsibility |
| --- | --- |
| `model.rs` | `ExportBatch`, `ExportBook`, `ExportAnnotation`, `ExportBookmark`, `ReadingSummary`, `build_export_batch` (native + liseur folding, ordering) |
| `sink.rs` | `Sink` trait, `SinkDescriptor`, `SinkState` (host cursor + opaque sink data), `string_setting` |
| `markdown.rs` | `RenderOptions`, `render_book`, `write_books`, `file_name`, `MarkdownSink`, `base_url_setting` |
| `git.rs` | `GitSink` (feature `git`): commit/push/rebase-once via `git2` on a `spawn_blocking` thread |
| `registry.rs` | `catalog()` of compiled-in sinks and `sink(id, root, values)` factory |
| `error.rs` | `AnnotationSyncError` (`Db`, `Io`, `Json`, `Sink`, `Git`, `GitConflict`) |

## Markdown format

```markdown
---
title: "Dune"
authors:
  - "Frank Herbert"
source: "native"
media_id: "m1"
identifiers:
  isbn: "9780441172696"
progression: 0.7512
page: 300
completed: false
last_read: "2023-11-14T22:14:30Z"
last_read_via: "liseur"
sessions: 2
reading_time_seconds: 5400
---

# Dune

## Highlights

- > The spice must flow.

  Opening line

  ↗ [ch1.xhtml](https://stump.example.com/books/m1/epub-reader) · 25.00% · 2023-11-14T22:13:30Z ^a1

## Bookmarks

- > A beginning is the time

  ↗ [ch3.xhtml · page 42](https://stump.example.com/books/m1/epub-reader) · 2023-11-14T22:14:20Z ^b1
```

Meta line parts, each present only when known: position (`href`, `cfi`, or
`position N` from the locator; bookmarks add `page N`), progression as a
percentage, liseur colour, created time (RFC 3339 UTC), then `^<row id>`.

## How to verify

```bash
cargo test -p stump_annotation_sync                    # model build, markdown golden + stability, git bare-repo publish/rebase/conflict
cargo check -p stump_annotation_sync --no-default-features   # markdown only, no libgit2
cargo test -p stump_core --lib annotation              # debounce, secret round-trip
```

## Deep docs

`docs/content/docs/developer/annotation-sync.mdx` — config keys, GraphQL
operations, sink settings, and the job lifecycle.
