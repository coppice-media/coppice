//! Cross-source dedupe for materialised provider series.
//!
//! The same work is usually carried by several sources. Every
//! materialisation records its dedupe key in `provider_series_identity` (the
//! normalised title plus the strongest cross-source id the source carried);
//! when a series from source B matches one that already exists from source A,
//! a `provider_series_links` row records the duplicate.
//!
//! The link is advisory on purpose: nothing is merged automatically, so a
//! false positive is a row an operator can look at rather than lost reading
//! progress. [`merge_series`] performs the merge an operator asks for: it
//! repoints every reading head of the dropped series onto the matching
//! chapter of the kept series through
//! [`models::services::reading_state::apply`] (so the usual conflict rule
//! decides), then deletes the dropped materialised rows. Only
//! provider-backed series can be merged; a locally scanned series is never
//! touched.

use std::collections::BTreeMap;

use chrono::Utc;
use models::txn::begin_write;
use models::{
	domain::reading_state::{Position, ProtocolUpdate, Publication},
	entity::{
		media, media_metadata, provider_series_identity, provider_series_link,
		reading_head, series,
	},
	services::{lists, reading_state},
};
use sea_orm::{
	prelude::*, sea_query::OnConflict, ActiveValue::Set, ColumnTrait, ConnectionTrait,
	DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};

use crate::{host::ProviderError, source::RemoteSeries};

/// External registries whose ids name the same work across sources, strongest
/// first. A source that reports several is keyed by the first it carries.
pub const EXTERNAL_REGISTRIES: [&str; 3] = ["al", "mal", "mu"];

/// A cross-source id match; the two series are the same work.
pub const REASON_EXTERNAL_KEY: &str = "EXTERNAL_KEY";
/// Only the normalised titles matched.
pub const REASON_TITLE: &str = "TITLE";

/// Reduce a series title to comparable form: lowercased, every
/// non-alphanumeric character treated as a separator, runs of separators
/// collapsed to one space. `"Solo Leveling (Web Comic)!"` and
/// `"solo   leveling - web comic"` both become `"solo leveling web comic"`.
pub fn normalise_title(title: &str) -> String {
	let mut out = String::with_capacity(title.len());
	let mut pending_space = false;
	for ch in title.chars() {
		if ch.is_alphanumeric() {
			if pending_space && !out.is_empty() {
				out.push(' ');
			}
			pending_space = false;
			out.extend(ch.to_lowercase());
		} else {
			pending_space = true;
		}
	}
	out
}

/// `<registry>:<id>` for the strongest external id the source carried.
pub fn external_key(external_ids: &BTreeMap<String, String>) -> Option<String> {
	EXTERNAL_REGISTRIES.iter().find_map(|registry| {
		let value = external_ids.get(*registry)?.trim();
		(!value.is_empty()).then(|| format!("{registry}:{value}"))
	})
}

/// A recorded duplicate, resolved to the two series it links.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesDuplicate {
	pub link: provider_series_link::Model,
	/// The duplicate (later) series.
	pub series: series::Model,
	/// The series it duplicates.
	pub canonical: series::Model,
}

/// What one [`merge_series`] call did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergeReport {
	pub kept_series_id: String,
	pub dropped_series_id: String,
	/// Reading heads moved onto a chapter of the kept series.
	pub heads_repointed: usize,
	/// Heads whose chapter has no counterpart in the kept series; those are
	/// dropped with the media row.
	pub heads_unmatched: usize,
	pub media_deleted: Vec<String>,
}

/// Record the dedupe key of a freshly materialised series and, when it
/// matches a series from another source, the duplicate link. Returns the link
/// when one exists (or was just created).
pub async fn record_identity<C: ConnectionTrait>(
	conn: &C,
	series_row: &series::Model,
	source_id: &str,
	details: &RemoteSeries,
) -> Result<Option<provider_series_link::Model>, ProviderError> {
	let normalised_title = normalise_title(&details.title);
	if normalised_title.is_empty() {
		return Ok(None);
	}
	let key = external_key(&details.external_ids);

	provider_series_identity::Entity::insert(provider_series_identity::ActiveModel {
		series_id: Set(series_row.id.clone()),
		source_provider: Set(source_id.to_string()),
		normalised_title: Set(normalised_title.clone()),
		external_key: Set(key.clone()),
		created_at: Set(Utc::now().into()),
	})
	.on_conflict(
		OnConflict::column(provider_series_identity::Column::SeriesId)
			.update_columns([
				provider_series_identity::Column::SourceProvider,
				provider_series_identity::Column::NormalisedTitle,
				provider_series_identity::Column::ExternalKey,
			])
			.to_owned(),
	)
	.exec(conn)
	.await?;

	if let Some(link) = provider_series_link::Entity::find_by_id(&series_row.id)
		.one(conn)
		.await?
	{
		return Ok(Some(link));
	}

	// An external id is authoritative; the normalised title is the fallback.
	let mut matched = None;
	if let Some(key) = key.as_deref() {
		matched = candidate(conn, source_id, &series_row.id, |query| {
			query.filter(provider_series_identity::Column::ExternalKey.eq(key))
		})
		.await?
		.map(|series_id| (series_id, REASON_EXTERNAL_KEY));
	}
	if matched.is_none() {
		matched = candidate(conn, source_id, &series_row.id, |query| {
			query.filter(
				provider_series_identity::Column::NormalisedTitle
					.eq(normalised_title.as_str()),
			)
		})
		.await?
		.map(|series_id| (series_id, REASON_TITLE));
	}
	let Some((canonical_series_id, reason)) = matched else {
		return Ok(None);
	};

	// Collapse chains: every duplicate points at the same canonical series.
	let canonical_series_id = resolve_canonical(conn, canonical_series_id).await?;
	if canonical_series_id == series_row.id {
		return Ok(None);
	}

	let link = provider_series_link::ActiveModel {
		series_id: Set(series_row.id.clone()),
		canonical_series_id: Set(canonical_series_id),
		reason: Set(reason.to_string()),
		created_at: Set(Utc::now().into()),
	}
	.insert(conn)
	.await?;
	tracing::debug!(
		series_id = %link.series_id,
		canonical_series_id = %link.canonical_series_id,
		reason = %link.reason,
		"Recorded a cross-source provider series duplicate"
	);
	Ok(Some(link))
}

/// The oldest identity from another source matching `filter`, if any.
async fn candidate<C: ConnectionTrait, F>(
	conn: &C,
	source_id: &str,
	series_id: &str,
	filter: F,
) -> Result<Option<String>, ProviderError>
where
	F: FnOnce(
		sea_orm::Select<provider_series_identity::Entity>,
	) -> sea_orm::Select<provider_series_identity::Entity>,
{
	let query = filter(provider_series_identity::Entity::find())
		.filter(provider_series_identity::Column::SourceProvider.ne(source_id))
		.filter(provider_series_identity::Column::SeriesId.ne(series_id))
		.order_by_asc(provider_series_identity::Column::CreatedAt)
		.order_by_asc(provider_series_identity::Column::SeriesId);
	for row in query.all(conn).await? {
		// The identity outlives nothing: a GC'd series takes its row with it,
		// but a stale row must never link to a series that is gone.
		let exists = series::Entity::find_by_id(&row.series_id)
			.filter(series::Column::SourceProvider.is_not_null())
			.one(conn)
			.await?;
		if exists.is_some() {
			return Ok(Some(row.series_id));
		}
	}
	Ok(None)
}

/// Follow existing links so a duplicate always points at the head of the
/// chain (bounded: a link chain cannot outlive the series it names).
async fn resolve_canonical<C: ConnectionTrait>(
	conn: &C,
	mut series_id: String,
) -> Result<String, ProviderError> {
	let mut seen = vec![series_id.clone()];
	while let Some(link) = provider_series_link::Entity::find_by_id(&series_id)
		.one(conn)
		.await?
	{
		if seen.contains(&link.canonical_series_id) {
			break;
		}
		series_id = link.canonical_series_id;
		seen.push(series_id.clone());
	}
	Ok(series_id)
}

/// Every recorded duplicate whose two series still exist.
pub async fn duplicates<C: ConnectionTrait>(
	conn: &C,
) -> Result<Vec<SeriesDuplicate>, ProviderError> {
	let links = provider_series_link::Entity::find()
		.order_by_asc(provider_series_link::Column::CreatedAt)
		.order_by_asc(provider_series_link::Column::SeriesId)
		.all(conn)
		.await?;
	let mut out = Vec::with_capacity(links.len());
	for link in links {
		let (Some(series), Some(canonical)) = (
			series::Entity::find_by_id(&link.series_id)
				.one(conn)
				.await?,
			series::Entity::find_by_id(&link.canonical_series_id)
				.one(conn)
				.await?,
		) else {
			continue;
		};
		out.push(SeriesDuplicate {
			link,
			series,
			canonical,
		});
	}
	Ok(out)
}

/// Merge two materialised provider series: move the dropped series' reading
/// heads onto the kept series' matching chapters, then delete the dropped
/// series and its media.
///
/// Chapters are matched by chapter number when both sides have one, otherwise
/// by normalised chapter title. A head whose chapter has no counterpart is
/// reported in [`MergeReport::heads_unmatched`] and lost with its media row.
pub async fn merge_series(
	conn: &DatabaseConnection,
	keep: &str,
	drop: &str,
) -> Result<MergeReport, ProviderError> {
	if keep == drop {
		return Err(ProviderError::Other(
			"Cannot merge a series into itself".to_string(),
		));
	}
	let kept = provider_series(conn, keep).await?;
	let dropped = provider_series(conn, drop).await?;

	let kept_media = chapters_of(conn, &kept.id).await?;
	let dropped_media = chapters_of(conn, &dropped.id).await?;
	let dropped_media_ids: Vec<String> = dropped_media
		.iter()
		.map(|(media, _)| media.id.clone())
		.collect();

	let heads = if dropped_media_ids.is_empty() {
		Vec::new()
	} else {
		reading_head::Entity::find()
			.filter(reading_head::Column::MediaId.is_in(dropped_media_ids.clone()))
			.order_by_asc(reading_head::Column::UserId)
			.order_by_asc(reading_head::Column::MediaId)
			.all(conn)
			.await?
	};

	let mut report = MergeReport {
		kept_series_id: kept.id.clone(),
		dropped_series_id: dropped.id.clone(),
		media_deleted: dropped_media_ids.clone(),
		..Default::default()
	};

	let txn = begin_write(conn).await?;
	for head in heads {
		let Some((_, chapter_key)) = dropped_media
			.iter()
			.find(|(media, _)| media.id == head.media_id)
		else {
			report.heads_unmatched += 1;
			continue;
		};
		let Some((target, _)) = kept_media
			.iter()
			.find(|(_, candidate)| candidate == chapter_key)
		else {
			report.heads_unmatched += 1;
			continue;
		};
		let position = match (&head.locator, head.page) {
			(Some(locator), _) => Position::Locator(locator.clone()),
			(None, Some(page)) => Position::Page(page),
			(None, None) => Position::None,
		};
		reading_state::apply(
			&txn,
			&head.user_id,
			Publication::from(target),
			ProtocolUpdate {
				protocol: head.source_protocol,
				device_id: head.source_device_id.clone(),
				updated_at: Some(head.updated_at.to_utc()),
				position,
				progression: Some(head.progression),
				// Completion is sticky: a merge never un-reads the kept
				// chapter, it only carries a finished one over.
				completed: head.completed.then_some(true),
				raw_payload: serde_json::json!({
					"mergeProviderSeries": {
						"keep": kept.id,
						"drop": dropped.id,
						"fromMediaId": head.media_id,
					}
				}),
			},
		)
		.await?;
		report.heads_repointed += 1;
	}

	// Duplicates of the dropped series now duplicate the kept one.
	provider_series_link::Entity::update_many()
		.col_expr(
			provider_series_link::Column::CanonicalSeriesId,
			Expr::value(kept.id.clone()),
		)
		.filter(provider_series_link::Column::CanonicalSeriesId.eq(&dropped.id))
		.exec(&txn)
		.await?;
	provider_series_link::Entity::delete_many()
		.filter(provider_series_link::Column::SeriesId.eq(&dropped.id))
		.exec(&txn)
		.await?;
	provider_series_identity::Entity::delete_many()
		.filter(provider_series_identity::Column::SeriesId.eq(&dropped.id))
		.exec(&txn)
		.await?;

	if !dropped_media_ids.is_empty() {
		lists::remove_memberships_for_media(&txn, &dropped_media_ids).await?;
		media::Entity::delete_many()
			.filter(media::Column::Id.is_in(dropped_media_ids))
			.exec(&txn)
			.await?;
	}
	lists::remove_memberships_for_series(&txn, &[dropped.id.clone()]).await?;
	series::Entity::delete_by_id(&dropped.id).exec(&txn).await?;
	txn.commit().await?;

	tracing::info!(
		keep = %report.kept_series_id,
		drop = %report.dropped_series_id,
		heads_repointed = report.heads_repointed,
		heads_unmatched = report.heads_unmatched,
		"Merged provider series"
	);
	Ok(report)
}

/// A provider-backed series by id; errors when it is missing or local.
async fn provider_series(
	conn: &DatabaseConnection,
	series_id: &str,
) -> Result<series::Model, ProviderError> {
	let row = series::Entity::find_by_id(series_id)
		.one(conn)
		.await?
		.ok_or_else(|| ProviderError::Other(format!("Series {series_id} not found")))?;
	if row.source_provider.is_none() {
		return Err(ProviderError::NotVirtual(row.path));
	}
	Ok(row)
}

/// The series' media with the key chapters are matched on: the chapter number
/// when known, otherwise the normalised title.
async fn chapters_of(
	conn: &DatabaseConnection,
	series_id: &str,
) -> Result<Vec<(media::Model, String)>, ProviderError> {
	let rows = media::Entity::find()
		.filter(media::Column::SeriesId.eq(series_id))
		.order_by_asc(media::Column::Name)
		.all(conn)
		.await?;
	let media_ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
	let numbers: Vec<(Option<String>, Option<Decimal>)> = if media_ids.is_empty() {
		Vec::new()
	} else {
		media_metadata::Entity::find()
			.select_only()
			.column(media_metadata::Column::MediaId)
			.column(media_metadata::Column::Number)
			.filter(media_metadata::Column::MediaId.is_in(media_ids))
			.into_tuple()
			.all(conn)
			.await?
	};
	Ok(rows
		.into_iter()
		.map(|row| {
			let number = numbers
				.iter()
				.find(|(media_id, _)| media_id.as_deref() == Some(row.id.as_str()))
				.and_then(|(_, number)| *number);
			let key = match number {
				Some(number) => format!("number:{}", number.normalize()),
				None => format!("title:{}", normalise_title(&row.name)),
			};
			(row, key)
		})
		.collect())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn normalise_title_ignores_case_punctuation_and_spacing() {
		assert_eq!(
			normalise_title("Solo Leveling (Web Comic)!"),
			"solo leveling web comic"
		);
		assert_eq!(
			normalise_title("  solo   leveling - web  comic  "),
			"solo leveling web comic"
		);
		assert_eq!(normalise_title("Ω Factor"), "ω factor");
		assert_eq!(normalise_title("---"), "");
	}

	#[test]
	fn external_key_prefers_the_strongest_registry() {
		let ids = BTreeMap::from([
			("mal".to_string(), "31234".to_string()),
			("al".to_string(), "30002".to_string()),
			("mu".to_string(), "abc".to_string()),
		]);
		assert_eq!(external_key(&ids).as_deref(), Some("al:30002"));

		let ids = BTreeMap::from([
			("mal".to_string(), "31234".to_string()),
			("al".to_string(), "  ".to_string()),
		]);
		assert_eq!(external_key(&ids).as_deref(), Some("mal:31234"));
		assert!(external_key(&BTreeMap::new()).is_none());
		assert!(
			external_key(&BTreeMap::from([("kitsu".to_string(), "9".to_string())]))
				.is_none()
		);
	}

	/// A materialised (provider-backed) series, or a local one when
	/// `source` is `None`.
	async fn seed_series(
		db: &DatabaseConnection,
		id: &str,
		title: &str,
		source: Option<&str>,
	) -> series::Model {
		series::ActiveModel {
			id: Set(id.to_string()),
			name: Set(title.to_string()),
			path: Set(match source {
				Some(source) => format!("provider://{source}/{id}"),
				None => format!("/library/{id}"),
			}),
			source_provider: Set(source.map(str::to_string)),
			remote_id: Set(source.map(|_| id.to_string())),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("series insert")
	}

	/// One chapter of `series_id` with `number` in its metadata.
	async fn seed_chapter(
		db: &DatabaseConnection,
		series_id: &str,
		number: i64,
	) -> media::Model {
		let row = media::ActiveModel {
			id: Set(format!("{series_id}-ch{number}")),
			name: Set(format!("Chapter {number}")),
			path: Set(format!("provider://x/{series_id}/ch{number}")),
			extension: Set("cbz".to_string()),
			size: Set(0),
			pages: Set(10),
			series_id: Set(Some(series_id.to_string())),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("media insert");
		media_metadata::ActiveModel {
			media_id: Set(Some(row.id.clone())),
			number: Set(Some(Decimal::from(number))),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("media metadata insert");
		row
	}

	fn remote(
		remote_id: &str,
		title: &str,
		external: Option<(&str, &str)>,
	) -> RemoteSeries {
		RemoteSeries {
			remote_id: remote_id.to_string(),
			title: title.to_string(),
			external_ids: external
				.map(|(registry, id)| {
					BTreeMap::from([(registry.to_string(), id.to_string())])
				})
				.unwrap_or_default(),
			..Default::default()
		}
	}

	#[tokio::test]
	async fn same_title_from_another_source_is_linked_to_the_first() {
		let db = ::tests::db::test_database().await;
		// A local series with the same title is never a dedupe candidate.
		seed_series(&db, "local", "Solo Leveling", None).await;

		let alpha = seed_series(&db, "a", "Solo Leveling", Some("mock-en")).await;
		let first =
			record_identity(&db, &alpha, "mock-en", &remote("a", "Solo Leveling", None))
				.await
				.expect("record alpha");
		assert!(first.is_none(), "the first source has nothing to link to");

		// Punctuation and case differ; the normalised titles match.
		let beta = seed_series(&db, "b", "solo leveling!", Some("other-en")).await;
		let link =
			record_identity(&db, &beta, "other-en", &remote("b", "solo leveling!", None))
				.await
				.expect("record beta")
				.expect("beta is a duplicate of alpha");
		assert_eq!(link.series_id, "b");
		assert_eq!(link.canonical_series_id, "a");
		assert_eq!(link.reason, REASON_TITLE);

		// A third source collapses onto the same canonical series, not onto
		// the duplicate it happened to match.
		let gamma = seed_series(&db, "c", "Solo  Leveling", Some("third-en")).await;
		let link = record_identity(
			&db,
			&gamma,
			"third-en",
			&remote("c", "Solo  Leveling", None),
		)
		.await
		.expect("record gamma")
		.expect("gamma is a duplicate");
		assert_eq!(link.canonical_series_id, "a");

		// Another series from an already-linked source is not linked to
		// itself, and a different title is not linked at all.
		let delta = seed_series(&db, "d", "Omniscient Reader", Some("other-en")).await;
		assert!(record_identity(
			&db,
			&delta,
			"other-en",
			&remote("d", "Omniscient Reader", None)
		)
		.await
		.expect("record delta")
		.is_none());

		// A shared external id wins over the title, and re-recording is
		// idempotent.
		let epsilon = seed_series(&db, "e", "Only I Level Up", Some("fourth-en")).await;
		record_identity(
			&db,
			&alpha,
			"mock-en",
			&remote("a", "Solo Leveling", Some(("al", "30002"))),
		)
		.await
		.expect("re-record alpha");
		let link = record_identity(
			&db,
			&epsilon,
			"fourth-en",
			&remote("e", "Only I Level Up", Some(("al", "30002"))),
		)
		.await
		.expect("record epsilon")
		.expect("matched by external id");
		assert_eq!(link.canonical_series_id, "a");
		assert_eq!(link.reason, REASON_EXTERNAL_KEY);

		let recorded = duplicates(&db).await.expect("duplicates");
		assert_eq!(
			recorded
				.iter()
				.map(|dup| (dup.link.series_id.as_str(), dup.canonical.id.as_str()))
				.collect::<Vec<_>>(),
			vec![("b", "a"), ("c", "a"), ("e", "a")]
		);
	}

	#[tokio::test]
	async fn merge_repoints_heads_and_deletes_the_dropped_series() {
		let db = ::tests::db::test_database().await;
		let keep = seed_series(&db, "keep", "Solo Leveling", Some("mock-en")).await;
		let drop = seed_series(&db, "drop", "solo leveling", Some("other-en")).await;
		let kept_ch1 = seed_chapter(&db, &keep.id, 1).await;
		seed_chapter(&db, &keep.id, 2).await;
		let dropped_ch1 = seed_chapter(&db, &drop.id, 1).await;
		let dropped_ch9 = seed_chapter(&db, &drop.id, 9).await;
		record_identity(
			&db,
			&keep,
			"mock-en",
			&remote("keep", "Solo Leveling", None),
		)
		.await
		.expect("record keep");
		record_identity(
			&db,
			&drop,
			"other-en",
			&remote("drop", "solo leveling", None),
		)
		.await
		.expect("record drop")
		.expect("linked");

		let reader = ::tests::fake_data::User::new("reader").insert(&db).await;
		for media_id in [&dropped_ch1.id, &dropped_ch9.id] {
			reading_state::apply(
				&db,
				&reader.id,
				Publication {
					media_id,
					pages: 10,
					duration_ms: None,
				},
				ProtocolUpdate {
					protocol: models::domain::reading_state::SourceProtocol::Komga,
					device_id: None,
					updated_at: None,
					position: Position::Page(4),
					progression: None,
					completed: None,
					raw_payload: serde_json::json!({}),
				},
			)
			.await
			.expect("seed head");
		}

		let report = merge_series(&db, &keep.id, &drop.id).await.expect("merge");
		assert_eq!(report.heads_repointed, 1, "chapter 1 has a counterpart");
		assert_eq!(report.heads_unmatched, 1, "chapter 9 does not");
		assert_eq!(report.media_deleted.len(), 2);

		let moved = reading_state::head(&db, &reader.id, &kept_ch1.id)
			.await
			.expect("head lookup")
			.expect("head moved onto the kept chapter");
		assert_eq!(moved.page, Some(4));
		assert!((moved.progression - 0.4).abs() < 1e-9);

		assert!(series::Entity::find_by_id(&drop.id)
			.one(&db)
			.await
			.expect("lookup")
			.is_none());
		assert_eq!(
			media::Entity::find()
				.filter(media::Column::SeriesId.eq(&drop.id))
				.count(&db)
				.await
				.expect("count"),
			0
		);
		assert!(duplicates(&db).await.expect("duplicates").is_empty());
		assert!(provider_series_identity::Entity::find_by_id(&drop.id)
			.one(&db)
			.await
			.expect("lookup")
			.is_none());
		// The kept series keeps its own chapters and identity.
		assert_eq!(
			media::Entity::find()
				.filter(media::Column::SeriesId.eq(&keep.id))
				.count(&db)
				.await
				.expect("count"),
			2
		);
	}

	#[tokio::test]
	async fn merge_refuses_local_series_and_self_merges() {
		let db = ::tests::db::test_database().await;
		let provider = seed_series(&db, "prov", "Solo Leveling", Some("mock-en")).await;
		let local = seed_series(&db, "local", "Solo Leveling", None).await;

		let error = merge_series(&db, &provider.id, &local.id)
			.await
			.expect_err("a local series is never merged away");
		assert!(matches!(error, ProviderError::NotVirtual(_)), "{error:?}");
		assert!(merge_series(&db, &local.id, &provider.id).await.is_err());
		assert!(merge_series(&db, &provider.id, &provider.id).await.is_err());

		for id in [&provider.id, &local.id] {
			assert!(series::Entity::find_by_id(id)
				.one(&db)
				.await
				.expect("lookup")
				.is_some());
		}
	}
}
