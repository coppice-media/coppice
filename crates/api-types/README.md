# stump_api_types

## Purpose

`stump_api_types` owns the two request-level value types that every transport
needs but none should define: `RequestOrigin` (scheme + host of the incoming
request, with the URL-building helpers the OPDS/Komga/Kobo feeds use for
absolute links) and `OffsetPagination` (1-/0-based page arithmetic with the
`page=1, page_size=20` defaults). It deliberately has **no** Axum, GraphQL or
SeaORM dependency and no cursor pagination — cursor/`Unpaginated` variants are
GraphQL-only and stay in `crates/graphql/src/pagination.rs`, which converts
to/from this crate's type.

## Reference / upstream

| Reference | Relation |
| --- | --- |
| Upstream Stump `core/src/context.rs` (`RequestContext`/service-context `format_url`) and `crates/graphql/src/pagination.rs` (pre-`b068deb9`) | Both types were extracted in commit `b068deb9`; `format_url` keeps the "legacy service-context semantics" byte-for-byte. |
| `crates/graphql/src/pagination.rs:33-80` | GraphQL `OffsetPagination` derives its defaults from this crate and implements `From` in both directions. |
| `docs/content/docs/developer/server-architecture.mdx` (§ "Neutral API contracts live in `stump_api_types`") | Boundary statement. |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| `format_url` keeps three branches: `/`-prefixed → append, `http`-prefixed → passthrough, else `origin/path` | OPDS 1.2/2.0, Komga and Kobo link builders relied on this exact behaviour before extraction; changing it would alter feed URLs. | `src/lib.rs:34-48`, tests in `src/lib.rs` |
| Separate `url_for_path` that trims both boundary slashes | Newer callers need a join that never yields `//`; added rather than changing `format_url`. | `src/lib.rs:50-58` |
| `cache_friendly_url` appends `?last_modified=<rfc3339>` only when a timestamp exists | Thumbnail cache-busting for clients that key on URL; no timestamp → identical URL. | `src/lib.rs:60-76` |
| `RequestOrigin::default()` is `http://localhost` | Keeps tests and non-HTTP contexts (jobs, CLI) working without a request. | `src/lib.rs:14-21` |
| `OffsetPagination` defaults: `page=1`, `page_size=Some(20)`, `zero_based=Some(false)` | Same as REST/GraphQL defaults clients already depend on. `Option`s stay so serde-omitted fields round-trip. | `src/lib.rs:79-112` |
| `offset()` = `page * size` when zero-based, `(page-1) * size` otherwise; `previous_page()` returns `None` at the first page in either mode | Both bases exist for Komga (0-based) and Stump REST (1-based) clients. | `src/lib.rs:114-146` |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | `RequestOrigin` + URL helpers, `OffsetPagination` + arithmetic, unit tests |
| `Cargo.toml` | `chrono`, `serde` only |

## How to verify

```text
cargo test -p stump_api_types                                      # URL formatting and offset arithmetic
cargo check -p stump_server --no-default-features --features minimal
```

Harness (from `../komga-compat/`): `make replay` — Komga page/`size` handling
and absolute link generation go through these types; `make replay-mihon`
covers zero-based `page` on `/api/v1/books`.

## Deep docs

- `docs/content/docs/developer/server-architecture.mdx`
- `docs/content/docs/developer/komga-compat.mdx` — paging parameters clients send
