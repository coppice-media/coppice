# stump_crosspoint

## Purpose

`stump_crosspoint` is Coppice's clean-room CrossPoint protocol crate. It owns
transport-neutral rich-sync DTOs and validation limits, target/delivery storage
vocabulary, and the LAN WebSocket transfer client. The server owns API-key
authentication, registry-device binding, user/media visibility, persistence,
and queue scheduling. Stock KOReader/KOSync remains in `stump_koreader`.

## Reference / upstream

| Reference           | Pin                                        | Use                                                                 |
| ------------------- | ------------------------------------------ | ------------------------------------------------------------------- |
| CrossPoint firmware | `c33a8b0e883816c22cb1304e51464a4a04fd16dd` | `/api/status`, UDP `hello`, WebSocket `START`/`READY`/binary/`DONE` |
| crosspoint-sync     | `d4cd96649dfcdded47ef40e939f994ebb923dbf6` | rich `/api/v1` JSON and delta semantics                             |
| Calibre plugin      | `460c53088860a118c4f408cb4bc37d3e3dea74d6` | client defaults, 2048-byte transfer chunks, retries                 |

All three are behavior evidence only. Their implementations are not vendored
or executed; this MIT tree contains a clean-room Rust implementation.

## Decisions

| Decision                                                                                                                              | Why                                                                                                                                                                              | Evidence                                                                                    |
| ------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| Rich sync is mounted only at `/koreader/{api_key}/api/v1`                                                                             | Root `/api/v1` conflicts with existing server ownership and would make a keyless surface; the path key is already the KOReader compatibility contract                            | `src/sync.rs`; `apps/server/src/routers/crosspoint.rs`; crosspoint-sync `docs/API.md`       |
| The body `device_id` is advisory and must equal the registry device bound to the path credential                                      | A client-supplied id cannot impersonate another paired device; session/JWT requests have no device binding                                                                       | `stump_auth::AuthContext::device_id`; server `api_key_middleware`                           |
| Rich deltas use server-clock `updated_at`, oldest-first cursors, permanent tombstones, and per-device rows                            | Device RTCs drift and a set-union/LWW merge must converge after an offline restart                                                                                               | crosspoint-sync `src/routes/v1/{bookmarks,clippings}.ts`                                    |
| Stats snapshots replace one device row and aggregate only on reads                                                                    | Pushing another device's counters into a local snapshot double counts after retries                                                                                              | crosspoint-sync `src/routes/v1/stats.ts`                                                    |
| Transfer accepts only verified private IPv4 literals on fixed ports 80/81, bounds size, rejects overwrite, and never deletes remotely | CrossPoint LAN I/O is unauthenticated; hostnames and public/loopback/link-local/multicast destinations permit SSRF or credential leakage, while firmware has no checksum/receipt | firmware `CrossPointWebServer.cpp`; plugin `ws_client.py`; `storage.rs`                     |
| No remote delete or overwrite is implemented                                                                                          | The firmware's delete endpoint is outside the server delivery contract and overwrite behavior differs between docs and source                                                    | firmware source/docs; delivery service                                                      |
| The pinned firmware currently speaks stock KOSync only; it has no rich-sync custom-URL setting                                        | Rich records are ready for a future firmware/companion client, but current physical devices cannot be told to use the keyed rich base                                            | firmware `KOReaderSyncClient.cpp`/`KOReaderSyncActivity.cpp`; crosspoint-sync `docs/API.md` |

## Layout

| Path              | Responsibility                                                         |
| ----------------- | ---------------------------------------------------------------------- |
| `src/lib.rs`      | Public crate documentation and re-exports                              |
| `src/sync.rs`     | Progress, position, bookmark/clipping, stats, metadata DTOs and limits |
| `src/storage.rs`  | Target, queue, attempt status and fixed-port storage vocabulary        |
| `src/transfer.rs` | LAN discovery/status/WebSocket upload client                           |
| `Cargo.toml`      | Workspace crate and transfer dependencies                              |

## How to verify

```text
cargo test -p stump_crosspoint
cargo check -p stump_server --no-default-features --features minimal
```

The project coordinator owns the one final gate and fixture replay; delegated
workers do not run builds, formatters, linters, or broad tests while this
feature is in flight.

## Deep docs

- `docs/content/docs/developer/crosspoint-sync.mdx` — server routes and auth
- `docs/content/docs/developer/crosspoint-transfer.mdx` — target verification and queue safety
- `crates/devices/README.md` — registry kinds, credential binding, and endpoints
- `crates/migrations/README.md` — append-only storage policy
