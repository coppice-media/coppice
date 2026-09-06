# crates/macros

## Purpose

Grouping directory for the workspace's proc-macro crates. It is not itself a
crate: the workspace registers `crates/macros/*` as members and explicitly
excludes `crates/macros/Cargo.toml`, so only the sub-crates build. Everything
here is compile-time only — no runtime types, no features — and therefore links
into every server profile, including `minimal`, because the code it generates is
not optional.

## Reference / upstream

| Reference | Relation |
| --- | --- |
| `Cargo.toml:3-18` | Workspace registers `crates/macros/*` as members and excludes `crates/macros/Cargo.toml`, keeping this directory a namespace rather than a crate. |
| Upstream Stump `core/src/config/stump_config.rs` | `StumpConfigGenerator` exists to generate that struct's loaders; still derived there after the config split (`bc347a78`, integrated `7617c66d`). |
| `crates/models/src/entity/*.rs`, `crates/graphql/src/filter/mod.rs` | The two surfaces `filter-gen` targets: entity ordering enums (behind `models/graphql`) and the GraphQL filter inputs. |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Each macro is its own crate, not a module of `stump_core`/`models` | A `proc-macro = true` crate compiles for the host and can export nothing but macros, so the macros cannot share a crate with the types they generate code for. | `filter-gen/Cargo.toml:7-9`, `stump-config-gen/Cargo.toml:7-8` |
| `filter-gen` — `Ordering` and `IntoFilter` derives for the SeaORM/GraphQL query surface | Sort enums and order-by inputs are mechanical per entity; hand-writing them drifts from the SeaORM `Column` enum. | `filter-gen/README.md` |
| `stump-config-gen` — `StumpConfigGenerator` derive for config structs and their partials | Defaults, TOML partials and env loading are three parallel restatements of the same field list. | `stump-config-gen/README.md` |
| Neither crate depends on workspace runtime crates; generated code resolves `crate::CoreError`, `Column`, `OrderBy` etc. at the expansion site | Keeps the macros buildable in isolation and free of the dependency cycle they would otherwise create with `core`/`models`. | `filter-gen/Cargo.toml:11-15`, `stump-config-gen/Cargo.toml:10-13` |
| Both are unconditional path dependencies of their consumers | Config loading and entity ordering are not feature-gated at the crate level; `Ordering` expansion is gated inside `models` by `#[cfg_attr(feature = "graphql", ...)]`. | `core/Cargo.toml:74`, `crates/models/Cargo.toml:25`, `crates/graphql/Cargo.toml:33` |

## Layout

| File | Responsibility |
| --- | --- |
| `filter-gen/` | `Ordering` + `IntoFilter` derives — see `filter-gen/README.md` |
| `stump-config-gen/` | `StumpConfigGenerator` derive — see `stump-config-gen/README.md` |

## How to verify

```text
cargo test -p filter-gen
cargo test -p stump-config-gen
cargo check -p models --features graphql
cargo check -p stump_core
```

The macro tests cover expansion in isolation; the two downstream checks are the
only proof that the generated code still compiles against the real entities and
config structs.

## Deep docs

- `crates/README.md` — crate index
- `docs/content/docs/guides/configuration/server-config.mdx` — config keys `stump-config-gen` loads
