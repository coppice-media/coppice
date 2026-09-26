# Architecture and source map

Use this reference when changing or debugging the Stump read-aloud path. Paths
and symbols are intentionally stable anchors; NEVER copy volatile line numbers.

## Evidence and status

| Concern                          | Source-map pointer                                                             | Meaning                                                                                                                                                             |
| -------------------------------- | ------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Product status and tier boundary | `docs/content/docs/developer/read-aloud.mdx` sections 1–10                     | Tier 1, map import/enqueue, strict source-SMIL import, optional external worker runners, and deterministic cached EPUB delivery are shipped; in-process runtime, readiness, and virtual composite remain planned. |
| Current shipped state            | `.omp/PROJECT_STATE.md` read-aloud section and `docs/content/docs/developer/state.mdx` | Cross-check current implementation; do not infer physical-reader evidence.                                               |
| Roadmap status                   | `docs/content/docs/developer/roadmap.mdx` open item **Ebook ↔ audiobook sync** | Separate shipped delivery and worker backends from remaining roadmap work.                                                |
| Peer comparison                  | `docs/content/docs/developer/comparison.mdx` read-aloud row and lead list      | “Partial” is not evidence of device playback.                                                                              |

`.omp/PROJECT_STATE.md` is the volatile state lookup. Ports, commits, image
hashes, test counts, and batch state MUST remain there, NEVER in this skill.

## Tier model

1. **Tier 1 — chapter + percentage (free, shipped).** A confirmed EPUB/audio
   pair uses an editable `media_chapter_map`; conversion is approximate and
   happens at read time. It needs no ML, worker, or alignment blob.
2. **Tier 2 — known-text forced alignment (external worker runners shipped).**
   Sentence or word cues can be imported or produced by optional operator-
   configured worker-local Storyteller and external native CTC runners. Stump
   stores validated alignment cues, not a transcript. `SyncMapV1` import,
   persistence, explicit deduplicated enqueue, and strict source-EPUB SMIL
   import are shipped. The server finalizer validates request/pair identity,
   digests, durations, output bytes and prepared EPUB targets before publishing.
   In-process model inference remains planned.
3. **Tier 3 — audiobook-only ASR (expensive, out of scope).** A recording with
   no known ebook would need transcription and search. NEVER expand tier 2
   into a transcript product.

The generic worker queue and typed align contract are shipped. A capable
worker runner is registered for operator-configured Storyteller or external
native CTC executable/model; there is no in-process ML fallback. The request
ledger is metadata-backed and shipped. Readiness checks, virtual composite
delivery, and synchronized physical-reader playback remain planned.

## Edition identity and pairing

- The authoritative relationship is two user-scoped
  `liseur_sync_media_links` rows sharing one `work_id`; there is no pair table.
- Pairing state belongs in `pair_status`, not `resolution_status`.
  `resolution_status` records edition-digest resolution and is rewritten by
  the liseur lane.
- `PairStatus` is `confirmed`, `suggested`, or `rejected`. Only confirmed links
  are editions. Rejection remains a row because on-demand suggestions rerun.
- The weaker side governs the pair: `rejected` > `suggested` > `confirmed`.
  NEVER present a suggested link as an edition or convert through it.
- Pair only the audio/text boundary. Two EPUBs are format duplicates; two audio
  rows are separate recordings and have no tier-1 text/time conversion.
- Stronger evidence must not be downgraded by a later weak heuristic, and a
  confirmed link is NEVER re-homed by dismissing a suggestion.

### Pairing source map

- Persistence/state machine: `crates/models/src/domain/edition_pair.rs`:
  `PairStatus`, `PairEvidence`, `suggest_pair`, `confirm_pair`, `reject_pair`,
  `linked_media`, `pair_status`, `decided_media_ids`.
- Candidate computation and map lifecycle:
  `crates/library/src/editions.rs`: `pair_editions`, `candidates`,
  `build_chapter_map`, `confirm_edition_pair`, `replace_chapter_map`,
  `set_chapter_map_entry`, `clear_chapter_map_entry`.
- Candidate evidence helpers:
  `crates/library/src/editions.rs`: `normalize_title`, `authors_agree`,
  `identifiers_of`, `enabled_edition_lookups`, `identifier_candidates`,
  `title_candidates`.
- Schema/entity: `crates/migrations/src/m20260946_000000_add_edition_pairing.rs`,
  `crates/models/src/entity/liseur_sync_media_link.rs`, and
  `crates/models/src/entity/media_chapter_map.rs`.
- GraphQL read/write surface:
  `crates/graphql/src/object/media.rs` methods `editions`,
  `edition_suggestions`, `paired_position`; `crates/graphql/src/query/edition_pair.rs`
  method `chapter_map`; `crates/graphql/src/mutation/edition_pair.rs` methods
  `confirm_edition_pair`, `reject_edition_pair`, `set_chapter_map_entry`.

## Tier-1 map and position flow

`build_chapter_map` removes front/back matter first, then claims matches by
normalised title, ordinal, and positional fallback. Its confidence is evidence,
not proof: title is strongest, ordinal next, positional entries weakest. The
composite identity is `(ebook_media_id, audio_media_id, ebook_spine_index)`;
there is no `pair_id` to reference. Manual edits replace one entry and use full
confidence.

The ebook is canonical text; the audiobook is canonical time. The pure mapping
functions are `models::domain::reading_state::map_time_to_locator` and
`map_locator_to_time`; their inputs include `SpineItem`, `AudioChapterSpan`,
and `ChapterMapping`.

- Audio → ebook: fraction within the mapped audio chapter becomes a fraction
  within the mapped spine item's character weight and produces a Readium locator.
- Ebook → audio: fraction within the mapped spine item becomes milliseconds in
  the mapped chapter, measured from publication start.
- Unmapped matter returns `None`; NEVER guess chapter 1 or a copyright page.
- Only a confirmed pair is eligible. A missing head, invalid span, or no map is
  also `None`.

`crates/graphql/src/object/edition_pair.rs` contains the public
`MappedPosition` and `ChapterMapEntry` shapes. `crates/graphql/src/object/media.rs`
method `mapped_position` enforces the request user's visibility and confirmed
pair before calling the pure functions.

## Read-only MappedPosition

`MappedPosition` is a distinct domain type from `Projection`. It carries target
locator or `position_ms`, target whole-publication progression, map confidence,
and `approximate`. Tier-1 conversion sets `approximate: true`; narrators are not
metronomes, so it is resume-near-here guidance, not highlighting accuracy.

A mapped result MUST NOT be sent to `reading_state::resolve` or written to
`reading_heads`. Opening the target edition and actually reading/listening there
creates a normal real head through the ordinary mutation. Tier 2 may reuse the
shape with `approximate: false` only after validated cues exist.

Read the full rationale in `docs/content/docs/developer/unified-reading-state.mdx`
section **Mapped positions**.

## Worker boundary

The worker protocol is already a durable asynchronous boundary:

- Server transport glue: `apps/server/src/routers/api/v2/workers.rs` handlers
  `socket` and `upload_output`.
- Queue and transitions: `crates/worker/src/service.rs` `WorkerJobs`,
  `enqueue_and_wait`, `update_status`, `finish`, `outcome_of`.
- Wire frames: `crates/worker/src/protocol.rs` `WorkerFrame`, `ServerFrame`,
  `parse_worker_frame`, `encode`.
- Capability and kind contract: `crates/worker/src/kind.rs` constant `ALIGN`,
  `align_requires`, `satisfies`, `KindRegistry`, `LocalRunner`; typed map/input
  and validation: `crates/worker/src/alignment.rs`.
- Client boundary: `crates/worker/src/client.rs` traits `JobRunner`,
  `Assignment`, and functions `run`, `run_once`.
- Durable row: `crates/models/src/entity/worker_job.rs` and migration
  `m20260944_000000_add_worker_jobs`.

The explicit alignment action creates an asynchronous job using the shipped
`AlignInput` contract. The worker downloads the original EPUB/audio through
authenticated routes and returns a compact `SyncMapV1`, not copied transcript,
modified publication or audio. Optional operator-configured Storyteller and
external native CTC runners execute locally on the worker; credentials,
executable/model paths and source content never enter the server job payload.
`SyncMapV1::validate_for` owns request-identity, provenance, timing and
probed-duration acceptance; worker output remains a candidate until it passes.

## Candidate ladder and derivative flow

Before compute work, collect available zero-GPU candidates: a local validated
`SyncMap`, strict native EPUB SMIL, and timing produced by the shipped
Storyteller worker delegate. Exact-edition shared manifests are a future
candidate only; public federation is not shipped. Validate exact content
identity (or declared exact-edition identity), duration, monotonic cues,
coverage, granularity, and provenance. Rank valid candidates by
coverage/confidence and provenance strength; persist why the winner was chosen.
A failed quality gate MUST fall through to another available candidate rather
than becoming a low-quality linear-map success.

The shipped worker-side Storyteller backend uploads the exact prepared EPUB and
single-file M4B, processes/polls/downloads through the operator's worker-local
instance, then returns only the validated `SyncMapV1`. Verified UUID/artifact
reuse remains planned. The shipped native CTC backend invokes an
operator-configured executable and model on the worker for the typed CPU/fp32/CTC
profile. Nothing is bundled or run in-process. See `references/contracts.md` for
exact provenance, retention and cache rules.

## Read-aloud output

The shipped read-aloud EPUB is a deterministic derivative: sentence-id-injected
XHTML, one SMIL overlay per spine item, `package.opf` overlay metadata and
`media:duration`, plus the original single-file M4B stored verbatim. SMIL is the
EPUB 3 **timing overlay**; it is neither the full EPUB nor Stump's internal
`SyncMap`. Source-EPUB SMIL import is accepted only when its embedded audio
bytes equal the sole paired external M4B.

Authenticated status/download endpoints, GraphQL map/artifact fields and OPDS
acquisition links are cache-only. GET/status never enqueues or runs ML; the
deterministic renderer publishes a completed cache artifact. Alignment enqueue
is explicit and deduplicates by exact input plus algorithm identity. The
renderer streams the original M4B rather than buffering it.

The virtual composite EPUB representation is planned, not shipped.

### Existing media seams

- Audio facts: `crates/media/src/audio/probe.rs` (`probe`, `probe_file`,
  `probe_folder`) and `crates/models/src/services/audio.rs` (`AudioBook`,
  `track_at`, `chapter_at`).
- Readium text geometry: `crates/media/src/media/readium.rs`
  (`ReadiumManifestGenerator`) and its positions output.
- Existing EPUB API: `apps/server/src/routers/api/v2/epub.rs` handlers
  `get_epub_manifest`, `get_epub_positions`, `get_epub_resource`; separate from
  the read-aloud route.
- Read-aloud API: `apps/server/src/routers/api/v2/read_aloud.rs` routes
  `GET /media/{id}/read-aloud` and `GET /media/{id}/read-aloud.epub`; both are
  authenticated and cache-only. The EPUB route serves a published cache hit or
  reports not-ready.
- Cache discipline: `crates/media/src/transform/cache.rs`
  (`TransformCache::cache_file_name`, `audio_path_for`, `publish`, `hit`) and
  `apps/server/src/routers/audio_transform.rs` (`transcoded_path`). Alignment
  identity MUST add immutable content/algorithm inputs rather than rely on mtime.
