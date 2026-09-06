//! GraphQL surface of the per-field metadata policy
//! (`stump_ingest::policy`). The wire vocabulary is the public
//! [`MetadataField`] enum every other metadata mutation already speaks, so a
//! client never has to learn the smaller internal ingest field set.

use async_graphql::{Enum, SimpleObject, ID};
use metadata_integrations::MetadataField;
use stump_ingest::{
	contract::MetadataField as IngestField,
	policy::{
		EffectivePolicy, FieldDecision, FieldOutcome, FieldPolicy, PolicyPlan,
		PolicyStrategy,
	},
	providers::apply::STORABLE_FIELDS,
};

/// How the offers of the allowed providers for one field combine.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Enum)]
pub enum MetadataPolicyStrategy {
	First,
	MergeUnion,
	Longest,
	PreferExisting,
	HighestResolution,
}

impl From<PolicyStrategy> for MetadataPolicyStrategy {
	fn from(strategy: PolicyStrategy) -> Self {
		match strategy {
			PolicyStrategy::First => Self::First,
			PolicyStrategy::MergeUnion => Self::MergeUnion,
			PolicyStrategy::Longest => Self::Longest,
			PolicyStrategy::PreferExisting => Self::PreferExisting,
			PolicyStrategy::HighestResolution => Self::HighestResolution,
		}
	}
}

impl From<MetadataPolicyStrategy> for PolicyStrategy {
	fn from(strategy: MetadataPolicyStrategy) -> Self {
		match strategy {
			MetadataPolicyStrategy::First => Self::First,
			MetadataPolicyStrategy::MergeUnion => Self::MergeUnion,
			MetadataPolicyStrategy::Longest => Self::Longest,
			MetadataPolicyStrategy::PreferExisting => Self::PreferExisting,
			MetadataPolicyStrategy::HighestResolution => Self::HighestResolution,
		}
	}
}

/// What the policy decided for one field of one book.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Enum)]
pub enum MetadataPolicyOutcome {
	Candidate,
	Merged,
	Kept,
	Locked,
	Unmatched,
}

impl From<FieldOutcome> for MetadataPolicyOutcome {
	fn from(outcome: FieldOutcome) -> Self {
		match outcome {
			FieldOutcome::Candidate => Self::Candidate,
			FieldOutcome::Merged => Self::Merged,
			FieldOutcome::Kept => Self::Kept,
			FieldOutcome::Locked => Self::Locked,
			FieldOutcome::Unmatched => Self::Unmatched,
		}
	}
}

/// One field's effective rule.
#[derive(Debug, Clone, SimpleObject)]
pub struct MetadataFieldPolicy {
	pub field: MetadataField,
	/// Provider ids in priority order. Empty means no provider may fill this
	/// field.
	pub providers: Vec<String>,
	pub strategy: MetadataPolicyStrategy,
	/// Whether a field the user locked is left alone.
	pub lock_respected: bool,
	/// `true` when this rule comes from the library's override rather than
	/// the server default.
	pub overridden: bool,
	/// `false` for the candidate-only fields the staged apply path has no
	/// column for (`COVER`, `LINKS`, `STATUS`): the policy still resolves
	/// them, and the editor shows the winner, but applying writes nothing.
	pub storable: bool,
}

/// The effective policy of one library: the server default with the library's
/// override applied, one row per field the policy vocabulary has.
#[derive(Debug, Clone, SimpleObject)]
pub struct MetadataPolicy {
	pub library_id: ID,
	/// `true` when the library stores its own override document.
	pub has_library_override: bool,
	pub fields: Vec<MetadataFieldPolicy>,
}

impl From<EffectivePolicy> for MetadataPolicy {
	fn from(effective: EffectivePolicy) -> Self {
		let fields = IngestField::ALL
			.iter()
			.copied()
			.map(|field| {
				let rule = effective.policy.fields.get(&field);
				let default = FieldPolicy::default();
				let rule = rule.unwrap_or(&default);
				MetadataFieldPolicy {
					field: field.to_public(),
					providers: rule.providers.clone(),
					strategy: rule.strategy.into(),
					lock_respected: rule.lock_respected,
					overridden: effective.is_overridden(field),
					storable: STORABLE_FIELDS.contains(&field),
				}
			})
			.collect();
		Self {
			library_id: effective.library_id.into(),
			has_library_override: effective.library_override.is_some(),
			fields,
		}
	}
}

/// One field's resolution during an "apply best" run.
#[derive(Debug, Clone, SimpleObject)]
pub struct MetadataPolicyDecision {
	pub field: MetadataField,
	pub strategy: MetadataPolicyStrategy,
	pub outcome: MetadataPolicyOutcome,
	/// Providers that contributed, in the order they were used.
	pub providers: Vec<String>,
	/// Whether this decision wrote anything.
	pub applied: bool,
}

impl From<&FieldDecision> for MetadataPolicyDecision {
	fn from(decision: &FieldDecision) -> Self {
		Self {
			field: decision.field.to_public(),
			strategy: decision.strategy.into(),
			outcome: decision.outcome.into(),
			providers: decision.providers.clone(),
			applied: decision.applied,
		}
	}
}

/// Result of applying a library's policy to one target: the same payload the
/// hand-picked apply returns, plus the per-field explanation.
#[derive(Debug, Clone, SimpleObject)]
pub struct IngestApplyBestPayload {
	pub drop_item: Option<crate::object::ingest::IngestDropItem>,
	pub media: Option<crate::object::media::Media>,
	pub decisions: Vec<MetadataPolicyDecision>,
}

pub fn policy_decisions(plan: &PolicyPlan) -> Vec<MetadataPolicyDecision> {
	plan.decisions
		.iter()
		.map(MetadataPolicyDecision::from)
		.collect()
}
