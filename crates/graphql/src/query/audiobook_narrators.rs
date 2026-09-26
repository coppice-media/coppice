//! `audiobookNarrators`: who reads a given work, gathered on demand for the
//! request dialog's narrator picker. Two sources run concurrently -- the
//! Hardcover audio editions of the hit's own book id and an Audible catalog
//! search by title and first author -- and are merged by narrator name. A
//! source that fails contributes nothing rather than failing the query, and
//! the merged answer is cached for an hour per `(provider, remoteId,
//! language, title, author)` so reopening the picker costs no request.

use std::{
	sync::{Arc, LazyLock},
	time::Duration,
};

use async_graphql::{Context, Object, Result, SimpleObject};
use metadata_integrations::{
	AudiobookEdition, ExternalMediaMetadata, MetadataProvider, SearchQuery, TtlCache,
};
use stump_auth::AuthContext;

use crate::data::CoreContext;

use super::unified_search::{
	audible_provider, get_hardcover_provider, language_matches, normalize_words,
	requested_language, AUDIBLE_SCOPE, AUDIBLE_SEARCH_TIMEOUT, HARDCOVER_TIMEOUT,
};

/// One narrator a requester can prefer, with what each source knows about
/// their reading of this work.
#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct NarratorOption {
	/// The name as the first source to mention it spells it.
	pub name: String,
	/// Which providers credit this narrator: `hardcover`, `audible`.
	pub sources: Vec<String>,
	/// The longest advertised length among this narrator's editions: the
	/// unabridged full reading, where an abridgement is the shorter cut.
	pub duration_seconds: Option<i32>,
	/// `true` only when every edition labelling its format is abridged;
	/// `false` as soon as one unabridged edition is known; `null` when no
	/// source says.
	pub abridged: Option<bool>,
	/// The ASIN of the edition the length was taken from, else the first
	/// edition naming this narrator that has one.
	pub asin: Option<String>,
}

const NARRATOR_TTL: Duration = Duration::from_secs(60 * 60);
const NARRATOR_CAPACITY: usize = 256;

/// Audible products fetched per lookup; the title filter keeps the ones that
/// are actually this work.
const AUDIBLE_NARRATOR_PRODUCTS: u32 = 5;

pub(super) type NarratorCache = TtlCache<(String, String), Arc<Vec<NarratorOption>>>;

static NARRATORS: LazyLock<NarratorCache> =
	LazyLock::new(|| TtlCache::new(NARRATOR_TTL, NARRATOR_CAPACITY));

#[derive(Default)]
pub struct AudiobookNarratorsQuery;

#[Object]
impl AudiobookNarratorsQuery {
	/// Narrators known for one external search hit. Open to every signed-in
	/// member, as filing a request is. `provider`/`remoteId` identify the hit
	/// (`hardcover` book id or `audible` ASIN); `title` and `authors` drive
	/// the Audible search. `language` (ISO 639 code or name, default `en`)
	/// keeps Hardcover editions in the requester's language; Audible's US
	/// catalog is English already.
	async fn audiobook_narrators(
		&self,
		ctx: &Context<'_>,
		provider: String,
		remote_id: String,
		title: String,
		authors: Option<String>,
		language: Option<String>,
	) -> Result<Vec<NarratorOption>> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let provider = provider.trim().to_ascii_lowercase();
		let remote_id = remote_id.trim().to_owned();
		if provider.is_empty() || remote_id.is_empty() || title.trim().is_empty() {
			return Ok(Vec::new());
		}
		let language = requested_language(language.as_deref());
		let key = cache_key(
			&provider,
			&remote_id,
			&language,
			title.trim(),
			authors.as_deref(),
		);
		if let Some(cached) = NARRATORS.get(&key) {
			return Ok(cached.as_ref().clone());
		}

		let hardcover = if provider == "hardcover" {
			match get_hardcover_provider(core, &auth.user.id).await {
				Ok(provider) => provider.map(|(_, provider)| provider),
				Err(error) => {
					tracing::warn!(%error, "Hardcover unavailable for narrator lookup");
					None
				},
			}
		} else {
			None
		};
		Ok(narrator_options(
			&NARRATORS,
			hardcover
				.as_deref()
				.map(|provider| provider as &dyn MetadataProvider),
			audible_provider(),
			&provider,
			&remote_id,
			title.trim(),
			authors.as_deref(),
			&language,
		)
		.await)
	}
}

/// The cache identity is every input a source reads: the Hardcover editions
/// follow `remote_id` and are kept by `language`, the Audible products follow
/// the normalized `title` and first author. Two hits sharing a remote id but
/// searched under another title or author would otherwise share one Audible
/// answer for an hour.
fn cache_key(
	provider: &str,
	remote_id: &str,
	language: &str,
	title: &str,
	authors: Option<&str>,
) -> (String, String) {
	let author = first_author(authors)
		.map(|author| normalize_words(&author))
		.unwrap_or_default();
	(
		provider.to_owned(),
		format!(
			"{remote_id}\u{1f}{language}\u{1f}{}\u{1f}{author}",
			normalize_words(title)
		),
	)
}

/// Serve from `cache` or gather from both sources concurrently and cache
/// the merge. `hardcover` is `None` when the hit is not a Hardcover book or
/// the caller has no Hardcover credential; the Audible half runs regardless.
/// Hardcover editions in another language than `language` are dropped. The
/// merge is cached only when a source that was actually asked answered: a
/// lookup whose only attempted source failed (or every attempted source did)
/// is returned but not cached, so the next opening of the picker asks again.
#[allow(clippy::too_many_arguments)]
pub(super) async fn narrator_options(
	cache: &NarratorCache,
	hardcover: Option<&dyn MetadataProvider>,
	audible: &dyn MetadataProvider,
	provider: &str,
	remote_id: &str,
	title: &str,
	authors: Option<&str>,
	language: &str,
) -> Vec<NarratorOption> {
	let key = cache_key(provider, remote_id, language, title, authors);
	if let Some(cached) = cache.get(&key) {
		return cached.as_ref().clone();
	}

	// `None`: the source was not asked, so it neither answered nor failed.
	let editions = async {
		let hardcover = hardcover?;
		Some(hardcover_editions(hardcover, remote_id, language, HARDCOVER_TIMEOUT).await)
	};
	let products = async {
		let query = SearchQuery {
			title: title.to_owned(),
			author: first_author(authors),
			limit: Some(AUDIBLE_NARRATOR_PRODUCTS),
			..Default::default()
		};
		match tokio::time::timeout(
			AUDIBLE_SEARCH_TIMEOUT,
			audible.search_media_brief(&query),
		)
		.await
		{
			Ok(Ok(outcome)) => {
				Ok(outcome
					.candidates
					.into_iter()
					.filter_map(|candidate| candidate.metadata.as_media().cloned())
					.filter(|product| {
						product.title.as_deref().is_some_and(|product_title| {
							title_matches(title, product_title)
						}) && language_matches(language, product.language.as_deref())
					})
					.collect::<Vec<_>>())
			},
			Ok(Err(error)) => {
				tracing::warn!(title, %error, "Audible search failed; narrators come from Hardcover only");
				Err(())
			},
			Err(_) => {
				tracing::warn!(
					title,
					"Audible search timed out; narrators come from Hardcover only"
				);
				Err(())
			},
		}
	};
	let (editions, products) = tokio::join!(editions, products);
	let an_attempted_source_answered =
		editions.as_ref().is_some_and(|editions| editions.is_ok()) || products.is_ok();

	let options = Arc::new(merge_narrators(
		&editions.and_then(Result::ok).unwrap_or_default(),
		&products.unwrap_or_default(),
	));
	if an_attempted_source_answered {
		cache.insert(key, Arc::clone(&options));
	}
	options.as_ref().clone()
}

/// The Hardcover audio editions of one book in `language`, given up after
/// `deadline`; a failure or timeout is logged and reported as `Err(())` so
/// the merge knows the source was asked and did not answer.
async fn hardcover_editions(
	hardcover: &dyn MetadataProvider,
	remote_id: &str,
	language: &str,
	deadline: Duration,
) -> Result<Vec<AudiobookEdition>, ()> {
	match tokio::time::timeout(deadline, hardcover.audiobook_editions(remote_id)).await {
		Ok(Ok(editions)) => Ok(editions
			.into_iter()
			.filter(|edition| language_matches(language, edition.language.as_deref()))
			.collect()),
		Ok(Err(error)) => {
			tracing::warn!(
				remote_id,
				%error,
				"Hardcover audio editions unavailable; narrators come from Audible only"
			);
			Err(())
		},
		Err(_) => {
			tracing::warn!(
				remote_id,
				"Hardcover audio editions timed out; narrators come from Audible only"
			);
			Err(())
		},
	}
}

/// The first of a `, `/`;`/`&`-separated author list, as the request stores
/// it; the catalog's `author` filter takes one name.
fn first_author(authors: Option<&str>) -> Option<String> {
	authors?
		.split([',', ';', '&'])
		.map(str::trim)
		.find(|author| !author.is_empty())
		.map(str::to_owned)
}

/// An Audible product is this work when its title is the request title or
/// begins with it as whole words ("Project Hail Mary: A Novel"). Anything
/// looser drags in the catalog's "customers also bought" padding.
fn title_matches(wanted: &str, found: &str) -> bool {
	let wanted = normalize_words(wanted);
	let found = normalize_words(found);
	!wanted.is_empty()
		&& (found == wanted
			|| found
				.strip_prefix(wanted.as_str())
				.is_some_and(|rest| rest.starts_with(' ')))
}

/// Combine the abridged flags of two editions naming the same narrator: one
/// unabridged edition settles it, otherwise any label beats none.
fn merge_abridged(current: Option<bool>, incoming: Option<bool>) -> Option<bool> {
	match (current, incoming) {
		(Some(false), _) | (_, Some(false)) => Some(false),
		(Some(true), _) | (_, Some(true)) => Some(true),
		(None, None) => None,
	}
}

/// Merge Hardcover editions with Audible products into one list per
/// narrator, keyed by the punctuation- and case-insensitive name. Order:
/// narrators both sources agree on, then Audible's (the marketplace a
/// request is most likely fulfilled from), then Hardcover-only ones by how
/// many Hardcover users shelved their edition; ties keep first mention.
pub(super) fn merge_narrators(
	editions: &[AudiobookEdition],
	products: &[ExternalMediaMetadata],
) -> Vec<NarratorOption> {
	// (name key, most-shelved edition naming this narrator, option)
	let mut options: Vec<(String, Option<i32>, NarratorOption)> = Vec::new();
	let mut credit = |name: &str,
	                  source: &str,
	                  duration_seconds: Option<i32>,
	                  abridged: Option<bool>,
	                  asin: Option<&str>,
	                  users_count: Option<i32>| {
		let key = normalize_words(name);
		if key.is_empty() {
			return;
		}
		let asin = asin.map(str::trim).filter(|asin| !asin.is_empty());
		match options.iter_mut().find(|(existing, _, _)| *existing == key) {
			Some((_, shelved, option)) => {
				if !option.sources.iter().any(|existing| existing == source) {
					option.sources.push(source.to_owned());
				}
				if duration_seconds > option.duration_seconds {
					option.duration_seconds = duration_seconds;
					if asin.is_some() {
						option.asin = asin.map(str::to_owned);
					}
				}
				if option.asin.is_none() {
					option.asin = asin.map(str::to_owned);
				}
				option.abridged = merge_abridged(option.abridged, abridged);
				*shelved = (*shelved).max(users_count);
			},
			None => options.push((
				key,
				users_count,
				NarratorOption {
					name: name.trim().to_owned(),
					sources: vec![source.to_owned()],
					duration_seconds,
					abridged,
					asin: asin.map(str::to_owned),
				},
			)),
		}
	};

	for edition in editions {
		for narrator in &edition.narrators {
			credit(
				narrator,
				"hardcover",
				edition.audio_seconds,
				edition.abridged,
				edition.asin.as_deref(),
				edition.users_count,
			);
		}
	}
	for product in products {
		let duration_seconds = product.audio_seconds.or_else(|| {
			product
				.runtime_minutes
				.map(|minutes| minutes.saturating_mul(60))
		});
		for narrator in product.narrators.iter().flatten() {
			credit(
				narrator,
				AUDIBLE_SCOPE,
				duration_seconds,
				product.abridged,
				Some(product.external_id.as_str()),
				None,
			);
		}
	}
	options.sort_by_key(|(_, shelved, option)| {
		let on_audible = option.sources.iter().any(|source| source == AUDIBLE_SCOPE);
		let rank = match (option.sources.len() > 1, on_audible) {
			(true, _) => 0,
			(false, true) => 1,
			(false, false) => 2,
		};
		(rank, std::cmp::Reverse(shelved.unwrap_or(0)))
	});
	options.into_iter().map(|(_, _, option)| option).collect()
}

#[cfg(test)]
mod tests {
	use metadata_integrations::{
		mock_http::{render_ok, MockServer},
		AudibleClient, HardcoverClient,
	};

	use super::*;

	fn edition(
		narrators: &[&str],
		seconds: Option<i32>,
		asin: Option<&str>,
	) -> AudiobookEdition {
		AudiobookEdition {
			external_id: None,
			narrators: narrators.iter().map(|name| name.to_string()).collect(),
			audio_seconds: seconds,
			abridged: None,
			asin: asin.map(str::to_owned),
			language: None,
			users_count: None,
		}
	}

	fn product(
		asin: &str,
		narrators: &[&str],
		minutes: Option<i32>,
		abridged: Option<bool>,
	) -> ExternalMediaMetadata {
		ExternalMediaMetadata {
			provider: "audible".to_owned(),
			external_id: asin.to_owned(),
			title: Some("Project Hail Mary".to_owned()),
			narrators: Some(narrators.iter().map(|name| name.to_string()).collect()),
			runtime_minutes: minutes,
			abridged,
			..Default::default()
		}
	}

	#[test]
	fn merge_dedupes_by_normalized_name_keeps_the_longest_edition_and_orders_by_agreement(
	) {
		let options = merge_narrators(
			&[
				AudiobookEdition {
					users_count: Some(3),
					..edition(&["Someone Else"], None, None)
				},
				edition(&["Ray Porter"], Some(57000), Some("HC-ASIN")),
				edition(&["  "], Some(1), None),
				AudiobookEdition {
					users_count: Some(40),
					..edition(&["Popular Reader"], Some(50000), None)
				},
			],
			&[
				product("B0AUDIBLE1", &["ray porter."], Some(941), Some(false)),
				product(
					"B0AUDIBLE2",
					&["Ray Porter", "Kate Reading"],
					Some(300),
					Some(true),
				),
			],
		);

		assert_eq!(
			options,
			vec![
				NarratorOption {
					name: "Ray Porter".to_owned(),
					sources: vec!["hardcover".to_owned(), "audible".to_owned()],
					duration_seconds: Some(57000),
					abridged: Some(false),
					asin: Some("HC-ASIN".to_owned()),
				},
				NarratorOption {
					name: "Kate Reading".to_owned(),
					sources: vec!["audible".to_owned()],
					duration_seconds: Some(18000),
					abridged: Some(true),
					asin: Some("B0AUDIBLE2".to_owned()),
				},
				NarratorOption {
					name: "Popular Reader".to_owned(),
					sources: vec!["hardcover".to_owned()],
					duration_seconds: Some(50000),
					abridged: None,
					asin: None,
				},
				NarratorOption {
					name: "Someone Else".to_owned(),
					sources: vec!["hardcover".to_owned()],
					duration_seconds: None,
					abridged: None,
					asin: None,
				},
			],
			"both sources first, then Audible, then Hardcover by shelf count"
		);
	}

	#[test]
	fn edition_languages_match_codes_and_names_and_unknown_is_kept() {
		assert_eq!(requested_language(None), "en");
		assert_eq!(requested_language(Some("  English ")), "english");
		for found in ["en", "eng", "English"] {
			assert!(language_matches("en", Some(found)), "{found}");
			assert!(language_matches("english", Some(found)), "{found}");
		}
		assert!(language_matches("en", None));
		assert!(language_matches("en", Some("  ")));
		for found in ["fi", "fin", "Finnish", "sv", "es", "nl"] {
			assert!(!language_matches("en", Some(found)), "{found}");
		}
		assert!(language_matches("fi", Some("Finnish")));
	}

	/// Every input a source reads is part of the identity; spelling that the
	/// sources themselves ignore (case, punctuation, later authors) is not.
	#[test]
	fn cache_identity_covers_language_title_and_first_author() {
		let base = cache_key(
			"hardcover",
			"1",
			"en",
			"Project Hail Mary",
			Some("Andy Weir"),
		);
		assert_ne!(
			base,
			cache_key(
				"hardcover",
				"1",
				"fi",
				"Project Hail Mary",
				Some("Andy Weir")
			)
		);
		assert_ne!(
			base,
			cache_key("hardcover", "1", "en", "Artemis", Some("Andy Weir")),
			"another title drives another Audible search"
		);
		assert_ne!(
			base,
			cache_key(
				"hardcover",
				"1",
				"en",
				"Project Hail Mary",
				Some("Someone Else")
			),
			"another author drives another Audible search"
		);
		assert_ne!(
			base,
			cache_key("hardcover", "1", "en", "Project Hail Mary", None)
		);
		assert_eq!(
			base,
			cache_key(
				"hardcover",
				"1",
				"en",
				"project hail mary!",
				Some("andy weir, Someone Else")
			),
			"normalized title and first author only"
		);
	}

	#[test]
	fn audible_products_must_be_this_title_and_author_is_the_first_listed() {
		assert!(title_matches(
			"Project Hail Mary",
			"Project Hail Mary: A Novel"
		));
		assert!(title_matches("Project Hail Mary", "project hail mary"));
		assert!(!title_matches("Project Hail Mary", "Project Hail Maryland"));
		assert!(
			!title_matches("Dune Messiah", "Dune"),
			"a shorter title is another work"
		);
		assert!(!title_matches("Ray", "Rayburn"));
		assert!(!title_matches("", "Dune"));
		assert_eq!(
			first_author(Some("Andy Weir, Someone")).as_deref(),
			Some("Andy Weir")
		);
		assert_eq!(
			first_author(Some(" & Ann Leckie")).as_deref(),
			Some("Ann Leckie")
		);
		assert_eq!(first_author(Some("  ")), None);
		assert_eq!(first_author(None), None);
	}

	fn hardcover_editions_body() -> String {
		serde_json::json!({
			"data": { "editions": [
				{
					"id": 7,
					"asin": "B08G9PRS1K",
					"audio_seconds": 57000,
					"language": { "language": "English", "code2": "en" },
					"contributions": [
						{ "contribution": "Narrator", "author": { "name": "Ray Porter" } }
					]
				},
				{
					"id": 8,
					"audio_seconds": 64980,
					"language": { "language": "Finnish", "code2": "fi" },
					"contributions": [
						{ "contribution": "Narrator", "author": { "name": "Aku Laitinen" } }
					]
				}
			] }
		})
		.to_string()
	}

	fn audible_catalog_body() -> String {
		serde_json::json!({ "products": [
			{
				"asin": "B08G9PRS1K",
				"title": "Project Hail Mary",
				"authors": [{ "name": "Andy Weir" }],
				"narrators": [{ "name": "Ray Porter" }],
				"runtime_length_min": 941,
				"format_type": "unabridged"
			},
			{
				"asin": "B0OTHER001",
				"title": "Project Hail Mary: A Novel",
				"narrators": [{ "name": "Kate Reading" }],
				"runtime_length_min": 900
			},
			{
				"asin": "B0NOISE001",
				"title": "Pride and Prejudice",
				"narrators": [{ "name": "Rosamund Pike" }],
				"runtime_length_min": 700
			},
			{
				"asin": "B0CTZVXP8H",
				"title": "Project Hail Mary (Italian edition)",
				"narrators": [{ "name": "William Angiuli" }],
				"runtime_length_min": 980,
				"language": "italian"
			}
		] })
		.to_string()
	}

	fn hardcover_at(server: &MockServer) -> HardcoverClient {
		HardcoverClient::new("token".to_owned(), Some(u32::MAX)).pointed_at(&server.url)
	}

	#[tokio::test]
	async fn narrators_merge_both_sources_then_come_from_the_cache() {
		let hardcover_server =
			MockServer::spawn(vec![render_ok(&hardcover_editions_body())]);
		let audible_server = MockServer::spawn(vec![render_ok(&audible_catalog_body())]);
		let hardcover = hardcover_at(&hardcover_server);
		let audible = AudibleClient::new().pointed_at(&audible_server.url);
		let cache = TtlCache::new(NARRATOR_TTL, NARRATOR_CAPACITY);

		let options = narrator_options(
			&cache,
			Some(&hardcover),
			&audible,
			"hardcover",
			"52709",
			"Project Hail Mary",
			Some("Andy Weir, Someone Else"),
			"en",
		)
		.await;

		assert_eq!(
			options,
			vec![
				NarratorOption {
					name: "Ray Porter".to_owned(),
					sources: vec!["hardcover".to_owned(), "audible".to_owned()],
					duration_seconds: Some(57000),
					abridged: Some(false),
					asin: Some("B08G9PRS1K".to_owned()),
				},
				NarratorOption {
					name: "Kate Reading".to_owned(),
					sources: vec!["audible".to_owned()],
					duration_seconds: Some(54000),
					abridged: None,
					asin: Some("B0OTHER001".to_owned()),
				},
			],
			"the noise product's narrator is not offered"
		);
		let audible_requests = audible_server.requests();
		assert_eq!(audible_requests.len(), 1);
		assert!(audible_requests[0].contains("title=Project+Hail+Mary"));
		assert!(
			audible_requests[0].contains("author=Andy+Weir"),
			"{}",
			audible_requests[0]
		);
		assert!(audible_requests[0].contains("num_results=5"));
		assert_eq!(hardcover_server.requests().len(), 1);

		let again = narrator_options(
			&cache,
			Some(&hardcover),
			&audible,
			"hardcover",
			"52709",
			"Project Hail Mary",
			Some("Andy Weir"),
			"en",
		)
		.await;
		assert_eq!(again, options);
		assert_eq!(audible_server.requests().len(), 1, "served from cache");
		assert_eq!(hardcover_server.requests().len(), 1, "served from cache");
		assert_eq!(cache.len(), 1);
	}

	#[tokio::test]
	async fn a_failing_source_contributes_nothing_and_the_other_still_answers() {
		let failing_graphql = serde_json::json!({
			"errors": [{ "message": "field editions not found" }]
		})
		.to_string();
		let hardcover_server = MockServer::spawn(vec![render_ok(&failing_graphql)]);
		let audible_server = MockServer::spawn(vec![render_ok(&audible_catalog_body())]);
		let hardcover = hardcover_at(&hardcover_server);
		let audible = AudibleClient::new().pointed_at(&audible_server.url);
		let cache = TtlCache::new(NARRATOR_TTL, NARRATOR_CAPACITY);

		let options = narrator_options(
			&cache,
			Some(&hardcover),
			&audible,
			"hardcover",
			"52709",
			"Project Hail Mary",
			None,
			"en",
		)
		.await;
		assert_eq!(
			options
				.iter()
				.map(|option| option.name.as_str())
				.collect::<Vec<_>>(),
			["Ray Porter", "Kate Reading"]
		);
		assert_eq!(options[0].sources, ["audible"]);
		assert_eq!(options[0].duration_seconds, Some(56460));

		let bad_request =
			"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
		let hardcover_server =
			MockServer::spawn(vec![render_ok(&hardcover_editions_body())]);
		let audible_server = MockServer::spawn(vec![bad_request.to_owned()]);
		let hardcover = hardcover_at(&hardcover_server);
		let audible = AudibleClient::new().pointed_at(&audible_server.url);
		let cache = TtlCache::new(NARRATOR_TTL, NARRATOR_CAPACITY);

		let options = narrator_options(
			&cache,
			Some(&hardcover),
			&audible,
			"hardcover",
			"52709",
			"Project Hail Mary",
			None,
			"en",
		)
		.await;
		assert_eq!(options.len(), 1);
		assert_eq!(options[0].name, "Ray Porter");
		assert_eq!(options[0].sources, ["hardcover"]);

		let audible_server = MockServer::spawn(vec![render_ok(&audible_catalog_body())]);
		let audible = AudibleClient::new().pointed_at(&audible_server.url);
		let options = narrator_options(
			&cache,
			None,
			&audible,
			AUDIBLE_SCOPE,
			"B08G9PRS1K",
			"Project Hail Mary",
			None,
			"en",
		)
		.await;
		assert_eq!(
			options.len(),
			2,
			"an Audible hit is looked up on Audible alone"
		);
		assert_eq!(cache.len(), 2);

		let hardcover_server = MockServer::spawn(vec![render_ok(&failing_graphql)]);
		let audible_server = MockServer::spawn(vec![bad_request.to_owned()]);
		let hardcover = hardcover_at(&hardcover_server);
		let audible = AudibleClient::new().pointed_at(&audible_server.url);
		let options = narrator_options(
			&cache,
			Some(&hardcover),
			&audible,
			"hardcover",
			"99",
			"Artemis",
			None,
			"en",
		)
		.await;
		assert!(options.is_empty());
		assert_eq!(
			cache.len(),
			2,
			"a lookup where every source failed is not cached"
		);
	}

	/// An Audible-only lookup (an `audible` hit, or a Hardcover hit without a
	/// credential) asks one source; when that one fails there is no answer
	/// to keep. Before, the unasked Hardcover half counted as a success and
	/// the empty list was cached for an hour.
	#[tokio::test]
	async fn an_audible_only_lookup_whose_source_fails_is_not_cached() {
		let bad_request =
			"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
		let audible_server = MockServer::spawn(vec![bad_request.to_owned()]);
		let audible = AudibleClient::new().pointed_at(&audible_server.url);
		let cache = TtlCache::new(NARRATOR_TTL, NARRATOR_CAPACITY);

		let options = narrator_options(
			&cache,
			None,
			&audible,
			AUDIBLE_SCOPE,
			"B08G9PRS1K",
			"Project Hail Mary",
			None,
			"en",
		)
		.await;
		assert!(options.is_empty());
		assert_eq!(cache.len(), 0, "the only source asked failed");

		let audible_server = MockServer::spawn(vec![render_ok(&audible_catalog_body())]);
		let audible = AudibleClient::new().pointed_at(&audible_server.url);
		let options = narrator_options(
			&cache,
			None,
			&audible,
			AUDIBLE_SCOPE,
			"B08G9PRS1K",
			"Project Hail Mary",
			None,
			"en",
		)
		.await;
		assert_eq!(
			options.len(),
			2,
			"the next opening of the picker asks again"
		);
		assert_eq!(cache.len(), 1);
	}

	/// The Hardcover editions call has the same deadline as the search: a
	/// stalled connection is a failed source, not a picker that never opens.
	#[tokio::test]
	async fn hardcover_editions_give_up_at_their_deadline() {
		let stalled = HardcoverClient::new("token".to_owned(), Some(u32::MAX))
			.pointed_at(&super::super::unified_search::tests::stalled_server(
				Duration::from_secs(3),
			));
		let started = std::time::Instant::now();
		let editions =
			hardcover_editions(&stalled, "52709", "en", Duration::from_millis(200)).await;
		assert_eq!(editions, Err(()));
		assert!(
			started.elapsed() < Duration::from_secs(2),
			"gave up after {:?}",
			started.elapsed()
		);

		let hardcover_server =
			MockServer::spawn(vec![render_ok(&hardcover_editions_body())]);
		let hardcover = hardcover_at(&hardcover_server);
		let editions =
			hardcover_editions(&hardcover, "52709", "en", Duration::from_millis(200))
				.await
				.expect("an answering Hardcover");
		assert_eq!(editions.len(), 1, "the Finnish edition is dropped");
	}
}
