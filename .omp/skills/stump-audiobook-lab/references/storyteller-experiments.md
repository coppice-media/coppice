# Storyteller experiments

Storyteller is an independent external reference/oracle for EPUB 3 Media
Overlays, not a Stump runtime dependency. The align package is MIT but currently
depends on GPL-3.0 `ghost-story`, so keep the entire toolchain behind the
process/container boundary. Read `docs/content/docs/developer/read-aloud.mdx`
for the product boundary and use the private evidence under `input/audio/` for
current observations. Keep every mode reproducible from the same shared slice.

## Mode directories

Use this layout, with no cross-mode writes:

```text
<fixture>/shared/   # complete-chapter source pair and slice manifest
<fixture>/whisper/  # Storyteller web Whisper/fuzzy result + validation
<fixture>/ctc/     # external CTC emissions/Viterbi result + validation
```

Treat `shared/` as immutable after slicing. Copy or bind-mount it read-only into
both experiments. Each mode records source hashes, output hashes, generator
name/version, model and model-license evidence, options, backend, and validation
JSON. Never overwrite one mode with another.

## Storyteller web Whisper/fuzzy

1. Start the operator's isolated Storyteller web container outside Stump.
2. Import the shared EPUB and M4B; select the Whisper/fuzzy alignment path.
3. Set the documented language/model/turbo and worker options explicitly.
4. Export the read-aloud EPUB and the tool's report/validation JSON.
5. Run the local helper `inspect` against the exported EPUB.
6. Store the container/config/backend evidence beside the Whisper output.

The web path transcribes audio, fuzzy-matches the transcript to known EPUB text,
and emits SMIL. It is an oracle for envelope and alignment behavior, not proof
that Stump has an aligner. The validation record should expose narrative
coverage, resolved text targets, overlay count, and monotonic-cue status.

## External CTC/Viterbi path

Use the current external `align` CLI as a separate experiment. Check the
installed `align --help` before invocation; do not assume an old flag layout.
The durable sequence is:

```text
align@<version> emit --model <id> --device <backend> --dtype <precision> <audio-dir> <emissions-dir>
mv <emissions-dir>/*.json <same-stem>.emissions
align@<version> markup --granularity sentence <epub> <marked-up.epub>
align@<version> align --epub <marked-up.epub> --audiobook <audio-dir> \
  --ctc --emissions <emissions-dir> --output <read-aloud.epub>
```

`emit` may write binary emissions with a `.json` suffix. Rename that artifact
to `.emissions` before `align --ctc`: the CTC loader reads the `STEM` magic and
will reject a misleading JSON suffix. Capture the Viterbi/alignment report and
then run the local `inspect` helper. Record the exact argv, tool versions,
input/output digests, model id/license, and whether CTC or Whisper was used.

Known compatibility evidence belongs in the troubleshooting record, not in a
Stump dependency. The observed `align@0.1.58` plus `ghost-story@0.1.15` pairing
failed in a Storyteller CUDA 13.1 environment with missing
`libcublasLt.so.12`, and in a CUDA 12.9 environment with missing
`libcurand.so.10`. A lab image built from a
`nvidia/cuda:12.9.1-cudnn-runtime-ubuntu24.04` base with Node 24 and ffmpeg
has produced a real CUDA-backed Node process. Record these as external-tool
observations and re-check them when versions change.

## GPU/backend proof

A successful container start is not GPU proof. Keep three independent artifacts:

1. host probe (`nvidia-smi` or equivalent) with its digest;
2. container device/config evidence showing the injected device and CUDA env;
3. model-run stderr showing CUDA device discovery and backend selection.

Prefer the host's working CDI device injection when the Docker NVIDIA runtime
is not registered. Require a real model process to report CUDA use; an env var
alone is insufficient. Use synthetic tone or another non-copyrighted probe for
backend smoke evidence, then use the lawfully accessed private fixture for the
alignment run.

## Compare and interpret

```text
python3 .omp/skills/stump-audiobook-lab/scripts/read-aloud-lab.py compare \
  <fixture>/whisper/fixture.epub <fixture>/ctc/fixture.epub
```

The comparison matches exact internal text targets and reports matched count,
unmatched target counts, and absolute begin/end timing median, p95, and max in
milliseconds. It does not print excerpts or choose a product winner. A mode
with lower timing deltas can still have worse coverage; inspect both reports.
