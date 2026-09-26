# Read-aloud contracts

This reference is normative for implementation and debugging. `align`/`SyncMapV1`
validation, map import/persistence, strict source-EPUB SMIL import, optional
operator-configured worker runners, deterministic EPUB rendering, and
authenticated cache-only status/download routes are shipped. In-process model
inference, readiness checks, virtual composite delivery, and synchronized
physical-reader playback remain planned; never present server delivery as device
playback evidence.

## Contract matrix

| Surface                             | Contract                                                                                    | Current state                                                                                     |
| ----------------------------------- | ------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `Media.editions`                    | Confirmed editions only; request-scoped visibility                                          | Shipped                                                                                           |
| `Media.editionSuggestions`          | On-demand heuristics; cached verdicts; review-only                                          | Shipped                                                                                           |
| `Media.pairedPosition(fromMediaId)` | Read-only cross-edition projection                                                          | Shipped, approximate                                                                             |
| `chapterMap` and map mutations      | Ebook-spine ↔ audio-chapter entries                                                         | Shipped                                                                                           |
| `align` worker job                  | Typed input/result, explicit enqueue, optional Storyteller/native CTC worker runners         | Shipped; external worker execution only                                                           |
| `Media.syncMap` / `media_sync_maps` | Validated sentence/word timing map                                                          | Shipped                                                                                           |
| Native source-EPUB SMIL import      | Strict audio identity, XHTML ids, cue and duration validation                              | Shipped                                                                                           |
| `read-aloud.epub`                   | Deterministic materialized SMIL derivative                                                  | Shipped; cache-only routes; virtual composite planned                                             |
| Status/download and OPDS links      | Authenticated; current-user/pair scoped; no request-time ML                                 | Shipped                                                                                           |
| Metadata-backed request ledger      | Intent, destination/visibility, manager approval/rejection and notifications                | Shipped                                                                                           |
| In-process model runtime/readiness  | Local inference and quality checks                                                         | Planned                                                                                           |
| Physical synchronized playback      | Consume derivative in a reader                                                             | Planned; no device evidence                                                                       |
| Public alignment federation         | Cross-installation manifest sharing                                                        | Planned; legal/privacy gate                                                                       |

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

The map implementation and validator are in `crates/worker/src/alignment.rs`;
`media_sync_maps` persistence and `Media.syncMap` are shipped. Every backend
returns this compact timing map, not XHTML, an EPUB, a transcript, or audio
bytes. Optional operator-configured Storyteller and external native CTC
worker runners are shipped. The native runner requires a configured executable
and model for the typed CPU/fp32/CTC profile. Credentials and model paths remain
worker-local; server validation matches request-controlled identity/provenance
and probe-supplied track durations before acceptance. An in-process Rust model
runtime remains planned.

SMIL is the EPUB 3 **Media Overlay** timing document. The shipped read-aloud EPUB
contains sentence-id-bearing XHTML, one SMIL document per spine item,
`package.opf` `media-overlay` references and `media:duration`, and the original
single-file M4B. Source-EPUB SMIL import is shipped only when its embedded audio
bytes equal the sole paired external M4B. SMIL is not the full EPUB and is not
the internal SyncMap.

## Zero-GPU candidate collection and ranking

The shipped reusable timing inputs are a local validated `SyncMap` and strict
native EPUB SMIL. SMIL import is accepted only when the embedded audio bytes
equal the sole paired external M4B. Optional worker alignment is explicitly
requested and may use operator-configured Storyteller or external native CTC;
these runners produce a validated map rather than reusing a verified Storyteller
UUID/artifact. Verified UUID reuse and public exact-edition manifest federation
remain planned.

For every available candidate, validate content identity, duration, chapter
correspondence, monotonic non-overlapping cues, coverage, granularity, confidence,
and generator provenance. Exact content identity outranks a heuristic re-anchor;
sentence cues outrank chapter-only timing for sentence requests. Among valid
candidates, rank coverage/confidence and provenance deterministically, then
persist the winner and why it won. A candidate that fails a gate MUST fall
through; NEVER accept a weak linear fallback merely because an earlier source
existed.

If no reusable candidate passes, an explicit align enqueue may use known-text
CTC/Viterbi. Storyteller Whisper/fuzzy is an external API path, not Stump's
in-process runtime. Audiobook-only ASR remains tier 3 and out of scope.

## Storyteller comparison boundary

| External path                                           | What it demonstrates                                                                                                                     | Stump boundary                                                                                                |
| ------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| Storyteller web/server pipeline                         | Whisper transcription followed by fuzzy reconciliation to known EPUB text; API supports artifact reuse plus upload/process/poll/download | Optional operator-configured worker-local `align` backend; never an in-process server dependency              |
| external `@storyteller-platform/align` CLI with `--ctc` | Direct CTC emissions plus Viterbi forced alignment; current package transitively depends on GPL-3.0 `ghost-story`                        | External reference only; nothing bundled                                                                     |
| Stump `align`                                           | Typed worker input/result, explicit enqueue, optional Storyteller/native CTC runners and validated `SyncMapV1`                           | Shipped worker-side contract and runners; in-process runtime remains planned                                  |

NEVER collapse the Storyteller web Whisper/fuzzy route into the CLI
CTC/Viterbi route. They have different compute, error, provenance and licensing
implications even when they render the same EPUB/SMIL envelope.

### Worker-side Storyteller backend

An operator configures the Storyteller endpoint and credentials in worker-local
configuration. Stump's server and `worker_jobs` never receive them. The shipped
worker backend uploads the exact prepared EPUB and single-file M4B, triggers
processing, polls and downloads the result. It extracts timing, constructs
`SyncMapV1`, and returns only that map to Stump. Verified UUID/artifact reuse
remains planned.

The advertised `align` capability must contain the actual algorithm/model/device
profile and immutable configuration fingerprint. A generic installation flag
is insufficient for deduplication and provenance. The optional worker runners
are shipped, but do not imply in-process model execution or bundled dependencies.

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

- GET serves an already published deterministic derivative cache hit.
- On a cache miss, GET/status returns not-ready/unavailable; it MUST NOT render
  on demand, run Whisper/CTC/ASR, or create an `align` job.
- Alignment enqueue is explicit and deduplicates by exact input plus algorithm
  identity; processing and cache publication happen outside the GET path.
- Rendered XHTML sentence IDs are deterministic from the original EPUB; original
  EPUB and audio bytes remain untouched. A multi-track folder is unsupported and
  not-ready.
- The renderer produces the materialized deterministic EPUB and streams the
  original single-file M4B rather than buffering it. The virtual composite
  representation remains a planned experiment.
- Range, ETag and `Content-Length` describe one immutable completed cached ZIP,
  never an in-flight worker result or mutable source.

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
