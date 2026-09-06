# filter-gen

## Purpose

`filter-gen` is the compile-time half of the GraphQL query surface: two derive
macros, `Ordering` and `IntoFilter`. `Ordering` turns a SeaORM entity `Model`
into a `<Name>Ordering` GraphQL enum, a `<Name>OrderBy` input object, and the
`OrderBy<Entity, <Name>OrderBy>` impl that applies them to a `Select<Entity>`.
`IntoFilter` emits an `into_filter(self) -> sea_orm::Condition` body from a
filter input struct. It owns **no** runtime code: the `OrderBy`/`OrderDirection`
trait and enum live in `crates/models/src/shared/ordering.rs`, the filter input
types and the `IntoFilter` trait live in `crates/graphql/src/filter/`, and no
query is ever executed here — the crate only produces tokens.

## Reference / upstream

| Reference | Relation |
| --- | --- |
| `crates/models/src/shared/ordering.rs:4-27` | Defines `OrderDirection` (-> `sea_orm::Order`) and the `OrderBy` trait that the generated `impl OrderBy<Entity, #order_by_ident>` targets; both must be in scope at the derive site. |
| `crates/models/src/entity/library.rs:17-20` | Canonical consumer: `#[cfg_attr(feature = "graphql", derive(Ordering))]` plus `graphql(name = "LibraryModel")`. Same shape in `log.rs`, `media.rs`, `media_metadata.rs`, `series.rs`, `series_metadata.rs`, `smart_list.rs`. |
| `crates/graphql/src/filter/mod.rs:2,28` | Imports the `IntoFilter` derive and declares the `IntoFilter` trait the generated impl targets. |
| SeaORM `DeriveEntityModel` `Column` enum | Generated variant idents must equal SeaORM's column variants; `heck::ToUpperCamelCase` reproduces its casing rule. |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Variant ident = field ident with a leading `r#` trimmed, then `to_upper_camel_case`; `#[sea_orm(enum_name = "...")]` overrides it verbatim | The generated match arms index `Column::#variant`, so they must match SeaORM's own derived naming, including raw identifiers and renamed columns. | `src/ordering.rs:91`, `src/ordering.rs:110-144`, test `build_col_map_enum_name` |
| Generated type names come from `#[graphql(name = "...")]` when present, else the struct ident | Entities are all called `Model`; without the GraphQL alias every entity would emit `ModelOrdering`/`ModelOrderBy` and collide. | `src/ordering.rs:32`, `src/ordering.rs:40-46`, `src/ordering.rs:229-252`, test `test_ordering_impl_graphql_name` |
| `OrderBy` match arms are emitted from the column map's values, sorted | `HashMap` iteration order is non-deterministic; sorting keeps expansion (and the token-comparison tests) stable. | `src/ordering.rs:151-162` |
| `#[ordering(skip)]` drops a field from both the column map and the enum; an empty variant list is a hard `syn::Error` | Non-sortable columns (JSON blobs, computed fields) must not reach `Column::*`, but an empty `async_graphql::Enum` is invalid GraphQL, so it fails at compile time instead. | `src/ordering.rs:93-108`, `src/ordering.rs:119-122`, `src/ordering.rs:205-227`, tests `test_skip`, `test_get_enum_def_empty` |
| Only same-entity columns are generated — no relation or nested ordering | Cross-entity sorting needs joins the macro has no schema knowledge of; it stays in hand-written resolvers. | `src/ordering.rs:254-290` |
| Errors are `syn::Error` at the offending field/struct span, surfaced via `to_compile_error()` | Points the compiler at the field rather than the derive line. | `src/lib.rs:11-17`, `src/ordering.rs:283-286`, test `test_get_ident_malformed` |
| `IntoFilter` is exported but nothing in-tree derives it; every GraphQL filter input implements the trait by hand | The derive predates the current filter inputs, which need `apply_string_filter`-style helpers and generics the macro cannot express. Kept as the target shape, not a live dependency. | `src/lib.rs:19-35`, `crates/graphql/src/filter/library.rs:29` |
| `IntoFilter` treats fields named `_and`/`_or`/`_not` as logical groups (`Condition::all`, `Condition::any`, `Condition::any().not()`); `#[nested_filter]` recurses into the field's own `into_filter()`; everything else needs `#[field_column("Path::To::Column")]` and panics without it | Field names are the only signal available before type resolution; the missing-attribute panic is flagged `TODO: Don't require this!`. | `src/lib.rs:54-95`, `src/lib.rs:261-293` |
| String vs numeric filter arms are selected by unwrapping `Option<FieldFilter<T>>` and matching `T` against a fixed type list; other `T` expands to `unreachable!` | The macro sees tokens, not types, so the inner type name is the discriminator; a wrong pairing fails loudly at runtime rather than silently building no condition. | `src/lib.rs:98-157`, `src/lib.rs:199-259` |
| `doctest = false` | The fenced blocks in the ordering docs show the *generated* enum, not compilable input. | `Cargo.toml:7-9`, `src/ordering.rs:7-23` |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | `#[proc_macro_derive(Ordering, attributes(ordering))]`, `#[proc_macro_derive(IntoFilter, attributes(field_column, nested_filter))]`, the `into_filter` condition codegen and filter-type detection helpers |
| `src/ordering.rs` | Ordering enum / `OrderBy` input object / `OrderBy` impl codegen, `graphql`/`sea_orm`/`ordering` attribute parsing, and the token-level unit tests (`src/ordering.rs:292-713`) |
| `Cargo.toml` | `proc-macro = true`, `doctest = false`; deps are `heck`, `syn`, `quote`, `proc-macro2` only |

## How to verify

```text
cargo test -p filter-gen
cargo check -p models --features graphql
cargo check -p models
cargo check -p graphql
```

The `models --features graphql` check is the only one that expands `Ordering`;
the bare `models` check proves the derive stays behind the feature gate.

## Deep docs

- `crates/graphql/README.md` — the filter/order-by surface these macros feed
- `crates/models/README.md` — entity + `OrderBy` trait side of the contract
