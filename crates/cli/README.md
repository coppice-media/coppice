# cli

## Purpose

`cli` owns Stump's operator command surface: the top-level `stump` clap parser
(`Cli` = global `CliConfig` flags + optional subcommand) and three subcommand
trees — `account` (`lock`, `unlock`, `list`, `reset-password`, `reset-owner`,
`migrate-oidc`), `system` (`set-journal-mode`) and `tools` (`list`, `plan`,
`apply`). It is a plain, always-linked dependency of `apps/server`, which
parses the same `Cli` and dispatches to `handle_command`; `cli-bin` exists only
as a standalone smoke harness. It deliberately owns **no** HTTP routes, **no**
tool implementations (those are `stump_tools`, see `crates/tools/README.md`),
and **no** migrations (`crates/migrations`, used here only by tests).

## Reference / upstream

| Reference | Relation |
| --- | --- |
| Upstream Stump `apps/server` CLI (`account`/`system` subcommands) | Same command names, flags and interactive flows; `tools` is new in this fork (`c4a0b239`, "tools crate ... + CLI"). |
| `stump_tools`' `Tool` plan/apply contract (`crates/tools/README.md`) | `tools plan`/`apply` are a thin shell over `Tool::plan` + `Tool::apply` and the `registry()`/`find()` lookup. |
| `dialoguer` 0.11, `indicatif` 0.17, `prettytable-rs` 0.10 (`Cargo.toml:24-26`) | Confirm/Input/Password prompts, progress spinners and bars, and every human-readable table. |
| `crates/migrations` (`Migrator::up`) | Builds the schema of the in-memory SQLite database the OIDC-migration test runs against. |
| `docs/content/docs/developer/source-definitions.mdx:205-215` | Canonical `cargo run -p cli --bin cli-bin -- tools plan/apply` invocations. |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Config is not read here: the caller bootstraps `StumpConfig`, then `CliConfig::merge_stump_config` overlays `--config-dir` and `--password-hash-cost` | One config source of truth for both entry points; the server needs the merged config for the no-subcommand (serve) path too. | `src/config.rs:16-28`, `apps/server/src/main.rs:36-50`, `bin/main.rs:10-20` |
| The database is opened per command via `stump_core::database::connect(config)`, never held | Commands are one-shot; each handler connects then drops, so no pool outlives the process. | `src/commands/account.rs:93`, `src/commands/account.rs:137`, `src/commands/system.rs:47` |
| Errors are a flat `CliError` whose `Display` is the inner message (`#[error("{0}")]`), including `CoreError` folded into `Unknown` | Operator output, not a stack: `cli-bin` prints it and exits 1, and the server re-wraps it via `EntryError::CliError`. | `src/error.rs:3-21`, `bin/main.rs:19-24`, `apps/server/src/errors.rs:7,36` |
| Passwords are only ever taken from a hidden `dialoguer::Password` prompt with confirmation, never a flag, and stored as `bcrypt::hash(password, config.auth.password_hash_cost)` | No plaintext password in shell history or `ps`; cost stays operator-tunable through `--password-hash-cost`. | `src/commands/account.rs:139-148`, `src/commands/account.rs:71-73`, `src/config.rs:11-13` |
| `tools plan` is always a dry run; `tools apply` plans first and returns `OperationFailed("refusing to apply N action(s) without --yes")` when `--yes` is absent | A mistyped path must never mutate a library; the refusal still prints the plan so the operator sees what was declined. | `src/commands/tools.rs:9-12`, `src/commands/tools.rs:97-117`, `src/commands/tools.rs:126-139` |
| Every `tools` subcommand takes `--json`; with it the progress bar becomes `ProgressBar::hidden()` and only JSON reaches stdout | Scripted callers (the source-definitions publishing flow) parse stdout; a spinner would corrupt it. | `src/commands/tools.rs:275-296`, `docs/content/docs/developer/source-definitions.mdx:209-210` |
| Human output is `prettytable` tables (`Tool`/`Description`, `Account`/`Status`); nested action detail is `--json`-only, tables get flattened `k=v` scalars | Tables stay readable for the common case without truncating machine data. | `src/commands/tools.rs:87-92`, `src/commands/account.rs:192-202`, `src/commands/tools.rs:255-273` |
| Destructive flows gate on an explicit `Confirm`, and OIDC migration prints a numbered summary plus an owner-transfer confirmation (declining it cancels migration) | A local owner cannot be deleted without knowingly preserving ownership. | `src/commands/account.rs::migrate_oidc_account` |
| OIDC migration discovers and transfers current SQLite user references inside one `BEGIN IMMEDIATE` transaction; ambiguous unique/non-FK data conflicts, duplicate Liseur sequences, or queued payloads block source deletion. Approved device pairings without an issued credential are denied and detached before transfer. | Dynamic FK/audit discovery covers new schema rows without a stale list, and one transactional attempt transfers each row once; owned data is never dropped to satisfy a destination collision. Unissued pairing nonces cannot gain the destination's permissions. | `src/commands/account.rs::user_references`, `transfer_references`, `test_oidc_current_data_survives_temporary_db_migration`, `test_oidc_conflict_rolls_back_temporary_db`, `test_oidc_migration_denies_unissued_pairing_before_owner_transfer` |
| The local username, permissions and preferences replace the destination values; destination OIDC issuer/email/ID remain. Both login/refresh sessions and Liseur login/unknown-kind tokens are revoked; device registrations and explicitly typed durable source credentials remain. Age/lock/session limits take the stricter value. | Preserve existing account behavior without silently widening authorization or logging in with a stale security context; reject populated Liseur sequence namespaces and credential authority conflicts before deleting the source. | `src/commands/account.rs::do_migrate_oidc_account` |
| Embedded user IDs in saved `jobs`/`worker_jobs` payloads block the migration until drained or archived; missing local preferences keep the OIDC row (or create defaults if neither exists) | Opaque queued work cannot be rewritten reliably and an account without preferences would otherwise break after removing the source. | `src/commands/account.rs::do_migrate_oidc_account`, `test_oidc_rejects_queued_user_id_outside_foreign_keys`, `test_oidc_preserves_destination_preferences_without_local_preferences` |
| `stump_core` and `migrations` are pulled with `default-features = false` | Only `config` + `database::connect` (and `Migrator` in tests) are used: this drops `pdf`/`rar`/`watcher`/`apalis`/`ingest` and `sea-orm-migration/cli`, so the CLI does not drag the media stack or a second argument parser into every server build. | `Cargo.toml:11-13`, `core/Cargo.toml` `[features] default`, `crates/migrations/Cargo.toml` `[features] default = ["cli"]` |
| `tools` handling is synchronous inside the async dispatcher | The tools are blocking filesystem work with no database; running them inline keeps ordering obvious. | `src/commands/mod.rs:31-33`, `src/commands/tools.rs:54-71` |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | `Cli` root parser, re-exports `handle_command`, `Commands`, `CliConfig`, `CliError`, `clap::Parser` |
| `src/config.rs` | `CliConfig` global flags and `merge_stump_config` overlay |
| `src/error.rs` | `CliError`/`CliResult`, `From<CoreError>` |
| `src/commands/mod.rs` | `Commands` enum, `handle_command` dispatch, shared `default_progress_spinner` |
| `src/commands/account.rs` | Account lock/unlock/list/reset-password/reset-owner and the transactional OIDC migration + its test module |
| `src/commands/system.rs` | `set-journal-mode` (confirm, read `PRAGMA journal_mode`, no-op when unchanged) |
| `src/commands/tools.rs` | `stump_tools` list/plan/apply, JSON vs table rendering, `BarSink` progress sink |
| `bin/main.rs` | `cli-bin` standalone harness: bootstrap config, run command, print error and exit 1 |

## How to verify

```text
flock /tmp/coppice-cargo.lock env CARGO_BUILD_JOBS=1 cargo test -p cli --lib commands::account::tests::test_oidc_
cargo run -p cli --bin cli-bin -- --config-dir <disposable-config-dir> account migrate-oidc --username <synthetic-local-user> --oidc-email <synthetic-oidc-email>
cargo run -p cli --bin cli-bin -- --help
cargo run -p cli --bin cli-bin -- tools list
cargo run -p cli --bin cli-bin -- tools plan epub2cbz <path> --json
cargo run -p stump_server -- account list
```

The OIDC tests include two disposable on-disk SQLite databases built by
`Migrator::up`: a populated migration covering current user-scoped state and a
conflicting destination proving full rollback. `tools plan` is a dry run.

## Deep docs

- `docs/content/docs/developer/cli/index.mdx` and `docs/content/docs/developer/cli/account.mdx` — the operator-facing command reference
- `docs/content/docs/developer/source-definitions.mdx` — `tools plan`/`apply` in a real publishing flow
- `docs/content/docs/developer/calibre-tooling.mdx` — what the maintenance tools do and their external binaries
- `crates/tools/README.md` — the `Tool` contract this crate shells out to
