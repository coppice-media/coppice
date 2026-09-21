# Server contract versus physical client

Hurl and curl-style probes exercise HTTP responses against a controlled fixture.
They do not exercise app parsers, cookie jars, local databases, download queues,
renderers, or device firmware. Keep these claims separate in every report.

## Komga clients

A Komga replay proves the adapter's route/DTO/auth contract. A Komelia or Mihon
run additionally checks login persistence, shared cookies, Readium rendering,
background downloads, tracker behavior, and UI refresh. If a device fails,
first match its request path/status/body shape to the Hurl evidence; a client
cache, stale cookie, or UI state can fail with a healthy server response.

Mihon's tracker uses the Komga profile but is a distinct lane: `replay-mihon`
checks its Basic/API-key request shapes and progress sequence. Do not infer
Komelia behavior from a Mihon pass.

## Kobo

The configured Kobo endpoint embeds a runtime API key under the Kobo path.
Server-side evidence should cover initialization, incremental library sync,
metadata/cover delivery, EPUB or optional KEPUB bytes, ranges, and ReadingState
GET/PUT when those claims are in scope. A stock Kobo run is required to claim
firmware parsing, shelf UI, sync scheduling, rendering, or real-device download.
KEPUB span locations are meaningful only for the exact transformed resource;
do not compare opaque span IDs across different files or converter settings.

## KOReader / KOSync

KOReader uses the `/koreader/{runtime-key}` surface and its own authentication
and document-hash conventions. A targeted server probe checks `users/auth` and
the progress write/read round trip. A physical KOReader run is required for
plugin configuration, document matching, and device-side progress behavior.
Do not call a KOReader probe a Liseur-native `/v1/*` replay.

## Liseur

Liseur has three separable surfaces: Komga catalog/client, OPDS/KOSync, and
native liseur-sync. `make replay-liseur-sync` proves the native bearer-token,
work/position/session/annotation/catalog contract. It does not prove the Liseur
Android UI, OPDS parser, or KOSync client. Use `make replay` only for its
Komga profile, and label OPDS/KOSync results independently.

## ABS

`make replay-abs` proves the Lissen-shaped REST contract. `make replay-abs-diff`
compares JSON key paths with the ABS reference; it does not prove ExoPlayer,
Android downloads, offline merge UI, or playback retry behavior. Those need a
physical-app run against an isolated fixture. Keep reference-container output
and credentials outside committed evidence.

## Device evidence record

Record client name/version, device or emulator class, fixture origin label,
route family exercised, and observed result. Never record credentials, cookies,
full tokens, private paths, raw media, or an endpoint-sheet line. A device
failure must include the matched HTTP trace or be marked client-only/unknown.
