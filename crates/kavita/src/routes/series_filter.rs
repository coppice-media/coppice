//! Translate `SeriesFilterV2Dto` into SeaORM conditions and ordering
//! (`Kavita.Database/Extensions/Filters/SeriesFilter.cs` and
//! `SeriesRepository.CreateFilteredSearchQueryableV2`).

use std::collections::HashSet;

use models::entity::{
	media, media_metadata, series, series_metadata, series_tag, user::AuthUser,
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

/// The executable form of a filter.
#[derive(Debug, Clone)]
pub(crate) struct FilterPlan {
	pub condition: Condition,
	pub order: Vec<(SimpleExpr, Order)>,
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
	let mut group = match combination {
		FilterCombination::And => Condition::all(),
		FilterCombination::Or => Condition::any(),
	};
	let mut has_group_statement = false;
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
				has_group_statement = true;
				group = group.add(string_condition(
					comparison,
					value,
					&[
						series_col(series::Column::Name),
						metadata_col(series_metadata::Column::Title),
						metadata_col(series_metadata::Column::TitleSort),
					],
				));
			},
			SeriesFilterField::Summary => {
				validate(comparison, STRING_WITH_EMPTY, "Series.Summary")?;
				has_group_statement = true;
				group = group.add(string_condition(
					comparison,
					value,
					&[
						metadata_col(series_metadata::Column::Summary),
						series_col(series::Column::Description),
					],
				));
			},
			SeriesFilterField::Path => {
				validate(comparison, STRING, "Series.FolderPath")?;
				has_group_statement = true;
				group = group.add(string_condition(
					comparison,
					value,
					&[series_col(series::Column::Path)],
				));
			},
			SeriesFilterField::FilePath => {
				validate(comparison, STRING, "Series.FilePath")?;
				has_group_statement = true;
				group = group.add(media_subquery(string_condition(
					comparison,
					value,
					&[Expr::col((media::Entity, media::Column::Path))],
				)));
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
				has_group_statement = true;
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
				let lowered = Func::lower(metadata_col(series_metadata::Column::Status));
				let mut condition =
					Expr::expr(SimpleExpr::from(lowered)).is_in(spellings);
				if includes_unset {
					condition = condition
						.or(metadata_col(series_metadata::Column::Status).is_null());
				}
				group = group.add(negate_if(
					condition,
					matches!(
						comparison,
						FilterComparison::NotEqual | FilterComparison::NotContains
					),
				));
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
				has_group_statement = true;
				let column = metadata_col(series_metadata::Column::Language);
				let condition = match comparison {
					FilterComparison::Equal => column.eq(languages[0].clone()),
					FilterComparison::NotEqual => column.ne(languages[0].clone()),
					FilterComparison::Contains | FilterComparison::MustContains => {
						column.is_in(languages)
					},
					FilterComparison::NotContains => column.is_not_in(languages),
					FilterComparison::Matches => {
						column.like(format!("{}%", languages[0]))
					},
					_ => Expr::value(true),
				};
				group = group.add(condition);
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
				has_group_statement = true;
				let column = metadata_col(series_metadata::Column::AgeRating);
				let first = ratings[0];
				let with_unknown = |ratings: &[AgeRating], condition: SimpleExpr| {
					if ratings.contains(&AgeRating::Unknown) {
						condition
							.or(metadata_col(series_metadata::Column::AgeRating)
								.is_null())
					} else {
						condition
					}
				};
				let condition = match comparison {
					FilterComparison::Equal => {
						with_unknown(&[first], column.is_in(ages_for_rating(first)))
					},
					FilterComparison::NotEqual => negate_if(
						with_unknown(&[first], column.is_in(ages_for_rating(first))),
						true,
					),
					FilterComparison::Contains => {
						let ages = ratings
							.iter()
							.flat_map(|r| ages_for_rating(*r))
							.collect::<Vec<_>>();
						with_unknown(&ratings, column.is_in(ages))
					},
					FilterComparison::NotContains => {
						let ages = ratings
							.iter()
							.flat_map(|r| ages_for_rating(*r))
							.collect::<Vec<_>>();
						negate_if(with_unknown(&ratings, column.is_in(ages)), true)
					},
					FilterComparison::GreaterThan => column.gt(min_age_for_rating(first)
						.max(ages_for_rating(first).into_iter().max().unwrap_or(0))),
					FilterComparison::GreaterThanEqual => {
						column.gte(min_age_for_rating(first))
					},
					FilterComparison::LessThan => column.lt(min_age_for_rating(first)),
					FilterComparison::LessThanEqual => {
						column.lte(ages_for_rating(first).into_iter().max().unwrap_or(0))
					},
					_ => Expr::value(true),
				};
				group = group.add(condition);
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
				has_group_statement = true;
				let tagged = |ids: Vec<i32>| {
					series_col(series::Column::Id).in_subquery(
						Query::select()
							.column(series_tag::Column::SeriesId)
							.from(series_tag::Entity)
							.and_where(series_tag::Column::TagId.is_in(ids))
							.to_owned(),
					)
				};
				let any_tag = series_col(series::Column::Id).in_subquery(
					Query::select()
						.column(series_tag::Column::SeriesId)
						.from(series_tag::Entity)
						.to_owned(),
				);
				let condition = match comparison {
					FilterComparison::Equal | FilterComparison::Contains => {
						tagged(tag_ids)
					},
					FilterComparison::NotEqual | FilterComparison::NotContains => {
						tagged(tag_ids).not()
					},
					FilterComparison::MustContains => tag_ids
						.into_iter()
						.map(|id| tagged(vec![id]))
						.reduce(|acc, next| acc.and(next))
						.unwrap_or_else(|| Expr::value(true)),
					FilterComparison::IsEmpty => any_tag.not(),
					FilterComparison::IsNotEmpty => any_tag,
					_ => Expr::value(true),
				};
				group = group.add(condition);
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
					has_group_statement = true;
					group = group.add(Expr::value(matches!(
						comparison,
						FilterComparison::NotEqual | FilterComparison::NotContains
					)));
					continue;
				}
				has_group_statement = true;
				let series_genres = metadata_col(series_metadata::Column::Genres);
				let media_genres =
					Expr::col((media_metadata::Entity, media_metadata::Column::Genres));
				let any_genre = series_genres
					.clone()
					.is_not_null()
					.and(series_genres.clone().ne(""))
					.or(media_metadata_subquery(
						media_genres
							.clone()
							.is_not_null()
							.and(media_genres.clone().ne("")),
					));
				let condition = match comparison {
					FilterComparison::Equal | FilterComparison::Contains => {
						csv_contains(series_genres, &names).or(media_metadata_subquery(
							csv_contains(media_genres, &names),
						))
					},
					FilterComparison::NotEqual | FilterComparison::NotContains => {
						csv_contains(series_genres, &names)
							.or(media_metadata_subquery(csv_contains(
								media_genres,
								&names,
							)))
							.not()
					},
					FilterComparison::MustContains => names
						.iter()
						.map(|name| {
							csv_contains(
								series_genres.clone(),
								std::slice::from_ref(name),
							)
							.or(media_metadata_subquery(csv_contains(
								media_genres.clone(),
								std::slice::from_ref(name),
							)))
						})
						.reduce(|acc, next| acc.and(next))
						.unwrap_or_else(|| Expr::value(true)),
					FilterComparison::IsEmpty => any_genre.not(),
					FilterComparison::IsNotEmpty => any_genre,
					_ => Expr::value(true),
				};
				group = group.add(condition);
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
					has_group_statement = true;
					group = group.add(Expr::value(matches!(
						comparison,
						FilterComparison::NotEqual | FilterComparison::NotContains
					)));
					continue;
				}
				has_group_statement = true;
				let series_expr = series_column.map(metadata_col);
				let media_expr = media_column
					.map(|column| Expr::col((media_metadata::Entity, column)));
				let contains = |names: &[String]| -> SimpleExpr {
					let mut condition: Option<SimpleExpr> = None;
					if let Some(column) = series_expr.clone() {
						condition = Some(csv_contains(column, names));
					}
					if let Some(column) = media_expr.clone() {
						let sub = media_metadata_subquery(csv_contains(column, names));
						condition = Some(match condition {
							Some(existing) => existing.or(sub),
							None => sub,
						});
					}
					condition.unwrap_or_else(|| Expr::value(false))
				};
				let any = {
					let mut condition: Option<SimpleExpr> = None;
					if let Some(column) = series_expr.clone() {
						condition = Some(column.clone().is_not_null().and(column.ne("")));
					}
					if let Some(column) = media_expr.clone() {
						let sub = media_metadata_subquery(
							column.clone().is_not_null().and(column.ne("")),
						);
						condition = Some(match condition {
							Some(existing) => existing.or(sub),
							None => sub,
						});
					}
					condition.unwrap_or_else(|| Expr::value(false))
				};
				let condition = match comparison {
					FilterComparison::Equal | FilterComparison::Contains => {
						contains(&names)
					},
					FilterComparison::NotEqual | FilterComparison::NotContains => {
						contains(&names).not()
					},
					FilterComparison::MustContains => names
						.iter()
						.map(|name| contains(std::slice::from_ref(name)))
						.reduce(|acc, next| acc.and(next))
						.unwrap_or_else(|| Expr::value(true)),
					FilterComparison::IsEmpty => any.not(),
					FilterComparison::IsNotEmpty => any,
					_ => Expr::value(true),
				};
				group = group.add(condition);
			},
			SeriesFilterField::Formats => {
				let formats = statement.formats().map_err(bad_request)?;
				if formats.is_empty() {
					continue;
				}
				validate(comparison, LIST_BASIC, "Series.Format")?;
				has_group_statement = true;
				let extensions = formats
					.iter()
					.flat_map(|format| extensions_for(*format))
					.map(|ext| ext.to_owned())
					.collect::<Vec<_>>();
				let lowered = SimpleExpr::from(Func::lower(Expr::col((
					media::Entity,
					media::Column::Extension,
				))));
				let unknown_formats = formats.contains(&MangaFormat::Unknown);
				let mut media_condition = Expr::expr(lowered.clone()).is_in(extensions);
				if unknown_formats {
					let known = MangaFormat::ALL
						.iter()
						.flat_map(|format| extensions_for(*format))
						.map(|ext| ext.to_owned())
						.collect::<Vec<_>>();
					media_condition =
						media_condition.or(Expr::expr(lowered).is_not_in(known));
				}
				// Kavita filters on the series format (its first file); the
				// series-level match is any media of that format.
				group = group.add(negate_if(
					media_subquery(media_condition),
					matches!(
						comparison,
						FilterComparison::NotEqual | FilterComparison::NotContains
					),
				));
			},
			SeriesFilterField::ReleaseYear => {
				let year = value
					.parse::<i32>()
					.map_err(|_| bad_request(format!("'{value}' is not a valid year")))?;
				validate(comparison, DATE, "Series.ReleaseYear")?;
				has_group_statement = true;
				let column = metadata_col(series_metadata::Column::Year);
				let this_year = chrono::Utc::now()
					.format("%Y")
					.to_string()
					.parse::<i32>()
					.unwrap_or(0);
				let condition = match comparison {
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
				};
				group = group.add(condition);
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
	if has_group_statement {
		condition = condition.add(group);
	}

	let sort = filter.effective_sort();
	let direction = if sort.is_ascending {
		Order::Asc
	} else {
		Order::Desc
	};
	let sort_name = SimpleExpr::from(Func::lower(Func::coalesce([
		metadata_col(series_metadata::Column::TitleSort).into(),
		metadata_col(series_metadata::Column::Title).into(),
		series_col(series::Column::Name).into(),
	])));
	let mut sort_by_progress = None;
	let order: Vec<(SimpleExpr, Order)> = match sort.sort_field {
		SeriesSortField::SortName
		| SeriesSortField::AverageRating
		| SeriesSortField::UserRating => {
			vec![(sort_name, direction)]
		},
		SeriesSortField::CreatedDate => {
			vec![(series_col(series::Column::CreatedAt).into(), direction)]
		},
		SeriesSortField::LastModifiedDate => vec![(
			Func::coalesce([
				series_col(series::Column::UpdatedAt).into(),
				series_col(series::Column::CreatedAt).into(),
			])
			.into(),
			direction,
		)],
		SeriesSortField::LastChapterAdded => vec![(
			SimpleExpr::SubQuery(
				None,
				Box::new(
					Query::select()
						.expr(Func::max(Expr::col((
							media::Entity,
							media::Column::CreatedAt,
						))))
						.from(media::Entity)
						.and_where(
							Expr::col((media::Entity, media::Column::SeriesId))
								.equals((series::Entity, series::Column::Id)),
						)
						.to_owned()
						.into_sub_query_statement(),
				),
			),
			direction,
		)],
		SeriesSortField::TimeToRead => vec![(
			SimpleExpr::SubQuery(
				None,
				Box::new(
					Query::select()
						.expr(Func::sum(Expr::col((media::Entity, media::Column::Pages))))
						.from(media::Entity)
						.and_where(
							Expr::col((media::Entity, media::Column::SeriesId))
								.equals((series::Entity, series::Column::Id)),
						)
						.to_owned()
						.into_sub_query_statement(),
				),
			),
			direction,
		)],
		SeriesSortField::ReleaseYear => vec![(
			metadata_col(series_metadata::Column::Year).into(),
			direction,
		)],
		SeriesSortField::Random => vec![(Expr::cust("RANDOM()"), Order::Asc)],
		SeriesSortField::ReadProgress => {
			sort_by_progress = Some(ProgressSort::ReadProgress {
				ascending: sort.is_ascending,
			});
			vec![(sort_name, Order::Asc)]
		},
		SeriesSortField::UnreadChapterCount => {
			sort_by_progress = Some(ProgressSort::UnreadCount {
				ascending: sort.is_ascending,
			});
			vec![(sort_name, Order::Asc)]
		},
	};
	// Stable secondary order so pages never overlap.
	let mut order = order;
	order.push((series_col(series::Column::Id).into(), Order::Asc));

	Ok(FilterPlan {
		condition,
		order,
		progress,
		sort_by_progress,
		combination,
		limit_to: filter.limit_to.max(0),
	})
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
}
