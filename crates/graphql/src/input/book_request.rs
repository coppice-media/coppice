use async_graphql::{Enum, InputObject, ID};

/// The requested release lane. Existing and unspecified requests accept any
/// format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Enum)]
pub enum RequestFormat {
	#[graphql(name = "EBOOK")]
	Ebook,
	#[graphql(name = "AUDIOBOOK")]
	Audiobook,
	#[graphql(name = "ANY")]
	#[default]
	Any,
}

impl RequestFormat {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Ebook => "EBOOK",
			Self::Audiobook => "AUDIOBOOK",
			Self::Any => "ANY",
		}
	}
}

impl From<&str> for RequestFormat {
	fn from(value: &str) -> Self {
		match value {
			"EBOOK" => Self::Ebook,
			"AUDIOBOOK" => Self::Audiobook,
			_ => Self::Any,
		}
	}
}

/// A catalog/work identity outside the Coppice library, stored as an immutable
/// metadata snapshot on the user's request.
#[derive(Debug, Clone, InputObject)]
pub struct ExternalWorkReferenceInput {
	pub source_provider: String,
	pub remote_id: String,
	pub external_key: Option<String>,
	pub title: String,
	pub authors: Option<String>,
	pub cover_url: Option<String>,
}

#[derive(Debug, Clone, InputObject)]
pub struct CreateBookRequestInput {
	pub media_id: Option<ID>,
	pub work_id: Option<ID>,
	pub external: Option<ExternalWorkReferenceInput>,
	pub title: Option<String>,
	pub authors: Option<String>,
	#[graphql(default_with = "RequestFormat::Any")]
	pub format: RequestFormat,
	pub isbn: Option<String>,
	pub cover_url: Option<String>,
	pub destination_shelf_id: Option<ID>,
	pub destination_device_id: Option<ID>,
	/// Biases audiobook release ranking toward this reader; ignored for
	/// ebook-only requests and never a filter.
	pub preferred_narrator: Option<String>,
}

#[cfg(test)]
mod tests {
	use super::RequestFormat;

	#[test]
	fn request_format_stores_the_contract_values_and_defaults_to_any() {
		assert_eq!(RequestFormat::default(), RequestFormat::Any);
		assert_eq!(RequestFormat::Ebook.as_str(), "EBOOK");
		assert_eq!(RequestFormat::Audiobook.as_str(), "AUDIOBOOK");
		assert_eq!(RequestFormat::from("ANY"), RequestFormat::Any);
	}
}
