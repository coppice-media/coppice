# integrations

## Purpose

`integrations` is the upstream notification-client crate: a `NotificationClient`
trait plus two outbound implementations, `DiscordClient` (webhook) and
`TelegramClient` (bot API), and the `NotificationEvent` enum they render into a
message. It owns request shaping only — it holds no credentials of its own, no
routing rules, no retry policy and no persistence; the caller passes the webhook
URL or bot token into the constructor.

It is currently an **orphan workspace member**: it compiles because the root
`Cargo.toml` lists `members = [..., "crates/integrations/*"]`, but no crate in
this tree depends on it (`integrations` appears in no other `Cargo.toml`) and
nothing imports it. The live notification lane is `crates/notify`
(`stump_notify`) with `stump_core::notification` for routing and dispatch;
`stump_notify` ships ntfy, email and webhook channels and does **not** cover
Discord or Telegram, so this crate is dormant rather than replaced feature-for-feature.

## Reference / upstream

| Reference                                                                                     | Relation                                                                                                                                   |
| --------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Upstream `stumpapp/stump` `crates/integrations`                                               | Whole crate is upstream; unchanged in this fork since the merge base `37fdb7d7` (`git log --oneline -- crates/integrations/notification`). |
| Discord execute-webhook JSON (`username`, `avatar_url`, `embeds[].{title,description,color}`) | `src/discord_client.rs:30-43`; embed colour reference linked in-source at `src/discord_client.rs:67`                                       |
| Telegram Bot API `sendMessage` (`https://api.telegram.org/bot<token>/sendMessage`)            | `src/telegram_client.rs:38-42`; API link in-source at `src/discord_client.rs:23`                                                           |
| `crates/notify/README.md`                                                                     | Owns the current channel trait, per-user routing precedence and retry classification; this crate has none of that.                         |
| `docs/content/docs/developer/notifications.mdx:31-33`                                         | States Telegram is intentionally not implemented in the `stump_notify` lane.                                                               |

## Decisions

| Decision                                                                                                                       | Why                                                                                                                                                                                                                  | Evidence                                                                           |
| ------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| Trait is split into a _static_ `payload_from_event` and an async `send_message`                                                | Lets the JSON body be asserted without a network call — the only non-ignored Discord test does exactly that.                                                                                                         | `src/lib.rs:15-22`, `src/discord_client.rs:91-105`                                 |
| Telegram returns `NotificationError::Unimplemented` from `payload_from_event`                                                  | Its `sendMessage` call passes `chat_id`/`text` as query parameters, so there is no request body to build; the trait method still must exist.                                                                         | `src/telegram_client.rs:27-33`                                                     |
| Non-2xx becomes `RequestFailed(<response body>)`, transport failure becomes `ReqwestError` via `#[from]`                       | Keeps the provider's own error text for the log; the two cases are distinguishable, but the crate does not classify them as retryable/fatal — `stump_notify` does that with `ChannelError::Transport` vs `Rejected`. | `src/error.rs:1-11`, `src/discord_client.rs:55-63`, `src/telegram_client.rs:43-51` |
| Credentials arrive as constructor arguments (`webhook_url`; `token` + `chat_id`), each client owning a plain `reqwest::Client` | No config or env reading inside the crate, and no shared middleware stack: an embedder supplies secrets and decides lifetimes.                                                                                       | `src/discord_client.rs:8-20`, `src/telegram_client.rs:8-23`                        |
| Sender identity is hard-coded (`NOTIFIER_ID = "Coppice Notifier"`, `FAVICON_URL`)                                              | Discord webhooks render a per-message username/avatar; one constant keeps every message attributable, while the upstream icon URL remains until Coppice has a canonical hosted asset.                                | `src/lib.rs:12-13`, `src/discord_client.rs:35-36`                                  |
| `NotificationEvent` has exactly one variant, `ScanCompleted`, and pluralises `0`/`>1` as "books"                               | Only event upstream wired; `into_message` is the shared plain-text rendering the Telegram query string uses, while Discord re-formats the same text into an embed description.                                       | `src/event.rs:1-23`, `src/discord_client.rs:39`                                    |
| Live-sending tests are `#[ignore]`d and read env vars at call time                                                             | `cargo test` must stay offline and key-free; see How to verify.                                                                                                                                                      | `src/discord_client.rs:73-89`, `src/telegram_client.rs:59-75`                      |

## Layout

| File                     | Responsibility                                                      |
| ------------------------ | ------------------------------------------------------------------- |
| `src/lib.rs`             | `NotificationClient` trait, `NOTIFIER_ID`/`FAVICON_URL`, re-exports |
| `src/event.rs`           | `NotificationEvent` + `into_message`                                |
| `src/discord_client.rs`  | Discord webhook client, embed payload, tests                        |
| `src/telegram_client.rs` | Telegram bot `sendMessage` client, tests                            |
| `src/error.rs`           | `NotificationError`, `NotificationResult`                           |

## How to verify

```text
cargo test -p integrations          # 2 tests run, 2 ignored
cargo check -p integrations
```

Two tests actually run: `discord_client::tests::test_scan_completed` (asserts
`embeds[0].description`, `src/discord_client.rs:91-105`) and
`telegram_client::tests::test_send_message_failed`, which posts to
`api.telegram.org` with `token = "bad"` and only asserts the call errors, so it
passes on a rejection _or_ on no network at all (`src/telegram_client.rs:77-88`). Both
`test_send_message` tests are `#[ignore = "No token"]` and would panic in
`get_debug_client()` on missing `DUMMY_DISCORD_WEBHOOK_URL` /
`DUMMY_TG_TOKEN` + `DUMMY_TG_CHAT_ID` (`src/discord_client.rs:73-77`,
`src/telegram_client.rs:59-63`). Those variables are **not** configured in this
tree — there is no `.env` and no `.env.example` in this crate (the pre-existing
README's "template provided" does not exist), and `dotenvy` is not a
dev-dependency here (`Cargo.toml:13-14`), so nothing loads them from a file.
No live Discord/Telegram delivery has been exercised.

## Deep docs

- `docs/content/docs/developer/notifications.mdx` — the `stump_notify` lane that superseded this crate
- `crates/notify/README.md` — channel trait, routing precedence, retry classification
