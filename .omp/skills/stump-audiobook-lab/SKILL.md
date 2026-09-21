---
name: stump-audiobook-lab
description: 'Build and validate private Stump audiobook/read-aloud fixtures: complete-chapter EPUB/M4B slices, OPF and EPUB 3 Media Overlays (SMIL), Storyteller Whisper/fuzzy reference runs, external CTC/Viterbi experiments, GPU/backend evidence, and deterministic timing comparisons. Use for audiobook lab work, ebook/audio pairing, SMIL fixture inspection, Storyteller experiments, forced alignment, ffmetadata chapter slicing, or read-aloud validation.'
---

# Stump Audiobook Lab

RFC 2119 applies: MUST, REQUIRED, SHOULD, RECOMMENDED, MAY, OPTIONAL;
`NEVER` and `AVOID` are the negative aliases.

<critical>
- Work only with lawfully accessed private fixtures; preserve originals.
- Keep mode outputs in separate immutable directories (`shared/`, `whisper/`, `ctc/`).
- Record source/output SHA-256, generator, model, options, and validation evidence.
- Storyteller and `@storyteller-platform/align` are external reference tools, never Stump dependencies.
- NEVER copy copyrighted text/audio, GPL/AGPL source, or non-redistributable models.
- Keep ports, current pins, image hashes, counts, and batch state in `.omp/PROJECT_STATE.md`.
</critical>

## Workflow

1. Locate the owner-only endpoint sheet locally; never print credentials.
2. Prepare a shared complete-chapter fixture with `scripts/read-aloud-lab.py slice`.
3. Run independent Storyteller Whisper/fuzzy and external CTC/Viterbi modes.
4. Inspect each output's OCF, OPF, overlays, targets, and cue invariants.
5. Compare only common text targets; preserve JSON evidence and provenance.
6. Report tool/model failures separately from Stump behavior or product readiness.

## Routing

- Audiobook assembly, chapters, reports, metadata: read `references/audiobook-pipeline.md`.
- EPUB/MOBI/KEPUB and calibre/boko paths: read `references/ebook-pipeline.md`.
- Storyteller, Whisper/fuzzy, CTC/Viterbi, GPU proof: read `references/storyteller-experiments.md`.
- Schemas, failure handling, and replay checks: read `references/validation-troubleshooting.md`.

## Deterministic helper

```text
python3 .omp/skills/stump-audiobook-lab/scripts/read-aloud-lab.py --help
slice   --epub SOURCE --audio SOURCE --out-dir MODE_DIR --fraction 0.10 \
        [--narrative-start ID --narrative-end ID]
inspect MODE_DIR/fixture.epub
compare whisper/fixture.epub ctc/fixture.epub
```

`slice` refuses existing outputs unless `--force`, uses whole audio chapters,
stream-copies through ffmpeg with explicit ffmetadata, truncates the OPF spine,
and writes a fixture manifest. Exact chapter labels may set the EPUB end;
otherwise both explicit narrative bounds are required. `inspect` and `compare`
emit versioned JSON and fail closed on ambiguous targets or invalid cues.
Do not run this script or the evals in a skill authoring pass unless the
coordinator explicitly requests it.

## Evidence rules

Treat private source bytes as inputs, not deliverables. Keep manifests and reports
free of excerpts; hashes and ordinals are sufficient. Use `.omp/PROJECT_STATE.md`
for volatile rollout facts and this skill for durable workflow and contracts.
