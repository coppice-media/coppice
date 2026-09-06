//! Input side of the per-field metadata policy. A `setMetadataPolicy` call
//! carries the library's *whole* override document: an empty field list and
//! no audio section clears it and the library goes back to inheriting the
//! server default, which is the only unambiguous way to express "stop
//! overriding this" in one mutation.

use async_graphql::InputObject;
use metadata_integrations::MetadataField;
use stump_ingest::{
	contract::MetadataField as IngestField,
	policy::{AudioPolicy, FieldPolicy, MetadataPolicy},
};

use crate::object::metadata_policy::MetadataPolicyStrategy;

#[derive(Debug, Clone, InputObject)]
pub struct MetadataFieldPolicyInput {
	pub field: MetadataField,
	/// Provider ids in priority order. An empty list means no provider may
	/// fill this field.
	pub providers: Vec<String>,
	pub strategy: MetadataPolicyStrategy,
	/// Leave a user-locked field alone. Defaults to `true`; turning it off is
	/// how a library says "the policy outranks my locks for this field".
	#[graphql(default = true)]
	pub lock_respected: bool,
}

/// The audio section of an override. Every key has the server default as its
/// GraphQL default, so a client that wants to change one toggle sends one
/// key rather than restating the section.
#[derive(Debug, Clone, InputObject)]
pub struct MetadataAudioPolicyInput {
	#[graphql(default = 30)]
	pub single_file_weight: u16,
	#[graphql(default = false)]
	pub auto_assemble: bool,
	#[graphql(default = false)]
	pub auto_chapters: bool,
	/// Turning this off is refused unless `autoAssemble` is on: there would
	/// be nothing to replace the source files with.
	#[graphql(default = true)]
	pub keep_original: bool,
}

impl From<MetadataAudioPolicyInput> for AudioPolicy {
	fn from(input: MetadataAudioPolicyInput) -> Self {
		Self {
			single_file_weight: input.single_file_weight,
			auto_assemble: input.auto_assemble,
			auto_chapters: input.auto_chapters,
			keep_original: input.keep_original,
		}
	}
}

#[derive(Debug, Clone, InputObject)]
pub struct MetadataPolicyInput {
	/// The complete override. Empty, with no `audio`, clears the library
	/// override.
	pub fields: Vec<MetadataFieldPolicyInput>,
	/// The audio section. Omitted leaves the library inheriting the server
	/// default audio rules, exactly as an unmentioned field does.
	pub audio: Option<MetadataAudioPolicyInput>,
}

impl MetadataPolicyInput {
	/// `None` when the input clears the override.
	pub fn into_policy(self) -> Result<Option<MetadataPolicy>, String> {
		if self.fields.is_empty() && self.audio.is_none() {
			return Ok(None);
		}
		let mut policy = MetadataPolicy {
			audio: self.audio.map(AudioPolicy::from),
			..MetadataPolicy::default()
		};
		for input in self.fields {
			let field = IngestField::from_public(input.field).ok_or_else(|| {
				format!(
					"metadata field {:?} cannot be the target of a policy rule",
					input.field
				)
			})?;
			let rule = FieldPolicy {
				providers: input.providers,
				strategy: input.strategy.into(),
				lock_respected: input.lock_respected,
			};
			// Two public fields that fold onto the same ingest field (e.g.
			// WRITERS and ARTISTS -> AUTHORS) would otherwise silently
			// overwrite each other's rule.
			if let Some(previous) = policy.fields.insert(field, rule.clone()) {
				if previous != rule {
					return Err(format!(
						"conflicting rules for {field:?}: two input fields map onto it"
					));
				}
			}
		}
		Ok(Some(policy))
	}
}
