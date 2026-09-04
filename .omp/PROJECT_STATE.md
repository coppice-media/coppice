# Stump Project State

On-demand resume file. The full as-of-2026-09-04 snapshot with evidence lives
in `docs/content/docs/developer/state.mdx` — read that first.

## Working tree

- Branch `headless-modular` @ f27265d1, 10 commits ahead of `origin/nightly`,
  nothing pushed. Tree intentionally dirty (~23 paths); never reset/clean it.

## Fixture and launcher

- Fixture `stump-komga` (restart via `hub`); launcher `scripts/dev-fixture-server.sh`,
  binary `target/debug/stump_server`. Base URL `http://127.0.0.1:25600`
  (or `http://$LAN_IP:25600`); root `$HOME/.local/share/stump-komga-test`.
- Credentials source: the launcher regenerates `$FIXTURE_ROOT/ENDPOINTS.md`
  (mode 0600) with every URL, username, password, and API key on each start;
  never copy or quote that generated sheet.
- Defaults `STUMP_ENABLE_KOMGA`, `ENABLE_KOBO_SYNC`, `ENABLE_KOREADER_SYNC`
  = true (latter two without `STUMP_` prefix); `PDFIUM_PATH` defaults to
  `/tmp/libpdfium.so`; `STUMP_ENABLE_UPLOAD=true`; after a rebuild log out/in
  once to store the komga-remember-me token.

## Replay commands

Run from the user-owned sibling harness `../komga-compat/` (Hurl is its
semantic test oracle, not a capture file):

```text
make replay                # 8/8 observed specs
make replay-negative-auth  # 1/1
make replay-mihon          # needs SERIES_ID
make replay-liseur-sync
```

Green = 100% of the observed replay on the deployed build; deploy via `hub`
restart of `stump-komga`, then run the replays.

## Only definition of green

Coordinator runs once after all workers yield; every command must exit 0.
Workers do not run formatters, linters, builds, or project-wide suites mid-batch:

```text
cargo fmt --all
cargo clippy -p stump_komga -p stump_opds -p stump_kobo -p stump_koreader -p stump_liseur_sync -p stump_kepub -p stump_core -p migrations -p stump_server --no-default-features --features headless,liseur-sync -- -D warnings
cargo test -p stump_komga --lib --tests
cargo test -p stump_opds --lib --tests
cargo test -p stump_kobo --lib --tests
cargo test -p stump_koreader --lib --tests
cargo test -p stump_liseur_sync --lib --tests
cargo test -p stump_kepub --lib --tests
cargo test -p stump_core --lib --tests
cargo test -p migrations --lib --tests
cargo test -p models --lib --tests
cargo test -p stump_server --lib
cargo build -p stump_server --no-default-features --features headless,liseur-sync
```

For structural changes, also `cargo check -p stump_server --no-default-features --features minimal`
and `cargo check -p stump_server` (default profile).

## In-flight and pending

- `LiseurCatalog`: implementing liseur catalog routes and the
  `/v1/works/{id}/annotations` SQL fix against pinned Liseur `31f8182d`.
- Mihon tracker device retest (harness green); `/bulk` grid not browser-driven;
  rework detail sheet shows a blank band above the header at 1280×900.
  Roadmap: `.omp/NEXT_STEPS.md`; gap analysis `local://feature-gaps.md`.

## Standing rules

- Contract evidence pinned to Komelia `65f92fde`, `komga-client` 0.11.0
  `74412a6e`, Liseur `31f8182d`, Grimmory main; detailed records in
  `docs/content/docs/developer/{komga-compat,kobo-sync-capabilities,kobo-device-database,unified-reading-state,liseur-sync-integration,liseur-providers,modular-ingest,server-architecture}.mdx`.
- The Komga mount test in `apps/server/src/routers/komga/mod.rs` must remain
  (Axum panics building overlapping routes before a request exposes it).
- Known upstream lints untouched: `emailer/sender.rs`,
  `tests/reading_progress/manual_progression_changes.rs`.
