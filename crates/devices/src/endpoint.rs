use models::shared::enums::DeviceKind;
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
			DeviceKind::Kobo => vec![keyed(
				"Kobo sync (api_endpoint)",
				format!("{base}/kobo/{secret}"),
			)],
			DeviceKind::Koreader => vec![with_user(
				"KOReader progress sync server",
				format!("{base}/koreader/{secret}"),
			)],
			DeviceKind::Opds => vec![
				keyed("OPDS 1.2 catalog", format!("{base}/opds/{secret}/v1.2")),
				with_user("OPDS 2.0 catalog (Bearer)", format!("{base}/opds/v2.0")),
			],
			DeviceKind::Mihon => vec![
				with_user("Komga server (X-API-Key)", base.clone()),
				keyed("Stump API (Bearer)", format!("{base}/api")),
			],
			DeviceKind::Komelia => vec![with_user("Komga server (X-API-Key)", base)],
			DeviceKind::Liseur => vec![with_user("liseur-sync server", base)],
			DeviceKind::Api | DeviceKind::Web => {
				vec![keyed("Stump API (Bearer)", format!("{base}/api"))]
			},
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
}
