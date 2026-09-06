//! Data-driven theme engines for the Stump provider host.
//!
//! Nothing in this crate knows about any particular website. Each engine
//! implements one `keiyoushi/extensions-source` `lib-multisrc` theme — the
//! shared base class a few hundred Mihon extensions subclass — and is driven
//! entirely by a [`stump_provider::SourceDefinition`]: base URL, language, and
//! the `override val`/single-expression `override fun` knobs the Kotlin
//! subclass declared. Definitions are data fetched at runtime
//! ([`stump_provider::definition`]); no site-specific code is compiled in.
//!
//! Engines and the base classes they execute:
//!
//! | `theme` | Kotlin base class |
//! |---|---|
//! | `madara`, `madaralegacy` | `lib-multisrc/madara`, `lib-multisrc/madaralegacy` |
//! | `mangathemesia` | `lib-multisrc/mangathemesia` |
//! | `mmrcms` | `lib-multisrc/mmrcms` |
//!
//! Every knob's meaning is cited against the base class it comes from in
//! `crates/provider-themes/README.md` and in the per-engine module docs.
//!
//! # Attribution
//!
//! The selector tables, request shapes, relative-date vocabularies and
//! lazy-image attribute orders these engines reproduce are derived from
//! `keiyoushi/extensions-source`, which is licensed under the Apache License,
//! Version 2.0. That work is reimplemented here, not copied; the required
//! notice is reproduced in `crates/provider-themes/README.md` under
//! "Attribution (Apache-2.0 NOTICE)". Stump itself remains MIT.

pub mod date;
pub mod dom;
pub mod madara;
pub mod mangathemesia;
pub mod mmrcms;
pub mod selector;
pub mod theme;

pub use madara::MadaraSource;
pub use mangathemesia::MangaThemesiaSource;
pub use mmrcms::MmrcmsSource;
pub use theme::{ThemeContext, ThemeError, DEFAULT_REQUESTS_PER_SECOND};

use std::sync::Arc;

use stump_provider::{
	definition::{DefinitionEngine, SourceDefinition},
	host::ProviderError,
	Source,
};

/// Every theme this build can execute, in stable order.
///
/// One engine per `theme` string a definition may carry. `madara` and
/// `madaralegacy` are the same engine because the two upstream base classes
/// differ only in knobs (`chapter_mode` versus `use_new_chapter_endpoint`,
/// `browse_mode` versus `use_load_more_request`), both of which
/// [`madara::MadaraSource`] accepts.
pub fn engines() -> Vec<DefinitionEngine> {
	vec![
		DefinitionEngine {
			theme: madara::THEMES[0],
			build: build_madara,
		},
		DefinitionEngine {
			theme: madara::THEMES[1],
			build: build_madara,
		},
		DefinitionEngine {
			theme: mangathemesia::THEME,
			build: build_mangathemesia,
		},
		DefinitionEngine {
			theme: mmrcms::THEME,
			build: build_mmrcms,
		},
	]
}

/// The themes [`engines`] covers, for diagnostics and the catalog surface.
pub fn supported_themes() -> Vec<&'static str> {
	engines().into_iter().map(|engine| engine.theme).collect()
}

/// Whether a definition's `theme` has an engine in this build.
pub fn supports_theme(theme: &str) -> bool {
	supported_themes().contains(&theme)
}

fn build_madara(
	definition: &SourceDefinition,
	row: &models::entity::provider_source::Model,
) -> Result<Arc<dyn Source>, ProviderError> {
	MadaraSource::build(definition, row).map_err(into_provider_error)
}

fn build_mangathemesia(
	definition: &SourceDefinition,
	row: &models::entity::provider_source::Model,
) -> Result<Arc<dyn Source>, ProviderError> {
	MangaThemesiaSource::build(definition, row).map_err(into_provider_error)
}

fn build_mmrcms(
	definition: &SourceDefinition,
	row: &models::entity::provider_source::Model,
) -> Result<Arc<dyn Source>, ProviderError> {
	MmrcmsSource::build(definition, row).map_err(into_provider_error)
}

/// A bad definition is an operator-visible configuration error, so it keeps
/// its message rather than collapsing into a generic build failure.
fn into_provider_error(error: ThemeError) -> ProviderError {
	ProviderError::Other(error.to_string())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn every_engine_covers_a_distinct_theme() {
		let themes = supported_themes();
		assert_eq!(
			themes,
			["madara", "madaralegacy", "mangathemesia", "mmrcms"]
		);
		let mut sorted = themes.clone();
		sorted.sort_unstable();
		sorted.dedup();
		assert_eq!(sorted.len(), themes.len());
		assert!(supports_theme("madaralegacy"));
		assert!(!supports_theme("heancms"));
	}

	/// A definition-backed row must instantiate through the same path the host
	/// uses, so a knob error surfaces as a provider error and not a panic.
	#[test]
	fn engines_build_sources_from_a_row() {
		let definition = SourceDefinition {
			schema: 1,
			id: "en.example".into(),
			name: "Example".into(),
			lang: "en".into(),
			base_url: "https://example.test".into(),
			theme: "madara".into(),
			version: 1,
			nsfw: false,
			knobs: Default::default(),
			upstream: None,
		};
		let row = models::entity::provider_source::Model {
			id: "en.example".into(),
			implementation: "en.example".into(),
			catalog_id: None,
			name: "Example".into(),
			lang: "en".into(),
			base_url: "https://mirror.test".into(),
			enabled: true,
			request_headers: None,
			created_by: None,
			created_at: chrono::Utc::now().into(),
			updated_at: None,
		};
		let engine = engines()
			.into_iter()
			.find(|engine| engine.theme == "madara")
			.expect("madara engine is registered");
		let source = (engine.build)(&definition, &row).expect("source builds");
		assert_eq!(source.info().id, "en.example");
		// The row's base URL wins, so an operator can point a source at a mirror.
		assert_eq!(source.info().base_url, "https://mirror.test");
		assert!(source.info().capabilities.search);

		let broken = SourceDefinition {
			knobs: [(
				"chapter_list_selector".to_string(),
				stump_provider::KnobValue::Text("li:nth-last-of-type(1)".into()),
			)]
			.into_iter()
			.collect(),
			..definition
		};
		let error = match (engine.build)(&broken, &row) {
			Ok(_) => panic!("an invalid selector knob must not build a source"),
			Err(error) => error,
		};
		assert!(
			error.to_string().contains("chapter_list_selector"),
			"{error}"
		);
	}
}
