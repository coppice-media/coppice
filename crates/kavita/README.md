# stump_kavita

## Purpose

`stump_kavita` is the Kavita compatibility **profile**: the `/api/<Controller>`
routes, DTOs, filters and integer identity mapping that the inventoried Kavita
clients (Kavita Tachiyomi extension, Mihon tracker, Kover, Turnleaf, Kamigura,
Kamare, Inkita) call, pinned to Kavita 0.9.1.4. It owns request routing
(`routes/`), the Stump→Kavita mapper, the `SeriesFilterV2Dto` → SQL planner,
the `kavita_ids`/`kavita_progress`/`kavita_on_deck_removals` side tables and the
Kavita JWT. It reaches persistence through `KavitaBackend` (a `DatabaseConnection`
accessor plus async file/image/credential methods). It deliberately does **not**
own authentication middleware, route mounting or the concrete backend
(`apps/server/src/routers/kavita/`), nor the full Kavita API.

## Reference / upstream

| Reference | Pin | Used for |
| --- | --- | --- |
| Kavita | 0.9.1.4, `develop` `d77d956b9551227d8be2ee488b08f14aa3a341e5` | Controller/service semantics quoted in doc comments |
| `kavita-ref` container | `jvmilazz0/kavita@sha256:31181a32…`, port 25620 (`komga-compat/kavita/`) | `openapi.json`, web-UI capture (`capture/webui-requests.jsonl`), Book/LightNovel library capture (`capture/book-library.json`) |
| Clients | extension `5f3c068c`, Mihon `5fadea61`, Kover `0bd8d084`, Turnleaf `b54f1f71`, Kamigura `f4baeff4`, Kamare `b85ab0a8`, Inkita `67c1f4a9` | Route inventory and query casing (`kavita-compat.mdx`) |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Integer ids come from `kavita_ids` `(kind, stump_id)` rows on one AUTOINCREMENT sequence; `KavitaIds::lookup_any` resolves an id to its kind | Ids must be stable and reverse-resolvable; one sequence lets a `seriesId` parameter accept two kinds without probing | `src/ids.rs:19-27,189-210`; `crates/migrations/src/m20260912_000000_add_kavita_compat.rs` |
| A media item is both the Kavita volume and its chapter (`volumeId == chapterId`), `number "-100000"` (`Parser.DefaultChapterNumber`) | Kavita's single-file-volume shape | `src/mapper.rs:39-40,332-440` |
| Book/WebNovel/LightNovel libraries present **every media item as its own Kavita series** (`SeriesKind::Book`), id kind `book_series` keyed by the media id | Kavita's Book/LightNovel scanner makes one series per file; collection folders showed one series "books" with volumes 1..N | `src/ids.rs:24`; `src/mapper.rs:112-118,453-460`; `src/routes/query.rs:34-57,280-360`; `kavita-ref` `book_series_list` (`book-library.json:95-185`) |
| A book's volume is the loose-leaf volume (`min/max/number -100000`, `name "-100000"`) holding one special chapter (`isSpecial: true`, `range`/`title` = title or file stem, `sortOrder -100000`, `count 0`, `totalCount 1`, `volumeTitle ""`, `publicationStatus 0`, `wordCount 0`) | Copied field-for-field from `kavita-ref` for a Book-library EPUB; LightNovel and a Comic-library PDF give the same shape | `src/mapper.rs:43,196-199,332-407`; `book-library.json:235-238,249-256,279-283,338-341,1330-1489,1630-1755` |
| Book `series-detail`: chapter under `specials`, `volumes []`, `totalCount 1`, `unreadCount` while unread | `kavita-ref` `book_series_detail_4` / `lightnovel_series_detail_6` | `src/mapper.rs:753-769`; `book-library.json:525-530` |
| Book series metadata from the media metadata: `maxCount 1`, `totalCount 1`, `publicationStatus 2` (Completed), `releaseYear` = year, no tags | `kavita-ref` `book_metadata_4` | `src/mapper.rs:647-675`; `book-library.json:585-589` |
| Book `SeriesDto`: `name == originalName` = title else file stem, `folderPath`/`lowestFolderPath` = the file's folder, `created`/`lastFolderScanned` from the file, `wordCount 0` | `kavita-ref` `book_series_list`/`book_series_4` | `src/mapper.rs:573-642`; `book-library.json:98-129,187-230` |
| Every `seriesId` route resolves through `find_series_input` (series or book); a `book_series` id whose media left a Book library is unknown | One resolution path for detail, volumes, metadata, cover, continue-point, has-progress, mark-read, remove-from-on-deck, Tachiyomi | `src/routes/query.rs:94-160,280-360`; `src/routes/image.rs:81-97` |
| Listings select `(kind, id)` keys with `series` rows outside Book libraries `UNION ALL` `media` rows inside them, ordered/counted/paged in SQL, then load only the page | One convention for filters, sort, `Pagination` total and paging across both kinds; Manga/Comic output unchanged | `src/routes/query.rs:487-551`; `src/routes/series.rs:178-223`; tests `routes::series::books` |
| `series_filter::plan` builds every statement and sort for both `Target`s (`series`/`series_metadata` vs `media`/`media_metadata`); books: `PublicationStatus` = Completed, `Tags` none, `Path` = file path | A `SeriesName` filter must match the book, not its folder | `src/routes/series_filter.rs:68-137,589-1168` |
| `on-deck`/`recently-updated-series` expand Book-library series into books; a recent book is its own group (`count 1`) | `kavita-ref` `recently_updated` lists each book separately | `src/routes/series.rs:436-494,519-647`; `book-library.json:1237-1329` |
| `kavita_on_deck_removals` stores the Kavita series' Stump target `(user_id, target_kind 'series'|'media', target_id)` with no FK to series/media (user FK only); read events delete rows, orphans are tolerated | A per-book removal cannot reference `series.id`; hiding sibling books would be silently wrong; a row for a deleted target hides nothing | `crates/migrations/src/m20260922_kavita_on_deck_removals.rs`; `crates/models/src/entity/kavita_on_deck_removal.rs`; `src/routes/series.rs:408-433,499-512` |
| Tachiyomi `latest-chapter` returns a book's chapter unencoded (`number "-100000"`, files kept); `mark-chapter-until-as-read` never marks a book | `kavita-ref` `book_latest_chapter_4` and the mark-until probe | `src/routes/tachiyomi.rs:85-112,145-156`; `book-library.json:1006`, `tachiyomi_mark_until_probe` |
| Book progress goes through the media's `reading_sessions` row (`upsert_reading_session`) plus `kavita_progress` and the unified reading head | Same row OPDS/Komga/native use; the book series reads it back | `src/routes/reader.rs:228-289`; test `routes::reader::book_progress` |
| Manga/Comic EPUBs keep the grouped volume shape (`number` from metadata/position, `isSpecial false`) although Kavita builds a loose-leaf special for them too | Out of this change's scope; `kavita-compat.mdx` Known deviations | `book-library.json:1630-1755` (comic PDF) |
| `chapter-info` copies `GetChapterInfo` verbatim, including `libraryType: 0`; `pageDimensions` come from `media_analysis`, `doublePairs` from `GetPairs`, and `extractPdf` is accepted and ignored | Kavita never assigns `LibraryType` there, so a faithful response is the default; Stump already records page dimensions and renders PDFs on demand | `src/routes/reader.rs` `chapter_info_for`; `src/mapper.rs` `map_chapter_info`/`double_pairs`; Kavita `Kavita.Server/Controllers/ReaderController.cs:194-256`, `Kavita.Services/Reading/ReaderService.cs:696-731`; `kavita-ref` `chapter-info?chapterId=1&includeDimensions=true` and `chapterId=4` |
| `Download/chapter-size` returns the media's byte size and `200`/`0` for an unresolvable chapter | `kavita-ref` answers `0` for `chapterId=9999`; the OpenAPI type is `int64` bytes, not megabytes | `src/routes/download.rs`; `kavita-ref` `chapter-size?chapterId=1` → `3522110` (file is 3522110 B), `?chapterId=9999` → `0` |
| `reading-profile/{libraryId}/{seriesId}` always serves Kavita's `Default Profile` (`kind: 0`) | Stump keeps no reader settings; `kind: 0` is the branch Kamigura reads to fall back to its own setting | `src/dto.rs` `UserReadingProfileDto::default_profile`; Kamigura `reader/ReaderScreen.kt:556-560`; `kavita-ref` `reading-profile/2/4?skipImplicit=false` |
| Want-to-read membership is `favorite_series` for a Stump series and `favorite_media` for a Book-library book | A book series has no `series` row of its own; both tables are the per-user shelf every Stump surface already reads | `src/routes/want_to_read.rs`; `crates/models/src/entity/favorite_{series,media}.rs` |
| A Kavita reading list is a Stump reading list; `update-by-*` reads the current membership and appends through `KavitaBackend::set_read_list_items` | One canonical container service for native, Komga, Kobo and Kavita writes; Kavita's own `AddChaptersToReadingList` is append-and-dedup | `src/routes/reading_list.rs`; `apps/server/src/routers/kavita/backend.rs`; `core/src/collections/service.rs`; Kavita `ReadingListController.cs:287-330,428-500` |
| The `Book` routes address Stump's synthetic page space: a spine item is worth `ceil(compressed/1KiB)` pages, `book-page?page=` resolves the item covering that page, and `epub://` URIs are rewritten onto `book-resources` | `book-info.pages` must equal the chapter's `pages` so book-reader progress lands in the same space as `Reader/progress`; every `0..pages-1` stays addressable (alice.epub: 15 spine items, 83 synthetic pages) | `src/routes/book.rs`; `crates/media/src/media/format/epub.rs` `spine_page_budget`/`structure`; Inkita `data/api/KavitaApi.kt:183-191,292-296`, `ui/reader/screen/ReaderScreen.kt:1108-1112` |
| Turnleaf's "No EPUB books found" needs a `series-detail` chapter with `format == 3` **and** `files[].extension == ".epub"`, reachable from `chapters ∪ specials ∪ storylineChapters` | Book-library series already put that chapter under `specials`; a Manga/Comic-library EPUB stays inside `volumes[]`, which Turnleaf never inspects — the same shape real Kavita returns for a multi-volume series | Turnleaf `src/lib/kavita/mapper.ts:11-14,38-41`, `src/lib/kavita/client.ts:41,61`, `src/lib/components/Library.svelte:576-577,830`; `src/mapper.rs:753-769`; Kavita `Kavita.Services/SeriesService.cs:596-625` |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs`, `src/dto.rs`, `src/filter.rs`, `src/errors.rs`, `src/macros.rs` | re-exports; Serde DTOs in Kavita field order; `SeriesFilterV2Dto` + `SmartFilterHelper` codec; `APIError` |
| `src/ids.rs`, `src/progress.rs`, `src/auth.rs` | `kavita_ids` kinds and lookups; `pagesRead` ↔ sessions, `kavita_progress`; Kavita JWT |
| `src/mapper.rs` | Stump rows → `LibraryDto`/`SeriesDto`/`VolumeDto`/`ChapterDto`/metadata/detail; `SeriesKind`, book shape |
| `src/routes/mod.rs` | `KavitaBackend`, `KavitaImage`, `router`/`public_router`, `is_kavita_path` |
| `src/routes/query.rs` | `SeriesKey`, loaders for both kinds, `select_series_keys` union listing |
| `src/routes/series_filter.rs` | filter → `FilterPlan` (`Target::Series`/`Target::Book` conditions, `SortKey`s) |
| `src/routes/{download,reading_profile,want_to_read,reading_list,book}.rs` | chapter size; the default reading profile; the want-to-read shelf; reading lists; the EPUB reader |
| `src/routes/{series,reader,tachiyomi,image,library,metadata,users,account,server,filter}.rs` | Kavita controllers |
| `src/test_support.rs` | in-memory DB with the Kavita side tables, `TestBackend`, library/series fixtures |

## How to verify

```text
cargo test -p stump_kavita          # 57 inline tests (ids, mapper, filters, on-deck, book series, progress, chapter-info, want-to-read, reading lists)
cargo check -p stump_kavita
cd ../komga-compat && make replay-kavita API_KEY=…   # specs/kavita.hurl against a running server
```

Live probe: `komga-compat/kavita/README.md` (`kavita-ref`, port 25620); the
Book library (id 2) and Light Novel library (id 3) hold `BookLib/Collection/`
and `LightNovelLib/Shelf/` with `alice.epub` + `leaves.epub`.

## Deep docs

- `docs/content/docs/developer/kavita-compat.mdx` — authoritative profile spec (identity mapping, filters, dashboard streams, route matrix, deviations).
- `docs/content/docs/developer/client-verification.mdx`, `clients.mdx`, `platforms.mdx` — client rows.
- `/home/al/Code/komga-compat/kavita/capture/book-library.json` — Book/LightNovel reference capture cited above.
