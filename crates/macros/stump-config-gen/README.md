# stump-config-gen

## Purpose

`stump-config-gen` exports a single derive, `StumpConfigGenerator`, which
generates the loading machinery for `StumpConfig` and its flattened groups:
`new(...)`/`debug()` constructors, a `Partial<Name>` deserialization struct with
`empty()`/`apply_to_config()`, `Partial<Name>::from_environment()`, and — for a
root struct only — `with_config_file()`/`with_environment()`. It owns no
configuration *values*: every field, default, env key and validator lives in
`core/src/config/`, and the generated code calls back into the consuming crate's
`crate::CoreResult`/`crate::CoreError` and `toml`/`itertools` imports rather than
depending on them itself.

## Reference / upstream

| Reference | Relation |
| --- | --- |
| `core/src/config/stump_config.rs:63-66` | The only production consumer: `StumpConfig` still derives `StumpConfigGenerator` and supplies `#[config_file_location(self.get_config_dir().join("Stump.toml"))]`. |
| `core/src/config/stump_config.rs:19-27`, `:67-123` | The config split (`bc347a78`, integrated in `7617c66d`) moved settings into `#[nested] #[serde(flatten)]` group structs (`ServerConfig`, `JobsConfig`, ...) that each derive the macro and hand their `Partial*` to the parent. |
| `core/src/config/env_keys.rs:9-15` | Env key constants (`STUMP_CONFIG_DIR`, `STUMP_PORT`, ...) passed to `#[env_key(...)]`; the macro synthesizes no prefix of its own. |
| `docs/content/docs/guides/configuration/server-config.mdx:18` | User-facing statement that grouping is structural only and TOML/env keys stay flat — the property the `#[nested]` + `serde(flatten)` codegen preserves. |
| `core/Cargo.toml:74` | Unconditional path dependency; the macro links into every server profile because config loading is not feature-gated. |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Attribute vocabulary is exactly `default_value`, `debug_value`, `required_by_new`, `env_key`, `validator`, `nested` (fields) and `config_file_location` (struct) | A closed set keeps expansion predictable; anything else is ignored silently by the parser. | `src/lib.rs:74-85`, `src/config_vars.rs:89-138` |
| Three generated pieces: constructors (`new`/`debug`), the partial (`Partial<Name>` + `apply_to_config`), and env loading (`from_environment`) | Each layer overlays the previous one — defaults, then `Stump.toml`, then environment — which is the order `StumpConfig::new().with_config_file().with_environment()` documents. | `src/gen_config_impls.rs:32-41`, `src/gen_partial_config.rs:36-46`, `core/src/config/stump_config.rs:50-55` |
| Only a struct carrying `#[config_file_location(...)]` gets `with_config_file`/`with_environment`; nested groups get `new`/`debug`/`Partial*::from_environment` only | Groups are loaded through their parent, so a group must not be able to read the TOML file independently and diverge. | `src/gen_config_impls.rs:18-30`, `src/lib.rs:58-65` |
| `new()` takes a parameter for every `#[required_by_new]` field, uses `#[default_value]` for the rest, and emits a spanned compile error when a field has neither | Config dir is caller-supplied; a missing default would otherwise expand to an incomplete struct literal with a useless error site. | `src/gen_config_impls.rs:44-79`, `src/config_vars.rs:45-47` |
| `debug()` prefers `#[debug_value]`, falls back to `#[default_value]`, else compile error | Debug profile only needs to override the handful of values that differ. | `src/gen_config_impls.rs:81-110` |
| `Partial<Name>` is `pub(crate)` and `#[nested]` fields embed `Partial<Type>` with `#[serde(flatten)]` | Flattening is what keeps `Stump.toml` keys top-level after the group split; `pub(crate)` means every nested group must live in the same crate as its parent. | `src/gen_partial_config.rs:17-30`, `src/gen_partial_config.rs:36-40`, test `test_nested_group_from_flat_toml` |
| `env_key` fields: parseable scalars (`u8`..`i64`, `f32`/`f64`, `usize`, `bool`) go through `str::parse` with an `InitializationError`; `Vec<T>` of strings is comma-split and trimmed; other scalars are taken as `String`; fields with no `env_key` emit nothing | The macro has token-level type knowledge only, so the type list is the discriminator; `Vec` of parseable types is rejected with a spanned error rather than guessed. | `src/gen_config_impls.rs:172-233`, `src/config_vars.rs:27-43` |
| `apply_to_config` semantics: `Option<T>` wraps in `Some`, plain `T` overwrites, `Vec<T>` **extends** with values not already present, `Option<Vec<T>>` is a compile error | Vec settings (e.g. allowed origins) must union file and env sources instead of clobbering; `Option<Vec<T>>` has no sensible merge, so it is refused. | `src/gen_partial_config.rs:74-103` |
| `#[validator(fn)]` gates the setter with `if #validator(&value)` inside the `Some` arm | Invalid file/env values leave the previous value in place instead of failing startup. | `src/gen_partial_config.rs:105-124` |
| Field types are reduced to their last path segment, so a field must name its type unqualified (`PathBuf`, not `std::path::PathBuf`) | The partial struct re-emits that ident; keeping only the segment avoids re-deriving generic paths, at the cost of requiring the type in scope. | `src/type_utils.rs:10-42`, `src/type_utils.rs:49-91` |
| Generated code references `crate::CoreResult`/`crate::CoreError`, `env`, `toml` and `collect_vec` from the *consumer* | Keeps the proc-macro crate dependency-free apart from `syn`/`quote`/`proc-macro2`; the test crate therefore defines its own `CoreError` at the root. | `src/gen_config_impls.rs:116-138`, `tests/basic_tests.rs:1-19`, `Cargo.toml:10-20` |
| `temp-env` drives the environment tests | `env::set_var` is process-global; `temp_env::with_vars` scopes it so the env and TOML tests can run in one binary. | `tests/basic_tests.rs:121-144` (`test_getting_config_from_environment`), `tests/basic_tests.rs:177-202` (`test_nested_group_from_environment`) |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | `#[proc_macro_derive(StumpConfigGenerator, attributes(...))]`, the attribute reference docs, and `config_file_location` parsing |
| `src/config_vars.rs` | Per-field model (`StumpConfigVariable`), attribute parsing, `is_parse_type`, `Partial<Type>` ident for `#[nested]` |
| `src/gen_config_impls.rs` | `new`, `debug`, `from_environment` env extractors, `with_config_file`, `with_environment` |
| `src/gen_partial_config.rs` | `Partial<Name>` struct definition, `empty()`, `apply_to_config()` merge semantics |
| `src/type_utils.rs` | `Option`/`Vec` unwrapping into `(inner type, is_optional, is_vec)` |
| `tests/basic_tests.rs`, `tests/data/basic-config.toml` | End-to-end derive tests: empty struct, defaults, partial apply, env, TOML, nested groups |

## How to verify

```text
cargo test -p stump-config-gen
cargo check -p stump_core
cargo check -p stump_core --no-default-features
```

The crate tests expand the derive against real structs (including a `#[nested]`
group and a flat TOML fixture); the `stump_core` checks prove `StumpConfig` and
its group structs still expand in every profile.

## Deep docs

- `docs/content/docs/guides/configuration/server-config.mdx` — the key set this macro loads
- `core/README.md` — where the config structs and their defaults live
