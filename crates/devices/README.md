# `stump_devices`

## Purpose

Unified device registry: a *device* is a registered client (Kobo, KOReader,
Mihon, Komelia, Liseur, an OPDS reader, a script, a browser) owned by a user.
The crate mints the credential a device authenticates with (a prefixed API key
or a liseur-sync token), tells the client which endpoints to configure, and
records last-seen / last-sync state every time a credential authenticates.

It is persistence-only: SeaORM entities from `models`, no HTTP. The server
calls `DeviceService::touch` from its protocol auth paths and forwards the
returned `DeviceSeen` through the core event channel. Pairing (`device_pairing`
rows, QR flow) lives in `apps/server/src/routers/api/v2/device_pairing.rs` and
`crates/graphql`, not here.

## Reference / upstream

| Contract | Where it must stay compatible |
| --- | --- |
| Prefixed API keys `stump_<short>_<long>` | `crates/models/src/shared/api_key.rs`, `apps/server/src/middleware/auth.rs::validate_api_key` |
| liseur-sync bearer secrets `liseur-<uuid>`, SHA-256 hashed | `apps/server/src/routers/liseur_sync/storage.rs::authenticate` |
| Endpoint shapes per kind | `docs/content/docs/developer/devices.mdx` |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| `touch` coalesces plain sightings in memory: one `last_seen_at` write per credential per `TOUCH_INTERVAL` (60 s); a coalesced call reads nothing | Page streams and cover bursts must not hammer SQLite; the map is shared by every clone of the service, so the debounce is process-wide | `src/service.rs::touch`, `seen_within_interval`; test `touch_coalesces_sightings_per_credential_for_an_interval` |
| A sync summary always writes (`last_sync_at` + `last_sync_summary`) | Sync events are rare and are what the UI shows | `src/service.rs::touch`; test `touch_records_sighting_and_sync_summary` |
| `DeviceSeen.first_seen` is `true` when the row had no `last_seen_at` before the write | Notification rules key on "device first seen" without a second lookup | `src/event.rs`, `src/service.rs::touch` |
| `device_for_credential` is the single credential → live device lookup | Adapters (Kobo transform profile) need per-device state; one lookup convention, revoked devices excluded | `src/service.rs::device_for_credential`; test `device_for_credential_resolves_live_devices_only` |
| Protocol kinds get narrowed API keys (`api_key_permissions_for`); creation requires the owner to hold those permissions plus `ACCESS_API_KEYS` | A key must never grant what the user does not have | `src/credential.rs`; test `create_device_requires_the_kind_permissions` |
| Revocation deletes credential rows and keeps the device row | History stays visible; `touch` can no longer resolve the credential | `src/service.rs::revoke`, `discard_credentials`; test `revoke_deletes_credentials_and_blocks_rotation` |
| Liseur device tokens are bound to the device id (`liseur_sync_tokens.device_id`) | The liseur ops push only knows the token's device id and must reach the registry device | `src/credential.rs::liseur::mint`; `apps/server/src/routers/liseur_sync/storage.rs::record_device_sync` |
| Default names are `"<Username>'s <Kind>"`, numbered on collision; names are unique per user | Rename also renames the credential row so API key lists stay readable | `src/service.rs::create_device`, `rename`; test `default_names_are_numbered_on_collision` |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | Re-exports; `liseur_token` helpers shared with the server storage |
| `src/service.rs` | `DeviceService`: list/get/create/rename/rotate/revoke/transform profile, `device_for_credential`, `touch` with the in-memory debounce, endpoints |
| `src/credential.rs` | `CredentialRef`, kind → protocol / credential kind / permissions maps, API key and liseur token minting |
| `src/endpoint.rs` | `Endpoint::for_kind`: what to configure on each device kind |
| `src/event.rs` | `DeviceSeen` |
| `src/error.rs` | `DeviceError` |
| `src/tests.rs` | Service tests against an in-memory SQLite schema |

## How to verify

```sh
cargo test -p stump_devices
cargo test -p stump_server --no-default-features --features headless,liseur-sync --test api_tests -- kobo::devices komga device_touch
```

## Deep docs

- `docs/content/docs/developer/devices.mdx` — kinds, endpoints, sightings and every `touch` call site, GraphQL surface, reading statistics.
