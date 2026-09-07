//! Reading a pair's chapter map.
//!
//! A pair has no id of its own — it is two `liseur_sync_media_links` rows
//! sharing a work — so the map is addressed by the two media rows it belongs
//! to, ebook first. That is also the only order the entries mean anything in:
//! the ebook is the canonical text and owns the spine index.

use async_graphql::{Context, Object, Result, ID};
use stump_library::editions;

use crate::{data::CoreContext, object::edition_pair::ChapterMapEntry};

#[derive(Default)]
pub struct EditionPairQuery;

#[Object]
impl EditionPairQuery {
	/// The tier-1 chapter map of one ebook↔audiobook pair, in spine order.
	///
	/// Empty when the pair has no map yet, which is also what the readiness
	/// checks read to say a book is not ready for read-aloud.
	async fn chapter_map(
		&self,
		ctx: &Context<'_>,
		#[graphql(desc = "The ebook edition; it owns the spine index")]
		ebook_media_id: ID,
		#[graphql(desc = "The audio edition; it owns the chapter index")]
		audio_media_id: ID,
	) -> Result<Vec<ChapterMapEntry>> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		Ok(
			editions::chapter_map(conn, ebook_media_id.as_str(), audio_media_id.as_str())
				.await?
				.into_iter()
				.map(ChapterMapEntry::from)
				.collect(),
		)
	}
}
