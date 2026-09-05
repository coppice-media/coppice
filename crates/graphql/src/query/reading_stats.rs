use async_graphql::{Context, Object, Result};
use chrono::Utc;
use models::domain::reading_progress::calculate_logical_date;

use crate::{
	data::CoreContext,
	object::reading_stats::{ReadingStats, ReadingStatsSpan},
};

#[derive(Default)]
pub struct ReadingStatsQuery;

#[Object]
impl ReadingStatsQuery {
	/// Reading activity over `span`, for the current user's sessions. With a
	/// `deviceId` the stats cover the sessions that device contributed to, for
	/// the device's owner (the server owner may name any device).
	async fn reading_stats(
		&self,
		ctx: &Context<'_>,
		span: ReadingStatsSpan,
		device_id: Option<String>,
	) -> Result<ReadingStats> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let subject_user_id = match &device_id {
			Some(device_id) => core.devices().get(user, device_id).await?.user_id,
			None => user.id.clone(),
		};
		let day_reset_offset = user
			.preferences
			.as_ref()
			.map_or(0, |preferences| preferences.day_reset_hour_offset);
		let today = calculate_logical_date(Utc::now(), day_reset_offset);

		ReadingStats::fetch(
			core.conn.as_ref(),
			&subject_user_id,
			today,
			span,
			device_id.as_deref(),
		)
		.await
	}
}
