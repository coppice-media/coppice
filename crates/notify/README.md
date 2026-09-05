# notify

| | |
| --- | --- |
| **Package** | `stump_notify` (`crates/notify`) |
| **Purpose** | Delivery channels (`ntfy`, `email`, `webhook`), per-user routing rules resolution, and the neutral `Notification`/`Attachment` types. DB access, the dispatch job, and the `CoreEvent` listener live in `stump_core::notification`, not here — this crate is transport-only. |
| **Reference / upstream** | ntfy publish API <https://docs.ntfy.sh/publish/> (`X-` headers, `X-Filename` attachment mode); email crate (lettre) for SMTP; ring HMAC-SHA256 for webhook signing. |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| `NotificationKind` variants are append-only and persist as `SCREAMING_SNAKE_CASE` strings | Names are stored in `notification_rules.event_kind` and queued job payloads; renames would orphan rows | `channel.rs` doc comment; `m20260919` migration |
| `AttachmentContent::Path` defers reads to the channel | Queued dispatch payloads never carry a whole book | `channel.rs` (`Attachment::load`) |
| ntfy topic defaults to `stump-<8 chars user id>-<random>` per user | Parallel installs sharing one ntfy server must not collide | `ntfy.rs::default_topic` |
| Email channel wraps a host-provided `EmailerClientConfig`; per-user setting is only `recipient_email` | SMTP credentials are server-wide (`emailers` table); `sendAttachmentEmail` shares the same path via `EmailChannel::deliver` | `email.rs`; `crates/graphql/src/mutation/emailer/sender.rs` |
| Webhook body signed `X-Stump-Signature: sha256=<hex>` only when a secret is set | Receivers without verification needs shouldn't need a secret | `webhook.rs::signature` |
| `ChannelError::Transport` vs `Rejected` split drives retries | Retrying a 4xx/config error is futile; SMTP/connection failures may be transient | `channel.rs::is_retryable`; core dispatch job retry loop |
| `resolve_channels`: exact-kind rule (even disabled) beats `*` | Users need a mute-one-kind escape hatch on catch-all channels | `routing.rs` tests |
| `Test` kind is not routable | `testNotificationChannel` must not fan out through rules | `routing.rs` (`kind.routable()` gate) |

## Layout

| File | Responsibility |
| --- | --- |
| `channel.rs` | `Channel` trait, `Notification`, `Attachment`, `Recipient`, `ChannelError` |
| `routing.rs` | `Rule`, `Target`, `Audience`, precedence resolution |
| `ntfy.rs` | ntfy push channel (reqwest) |
| `email.rs` | SMTP channel over the `email` crate |
| `webhook.rs` | Signed JSON webhook channel (ring HMAC-SHA256) |
| `registry.rs` | `ChannelRegistry` (id → channel lookup) |

## How to verify

```bash
cargo test -p stump_notify                 # channels, routing, precedence, mock HTTP
cargo check -p stump_notify --no-default-features              # trait + routing only
cargo check -p stump_notify --no-default-features --features ntfy
```

End-to-end delivery, retry, and the `CoreEvent` mapping live in
`stump_core::notification` (`cargo test -p stump_core -- notification`).
Feature docs: `docs/content/docs/developer/notifications.mdx`.
