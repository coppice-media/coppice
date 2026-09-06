//! `sources-import` tests: every case is a synthetic extensions-source
//! checkout built in code, so the recogniser is pinned against the Kotlin and
//! Gradle shapes it claims to handle rather than against one upstream commit.

use std::{
	fs,
	path::{Path, PathBuf},
};

use pretty_assertions::assert_eq;
use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::{NoopProgress, Tool, ToolInput};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// A synthetic checkout: `.git` provenance, `lib-multisrc/` themes, and one
/// directory per extension.
struct Checkout {
	dir: TempDir,
}

impl Checkout {
	fn new() -> Self {
		let checkout = Self {
			dir: TempDir::new().expect("temp dir"),
		};
		let git = checkout.root().join(".git");
		fs::create_dir_all(git.join("refs/heads")).expect("git dir");
		fs::write(git.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
		fs::write(git.join("refs/heads/main"), format!("{COMMIT}\n")).expect("ref");
		fs::write(
			git.join("config"),
			"[core]\n\tbare = false\n[remote \"origin\"]\n\t\
			 url = https://github.com/keiyoushi/extensions-source.git\n",
		)
		.expect("config");
		fs::create_dir_all(checkout.root().join("src")).expect("src");

		checkout
			.theme("madara", &["MadaraBase", "Madara", "MadaraNoAjax"])
			.theme("mangathemesia", &["MangaThemesia"])
			.theme("mmrcms", &["MMRCMS"]);
		checkout
	}

	fn root(&self) -> &Path {
		self.dir.path()
	}

	/// A `lib-multisrc/<module>` theme declaring `classes`.
	fn theme(&self, module: &str, classes: &[&str]) -> &Self {
		let dir = self
			.root()
			.join("lib-multisrc")
			.join(module)
			.join("src/eu/kanade/tachiyomi/multisrc")
			.join(module);
		fs::create_dir_all(&dir).expect("theme dir");
		let declarations = classes
			.iter()
			.map(|class| format!("abstract class {class} : KeiSource() {{\n}}\n\n"))
			.collect::<String>();
		fs::write(
			dir.join(format!("{module}.kt")),
			format!("package eu.kanade.tachiyomi.multisrc.{module}\n\n{declarations}"),
		)
		.expect("theme source");
		self
	}

	/// One `src/<lang>/<name>` extension.
	fn extension(&self, lang: &str, name: &str, gradle: &str, kotlin: &str) -> &Self {
		let dir = self.root().join("src").join(lang).join(name);
		let source = dir.join(format!("src/eu/kanade/tachiyomi/extension/{lang}/{name}"));
		fs::create_dir_all(&source).expect("extension dir");
		fs::write(dir.join("build.gradle.kts"), gradle).expect("gradle");
		fs::write(source.join("Source.kt"), kotlin).expect("kotlin");
		self
	}

	fn plan(&self, output: &Path) -> Plan {
		self.plan_with(output, json!({}))
	}

	fn plan_with(&self, output: &Path, mut options: serde_json::Value) -> Plan {
		options["extensions_source"] = json!(self.root());
		options["output_dir"] = json!(output);
		let input = ToolInput::new(vec![self.root().to_path_buf()]).with_options(options);
		SourcesImport.plan(&input).expect("plan")
	}
}

/// The Gradle DSL of a themed extension, with `body` as the `keiyoushi {}`
/// tail (the `source {}` blocks and anything else).
fn gradle(name: &str, version: i64, warning: &str, theme: &str, body: &str) -> String {
	let theme = if theme.is_empty() {
		String::new()
	} else {
		format!("    theme = \"{theme}\"\n")
	};
	format!(
		"import io.github.keiyoushi.gradle.api.ContentWarning\n\n\
		 plugins {{\n    alias(kei.plugins.extension)\n}}\n\n\
		 keiyoushi {{\n    name = \"{name}\"\n    versionCode = {version}\n    \
		 contentWarning = ContentWarning.{warning}\n    libVersion = \"1.6\"\n\
		 {theme}\n{body}}}\n"
	)
}

fn source_block(lang: &str, base_url: &str) -> String {
	format!("    source {{\n        lang = \"{lang}\"\n        baseUrl = \"{base_url}\"\n    }}\n")
}

fn definitions(plan: &Plan) -> Vec<Definition> {
	plan.actions
		.iter()
		.filter(|action| action.kind == KIND_DEFINITION)
		.map(|action| serde_json::from_value(action.detail.clone()).expect("definition"))
		.collect()
}

fn definition(plan: &Plan, id: &str) -> Definition {
	definitions(plan)
		.into_iter()
		.find(|definition| definition.id == id)
		.unwrap_or_else(|| panic!("no definition {id:?} in {:?}", unsupported(plan)))
}

fn unsupported(plan: &Plan) -> Vec<UnsupportedRow> {
	rows(plan, KIND_UNSUPPORTED)
}

fn index(plan: &Plan) -> Vec<IndexRow> {
	rows(plan, KIND_INDEX)
}

fn rows<T: serde::de::DeserializeOwned>(plan: &Plan, kind: &str) -> Vec<T> {
	let action = plan
		.actions
		.iter()
		.find(|action| action.kind == kind)
		.unwrap_or_else(|| panic!("no {kind} action"));
	serde_json::from_value(action.detail["rows"].clone()).expect("rows")
}

fn stats(plan: &Plan) -> serde_json::Value {
	plan.actions
		.iter()
		.find(|action| action.kind == KIND_STATS)
		.expect("stats action")
		.detail
		.clone()
}

fn reason(plan: &Plan, id: &str) -> (String, String) {
	let row = unsupported(plan)
		.into_iter()
		.find(|row| row.id == id)
		.unwrap_or_else(|| panic!("{id:?} is not unsupported"));
	(row.code, row.reason)
}

fn knob(definition: &Definition, key: &str) -> Knob {
	definition
		.knobs
		.get(key)
		.unwrap_or_else(|| panic!("no knob {key:?} in {:?}", definition.knobs))
		.clone()
}

fn text(value: &str) -> Knob {
	Knob::Str(value.to_string())
}

#[test]
fn positional_constructor_arguments_fill_name_base_url_and_lang() {
	let checkout = Checkout::new();
	// The pre-KSP layout: the DSL declares the source but the metadata still
	// comes from the base class constructor call.
	checkout.extension(
		"en",
		"legacy",
		&gradle(
			"Legacy",
			7,
			"MIXED",
			"madara",
			"    source {\n    }\n",
		),
		"package eu.kanade.tachiyomi.extension.en.legacy\n\n\
		 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
		 import keiyoushi.annotation.Source\n\n\
		 @Source\n\
		 abstract class Legacy : Madara(\"Legacy Reader\", \"https://legacy.test/\", \"pt-BR\")\n",
	);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());
	let definition = definition(&plan, "en.legacy");

	assert_eq!(definition.name, "Legacy Reader");
	assert_eq!(definition.base_url, "https://legacy.test");
	assert_eq!(definition.lang, "pt-BR");
	assert_eq!(definition.theme, "madara");
	assert_eq!(definition.version, 7);
	assert!(definition.nsfw, "MIXED is nsfw upstream");
	assert_eq!(definition.schema, SCHEMA);
	assert_eq!(definition.upstream.path, "src/en/legacy");
}

#[test]
fn named_constructor_arguments_become_knobs() {
	let checkout = Checkout::new();
	checkout.extension(
		"en",
		"named",
		&gradle(
			"Named",
			1,
			"SAFE",
			"mangathemesia",
			&source_block("en", "https://named.test"),
		),
		"package eu.kanade.tachiyomi.extension.en.named\n\n\
		 import eu.kanade.tachiyomi.multisrc.mangathemesia.MangaThemesia\n\
		 import keiyoushi.annotation.Source\n\n\
		 @Source\n\
		 abstract class Named : MangaThemesia(\n    \
		 dateFormat = SimpleDateFormat(\"MMM d, yyyy\", Locale.US),\n    \
		 mangaUrlDirectory = \"/series\",\n)\n",
	);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());
	let definition = definition(&plan, "en.named");

	assert_eq!(knob(&definition, "date_format"), text("MMM d, yyyy"));
	assert_eq!(knob(&definition, "date_locale"), text("en-US"));
	assert_eq!(knob(&definition, "manga_url_directory"), text("/series"));
	// The name is the DSL's, not a constructor argument's.
	assert_eq!(definition.name, "Named");
}

#[test]
fn override_values_become_snake_case_knobs_of_the_declared_type() {
	let checkout = Checkout::new();
	checkout.extension(
		"vi",
		"knobs",
		&gradle(
			"Knobs",
			3,
			"SAFE",
			"madara",
			&source_block("vi", "https://knobs.test"),
		),
		"package eu.kanade.tachiyomi.extension.vi.knobs\n\n\
		 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
		 import keiyoushi.annotation.Source\n\n\
		 @Source\n\
		 abstract class Knobs : Madara() {\n    \
		 override val mangaSubString = \"truyen\"\n    \
		 override val filterNonMangaItems = false\n    \
		 override val pageSize: Int = 24\n    \
		 override val apiUrl = \"$baseUrl/api/v1\"\n    \
		 override val genreDirectory get() = \"the-loai\"\n\
		 }\n",
	);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());
	let definition = definition(&plan, "vi.knobs");

	assert_eq!(knob(&definition, "manga_sub_string"), text("truyen"));
	assert_eq!(
		knob(&definition, "filter_non_manga_items"),
		Knob::Bool(false)
	);
	assert_eq!(knob(&definition, "page_size"), Knob::Int(24));
	// A `$baseUrl` template is resolved against the source's own base URL.
	assert_eq!(
		knob(&definition, "api_url"),
		text("https://knobs.test/api/v1")
	);
	assert_eq!(knob(&definition, "genre_directory"), text("the-loai"));
	// Universal knobs.
	assert_eq!(knob(&definition, "base_class"), text("Madara"));
	assert_eq!(knob(&definition, "browse_mode"), text("ajax"));
}

#[test]
fn single_expression_functions_and_aliases_become_selector_knobs() {
	let checkout = Checkout::new();
	checkout.extension(
		"en",
		"selectors",
		&gradle(
			"Selectors",
			1,
			"SAFE",
			"mmrcms",
			&source_block("en", "https://selectors.test"),
		),
		"package eu.kanade.tachiyomi.extension.en.selectors\n\n\
		 import eu.kanade.tachiyomi.multisrc.mmrcms.MMRCMS\n\
		 import keiyoushi.annotation.Source\n\n\
		 @Source\n\
		 abstract class Selectors : MMRCMS() {\n\n    \
		 override fun popularMangaSelector(): String = \"div.comic-list .group\"\n\n    \
		 override fun popularMangaNextPageSelector(): String? = \"nav a[rel=next]\"\n\n    \
		 override fun latestUpdatesSelector() = popularMangaSelector()\n\n    \
		 override fun chapterListSelector(): String =\n        \
		 \".overflow-hidden > a\"\n\
		 }\n",
	);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());
	let definition = definition(&plan, "en.selectors");

	assert_eq!(
		knob(&definition, "popular_manga_selector"),
		text("div.comic-list .group")
	);
	assert_eq!(
		knob(&definition, "popular_manga_next_page_selector"),
		text("nav a[rel=next]")
	);
	// `= popularMangaSelector()` is an alias for another member of the class.
	assert_eq!(
		knob(&definition, "latest_updates_selector"),
		text("div.comic-list .group")
	);
	// A wrapped single-expression body is still one expression.
	assert_eq!(
		knob(&definition, "chapter_list_selector"),
		text(".overflow-hidden > a")
	);
}

#[test]
fn enum_and_date_overrides_become_string_knobs() {
	let checkout = Checkout::new();
	checkout.extension(
		"vi",
		"dates",
		&gradle(
			"Dates",
			1,
			"NSFW",
			"madara",
			&source_block("vi", "https://dates.test"),
		),
		"package eu.kanade.tachiyomi.extension.vi.dates\n\n\
		 import eu.kanade.tachiyomi.multisrc.madara.MadaraNoAjax\n\
		 import keiyoushi.annotation.Source\n\n\
		 @Source\n\
		 abstract class Dates : MadaraNoAjax() {\n    \
		 override val chapterMode = ChapterMode.MangaAjaxPaginated\n    \
		 override val chapterDateFormat = DateTimeFormatter.ofPattern(\"dd/MM/yyyy\", Locale.forLanguageTag(\"vi\"))\n    \
		 override val dateFormat = SimpleDateFormat(\"MMMM d, yyyy\", Locale.ROOT)\n\
		 }\n",
	);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());
	let definition = definition(&plan, "vi.dates");

	assert_eq!(
		knob(&definition, "chapter_mode"),
		text("manga_ajax_paginated")
	);
	assert_eq!(knob(&definition, "chapter_date_format"), text("dd/MM/yyyy"));
	assert_eq!(knob(&definition, "chapter_date_locale"), text("vi"));
	assert_eq!(knob(&definition, "date_format"), text("MMMM d, yyyy"));
	// `Locale.ROOT` is the empty tag, not a missing knob.
	assert_eq!(knob(&definition, "date_locale"), text(""));
	// The base class variant is data: the engine browses differently.
	assert_eq!(knob(&definition, "base_class"), text("MadaraNoAjax"));
	assert_eq!(knob(&definition, "browse_mode"), text("no_ajax"));
	assert!(definition.nsfw);
}

#[test]
fn rate_limits_are_derived_from_configure_client_and_a_pure_client_chain() {
	let checkout = Checkout::new();
	checkout
		.extension(
			"en",
			"polite",
			&gradle(
				"Polite",
				1,
				"SAFE",
				"madara",
				&source_block("en", "https://polite.test"),
			),
			"package eu.kanade.tachiyomi.extension.en.polite\n\n\
			 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\n\
			 abstract class Polite : Madara() {\n    \
			 override fun OkHttpClient.Builder.configureClient() = rateLimit(3, 2.seconds) { !it.encodedPath.startsWith(\"/wp-content/\") }\n\
			 }\n",
		)
		.extension(
			"en",
			"chained",
			&gradle(
				"Chained",
				1,
				"SAFE",
				"madara",
				&source_block("en", "https://chained.test"),
			),
			"package eu.kanade.tachiyomi.extension.en.chained\n\n\
			 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\n\
			 abstract class Chained : Madara() {\n    \
			 override val client = super.client.newBuilder()\n        \
			 .rateLimit(1)\n        \
			 .build()\n\
			 }\n",
		)
		.extension(
			"en",
			"intercepted",
			&gradle(
				"Intercepted",
				1,
				"SAFE",
				"madara",
				&source_block("en", "https://intercepted.test"),
			),
			"package eu.kanade.tachiyomi.extension.en.intercepted\n\n\
			 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\n\
			 abstract class Intercepted : Madara() {\n    \
			 override val client = super.client.newBuilder()\n        \
			 .addNetworkInterceptor(::imageDescrambler)\n        \
			 .build()\n\
			 }\n",
		);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());

	let polite = definition(&plan, "en.polite");
	assert_eq!(knob(&polite, "rate_limit_permits"), Knob::Int(3));
	assert_eq!(knob(&polite, "rate_limit_period_seconds"), Knob::Int(2));

	// `rateLimit(permits)` is per second upstream (RateLimit.kt: `period =
	// 1.seconds`).
	let chained = definition(&plan, "en.chained");
	assert_eq!(knob(&chained, "rate_limit_permits"), Knob::Int(1));
	assert_eq!(knob(&chained, "rate_limit_period_seconds"), Knob::Int(1));

	// An interceptor is behaviour no engine can reproduce from data.
	let (code, reason) = reason(&plan, "en.intercepted");
	assert_eq!(code, "custom-client");
	assert!(reason.contains("addNetworkInterceptor"), "{reason}");
}

#[test]
fn request_overrides_become_url_templates() {
	let checkout = Checkout::new();
	checkout.extension(
		"en",
		"requests",
		&gradle(
			"Requests",
			1,
			"SAFE",
			"mmrcms",
			&source_block("en", "https://requests.test"),
		),
		"package eu.kanade.tachiyomi.extension.en.requests\n\n\
		 import eu.kanade.tachiyomi.multisrc.mmrcms.MMRCMS\n\
		 import keiyoushi.annotation.Source\n\n\
		 @Source\n\
		 abstract class Requests : MMRCMS() {\n    \
		 override fun popularMangaRequest(page: Int) = GET(\"$baseUrl/comic-list?sort=views&page=$page\")\n    \
		 override fun latestUpdatesRequest(page: Int): Request = GET(\"$baseUrl/comic-list?sort=latest&page=$page\", headers)\n\
		 }\n",
	);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());
	let definition = definition(&plan, "en.requests");

	assert_eq!(
		knob(&definition, "popular_manga_url"),
		text("https://requests.test/comic-list?sort=views&page={page}")
	);
	assert_eq!(
		knob(&definition, "latest_updates_url"),
		text("https://requests.test/comic-list?sort=latest&page={page}")
	);
}

#[test]
fn several_source_blocks_expand_to_one_definition_each() {
	let checkout = Checkout::new();
	checkout.extension(
		"ja",
		"twin",
		&gradle(
			"Twin",
			4,
			"MIXED",
			"madara",
			"    source {\n        name = \"Twin JP\"\n        lang = \"ja\"\n        \
			 baseUrl = \"https://jp.twin.test\"\n    }\n\n    \
			 source {\n        name = \"Twin EN\"\n        lang = \"en\"\n        \
			 baseUrl = \"https://en.twin.test\"\n    }\n",
		),
		"package eu.kanade.tachiyomi.extension.ja.twin\n\n\
		 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
		 import keiyoushi.annotation.Source\n\n\
		 @Source\n\
		 abstract class Twin : Madara() {\n    \
		 override val apiUrl = \"$baseUrl/api\"\n\
		 }\n",
	);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());
	let derived = definitions(&plan);
	assert_eq!(derived.len(), 2, "{derived:?}");

	let jp = definition(&plan, "ja.twin.twinjp");
	let en = definition(&plan, "ja.twin.twinen");
	assert_eq!(jp.name, "Twin JP");
	assert_eq!(jp.lang, "ja");
	assert_eq!(en.name, "Twin EN");
	assert_eq!(en.lang, "en");
	// One shared Kotlin class, but the template resolves per block.
	assert_eq!(knob(&jp, "api_url"), text("https://jp.twin.test/api"));
	assert_eq!(knob(&en, "api_url"), text("https://en.twin.test/api"));
	// Both blocks carry the extension's version and content warning.
	assert_eq!((jp.version, jp.nsfw), (4, true));
	assert_eq!((en.version, en.nsfw), (4, true));

	let files = index(&plan)
		.into_iter()
		.map(|row| row.file)
		.collect::<Vec<_>>();
	assert_eq!(
		files,
		vec!["en/ja.twin.twinen.json", "ja/ja.twin.twinjp.json"]
	);
}

/// An id is the data set's primary key and names a file, so which
/// discriminator wins is a contract, not an implementation detail.
#[test]
fn multi_source_ids_fall_back_from_name_to_lang_to_block_index() {
	let checkout = Checkout::new();
	let kotlin = |package: &str, class: &str| {
		format!(
			"package eu.kanade.tachiyomi.extension.all.{package}\n\n\
			 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\n\
			 abstract class {class} : Madara() {{\n}}\n"
		)
	};
	checkout
		.extension(
			"all",
			"bylang",
			&gradle(
				"ByLang",
				1,
				"SAFE",
				"madara",
				// One name, two languages: upstream's multi-language shape.
				"    source {\n        lang = \"en\"\n        \
				 baseUrl = \"https://en.bylang.test\"\n    }\n\n    \
				 source {\n        lang = \"es\"\n        \
				 baseUrl = \"https://es.bylang.test\"\n    }\n",
			),
			&kotlin("bylang", "ByLang"),
		)
		.extension(
			"all",
			"byindex",
			&gradle(
				"ByIndex",
				1,
				"SAFE",
				"madara",
				// Neither the name nor the lang tells the blocks apart.
				"    source {\n        lang = \"all\"\n        \
				 baseUrl = \"https://one.byindex.test\"\n    }\n\n    \
				 source {\n        lang = \"all\"\n        \
				 baseUrl = \"https://two.byindex.test\"\n    }\n",
			),
			&kotlin("byindex", "ByIndex"),
		);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());

	let url = |id: &str| definition(&plan, id).base_url;
	assert_eq!(url("all.bylang.en"), "https://en.bylang.test");
	assert_eq!(url("all.bylang.es"), "https://es.bylang.test");
	assert_eq!(url("all.byindex.1"), "https://one.byindex.test");
	assert_eq!(url("all.byindex.2"), "https://two.byindex.test");
}

#[test]
fn base_url_modes_and_comments_are_parsed() {
	let checkout = Checkout::new();
	checkout
		.extension(
			"zh",
			"custom",
			&gradle(
				"Custom",
				1,
				"SAFE",
				"madara",
				"    source {\n        lang = \"zh-Hant\"\n        \
				 // The user may override this; the literal is the default.\n        \
				 baseUrl {\n            custom(\"https://custom.test\")\n        }\n    }\n",
			),
			"package eu.kanade.tachiyomi.extension.zh.custom\n\n\
			 /* A block comment holding a fake member:\n   \
			 override val mangaSubString = \"ignored\"\n*/\n\
			 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\n\
			 abstract class Custom : Madara() {\n    \
			 // override val genreDirectory = \"ignored\"\n    \
			 override val mangaSubString = \"https://not-a-comment.test//path\"\n\
			 }\n",
		)
		.extension(
			"zh",
			"mirrored",
			&gradle(
				"Mirrored",
				1,
				"SAFE",
				"madara",
				"    source {\n        lang = \"zh\"\n        baseUrl {\n            \
				 mirrors(\n                \"Main\" to \"https://first.test\",\n                \
				 \"Mirror\" to \"https://second.test\",\n            )\n        }\n    }\n",
			),
			"package eu.kanade.tachiyomi.extension.zh.mirrored\n\n\
			 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\n\
			 abstract class Mirrored : Madara()\n",
		);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());

	let custom = definition(&plan, "zh.custom");
	assert_eq!(custom.base_url, "https://custom.test");
	assert_eq!(custom.lang, "zh-Hant");
	// A comment never contributes a knob, and `//` inside a string is not one.
	assert_eq!(
		custom.knobs.keys().collect::<Vec<_>>(),
		vec!["base_class", "browse_mode", "manga_sub_string"]
	);
	assert_eq!(
		knob(&custom, "manga_sub_string"),
		text("https://not-a-comment.test//path")
	);

	// The first mirror is the default, taken from the `label to url` pair.
	assert_eq!(
		definition(&plan, "zh.mirrored").base_url,
		"https://first.test"
	);
}

#[test]
fn behaviour_overrides_are_unsupported_with_the_member_named() {
	let checkout = Checkout::new();
	let kotlin = |body: &str| {
		format!(
			"package eu.kanade.tachiyomi.extension.en.x\n\n\
			 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\n\
			 abstract class X : Madara() {{\n{body}}}\n"
		)
	};
	let dsl = |name: &str| {
		gradle(
			name,
			1,
			"SAFE",
			"madara",
			&source_block("en", "https://x.test"),
		)
	};

	checkout
		.extension(
			"en",
			"parsed",
			&dsl("Parsed"),
			&kotlin(
				"    override fun pageListParse(document: Document): List<Page> {\n        \
				 return document.select(\"img\").map { Page(0, imageUrl = it.attr(\"src\")) }\n    }\n",
			),
		)
		.extension(
			"en",
			"lambda",
			&dsl("Lambda"),
			&kotlin(
				"    override fun pageListParse(document: Document) = document.select(\"img\").mapIndexed { i, img ->\n        \
				 Page(i, imageUrl = img.attr(\"src\"))\n    }\n",
			),
		)
		.extension(
			"en",
			"lazy",
			&dsl("Lazy"),
			&kotlin("    override val apiUrl by lazy { baseUrl.toHttpUrl().host }\n"),
		)
		.extension(
			"en",
			"computed",
			&dsl("Computed"),
			&kotlin("    override val ongoingStatus = super.ongoingStatus + \"dang dich\"\n"),
		)
		.extension(
			"en",
			"started",
			&dsl("Started"),
			&kotlin("    init {\n        Injekt.get<Application>()\n    }\n"),
		)
		.extension(
			"en",
			"configurable",
			&dsl("Configurable"),
			&"package eu.kanade.tachiyomi.extension.en.configurable\n\n\
			 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\n\
			 abstract class Configurable :\n    Madara(),\n    ConfigurableSource {\n    \
			 override val mangaSubString = \"manga\"\n}\n"
				.to_string(),
		);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());
	assert!(definitions(&plan).is_empty(), "{:?}", definitions(&plan));

	let (code, parsed) = reason(&plan, "en.parsed");
	assert_eq!(code, "custom-override");
	assert!(parsed.contains("pageListParse"), "{parsed}");
	assert!(parsed.contains("block body"), "{parsed}");

	// A single-expression body is not enough: a lambda is still logic.
	assert_eq!(reason(&plan, "en.lambda").0, "custom-override");
	let (code, delegated) = reason(&plan, "en.lazy");
	assert_eq!(code, "unparsed-knob");
	assert!(delegated.contains("delegated"), "{delegated}");
	let (code, computed) = reason(&plan, "en.computed");
	assert_eq!(code, "unparsed-knob");
	assert!(computed.contains("super.ongoingStatus"), "{computed}");
	assert_eq!(reason(&plan, "en.started").0, "init-block");
	let (code, supertype) = reason(&plan, "en.configurable");
	assert_eq!(code, "extra-supertype");
	assert!(supertype.contains("ConfigurableSource"), "{supertype}");

	// Every unsupported row is attributed to the theme it would have used, so
	// the stats table can be read per theme.
	for row in unsupported(&plan) {
		assert_eq!(row.theme.as_deref(), Some("madara"), "{row:?}");
		assert_eq!(row.lang, "en");
	}
}

/// `unsupported.json` is the porting work list for `crates/provider-themes`, so
/// a class that needs four members ported must not read as if it needed one.
#[test]
fn every_blocking_member_is_named_not_just_the_first() {
	let checkout = Checkout::new();
	checkout.extension(
		"en",
		"heavy",
		&gradle(
			"Heavy",
			1,
			"SAFE",
			"mmrcms",
			&source_block("en", "https://heavy.test"),
		),
		"package eu.kanade.tachiyomi.extension.en.heavy\n\n\
		 import eu.kanade.tachiyomi.multisrc.mmrcms.MMRCMS\n\
		 import keiyoushi.annotation.Source\n\n\
		 @Source\n\
		 abstract class Heavy : MMRCMS() {\n    \
		 override val itemPath = \"comic\"\n\n    \
		 override fun popularMangaFromElement(element: Element): SManga = \
		 SManga.create().apply {\n        title = element.text()\n    }\n\n    \
		 override fun mangaDetailsParse(document: Document): SManga {\n        \
		 return SManga.create()\n    }\n\n    \
		 override fun chapterListSelector() = \".chapters > li\"\n\n    \
		 override fun pageListParse(document: Document) = \
		 document.select(\"img\").map { Page(0) }\n}\n",
	);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());

	// The first blocker in file order owns the code and leads the reason...
	let (code, blocked) = reason(&plan, "en.heavy");
	assert_eq!(code, "custom-override");
	assert!(blocked.starts_with("popularMangaFromElement:"), "{blocked}");
	// ...and every later one is named, so the row is countable work.
	assert!(
		blocked.ends_with("(and 2 more: mangaDetailsParse, pageListParse)"),
		"{blocked}"
	);
	// Members that *are* data are never listed: both of these derived.
	assert!(!blocked.contains("itemPath"), "{blocked}");
	assert!(!blocked.contains("chapterListSelector"), "{blocked}");
}

#[test]
fn a_source_without_a_theme_is_never_masked_by_its_overrides() {
	let checkout = Checkout::new();
	checkout
		.extension(
			"en",
			"handwritten",
			&gradle(
				"Handwritten",
				1,
				"SAFE",
				"",
				&source_block("en", "https://handwritten.test"),
			),
			"package eu.kanade.tachiyomi.extension.en.handwritten\n\n\
			 import keiyoushi.annotation.Source\n\
			 import keiyoushi.source.KeiSource\n\n\
			 @Source\n\
			 abstract class Handwritten : KeiSource() {\n    \
			 override fun popularMangaParse(response: Response): MangasPage {\n        \
			 return MangasPage(emptyList(), false)\n    }\n\
			 }\n",
		)
		.extension(
			"en",
			"mismatched",
			&gradle(
				"Mismatched",
				1,
				"SAFE",
				"madara",
				&source_block("en", "https://mismatched.test"),
			),
			"package eu.kanade.tachiyomi.extension.en.mismatched\n\n\
			 import eu.kanade.tachiyomi.multisrc.mangathemesia.MangaThemesia\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\n\
			 abstract class Mismatched : MangaThemesia()\n",
		)
		.extension(
			"en",
			"generated",
			&gradle(
				"Generated",
				1,
				"SAFE",
				"madara",
				"    languages = listOf(\"en\", \"fr\")\n",
			),
			"package eu.kanade.tachiyomi.extension.en.generated\n\n\
			 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\n\
			 abstract class Generated : Madara()\n",
		);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());
	assert!(definitions(&plan).is_empty());

	// The class carries logic, but the root cause is that it is not themed.
	let (code, reason_text) = reason(&plan, "en.handwritten");
	assert_eq!(code, "no-theme");
	assert!(reason_text.contains("KeiSource"), "{reason_text}");

	let (code, reason_text) = reason(&plan, "en.mismatched");
	assert_eq!(code, "unknown-base-class");
	assert!(reason_text.contains("MangaThemesia"), "{reason_text}");

	// An unknown DSL key means the generator would silently drop a setting.
	let (code, reason_text) = reason(&plan, "en.generated");
	assert_eq!(code, "unparsed-gradle");
	assert!(reason_text.contains("languages"), "{reason_text}");
}

#[test]
fn apply_writes_the_definitions_index_unsupported_and_notice() {
	let checkout = Checkout::new();
	checkout
		.extension(
			"en",
			"good",
			&gradle(
				"Good",
				2,
				"SAFE",
				"mangathemesia",
				&source_block("en", "https://good.test"),
			),
			"package eu.kanade.tachiyomi.extension.en.good\n\n\
			 import eu.kanade.tachiyomi.multisrc.mangathemesia.MangaThemesia\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\n\
			 abstract class Good : MangaThemesia() {\n    \
			 override val hasProjectPage = true\n\
			 }\n",
		)
		.extension(
			"en",
			"bad",
			&gradle(
				"Bad",
				1,
				"NSFW",
				"mangathemesia",
				&source_block("en", "https://bad.test"),
			),
			"package eu.kanade.tachiyomi.extension.en.bad\n\n\
			 import eu.kanade.tachiyomi.multisrc.mangathemesia.MangaThemesia\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\n\
			 abstract class Bad : MangaThemesia() {\n    \
			 override fun pageListParse(document: Document): List<Page> {\n        \
			 return emptyList()\n    }\n\
			 }\n",
		);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());
	let report = SourcesImport
		.apply(&plan, &mut NoopProgress)
		.expect("apply");
	assert!(report.skipped.is_empty(), "{:?}", report.skipped);
	assert_eq!(report.applied.len(), plan.actions.len());

	let written =
		fs::read_to_string(output.path().join("en/en.good.json")).expect("definition");
	let definition: Definition = serde_json::from_str(&written).expect("json");
	assert_eq!(definition.id, "en.good");
	assert_eq!(knob(&definition, "has_project_page"), Knob::Bool(true));
	assert_eq!(definition.upstream.commit, COMMIT);
	assert_eq!(definition.upstream.repo, "keiyoushi/extensions-source");
	assert!(written.ends_with("}\n"), "files end with a newline");

	let index: Vec<IndexRow> = serde_json::from_str(
		&fs::read_to_string(output.path().join("index.json")).unwrap(),
	)
	.expect("index");
	assert_eq!(index.len(), 1);
	assert_eq!(index[0].file, "en/en.good.json");
	assert_eq!(index[0].theme, "mangathemesia");
	assert_eq!(index[0].version, 2);
	assert!(!index[0].nsfw);
	assert!(
		output.path().join(&index[0].file).is_file(),
		"every index row points at a written file"
	);

	let unsupported: Vec<UnsupportedRow> = serde_json::from_str(
		&fs::read_to_string(output.path().join("unsupported.json")).unwrap(),
	)
	.expect("unsupported");
	assert_eq!(unsupported.len(), 1);
	assert_eq!(unsupported[0].id, "en.bad");
	assert_eq!(unsupported[0].path, "src/en/bad");
	assert_eq!(unsupported[0].code, "custom-override");

	let notice = fs::read_to_string(output.path().join("NOTICE")).expect("notice");
	assert!(notice.contains(COMMIT), "the NOTICE pins the commit");
	assert!(notice.contains("Apache License"), "{notice}");
	assert!(notice.contains("keiyoushi/extensions-source"));
}

#[test]
fn plan_writes_nothing() {
	let checkout = Checkout::new();
	checkout.extension(
		"en",
		"good",
		&gradle(
			"Good",
			1,
			"SAFE",
			"madara",
			&source_block("en", "https://good.test"),
		),
		"package eu.kanade.tachiyomi.extension.en.good\n\n\
		 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
		 import keiyoushi.annotation.Source\n\n\
		 @Source\n\
		 abstract class Good : Madara()\n",
	);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());
	assert!(!plan.actions.is_empty());
	assert_eq!(
		fs::read_dir(output.path()).unwrap().count(),
		0,
		"plan must not write anything"
	);
}

#[test]
fn theme_and_lang_filters_exclude_sources_from_both_files() {
	let checkout = Checkout::new();
	let kotlin = |package: &str, class: &str, base: &str, module: &str| {
		format!(
			"package eu.kanade.tachiyomi.extension.{package}\n\n\
			 import eu.kanade.tachiyomi.multisrc.{module}.{base}\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\nabstract class {class} : {base}()\n"
		)
	};
	checkout
		.extension(
			"en",
			"keep",
			&gradle(
				"Keep",
				1,
				"SAFE",
				"madara",
				&source_block("en", "https://keep.test"),
			),
			&kotlin("en.keep", "Keep", "Madara", "madara"),
		)
		.extension(
			"fr",
			"otherlang",
			&gradle(
				"Other Lang",
				1,
				"SAFE",
				"madara",
				&source_block("fr", "https://fr.test"),
			),
			&kotlin("fr.otherlang", "OtherLang", "Madara", "madara"),
		)
		.extension(
			"en",
			"othertheme",
			&gradle(
				"Other Theme",
				1,
				"SAFE",
				"mmrcms",
				&source_block("en", "https://mmrcms.test"),
			),
			&kotlin("en.othertheme", "OtherTheme", "MMRCMS", "mmrcms"),
		)
		.extension(
			"en",
			"otherthemebroken",
			&gradle(
				"Other Theme Broken",
				1,
				"SAFE",
				"mmrcms",
				&source_block("en", "https://broken.test"),
			),
			"package eu.kanade.tachiyomi.extension.en.otherthemebroken\n\n\
			 import eu.kanade.tachiyomi.multisrc.mmrcms.MMRCMS\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\n\
			 abstract class OtherThemeBroken : MMRCMS() {\n    \
			 override fun pageListParse(document: Document): List<Page> {\n        \
			 return emptyList()\n    }\n\
			 }\n",
		);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan_with(
		output.path(),
		json!({ "themes": ["madara"], "langs": ["en"] }),
	);

	let ids = definitions(&plan)
		.into_iter()
		.map(|definition| definition.id)
		.collect::<Vec<_>>();
	assert_eq!(ids, vec!["en.keep"]);
	// A filtered-out source is neither derived nor reported as unsupported,
	// including one that would have been unsupported.
	assert!(unsupported(&plan).is_empty(), "{:?}", unsupported(&plan));
	assert_eq!(stats(&plan)["filtered"], json!(3));
}

#[test]
fn the_stats_table_counts_derived_and_unsupported_per_theme() {
	let checkout = Checkout::new();
	checkout
		.extension(
			"en",
			"one",
			&gradle(
				"One",
				1,
				"SAFE",
				"madara",
				&source_block("en", "https://one.test"),
			),
			"package eu.kanade.tachiyomi.extension.en.one\n\n\
			 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\nabstract class One : Madara()\n",
		)
		.extension(
			"en",
			"two",
			&gradle(
				"Two",
				1,
				"SAFE",
				"madara",
				&source_block("en", "https://two.test"),
			),
			"package eu.kanade.tachiyomi.extension.en.two\n\n\
			 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\n\
			 abstract class Two : Madara() {\n    \
			 override fun chapterFromElement(element: Element): SChapter {\n        \
			 return SChapter.create()\n    }\n\
			 }\n",
		);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());
	let stats = stats(&plan);

	assert_eq!(stats["extensions"], json!(2));
	assert_eq!(stats["derived"], json!(1));
	assert_eq!(stats["unsupported"], json!(1));
	assert_eq!(
		stats["themes"],
		json!([{ "theme": "madara", "derived": 1, "unsupported": 1 }])
	);
	// One column-aligned row per theme, derived-first, with a TOTAL footer.
	assert_eq!(
		stats["table"].as_str().expect("table"),
		"theme   derived  unsupported\n\
		 madara        1            1\n\
		 TOTAL         1            1\n"
	);
}

#[test]
fn provenance_is_read_from_the_checkout_without_running_git() {
	let checkout = Checkout::new();
	checkout.extension(
		"en",
		"good",
		&gradle(
			"Good",
			1,
			"SAFE",
			"madara",
			&source_block("en", "https://good.test"),
		),
		"package eu.kanade.tachiyomi.extension.en.good\n\n\
		 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
		 import keiyoushi.annotation.Source\n\n\
		 @Source\nabstract class Good : Madara()\n",
	);
	let output = TempDir::new().expect("out");

	// A symbolic HEAD resolved through `refs/heads`.
	assert_eq!(
		definition(&checkout.plan(output.path()), "en.good")
			.upstream
			.commit,
		COMMIT
	);

	// The same ref, only in `packed-refs`, and a fork remote.
	let git = checkout.root().join(".git");
	fs::remove_file(git.join("refs/heads/main")).expect("unpack");
	fs::write(
		git.join("packed-refs"),
		format!(
			"# pack-refs with: peeled fully-peeled sorted \n{COMMIT} refs/heads/main\n"
		),
	)
	.expect("packed-refs");
	fs::write(
		git.join("config"),
		"[remote \"origin\"]\n\turl = git@github.com:someone/fork.git\n",
	)
	.expect("config");
	let upstream = definition(&checkout.plan(output.path()), "en.good").upstream;
	assert_eq!(upstream.commit, COMMIT);
	assert_eq!(upstream.repo, "someone/fork");

	// A detached HEAD holds the id itself.
	fs::write(git.join("HEAD"), format!("{COMMIT}\n")).expect("HEAD");
	assert_eq!(
		definition(&checkout.plan(output.path()), "en.good")
			.upstream
			.commit,
		COMMIT
	);

	// No checkout at all: the definitions are still emitted, and say so.
	fs::remove_dir_all(&git).expect("drop git");
	let plan = checkout.plan(output.path());
	assert_eq!(
		definition(&plan, "en.good").upstream.commit,
		"unknown",
		"an unresolvable commit is never invented"
	);
	assert_eq!(definition(&plan, "en.good").upstream.repo, DEFAULT_REPO);
	assert!(
		plan.warnings
			.iter()
			.any(|warning| warning.code == "unresolved-upstream-commit"),
		"{:?}",
		plan.warnings
	);
}

#[test]
fn a_checkout_or_output_that_cannot_be_used_is_refused_before_any_parsing() {
	let checkout = Checkout::new();
	let output = TempDir::new().expect("out");

	let error = SourcesImport
		.plan(
			&ToolInput::new(vec![]).with_options(json!({ "output_dir": output.path() })),
		)
		.expect_err("no checkout");
	assert!(matches!(error, ToolError::Invalid(_)), "{error}");

	let error = SourcesImport
		.plan(&ToolInput::new(vec![checkout.root().to_path_buf()]))
		.expect_err("no output_dir");
	assert!(
		matches!(&error, ToolError::Invalid(reason) if reason.contains("output_dir")),
		"{error}"
	);

	let not_a_checkout = TempDir::new().expect("dir");
	let error = SourcesImport
		.plan(&ToolInput::new(vec![]).with_options(json!({
			"extensions_source": not_a_checkout.path(),
			"output_dir": output.path(),
		})))
		.expect_err("no src/");
	assert!(
		matches!(&error, ToolError::Invalid(reason) if reason.contains("src/")),
		"{error}"
	);

	let error = SourcesImport
		.plan(&ToolInput::new(vec![]).with_options(json!({
			"extensions_source": checkout.root(),
			"output_dir": output.path().join("missing"),
		})))
		.expect_err("output_dir must exist");
	assert!(matches!(error, ToolError::Invalid(_)), "{error}");

	// A plan from another tool can never be applied by this one.
	let error = SourcesImport
		.apply(&Plan::new("other-tool"), &mut NoopProgress)
		.expect_err("plan mismatch");
	assert!(matches!(error, ToolError::PlanMismatch { .. }), "{error}");
}

#[test]
fn an_extension_with_no_usable_source_is_reported_not_guessed() {
	let checkout = Checkout::new();
	checkout
		.extension(
			"all",
			"selfhosted",
			&gradle(
				"Self Hosted",
				1,
				"SAFE",
				"madara",
				"    source {\n        lang = \"all\"\n    }\n",
			),
			"package eu.kanade.tachiyomi.extension.all.selfhosted\n\n\
			 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\nabstract class SelfHosted : Madara()\n",
		)
		.extension(
			"en",
			"classless",
			&gradle(
				"Classless",
				1,
				"SAFE",
				"madara",
				&source_block("en", "https://classless.test"),
			),
			"package eu.kanade.tachiyomi.extension.en.classless\n\n\
			 class NotASource\n",
		)
		.extension(
			"en",
			"twoclasses",
			&gradle(
				"Two Classes",
				1,
				"SAFE",
				"madara",
				&source_block("en", "https://twoclasses.test"),
			),
			"package eu.kanade.tachiyomi.extension.en.twoclasses\n\n\
			 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\nabstract class First : Madara()\n\n\
			 @Source\nabstract class Second : Madara()\n",
		);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());
	assert!(definitions(&plan).is_empty());

	assert_eq!(reason(&plan, "all.selfhosted").0, "no-base-url");
	assert_eq!(reason(&plan, "en.classless").0, "no-source-class");
	assert_eq!(reason(&plan, "en.twoclasses").0, "multiple-source-classes");
}

#[test]
fn identifiers_and_locales_are_converted_by_one_rule() {
	assert_eq!(snake_case("mangaSubString"), "manga_sub_string");
	assert_eq!(snake_case("MangaAjaxPaginated"), "manga_ajax_paginated");
	assert_eq!(snake_case("SEARCH_URL"), "search_url");
	assert_eq!(
		snake_case("useNewChapterEndpoint"),
		"use_new_chapter_endpoint"
	);
	assert_eq!(
		snake_case("popularMangaUrlSelectorImg"),
		"popular_manga_url_selector_img"
	);

	assert_eq!(locale("Locale.US").as_deref(), Some("en-US"));
	assert_eq!(locale("Locale.ROOT").as_deref(), Some(""));
	assert_eq!(locale("Locale(\"tr\")").as_deref(), Some("tr"));
	assert_eq!(locale("Locale(\"pt\", \"BR\")").as_deref(), Some("pt-BR"));
	assert_eq!(
		locale("Locale.forLanguageTag(\"vi\")").as_deref(),
		Some("vi")
	);
	// A device-dependent locale is not data.
	assert_eq!(locale("Locale.getDefault()"), None);
}

#[test]
fn options_are_validated_against_the_documented_table() {
	let options = serde_json::from_value::<SourcesImportOptions>(json!({
		"extensions_source": "/tmp/checkout",
		"output_dir": "/tmp/out",
		"themes": ["madara"],
		"langs": ["en", "fr"],
	}))
	.expect("options");
	assert_eq!(
		options.extensions_source.as_deref(),
		Some(Path::new("/tmp/checkout"))
	);
	assert_eq!(options.output_dir.as_deref(), Some(Path::new("/tmp/out")));
	assert_eq!(options.themes.as_deref(), Some(&["madara".to_string()][..]));
	assert_eq!(options.langs.unwrap().len(), 2);

	assert_eq!(
		SourcesImportOptions::default(),
		serde_json::from_value(json!({})).expect("empty options")
	);
	serde_json::from_value::<SourcesImportOptions>(json!({ "theme": "madara" }))
		.expect_err("a typo in an option name is refused");
}

/// The one path that would silently corrupt a data repo: two extensions whose
/// ids collide.
#[test]
fn colliding_ids_are_reported_and_written_once() {
	let checkout = Checkout::new();
	let kotlin = |package: &str, class: &str| {
		format!(
			"package eu.kanade.tachiyomi.extension.{package}\n\n\
			 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
			 import keiyoushi.annotation.Source\n\n\
			 @Source\nabstract class {class} : Madara()\n"
		)
	};
	// `pkgName` moves an extension's application id, which is exactly how two
	// directories can claim one id.
	checkout
		.extension(
			"en",
			"first",
			&gradle(
				"First",
				1,
				"SAFE",
				"madara",
				&source_block("en", "https://first.test"),
			),
			&kotlin("en.first", "First"),
		)
		.extension(
			"en",
			"second",
			&format!(
				"import io.github.keiyoushi.gradle.api.ContentWarning\n\n\
				 keiyoushi {{\n    name = \"Second\"\n    versionCode = 1\n    \
				 contentWarning = ContentWarning.SAFE\n    libVersion = \"1.6\"\n    \
				 theme = \"madara\"\n    pkgName = \"en.first\"\n\n{}}}\n",
				source_block("en", "https://second.test")
			),
			&kotlin("en.second", "Second"),
		);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());
	assert!(
		plan.warnings
			.iter()
			.any(|warning| warning.code == "duplicate-id"),
		"{:?}",
		plan.warnings
	);

	let report = SourcesImport
		.apply(&plan, &mut NoopProgress)
		.expect("apply");
	assert_eq!(report.skipped.len(), 1, "the second write is refused");
	assert_eq!(
		fs::read_dir(output.path().join("en")).unwrap().count(),
		1,
		"one file per id"
	);
}

/// Paths are only ever POSIX-relative in the emitted data.
#[test]
fn emitted_paths_are_relative_and_posix() {
	let checkout = Checkout::new();
	checkout.extension(
		"pt",
		"posix",
		&gradle(
			"Posix",
			1,
			"SAFE",
			"madara",
			&source_block("pt-BR", "https://posix.test"),
		),
		"package eu.kanade.tachiyomi.extension.pt.posix\n\n\
		 import eu.kanade.tachiyomi.multisrc.madara.Madara\n\
		 import keiyoushi.annotation.Source\n\n\
		 @Source\nabstract class Posix : Madara()\n",
	);

	let output = TempDir::new().expect("out");
	let plan = checkout.plan(output.path());
	let definition = definition(&plan, "pt.posix");
	assert_eq!(definition.upstream.path, "src/pt/posix");
	assert_eq!(definition_file(&definition), "pt-BR/pt.posix.json");

	let target = plan
		.actions
		.iter()
		.find(|action| action.kind == KIND_DEFINITION)
		.and_then(|action| action.target.clone())
		.expect("target");
	assert_eq!(target, output.path().join("pt-BR/pt.posix.json"));
	assert_eq!(
		plan.actions
			.iter()
			.filter(|action| action.kind == KIND_DEFINITION)
			.filter_map(|action| action.source.clone())
			.collect::<Vec<_>>(),
		vec![checkout.root().join("src/pt/posix")] as Vec<PathBuf>
	);
}
