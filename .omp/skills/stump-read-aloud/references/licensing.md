# Licensing, provenance, and sharing

Read this reference before adding an aligner, importing a timing artifact, or
proposing a shared manifest service. Coppice's tree is MIT; behavior and
published wire formats may be studied, but copyleft source MUST NEVER enter the
product.

The root repository is MIT; `apps/expo/LICENSE` remains GPL-3.0 and must be
treated as a separately licensed nested app.

## Allowed boundaries

- **Storyteller:** its server/apps and the cited `stalign` CLI are external MIT
  references. Use them as an oracle, fixture producer, or comparison target only;
  NEVER make Storyteller a Coppice runtime dependency or default backend.
- **Optional worker API:** an explicitly configured worker-local adapter MAY call
  an external Storyteller API after an ebook/audio edition pair is confirmed. It
  MUST return only the typed `SyncMapV1` contract; it MUST NOT make Storyteller a
  server dependency, search/catalog provider, metadata provider, or fulfillment
  connector.
- **Storyteller web path:** Whisper transcription plus fuzzy reconciliation. It
  is useful for validating preparation, GPU behavior, and the EPUB/SMIL envelope;
  it is not Coppice's shipped alignment engine.
- **Storyteller CLI path:** `pipeline --ctc` demonstrates direct CTC emissions and
  Viterbi forced alignment. It is a reference for a future known-text worker,
  not a bundled executable or implicit dependency.
- **Permissive model candidates:** `facebook/wav2vec2-base-960h` is Apache-2.0;
  NVIDIA NeMo CTC models are CC-BY-4.0 candidates. Record model identity and
  options in map provenance before reuse.

## NEVER ship these defaults

- `ghost-story` is GPL-3.0. It MAY remain an external CLI/container reference,
  but MUST NOT become a Coppice product dependency or default aligner.
- MMS forced-aligner models are CC-BY-NC 4.0. They MUST NOT be the product
  default or a dependency for a generally redistributable Coppice build.
- NEVER copy GPL/AGPL code from Audiobookshelf, Grimmory, BookOrbit, Calibre,
  DeDRM, or peer implementations. Their behavior/specifications may inform a
  contract; source does not.
- NEVER add a model, container, or binary whose license/weights permit only
  non-commercial use to a default distribution without a separate legal decision.

## Fixture and input rules

- Alignment fixtures MUST be lawfully accessed, private, and DRM-free.
- Preserve the original EPUB and audiobook bytes; sentence IDs and overlays are
  deterministic derivatives, NEVER in-place edits.
- Coppice MUST NOT circumvent DRM. A DRM-protected source is refused, not repaired.
- Workers receive authenticated source downloads and return timing/provenance,
  not copied transcript text, XHTML, cover, or audio content.

## Playback boundary

Synchronized Storyteller read-aloud EPUB playback is not shipped. Ordinary EPUB
playback and ordinary audiobook playback remain separate; an imported
`SyncMapV1` or future SMIL derivative does not imply a bundled Storyteller
player.

## Manifest and federation gate

An exact-edition manifest MAY contain timing, ordinals, sentence shape,
content/audio digests, coverage, and generator provenance. It MUST NOT contain
transcript text, sentence excerpts, EPUB content, audio, cover art, or a way to
reconstruct them. Local import/export comes before any network registry.

Public alignment-manifest federation is deferred until legal/privacy and abuse
review. NEVER implement, document as available, or exchange transcripts/content
between installations. Proof of possession and digest checks reduce risk but
CANNOT settle copyright, contract, privacy, compilation, or jurisdiction questions.
Use `docs/content/docs/developer/read-aloud.mdx` section 4c and the project's
`.omp/PROJECT_STATE.md` for the current decision; NEVER turn a design note into
an enabled public service.

The evidence labels for this work are exact: **Shipped**, **Contract-tested**,
**Device-tested**, **Source-only**, **Planned**, and **Blocked**. Optional
external references and local fixtures do not become app/device evidence.
