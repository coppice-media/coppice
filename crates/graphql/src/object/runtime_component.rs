use async_graphql::{Enum, SimpleObject};
use chrono::{DateTime, FixedOffset};
use models::shared::enums::{DeviceKind, DeviceProtocol};
use stump_core::component_runtime::{
	GaugeKind, ProcessMemory, RuntimeComponent as CoreRuntimeComponent,
	RuntimeGauge as CoreRuntimeGauge, TransitionMode, UsageStatus,
};

/// Whether a component toggle takes effect immediately or at process restart.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Enum)]
pub enum RuntimeTransitionMode {
	#[graphql(name = "HOT")]
	Hot,
	#[graphql(name = "RESTART")]
	Restart,
}

impl From<TransitionMode> for RuntimeTransitionMode {
	fn from(value: TransitionMode) -> Self {
		match value {
			TransitionMode::Hot => Self::Hot,
			TransitionMode::Restart => Self::Restart,
		}
	}
}

/// Evidence-backed activity state for a compiled component.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Enum)]
pub enum RuntimeUsageStatus {
	#[graphql(name = "USED")]
	Used,
	#[graphql(name = "UNUSED")]
	Unused,
	#[graphql(name = "UNKNOWN")]
	Unknown,
}

impl From<UsageStatus> for RuntimeUsageStatus {
	fn from(value: UsageStatus) -> Self {
		match value {
			UsageStatus::Used => Self::Used,
			UsageStatus::Unused => Self::Unused,
			UsageStatus::Unknown => Self::Unknown,
		}
	}
}

/// The kind of an owned resource gauge. Total process RSS is not a component
/// gauge and is exposed separately through [`RuntimeMemory`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Enum)]
pub enum RuntimeGaugeKind {
	#[graphql(name = "BYTES")]
	Bytes,
	#[graphql(name = "COUNT")]
	Count,
}

#[derive(Clone, Debug, SimpleObject)]
pub struct RuntimeGauge {
	pub name: String,
	pub kind: RuntimeGaugeKind,
	pub value: u64,
}

impl From<CoreRuntimeGauge> for RuntimeGauge {
	fn from(value: CoreRuntimeGauge) -> Self {
		Self {
			name: value.name,
			kind: match value.kind {
				GaugeKind::Bytes => RuntimeGaugeKind::Bytes,
				GaugeKind::Count => RuntimeGaugeKind::Count,
			},
			value: value.value,
		}
	}
}

/// A code-owned runtime descriptor together with persisted lifecycle state.
#[derive(Clone, Debug, SimpleObject)]
pub struct RuntimeComponent {
	pub key: String,
	pub label: String,
	pub description: String,
	pub category: String,
	pub compiled: bool,
	pub desired_enabled: bool,
	pub effective_enabled: bool,
	pub transition_mode: RuntimeTransitionMode,
	pub transition_reason: String,
	pub dependencies: Vec<String>,
	pub health: String,
	pub restart_required: bool,
	pub usage_status: RuntimeUsageStatus,
	pub usage_evidence: Option<String>,
	pub activity_count: Option<u64>,
	pub last_activity_at: Option<DateTime<FixedOffset>>,
	pub last_transition_at: Option<DateTime<FixedOffset>>,
	pub last_error: Option<String>,
	pub owned_gauges: Vec<RuntimeGauge>,
}

impl From<CoreRuntimeComponent> for RuntimeComponent {
	fn from(value: CoreRuntimeComponent) -> Self {
		Self {
			key: value.key,
			label: value.label,
			description: value.description,
			category: value.category,
			compiled: value.compiled,
			desired_enabled: value.desired_enabled,
			effective_enabled: value.effective_enabled,
			transition_mode: value.transition_mode.into(),
			transition_reason: value.transition_reason,
			dependencies: value.dependencies,
			health: value.health,
			restart_required: value.restart_required,
			usage_status: value.usage_status.into(),
			usage_evidence: value.usage_evidence,
			activity_count: value.activity_count,
			last_activity_at: value.last_activity_at.map(|value| value.fixed_offset()),
			last_transition_at: value
				.last_transition_at
				.map(|value| value.fixed_offset()),
			last_error: value.last_error,
			owned_gauges: value.owned_gauges.into_iter().map(Into::into).collect(),
		}
	}
}

/// Process-wide RSS and Linux PSS breakdown are measured independently from
/// component-owned gauges. The breakdown never attributes pages to a component.
#[derive(Clone, Debug, SimpleObject)]
pub struct RuntimeMemory {
	pub total_process_rss_bytes: Option<u64>,
	pub rss_available: bool,
	pub rss_unavailable_reason: Option<String>,
	pub anonymous_pss_bytes: Option<u64>,
	pub file_backed_pss_bytes: Option<u64>,
	pub private_dirty_bytes: Option<u64>,
	pub memory_breakdown_available: bool,
	pub memory_breakdown_unavailable_reason: Option<String>,
}

impl From<ProcessMemory> for RuntimeMemory {
	fn from(value: ProcessMemory) -> Self {
		Self {
			total_process_rss_bytes: value.total_process_rss_bytes,
			rss_available: value.rss_available,
			rss_unavailable_reason: value.rss_unavailable_reason,
			anonymous_pss_bytes: value.anonymous_pss_bytes,
			file_backed_pss_bytes: value.file_backed_pss_bytes,
			private_dirty_bytes: value.private_dirty_bytes,
			memory_breakdown_available: value.memory_breakdown_available,
			memory_breakdown_unavailable_reason: value
				.memory_breakdown_unavailable_reason,
		}
	}
}

/// Availability of a client/protocol registration kind in this server build.
#[derive(Clone, Debug, SimpleObject)]
pub struct DeviceCapability {
	pub kind: DeviceKind,
	pub protocol: DeviceProtocol,
	pub component_key: String,
	pub compiled: bool,
	/// Effective runtime state, not the persisted desired state.
	pub enabled: bool,
	pub available: bool,
	pub reason: Option<String>,
}

impl DeviceCapability {
	pub fn new(
		kind: DeviceKind,
		protocol: DeviceProtocol,
		component_key: impl Into<String>,
		compiled: bool,
		enabled: bool,
	) -> Self {
		let component_key = component_key.into();
		let reason = if !compiled {
			Some("not compiled into this server".to_owned())
		} else if !enabled {
			Some("disabled by server runtime state".to_owned())
		} else {
			None
		};
		Self {
			kind,
			protocol,
			component_key,
			compiled,
			enabled,
			available: compiled && enabled,
			reason,
		}
	}
}
