use axum::{
	extract::OriginalUri,
	response::{IntoResponse, Redirect},
	routing::get,
	Router,
};

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
};

/// Prefixes whose unknown routes must remain 404s rather than becoming browser redirects.
/// `/v1/` is liseur-sync (Liseur retries its `/v1/events` stream forever on
/// anything but 401/403/404/501), `/kobo/` the Kobo store API and `/komga/`
/// the Grimmory profile; a 303 to the HTML shell is never a valid client answer.
const NON_BROWSER_PREFIXES: [&str; 7] = [
	"/api/",
	"/opds/",
	"/public/",
	"/koreader/",
	"/v1/",
	"/kobo/",
	"/komga/",
];

pub(crate) fn mount() -> Router<AppState> {
	Router::new()
		.route("/", get(redirect_home))
		.fallback(redirect_unknown_browser_path)
}

async fn redirect_home() -> Redirect {
	Redirect::to("/app")
}

async fn redirect_unknown_browser_path(
	OriginalUri(uri): OriginalUri,
) -> APIResult<impl IntoResponse> {
	let path = uri.path();
	if NON_BROWSER_PREFIXES
		.iter()
		.any(|prefix| path.starts_with(prefix))
	{
		return Err(APIError::NotFound(format!("No route for {path}")));
	}

	Ok(Redirect::to("/app"))
}
