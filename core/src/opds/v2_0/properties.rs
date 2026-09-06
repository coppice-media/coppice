//! A module for representing properties in an OPDS 2.0 feed. OPDS 2.0 properties do not have an explicit
//! section in the spec, but are used throughout. This module is an interpretation of the examples given
//! in the spec.

use derive_builder::Builder;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use super::link::OPDSLinkType;

/// A struct for representing dynamic properties of an OPDS feed or collection. This is just
/// a wrapper around a [`serde_json::Value`], which can be used to store any arbitrary JSON data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OPDSDynamicProperties(pub serde_json::Value);

/// The route for the authentication document for Stump's OPDS 2.0 implementation
pub const AUTH_ROUTE: &str = "/opds/v2.0/auth";

/// A struct for representing properties of an OPDS feed or collection
#[skip_serializing_none]
#[derive(Debug, Default, Builder, Clone, Serialize, Deserialize)]
#[builder(build_fn(error = "crate::CoreError"), default, setter(into))]
pub struct OPDSProperties {
	/// The URI of the authentication document
	pub authenticate: Option<OPDSAuthenticateProperties>,
	#[serde(flatten)]
	pub dynamic_properties: Option<OPDSDynamicProperties>,
}

impl OPDSProperties {
	/// Create a new [`OPDSProperties`] object with the given authentication URL
	pub fn with_auth(self, url: String) -> Self {
		Self {
			authenticate: Some(OPDSAuthenticateProperties::new(url)),
			..self
		}
	}

	/// Advertise the length of the linked resource in octets.
	///
	/// OPDS 2.0 has no length key of its own, but Atom's `link@length`
	/// (RFC 4287 4.2.7.5) is exactly this advisory value and the OPDS 1.2
	/// feed already speaks it, so the JSON feed reuses the name inside the
	/// open `properties` object rather than coining a Stump-only one. An
	/// audiobook is the first publication in the feed made of many files,
	/// and a client choosing which tracks to download needs their sizes.
	pub fn with_length(self, length: i64) -> Self {
		let Self {
			authenticate,
			dynamic_properties,
		} = self;

		// A dynamic value that is not an object cannot be flattened into
		// `properties` at all, so there is nothing to preserve in that case.
		let mut properties = match dynamic_properties {
			Some(OPDSDynamicProperties(serde_json::Value::Object(properties))) => {
				properties
			},
			_ => serde_json::Map::new(),
		};
		properties.insert(String::from("length"), serde_json::Value::from(length));

		Self {
			authenticate,
			dynamic_properties: Some(OPDSDynamicProperties(serde_json::Value::Object(
				properties,
			))),
		}
	}
}

/// A struct for representing auth-related properties in an OPDS feed or collection. This
/// instructs the client on how to authenticate with the server for a given OPDS item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OPDSAuthenticateProperties {
	/// The URI of the authentication document
	pub href: String,
	/// The type of the link
	#[serde(rename = "type")]
	_type: OPDSLinkType,
}

impl Default for OPDSAuthenticateProperties {
	fn default() -> Self {
		Self {
			href: String::from(AUTH_ROUTE),
			_type: OPDSLinkType::OpdsAuthJson,
		}
	}
}

impl OPDSAuthenticateProperties {
	pub fn new(href: String) -> Self {
		Self {
			href,
			_type: OPDSLinkType::OpdsAuthJson,
		}
	}

	pub fn document(href: String) -> Self {
		Self {
			href,
			_type: OPDSLinkType::OpdsAuthDocument,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_opds_properties() {
		let properties = OPDSProperties::default().with_auth(AUTH_ROUTE.to_string());
		assert_eq!(properties.authenticate.unwrap().href, AUTH_ROUTE);
	}

	#[test]
	fn test_default_type() {
		let properties = OPDSAuthenticateProperties::default();
		assert_eq!(properties._type, OPDSLinkType::OpdsAuthJson);
	}

	#[test]
	fn test_document_type() {
		let properties = OPDSAuthenticateProperties::document(AUTH_ROUTE.to_string());
		assert_eq!(properties._type, OPDSLinkType::OpdsAuthDocument);
	}
}
