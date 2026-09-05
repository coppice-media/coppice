# metadata_integrations

## Purpose

`metadata_integrations` owns the outbound *metadata provider* clients and the
pure logic around them: the `MetadataProvider` trait, six provider
implementations (Comic Vine, Hardcover, AniList, MyAnimeList, MangaDex,
MangaUpdates), `create_provider`/`requires_api_token` registry, per-provider
rate limiting (`governor`), retrying HTTP client, candidate scoring, and the
`FieldMerger`/`MergeStrategy` rules that decide how external fields land on
existing metadata. It deliberately does **not** persist anything, hold
credentials, schedule fetch jobs, or talk to the library: those live in
`core/src/filesystem/metadata/` and `core/src/ingest/providers/`
(`provider_cache.rs`, `fetch_job.rs`, `apply.rs`). GraphQL derives on its
enums/DTOs are opt-in (`graphql` feature).

## Reference / upstream

| Reference | Relation |
| --- | --- |
| Upstream `stumpapp/stump` `crates/integrations/metadata` (merge base `37fdb7d7`) | Comic Vine + Hardcover providers, scoring and merge rules are upstream. |
| Komf `master` `428dac6028cc22db47b41ff9f979d253d2095715` (`komf-core/src/commonMain/kotlin/snd/komf/providers/{anilist,mal,mangadex,mangaupdates}/`) | AniList, MAL, MangaDex, MangaUpdates providers mirror Komf's mappers (title preference, HTML stripping, tag ranking, staff-role expansion). Added in `294bd461`. |
| AniList GraphQL `https://graphql.anilist.co` (30 req/min, 90 degraded) | `src/providers/anilist.rs:1-12` |
| MyAnimeList v2 `https://api.myanimelist.net/v2` (`X-MAL-CLIENT-ID`) | `src/providers/mal.rs:1-11` |
| MangaDex `https://api.mangadex.org` (`GET /manga`) | `src/providers/mangadex.rs:1-10` |
| MangaUpdates v1 `https://api.mangaupdates.com/v1` | `src/mangaupdates.rs:1-9` |
| Comic Vine `https://comicvine.gamespot.com/api` (200 req/hour) | `src/providers/comic_vine/client.rs:27-30` |
| Hardcover `https://api.hardcover.app/v1/graphql` | `src/providers/hardcover.rs:27,44`; account/progress sync is *proposed only* (`docs/.../hardcover-integration.mdx`) |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Providers are constructed by string id (`COMIC_VINE`, `HARDCOVER`, `ANILIST`, `MANGA_UPDATES`, `MAL`, `MANGADEX`) via `create_provider` | The stored `metadata_provider_config.provider_type` is a string; adding a provider is one match arm + one `requires_api_token` entry. | `src/lib.rs:31-56` |
| Keyless providers (AniList, MangaDex, MangaUpdates) ignore the token argument; MAL stores its client id *as* the API token | Avoids a second credential column; UI hides the key field when `requires_api_token` is false. | `src/lib.rs:38-48,53-56` |
| Per-second `RateLimiter` per client with conservative defaults: Comic Vine 1, MAL 1, AniList 2, MangaUpdates 2, MangaDex 4, Hardcover 5 | Public APIs advertise minute/hour windows; per-second floors stay under all of them. | `*_DEFAULT_RATE_LIMIT` consts in each provider; `src/rate_limit.rs` |
| HTTP 429 maps to `MetadataProviderError` with `is_rate_limited() == true`; malformed JSON must not | Fetch jobs back off on rate limits but fail fast on provider bugs. | `src/providers/mal.rs:858-887`, `src/providers/mangadex.rs:763-802` |
| Shared `build_client_with_retry` (reqwest-middleware + retry + http-cache) | One retry/backoff policy; cache manager narrowed to avoid CACache in the server graph. | `src/client.rs:35-49`; `docs/.../server-architecture.mdx:20` |
| Scoring floors: ISBN ≥ 0.98, exact title ≥ 0.90, alt-title ≥ 0.80, fuzzy 0–0.75 (Dice-Sørensen + Jaro-Winkler), author +0.05 | Explicitly provisional baseline from upstream; tests pin current behaviour. | `src/scoring.rs:8-30` |
| `MergeStrategy`: `FillGaps` (default), `PreferExternal`, `PreferExternalAndMergeLists`, `FillAndMergeLists`; per-field overrides via `MetadataFieldOverride` | Users choose whether external data may overwrite curated fields; lists dedupe rather than clobber. | `src/merge/config.rs:11-19`, `src/merge/merger.rs` |
| Provider tests use an in-crate blocking `MockServer` on `127.0.0.1:0`, never the network | Deterministic, offline `cargo test`; every client takes an `api_url` override. | `src/mock_http.rs:1-20` |
| User agent `stump/<version> (+https://github.com/stumpapp/stump)` on providers that require identification | AniList/MangaDex policy. | `src/providers/anilist.rs:140,927` |

## Layout

| Path | Responsibility |
| --- | --- |
| `src/lib.rs` | Re-exports, `create_provider`, `requires_api_token` |
| `src/provider.rs` | `MetadataProvider` trait (`search_series`, `search_media`, `score_search`, `fetch_*_metadata`, `verify_credentials`), `ProviderCredentialVerification` |
| `src/providers/{anilist,mal,mangadex,hardcover}.rs`, `src/providers/comic_vine/` | Provider clients + mappers + tests |
| `src/mangaupdates.rs` | MangaUpdates client (kept at crate root from upstream layout) |
| `src/types/` | `ExternalMetadata`, `ExternalSeriesMetadata`, `ExternalMediaMetadata`, `MatchCandidate`, `SearchQuery`, `SearchOutcome`, enums |
| `src/scoring.rs` | `MatchScorer`, `title_similarity` |
| `src/merge/` | `MergeStrategy`, `AutoApplyConfig`, `FieldMerger`, overrides |
| `src/rate_limit.rs`, `src/client.rs`, `src/error.rs`, `src/serde_utils.rs` | Infrastructure |
| `src/mock_http.rs` | Test-only mock HTTP server |

## How to verify

```text
cargo test -p metadata_integrations                    # ~80 tests, offline via MockServer
cargo test -p metadata_integrations --features graphql
cargo check -p stump_server --no-default-features --features minimal   # no async-graphql derives pulled in
cargo test -p stump_core --lib --tests                 # fetch/apply/provider_cache consumers
```

Live probe: Settings → Metadata providers on the `stump-komga` fixture
(`http://127.0.0.1:25600`), "verify credentials" then a series search; provider
search results also drive the ingest editor (`docs/.../modular-ingest.mdx`).
No Hurl replay covers outbound providers (`make replay-komf` exercises the
Komga *write* API used by Komf, not this crate).

## Deep docs

- `docs/content/docs/developer/modular-ingest.mdx` — provider search in the ingest flow
- `docs/content/docs/developer/liseur-providers.mdx`
- `docs/content/docs/developer/hardcover-integration.mdx` — proposed, not implemented
- `docs/content/docs/developer/server-architecture.mdx` — lazy provider clients, narrowed http-cache
