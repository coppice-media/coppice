# stump_provider_themes

## Purpose

Executes the `lib-multisrc` themes from `keiyoushi/extensions-source` through
data-only [`SourceDefinition`](../provider/src/definition.rs) records. One
engine serves each shared theme; site-specific configuration remains JSON.
The crate owns theme engines, a `Send` HTML tree, jsoup-compatible selectors,
and Java-pattern, relative, and locale-aware chapter-date parsing. Schema,
loading, and registry belong to `stump_provider`; API-shaped sources such as
MangaDex use `stump_provider_mangadex`.

| Theme (`theme` value) | Base class | Extensions @ `064c1a0e` |
| --- | --- | --- |
| `madara` | `lib-multisrc/madara/.../{MadaraBase,Madara,MadaraNoAjax}.kt` | 177 |
| `madaralegacy` | `lib-multisrc/madaralegacy/.../Madara.kt` | 104 |
| `mangathemesia` | `lib-multisrc/mangathemesia/.../MangaThemesia.kt` | 140 |
| `mmrcms` | `lib-multisrc/mmrcms/.../{MMRCMS,Dto}.kt` | 5 |

## Reference / upstream

- `keiyoushi/extensions-source` @ `064c1a0e8c58be4b36c21444c3d3df6840636950`
  (2026-09-06, "Add HunlightComics (#18871)"), inspected read-only. Every
  knob below cites the Kotlin member it derives from.
- Selector semantics follow **jsoup**, because the selector tables were written
  against it: case-insensitive class and attribute-value matching,
  `:contains`/`:containsOwn`/`:containsData`, `:has(> a)`, and `abs:` attribute
  resolution.
- Rate-limit semantics follow upstream
  `core/src/main/kotlin/keiyoushi/network/RateLimit.kt` (a bare `rateLimit(n)`
  is *n* per **second**).

### Attribution (Apache-2.0 NOTICE)

The selector tables, request bodies, status/relative-date vocabularies and
lazy-image attribute orders these engines reproduce are derived from
`keiyoushi/extensions-source`, licensed under the Apache License, Version 2.0:

```text
Copyright the Keiyoushi contributors and the Tachiyomi/Mihon contributors.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

No Kotlin is copied: the behavior is reimplemented in Rust. Source definitions
are data assets; this crate implements the parser and theme engines. Coppice is
licensed under MIT.

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| One engine serves `madara` and `madaralegacy` | The two base classes differ only in knobs — `chapter_mode` vs `useNewChapterEndpoint`, `browse_mode` vs `useLoadMoreRequest` — and `MadaraSource` accepts both spellings | `src/madara.rs` `chapter_mode()`/`browse_mode()`; `madara::tests::chapter_mode_reads_the_modern_knob_and_the_legacy_boolean` |
| Remote ids are slugs, not WordPress post ids | Upstream keys series on the numeric post id, which changes on migration and cannot be turned into a URL without a request. A slug is stable, reversible and readable inside a `provider://` path | `src/madara.rs` `parse_archive`; `madara::tests::archive_items_key_on_the_slug_and_prefer_lazy_covers` |
| `parse_archive` does not require `data-post-id` | Current `MadaraBase.parseArchive` skips cards without it; keying on the slug makes the attribute unnecessary, and many hosts omit it | `src/madara.rs` `parse_archive` (deviation from `MadaraBase.kt`) |
| Every extraction point is a knob with the base-class default | Read Comics Online (`mmrcms`) rebuilt its markup and overrides list/details/chapter/page parsing in block-body `override fun`s. Without per-field knobs it would need code | `src/mmrcms.rs`; `mmrcms::tests::redesigned_{cards,details,chapters}*` |
| `:self` sentinel for "the matched element *is* the anchor" | Redesigned hosts make the card itself an `<a>`; a selector cannot express "myself" | `src/theme.rs` `SELF_SELECTOR`; `mmrcms::tests::self_selector_lets_the_card_be_its_own_anchor` |
| Own arena DOM instead of `RcDom` | `markup5ever_rcdom` is `Rc`-based and therefore `!Send`, so it cannot live across an `.await`; flattening once keeps parsing synchronous and the async fns `Send` | `src/dom.rs` `Document::absorb` |
| Own selector engine instead of `scraper`/`selectors` | `selectors` implements CSS, not jsoup: no `:contains`, no `:containsData`, no case-insensitive attribute values, no `:has(> a)`. 56 `:contains` + 29 `:has` + 6 `:containsData` uses across the three themes | `src/selector.rs`; `selector::tests::real_theme_selectors_parse` pins 25 verbatim upstream selectors |
| An unsupported selector is a construction error | A silent mismatch returns an empty series list, which looks like a dead site. Failing at `enableProviderSource` names the knob | `src/theme.rs` `ThemeError::Selector`; `madara::tests::a_bad_selector_knob_fails_at_construction` |
| The `-of-type` family (`:first-of-type`, `:last-of-type`, `:nth-of-type(n\|odd\|even)`) counts siblings by tag name; `:nth-child` and `:nth-of-type` share one argument parser, and `an+b` stays a parse error | `all.allporncomicsco` carries `h3 > a:not([target=_self]):last-of-type` verbatim from its Kotlin, so refusing the pseudo-class refused the whole definition. Counting all element siblings instead of same-tag ones would silently pick the wrong link when a `<span>` precedes the anchors | `src/dom.rs` `Document::type_position`; `src/selector.rs` `Simple::{FirstOfType,LastOfType,NthOfType}`, `Parser::nth_pseudo`; `selector::tests::of_type_counts_only_siblings_with_the_same_tag` |
| Unit words match on word boundaries, not substrings | `MadaraBase`'s `MONTH_WORDS` contains Turkish `ay`, which is a substring of English "days"; upstream turns "2 days ago" into two months | `src/date.rs` `mentions()`; `date::tests::relative_phrases_resolve_against_now` |
| Locale-aware month tables | `chrono` only knows English month names, and `date_locale` is emitted precisely because a Turkish or Indonesian month would otherwise parse wrong or not at all | `src/date.rs` `months_for`; `date::tests::locale_month_names_are_translated_before_parsing` |
| 2 req/s default, declared limits honoured exactly, unreadable limits clamped to 1 req/s | Mihon leaves themed sources unlimited because it is interactive; a server browses for many users. A definition that carries `rate_limit_*` means the host pushed back, so an unparseable value must slow down, not speed up | `src/theme.rs` `rate_limiter`; `theme::tests::rate_limits_honour_the_declared_permits_and_period` |
| AES-protected Madara chapters are refused, not skipped | `#chapter-protector-data` needs a per-site key schedule. Returning no pages looks like an empty chapter; `Unsupported` is visible | `src/madara.rs` `parse_pages`; `madara::tests::encrypted_chapters_are_refused_instead_of_returning_nothing` |
| `ChapterMode::MangaAjaxPaginated` is bounded at 200 pages | Upstream loops until the response repeats; a host that always answers 200 with the same body would spin forever | `src/madara.rs` `MAX_CHAPTER_PAGES` |
| MMRCMS search decides JSON vs HTML from the response body | The base class gets `{"suggestions":[...]}` from `/search?query=`; a redesigned host answers HTML from `/advanced-search`. Deciding on the body means no extra knob and no breakage when a host changes shape | `src/mmrcms.rs` `Source::search`; `mmrcms::tests::json_search_directory_pages_locally` |
| `(?s)` on the `ts_reader` image regex | Upstream's `JSON_IMAGE_LIST_REGEX` assumes a single-line payload; a pretty-printed reader script otherwise yields zero pages | `src/mangathemesia.rs` `images`; `mangathemesia::tests::pages_fall_back_to_the_ts_reader_payload` |
| Pages always carry `Referer` (+ an image `Accept`) | Madara and MangaThemesia CDNs answer 403 without it; upstream sets it in `imageRequest` | `src/theme.rs` `pages_with_referer` |
| The `provider_sources` row's `base_url` overrides the definition's | Lets an operator repoint a source at a mirror without editing the definition repository | `src/theme.rs` `ThemeContext::new`; `madara::tests::base_url_override_from_the_row_wins` |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | `engines()` — the `DefinitionEngine` set registered with the host — plus `supported_themes`/`supports_theme` |
| `src/dom.rs` | Flat, `Send` HTML arena built from `html5ever`; jsoup `text`/`own_text`/`data`/`abs_attr` accessors |
| `src/selector.rs` | jsoup-subset selector parser and evaluator |
| `src/date.rs` | Java date-pattern translation, locale month tables, relative-phrase vocabulary |
| `src/theme.rs` | Knob resolution, rate limiting, lazy-image precedence, status vocabulary, `RemoteSeries`/`RemoteChapter`/`RemotePage` construction |
| `src/madara.rs` | `madara` + `madaralegacy` engine |
| `src/mangathemesia.rs` | `mangathemesia` engine |
| `src/mmrcms.rs` | `mmrcms` engine |

## How to verify

```text
cargo test -p stump_provider_themes                 # synthetic HTML/JSON fixtures
cargo check -p stump_provider -p stump_core -p stump_server \
  --no-default-features --features headless,liseur-sync
```

Live, against a definition repository on disk:

```text
STUMP_ENABLE_PROVIDERS=true \
STUMP_SOURCE_DEFINITIONS_URL=file:///path/to/stump-sources \
  target/debug/stump_server
# then, per theme: enableProviderSource(catalogId) -> browse -> series
# -> chapters -> first page bytes
```

Definitions are data: to reproduce a parse failure, edit the site's JSON under
`$STUMP_SOURCE_DEFINITIONS_URL` and restart — no rebuild.

## Deep docs

- `docs/content/docs/developer/source-definitions.mdx` — schema v1 and the generator
- `docs/content/docs/developer/provider-host.mdx` — "Theme engines and definitions"
- `crates/provider/README.md` — host, catalog, health, materialisation
