# annotation-sync

|                          |                                                                                                     |
| ------------------------ | --------------------------------------------------------------------------------------------------- |
| **Package**              | `stump_annotation_sync` (`crates/annotation-sync`)                                                  |
| **Purpose**              | Canonical per-user annotation export model plus safe Markdown and Git sinks.                        |
| **Reference / upstream** | liseur-sync CAS tables; `models::shared::readium::ReadiumLocator`; Obsidian block ids; `git2` 0.21. |

## Decisions

| Decision                                                                                                                                                        | Why                                                                                                                                              | Evidence                                                                              |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------- |
| Versionless sink rows stay on the legacy key/UUID filename and renderer; format 2 is explicit                                                                   | Existing vault links and paths must remain byte/path compatible until a user upgrades                                                            | `RenderOptions::from_settings`; `file_name`; legacy renderer test                     |
| Format 2 defaults to `{{author}} - {{title}}/annotations.md` below the per-user root                                                                            | Human navigation without allowing a user path to escape the admin-mounted root                                                                   | `DEFAULT_PATH_TEMPLATE`; `path_for_book`                                              |
| Built-in presets are Obsidian folders, flat notes, and author folders; custom templates allow only fixed book variables                                         | UI can offer safe choices without exposing filesystem or expression evaluation                                                                   | `preset_descriptors`; `validate_path_template`; `validate_body_template`              |
| NFC-normalize and sanitize title/author components; reject traversal, roots, reserved names, control bytes, and oversized segments                              | Unicode-equivalent names should not split books, and path input must not cross the mounted root                                                  | `normalize_component`; `validate_relative_destination`                                |
| Format-2 source-key → relative-path map lives in `SinkState.data` and collision suffixes are deterministic                                                      | Duplicate human names must never overwrite one another; title changes retain ownership                                                           | `plan_paths`; `prior_path_map`                                                        |
| Frontmatter is structured, ordered YAML with schema/book/source/source ids, source_updated_at, reading state, row provenance, and timestamps; no generated time | The export is inspectable and unchanged data remains byte-identical                                                                              | `push_frontmatter_v2`; `source_updated_at`                                            |
| V2 body block ids include the source book key; legacy ids remain unchanged                                                                                      | Native and liseur rows can share an id without broken Obsidian links                                                                             | `block_id_v2`; legacy renderer test                                                   |
| Reviews are optional, user-scoped, and rendered once with explicit rating/content/privacy/timestamps                                                            | A work review must not be copied into both editions or leak another user's private note; deterministic exports need review changes to be visible | `ExportReview`; `review_for_native`; `push_review_frontmatter_v2`; `push_review_body` |
| Every user export takes a lock; files use same-directory 0600 temp + sync + atomic rename, and every existing path component/final target rejects symlinks      | Concurrent Markdown/Git exports cannot interleave or publish partial/symlinked output                                                            | `acquire_user_lock`; `write_atomic`; `ensure_no_symlink`                              |
| Git stages only returned managed relative paths and rejects overlapping local remotes                                                                           | Unmanaged vault edits must not enter an export commit, and a worktree must not be its own remote                                                 | `write_books_locked_for_git`; `commit_all`; `validate_remote_boundary`                |
| Secret sink settings are encrypted by the host; omitted object keys merge with the stored row                                                                   | Updating layout or metadata must not erase an encrypted token                                                                                    | `stump_core::annotation_sync::merge_sink_settings`                                    |

## Layout

| File          | Responsibility                                                                                           |
| ------------- | -------------------------------------------------------------------------------------------------------- |
| `model.rs`    | `ExportBatch`/`ExportBook`/`ExportReview` model, deterministic native/liseur folding and reading summary |
| `sink.rs`     | `Sink`, `SinkDescriptor`, `SinkPresetDescriptor`, and durable `SinkState`                                |
| `markdown.rs` | Legacy/v2 rendering, safe templates/paths, review rendering, collision map, atomic writer and lock       |
| `git.rs`      | Git worktree commit/push/rebase; managed-path staging (feature `git`)                                    |
| `registry.rs` | Compiled sink catalog and factory                                                                        |
| `error.rs`    | Export model and sink errors                                                                             |

## Format 2

The frontmatter is deterministic and ordered:

```yaml
schema: 'coppice.annotation/v2'
book:
  key: 'native:m1'
  title: 'Dune'
  authors:
    - 'Frank Herbert'
  identifiers: {}
source:
  kind: 'native'
  id: 'm1'
  media_id: 'm1'
  key: 'native:m1'
source_updated_at: '2023-11-14T22:14:30Z'
reading:
  progression: 0.7512
  page: 300
  completed: false
  finished: false
  last_read_at: '2023-11-14T22:14:30Z'
  source_protocol: 'liseur'
  session_count: 2
  total_seconds: 5400
  last_session_at: null
annotations:
  - id: 'a1'
    kind: 'highlight'
    source: 'native'
    source_id: 'm1'
    provenance:
      kind: 'native'
      source_id: 'm1'
    created_at: '2023-11-14T22:13:30Z'
    updated_at: '2023-11-14T22:13:30Z'
    deleted: false
bookmarks: []
```

Tombstones remain in structured frontmatter for provenance but never render in
body sections. Source-aware body blocks use `^native-m1-a1` (or the liseur
source key), while legacy files retain `^a1`.

## How to verify

```bash
cargo test -p stump_annotation_sync
cargo check -p stump_annotation_sync --no-default-features
cargo test -p stump_core --lib annotation
```

## Deep docs

`docs/content/docs/developer/annotation-sync.mdx` documents config keys,
GraphQL operations, safe templates, and the job lifecycle.
