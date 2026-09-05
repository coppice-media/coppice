# stump_auth

## Purpose

`stump_auth` owns the *authenticated request context* shared by every transport
in Stump: the `AuthContext` (resolved `AuthUser` plus optional API key) and the
neutral `AuthorizationError` (`LockedAccount`, `ForbiddenAction`) with the
permission/server-owner enforcement helpers on top of it. It deliberately does
**not** authenticate: no session store, cookie, Basic/Bearer parsing, OIDC, JWT
or password hashing lives here — that is `apps/server/src/middleware/auth.rs`.
It also has no Axum, GraphQL or database dependency, so protocol crates
(`stump_komga`, `stump_opds`, `stump_kobo`, `stump_koreader`,
`stump_liseur_sync`) and `graphql` all consume the same type.

## Reference / upstream

| Reference | Relation |
| --- | --- |
| Upstream Stump `core/src/context.rs` / server `middleware/auth.rs` (pre-`b068deb9`) | `AuthContext` and the error enum were extracted verbatim from the server-local types in commit `b068deb9` ("neutral contract crates"). Semantics unchanged. |
| `models::shared::permission_set::user_has_all_permissions` | Source of truth for permission inheritance; this crate only adds lock/owner ordering on top. |
| `docs/content/docs/developer/server-architecture.mdx` (§ "Authentication context lives in `stump_auth`") | Documents the boundary: neutral errors here, GraphQL maps them at its adapter. |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Server owner bypasses every check, including the locked-account check | Matches upstream behaviour; locking the owner must never brick admin access. | `src/lib.rs:56-58`, test `mod tests` in `src/lib.rs` |
| Locked account is rejected **before** permissions are evaluated | A locked user must get `LockedAccount`, not `ForbiddenAction`, regardless of their permission set. | `src/lib.rs:60-62` |
| `AuthorizationError` is `Copy`, `Eq`, `thiserror` and carries user-facing messages | Transport adapters (`graphql::error`, Axum `APIError`) map it to 401/403 without re-wording. | `src/lib.rs:6-17` |
| No framework dependencies (`models` + `thiserror` + `tracing` only) | Lets protocol crates and the `minimal` server profile link it without `async-graphql`/`axum`. | `Cargo.toml` |
| `api_key` travels with the context | OPDS/Komga/Kobo paths authenticate with API keys and later need the key identity (scoping, audit). | `src/lib.rs:19-29` |
| Accessors clone (`user()`, `id()`, `api_key()`) | Kept for upstream API parity with the moved type; callers on hot paths use the public fields directly. | `src/lib.rs:31-45` |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | `AuthorizationError`, `AuthContext`, `enforce_permissions`, `user_and_enforce_permissions`, `enforce_server_owner`, `server_owner_user`, unit tests |
| `Cargo.toml` | Depends only on `models` (no default features), `thiserror`, `tracing` |

Consumers (2026-09-05): `apps/server` (21 files), `crates/graphql` (56),
`crates/komga` (11), one file each in `kobo`, `koreader`, `liseur-sync`, `opds`.

## How to verify

```text
cargo test -p stump_auth                                   # unit tests (owner bypass, lock ordering, permission inheritance)
cargo check -p stump_server --no-default-features --features minimal   # must link without async-graphql/axum guards
cargo test -p stump_server --lib                           # middleware/auth.rs consumes AuthContext
```

Harness replay that exercises it end-to-end (from `../komga-compat/`):
`make replay-negative-auth` (403/401 mapping) and `make replay` (API-key and
Basic contexts on Komga routes). Live probe: any authenticated request against
the `stump-komga` fixture (`http://127.0.0.1:25600`, credentials in the
launcher-generated `ENDPOINTS.md`).

## Deep docs

- `docs/content/docs/developer/server-architecture.mdx` — crate boundaries, feature profiles
- `docs/content/docs/developer/komga-compat.mdx` — Basic/API-key scope rules that produce this context
- `.omp/RULES.md` — "OPDS Basic auth is unconditional; `X-Stump-Save-Session` controls session creation only"
