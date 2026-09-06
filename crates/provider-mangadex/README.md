# stump_provider_mangadex

## Purpose

`stump_provider_mangadex` is the one compiled `stump_provider::Source`: the
MangaDex API bound to a language (`mangadex-en`, `mangadex-ja`, ...). It maps
`/manga`, `/manga/{id}`, `/manga/{id}/feed`, and `/at-home/server/{chapterId}`
onto `RemoteSeries`/`RemoteChapter`/`RemotePage`, and reports MangaDex@Home
image fetches back to the network as its rules require. It owns no route, no
persistence, and no policy: the host (`crates/provider`) decides what is
cached, materialised, or served.

## Reference / upstream

| Reference | Pin | Used for |
| --- | --- | --- |
| MangaDex OpenAPI | <https://api.mangadex.org/docs/static/api.yaml> | every DTO field below; `ChapterAttributes`, `MangaAttributes`, `CoverAttributes`, `/at-home/server` responses |
| MangaDex@Home report | <https://api.mangadex.network/report> | `MangaDexSource::report` (required for non-`uploads.mangadex.org` nodes) |
| Keiyoushi extension | `eu.kanade.tachiyomi.extension.all.mangadex` (`CATALOG_PKG`) | catalog entry this implementation stands in for |
| Mihon MangaDex extension | browse/search/feed query shape (`order[...]`, `contentRating[]`, `includes[]`) | `manga_query`, `chapters` |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| One limiter per instance at 5 requests/second (`REQUESTS_PER_SECOND`) | MangaDex's documented global limit; browse, details, feed, and at-home all share it | `src/lib.rs::MangaDexSource::new`; `stump_provider::RateLimiter` |
| Chapter feed is paged in full (`limit=500`) and **every** entry is returned | The host reports how many chapters a source cannot serve; dropping them in the source hides that from the operator | `src/lib.rs::chapters`; test `chapters_follow_feed_pagination_and_flag_unreadable` |
| `readable = false` when `externalUrl` is set, `pages == 0`, or `isUnavailable`; `external_url` is carried through | Spec: `externalUrl` "Denotes a chapter that links to an external source", `pages` is the "Count of readable images for this chapter", `isUnavailable` withholds a listed chapter — `/at-home/server/{chapterId}` answers `404` for all three | `src/lib.rs::Chapter::readable`; spec `ChapterAttributes`, `/at-home/server/{chapterId}` `404` |
| `includes[]=cover_art` on the list, search, **and** detail queries; cover URL is `{COVER_URL}/{manga}/{fileName}.512.jpg` | Mode B materialises a series straight out of a browse response, so the cover must survive the list path, not only `/manga/{id}`; the `.512` variant is the resized cover MangaDex serves for the same file | `src/lib.rs::{manga_query,details}`; test `list_search_and_detail_queries_all_carry_the_cover` |
| `contentRating` maps onto `ContentRating` (`safe`/`suggestive`/`erotica`/`pornographic`); `nsfw` stays the adult-only projection | The host needs the full rating for `series_metadata.age_rating`, while `nsfw` remains the Mihon-shaped boolean other code already reads | `src/lib.rs::From<Manga> for RemoteSeries`; test `content_rating_maps_every_documented_value` |
| An absent or unknown `contentRating`/`status` maps to `None`/`Unknown`, never a guess | A new MangaDex vocabulary value must not silently become a rating the operator did not ask for | `src/lib.rs::From<Manga> for RemoteSeries`; test `content_rating_maps_every_documented_value` |
| Titles prefer `en`, then `ja-ro`, then any locale; `links` keeps only bare cross-registry ids | The stored row needs one deterministic title, and dedupe keys must be ids rather than per-site URLs | `src/lib.rs::preferred`; `stump_provider::identity::EXTERNAL_REGISTRIES` |
| Image fetches are reported to `api.mangadex.network/report` unless the host is `uploads.mangadex.org` | MangaDex@Home requires success/bytes/duration reports; covers served from `uploads` are not @Home traffic | `src/lib.rs::{is_home_node,report}`; test `pages_build_at_home_urls_and_fetch_reports_to_network` |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | `factory()`, `MangaDexSource`, query builders, the API DTOs, and their `RemoteSeries`/`RemoteChapter` projections |

## How to verify

```text
cargo test -p stump_provider_mangadex        # mapping, feed paging, readability flags, ratings, @Home report (no network)
cargo test -p stump_provider_mangadex -- --ignored   # live API smoke: search → details → chapters → first page
```

## Deep docs

- `crates/provider/README.md` — the host: caching, materialisation, GC, health.
- `docs/content/docs/developer/provider-host.mdx` — "Unavailable chapters",
  "Covers", and "Age rating" describe what the host does with the fields this
  crate reports.
