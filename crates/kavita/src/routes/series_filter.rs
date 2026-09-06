//! Translate `SeriesFilterV2Dto` into SeaORM conditions and ordering
//! (`Kavita.Database/Extensions/Filters/SeriesFilter.cs` and
//! `SeriesRepository.CreateFilteredSearchQueryableV2`).
//!
//! A Kavita series is backed by a Stump series row in Manga/Comic libraries
//! and by a media row in Book/LightNovel libraries, so every statement and
//! sort is built for both [`Target`]s: the series form reads `series` and
//! `series_metadata` (reaching media through subqueries), the book form reads
//! `media` and `media_metadata` on a query that already joins the media's
//! `series`. The listing runs both under one `UNION ALL`.

use std::collections::HashSet;

use models::entity::{
	media, media_metadata, media_tag, series, series_metadata, series_tag,
	user::AuthUser,
};
use sea_orm::{
	prelude::*,
	sea_query::{Expr, Func, Order, Query, SimpleExpr},
	Condition, DatabaseConnection, Statement, Value,
};

use crate::{
	dto::{AgeRating, MangaFormat, PublicationStatus},
	errors::{APIError, APIResult},
	filter::{
		FilterCombination, FilterComparison, SeriesFilterField, SeriesFilterV2Dto,
		SeriesSortField,
	},
	ids::{IdKind, KavitaIds},
	mapper::name_id,
};

/// The parts of a filter that need reading progress and therefore run in
/// memory after the SQL stage.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProgressFilters {
	pub statements: Vec<(FilterComparison, f32)>,
}

impl ProgressFilters {
	/// Kavita computes `Sum(pagesRead / pages) * 100` per series.
	pub fn matches(&self, percentage: f32, combination: FilterCombination) -> bool {
		if self.statements.is_empty() {
			return true;
		}
		let mut results = self.statements.iter().map(|(comparison, value)| {
			let tolerance = 0.001;
			match comparison {
				FilterComparison::Equal => (percentage - value).abs() < tolerance,
				FilterComparison::NotEqual => (percentage - value).abs() >= tolerance,
				FilterComparison::GreaterThan => percentage > *value,
				FilterComparison::GreaterThanEqual => percentage >= *value,
				FilterComparison::LessThan => percentage < *value,
				FilterComparison::LessThanEqual => percentage <= *value,
				_ => true,
			}
		});
		match combination {
			FilterCombination::And => results.all(|matched| matched),
			FilterCombination::Or => results.any(|matched| matched),
		}
	}
}

/// Where a Kavita series' columns live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
	/// A Stump series row with its `series_metadata` (Manga/Comic libraries).
	Series,
	/// A media row with its `media_metadata`, joined to its `series`
	/// (Book/LightNovel libraries, where every file is its own series).
	Book,
}

/// One sort criterion with its expression for each target.
#[derive(Debug, Clone)]
pub(crate) struct SortKey {
	pub series: SimpleExpr,
	pub book: SimpleExpr,
	pub order: Order,
}

impl SortKey {
	pub fn expr(&self, target: Target) -> &SimpleExpr {
		match target {
			Target::Series => &self.series,
			Target::Book => &self.book,
		}
	}

	/// Kavita's `Created`: when the series row (or the book's file) was added.
	pub fn created(order: Order) -> Self {
		Self {
			series: series_col(series::Column::CreatedAt).into(),
			book: media_col(media::Column::CreatedAt).into(),
			order,
		}
	}
}

/// The executable form of a filter.
#[derive(Debug, Clone)]
pub(crate) struct FilterPlan {
	/// The SQL condition over Stump series rows ([`Target::Series`]).
	pub condition: Condition,
	/// The SQL condition over media rows of Book libraries ([`Target::Book`]).
	pub book_condition: Condition,
	pub order: Vec<SortKey>,
	pub progress: ProgressFilters,
	/// Sorts that need reading progress (`ReadProgress`, `UnreadChapterCount`).
	pub sort_by_progress: Option<ProgressSort>,
	pub combination: FilterCombination,
	pub limit_to: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressSort {
	ReadProgress { ascending: bool },
	UnreadCount { ascending: bool },
}

impl FilterPlan {
	pub fn needs_memory_pass(&self) -> bool {
		!self.progress.statements.is_empty()
			|| self.sort_by_progress.is_some()
			|| self.limit_to > 0
	}
}

/// The group statements of a filter, collected for both targets and combined
/// with the filter's `combination` at the end.
#[derive(Default)]
struct Groups {
	series: Vec<SimpleExpr>,
	book: Vec<SimpleExpr>,
}

impl Groups {
	fn add(&mut self, build: impl Fn(Target) -> SimpleExpr) {
		self.series.push(build(Target::Series));
		self.book.push(build(Target::Book));
	}

	fn add_both(&mut self, expr: SimpleExpr) {
		self.series.push(expr.clone());
		self.book.push(expr);
	}

	fn is_empty(&self) -> bool {
		self.series.is_empty()
	}

	fn condition(terms: Vec<SimpleExpr>, combination: FilterCombination) -> Condition {
		let group = match combination {
			FilterCombination::And => Condition::all(),
			FilterCombination::Or => Condition::any(),
		};
		terms.into_iter().fold(group, Condition::add)
	}
}

fn bad_request(message: impl Into<String>) -> APIError {
	APIError::BadRequest(message.into())
}

fn validate(
	comparison: FilterComparison,
	allowed: &[FilterComparison],
	field: &str,
) -> APIResult<()> {
	if allowed.contains(&comparison) {
		Ok(())
	} else {
		Err(bad_request(format!(
			"{comparison:?} is not applicable for {field}"
		)))
	}
}

const STRING: &[FilterComparison] = &[
	FilterComparison::Equal,
	FilterComparison::NotEqual,
	FilterComparison::BeginsWith,
	FilterComparison::EndsWith,
	FilterComparison::Matches,
];
const STRING_WITH_EMPTY: &[FilterComparison] = &[
	FilterComparison::Equal,
	FilterComparison::NotEqual,
	FilterComparison::BeginsWith,
	FilterComparison::EndsWith,
	FilterComparison::Matches,
	FilterComparison::IsEmpty,
	FilterComparison::IsNotEmpty,
];
const NUMERIC: &[FilterComparison] = &[
	FilterComparison::Equal,
	FilterComparison::NotEqual,
	FilterComparison::GreaterThan,
	FilterComparison::GreaterThanEqual,
	FilterComparison::LessThan,
	FilterComparison::LessThanEqual,
];
const NUMERIC_WITH_LIST: &[FilterComparison] = &[
	FilterComparison::Equal,
	FilterComparison::NotEqual,
	FilterComparison::GreaterThan,
	FilterComparison::GreaterThanEqual,
	FilterComparison::LessThan,
	FilterComparison::LessThanEqual,
	FilterComparison::Contains,
	FilterComparison::NotContains,
];
const LIST_BASIC: &[FilterComparison] = &[
	FilterComparison::Equal,
	FilterComparison::NotEqual,
	FilterComparison::Contains,
	FilterComparison::NotContains,
];
const LIST_WITH_MATCHES: &[FilterComparison] = &[
	FilterComparison::Equal,
	FilterComparison::NotEqual,
	FilterComparison::Contains,
	FilterComparison::NotContains,
	FilterComparison::MustContains,
	FilterComparison::Matches,
];
const LIST_WITH_EMPTY: &[FilterComparison] = &[
	FilterComparison::Equal,
	FilterComparison::NotEqual,
	FilterComparison::Contains,
	FilterComparison::NotContains,
	FilterComparison::MustContains,
	FilterComparison::IsEmpty,
	FilterComparison::IsNotEmpty,
];
const DATE: &[FilterComparison] = &[
	FilterComparison::Equal,
	FilterComparison::NotEqual,
	FilterComparison::GreaterThan,
	FilterComparison::GreaterThanEqual,
	FilterComparison::LessThan,
	FilterComparison::LessThanEqual,
	FilterComparison::IsBefore,
	FilterComparison::IsAfter,
	FilterComparison::IsInLast,
	FilterComparison::IsNotInLast,
	FilterComparison::IsEmpty,
	FilterComparison::IsNotEmpty,
];

fn series_col(column: series::Column) -> Expr {
	Expr::col((series::Entity, column))
}

fn metadata_col(column: series_metadata::Column) -> Expr {
	Expr::col((series_metadata::Entity, column))
}

fn media_col(column: media::Column) -> Expr {
	Expr::col((media::Entity, column))
}

fn media_metadata_col(column: media_metadata::Column) -> Expr {
	Expr::col((media_metadata::Entity, column))
}

/// The columns Kavita's `SeriesName` filter searches.
fn name_columns(target: Target) -> Vec<Expr> {
	match target {
		Target::Series => vec![
			series_col(series::Column::Name),
			metadata_col(series_metadata::Column::Title),
			metadata_col(series_metadata::Column::TitleSort),
		],
		Target::Book => vec![
			media_col(media::Column::Name),
			media_metadata_col(media_metadata::Column::Title),
			media_metadata_col(media_metadata::Column::TitleSort),
		],
	}
}

fn summary_columns(target: Target) -> Vec<Expr> {
	match target {
		Target::Series => vec![
			metadata_col(series_metadata::Column::Summary),
			series_col(series::Column::Description),
		],
		Target::Book => vec![media_metadata_col(media_metadata::Column::Summary)],
	}
}

/// Kavita's `FolderPath`: the series folder, or the book's file path (its
/// folder is a prefix of it).
fn path_column(target: Target) -> Expr {
	match target {
		Target::Series => series_col(series::Column::Path),
		Target::Book => media_col(media::Column::Path),
	}
}

fn language_column(target: Target) -> Expr {
	match target {
		Target::Series => metadata_col(series_metadata::Column::Language),
		Target::Book => media_metadata_col(media_metadata::Column::Language),
	}
}

fn age_rating_column(target: Target) -> Expr {
	match target {
		Target::Series => metadata_col(series_metadata::Column::AgeRating),
		Target::Book => media_metadata_col(media_metadata::Column::AgeRating),
	}
}

fn year_column(target: Target) -> Expr {
	match target {
		Target::Series => metadata_col(series_metadata::Column::Year),
		Target::Book => media_metadata_col(media_metadata::Column::Year),
	}
}

/// A media-level condition as seen from the target: a series matches when any
/// of its media does, a book is the media row itself.
fn through_media(target: Target, condition: SimpleExpr) -> SimpleExpr {
	match target {
		Target::Series => media_subquery(condition),
		Target::Book => condition,
	}
}

/// A comma-separated name column (genres, people) as seen from a target:
/// `direct` sits on the target's own metadata row, `via_media` is reached
/// through the series' media metadata ([`Target::Series`] only).
struct NameColumns {
	direct: Option<Expr>,
	via_media: Option<Expr>,
}

impl NameColumns {
	fn combine(&self, build: impl Fn(Expr) -> SimpleExpr) -> SimpleExpr {
		let mut condition: Option<SimpleExpr> = None;
		if let Some(column) = self.direct.clone() {
			condition = Some(build(column));
		}
		if let Some(column) = self.via_media.clone() {
			let sub = media_metadata_subquery(build(column));
			condition = Some(match condition {
				Some(existing) => existing.or(sub),
				None => sub,
			});
		}
		condition.unwrap_or_else(|| Expr::value(false))
	}

	/// `LIKE '%name%'` for any of the names.
	fn contains(&self, names: &[String]) -> SimpleExpr {
		self.combine(|column| csv_contains(column, names))
	}

	/// Any non-empty value.
	fn any(&self) -> SimpleExpr {
		self.combine(|column| column.clone().is_not_null().and(column.ne("")))
	}
}

fn genre_columns(target: Target) -> NameColumns {
	match target {
		Target::Series => NameColumns {
			direct: Some(metadata_col(series_metadata::Column::Genres)),
			via_media: Some(media_metadata_col(media_metadata::Column::Genres)),
		},
		Target::Book => NameColumns {
			direct: Some(media_metadata_col(media_metadata::Column::Genres)),
			via_media: None,
		},
	}
}

fn people_name_columns(target: Target, field: SeriesFilterField) -> NameColumns {
	let (series_column, media_column) = people_columns(field);
	match target {
		Target::Series => NameColumns {
			direct: series_column.map(metadata_col),
			via_media: media_column.map(media_metadata_col),
		},
		Target::Book => NameColumns {
			direct: media_column.map(media_metadata_col),
			via_media: None,
		},
	}
}

fn string_condition(
	comparison: FilterComparison,
	value: &str,
	columns: &[Expr],
) -> SimpleExpr {
	let per_column = |column: &Expr| -> SimpleExpr {
		match comparison {
			FilterComparison::Equal => column.clone().eq(value),
			FilterComparison::BeginsWith => column.clone().like(format!("{value}%")),
			FilterComparison::EndsWith => column.clone().like(format!("%{value}")),
			FilterComparison::Matches => column.clone().like(format!("%{value}%")),
			FilterComparison::NotEqual => column.clone().ne(value),
			FilterComparison::IsEmpty => {
				column.clone().is_null().or(column.clone().eq(""))
			},
			FilterComparison::IsNotEmpty => {
				column.clone().is_not_null().and(column.clone().ne(""))
			},
			_ => Expr::value(true),
		}
	};
	let mut iter = columns.iter();
	let first = per_column(iter.next().expect("at least one column"));
	iter.fold(first, |acc, column| acc.or(per_column(column)))
}

/// `LIKE '%name%'` over a comma-separated metadata column.
fn csv_contains(column: Expr, names: &[String]) -> SimpleExpr {
	let mut iter = names.iter();
	let Some(first) = iter.next() else {
		return Expr::value(false);
	};
	let like = |name: &String| column.clone().like(format!("%{name}%"));
	iter.fold(like(first), |acc, name| acc.or(like(name)))
}

fn media_subquery(condition: SimpleExpr) -> SimpleExpr {
	series_col(series::Column::Id).in_subquery(
		Query::select()
			.column((media::Entity, media::Column::SeriesId))
			.from(media::Entity)
			.and_where(Expr::col((media::Entity, media::Column::DeletedAt)).is_null())
			.and_where(condition)
			.to_owned(),
	)
}

fn media_metadata_subquery(condition: SimpleExpr) -> SimpleExpr {
	series_col(series::Column::Id).in_subquery(
		Query::select()
			.column((media::Entity, media::Column::SeriesId))
			.from(media::Entity)
			.inner_join(
				media_metadata::Entity,
				Expr::col((media_metadata::Entity, media_metadata::Column::MediaId))
					.equals((media::Entity, media::Column::Id)),
			)
			.and_where(Expr::col((media::Entity, media::Column::DeletedAt)).is_null())
			.and_where(condition)
			.to_owned(),
	)
}

fn negate_if(condition: SimpleExpr, negate: bool) -> SimpleExpr {
	if negate {
		condition.not()
	} else {
		condition
	}
}

/// Resolve the names behind Kavita name ids (genres, people) by scanning the
/// distinct values stored in the given columns. Free-text values (the Kavita
/// Tachiyomi extension sends names for people prefixes) pass through as-is.
async fn resolve_names(
	conn: &DatabaseConnection,
	value: &str,
	columns: &[(&str, &str)],
) -> APIResult<Vec<String>> {
	let mut wanted_ids = HashSet::new();
	let mut names = Vec::new();
	for raw in value
		.split(',')
		.map(str::trim)
		.filter(|raw| !raw.is_empty())
	{
		match raw.parse::<i32>() {
			Ok(id) => {
				wanted_ids.insert(id);
			},
			Err(_) => names.push(raw.to_owned()),
		}
	}
	if wanted_ids.is_empty() {
		return Ok(names);
	}
	for (table, column) in columns {
		let rows = conn
			.query_all(Statement::from_sql_and_values(
				conn.get_database_backend(),
				&format!("SELECT DISTINCT {column} AS v FROM {table} WHERE {column} IS NOT NULL"),
				Vec::<Value>::new(),
			))
			.await?;
		for row in rows {
			let raw: String = row.try_get("", "v")?;
			for name in crate::mapper::split_csv(Some(&raw)) {
				if wanted_ids.contains(&name_id(&name)) && !names.contains(&name) {
					names.push(name);
				}
			}
		}
	}
	Ok(names)
}

const SERIES_METADATA_TABLE: &str = "series_metadata";
const MEDIA_METADATA_TABLE: &str = "media_metadata";

/// People fields map onto the series-level column when Stump has one and the
/// per-media metadata column otherwise.
fn people_columns(
	field: SeriesFilterField,
) -> (
	Option<series_metadata::Column>,
	Option<media_metadata::Column>,
) {
	match field {
		SeriesFilterField::Writers => (
			Some(series_metadata::Column::Writers),
			Some(media_metadata::Column::Writers),
		),
		SeriesFilterField::Publisher => (
			Some(series_metadata::Column::Publisher),
			Some(media_metadata::Column::Publisher),
		),
		SeriesFilterField::Characters => (
			Some(series_metadata::Column::Characters),
			Some(media_metadata::Column::Characters),
		),
		SeriesFilterField::Imprint => (Some(series_metadata::Column::Imprint), None),
		SeriesFilterField::Penciller => (None, Some(media_metadata::Column::Pencillers)),
		SeriesFilterField::Inker => (None, Some(media_metadata::Column::Inkers)),
		SeriesFilterField::Colorist => (None, Some(media_metadata::Column::Colorists)),
		SeriesFilterField::Letterer => (None, Some(media_metadata::Column::Letterers)),
		SeriesFilterField::CoverArtist => {
			(None, Some(media_metadata::Column::CoverArtists))
		},
		SeriesFilterField::Editor => (None, Some(media_metadata::Column::Editors)),
		SeriesFilterField::Team => (None, Some(media_metadata::Column::Teams)),
		_ => (None, None),
	}
}

fn column_name<C: sea_orm::ColumnTrait + sea_orm::Iden>(column: C) -> String {
	column.to_string()
}

/// The Stump `status` spellings that fold into each Kavita status.
fn status_spellings(status: PublicationStatus) -> &'static [&'static str] {
	match status {
		PublicationStatus::OnGoing => {
			&["ongoing", "on going", "on-going", "continuing", ""]
		},
		PublicationStatus::Hiatus => &["hiatus", "on hiatus", "paused"],
		PublicationStatus::Completed => &["completed", "complete", "finished"],
		PublicationStatus::Cancelled => &["cancelled", "canceled", "abandoned"],
		PublicationStatus::Ended => &["ended", "end"],
	}
}

/// The Stump minimum ages that map onto a Kavita rating (see
/// `AgeRating::from_min_age`).
fn ages_for_rating(rating: AgeRating) -> Vec<i32> {
	(0..=30)
		.filter(|age| AgeRating::from_min_age(Some(*age)) == rating)
		.collect()
}

/// The smallest Stump age that maps onto the rating, for ordered comparisons.
fn min_age_for_rating(rating: AgeRating) -> i32 {
	ages_for_rating(rating).into_iter().min().unwrap_or(0)
}

/// Build the SQL/memory plan for a filter. `user` scopes the library
/// statements to libraries the user can see.
pub(crate) async fn plan(
	conn: &DatabaseConnection,
	_user: &AuthUser,
	filter: &SeriesFilterV2Dto,
) -> APIResult<FilterPlan> {
	let combination = filter.combination;
	let mut groups = Groups::default();
	let mut progress = ProgressFilters::default();
	let mut include_libraries: Vec<String> = Vec::new();
	let mut exclude_libraries: Vec<String> = Vec::new();

	for statement in &filter.statements {
		let comparison = statement.comparison;
		let value = statement.value.trim();
		let Some(field) = statement.field.known() else {
			tracing::debug!(
				field = statement.field.value(),
				"Ignoring unknown Kavita filter field"
			);
			continue;
		};
		let negated = matches!(
			comparison,
			FilterComparison::NotEqual | FilterComparison::NotContains
		);
		match field {
			SeriesFilterField::Libraries => {
				let ids = statement.int_values().map_err(bad_request)?;
				let mut stump_ids = Vec::with_capacity(ids.len());
				for id in ids {
					if let Some(stump_id) =
						KavitaIds::lookup(conn, IdKind::Library, id).await?
					{
						stump_ids.push(stump_id);
					} else {
						// An unknown library id can never match; keep the
						// statement effective by filtering on an impossible id.
						stump_ids.push(format!("kavita-unknown-library-{id}"));
					}
				}
				if matches!(
					comparison,
					FilterComparison::Equal | FilterComparison::Contains
				) {
					include_libraries.extend(stump_ids);
				} else {
					exclude_libraries.extend(stump_ids);
				}
			},
			SeriesFilterField::SeriesName => {
				if value.is_empty() {
					continue;
				}
				validate(comparison, STRING, "Series.Name")?;
				groups.add(|target| {
					string_condition(comparison, value, &name_columns(target))
				});
			},
			SeriesFilterField::Summary => {
				validate(comparison, STRING_WITH_EMPTY, "Series.Summary")?;
				groups.add(|target| {
					string_condition(comparison, value, &summary_columns(target))
				});
			},
			SeriesFilterField::Path => {
				validate(comparison, STRING, "Series.FolderPath")?;
				groups.add(|target| {
					string_condition(comparison, value, &[path_column(target)])
				});
			},
			SeriesFilterField::FilePath => {
				validate(comparison, STRING, "Series.FilePath")?;
				groups.add(|target| {
					through_media(
						target,
						string_condition(
							comparison,
							value,
							&[media_col(media::Column::Path)],
						),
					)
				});
			},
			SeriesFilterField::PublicationStatus => {
				let statuses = statement
					.int_values()
					.map_err(bad_request)?
					.into_iter()
					.map(|value| {
						PublicationStatus::try_from(value).map_err(|value| {
							bad_request(format!(
								"{value} is not a valid PublicationStatus"
							))
						})
					})
					.collect::<APIResult<Vec<_>>>()?;
				if statuses.is_empty() {
					continue;
				}
				validate(comparison, LIST_BASIC, "Series.PublicationStatus")?;
				let selected = match comparison {
					FilterComparison::Equal | FilterComparison::NotEqual => {
						vec![statuses[0]]
					},
					_ => statuses,
				};
				let spellings = selected
					.iter()
					.flat_map(|status| {
						status_spellings(*status).iter().map(|s| String::from(*s))
					})
					.collect::<Vec<_>>();
				let includes_unset = selected.contains(&PublicationStatus::OnGoing);
				groups.add(|target| {
					let matched = match target {
						Target::Series => {
							let lowered = Func::lower(metadata_col(
								series_metadata::Column::Status,
							));
							let mut condition = Expr::expr(SimpleExpr::from(lowered))
								.is_in(spellings.clone());
							if includes_unset {
								condition = condition
									.or(metadata_col(series_metadata::Column::Status)
										.is_null());
							}
							condition
						},
						// A book is a complete work: `maxCount == totalCount == 1`
						// makes it `Completed` in Kavita's scanner.
						Target::Book => {
							Expr::value(selected.contains(&PublicationStatus::Completed))
						},
					};
					negate_if(matched, negated)
				});
			},
			SeriesFilterField::Languages => {
				let languages = value
					.split(',')
					.map(str::trim)
					.filter(|language| !language.is_empty())
					.map(str::to_owned)
					.collect::<Vec<_>>();
				if languages.is_empty() {
					continue;
				}
				validate(comparison, LIST_WITH_MATCHES, "Series.Language")?;
				groups.add(|target| {
					let column = language_column(target);
					match comparison {
						FilterComparison::Equal => column.eq(languages[0].clone()),
						FilterComparison::NotEqual => column.ne(languages[0].clone()),
						FilterComparison::Contains | FilterComparison::MustContains => {
							column.is_in(languages.clone())
						},
						FilterComparison::NotContains => {
							column.is_not_in(languages.clone())
						},
						FilterComparison::Matches => {
							column.like(format!("{}%", languages[0]))
						},
						_ => Expr::value(true),
					}
				});
			},
			SeriesFilterField::AgeRating => {
				let ratings = statement
					.int_values()
					.map_err(bad_request)?
					.into_iter()
					.map(|value| {
						AgeRating::try_from(value).map_err(|value| {
							bad_request(format!("{value} is not a valid AgeRating"))
						})
					})
					.collect::<APIResult<Vec<_>>>()?;
				if ratings.is_empty() {
					continue;
				}
				validate(comparison, NUMERIC_WITH_LIST, "Series.AgeRating")?;
				let first = ratings[0];
				let all_ages = ratings
					.iter()
					.flat_map(|r| ages_for_rating(*r))
					.collect::<Vec<_>>();
				groups.add(|target| {
					let column = age_rating_column(target);
					let with_unknown = |ratings: &[AgeRating], condition: SimpleExpr| {
						if ratings.contains(&AgeRating::Unknown) {
							condition.or(age_rating_column(target).is_null())
						} else {
							condition
						}
					};
					match comparison {
						FilterComparison::Equal => {
							with_unknown(&[first], column.is_in(ages_for_rating(first)))
						},
						FilterComparison::NotEqual => negate_if(
							with_unknown(&[first], column.is_in(ages_for_rating(first))),
							true,
						),
						FilterComparison::Contains => {
							with_unknown(&ratings, column.is_in(all_ages.clone()))
						},
						FilterComparison::NotContains => negate_if(
							with_unknown(&ratings, column.is_in(all_ages.clone())),
							true,
						),
						FilterComparison::GreaterThan => column
							.gt(min_age_for_rating(first).max(
								ages_for_rating(first).into_iter().max().unwrap_or(0),
							)),
						FilterComparison::GreaterThanEqual => {
							column.gte(min_age_for_rating(first))
						},
						FilterComparison::LessThan => {
							column.lt(min_age_for_rating(first))
						},
						FilterComparison::LessThanEqual => column
							.lte(ages_for_rating(first).into_iter().max().unwrap_or(0)),
						_ => Expr::value(true),
					}
				});
			},
			SeriesFilterField::Tags => {
				validate(comparison, LIST_WITH_EMPTY, "Series.Tags")?;
				let tag_ids = statement.int_values().map_err(bad_request)?;
				if tag_ids.is_empty()
					&& !matches!(
						comparison,
						FilterComparison::IsEmpty | FilterComparison::IsNotEmpty
					) {
					continue;
				}
				// Stump attaches tags to a series row and to a media item, and
				// a Kavita series' tag list is the union of both
				// (`map_series_metadata`), so either side matches. A book is
				// its file, so only the media side applies.
				let media_tagged = |target: Target, ids: Option<Vec<i32>>| {
					through_media(
						target,
						media_col(media::Column::Id).in_subquery(
							Query::select()
								.column(media_tag::Column::MediaId)
								.from(media_tag::Entity)
								.and_where_option(
									ids.map(|ids| media_tag::Column::TagId.is_in(ids)),
								)
								.to_owned(),
						),
					)
				};
				let tagged = |target: Target, ids: Option<Vec<i32>>| {
					let via_media = media_tagged(target, ids.clone());
					match target {
						Target::Series => series_col(series::Column::Id)
							.in_subquery(
								Query::select()
									.column(series_tag::Column::SeriesId)
									.from(series_tag::Entity)
									.and_where_option(ids.map(|ids| {
										series_tag::Column::TagId.is_in(ids)
									}))
									.to_owned(),
							)
							.or(via_media),
						Target::Book => via_media,
					}
				};
				groups.add(|target| match comparison {
					FilterComparison::Equal | FilterComparison::Contains => {
						tagged(target, Some(tag_ids.clone()))
					},
					FilterComparison::NotEqual | FilterComparison::NotContains => {
						tagged(target, Some(tag_ids.clone())).not()
					},
					FilterComparison::MustContains => tag_ids
						.iter()
						.map(|id| tagged(target, Some(vec![*id])))
						.reduce(|acc, next| acc.and(next))
						.unwrap_or_else(|| Expr::value(true)),
					FilterComparison::IsEmpty => tagged(target, None).not(),
					FilterComparison::IsNotEmpty => tagged(target, None),
					_ => Expr::value(true),
				});
			},
			SeriesFilterField::Genres => {
				validate(comparison, LIST_WITH_EMPTY, "Series.Genres")?;
				let is_empty_check = matches!(
					comparison,
					FilterComparison::IsEmpty | FilterComparison::IsNotEmpty
				);
				let names = if is_empty_check {
					Vec::new()
				} else {
					resolve_names(
						conn,
						value,
						&[
							(
								SERIES_METADATA_TABLE,
								&column_name(series_metadata::Column::Genres),
							),
							(
								MEDIA_METADATA_TABLE,
								&column_name(media_metadata::Column::Genres),
							),
						],
					)
					.await?
				};
				if names.is_empty() && !is_empty_check {
					if value.is_empty() {
						continue;
					}
					// Ids that match no stored genre: Contains matches nothing,
					// NotContains matches everything, exactly like Kavita.
					groups.add_both(Expr::value(negated));
					continue;
				}
				groups.add(|target| {
					name_list_condition(comparison, &genre_columns(target), &names)
				});
			},
			SeriesFilterField::Writers
			| SeriesFilterField::Publisher
			| SeriesFilterField::Characters
			| SeriesFilterField::Imprint
			| SeriesFilterField::Penciller
			| SeriesFilterField::Inker
			| SeriesFilterField::Colorist
			| SeriesFilterField::Letterer
			| SeriesFilterField::CoverArtist
			| SeriesFilterField::Editor
			| SeriesFilterField::Team
			| SeriesFilterField::Translators
			| SeriesFilterField::Location => {
				validate(comparison, LIST_WITH_EMPTY, "Series.People")?;
				let (series_column, media_column) = people_columns(field);
				let is_empty_check = matches!(
					comparison,
					FilterComparison::IsEmpty | FilterComparison::IsNotEmpty
				);
				let mut sources: Vec<(&str, String)> = Vec::new();
				if let Some(column) = series_column {
					sources.push((SERIES_METADATA_TABLE, column_name(column)));
				}
				if let Some(column) = media_column {
					sources.push((MEDIA_METADATA_TABLE, column_name(column)));
				}
				let source_refs = sources
					.iter()
					.map(|(table, column)| (*table, column.as_str()))
					.collect::<Vec<_>>();
				let names = if is_empty_check {
					Vec::new()
				} else {
					resolve_names(conn, value, &source_refs).await?
				};
				if names.is_empty() && !is_empty_check {
					if value.is_empty() {
						continue;
					}
					groups.add_both(Expr::value(negated));
					continue;
				}
				groups.add(|target| {
					name_list_condition(
						comparison,
						&people_name_columns(target, field),
						&names,
					)
				});
			},
			SeriesFilterField::Formats => {
				let formats = statement.formats().map_err(bad_request)?;
				if formats.is_empty() {
					continue;
				}
				validate(comparison, LIST_BASIC, "Series.Format")?;
				let extensions = formats
					.iter()
					.flat_map(|format| extensions_for(*format))
					.map(|ext| String::from(*ext))
					.collect::<Vec<_>>();
				let unknown_formats = formats.contains(&MangaFormat::Unknown);
				let known = MangaFormat::ALL
					.iter()
					.flat_map(|format| extensions_for(*format))
					.map(|ext| String::from(*ext))
					.collect::<Vec<_>>();
				// Kavita keys a series by format, so `Series.Format` is a
				// single value; Stump reports the format of the series' first
				// media (`SeriesInput::format`). The statement matches that
				// file's extension, never any file's: a manga series holding a
				// stray EPUB stays out of `Formats = Epub`.
				groups.add(|target| {
					let condition = match target {
						Target::Series => first_media_format_condition(
							&extensions,
							unknown_formats,
							&known,
						),
						// A book series *is* its media row.
						Target::Book => extension_format_condition(
							SimpleExpr::from(Func::lower(media_col(
								media::Column::Extension,
							))),
							&extensions,
							unknown_formats,
							&known,
						),
					};
					negate_if(condition, negated)
				});
			},
			SeriesFilterField::ReleaseYear => {
				let year = value
					.parse::<i32>()
					.map_err(|_| bad_request(format!("'{value}' is not a valid year")))?;
				validate(comparison, DATE, "Series.ReleaseYear")?;
				let this_year = chrono::Utc::now()
					.format("%Y")
					.to_string()
					.parse::<i32>()
					.unwrap_or(0);
				groups.add(|target| {
					let column = year_column(target);
					match comparison {
						FilterComparison::Equal => column.eq(year),
						FilterComparison::NotEqual => column.ne(year),
						FilterComparison::GreaterThan | FilterComparison::IsAfter => {
							column.gt(year)
						},
						FilterComparison::GreaterThanEqual => column.gte(year),
						FilterComparison::LessThan | FilterComparison::IsBefore => {
							column.lt(year)
						},
						FilterComparison::LessThanEqual => column.lte(year),
						FilterComparison::IsInLast => column.gte(this_year - year),
						FilterComparison::IsNotInLast => column.lt(this_year - year),
						FilterComparison::IsEmpty => {
							column.clone().is_null().or(column.eq(0))
						},
						FilterComparison::IsNotEmpty => {
							column.clone().is_not_null().and(column.ne(0))
						},
						_ => Expr::value(true),
					}
				});
			},
			SeriesFilterField::ReadProgress => {
				validate(comparison, NUMERIC, "Series.ReadProgress")?;
				let percentage = statement.float_value().map_err(bad_request)?;
				progress.statements.push((comparison, percentage));
			},
			SeriesFilterField::UserRating
			| SeriesFilterField::AverageRating
			| SeriesFilterField::ReadTime
			| SeriesFilterField::ReadingDate
			| SeriesFilterField::ReadLast
			| SeriesFilterField::FileSize
			| SeriesFilterField::WantToRead
			| SeriesFilterField::CollectionTags
			| SeriesFilterField::CollapseSeriesRelationships => {
				tracing::debug!(
					?field,
					"Kavita filter field has no Stump equivalent; ignored"
				);
			},
		}
	}

	// Both targets see the media's `series` row, so library scoping is shared;
	// a book additionally needs its own file undeleted.
	let mut condition =
		Condition::all().add(series_col(series::Column::DeletedAt).is_null());
	// `ApplyLibraryFilter`: includes always apply; excludes only under AND.
	if !include_libraries.is_empty() {
		condition =
			condition.add(series_col(series::Column::LibraryId).is_in(include_libraries));
	}
	if !exclude_libraries.is_empty() && combination == FilterCombination::And {
		condition = condition
			.add(series_col(series::Column::LibraryId).is_not_in(exclude_libraries));
	}
	let mut book_condition = condition
		.clone()
		.add(media_col(media::Column::DeletedAt).is_null());
	if !groups.is_empty() {
		let Groups { series, book } = groups;
		condition = condition.add(Groups::condition(series, combination));
		book_condition = book_condition.add(Groups::condition(book, combination));
	}

	let sort = filter.effective_sort();
	let direction = if sort.is_ascending {
		Order::Asc
	} else {
		Order::Desc
	};
	let sort_name = |target: Target| -> SimpleExpr {
		let columns = match target {
			Target::Series => [
				metadata_col(series_metadata::Column::TitleSort),
				metadata_col(series_metadata::Column::Title),
				series_col(series::Column::Name),
			],
			Target::Book => [
				media_metadata_col(media_metadata::Column::TitleSort),
				media_metadata_col(media_metadata::Column::Title),
				media_col(media::Column::Name),
			],
		};
		SimpleExpr::from(Func::lower(Func::coalesce(
			columns.into_iter().map(SimpleExpr::from),
		)))
	};
	let key = |series: SimpleExpr, book: SimpleExpr, order: Order| SortKey {
		series,
		book,
		order,
	};
	let mut sort_by_progress = None;
	let mut order: Vec<SortKey> = match sort.sort_field {
		SeriesSortField::SortName
		| SeriesSortField::AverageRating
		| SeriesSortField::UserRating => {
			vec![key(
				sort_name(Target::Series),
				sort_name(Target::Book),
				direction,
			)]
		},
		SeriesSortField::CreatedDate => vec![SortKey::created(direction)],
		SeriesSortField::LastModifiedDate => vec![key(
			Func::coalesce([
				series_col(series::Column::UpdatedAt).into(),
				series_col(series::Column::CreatedAt).into(),
			])
			.into(),
			Func::coalesce([
				media_col(media::Column::UpdatedAt).into(),
				media_col(media::Column::CreatedAt).into(),
			])
			.into(),
			direction,
		)],
		SeriesSortField::LastChapterAdded => vec![key(
			SimpleExpr::SubQuery(
				None,
				Box::new(
					Query::select()
						.expr(Func::max(media_col(media::Column::CreatedAt)))
						.from(media::Entity)
						.and_where(
							media_col(media::Column::SeriesId)
								.equals((series::Entity, series::Column::Id)),
						)
						.to_owned()
						.into_sub_query_statement(),
				),
			),
			media_col(media::Column::CreatedAt).into(),
			direction,
		)],
		SeriesSortField::TimeToRead => vec![key(
			SimpleExpr::SubQuery(
				None,
				Box::new(
					Query::select()
						.expr(Func::sum(media_col(media::Column::Pages)))
						.from(media::Entity)
						.and_where(
							media_col(media::Column::SeriesId)
								.equals((series::Entity, series::Column::Id)),
						)
						.to_owned()
						.into_sub_query_statement(),
				),
			),
			media_col(media::Column::Pages).into(),
			direction,
		)],
		SeriesSortField::ReleaseYear => vec![key(
			year_column(Target::Series).into(),
			year_column(Target::Book).into(),
			direction,
		)],
		SeriesSortField::Random => {
			vec![key(
				Expr::cust("RANDOM()"),
				Expr::cust("RANDOM()"),
				Order::Asc,
			)]
		},
		SeriesSortField::ReadProgress => {
			sort_by_progress = Some(ProgressSort::ReadProgress {
				ascending: sort.is_ascending,
			});
			vec![key(
				sort_name(Target::Series),
				sort_name(Target::Book),
				Order::Asc,
			)]
		},
		SeriesSortField::UnreadChapterCount => {
			sort_by_progress = Some(ProgressSort::UnreadCount {
				ascending: sort.is_ascending,
			});
			vec![key(
				sort_name(Target::Series),
				sort_name(Target::Book),
				Order::Asc,
			)]
		},
	};
	// Stable secondary order so pages never overlap.
	order.push(key(
		series_col(series::Column::Id).into(),
		media_col(media::Column::Id).into(),
		Order::Asc,
	));

	Ok(FilterPlan {
		condition,
		book_condition,
		order,
		progress,
		sort_by_progress,
		combination,
		limit_to: filter.limit_to.max(0),
	})
}

/// A list comparison over a name column: any/all of `names`, or emptiness.
fn name_list_condition(
	comparison: FilterComparison,
	columns: &NameColumns,
	names: &[String],
) -> SimpleExpr {
	match comparison {
		FilterComparison::Equal | FilterComparison::Contains => columns.contains(names),
		FilterComparison::NotEqual | FilterComparison::NotContains => {
			columns.contains(names).not()
		},
		FilterComparison::MustContains => names
			.iter()
			.map(|name| columns.contains(std::slice::from_ref(name)))
			.reduce(|acc, next| acc.and(next))
			.unwrap_or_else(|| Expr::value(true)),
		FilterComparison::IsEmpty => columns.any().not(),
		FilterComparison::IsNotEmpty => columns.any(),
		_ => Expr::value(true),
	}
}

fn extensions_for(format: MangaFormat) -> &'static [&'static str] {
	match format {
		MangaFormat::Archive => &["cbz", "cbr", "cb7", "cbt", "zip", "rar", "7z", "tar"],
		MangaFormat::Epub => &["epub"],
		MangaFormat::Pdf => &["pdf"],
		MangaFormat::Image => &[
			"png", "jpg", "jpeg", "webp", "gif", "avif", "bmp", "tiff", "tif", "jxl",
			"heif", "heic",
		],
		MangaFormat::Unknown => &[],
	}
}

/// A format statement over one already-lowered extension expression: the
/// wanted extensions, plus everything Stump cannot classify when the
/// statement asks for `Unknown`.
fn extension_format_condition(
	extension: SimpleExpr,
	extensions: &[String],
	unknown_formats: bool,
	known: &[String],
) -> SimpleExpr {
	let condition = Expr::expr(extension.clone()).is_in(extensions.to_vec());
	if unknown_formats {
		condition.or(Expr::expr(extension).is_not_in(known.to_vec()))
	} else {
		condition
	}
}

/// The format of a grouped series: the extension of its *first* media, the
/// same file `SeriesInput::format` reports. `mapper::sort_media` orders media
/// by volume number (falling back to an integral chapter number, else the
/// media's 1-based position in name order) and then by name, so the ordering
/// is rebuilt here with window functions.
fn first_media_format_condition(
	extensions: &[String],
	unknown_formats: bool,
	known: &[String],
) -> SimpleExpr {
	let placeholders = |count: usize| vec!["?"; count].join(", ");
	let mut predicate =
		format!(r#""first"."ext" IN ({})"#, placeholders(extensions.len()));
	let mut values = extensions
		.iter()
		.map(|ext| Value::from(ext.clone()))
		.collect::<Vec<_>>();
	if unknown_formats {
		predicate.push_str(&format!(
			r#" OR "first"."ext" NOT IN ({})"#,
			placeholders(known.len())
		));
		values.extend(known.iter().map(|ext| Value::from(ext.clone())));
	}
	Expr::cust_with_values(
		format!(
			r#""series"."id" IN (
	SELECT "first"."series_id" FROM (
		SELECT
			"ranked"."series_id" AS "series_id",
			"ranked"."ext" AS "ext",
			ROW_NUMBER() OVER (
				PARTITION BY "ranked"."series_id"
				ORDER BY "ranked"."number", "ranked"."name"
			) AS "position"
		FROM (
			SELECT
				"m"."series_id" AS "series_id",
				LOWER("m"."extension") AS "ext",
				"m"."name" AS "name",
				CASE
					WHEN "mm"."volume" > 0 THEN "mm"."volume"
					WHEN CAST("mm"."number" AS REAL) > 0
						AND CAST("mm"."number" AS REAL)
							= CAST(CAST("mm"."number" AS INTEGER) AS REAL)
						THEN CAST("mm"."number" AS INTEGER)
					ELSE ROW_NUMBER() OVER (
						PARTITION BY "m"."series_id" ORDER BY "m"."name"
					)
				END AS "number"
			FROM "media" AS "m"
			LEFT JOIN "media_metadata" AS "mm" ON "mm"."media_id" = "m"."id"
			WHERE "m"."deleted_at" IS NULL AND "m"."series_id" IS NOT NULL
		) AS "ranked"
	) AS "first"
	WHERE "first"."position" = 1 AND ({predicate})
)"#
		),
		values,
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn progress_filters_follow_kavita_comparisons() {
		let filters = ProgressFilters {
			statements: vec![
				(FilterComparison::GreaterThan, 0.0),
				(FilterComparison::LessThan, 100.0),
			],
		};
		assert!(filters.matches(50.0, FilterCombination::And));
		assert!(!filters.matches(0.0, FilterCombination::And));
		assert!(!filters.matches(100.0, FilterCombination::And));
		assert!(filters.matches(100.0, FilterCombination::Or));
		assert!(ProgressFilters::default().matches(3.0, FilterCombination::And));
	}

	#[test]
	fn age_rating_buckets_cover_every_stump_age() {
		for age in 0..=30 {
			let rating = AgeRating::from_min_age(Some(age));
			assert!(ages_for_rating(rating).contains(&age));
		}
		assert_eq!(min_age_for_rating(AgeRating::Teen), 13);
		assert!(ages_for_rating(AgeRating::Unknown).is_empty());
	}

	#[test]
	fn publication_status_spellings_round_trip() {
		for status in PublicationStatus::ALL {
			for spelling in status_spellings(*status) {
				assert_eq!(PublicationStatus::from_status_text(Some(spelling)), *status);
			}
		}
	}

	/// `kavita-ref` `25620`: `Formats Contains 3` lists the EPUB *series*, not
	/// every series that happens to hold an EPUB file. Kavita keys a series by
	/// format; Stump reports the first media's format, so a manga series with
	/// one stray EPUB keeps its `Archive` format and stays out.
	#[tokio::test]
	async fn formats_match_the_series_own_format() {
		use crate::dto::MangaFormat;
		use crate::filter::SeriesFilterStatementDto;
		use crate::routes::series::{list_series, UserParams};
		use crate::test_support::{
			auth_user, db, library_of_type, series_with_files, TestBackend,
		};
		use models::shared::enums::LibraryType as StumpLibraryType;

		let conn = db().await;
		let user_row = ::tests::fake_data::User::new("reader").insert(&conn).await;
		let user = auth_user(&user_row);
		let manga = library_of_type(&conn, StumpLibraryType::Manga).await;
		let books = library_of_type(&conn, StumpLibraryType::Book).await;
		series_with_files(
			&conn,
			&manga.id,
			"Mixed",
			&[("v01", "cbz", 20), ("v02", "epub", 20)],
		)
		.await;
		series_with_files(&conn, &books.id, "Shelf", &[("alice", "epub", 15)]).await;
		// Volume metadata, not the file name, decides which file is first: the
		// EPUB sorts first by name but is volume 2.
		let (_, volumes) = series_with_files(
			&conn,
			&manga.id,
			"Volumed",
			&[("a-extra", "epub", 20), ("b-main", "cbz", 20)],
		)
		.await;
		for (media, volume) in volumes.iter().zip([2, 1]) {
			models::entity::media_metadata::ActiveModel {
				media_id: sea_orm::ActiveValue::Set(Some(media.id.clone())),
				volume: sea_orm::ActiveValue::Set(Some(volume)),
				..Default::default()
			}
			.insert(&conn)
			.await
			.unwrap();
		}
		let backend = TestBackend::new(conn);

		let listed = |comparison: FilterComparison, formats: &str| {
			let filter = SeriesFilterV2Dto {
				statements: vec![SeriesFilterStatementDto {
					comparison,
					field: SeriesFilterField::Formats.into(),
					value: formats.to_owned(),
				}],
				..Default::default()
			};
			let backend = &backend;
			let user = &user;
			async move {
				list_series(backend, user, &filter, UserParams::parse(""), None, None)
					.await
					.unwrap()
					.0
			}
		};

		let epub = listed(
			FilterComparison::Contains,
			&(MangaFormat::Epub as i32).to_string(),
		)
		.await;
		assert_eq!(
			epub.iter().map(|dto| dto.name.as_str()).collect::<Vec<_>>(),
			["alice"],
			"the mixed manga series is not an EPUB series"
		);
		assert_eq!(epub[0].format, MangaFormat::Epub);

		let archive = listed(
			FilterComparison::Contains,
			&(MangaFormat::Archive as i32).to_string(),
		)
		.await;
		assert_eq!(
			archive
				.iter()
				.map(|dto| (dto.name.as_str(), dto.format))
				.collect::<Vec<_>>(),
			[
				("Mixed", MangaFormat::Archive),
				("Volumed", MangaFormat::Archive)
			],
			"a series keeps the format of its first file, volume order first"
		);

		// The negated form is the complement over the same series format.
		let not_epub = listed(
			FilterComparison::NotContains,
			&(MangaFormat::Epub as i32).to_string(),
		)
		.await;
		assert_eq!(
			not_epub
				.iter()
				.map(|dto| dto.name.as_str())
				.collect::<Vec<_>>(),
			["Mixed", "Volumed"]
		);
	}
}
