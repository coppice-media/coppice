# Validation and troubleshooting

The local helper is the deterministic gate for fixture artifacts. It uses only
Python's standard library; ffprobe/ffmpeg are explicit external executables for
media probing and stream-copy slicing. Run it from the repository root.

## JSON contracts

`slice` writes `stump-read-aloud-fixture/v1` with `source`, `fixture`,
`selection`, `provenance`, and SHA-256 values. `inspect` emits
`stump-read-aloud-inspection/v1` with `valid`, `errors`, `zip`, `opf`,
`overlays`, and `targets`. `compare` emits
`stump-read-aloud-comparison/v1` with common/matched counts, unmatched target
counts, and `timing_delta_ms.begin|end.{median,p95,max}`. Error exits emit
`stump-read-aloud-lab-error/v1`; do not treat a pretty JSON error as success.

Stable output requires sorted JSON keys, fixed integer milliseconds, no timestamps
or random ids, and no source excerpts. A manifest may name private paths and
hashes; it must not contain embedded text, audio, covers, credentials, or tokens.

## Inspect gate

`inspect` fails closed unless all of these hold:

- the archive is readable and `mimetype` is the first, stored exact entry;
- `META-INF/container.xml` resolves exactly one OPF rootfile;
- manifest ids/hrefs and spine idrefs are unique and resolvable;
- spine media-overlay attributes point at existing SMIL manifest items;
- at least one SMIL overlay exists and every `<par>` has one text/audio child;
- every text target resolves to one archive XHTML element fragment;
- every audio target resolves to an archive-local member;
- `clipBegin`/`clipEnd` are finite, positive, duration-bounded milliseconds;
- cues are monotonic and non-overlapping in each SMIL/audio-target sequence;
- OPF exposes one finite unrefined `media:duration` value.

`compare` runs the same gate on both inputs and matches XHTML path plus a hash of
normalized target text, never source excerpts. Repeated semantic targets must
have equal cardinality and are paired by document order. Zero common targets is
a valid comparison with null timing metrics; an invalid EPUB is not.

## Replay sequence

1. Read the owner-only endpoint sheet at its local fixture path; never copy it.
2. Verify source hashes and the shared manifest before running a mode.
3. Validate each mode EPUB with `inspect`.
4. Compare modes only after both inspections pass.
5. Preserve stdout JSON, stderr tails, command argv, and artifact hashes.
6. Put current ports, pins, image digests, counts, and batch status in
   `.omp/PROJECT_STATE.md`, not in this skill.

For Stump-side checks, use the existing `epub-check`, `epub-polish`,
`audio-report`, `audio-assemble`, `audio-chapters`, and `meta-edit` contracts;
see `references/audiobook-pipeline.md` and `references/ebook-pipeline.md`.

## Failure recovery

| Symptom                                             | Recovery                                                                                                                                                                                                   |
| --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Existing output refused                             | Choose a new mode directory or pass `--force` only after checking the three named outputs.                                                                                                                 |
| Source path equals output                           | Stop; restore the immutable source and choose a derived path.                                                                                                                                              |
| No/overlapping audio chapters                       | Run `audio-report`; repair/assemble with Stump tooling, then re-hash. Never guess boundaries.                                                                                                              |
| Ambiguous bodymatter or chapter/spine match         | Pass explicit `--narrative-start` and inclusive `--narrative-end` idrefs/hrefs; never assume one physical audio part equals one EPUB chapter.                                                              |
| ffprobe/ffmpeg missing or non-zero                  | Save the exact diagnostic and install/select the operator tool; do not create a partial fixture.                                                                                                           |
| CTC loader says unexpected `S` token                | The binary `STEM` emission was left with `.json`; rename it to `.emissions` before `align --ctc`.                                                                                                          |
| CTC matches zero English chapters                   | Check model-aware known-text normalization; the tested Wav2Vec vocabulary requires upper-case symbols. Version and cache this normalization.                                                               |
| Aggregate `media:duration` ends before the last cue | Reject the raw EPUB. Compare probed audio duration with the sum of refined overlay durations; a Stump renderer must compute the aggregate deterministically.                                               |
| CUDA library-major error                            | Record the observed missing `libcublasLt.so.12` (CUDA 13.1) or `libcurand.so.10` (CUDA 12.9) as external incompatibility; align the CUDA runtime, `ghost-story`, and Node userspace, then rerun GPU proof. |
| CUDA env exists but logs show CPU                   | Reject GPU proof; use CDI/device injection and require backend/device lines from a real model process.                                                                                                     |
| Missing SMIL target or non-monotonic cue            | Reject the mode output; inspect OPF href bases and regenerate from the same shared slice.                                                                                                                  |
| DRM or copyrighted fixture cannot be shared         | Keep it local, remove excerpts from evidence, and do not circumvent protection.                                                                                                                            |

Do not weaken validation to make an external result pass. A failed reference run
is useful evidence when its inputs, versions, command, stderr, and hashes are
recorded without credentials or source expression.
