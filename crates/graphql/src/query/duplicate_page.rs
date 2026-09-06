use std::collections::{BTreeSet, HashMap};

use async_graphql::{Context, Object, Result, ID};
use models::{
	entity::{known_duplicate_page, library, media, page_hash, series},
	shared::enums::UserPermission,
};
use sea_orm::{
	prelude::*,
	sea_query::{ExprTrait, Func, SimpleExpr},
	DatabaseConnection, FromQueryResult, JoinType, QueryOrder, QuerySelect,
};
use stump_core::{
	filesystem::media::visible_pages::{
		matches_any, visible_pages, VisiblePages, DUPLICATE_PAGE_TOLERANCE,
	},
	ingest::quality::duplicate_pages_across_books::{
		dhash_hex, MIN_DUPLICATE_BOOKS_DEFAULT,
	},
};

use crate::{
	data::CoreContext,
	guard::PermissionGuard,
	object::duplicate_page::{
		DuplicatePageCandidate, DuplicatePageOccurrence, KnownDuplicatePage,
	},
};

/// Default and hard cap on the candidate groups returned by one query.
const DEFAULT_CANDIDATE_LIMIT: usize = 50;
const MAX_CANDIDATE_LIMIT: usize = 500;

#[derive(Default)]
pub struct DuplicatePageQuery;

#[Object]
impl DuplicatePageQuery {
	/// Every reviewed page hash (`SKIP`/`KEEP`) of a library, newest first.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn known_duplicate_pages(
		&self,
		ctx: &Context<'_>,
		library_id: ID,
	) -> Result<Vec<KnownDuplicatePage>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		assert_library_access(conn, user, &library_id).await?;

		let rows = known_duplicate_page::Entity::find()
			.filter(known_duplicate_page::Column::LibraryId.eq(library_id.to_string()))
			.order_by_desc(known_duplicate_page::Column::CreatedAt)
			.all(conn)
			.await?;
		Ok(rows.into_iter().map(KnownDuplicatePage::from).collect())
	}

	/// Page hashes that recur in at least `minBooks` distinct books of the
	/// library and have not been reviewed yet, most widespread first.
	/// Hashes within the duplicate tolerance of each other are grouped.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn duplicate_page_candidates(
		&self,
		ctx: &Context<'_>,
		library_id: ID,
		#[graphql(
			desc = "Minimum distinct books; defaults to the quality check default"
		)]
		min_books: Option<i32>,
		#[graphql(desc = "Maximum groups returned; defaults to 50, capped at 500")]
		limit: Option<i32>,
	) -> Result<Vec<DuplicatePageCandidate>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();
		assert_library_access(conn, user, &library_id).await?;

		let min_books = min_books
			.map(i64::from)
			.filter(|value| *value >= 2)
			.unwrap_or(MIN_DUPLICATE_BOOKS_DEFAULT);
		let limit = limit
			.and_then(|value| usize::try_from(value).ok())
			.filter(|value| *value > 0)
			.unwrap_or(DEFAULT_CANDIDATE_LIMIT)
			.min(MAX_CANDIDATE_LIMIT);

		let known = known_duplicate_page::Entity::find()
			.select_only()
			.column(known_duplicate_page::Column::Dhash)
			.filter(known_duplicate_page::Column::LibraryId.eq(library_id.to_string()))
			.into_tuple::<i64>()
			.all(conn)
			.await?;

		let groups = recurring_hashes(conn, &library_id).await?;
		let clusters = cluster_hashes(groups, &known, min_books);
		if clusters.is_empty() {
			return Ok(Vec::new());
		}

		let wanted = clusters
			.iter()
			.flat_map(|cluster| cluster.hashes.iter().copied())
			.collect::<Vec<_>>();
		let rows = page_hash::Entity::find()
			.find_also_related(media::Entity)
			.join(JoinType::InnerJoin, media::Relation::Series.def())
			.filter(series::Column::LibraryId.eq(library_id.to_string()))
			.filter(media::Column::DeletedAt.is_null())
			.filter(page_hash::Column::Dhash.is_in(wanted))
			.order_by_asc(media::Column::Name)
			.order_by_asc(page_hash::Column::Page)
			.all(conn)
			.await?;

		let mut by_hash: HashMap<i64, Vec<(page_hash::Model, media::Model)>> =
			HashMap::new();
		for (hash, media) in rows {
			if let Some(media) = media {
				by_hash.entry(hash.dhash).or_default().push((hash, media));
			}
		}

		let cache = core.visible_pages_cache();
		let mut visible_by_media: HashMap<String, VisiblePages> = HashMap::new();
		let mut candidates = Vec::with_capacity(clusters.len());
		for cluster in clusters {
			let mut books = BTreeSet::new();
			let mut occurrences = Vec::new();
			for hash in &cluster.hashes {
				for (row, book) in by_hash.remove(hash).unwrap_or_default() {
					books.insert(book.id.clone());
					let visible = match visible_by_media.get(&book.id) {
						Some(visible) => visible.clone(),
						None => {
							let visible =
								visible_pages(conn, &cache, &book.id, book.pages)
									.await
									.map_err(crate::error::map_core_error)?;
							visible_by_media.insert(book.id.clone(), visible.clone());
							visible
						},
					};
					occurrences.push(DuplicatePageOccurrence {
						media_id: ID(book.id),
						media_name: book.name,
						page: row.page,
						visible_page: visible
							.iter()
							.position(|page| *page == row.page)
							.map(|index| index as i32 + 1),
						dhash: dhash_hex(row.dhash),
					});
				}
			}
			if (books.len() as i64) < min_books {
				continue;
			}
			candidates.push(DuplicatePageCandidate {
				dhash: dhash_hex(cluster.representative),
				book_count: books.len() as i32,
				page_count: occurrences.len() as i32,
				occurrences,
			});
		}
		candidates.sort_by(|left, right| {
			right
				.book_count
				.cmp(&left.book_count)
				.then_with(|| right.page_count.cmp(&left.page_count))
				.then_with(|| left.dhash.cmp(&right.dhash))
		});
		candidates.truncate(limit);
		Ok(candidates)
	}

	/// The physical 1-based pages of a book that remain visible once the
	/// library's `SKIP` marks are applied; index `n - 1` is the physical page
	/// served for visible page `n`.
	async fn media_visible_pages(&self, ctx: &Context<'_>, id: ID) -> Result<Vec<i32>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();

		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(id.to_string()))
			.into_model::<media::Model>()
			.one(conn)
			.await?
			.ok_or("Media not found")?;
		let cache = core.visible_pages_cache();
		let visible = visible_pages(conn, &cache, &book.id, book.pages)
			.await
			.map_err(crate::error::map_core_error)?;
		Ok(visible.to_vec())
	}
}

pub(crate) async fn assert_library_access(
	conn: &DatabaseConnection,
	user: &models::entity::user::AuthUser,
	library_id: &ID,
) -> Result<()> {
	library::Entity::find_for_user(user)
		.filter(library::Column::Id.eq(library_id.to_string()))
		.into_model::<library::LibraryIdentSelect>()
		.one(conn)
		.await?
		.ok_or("Library not found")?;
	Ok(())
}

#[derive(Debug, FromQueryResult)]
struct HashGroup {
	dhash: i64,
	books: i64,
}

/// Every hash that occurs in at least two distinct books of the library, with
/// its distinct-book count, most widespread first.
async fn recurring_hashes(
	conn: &DatabaseConnection,
	library_id: &ID,
) -> Result<Vec<HashGroup>> {
	let distinct_books: SimpleExpr =
		Func::count_distinct(Expr::col((page_hash::Entity, page_hash::Column::MediaId)))
			.into();
	Ok(page_hash::Entity::find()
		.select_only()
		.column(page_hash::Column::Dhash)
		.column_as(distinct_books.clone(), "books")
		.join(JoinType::InnerJoin, page_hash::Relation::Media.def())
		.join(JoinType::InnerJoin, media::Relation::Series.def())
		.filter(series::Column::LibraryId.eq(library_id.to_string()))
		.filter(media::Column::DeletedAt.is_null())
		.group_by(page_hash::Column::Dhash)
		.having(distinct_books.gte(2))
		.order_by_desc(Expr::cust("books"))
		.into_model::<HashGroup>()
		.all(conn)
		.await?)
}

#[derive(Debug)]
struct Cluster {
	representative: i64,
	hashes: Vec<i64>,
	/// Upper bound on distinct books (sum of member counts).
	max_books: i64,
}

/// Greedily groups near-identical hashes (Hamming distance within
/// [`DUPLICATE_PAGE_TOLERANCE`]) around the most widespread member, drops
/// groups already covered by a known mark, and keeps groups that may reach
/// `min_books`. The exact distinct-book count is settled from occurrences.
fn cluster_hashes(groups: Vec<HashGroup>, known: &[i64], min_books: i64) -> Vec<Cluster> {
	let mut clusters: Vec<Cluster> = Vec::new();
	for group in groups {
		match clusters.iter_mut().find(|cluster| {
			stump_media::hamming(cluster.representative as u64, group.dhash as u64)
				<= DUPLICATE_PAGE_TOLERANCE
		}) {
			Some(cluster) => {
				cluster.hashes.push(group.dhash);
				cluster.max_books += group.books;
			},
			None => clusters.push(Cluster {
				representative: group.dhash,
				hashes: vec![group.dhash],
				max_books: group.books,
			}),
		}
	}
	clusters.retain(|cluster| {
		cluster.max_books >= min_books && !matches_any(cluster.representative, known)
	});
	clusters
}

#[cfg(test)]
mod tests {
	use super::*;

	fn group(dhash: i64, books: i64) -> HashGroup {
		HashGroup { dhash, books }
	}

	#[test]
	fn clusters_near_hashes_around_most_widespread() {
		let clusters = cluster_hashes(
			vec![
				group(0b1111_0000, 3),
				group(0b1111_0001, 1),
				group(0b0000_1111, 2),
			],
			&[],
			3,
		);
		assert_eq!(clusters.len(), 1);
		assert_eq!(clusters[0].representative, 0b1111_0000);
		assert_eq!(clusters[0].hashes, vec![0b1111_0000, 0b1111_0001]);
		assert_eq!(clusters[0].max_books, 4);
	}

	#[test]
	fn drops_clusters_covered_by_known_marks() {
		let clusters = cluster_hashes(vec![group(0xff, 5), group(0xff00, 5)], &[0xfe], 2);
		assert_eq!(clusters.len(), 1);
		assert_eq!(clusters[0].representative, 0xff00);
	}

	#[test]
	fn drops_clusters_below_min_books() {
		assert!(cluster_hashes(vec![group(1, 2)], &[], 3).is_empty());
	}
}
