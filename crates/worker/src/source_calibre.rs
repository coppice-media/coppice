//! Read-only Calibre `metadata.db` discovery for source workers.
//!
//! Calibre's SQLite catalog is authoritative for this source.  The adapter
//! never writes the database, never walks arbitrary files, and publishes only
//! format files that the catalog names and that resolve safely below the
//! configured root.  The resulting observations use the ordinary source
//! catalog/manifest path, so linking or materialising an item remains an
//! explicit server-side decision.

use std::{
	collections::{BTreeMap, BTreeSet},
	fs, io,
	path::{Component, Path, PathBuf},
};

use sea_orm::{
	ConnectionTrait, DatabaseBackend, DatabaseConnection, QueryResult,
	SqlxSqliteConnector, Statement,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::source_catalog::{
	modified_at_ms, quick_fingerprint, reject_symlink_ancestors, CatalogObservation,
	SourceRootConfig,
};

/// Calibre publishes an integer `user_version` in its schema. Version 27 is
/// the current release schema (v9.15.0); Calibre master adds
/// `upgrade_version_27` (`src/calibre/db/schema_upgrades.py`), which only
/// creates the viewer `book_storage` table and bumps `user_version` to 28. Both
/// are read; anything newer is rejected as an actionable unsupported schema
/// rather than guessing at columns.
pub const SUPPORTED_SCHEMA_VERSION: i64 = 28;
/// Calibre's current SQLite application id (`"cali"` big-endian). Zero is
/// accepted for older synthetic/legacy databases that did not set it.
pub const CALIBRE_APPLICATION_ID: i64 = 0x6361_6c69;

/// The source-root kind understood by this adapter.
pub const CALIBRE_ROOT_KIND: &str = "calibre";

const METADATA_DB_FILE: &str = "metadata.db";

/// One Calibre format row selected for import/serving. The `relative_path` is
/// built from the Calibre book directory and the lower-case format suffix;
/// `data.name` itself is only a basename.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalibreFormat {
	/// Calibre's format name, normalised to uppercase (`EPUB`, `PDF`, ...).
	pub format: String,
	/// Relative path below the configured Calibre root.
	pub relative_path: String,
	pub media_type: Option<String>,
	pub size: u64,
}

/// One book as represented by Calibre's catalog tables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibreBook {
	pub id: i64,
	pub title: String,
	pub authors: Vec<String>,
	pub series: Option<String>,
	pub series_index: Option<f64>,
	pub tags: Vec<String>,
	pub identifiers: BTreeMap<String, Vec<String>>,
	pub cover_relative_path: Option<String>,
	pub formats: Vec<CalibreFormat>,
}

/// Errors are deliberately specific so a bad/unsupported DB cannot silently
/// turn into an empty library.
#[derive(Debug, Error)]
pub enum CalibreSourceError {
	#[error("Calibre root `{root}` is not a regular non-symlink directory")]
	UnsafeRoot { root: PathBuf },
	#[error("Calibre metadata database is missing at `{path}`")]
	MissingDatabase { path: PathBuf },
	#[error("cannot open Calibre metadata database `{path}` read-only: {error}")]
	OpenDatabase { path: PathBuf, error: String },
	#[error("Calibre metadata database schema mismatch: {0}")]
	SchemaMismatch(String),
	#[error("unsupported Calibre metadata database schema user_version={version}; supported version is {supported}")]
	UnsupportedSchemaVersion { version: i64, supported: i64 },
	#[error("Calibre metadata query failed: {0}")]
	Query(String),
	#[error("Calibre book {book_id} has an invalid catalog path `{path}`: {reason}")]
	InvalidBookPath {
		book_id: i64,
		path: String,
		reason: String,
	},
	#[error(
		"Calibre book {book_id} format `{format}` has an invalid path `{path}`: {reason}"
	)]
	InvalidFormatPath {
		book_id: i64,
		format: String,
		path: String,
		reason: String,
	},
	#[error("Calibre book {book_id} format `{format}` is missing at `{path}`")]
	MissingFormat {
		book_id: i64,
		format: String,
		path: PathBuf,
	},
	#[error("Calibre book {book_id} contains duplicate format `{format}`")]
	DuplicateFormat { book_id: i64, format: String },
	#[error("Calibre root contains more than {max_entries} cataloged formats")]
	EntryLimit { max_entries: usize },
	#[error("Calibre metadata row for book {book_id} is invalid: {reason}")]
	InvalidBook { book_id: i64, reason: String },
	#[error("Calibre metadata database I/O failed: {0}")]
	Io(#[from] io::Error),
}

/// Discover all cataloged formats below `root` using one read-only SQLite
/// connection.  Results are ordered by Calibre book id and then format name.
pub async fn discover(
	root: &SourceRootConfig,
	max_entries: usize,
) -> Result<Vec<CatalogObservation>, CalibreSourceError> {
	if !root.kind.eq_ignore_ascii_case(CALIBRE_ROOT_KIND) {
		return Err(CalibreSourceError::SchemaMismatch(format!(
			"source root kind must be `{CALIBRE_ROOT_KIND}` (got `{}`)",
			root.kind
		)));
	}
	reject_symlink_ancestors(&root.path).map_err(|_| CalibreSourceError::UnsafeRoot {
		root: root.path.clone(),
	})?;
	let root_metadata = fs::symlink_metadata(&root.path)?;
	if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
		return Err(CalibreSourceError::UnsafeRoot {
			root: root.path.clone(),
		});
	}
	let db_path = root.path.join(METADATA_DB_FILE);
	let db_metadata = match fs::symlink_metadata(&db_path) {
		Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
			return Err(CalibreSourceError::MissingDatabase { path: db_path });
		},
		Ok(metadata) => metadata,
		Err(error) if error.kind() == io::ErrorKind::NotFound => {
			return Err(CalibreSourceError::MissingDatabase { path: db_path });
		},
		Err(error) => return Err(CalibreSourceError::Io(error)),
	};
	if db_metadata.len() == 0 {
		return Err(CalibreSourceError::OpenDatabase {
			path: db_path,
			error: "database file is empty".into(),
		});
	}

	let options = sea_orm::sqlx::sqlite::SqliteConnectOptions::new()
		.filename(&db_path)
		.read_only(true)
		.create_if_missing(false);
	let pool = sea_orm::sqlx::sqlite::SqlitePoolOptions::new()
		.max_connections(1)
		.min_connections(1)
		.connect_with(options)
		.await
		.map_err(|error| CalibreSourceError::OpenDatabase {
			path: db_path.clone(),
			error: error.to_string(),
		})?;
	let db = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);
	let result = discover_connection(&db, &root.path, max_entries).await;
	let close_result = db.close().await;
	if let Err(error) = close_result {
		if result.is_ok() {
			return Err(CalibreSourceError::Query(format!(
				"failed to close read-only metadata database: {error}"
			)));
		}
	}
	result
}

async fn discover_connection(
	db: &DatabaseConnection,
	root: &Path,
	max_entries: usize,
) -> Result<Vec<CatalogObservation>, CalibreSourceError> {
	let application_id = pragma_i64(db, "application_id").await?;
	if application_id != 0 && application_id != CALIBRE_APPLICATION_ID {
		return Err(CalibreSourceError::SchemaMismatch(format!(
			"SQLite application_id {application_id:#x} is not Calibre's {CALIBRE_APPLICATION_ID:#x}"
		)));
	}
	let version = pragma_i64(db, "user_version").await?;
	if version > SUPPORTED_SCHEMA_VERSION {
		return Err(CalibreSourceError::UnsupportedSchemaVersion {
			version,
			supported: SUPPORTED_SCHEMA_VERSION,
		});
	}
	validate_schema(db).await?;

	let optional_books = table_columns(db, "books").await?;
	let has_cover = optional_books.contains("has_cover");
	let series_index = optional_books.contains("series_index");
	let select = format!(
		"SELECT id, title, path, {} AS has_cover, {} AS series_index FROM books ORDER BY id ASC",
		if has_cover { "has_cover" } else { "0" },
		if series_index { "series_index" } else { "NULL" },
	);
	let rows = db
		.query_all(Statement::from_string(DatabaseBackend::Sqlite, select))
		.await
		.map_err(query_error)?;

	let mut books = BTreeMap::<i64, CalibreBook>::new();
	let mut book_paths = BTreeMap::<i64, String>::new();
	for row in rows {
		let book_id = row_i64(&row, "id")?;
		let title = row_string(&row, "title")?;
		let path = row_string(&row, "path")?;
		validate_catalog_path(book_id, &path)?;
		let cover = row_i64_or_zero(&row, "has_cover")? != 0;
		let index = row_optional_f64(&row, "series_index")?;
		book_paths.insert(book_id, path.clone());
		books.insert(
			book_id,
			CalibreBook {
				id: book_id,
				title,
				authors: Vec::new(),
				series: None,
				series_index: index,
				tags: Vec::new(),
				identifiers: BTreeMap::new(),
				cover_relative_path: cover.then(|| format!("{path}/cover.jpg")),
				formats: Vec::new(),
			},
		);
	}
	if books.is_empty() {
		return Ok(Vec::new());
	}

	let authors =
		relation_names(db, "authors", "books_authors_link", "author", "name").await?;
	let tags = relation_names(db, "tags", "books_tags_link", "tag", "name").await?;
	for (book_id, names) in authors {
		if let Some(book) = books.get_mut(&book_id) {
			book.authors = names;
		}
	}
	for (book_id, names) in tags {
		if let Some(book) = books.get_mut(&book_id) {
			book.tags = names;
		}
	}
	let series =
		relation_names(db, "series", "books_series_link", "series", "name").await?;
	for (book_id, names) in series {
		if let Some(book) = books.get_mut(&book_id) {
			book.series = names.into_iter().next();
		}
	}
	let identifiers = identifier_values(db).await?;
	for (book_id, values) in identifiers {
		if let Some(book) = books.get_mut(&book_id) {
			book.identifiers = values;
		}
	}

	let format_rows = db
		.query_all(Statement::from_string(
			DatabaseBackend::Sqlite,
			"SELECT book, format, name, uncompressed_size FROM data ORDER BY book ASC, lower(format) ASC, name ASC",
		))
		.await
		.map_err(query_error)?
		.iter()
		.filter_map(|row| {
			let book_id = match row_i64(row, "book") {
				Ok(book_id) => book_id,
				Err(error) => return Some(Err(error)),
			};
			// A `data` row for a book the catalog does not list is not a
			// format of anything; it is skipped, as it always was.
			books.contains_key(&book_id).then(|| {
				Ok(FormatRow {
					book_id,
					format: row_string(row, "format")?,
					name: row_string(row, "name")?,
					uncompressed_size: row_i64(row, "uncompressed_size")?,
				})
			})
		})
		.collect::<Result<Vec<_>, CalibreSourceError>>()?;
	// Everything below touches the filesystem (symlink checks, metadata, and
	// the sampled fingerprint read of every format file).  On a large or
	// network-mounted library that is seconds of blocking work, so it runs on
	// the blocking pool rather than the runtime thread that also services
	// control frames and grants.  The caller's per-root permit bounds it.
	let root = root.to_path_buf();
	tokio::task::spawn_blocking(move || {
		observe_formats(books, &book_paths, format_rows, &root, max_entries)
	})
	.await
	.map_err(|error| {
		CalibreSourceError::Query(format!(
			"Calibre format discovery task failed: {error}"
		))
	})?
}

/// One decoded `data` row, owned so it can cross into the blocking task.
struct FormatRow {
	book_id: i64,
	format: String,
	name: String,
	uncompressed_size: i64,
}

/// Resolve every catalogued format below `root` and fingerprint it.  Pure
/// filesystem work: no database handle crosses into this function.
fn observe_formats(
	mut books: BTreeMap<i64, CalibreBook>,
	book_paths: &BTreeMap<i64, String>,
	format_rows: Vec<FormatRow>,
	root: &Path,
	max_entries: usize,
) -> Result<Vec<CatalogObservation>, CalibreSourceError> {
	let mut observations = Vec::new();
	let mut seen_relative_paths = BTreeSet::new();
	for row in format_rows {
		let book_id = row.book_id;
		let Some(book) = books.get_mut(&book_id) else {
			continue;
		};
		let format = normalise_format(&row.format).ok_or_else(|| {
			CalibreSourceError::InvalidBook {
				book_id,
				reason: "format name is empty or contains a path separator".into(),
			}
		})?;
		if book
			.formats
			.iter()
			.any(|existing| existing.format.eq_ignore_ascii_case(&format))
		{
			return Err(CalibreSourceError::DuplicateFormat { book_id, format });
		}
		let name = row.name;
		if row.uncompressed_size < 0 {
			return Err(CalibreSourceError::InvalidBook {
				book_id,
				reason: "data.uncompressed_size is negative".into(),
			});
		}
		let catalog_size = row.uncompressed_size as u64;
		let relative = safe_format_path(book_id, &book_paths[&book_id], &format, &name)?;
		if !seen_relative_paths.insert(relative.clone()) {
			return Err(CalibreSourceError::InvalidBook {
				book_id,
				reason: format!("duplicate format path `{relative}`"),
			});
		}
		let path = root.join(&relative);
		reject_symlink_ancestors(&path).map_err(|error| {
			if error.kind() == io::ErrorKind::NotFound {
				CalibreSourceError::MissingFormat {
					book_id,
					format: format.clone(),
					path: path.clone(),
				}
			} else {
				CalibreSourceError::InvalidFormatPath {
					book_id,
					format: format.clone(),
					path: name.clone(),
					reason: "format path has a symlink ancestor".into(),
				}
			}
		})?;
		let metadata = fs::symlink_metadata(&path).map_err(|error| {
			if error.kind() == io::ErrorKind::NotFound {
				CalibreSourceError::MissingFormat {
					book_id,
					format: format.clone(),
					path: path.clone(),
				}
			} else {
				CalibreSourceError::Io(error)
			}
		})?;
		if metadata.file_type().is_symlink() || !metadata.is_file() {
			return Err(CalibreSourceError::MissingFormat {
				book_id,
				format: format.clone(),
				path,
			});
		}
		let media_type = media_type_for_format(&format);
		if catalog_size != metadata.len() {
			tracing::debug!(
				book_id,
				format = %format,
				catalog_size,
				filesystem_size = metadata.len(),
				"Calibre data size differs; filesystem size is authoritative"
			);
		}
		let modified_at_ms = modified_at_ms(&metadata);
		let quick =
			quick_fingerprint(&root.join(&relative), metadata.len(), modified_at_ms)?;
		let format_record = CalibreFormat {
			format: format.clone(),
			relative_path: relative.clone(),
			media_type: media_type.clone(),
			size: metadata.len(),
		};
		book.formats.push(format_record);
		if observations.len() >= max_entries {
			return Err(CalibreSourceError::EntryLimit { max_entries });
		}
		let metadata_value = metadata_value(book, &format, &relative, catalog_size);
		observations.push(CatalogObservation {
			absolute_path: root.join(&relative),
			relative_path: relative,
			size: metadata.len(),
			modified_at_ms,
			quick_fingerprint: quick,
			media_type,
			metadata: Some(metadata_value),
		});
	}
	observations.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
	Ok(observations)
}

async fn validate_schema(db: &DatabaseConnection) -> Result<(), CalibreSourceError> {
	const REQUIRED: &[(&str, &[&str])] = &[
		("books", &["id", "title", "path"]),
		("authors", &["id", "name"]),
		("books_authors_link", &["book", "author"]),
		("series", &["id", "name"]),
		("books_series_link", &["book", "series"]),
		("data", &["book", "format", "name", "uncompressed_size"]),
		("tags", &["id", "name"]),
		("books_tags_link", &["book", "tag"]),
		("identifiers", &["book", "type", "val"]),
	];
	for (table, columns) in REQUIRED {
		let found = table_columns(db, table).await?;
		if found.is_empty() {
			return Err(CalibreSourceError::SchemaMismatch(format!(
				"required table `{table}` is missing"
			)));
		}
		if let Some(missing) = columns.iter().find(|column| !found.contains(**column)) {
			return Err(CalibreSourceError::SchemaMismatch(format!(
				"table `{table}` is missing required column `{missing}`; this may be a newer or unsupported Calibre schema"
			)));
		}
	}
	Ok(())
}

async fn table_columns(
	db: &DatabaseConnection,
	table: &str,
) -> Result<BTreeSet<String>, CalibreSourceError> {
	let sql = format!("PRAGMA table_info({table})");
	let rows = db
		.query_all(Statement::from_string(DatabaseBackend::Sqlite, sql))
		.await
		.map_err(query_error)?;
	let mut columns = BTreeSet::new();
	for row in rows {
		columns.insert(row_string(&row, "name")?);
	}
	Ok(columns)
}

async fn pragma_i64(
	db: &DatabaseConnection,
	pragma: &str,
) -> Result<i64, CalibreSourceError> {
	let row = db
		.query_one(Statement::from_string(
			DatabaseBackend::Sqlite,
			format!("PRAGMA {pragma}"),
		))
		.await
		.map_err(query_error)?
		.ok_or_else(|| {
			CalibreSourceError::Query(format!("PRAGMA {pragma} returned no row"))
		})?;
	row.try_get::<i64>("", pragma)
		.map_err(|error| CalibreSourceError::Query(format!("PRAGMA {pragma}: {error}")))
}

async fn relation_names(
	db: &DatabaseConnection,
	value_table: &str,
	link_table: &str,
	value_column: &str,
	name_column: &str,
) -> Result<BTreeMap<i64, Vec<String>>, CalibreSourceError> {
	let sql = format!(
		"SELECT l.book AS book, v.{name_column} AS name FROM {link_table} l JOIN {value_table} v ON v.id = l.{value_column} ORDER BY l.book ASC, v.{name_column} COLLATE NOCASE ASC, v.id ASC"
	);
	let rows = db
		.query_all(Statement::from_string(DatabaseBackend::Sqlite, sql))
		.await
		.map_err(query_error)?;
	let mut result = BTreeMap::new();
	for row in rows {
		result
			.entry(row_i64(&row, "book")?)
			.or_insert_with(Vec::new)
			.push(row_string(&row, "name")?);
	}
	Ok(result)
}

async fn identifier_values(
	db: &DatabaseConnection,
) -> Result<BTreeMap<i64, BTreeMap<String, Vec<String>>>, CalibreSourceError> {
	let rows = db
		.query_all(Statement::from_string(
			DatabaseBackend::Sqlite,
			"SELECT book, type, val FROM identifiers ORDER BY book ASC, type COLLATE NOCASE ASC, val ASC",
		))
		.await
		.map_err(query_error)?;
	let mut result = BTreeMap::new();
	for row in rows {
		let book = row_i64(&row, "book")?;
		let kind = row_string(&row, "type")?;
		let value = row_string(&row, "val")?;
		result
			.entry(book)
			.or_insert_with(BTreeMap::new)
			.entry(kind)
			.or_insert_with(Vec::new)
			.push(value);
	}
	Ok(result)
}

fn metadata_value(
	book: &CalibreBook,
	format: &str,
	relative_path: &str,
	catalog_size: u64,
) -> serde_json::Value {
	json!({
		"source": "calibre",
		"book_id": book.id,
		"title": &book.title,
		"authors": &book.authors,
		"series": &book.series,
		"series_index": book.series_index,
		"tags": &book.tags,
		"identifiers": &book.identifiers,
		"cover_relative_path": &book.cover_relative_path,
		"format": format,
		"relative_path": relative_path,
		"catalog_uncompressed_size": catalog_size,
	})
}

fn validate_catalog_path(book_id: i64, path: &str) -> Result<(), CalibreSourceError> {
	if path.is_empty() || path.contains('\0') {
		return Err(CalibreSourceError::InvalidBookPath {
			book_id,
			path: path.into(),
			reason: "path is empty or contains NUL".into(),
		});
	}
	if Path::new(path).is_absolute()
		|| path.contains('\\')
		|| path.split('/').any(str::is_empty)
	{
		return Err(CalibreSourceError::InvalidBookPath {
			book_id,
			path: path.into(),
			reason: "path must be relative and use single POSIX separators".into(),
		});
	}
	for component in Path::new(path).components() {
		if !matches!(component, Component::Normal(_)) {
			return Err(CalibreSourceError::InvalidBookPath {
				book_id,
				path: path.into(),
				reason: "path contains `.`/`..`/root components".into(),
			});
		}
	}
	Ok(())
}

fn safe_format_path(
	book_id: i64,
	book_path: &str,
	format: &str,
	name: &str,
) -> Result<String, CalibreSourceError> {
	validate_catalog_path(book_id, book_path)?;
	if name.is_empty()
		|| name.contains('\0')
		|| name.contains('/')
		|| name.contains('\\')
		|| name.chars().any(char::is_control)
	{
		return Err(CalibreSourceError::InvalidFormatPath {
			book_id,
			format: format.into(),
			path: name.into(),
			reason: "data.name must be one safe basename without path separators".into(),
		});
	}
	let name_path = Path::new(name);
	let mut components = name_path.components();
	if !matches!(components.next(), Some(Component::Normal(_)))
		|| components.next().is_some()
	{
		return Err(CalibreSourceError::InvalidFormatPath {
			book_id,
			format: format.into(),
			path: name.into(),
			reason: "data.name must be one safe basename without `.` or `..`".into(),
		});
	}
	let filename = format!("{name}.{}", format.to_ascii_lowercase());
	let relative = Path::new(book_path).join(filename);
	let Some(relative) = relative.to_str() else {
		return Err(CalibreSourceError::InvalidFormatPath {
			book_id,
			format: format.into(),
			path: name.into(),
			reason: "path is not valid UTF-8".into(),
		});
	};
	Ok(relative.replace(std::path::MAIN_SEPARATOR, "/"))
}

fn normalise_format(value: &str) -> Option<String> {
	let trimmed = value.trim();
	if trimmed.is_empty()
		|| trimmed.contains('/')
		|| trimmed.contains('\\')
		|| trimmed.chars().any(char::is_control)
	{
		None
	} else {
		Some(trimmed.to_ascii_uppercase())
	}
}

fn media_type_for_format(format: &str) -> Option<String> {
	let extension = format.to_ascii_lowercase();
	Some(
		match extension.as_str() {
			"epub" => "application/epub+zip",
			"pdf" => "application/pdf",
			"mobi" | "azw" | "azw3" => "application/x-mobipocket-ebook",
			"cbz" => "application/vnd.comicbook+zip",
			"cbr" => "application/vnd.comicbook-rar",
			"txt" => "text/plain",
			"rtf" => "application/rtf",
			"html" | "htm" => "text/html",
			_ => return None,
		}
		.into(),
	)
}

fn query_error(error: sea_orm::DbErr) -> CalibreSourceError {
	CalibreSourceError::Query(error.to_string())
}

fn row_string(row: &QueryResult, column: &str) -> Result<String, CalibreSourceError> {
	row.try_get::<String>("", column)
		.map_err(|error| CalibreSourceError::Query(format!("column `{column}`: {error}")))
}

fn row_i64(row: &QueryResult, column: &str) -> Result<i64, CalibreSourceError> {
	row.try_get::<i64>("", column)
		.map_err(|error| CalibreSourceError::Query(format!("column `{column}`: {error}")))
}

fn row_i64_or_zero(row: &QueryResult, column: &str) -> Result<i64, CalibreSourceError> {
	row.try_get::<Option<i64>>("", column)
		.map(|value| value.unwrap_or_default())
		.map_err(|error| CalibreSourceError::Query(format!("column `{column}`: {error}")))
}

fn row_optional_f64(
	row: &QueryResult,
	column: &str,
) -> Result<Option<f64>, CalibreSourceError> {
	row.try_get::<Option<f64>>("", column)
		.map_err(|error| CalibreSourceError::Query(format!("column `{column}`: {error}")))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rejects_traversal_and_absolute_calibre_paths() {
		assert!(validate_catalog_path(1, "Author/Book").is_ok());
		assert!(validate_catalog_path(1, "../outside").is_err());
		assert!(safe_format_path(1, "Author/Book", "EPUB", "Book").is_ok());
		assert_eq!(
			safe_format_path(1, "Author/Book", "EPUB", "Book").unwrap(),
			"Author/Book/Book.epub"
		);
		assert!(safe_format_path(1, "Author/Book", "EPUB", "../Book").is_err());
		assert!(safe_format_path(1, "Author/Book", "EPUB", "/tmp/Book").is_err());
	}

	#[test]
	fn normalises_formats_and_media_types_deterministically() {
		assert_eq!(normalise_format(" epub "), Some("EPUB".to_owned()));
		assert_eq!(normalise_format("../epub"), None);
		assert_eq!(
			media_type_for_format("EPUB").as_deref(),
			Some("application/epub+zip")
		);
		assert_eq!(
			media_type_for_format("MOBI").as_deref(),
			Some("application/x-mobipocket-ebook")
		);
	}

	#[test]
	fn metadata_contains_catalog_identity_not_absolute_paths() {
		let book = CalibreBook {
			id: 7,
			title: "A Book".into(),
			authors: vec!["An Author".into()],
			series: Some("A Series".into()),
			series_index: Some(1.0),
			tags: vec!["fiction".into()],
			identifiers: BTreeMap::new(),
			cover_relative_path: Some("Author/A Book/cover.jpg".into()),
			formats: Vec::new(),
		};
		let value = metadata_value(&book, "EPUB", "Author/A Book/A Book.epub", 12);
		assert_eq!(value["source"], "calibre");
		assert_eq!(value["book_id"], 7);
		assert!(value.to_string().contains("Author/A Book/A Book.epub"));
		assert_eq!(value["catalog_uncompressed_size"], 12);
		assert!(!value.to_string().contains("/home/"));
	}

	/// Writes a minimal Calibre library (one EPUB) below `root_path` with the
	/// given `user_version` plus any `extra_sql` statements, returning the root
	/// config `discover` expects.
	async fn write_calibre_fixture(
		root_path: &Path,
		user_version: i64,
		extra_sql: &[&str],
	) -> SourceRootConfig {
		use crate::source_protocol::SourceTransport;

		let book_dir = root_path.join("Author").join("Book");
		std::fs::create_dir_all(&book_dir).unwrap();
		std::fs::write(book_dir.join("Book.epub"), b"fixture epub").unwrap();

		let db_path = root_path.join("metadata.db");
		let options = sea_orm::sqlx::sqlite::SqliteConnectOptions::new()
			.filename(&db_path)
			.create_if_missing(true);
		let pool = sea_orm::sqlx::sqlite::SqlitePoolOptions::new()
			.max_connections(1)
			.min_connections(1)
			.connect_with(options)
			.await
			.unwrap();
		let db = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);
		let user_version = format!("PRAGMA user_version = {user_version}");
		for sql in [
			"CREATE TABLE books (id INTEGER PRIMARY KEY, title TEXT NOT NULL, path TEXT NOT NULL, has_cover INTEGER NOT NULL DEFAULT 0, series_index REAL)",
			"CREATE TABLE authors (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
			"PRAGMA application_id = 0x63616c69",
			user_version.as_str(),
			"CREATE TABLE books_authors_link (book INTEGER NOT NULL, author INTEGER NOT NULL)",
			"CREATE TABLE series (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
			"CREATE TABLE books_series_link (book INTEGER NOT NULL, series INTEGER NOT NULL)",
			"CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
			"CREATE TABLE books_tags_link (book INTEGER NOT NULL, tag INTEGER NOT NULL)",
			"CREATE TABLE identifiers (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, type TEXT NOT NULL, val TEXT NOT NULL)",
			"CREATE TABLE data (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, format TEXT NOT NULL, uncompressed_size INTEGER NOT NULL, name TEXT NOT NULL)",
			"INSERT INTO books (id, title, path, has_cover, series_index) VALUES (1, 'Book', 'Author/Book', 0, 1.0)",
			"INSERT INTO authors (id, name) VALUES (1, 'Author')",
			"INSERT INTO books_authors_link (book, author) VALUES (1, 1)",
			"INSERT INTO series (id, name) VALUES (1, 'Series')",
			"INSERT INTO books_series_link (book, series) VALUES (1, 1)",
			"INSERT INTO tags (id, name) VALUES (1, 'fiction')",
			"INSERT INTO books_tags_link (book, tag) VALUES (1, 1)",
			"INSERT INTO identifiers (id, book, type, val) VALUES (1, 1, 'isbn', 'fixture-1')",
			"INSERT INTO data (id, book, format, uncompressed_size, name) VALUES (1, 1, 'EPUB', 12, 'Book')",
		]
		.into_iter()
		.chain(extra_sql.iter().copied())
		{
			db.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
				.await
				.unwrap();
		}
		db.close().await.unwrap();

		SourceRootConfig {
			root_id: "calibre".into(),
			label: "Calibre".into(),
			kind: CALIBRE_ROOT_KIND.into(),
			privacy_mode: "catalog".into(),
			path: root_path.to_path_buf(),
			transport: SourceTransport::Tunnel,
			direct_base_url: None,
		}
	}

	/// THROWAWAY PROOF (removed before hand-off): the per-format loop stalls
	/// the runtime thread when run inline, and does not when `discover` runs
	/// it through `spawn_blocking`.
	#[tokio::test(flavor = "current_thread")]
	async fn throwaway_blocking_proof() {
		use std::sync::atomic::{AtomicU64, Ordering};
		use std::sync::Arc;
		use std::time::{Duration, Instant};
		use tempfile::tempdir;

		let dir = tempdir().unwrap();
		let root_path = dir.path().join("calibre");
		let mut extra = Vec::new();
		for id in 2..4000_i64 {
			let book_dir = root_path.join("Author").join(format!("Book{id}"));
			std::fs::create_dir_all(&book_dir).unwrap();
			std::fs::write(book_dir.join(format!("Book{id}.epub")), vec![b'x'; 65_536])
				.unwrap();
			extra.push(format!(
				"INSERT INTO books (id, title, path, has_cover, series_index) VALUES ({id}, 'Book{id}', 'Author/Book{id}', 0, 1.0)"
			));
			extra.push(format!(
				"INSERT INTO data (id, book, format, uncompressed_size, name) VALUES ({id}, {id}, 'EPUB', 65536, 'Book{id}')"
			));
		}
		let extra_refs: Vec<&str> = extra.iter().map(String::as_str).collect();
		let root = write_calibre_fixture(&root_path, 27, &extra_refs).await;

		// Heartbeat: records the longest gap between two consecutive ticks.
		let max_gap = Arc::new(AtomicU64::new(0));
		let heartbeat = {
			let max_gap = max_gap.clone();
			tokio::spawn(async move {
				let mut last = Instant::now();
				loop {
					tokio::time::sleep(Duration::from_millis(1)).await;
					let gap = last.elapsed().as_millis() as u64;
					max_gap.fetch_max(gap, Ordering::Relaxed);
					last = Instant::now();
				}
			})
		};

		// Warm up the page cache so both passes read the same bytes.
		let _ = discover(&root, 10_000).await.unwrap();

		max_gap.store(0, Ordering::Relaxed);
		tokio::time::sleep(Duration::from_millis(20)).await;
		let started = Instant::now();
		let observations = discover(&root, 10_000).await.unwrap();
		let blocking_elapsed = started.elapsed();
		tokio::time::sleep(Duration::from_millis(20)).await;
		let blocking_gap = max_gap.load(Ordering::Relaxed);

		// The old shape: the same loop inline on the runtime thread.
		let books = observations
			.iter()
			.map(|observation| {
				let id = observation.metadata.as_ref().unwrap()["book_id"]
					.as_i64()
					.unwrap();
				(
					id,
					CalibreBook {
						id,
						title: String::new(),
						authors: Vec::new(),
						series: None,
						series_index: None,
						tags: Vec::new(),
						identifiers: BTreeMap::new(),
						cover_relative_path: None,
						formats: Vec::new(),
					},
				)
			})
			.collect::<BTreeMap<_, _>>();
		let book_paths = observations
			.iter()
			.map(|observation| {
				let id = observation.metadata.as_ref().unwrap()["book_id"]
					.as_i64()
					.unwrap();
				let path = observation
					.relative_path
					.rsplit_once('/')
					.unwrap()
					.0
					.to_owned();
				(id, path)
			})
			.collect::<BTreeMap<_, _>>();
		let rows = observations
			.iter()
			.map(|observation| FormatRow {
				book_id: observation.metadata.as_ref().unwrap()["book_id"]
					.as_i64()
					.unwrap(),
				format: "EPUB".into(),
				name: observation
					.relative_path
					.rsplit_once('/')
					.unwrap()
					.1
					.trim_end_matches(".epub")
					.to_owned(),
				uncompressed_size: 65_536,
			})
			.collect::<Vec<_>>();
		max_gap.store(0, Ordering::Relaxed);
		tokio::time::sleep(Duration::from_millis(20)).await;
		let started = Instant::now();
		let inline =
			observe_formats(books, &book_paths, rows, &root_path, 10_000).unwrap();
		let inline_elapsed = started.elapsed();
		tokio::time::sleep(Duration::from_millis(20)).await;
		let inline_gap = max_gap.load(Ordering::Relaxed);
		heartbeat.abort();

		eprintln!(
			"PROOF formats={} discover(spawn_blocking): elapsed={blocking_elapsed:?} max_heartbeat_gap={blocking_gap}ms | inline loop: elapsed={inline_elapsed:?} max_heartbeat_gap={inline_gap}ms",
			inline.len()
		);
		assert_eq!(inline.len(), observations.len());
	}

	#[tokio::test]
	async fn discovers_one_format_from_calibre_data_table() {
		use tempfile::tempdir;

		let dir = tempdir().unwrap();
		let root = write_calibre_fixture(&dir.path().join("calibre"), 27, &[]).await;
		let observations = discover(&root, 10).await.unwrap();
		assert_eq!(observations.len(), 1);
		assert_eq!(observations[0].relative_path, "Author/Book/Book.epub");
		assert_eq!(observations[0].size, b"fixture epub".len() as u64);
		let metadata = observations[0].metadata.as_ref().unwrap();
		assert_eq!(metadata["title"], "Book");
		assert_eq!(metadata["authors"][0], "Author");
		assert_eq!(metadata["series"], "Series");
		assert_eq!(metadata["format"], "EPUB");
	}

	/// Calibre master's `upgrade_version_27` only adds the viewer
	/// `book_storage` table (schema 28); it must read like schema 27, while
	/// anything newer stays an actionable rejection.
	#[tokio::test]
	async fn accepts_schema_28_book_storage_and_rejects_newer() {
		use tempfile::tempdir;

		let dir = tempdir().unwrap();
		let root = write_calibre_fixture(
			&dir.path().join("v28"),
			28,
			&[
				"CREATE TABLE book_storage (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, format TEXT NOT NULL COLLATE NOCASE, user_type TEXT NOT NULL, user TEXT NOT NULL, timestamp REAL NOT NULL, data TEXT NOT NULL DEFAULT '{}', UNIQUE(book, format, user_type, user), FOREIGN KEY (book) REFERENCES books(id) ON DELETE CASCADE)",
				"INSERT INTO book_storage (book, format, user_type, user, timestamp, data) VALUES (1, 'EPUB', 'local', 'viewer', 1.0, '{}')",
			],
		)
		.await;
		let observations = discover(&root, 10).await.unwrap();
		assert_eq!(observations.len(), 1);
		assert_eq!(observations[0].relative_path, "Author/Book/Book.epub");

		let root = write_calibre_fixture(&dir.path().join("v29"), 29, &[]).await;
		match discover(&root, 10).await {
			Err(CalibreSourceError::UnsupportedSchemaVersion { version, supported }) => {
				assert_eq!(version, 29);
				assert_eq!(supported, SUPPORTED_SCHEMA_VERSION);
			},
			other => panic!("expected unsupported schema rejection, got {other:?}"),
		}
	}
}
