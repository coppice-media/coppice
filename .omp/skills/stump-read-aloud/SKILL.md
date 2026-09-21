---
name: stump-read-aloud
description: >-
  Use for Coppice ebook↔audiobook read-aloud/read-along architecture,
  implementation, debugging, tier/status lookup, edition pairing, MappedPosition,
  SyncMap versus SMIL, zero-GPU alignment candidates, worker jobs, immutable
  cache identity, cached on-demand EPUB overlays, or Storyteller comparisons.
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
2. Confirm the evidence in `docs/content/docs/developer/state.mdx` and
   `docs/content/docs/developer/read-aloud.mdx` sections 2–9.
3. NEVER claim that ML alignment ships today.

| Capability                                              | Current status                                                                          |
| ------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| Tier 1 pairing, chapter map, approximate conversion     | **Shipped**                                                                             |
| Generic worker queue/protocol and `transcode` substrate | **Shipped**; `align` is reserved                                                        |
| Tier 2 sentence/word alignment and `media_sync_maps`    | **Planned**                                                                             |
| Tier 3 audiobook-only ASR/transcript                    | **Blocked** (out of scope)                                                              |
| Read-aloud EPUB/SMIL renderer and readiness             | **Planned**                                                                             |
| `requests`/wanted ledger                                | **Planned**; no ledger/table/page exists                                                |
| Storyteller worker-side adapter                         | **Planned**; optional external API only                                                 |
| Synchronized Storyteller read-aloud EPUB playback       | **Planned**; not shipped; ordinary EPUB and ordinary audiobook playback remain separate |
| Public alignment-manifest federation                    | **Planned**; legal/privacy gate                                                         |
| Shelfmark request-service bridge                        | **Blocked**; Shelfmark is absent and a stable token-auth API is required                |

## Storyteller boundary

Storyteller may be referenced or called through an explicitly configured external worker-local API, but Coppice MUST NOT bundle Storyteller, GPL alignment code, models, or a default alignment backend. Storyteller is a worker-side alignment backend only; it is NEVER a search, catalog, metadata, or fulfillment provider.

## Non-negotiables

- Pair only confirmed ebook/audio editions; NEVER convert through suggestions.
- Treat `MappedPosition` as read-only; derived positions NEVER update a head.
- Keep the ebook as canonical text and audiobook as canonical time.
- Collect and validate zero-GPU timing artifacts before scheduling GPU work.
- GET handlers are cache-first and MUST NEVER start ML or enqueue alignment.
- An enqueue action deduplicates exact input plus algorithm identity.
- Distinguish internal `SyncMap` from EPUB timing-overlay SMIL.
- Preserve originals; exchange no transcript, sentence text, or audio content.

## Route by task

- Architecture/contracts: read `references/architecture.md` and
  `references/contracts.md`.
- Storyteller, GPL, model, or federation questions: read
  `references/licensing.md`.
- Source navigation: use the symbol map in `references/architecture.md`, not
  line numbers.
- Implementation/debugging: classify tier, inspect cache identity, then trace
  pair → map → enqueue → worker → validated map → deterministic derivative.
- Storyteller comparison: separate web Whisper/fuzzy from CLI CTC/Viterbi;
  both are external references, NEVER Coppice runtime dependencies. An optional
  worker-local Storyteller API adapter may return only the typed `SyncMapV1`
  contract after an edition pair is confirmed.

The full design and its shipped/planned boundary are in
`docs/content/docs/developer/read-aloud.mdx`; worker wire behavior is in
`docs/content/docs/developer/workers.mdx`. Evidence labels are exact and must
remain **Shipped**, **Contract-tested**, **Device-tested**, **Source-only**,
**Planned**, or **Blocked**.
