---
name: stump-read-aloud
description: >-
  Use for Coppice ebook↔audiobook read-aloud/read-along architecture,
  implementation, debugging, tier/status lookup, edition pairing, MappedPosition,
  SyncMap versus SMIL, zero-GPU alignment candidates, worker jobs, immutable
  cache-only EPUB Media Overlays, or Storyteller comparisons.
  Trigger on requests mentioning Media.pairedPosition, media_chapter_map,
  media_sync_maps, read-aloud EPUB, forced alignment, Whisper/fuzzy, CTC/Viterbi,
  Storyteller, or alignment manifests in the Coppice repository. NEVER use for a
  generic EPUB/audio problem with no Coppice contract.
---

# Coppice Read-Aloud

Apply this skill to Coppice's ebook↔audiobook position and timing contracts.
RFC 2119 applies: MUST, REQUIRED, SHOULD, MAY; NEVER and AVOID are prohibitions.

## Status lookup (before making claims)

1. Read `.omp/PROJECT_STATE.md` for volatile rollout state.
2. Confirm current contracts in `docs/content/docs/developer/read-aloud.mdx`
   and `.omp/PROJECT_STATE.md` before making status claims.
3. Distinguish shipped external worker execution and deterministic delivery from
   planned in-process model runtime, readiness checks, virtual composite, and
   synchronized physical playback. Shipped source/contracts are not device proof.

| Capability                                              | Current status                                                                                   |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| Tier 1 pairing, chapter map, approximate conversion     | **Shipped**                                                                                      |
| Persistent `SyncMapV1` import and alignment enqueue      | **Shipped**                                                                                      |
| Source-EPUB SMIL import                                 | **Shipped**, only with strict paired-audio byte identity                                         |
| Storyteller and external native CTC worker runners      | **Shipped**, optional and operator-configured; worker-local                                    |
| Deterministic read-aloud EPUB renderer and cache routes | **Shipped**, authenticated/cache-only; no request-time ML                                       |
| Metadata-backed request ledger                         | **Shipped**; intent, approval/rejection, visibility, notifications                              |
| In-process model runtime                               | **Planned**                                                                                      |
| Readiness quality checks and virtual composite         | **Planned**                                                                                      |
| Synchronized physical-reader playback/device evidence  | **Planned**; no physical playback evidence                                                       |
| Tier 3 audiobook-only ASR/transcript                   | **Blocked** (out of scope)                                                                       |
| Public alignment-manifest federation                  | **Planned**; legal/privacy gate                                                                  |
| Shelfmark request-service bridge                       | **Blocked**; Shelfmark is absent and a stable token-auth API is required                          |

The generic worker protocol, map persistence/import, strict native SMIL
acceptance, optional worker runners, deterministic ZIP renderer, authenticated
status/download endpoints, and cache-only OPDS links are shipped. Do not infer
in-process inference or synchronized playback from those contracts.

## Storyteller boundary

Storyteller may be called through an explicitly configured worker-local API, and
the external native CTC executable/model may be configured on a worker. Coppice
MUST NOT bundle either runner, alignment code, or model weights. Credentials and
model paths remain worker-local and never enter server jobs. Storyteller is
NEVER a search, catalog, metadata, or fulfillment provider.

## Non-negotiables

- Pair only confirmed ebook/audio editions; NEVER convert through suggestions.
- Treat `MappedPosition` as read-only; derived positions NEVER update a head.
- Keep the ebook as canonical text and audiobook as canonical time.
- Collect and validate zero-GPU timing artifacts before scheduling GPU work.
- GET/status handlers are cache-only and MUST NEVER start ML or enqueue alignment.
- An enqueue action deduplicates exact input plus algorithm identity.
- Distinguish internal `SyncMap` from EPUB timing-overlay SMIL.
- Accept imported SMIL only when embedded audio bytes equal the sole paired M4B.
- Preserve originals; exchange no transcript, sentence text, or audio content.

## Route by task

- Architecture/contracts: read `references/architecture.md` and
  `references/contracts.md`.
- Storyteller, GPL, model, or federation questions: read
  `references/licensing.md`.
- Source navigation: use the symbol map in `references/architecture.md`, not
  line numbers.
- Implementation/debugging: classify tier, inspect cache identity, then trace
  pair → map/import → enqueue/worker → validated map → deterministic derivative.
- Storyteller comparison: separate web Whisper/fuzzy from CLI CTC/Viterbi;
  external worker runners are optional, operator-configured, and return only
  typed `SyncMapV1`.

The full design and its shipped/planned boundary are in
`docs/content/docs/developer/read-aloud.mdx`; worker wire behavior is in
`docs/content/docs/developer/workers.mdx`. Evidence labels are exact and must
remain **Shipped**, **Contract-tested**, **Device-tested**, **Source-only**,
**Planned**, or **Blocked**.

