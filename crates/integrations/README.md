# crates/integrations

## Purpose

Directory index for the two third-party *integration client* crates. Both are
outbound-only: they shape requests to someone else's API and map the answer to a
neutral type. Neither persists anything, holds credentials or schedules work.

They are not equal citizens. `metadata_integrations` is always in the graph
(three dependants). `integrations` (the Discord/Telegram notifiers) has **no
dependant at all** — it compiles solely because the root `Cargo.toml` lists
`members = [..., "crates/integrations/*"]`, and the live notification lane is
`crates/notify` (`stump_notify`), whose channels are ntfy, email and webhook —
it does not cover Discord or Telegram.

This directory is a grouping only: it has no manifest, no `src/` and no crate
doc (root `Cargo.toml:15-18` still `exclude`s a `crates/integrations/Cargo.toml`
that no longer exists). This file is the index; each sub-crate README is
authoritative for its own contract.

## Reference / upstream

| Reference | Relation |
| --- | --- |
| `crates/integrations/metadata/README.md` | Pinned per-provider detail: base URLs, rate-limit constants, date parsing, scoring floors, merge strategies. Read that, not this file, for provider behaviour. |
| `crates/integrations/notification/README.md` | Discord webhook / Telegram bot request shapes, error mapping, dormancy. |
| `crates/notify/README.md` | Current notification channels, routing precedence and retry classification — the reason the notification crate here is dormant. |
| External APIs: AniList, MyAnimeList, MangaDex, MangaUpdates, Comic Vine, Hardcover, Open Library, Google Books, Metron, Audible/Audnexus; Discord webhooks, Telegram Bot API | URLs and quotas are pinned in the two sub-crate READMEs, not duplicated here. |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Metadata and notification stay two crates, not one `integrations` crate | Nothing is shared: different dependency sets (`governor`, `strsim`, `dateparser`, `http-cache-reqwest` vs `lettre` + `reqwest`), different lifetimes (one always linked, one dormant), and metadata carries the GraphQL-derive question that notification must not inherit. | `metadata/Cargo.toml:10-28` vs `notification/Cargo.toml:6-11` |
| `metadata_integrations` exposes an opt-in `graphql` feature; `integrations` has no features at all | GraphQL derives on provider enums/DTOs are needed only by the schema crate: `crates/graphql` takes `features = ["graphql"]`, while `stump_core` and `crates/ingest` take `default-features = false` so a headless/minimal build never pulls `async-graphql` through a provider client. `stump_core` re-exports the switch inside its own `graphql` feature. | `metadata/Cargo.toml:6-11`, `crates/graphql/Cargo.toml:38`, `core/Cargo.toml:9-16,50`, `crates/ingest/Cargo.toml:15` |
| `metadata/` → `metadata_integrations`: clients + pure logic only, no persistence | Fetch scheduling, credential storage and application of results live in `crates/ingest/src/providers/` (`apply.rs`, `registry.rs`, `facade.rs`) and `core/src/filesystem/metadata/`. Detail and evidence: `crates/integrations/metadata/README.md`. | `crates/integrations/metadata/README.md` (Purpose, Decisions) |
| `notification/` → `integrations`: kept in the tree, unwired | Upstream code, untouched in this fork since the merge base `37fdb7d7`; deleting it would drop the only Discord/Telegram implementations, which `stump_notify` has not re-implemented. Detail and evidence: `crates/integrations/notification/README.md`. | `git log --oneline -- crates/integrations/notification`; `crates/notify/src/` has `ntfy.rs`, `email.rs`, `webhook.rs` and no Discord/Telegram module |
| Provider modules live under `metadata/src/providers/`, except MangaUpdates at the crate root | Upstream layout for `mangaupdates.rs` was left alone rather than churn imports. | `metadata/src/providers/{anilist,audible,googlebooks,hardcover,mal,mangadex,metron,openlibrary}.rs`, `metadata/src/providers/comic_vine/`, `metadata/src/mangaupdates.rs` |

## Layout

| Path | Package | Responsibility |
| --- | --- | --- |
| `metadata/` | `metadata_integrations` | `MetadataProvider` trait, ten provider clients (AniList, Audible/Audnexus, Comic Vine, Google Books, Hardcover, MAL, MangaDex, MangaUpdates, Metron, Open Library), rate limiting, retrying HTTP client, candidate scoring, field-merge rules → `crates/integrations/metadata/README.md` |
| `notification/` | `integrations` | `NotificationClient` trait, `DiscordClient` (webhook), `TelegramClient` (bot API), `NotificationEvent` → `crates/integrations/notification/README.md` |
| (no `Cargo.toml`, no `src/`) | — | Grouping directory; `metadata/` and `notification/` are workspace members via root `Cargo.toml:13` |

## How to verify

```text
cargo test -p metadata_integrations                       # offline, in-crate MockServer
cargo test -p metadata_integrations --features graphql
cargo check -p metadata_integrations --features graphql
cargo test -p integrations                                # 2 tests run, 2 ignored
```

Live-key and live-network provider tests are `#[ignore]`d and read their
credentials at call time, several after a `dotenvy::dotenv().ok()`:
`HARDCOVER_API_TOKEN` (`metadata/src/providers/hardcover.rs:649-655`),
`COMIC_VINE_API_KEY` (`metadata/src/providers/comic_vine/client.rs:514-520`),
`MAL_CLIENT_ID` (`metadata/src/providers/mal.rs:907-911`), `METRON_TOKEN`
(`metadata/src/providers/metron.rs:1038-1041`), `ANILIST_LIVE_TESTS=1`
(`metadata/src/providers/anilist.rs:1141-1145`), plus network-only ignores for
Google Books, Open Library, MangaDex, MangaUpdates and Audible. None of those
keys are configured in this tree: `metadata/.env.example` ships
`HARDCOVER_API_TOKEN=` and `COMIC_VINE_API_KEY=` empty, there is no `.env`, and
no Metron or Google Books entry exists at all. No live provider or
Discord/Telegram call has been exercised here.

## Deep docs

- `docs/content/docs/developer/modular-ingest.mdx` — provider search in the ingest flow
- `docs/content/docs/developer/liseur-providers.mdx`
- `docs/content/docs/developer/notifications.mdx` — the `stump_notify` lane
- `docs/content/docs/developer/hardcover-integration.mdx` — proposed, not implemented
