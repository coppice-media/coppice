# `stump_kindle`

## Purpose

The send-to-Kindle delivery lane: pick the format a Kindle can read, convert
it when the operator's `boko` can, refuse what Amazon would silently discard,
mail it through a caller-supplied transport, and record the delivery.

Two surfaces share every rule in here, which is why the rules are not in
either of them:

| Lane                                        | Entry point                  | Policy                                                                                                     |
| ------------------------------------------- | ---------------------------- | ---------------------------------------------------------------------------------------------------------- |
| E-mail (`sendToKindle`)                     | `KindleSend::send_to_kindle` | `FormatPolicy::PreferKindle` — Amazon converts an EPUB itself, so an unconverted EPUB is a successful send |
| USB (`POST /api/v2/media/{id}/kindle-file`) | `prepare_file`               | `FormatPolicy::RequireKindle` — a mounted Kindle has no gateway, so an unconvertible EPUB is refused       |

The crate owns no transport and no HTTP: `KindleMailer` is implemented by the
caller (`crates/graphql/src/mutation/kindle.rs` wires it to the primary emailer
and `stump_notify::EmailChannel`), which keeps one SMTP client, one
forbidden-recipient list and one send history for the whole server.

## Reference / upstream

| Contract                                                                                                                       | Where it must stay compatible                                                                                                                           |
| ------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Amazon Send to Kindle accepted types (DOC, DOCX, HTML, HTM, RTF, TXT, PDF, EPUB) and its 50 MB per-document attachment ceiling | <https://www.amazon.com/sendtokindle/email>; `KINDLE_FORMATS`, `AMAZON_MAX_ATTACHMENT_BYTES` in `src/file.rs`                                           |
| `boko convert` CLI shape and the EPUB → AZW3 pair                                                                              | `stump_tools::boko::{BokoConvert, KindleExport}`, `crates/tools/README.md`; boko is the operator's own GPL-3.0 install, shelled out to and never linked |
| Kindle MIME types (`application/vnd.amazon.mobi8-ebook` for AZW3)                                                              | `stump_media::ContentType::from_bytes_with_fallback`                                                                                                    |
| `devices.kindle_email`, `last_sync_summary` shape                                                                              | `stump_devices::{normalize_kindle_email, KindleSendSummary, DeviceService::record_delivery}`                                                            |
| `kindle_deliveries` columns                                                                                                    | `crates/migrations/src/m20260940_000000_add_kindle_deliveries.rs`, `models::entity::kindle_delivery`                                                    |

## Decisions

| Decision                                                                                                                                                                            | Why                                                                                                                                                                                                                                                                                  | Evidence                                                                                                                                      |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------- |
| The lane lives in its own crate, not in `stump_devices` or `crates/graphql`: the mutation and the USB route both call it                                                            | The USB download has to make the _same_ format decision as the mail, and the mutation's copy could not be reached from the server's router; `stump_devices` stays persistence-only rather than growing `stump_tools`/`email` dependencies for every consumer (`stump_core` included) | `src/lib.rs`; `crates/graphql/src/mutation/kindle.rs`; `apps/server/src/routers/api/v2/media.rs::get_media_kindle_file`                       |
| An EPUB that cannot be converted is a **successful** e-mail delivery with a `note`, and a **refused** USB download (`KindleError::ConversionRequired`)                              | Amazon's gateway converts EPUB; a Kindle mounted as a disk does not, so handing one over would put a file on the device that it cannot open while looking like success                                                                                                               | `src/file.rs::prepare_file`, `FormatPolicy`; tests `mails_the_epub_unchanged_when_amazon_accepts_it`, `the_usb_lane_requires_a_kindle_format` |
| AZW3 is preferred whenever `boko` is present, even though Amazon would convert                                                                                                      | AZW3 _is_ KF8: the delivery then depends on nothing external, and the identical file sideloads over USB                                                                                                                                                                              | `src/file.rs`, `src/convert.rs`; test `converts_to_azw3_when_amazon_would_have_to`                                                            |
| Amazon's 50 MB ceiling is checked before the emailer's own cap, and both before an SMTP connection is opened                                                                        | Amazon discards an over-size attachment without telling the sender, which would read as a successful send; the operator can raise their own cap but never Amazon's, so its message comes first                                                                                       | `src/send.rs::check_size`; tests `refuses_a_book_over_amazons_fifty_megabyte_limit`, `refuses_an_attachment_over_the_emailer_limit`           |
| A `kindle_deliveries` row is written for a **failed** send too, with the transport's message in `error`, while `emailer_send_records` and the device's `last_sync_*` stay untouched | The emailer history means "mail went out" and the device summary means "the device has the book"; neither may claim a delivery that was refused, and the question "did this book reach this Kindle" needs the failures to be visible                                                 | `src/send.rs::send_to_kindle`; test `records_the_failure_when_the_relay_refuses`                                                              |
| The row keeps `recipient`, `format`, `bytes`, `converted` and `note` as they were at send time rather than joining the device                                                       | A device's address can be re-pointed and its transform profile changed; a history that re-derived them would rewrite itself                                                                                                                                                          | `crates/models/src/entity/kindle_delivery.rs`; test `records_the_delivery_and_the_device_sync_summary`                                        |
| `KindleMailer::check_recipient` runs before the book is read or converted                                                                                                           | The server's forbidden-recipient list is policy, and paying for a conversion before checking it wastes a `boko` run per refusal                                                                                                                                                      | `src/send.rs`; test `refuses_a_forbidden_recipient_before_converting`                                                                         |
| A comic (CBZ/CBR) is refused locally instead of being mailed                                                                                                                        | Amazon discards what a Kindle cannot open, so the operator would see a successful send and no book                                                                                                                                                                                   | `src/file.rs::KINDLE_FORMATS`; test `refuses_a_format_amazon_does_not_accept`                                                                 |
| The subject is the book's name, except that a book named `Convert` is qualified                                                                                                     | `Convert` is Amazon's magic subject asking the service to reflow a PDF; Coppice never asks for that on the operator's behalf.                                                                                                                                                        | `src/send.rs::subject`; test `the_subject_never_reads_as_the_convert_keyword`                                                                 |

## Layout

| Path             | Contents                                                                                                                                                                                                          |
| ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/lib.rs`     | Crate doc (both lanes) and re-exports                                                                                                                                                                             |
| `src/file.rs`    | `KINDLE_FORMATS`, `AMAZON_MAX_ATTACHMENT_BYTES`, `FormatPolicy`, `KindleFile`, `prepare_file`: which formats are allowed, when the book is converted, what it is called and typed as                              |
| `src/convert.rs` | `KindleConverter` trait, `BokoConverter` (`boko-convert` on `spawn_blocking`), `Conversion`                                                                                                                       |
| `src/send.rs`    | `KindleMailer` trait, `KindleSend::send_to_kindle`, the size caps, the `kindle_deliveries` write, `deliveries()` for the history query, `subject()`                                                               |
| `src/error.rs`   | `KindleError`: one operator-facing sentence per refusal                                                                                                                                                           |
| `src/tests.rs`   | 13 tests; the mailer builds the production message (`EmailerClient::build_message`) and sends it through `lettre`'s `StubTransport`, so assertions are made on the MIME document that would have been transmitted |

Consumers: `crates/graphql` (the `sendToKindle` mutation and the
`kindleDeliveries` query) and `apps/server`
(`POST /api/v2/media/{id}/kindle-file`).

## How to verify

```bash
cargo test -p stump_kindle                                   # 13 tests, no boko needed
cargo test -p graphql --no-default-features --features web --lib kindle   # emailer history + forbidden recipient
cargo test -p stump_server --no-default-features --features minimal --test api_tests kindle  # the USB route
```

## Deep docs

- `docs/content/docs/developer/devices.mdx` (`## Send to Kindle`) — operator
  rules, both lanes, the GraphQL surface.
- `docs/content/docs/developer/platforms.mdx` (`## Amazon Kindle`) — what a
  Kindle can and cannot do.
- `docs/content/docs/developer/calibre-tooling.mdx` — `boko` vs calibre and why
  nothing GPL is linked.
