use models::shared::enums::{DeviceKind, DeviceProtocol};
use serde::Serialize;
use stump_api_types::RequestOrigin;

/// One thing a client has to be configured with: a URL plus, when the protocol
/// wants them, a username and the device secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "DeviceEndpoint"))]
pub struct Endpoint {
	pub label: String,
	pub url: String,
	pub username: Option<String>,
	/// The device secret when it is known (right after minting), otherwise a
	/// redacted hint such as `stump_abcdefgh_…`.
	pub secret_hint: String,
}

impl Endpoint {
	/// The endpoints a device of `kind` uses against `origin`. `secret` is the
	/// full credential when the caller just minted it, or a redacted hint; it is
	/// embedded wherever the protocol carries the key in the URL.
	pub fn for_kind(
		kind: DeviceKind,
		origin: &RequestOrigin,
		username: &str,
		secret: &str,
	) -> Vec<Endpoint> {
		let base = origin.url();
		let with_user = |label: &str, url: String| Endpoint {
			label: label.to_string(),
			url,
			username: Some(username.to_string()),
			secret_hint: secret.to_string(),
		};
		let keyed = |label: &str, url: String| Endpoint {
			label: label.to_string(),
			url,
			username: None,
			secret_hint: secret.to_string(),
		};

		match kind {
			DeviceKind::Coppice => Self::for_credential(
				DeviceKind::Coppice,
				DeviceProtocol::Koreader,
				origin,
				username,
				secret,
			),
			DeviceKind::Kobo => vec![keyed(
				"Kobo sync (api_endpoint)",
				format!("{base}/kobo/{secret}"),
			)],
			DeviceKind::Koreader => vec![with_user(
				"KOReader progress sync server",
				format!("{base}/koreader/{secret}"),
			)],
			DeviceKind::Crosspoint => vec![with_user(
				"CrossPoint KOSync + rich sync",
				format!("{base}/koreader/{secret}"),
			)],
			DeviceKind::Opds => vec![
				keyed("OPDS 1.2 catalog", format!("{base}/opds/{secret}/v1.2")),
				with_user("OPDS 2.0 catalog (Bearer)", format!("{base}/opds/v2.0")),
			],
			DeviceKind::Mihon => vec![
				with_user("Komga server (X-API-Key)", base.clone()),
				keyed("Coppice API (Bearer)", format!("{base}/api")),
			],
			DeviceKind::Komelia => vec![with_user("Komga server (X-API-Key)", base)],
			DeviceKind::Kavita => vec![keyed("Kavita server", base)],
			DeviceKind::Liseur => vec![with_user("liseur-sync server", base)],
			// An Audiobookshelf client is pointed at the server root and
			// logs in with a username and password; the API key works as a
			// bearer token for clients that skip the login.
			DeviceKind::Abs => vec![with_user("Audiobookshelf server", base)],
			// The compute and source roles use separate keys and sockets even
			// though the same worker binary may implement both.
			DeviceKind::Worker => vec![keyed("stump-worker --server", base)],
			DeviceKind::SourceWorker => {
				vec![keyed("stump-worker --server --source-api-key", base)]
			},
			DeviceKind::Api | DeviceKind::Web => {
				vec![keyed("Coppice API (Bearer)", format!("{base}/api"))]
			},
		}
	}

	/// Returns the endpoint set for one explicit Coppice credential.
	///
	/// Coppice deliberately has separate API/KOSync and liseur-sync lanes. The
	/// credential metadata chooses the lane, so callers never have to infer it
	/// from the order of a credential list.
	pub fn for_credential(
		kind: DeviceKind,
		protocol: DeviceProtocol,
		origin: &RequestOrigin,
		username: &str,
		secret: &str,
	) -> Vec<Endpoint> {
		if kind != DeviceKind::Coppice {
			return Self::for_kind(kind, origin, username, secret);
		}

		let base = origin.url();
		match protocol {
			DeviceProtocol::Koreader => vec![
				Endpoint {
					label: "Coppice KOReader progress sync".to_string(),
					url: format!("{base}/koreader/{secret}"),
					username: Some(username.to_string()),
					secret_hint: secret.to_string(),
				},
				Endpoint {
					label: "Coppice API (Bearer)".to_string(),
					url: format!("{base}/api"),
					username: None,
					secret_hint: secret.to_string(),
				},
				Endpoint {
					label: "Coppice OPDS 1.2 catalog".to_string(),
					url: format!("{base}/opds/{secret}/v1.2"),
					username: None,
					secret_hint: secret.to_string(),
				},
				Endpoint {
					label: "Coppice OPDS 2.0 catalog (Basic)".to_string(),
					url: format!("{base}/opds/v2.0"),
					username: Some(username.to_string()),
					secret_hint: secret.to_string(),
				},
			],
			DeviceProtocol::Liseur => vec![Endpoint {
				label: "Coppice annotations/read-state (liseur-sync)".to_string(),
				url: format!("{base}/v1"),
				username: None,
				secret_hint: secret.to_string(),
			}],
			_ => Vec::new(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn origin() -> RequestOrigin {
		RequestOrigin::new("stump.example".to_string(), "https".to_string())
	}

	#[test]
	fn kobo_endpoint_embeds_key_in_path() {
		let endpoints =
			Endpoint::for_kind(DeviceKind::Kobo, &origin(), "al", "stump_a_b");
		assert_eq!(endpoints.len(), 1);
		assert_eq!(endpoints[0].url, "https://stump.example/kobo/stump_a_b");
		assert_eq!(endpoints[0].username, None);
		assert_eq!(endpoints[0].secret_hint, "stump_a_b");
	}

	#[test]
	fn koreader_and_komga_endpoints_carry_username() {
		let koreader =
			Endpoint::for_kind(DeviceKind::Koreader, &origin(), "al", "stump_a_b");
		assert_eq!(koreader[0].url, "https://stump.example/koreader/stump_a_b");
		assert_eq!(koreader[0].username.as_deref(), Some("al"));

		let komelia = Endpoint::for_kind(DeviceKind::Komelia, &origin(), "al", "k");
		assert_eq!(komelia[0].url, "https://stump.example");
		assert_eq!(komelia[0].username.as_deref(), Some("al"));
	}

	#[test]
	fn opds_lists_both_catalog_versions() {
		let endpoints = Endpoint::for_kind(DeviceKind::Opds, &origin(), "al", "k");
		let urls: Vec<&str> = endpoints.iter().map(|e| e.url.as_str()).collect();
		assert_eq!(
			urls,
			vec![
				"https://stump.example/opds/k/v1.2",
				"https://stump.example/opds/v2.0"
			]
		);
	}

	#[test]
	fn liseur_and_api_endpoints() {
		let liseur = Endpoint::for_kind(DeviceKind::Liseur, &origin(), "al", "liseur-x");
		assert_eq!(liseur[0].url, "https://stump.example");
		let api = Endpoint::for_kind(DeviceKind::Api, &origin(), "al", "k");
		assert_eq!(api[0].url, "https://stump.example/api");
		assert_eq!(api[0].username, None);
	}

	#[test]
	fn coppice_credentials_select_their_endpoint_lane() {
		let api = Endpoint::for_credential(
			DeviceKind::Coppice,
			DeviceProtocol::Koreader,
			&origin(),
			"al",
			"stump_api",
		);
		assert!(api.iter().any(|endpoint| {
			endpoint.url == "https://stump.example/koreader/stump_api"
				&& endpoint.username.as_deref() == Some("al")
		}));
		assert!(api.iter().any(|endpoint| {
			endpoint.url == "https://stump.example/api" && endpoint.username.is_none()
		}));

		let liseur = Endpoint::for_credential(
			DeviceKind::Coppice,
			DeviceProtocol::Liseur,
			&origin(),
			"al",
			"liseur_token",
		);
		assert_eq!(liseur.len(), 1);
		assert_eq!(liseur[0].url, "https://stump.example/v1");
		assert_eq!(liseur[0].username, None);
		assert_eq!(liseur[0].secret_hint, "liseur_token");
	}
	#[test]
	fn source_worker_endpoint_is_keyed_and_separate() {
		let endpoints =
			Endpoint::for_kind(DeviceKind::SourceWorker, &origin(), "al", "stump_source");
		assert_eq!(endpoints.len(), 1);
		assert_eq!(endpoints[0].url, "https://stump.example");
		assert_eq!(endpoints[0].username, None);
		assert_eq!(endpoints[0].secret_hint, "stump_source");
		assert!(endpoints[0].label.contains("source-api-key"));
	}
}
