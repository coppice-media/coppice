#[cfg(feature = "abs")]
mod abs;
#[cfg(feature = "graphql")]
mod audio;
mod common;
#[cfg(feature = "graphql")]
mod device_pairing;
#[cfg(feature = "graphql")]
mod device_security;
#[cfg(all(feature = "graphql", feature = "liseur-sync"))]
mod device_touch;
#[cfg(feature = "readium")]
mod epub;
#[cfg(feature = "graphql")]
mod graphql;
mod kindle;
#[cfg(feature = "kobo")]
mod kobo;
#[cfg(feature = "komf")]
mod komf;
#[cfg(feature = "komga")]
mod komga;
#[cfg(feature = "koreader")]
mod koreader;
#[cfg(any(feature = "komga", feature = "kavita", feature = "opds"))]
mod library_scope;
#[cfg(feature = "liseur-sync")]
mod liseur;
#[cfg(feature = "opds")]
mod opds;
#[cfg(feature = "graphql")]
mod reading_progress;
#[cfg(feature = "webui")]
mod webui;
#[cfg(feature = "graphql")]
mod worker;

use common::TestApp;

/// server should start, the first user can register and login successfully
#[tokio::test]
async fn test_server_boots_and_auth_works() {
	let app = TestApp::new().await;
	let token = app.create_initial_account().await;
	assert!(!token.is_empty(), "expected a non-empty access token");
}
