# email

## Purpose

`email` owns SMTP sending and nothing else: the `EmailerClientConfig` value
type (sender identity, host/port, the TLS switch, the credential and the
operator's attachment caps), `AttachmentPayload`, and `EmailerClient`, which
builds an RFC 5322 multipart message and puts it on the wire through `lettre`.
It owns no templates, no routing and no persistence. Notification channels,
per-user routing rules and the retry split live in `crates/notify`
(`stump_notify::EmailChannel`); Kindle format and size policy lives in
`crates/kindle`; the `emailers` table, password encryption, send history and
enforcement of the caps live in `crates/graphql` and `stump_core`.

## Reference / upstream

| Reference | Relation |
| --- | --- |
| `lettre` 0.11.18, `default-features = false`, features `builder`, `hostname`, `smtp-transport`, `tracing`, `tokio1-rustls-tls` | Workspace pin at `Cargo.toml:62-68`; the only transport, and the source of the re-exported `EmailContentType`. |
| `lettre` issue [#359](https://github.com/lettre/lettre/issues/359) | Cited in-source as the reason the transport is picked by port, not by a single builder. `src/emailer.rs:162` |
| Upstream Stump `crates/email` | Same crate name and public API (`EmailerClient`, `EmailerClientConfig`, `AttachmentPayload`, `EmailError`); the `async-graphql` derive became opt-in here in `b068deb9`. |
| `crates/notify/README.md` (email channel row) and `crates/kindle/README.md` | The consumers that own the policy this crate refuses to hold; `stump_kindle` deliberately speaks this crate's `AttachmentPayload` instead of inventing its own (`crates/kindle/Cargo.toml:10-11`). |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Transport is chosen by port: `tls_enabled` + port 465 → `SmtpTransport::relay` (implicit TLS), `tls_enabled` + any other port → `starttls_relay` | 465 is implicit TLS and 587 is STARTTLS; a single builder would fail on one of the two, per `lettre#359`. | `src/emailer.rs:162-177` |
| `tls_enabled = false` falls back to `builder_dangerous` with `PLAIN`/`LOGIN` and a `warn!` log | Operators relaying through a local MTA need a plaintext path, but it must be visible in the logs rather than silent. | `src/emailer.rs:178-185` |
| `password` is `Option<String>` and a missing one is the dedicated `EmailError::NoPassword` at send time | The same config type is reused for emailer *updates*, where the stored password is not resent; failing at deserialize would make partial updates impossible. | `src/emailer.rs:24-27`, `src/error.rs:13-14`, `src/emailer.rs:152-160` |
| The credential is held in the config in plaintext and handed to `lettre::Credentials` per send; the crate does no crypto | Decryption with the server key belongs to the owner of the `emailers` row, so the key never enters this crate. | `src/emailer.rs:152-160`; caller: `core/src/notification.rs:262-275` |
| `max_attachment_size_bytes` and `max_num_attachments` are carried but never enforced here | They are operator configuration applied by the sending lane, which knows what it is batching: chunking at `crates/graphql/src/mutation/emailer/sender.rs:114`, per-book size check at `:245`, and Amazon's own 50 MB ceiling in `crates/kindle`. | `src/emailer.rs:34-37` |
| `build_message` is public and separate from `send_message` | A caller — or a test with a `lettre` stub transport — can hold the exact bytes a send would transmit without opening an SMTP connection. | `src/emailer.rs:196-247`; used that way in `crates/kindle/src/tests.rs:17` |
| Body is always `text/plain`, wrapped in `MultiPart::mixed` with the text part first and one part per attachment | No HTML template engine is in the tree; `send_attachments` supplies the standard Stump notice, `send_message` lets the caller pass its own prose. | `src/lib.rs:1-2`, `src/emailer.rs:231-243`, `src/emailer.rs:295-307` |
| Sends use `lettre`'s **blocking** `SmtpTransport` called from `async fn`, not `AsyncSmtpTransport` | The blocking builder is the one that carries both TLS modes and the credential mechanisms used above; callers invoke it from job/dispatch tasks rather than a request handler. | `src/emailer.rs:145-195`, `Cargo.toml:62-68` |
| `default = []`; the `graphql` feature only adds `derive(async_graphql::InputObject)` on `EmailerClientConfig` via `cfg_attr` | `b068deb9` made GraphQL derives opt-in so `stump_core` and `stump_notify` can link the crate with `default-features = false` and no `async-graphql` (`core/Cargo.toml:42`, `crates/notify/Cargo.toml:21`); only `crates/graphql` and `stump_core`'s `graphql` feature turn it on (`core/Cargo.toml:9-17`). | `Cargo.toml:6-11`, `src/emailer.rs:15-17` |
| Error taxonomy is `InvalidEmail` / `EmailBuildFailed` / `NoPassword` / `SendFailed`, with the `lettre` errors kept as `#[from]` sources | `stump_notify` splits retryable transport failures from permanent rejections on exactly this shape, so an SMTP error must stay distinguishable from an address or build error. | `src/error.rs:7-17`; parse errors mapped at `src/emailer.rs:211-230` |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | Crate doc, module wiring, re-exports (`EmailContentType` is `lettre`'s `ContentType`) |
| `src/emailer.rs` | `EmailerClientConfig`, `AttachmentPayload`, `EmailerClient`: `build_message`, `send_message`, `send_attachment(s)`, `send_test_email`, transport selection |
| `src/error.rs` | `EmailError`, `EmailResult` |
| `Cargo.toml` | `lettre`, `serde`, `thiserror`, `tracing`; optional `async-graphql` behind `graphql` |

## How to verify

```text
cargo test -p email                                                     # `no_run` doc examples; no unit tests yet
cargo check -p email --features graphql                                 # the InputObject derive still compiles
cargo test -p stump_kindle                                              # asserts on the MIME document build_message produces
cargo test -p graphql --no-default-features --features web --lib emailer # caps, chunking, send records
```

The crate itself has no unit tests (`src/emailer.rs:311`); the message it
builds and the caps its consumers apply are covered by the two suites above.

## Deep docs

- `docs/content/docs/developer/notifications.mdx` — the email channel, the
  server-wide emailer config and the shared delivery path.
- `docs/content/docs/developer/devices.mdx` (`## Send to Kindle`) — the other
  consumer of this transport.
