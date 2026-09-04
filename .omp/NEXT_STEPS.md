# Stump Next Steps

This is the on-demand roadmap. Keep implementation status conservative: a design
page or a replay fixture is evidence for a contract, not proof that the feature
is shipped.

## Open implementation tracks

1. **Unified reading state.** Implement the canonical per-user position/annotation
   head and provider projections using the lossless rules in
   `docs/content/docs/developer/unified-reading-state.mdx`. Preserve provider
   identity, timestamps, precision, sequences, revisions, and tombstones.
2. **Kobo device import and companion workflows.** Design the read-only
   `KoboReader.sqlite` importer, then evaluate NickelStump/NickelHardcover-style
   extraction and Syncthing handoff. Treat
   `docs/content/docs/developer/kobo-device-database.mdx` and
   `docs/content/docs/developer/kobo-sync-capabilities.mdx` as evidence; do not
   write back to an unknown device database.
3. **Provider audit.** Complete a route, auth, DTO, persistence, and error audit
   for Komga/Grimmory, OPDS, Kobo, KOReader, and liseur-sync. Keep the comparison
   and coverage vocabulary in
   `docs/content/docs/developer/liseur-providers.mdx` and
   `docs/content/docs/developer/liseur-sync-integration.mdx`.
4. **Modular ingest and editor.** Turn the staged ingest proposal into an
   additive implementation and first-party SvelteKit editor only after the
   existing scanner/metadata/job contracts are preserved. See
   `docs/content/docs/developer/modular-ingest.mdx`.
5. **Rate limiting.** Specify bounded per-user/provider request limits, retry and
   `Retry-After` behavior, and observability without placing secrets in logs.
   The external API evidence and proposed policy are recorded in
   `docs/content/docs/developer/hardcover-integration.mdx`.
6. **CI matrix.** Add or refine profile/provider checks so minimal, default/full,
   headless+liseur-sync, and compatibility replay are exercised without making a
   network client or optional provider a default startup dependency. Existing CI
   entrypoint: `.github/workflows/ci.yaml`.
7. **Real-client coverage.** Run and preserve repeatable Mihon, Liseur, and stock
   Kobo tests against fresh synthetic fixtures, in addition to the Komelia/Hurl
   replay. Keep credentials, cookies, databases, media, emulator state, and
   machine paths outside the repository.

## Verification after the current batch

- Verify the preferred `kepubify` shell-out and the built-in `stump_kepub`
  fallback against a fresh EPUB, including cache identity and response metadata;
  the adapter hook is `apps/server/src/routers/kobo_backend/kepub.rs` and the
  conversion crate is `crates/kepub`.
- After the coordinator's pending gate, deploy via a `hub` restart of the
  supervised fixture named `stump-komga`, run the observed `make replay` suite
  from `../komga-compat`, and add focused real-client probes
  only when their wire behavior is captured and sanitized.

## Future items

These remain intentionally later than the current provider and state work:

- Rust converter parity with every required `kepubify` option (the current
  converter intentionally does not promise byte-for-byte parity); see
  `docs/content/docs/developer/kobo-sync-capabilities.mdx`.
- A Readium web viewer, including progression/resource interoperability, after
  the existing EPUB route contract is stable; see
  `docs/content/docs/developer/server-architecture.mdx`.
- Tauri packaging/integration work beyond the existing desktop surface.
- An OIDC account page and subaccounts, with explicit ownership and migration
  rules; current OIDC routes are under `apps/server/src/routers/api/v2/oidc.rs`.
- QR/device approval for controlled device enrollment, with auditable expiry and
  revocation; do not treat a QR code as authentication by itself.
