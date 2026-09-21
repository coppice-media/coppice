# Read-aloud contracts

This reference is normative for implementation and debugging. The shared
`align`/`SyncMapV1` Rust wire contract and validator are built; the runner,
persistence and renderer are planned. NEVER report those later layers as shipped.

## Contract matrix

| Surface                             | Contract                                                                                    | Current state                       |
| ----------------------------------- | ------------------------------------------------------------------------------------------- | ----------------------------------- |
| `Media.editions`                    | Confirmed editions only; request-scoped visibility                                          | Shipped                             |
| `Media.editionSuggestions`          | On-demand heuristics; cached verdicts; review-only                                          | Shipped                             |
| `Media.pairedPosition(fromMediaId)` | Read-only cross-edition projection                                                          | Shipped, approximate                |
| `chapterMap` and map mutations      | Ebook-spine ↔ audio-chapter entries                                                         | Shipped                             |
| `align` worker job                  | Typed input/result/capability contract; Storyteller adapter planned first; native CTC later | Contract built; execution unshipped |
| `Media.syncMap` / `media_sync_maps` | Sentence/word cue map                                                                       | Unshipped                           |
| `read-aloud.epub`                   | Deterministic SMIL derivative; materialized baseline, virtual composite experiment planned  | Unshipped                           |
| `requests` / federation             | Human request; public sharing is gated                                                      | Unshipped/legal review              |

## Edition-pair invariants

- Identity is user-scoped: two `liseur_sync_media_links` rows share one
  `work_id`; there is no standalone pair row.
- `pair_status` is independent of liseur `resolution_status`.
- `confirmed` is an asserted edition and the only state eligible for conversion.
  `suggested` is a guess; `rejected` is a sticky refusal.
- Pair status is the weaker link: rejected beats suggested beats confirmed.
- Pair only complementary media: one ebook and one audiobook. NEVER pair two
  EPUBs or two recordings for read-aloud conversion.
- Pair heuristics run on demand but persist every verdict, so a rejection does
  not reappear. Provider lookup failure removes evidence, not the book page.
- A confirmed link is NEVER re-homed by suggestion logic. A stronger evidence
  result may replace weaker evidence, NEVER the reverse.

## Tier-1 MappedPosition behavior

`map_time_to_locator` and `map_locator_to_time` operate on chapter spans, spine
character weights, and `media_chapter_map` entries. The formulas are:

```text
audio → ebook: f = (time_ms - chapter.start_ms) / chapter.duration_ms
               target locator = f within mapped spine character weight
ebook → audio: target_ms = chapter.start_ms + f × chapter.duration_ms
```

The target progression is whole-publication progression; it is not a page ordinal.
Use `locations.progression` inside a spine item and `locations.total_progression`
for the cumulative character-weight fallback.

A returned `MappedPosition` contains the target locator or `position_ms`, target
progression, map confidence, and `approximate`. Tier 1 sets `approximate: true`.
It MUST remain outside `reading_state::resolve` and MUST NOT be written to a
head. A real head changes only after the user reads/listens on the target edition.
Missing heads, invalid/non-positive spans, no confirmed pair, and unmapped front
or back matter return `None`; guessing chapter 1 is a contract violation.

## SyncMap versus SMIL

`SyncMapV1` is Stump's compact internal worker/result shape:

```json
{
	"schema": 1,
	"text_media_id": "…",
	"audio_media_id": "…",
	"provenance": {
		"text_digest": "<sha256>",
		"audio_manifest_digest": "<sha256>",
		"implementation": "…",
		"version": "…",
		"algorithm": "ctc",
		"model": "…",
		"model_revision": "…",
		"language": "en",
		"granularity": "sentence",
		"execution_provider": "cuda",
		"precision": "fp16",
		"options": {}
	},
	"cues": [
		{
			"spine_index": 4,
			"ordinal": 412,
			"element_id": "s0412",
			"track_index": 7,
			"begin_ms": 123400,
			"end_ms": 127800,
			"confidence": 0.94
		}
	]
}
```

The map implementation is in `crates/worker/src/alignment.rs`; future
`media_sync_maps`/`Media.syncMap` persistence is not. Every backend returns this
compact timing map, not XHTML, an EPUB, a transcript, or audio bytes. The first
planned executable backend delegates to an operator's Storyteller instance from
the worker; a native CTC/Viterbi backend follows. Server validation must match
request-controlled identity/provenance and probe-supplied track durations before
acceptance.

SMIL is the EPUB 3 **Media Overlay** timing document. A future read-aloud EPUB
contains sentence-id-bearing XHTML, one SMIL document per spine item,
`package.opf` `media-overlay` references and `media:duration`, and the original
single-file audio. SMIL is not the full EPUB and is not the internal SyncMap;
SMIL is a deterministic publication derivative rendered from a validated map.

## Zero-GPU candidate collection and ranking

Collect reusable artifacts before scheduling ML:

1. local validated `SyncMap`;
2. exact-edition shared `AlignmentManifestV1`;
3. native EPUB SMIL;
4. Storyteller timing artifact.

For every candidate, validate content identity, duration, chapter correspondence,
monotonic non-overlapping cues, coverage, granularity, confidence, and generator
provenance. Exact content identity outranks a heuristic re-anchor; sentence cues
outrank chapter-only timing for sentence requests. Among valid candidates, rank
coverage/confidence and provenance deterministically, then persist the winner and
why it won. A candidate that fails a gate MUST fall through; NEVER accept a weak
linear fallback merely because an earlier source existed.

Only when no candidate passes may the system enqueue known-text CTC/Viterbi.
Whisper/fuzzy is a final fallback for a Storyteller-compatible path, not the
preferred tier-2 implementation. Audiobook-only ASR remains tier 3 and out of
scope.

## Storyteller comparison boundary

| External path                                           | What it demonstrates                                                                                                                     | Stump boundary                                                                                                |
| ------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| Storyteller web/server pipeline                         | Whisper transcription followed by fuzzy reconciliation to known EPUB text; API supports artifact reuse plus upload/process/poll/download | Optional external `align` backend on a worker; never an in-process server dependency                          |
| external `@storyteller-platform/align` CLI with `--ctc` | Direct CTC emissions plus Viterbi forced alignment; current package transitively depends on GPL-3.0 `ghost-story`                        | External reference for the later native cheap known-text path; Stump owns its worker, map, gates and renderer |
| Stump `align`                                           | Typed worker input/result, capability requirement and validated `SyncMapV1`                                                              | Contract built; neither the Storyteller adapter nor native CTC runner/enqueue route exists today              |

NEVER collapse the Storyteller web Whisper/fuzzy route into the CLI
CTC/Viterbi route. They have different compute, error, provenance and licensing
implications even when they render the same EPUB/SMIL envelope.

### Worker-side Storyteller backend

An operator may run Storyteller on the worker device or on a host reachable
from it. Keep its URL and credentials in worker-local configuration; Stump's
server and `worker_jobs` never receive them. Prefer an exact existing
Storyteller UUID/source identity. If no artifact exists, the adapter may upload
the exact EPUB/audio, trigger processing, poll and download the read-aloud EPUB.
In both modes it extracts timing, constructs `SyncMapV1`, and returns only that
map to Stump.

The advertised `align` capability must contain the actual Storyteller
algorithm/model/device profile and an immutable configuration fingerprint. A
generic installation flag is insufficient for deduplication and provenance.
Title-only matching is prohibited. Before shipping, pin/probe the API contract
and define cancellation, failure, and remote-item retention semantics.

## Async enqueue and immutable cache identity

Alignment is asynchronous. An explicit enqueue action creates or reuses a job
whose identity is the canonical exact input plus canonical algorithm identity.
Equivalent concurrent requests MUST deduplicate rather than launch two models.
The conceptual key includes:

```text
input:    exact_text_digest, robust_audio_digest, duration/chapter identity,
          ebook/audio edition identity, text-normalization version,
algorithm: generator, implementation version, model id, options, granularity
```

Use immutable content/algorithm digests, canonicalized options, and persisted
provenance. A media path, mtime, worker id, mutable pair status, or output name
alone is not an identity. Any source-content, normalization, model, option, or
renderer change yields a new key. Reuse requires re-validating the stored map
against the current exact inputs.

Keep the alignment-map cache identity separate from the EPUB derivative identity.
The derivative key additionally includes the accepted map payload digest and
renderer/schema version. Publish both atomically; a reader sees a complete old
or complete new artifact, NEVER a partial ZIP. Existing
`stump_media::transform::cache::TransformCache` is a cache-discipline precedent,
not permission to key alignment solely by source mtime.

## GET and cached on-demand EPUB semantics

- GET may serve a completed derivative cache hit.
- GET may build/cache a deterministic derivative from an already validated
  `SyncMap`; it MUST NOT run Whisper/CTC/ASR or create an `align` job.
- If no accepted map exists, GET reports unavailable/readiness/needs-worker
  state; it MUST NOT silently enqueue ML.
- The enqueue action is the only ML trigger and returns the deduplicated job/map
  identity for polling or subscription.
- Rendered XHTML sentence IDs are deterministic from the original EPUB; original
  EPUB and audio bytes remain untouched. A multi-track folder must be assembled
  into one audio edition before rendering.
- The initial derivative stores original `audio/mp4` bytes and deterministic
  `clipBegin` / `clipEnd` references. A later virtual composite representation
  MAY map the logical stored audio entry directly to ranges in the source M4B,
  but only after the experiment below passes.
- Range, ETag and `Content-Length` describe one immutable complete logical ZIP,
  never an in-flight worker result or a mutable source.

### Future virtual composite EPUB experiment

Do not use ordinary non-seekable ZIP streaming. Precompute an immutable ZIP
layout containing:

- generated local headers and deterministic XHTML/SMIL/OPF resources;
- one stored audio entry whose data segment maps to the original M4B;
- a generated central directory and EOCD/ZIP64 records;
- every offset, size, CRC-32, representation ETag and total length.

Serve arbitrary single byte ranges by slicing that logical segment table.
Invalidate the plan if the source size or digest changes. The experiment passes
only when its full byte stream matches a materialized reference, `unzip -t` and
`epub-check` pass, boundary and M4B ranges reconstruct correctly, and Thorium,
Readest plus one mobile reader can open/seek/resume it. Until then, the
materialized deterministic ZIP remains the compatibility-safe output.

Read `docs/content/docs/developer/read-aloud.mdx` sections 4a, 4c, 5, and 6 for
the evidence and planned envelope; `docs/content/docs/developer/workers.mdx`
for the shipped queue and wire protocol.
