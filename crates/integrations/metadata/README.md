# metadata_integrations

## Purpose

`metadata_integrations` owns the outbound *metadata provider* clients and the
pure logic around them: the `MetadataProvider` trait, ten provider
implementations (Comic Vine, Hardcover, AniList, MyAnimeList, MangaDex,
MangaUpdates, Open Library, Google Books, Metron, Audible),
`create_provider`/`requires_api_token` registry, per-provider
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
| Open Library `https://openlibrary.org` (politeness ~1 req/s) | `src/providers/openlibrary.rs:1-16` |
| Google Books `https://www.googleapis.com/books/v1` (1,000 req/day anonymous) | `src/providers/googlebooks.rs:1-13` |
| Metron `https://metron.cloud/api` (Basic auth, ~30 req/min; field shapes from the official `Metron-Project/mokkari` client) | `src/providers/metron.rs:1-15` |
| Audnexus `https://api.audnex.us` (community-run, 100 req/min published; `GET /books/{asin}`, `GET /authors/{asin}`) plus the unauthenticated Audible catalog `https://api.audible.com/1.0/catalog/products` (`num_results` max 50, `products_sort_by=Relevance`, response groups `product_desc,product_attrs,contributors,series,media`) | `src/providers/audible.rs:1-31`; field shapes verified against live records and `advplyr/audiobookshelf` `server/providers/{Audible,Audnexus}.js` |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Providers are constructed by string id (`COMIC_VINE`, `HARDCOVER`, `ANI_LIST`, `MANGA_UPDATES`, `MAL`, `MANGA_DEX`, `OPEN_LIBRARY`, `GOOGLE_BOOKS`, `METRON`) via `create_provider` | The stored `metadata_provider_config.provider_type` is a string; adding a provider is one match arm + one `requires_api_token` entry. Ids are the stored `models::MetadataProvider` `Display` spelling; `core` provider_cache test guards every variant (a mismatch silently disabled AniList/MangaDex). | `src/lib.rs:36-70` |
| Keyless providers (AniList, MangaDex, MangaUpdates, Open Library, Google Books) ignore the token argument; MAL stores its client id *as* the API token; Metron stores `username:password` (or a pre-encoded Basic token) as the token | Avoids a second credential column; UI hides the key field when `requires_api_token` is false. | `src/lib.rs:41-66`, `src/providers/metron.rs:431-448` |
| Per-second `RateLimiter` per client with conservative defaults: Comic Vine 1, MAL 1, AniList 2, MangaUpdates 2, MangaDex 4, Hardcover 5; Open Library/Google Books 1; Metron 30 and Audible 60 per **minute** | Public APIs advertise minute/hour windows; per-second floors stay under all of them — Metron and Audnexus publish per-minute quotas, so they use `RateLimiter::per_minute`. | `*_DEFAULT_RATE_LIMIT` consts in each provider; `src/rate_limit.rs` |
| HTTP 429 maps to `MetadataProviderError` with `is_rate_limited() == true`; malformed JSON must not | Fetch jobs back off on rate limits but fail fast on provider bugs. | `src/providers/mal.rs:858-887`, `src/providers/mangadex.rs:763-802` |
| Free-text release dates go through `dateparser::parse_with_timezone(.., &Utc)`, never `dateparser::parse` | `parse` anchors date-only input at local midnight and converts to UTC, so hosts east of UTC lose a day (`October 1, 1988` → Sep 30). Provider dates are calendar dates, not instants. | `src/providers/openlibrary.rs:526`, `src/providers/comic_vine/utils.rs:51`, `src/providers/hardcover.rs:423` |
| Shared `build_client_with_retry` (reqwest-middleware + retry + http-cache) | One retry/backoff policy; cache manager narrowed to avoid CACache in the server graph. | `src/client.rs:35-49`; `docs/.../server-architecture.mdx:20` |
| Provider DTOs carry only fields a mapping reads: AniList no longer requests `idMal`/`countryOfOrigin`, MAL detail lookups omit `media_type`, and unread MangaUpdates/MangaDex/Metron echo fields are gone; Metron issues now map their `publisher` to `ExternalMediaMetadata.publisher` | Unread wire fields were dead code that only silenced warnings; the issue publisher had an existing carrier and was simply unmapped. | `src/providers/anilist.rs` `MANGA_FIELDS`; `src/providers/mal.rs` `DETAIL_FIELDS`; `src/providers/metron.rs` `fetch_media_metadata`; `cargo check -p metadata_integrations` is warning-free |
| Scoring floors: ISBN ≥ 0.98, exact title ≥ 0.90, alt-title ≥ 0.80, fuzzy 0–0.75 (Dice-Sørensen + Jaro-Winkler), author +0.05 | Preserve the current search-result ranking contract. | `src/scoring.rs:8-30` |
| Cross-provider work matches compare normalized titles/aliases, year, authors, publisher, and optional provider format; merges require every provider pair to reach `AUTO_MATCH_THRESHOLD` (0.90) | Keep IDs provider-qualified, require strong pairwise evidence, and prevent a weak transitive match from joining unrelated works; unknown evidence is not a conflict. | `src/scoring/cross_provider.rs` |
| `MergeStrategy`: `FillGaps` (default), `PreferExternal`, `PreferExternalAndMergeLists`, `FillAndMergeLists`; per-field overrides via `MetadataFieldOverride` | Users choose whether external data may overwrite curated fields; lists dedupe rather than clobber. | `src/merge/config.rs:11-19`, `src/merge/merger.rs` |
| Provider tests use an in-crate blocking `MockServer` on `127.0.0.1:0`, never the network | Deterministic, offline `cargo test`; every client takes an `api_url` override. | `src/mock_http.rs:1-20` |
| User agent `stump/<version> (+https://github.com/stumpapp/stump)` on providers that require identification | AniList/MangaDex policy. | `src/providers/anilist.rs:140,927` |
| One `AudibleClient` holds **two** base URLs (`audnex_url` + `catalog_url`), each with its own `#[cfg(test)]` override | Audnexus is the record source but has no search endpoint; the Audible catalog searches but answers with a store product. One client keeps one rate limiter, one retry policy and one user agent across both halves, and two overrides let a test aim both at one `MockServer` and still tell the endpoints apart by path. The two upstreams disagree on casing (`publisherName` vs `publisher_name`) and on which fields exist, so each gets its own DTO rather than one struct behind a dozen aliases. | `src/providers/audible.rs:86-150` (bases), `src/providers/audible.rs:663-776` (one DTO per upstream) |
| Narrators are a first-class `ExternalMediaMetadata::narrators`/`MetadataField::NARRATORS`, never folded into `writers` | A narrator is not an author. Folding them in would file a performer in the author column of every audiobook, and nothing downstream could separate them again; new `MetadataField` variants are appended so no existing variant's ordinal shifts. | `src/types/metadata.rs:73-77`, `src/types/enums.rs:54-62` |
| `ExternalMediaMetadata::runtime_minutes` is candidate *evidence*, never a stored value | The advertised runtime is what tells an abridged edition from an unabridged one while an operator picks a match; the authoritative duration is measured from the file itself (`media_audio.duration_ms`) and is never overwritten by a provider's number. | `src/types/metadata.rs:86-93` |
| Hardcover book search falls back to author names parsed from the indexed hit's `author_names` or writer-role `cached_contributors` when detail fetch omits them | Search-indexed contributions remain available even when a detail response has no contributors, so external book results retain authors | `src/providers/hardcover.rs` `search_media_maps_mocked_book_response`; `book_search_document_extracts_cached_contributors` |
| `MetadataProvider::search_media_brief` builds candidates from the search index alone (one request); Hardcover maps `title`, `author_names`/writer `cached_contributors`, `release_year`, `isbns` (10/13 by length), `featured_series`/`series_names` + position, `image.url`, `slug`. Default impl delegates to `search_media`, which keeps its per-hit detail fetch for matching callers | Interactive search made up to 21 sequential Hardcover round-trips (~10 s); the index already carries every field a result list shows. Brief hits keep the index's relevance order and are not `score_search`ed | `src/provider.rs`; `src/providers/hardcover.rs` `search_media_brief_maps_index_document_with_one_request` |
| `BriefSearchCache`: bounded (`capacity`), TTL'd in-memory cache in front of `search_media_brief`, keyed by caller scope + lowercased/whitespace-collapsed title + limit; errors are never cached; full cache drops expired then oldest. It is a thin key layer over the generic `TtlCache<K, V>` | Type-ahead and the `/search` page repeat one term many times; the scope key keeps one credential's results from serving another caller without a credential of its own. The generic map lets callers cache other merged lookups (narrators) with the same bounds | `src/search_cache.rs` tests (`repeated_query_hits_cache_and_different_query_fetches`, `capacity_evicts_oldest_entry`) |
| Hardcover brief hits carry the index's `has_ebook`/`has_audiobook`; Audible products carry `has_audiobook: true`, `abridged` from `format_type` and `language` (`english`/`italian`). Live Hardcover index documents (2026-09-25) have **no** `audio_seconds` key despite the docs, so a Hardcover hit never has a length; it is still parsed leniently (int/float/string) should the index add it. All five ride on `ExternalMediaMetadata` but are `graphql(skip)`ped there | Availability is what the request dialog needs to offer the right format, and it comes free with the one search request; audiobook lengths come from Audible products and Hardcover audio editions (the narrator popover) only. The GraphQL `ExternalMediaMetadata` type used by matching flows stays unchanged and the unified search hit exposes the fields instead | `src/types/metadata.rs`; `search_media_brief_maps_index_document_with_one_request`; `src/providers/audible.rs` `search_media_maps_catalog_products_with_search_params`, `brief_search_keeps_catalog_order_and_never_shortcuts_to_audnexus` |
| `MetadataProvider::audiobook_editions(external_id)` returns `AudiobookEdition { narrators, audio_seconds, asin, language, users_count }`; default is an empty list. Hardcover answers with one `editions` query (`book_id` + `reading_format_id: 2`, most-shelved first, capped at 25) carrying `language { code2 code3 language }` and `users_count`, reading narrators from `contributions` then `cached_contributors` by a role containing "narrat" | The narrator picker wants "who reads this book", not another candidate to match; an empty default lets a merging caller treat "no editions" and "no edition data" alike. Roles are free text on Hardcover, so the match is on the word; the language lets callers drop translations | `src/providers/hardcover.rs` `audiobook_editions_use_one_query_and_read_narrators_from_either_shape`; Hardcover docs `api/GraphQL/Schemas/Editions.mdx` (`reading_format_id` 2 = Audio), `Languages.mdx` |
| `AudibleClient::search_media_brief` is the catalog search alone: no ASIN shortcut, no re-scoring, catalog relevance order. Products the catalog labels a podcast, episode, periodical, newspaper or magazine (`content_type`/`content_delivery_type`) are dropped by `media_from_product` and counted as failed | A type-ahead term is never an ASIN, and the caller wants what Audible ranks first, not the match scorer's reordering; a talk-radio hour is never a book to match or request | `src/providers/audible.rs` `brief_search_keeps_catalog_order_and_never_shortcuts_to_audnexus` |
| `pointed_at(base_url)` mock overrides on `HardcoverClient` and `AudibleClient` are public under `feature = "mock"` | Consumer crates (`graphql`) test their merge logic against a local `MockServer` instead of the network | `src/providers/hardcover.rs`, `src/providers/audible.rs`; `crates/graphql/Cargo.toml` dev-dependency |

## Layout

| Path | Responsibility |
| --- | --- |
| `src/providers/{anilist,audible,mal,mangadex,hardcover,openlibrary,googlebooks,metron}.rs`, `src/providers/comic_vine/` | Provider clients + mappers + tests |
| `src/lib.rs` | Re-exports, `create_provider`, `requires_api_token` |
| `src/provider.rs` | `MetadataProvider` trait (`search_series`, `search_media`, `search_media_brief`, `audiobook_editions`, `score_search`, `fetch_*_metadata`, `verify_credentials`), `ProviderCredentialVerification` |
| `src/mangaupdates.rs` | MangaUpdates client (kept at crate root from upstream layout) |
| `src/types/` | `ExternalMetadata`, `ExternalSeriesMetadata`, `ExternalMediaMetadata`, `AudiobookEdition`, `MatchCandidate`, `SearchQuery`, `SearchOutcome`, enums |
| `src/scoring.rs`, `src/scoring/cross_provider.rs` | `MatchScorer`, `title_similarity`, cross-provider work scoring, ranking, and merge groups |
| `src/merge/` | `MergeStrategy`, `AutoApplyConfig`, `FieldMerger`, overrides |
| `src/rate_limit.rs`, `src/client.rs`, `src/error.rs`, `src/serde_utils.rs` | Infrastructure |
| `src/mock_http.rs` | Test-only mock HTTP server |
| `src/search_cache.rs` | `TtlCache` (bounded TTL map) and `BriefSearchCache` for interactive searches |

## How to verify

```text
cargo test -p metadata_integrations                    # 119 tests, offline via MockServer
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
