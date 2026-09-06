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
| Destructive flows gate on an explicit `Confirm`, and the OIDC migration prints a numbered summary plus a second `default(false)` confirm before touching anything | `set-journal-mode` and the account merges are irreversible; ownership transfer inside the migration is an extra opt-in. | `src/commands/system.rs:35-42`, `src/commands/account.rs:318-359` |
| The OIDC migration runs entirely inside `models::txn::begin_write` | Reassigning ~15 user-owned tables then deleting the local account must not half-apply. | `src/commands/account.rs:387-397` |
| Locking an account and any owner change also delete that user's `session` rows | A lock or demotion that leaves a live session in place would not take effect until expiry. | `src/commands/account.rs:110-120`, `src/commands/account.rs:257-270` |
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
cargo test -p cli                          # commands::account::tests::test_oidc_user_migration
cargo run -p cli --bin cli-bin -- --help
cargo run -p cli --bin cli-bin -- tools list
cargo run -p cli --bin cli-bin -- tools plan epub2cbz <path> --json
cargo run -p stump_server -- account list
```

`test_oidc_user_migration` migrates a fully populated local user against an
in-memory SQLite database built by `Migrator::up`; the `tools plan` form is the
dry run, so it is safe to run against a real library path.

## Deep docs

- `docs/content/docs/developer/cli/index.mdx` and `docs/content/docs/developer/cli/account.mdx` — the operator-facing command reference
- `docs/content/docs/developer/source-definitions.mdx` — `tools plan`/`apply` in a real publishing flow
- `docs/content/docs/developer/calibre-tooling.mdx` — what the maintenance tools do and their external binaries
- `crates/tools/README.md` — the `Tool` contract this crate shells out to
