//! `FilterController`: smart filters. Stump stores none, so the list is
//! empty and `decode` only parses the encoded string.

use axum::{
	routing::{get, post},
	Json, Router,
};
use stump_auth::AuthContext;

use crate::{
	dto::{DecodeFilterDto, SmartFilterDto},
	errors::{APIError, APIResult},
	filter::{decode_series_filter, SeriesFilterV2Dto},
};

use super::route_ci;

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = route_ci(Router::<S>::new(), "/api/Filter", get(list_filters));
	route_ci(router, "/api/Filter/decode", post(decode_filter))
}

async fn list_filters(
	axum::Extension(_auth): axum::Extension<AuthContext>,
) -> Json<Vec<SmartFilterDto>> {
	Json(Vec::new())
}

async fn decode_filter(
	axum::Extension(_auth): axum::Extension<AuthContext>,
	Json(body): Json<DecodeFilterDto>,
) -> APIResult<Json<SeriesFilterV2Dto>> {
	let decoded =
		decode_series_filter(body.encoded_filter.as_deref().unwrap_or_default())
			.map_err(APIError::BadRequest)?;
	Ok(Json(decoded))
}
