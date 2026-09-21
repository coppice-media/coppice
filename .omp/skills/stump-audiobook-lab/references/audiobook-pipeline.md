# Audiobook pipeline

Read the source behavior in `docs/content/docs/guides/fundamentals/audiobooks.mdx`
and the implementations in `crates/tools/src/audio_assemble.rs`,
`audio_chapters.rs`, `audio_report.rs`, and `meta_edit.rs` before changing a
fixture workflow. This reference records how the lab exercises those existing
contracts; it does not add application code.

## Prepare and preserve

- Accept only lawfully accessed, DRM-free private EPUB/M4B fixtures.
- Hash source bytes before every derived run.
- Never edit, rename, delete, or replace the source EPUB or M4B.
- Put `shared`, `whisper`, and `ctc` outputs in different directories.
- Keep source and output hashes in the fixture manifest and reports.
- Keep copyrighted excerpts out of manifests, logs, and documentation.

Use the shared directory for the complete-chapter slice. Derive every alignment
mode from that exact shared pair; do not slice each mode independently.

## Stump audio tools

| Tool             | Use in the lab                                               | Important contract                                                                    |
| ---------------- | ------------------------------------------------------------ | ------------------------------------------------------------------------------------- |
| `audio-report`   | Record duration, codec, chapter source, cover, and warnings. | Read-only; one finding per publication.                                               |
| `audio-assemble` | Make a canonical single M4B before read-aloud work.          | Creates a new output; remuxes AAC when possible and plans ffmpeg transcode otherwise. |
| `audio-chapters` | Embed derived chapter marks into M4B/M4A/MP3.                | Protects publisher-authored marks unless forced; refuses unsupported containers.      |
| `meta-edit`      | Fill title/author/narrator/series/year/cover metadata.       | Stages changes atomically; does not alter samples or chapter structures.              |

A folder audiobook is not a read-aloud input until it is assembled into one
single-file publication. A complete-chapter slice is preferable to a time cut:
it keeps chapter semantics, avoids partial cues, and makes comparisons
repeatable.

## Complete-chapter slice

Run the helper from the repository root. The helper calls `ffprobe` to discover
embedded chapter boundaries, chooses a contiguous prefix whose end is nearest
to the target fraction, and calls ffmpeg with `-c copy` plus an explicit
`FFMETADATA1` input. It rewrites only the derived EPUB's OPF spine and writes a
versioned JSON manifest.

```text
python3 .omp/skills/stump-audiobook-lab/scripts/read-aloud-lab.py slice \
  --epub <source.epub> --audio <source.m4b> \
  --out-dir input/audio/experiments/<name>/shared \
  --fraction 0.10 --mode shared
```

Use `--narrative-start <spine-idref-or-href>` when the EPUB has no unique
`bodymatter` landmark. When exact numbered audio chapters do not match EPUB
spine labels, also pass an inclusive `--narrative-end`; the helper never assumes
one physical audio part equals one EPUB chapter. An ambiguous landmark,
duplicate manifest id, duplicate spine idref, partial chapter-number match, or
missing explicit range is an error.

The manifest must retain:

- source and fixture paths plus SHA-256 digests;
- selected chapter indexes and exact start/end milliseconds;
- target and achieved fraction;
- narrative-start detection method and retained spine idrefs;
- ffprobe/ffmpeg executable names and ffmetadata digest;
- explicit `stream_copy: true` provenance.

The helper refuses to overwrite named outputs by default. `--force` is an
operator decision for those outputs only; it never authorizes source mutation.

## Evidence handoff

After slicing, run Stump's existing report/check tools on the source and shared
fixture as appropriate. Preserve JSON reports next to the mode that produced
them. The lab records facts and provenance, not a claim that an external
alignment result is a Stump implementation. For volatile fixture locations,
ports, image pins, and current measurements, use `.omp/PROJECT_STATE.md` and
repository `input/audio/` evidence rather than editing this reference.
