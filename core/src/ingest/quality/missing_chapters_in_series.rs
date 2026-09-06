use std::{path::Path, sync::Arc};

use models::entity::{media, series};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect};
use serde_json::json;
use stump_api_types::settings::{SettingDefinition, SettingValues};
use stump_scanner::sequence::analyze_sequence;

use crate::ingest::contract::{
	BookSnapshot, QualityCheck, QualityCheckError, QualityCheckOutcome, QualityStatus,
};

use super::{disabled_outcome, enabled_setting, outcome, QUALITY_VERSION};

/// Upper bound on the gaps and unidentified names recorded in evidence, so a
/// badly named series cannot produce an unbounded report.
const MAX_EVIDENCE_MISSING: usize = 100;
const MAX_EVIDENCE_UNIDENTIFIED: usize = 25;

/// Flags series whose file names skip a chapter or volume number.
///
/// The staged book's own name is analysed together with the visible file names
/// of the series it joins, using the one workspace sequence parser
/// ([`stump_scanner::sequence`]): identifier vocabulary, omnibus ranges,
/// decimal chapters, `()`/`[]` metadata stripping, and the changing-token
/// fallback for names without an identifier. The check only reads media rows
/// and never mutates anything.
pub struct MissingChaptersInSeriesCheck {
	conn: Arc<DatabaseConnection>,
}

impl MissingChaptersInSeriesCheck {
	pub fn new(conn: Arc<DatabaseConnection>) -> Self {
		Self { conn }
	}

	/// The series the snapshot belongs to: for library rework the media row's
	/// own series, for a staged item the library series named by the drop
	/// folder the file was uploaded into.
	async fn resolve_series(
		&self,
		book: &BookSnapshot,
	) -> Result<Option<(String, String)>, QualityCheckError> {
		let own = media::Entity::find_by_id(book.drop_item_id.clone())
			.one(self.conn.as_ref())
			.await
			.map_err(|error| self.internal(error))?;
		if let Some(series_id) = own.and_then(|media| media.series_id) {
			let name = series::Entity::find_by_id(series_id.clone())
				.one(self.conn.as_ref())
				.await
				.map_err(|error| self.internal(error))?
				.map(|series| series.name)
				.unwrap_or_default();
			return Ok(Some((series_id, name)));
		}

		let Some(folder) = series_folder(&book.relative_path) else {
			return Ok(None);
		};
		Ok(series::Entity::find()
			.filter(series::Column::LibraryId.eq(book.library_id.clone()))
			.filter(series::Column::DeletedAt.is_null())
			.filter(series::Column::Name.eq(folder))
			.one(self.conn.as_ref())
			.await
			.map_err(|error| self.internal(error))?
			.map(|series| (series.id, series.name)))
	}

	async fn sibling_file_names(
		&self,
		book: &BookSnapshot,
		series_id: &str,
	) -> Result<Vec<String>, QualityCheckError> {
		let rows = media::Entity::find()
			.select_only()
			.columns(media::MediaIdentSelect::columns())
			.filter(media::Column::SeriesId.eq(series_id.to_owned()))
			.filter(media::Column::DeletedAt.is_null())
			// For library rework the snapshot's target id is the media row
			// under analysis: it contributes its staged name below instead of
			// its persisted one. For staged runs no media row carries a
			// drop-item id, so this is a no-op.
			.filter(media::Column::Id.ne(book.drop_item_id.clone()))
			.into_model::<media::MediaIdentSelect>()
			.all(self.conn.as_ref())
			.await
			.map_err(|error| self.internal(error))?;
		Ok(rows
			.into_iter()
			.map(|row| file_name(&row.path).to_string())
			.collect())
	}

	fn internal(&self, error: sea_orm::DbErr) -> QualityCheckError {
		QualityCheckError::Internal {
			check_id: self.id().to_string(),
			message: error.to_string(),
		}
	}
}

#[async_trait::async_trait]
impl QualityCheck for MissingChaptersInSeriesCheck {
	fn id(&self) -> &'static str {
		"missing_chapters_in_series"
	}

	fn name(&self) -> &'static str {
		"Missing chapters in series"
	}

	fn version(&self) -> &'static str {
		QUALITY_VERSION
	}

	fn weight(&self) -> u16 {
		5
	}

	fn settings(&self) -> &[SettingDefinition] {
		super::enabled_settings()
	}

	async fn run(
		&self,
		book: &BookSnapshot,
		settings: &SettingValues,
	) -> Result<QualityCheckOutcome, QualityCheckError> {
		if !enabled_setting(settings) {
			return Ok(disabled_outcome(self.id(), self.name()));
		}

		let Some((series_id, series_name)) = self.resolve_series(book).await? else {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::NotApplicable,
				0.0,
				json!({
					"reason": "no_series_context",
					"found_ranges": [],
					"missing": [],
					"unidentified": [],
				}),
			));
		};

		let mut names = self.sibling_file_names(book, &series_id).await?;
		names.push(staged_file_name(book).to_string());
		names.sort();

		let analysis = analyze_sequence(&names);
		let evidence = |reason: Option<&str>| {
			let missing = analysis
				.missing
				.iter()
				.take(MAX_EVIDENCE_MISSING)
				.copied()
				.collect::<Vec<_>>();
			let unidentified = analysis
				.unidentified
				.iter()
				.take(MAX_EVIDENCE_UNIDENTIFIED)
				.cloned()
				.collect::<Vec<_>>();
			json!({
				"reason": reason,
				"series_id": series_id,
				"series_name": series_name,
				"file_count": analysis.entries.len(),
				"identified_count": analysis.identified_count(),
				"kind": analysis.kind.map(|kind| kind.as_str()),
				"source": analysis.source.as_str(),
				"found_ranges": analysis.found_ranges_display(),
				"missing": missing,
				"unidentified": unidentified,
				"decimals": analysis
					.decimals
					.iter()
					.map(ToString::to_string)
					.collect::<Vec<_>>(),
				"truncated": analysis.missing.len() > MAX_EVIDENCE_MISSING
					|| analysis.unidentified.len() > MAX_EVIDENCE_UNIDENTIFIED,
				"span_exceeded": analysis.span_exceeded,
			})
		};

		// One numbered file cannot prove a gap, and a span too wide to
		// enumerate is a naming problem the filename check already reports.
		if analysis.identified_count() < 2 {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::NotApplicable,
				0.0,
				evidence(Some("insufficient_sequence")),
			));
		}
		if analysis.span_exceeded {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::NotApplicable,
				0.0,
				evidence(Some("span_exceeded")),
			));
		}

		let (status, normalized_score) = if analysis.has_gaps() {
			(QualityStatus::Warn, 0.5)
		} else {
			(QualityStatus::Pass, 1.0)
		};
		Ok(outcome(
			self.id(),
			self.name(),
			status,
			normalized_score,
			evidence(None),
		))
	}
}

/// The drop-folder directory that names the series a staged item joins.
/// `relative_path` is normalized (`/`-separated, no `..`) before staging.
fn series_folder(relative_path: &str) -> Option<String> {
	let (directories, _) = relative_path.rsplit_once('/')?;
	let folder = directories.rsplit('/').next()?.trim();
	(!folder.is_empty()).then(|| folder.to_string())
}

fn staged_file_name(book: &BookSnapshot) -> &str {
	if book.relative_path.is_empty() {
		&book.source_filename
	} else {
		file_name(&book.relative_path)
	}
}

fn file_name(path: &str) -> &str {
	Path::new(path)
		.file_name()
		.and_then(|name| name.to_str())
		.unwrap_or(path)
}
