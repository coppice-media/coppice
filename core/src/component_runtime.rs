//! Persistent runtime component state and process-level resource reporting.
//!
//! Component descriptors are deliberately split from the persisted row. Code
//! owns the compile flag, dependency graph, transition policy, and labels;
//! SQLite owns the operator's desired state and the last effective transition.
//! This keeps a restart truthful when a component cannot be stopped and
//! started safely in the current process.

use std::{
	collections::{BTreeMap, HashMap, HashSet},
	sync::{
		atomic::{AtomicBool, Ordering},
		Arc, RwLock,
	},
};

use chrono::{DateTime, Duration, Utc};
use models::entity::runtime_component;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait};

use crate::{config::StumpConfig, CoreError, CoreResult};

pub const COMPONENT_BACKGROUND_JOBS: &str = "background-jobs";
pub const COMPONENT_WATCHER: &str = "watcher";
pub const COMPONENT_SCHEDULER: &str = "scheduler";
pub const COMPONENT_PROVIDERS: &str = "providers";
pub const COMPONENT_CROSSPOINT_DELIVERY: &str = "crosspoint-delivery";
pub const COMPONENT_INGEST: &str = "ingest";
pub const COMPONENT_WORKER: &str = "worker";
pub const COMPONENT_DEVICES: &str = "devices";
pub const COMPONENT_ANNOTATION_SYNC: &str = "annotation-sync";
pub const COMPONENT_NOTIFICATIONS: &str = "notifications";
pub const COMPONENT_TRANSFORM: &str = "transform";
pub const COMPONENT_PDF: &str = "pdf";
pub const COMPONENT_RAR: &str = "rar";
pub const COMPONENT_KOBO: &str = "kobo";
pub const COMPONENT_KOREADER: &str = "koreader";
pub const COMPONENT_CROSSPOINT: &str = "crosspoint";
pub const COMPONENT_KOMGA: &str = "komga";
pub const COMPONENT_KAVITA: &str = "kavita";
pub const COMPONENT_ABS: &str = "abs";
pub const COMPONENT_LISEUR_SYNC: &str = "liseur-sync";
pub const COMPONENT_OPDS: &str = "opds";
pub const COMPONENT_API: &str = "api";
pub const COMPONENT_WEBUI: &str = "webui";

/// A component is only called unused after a full observation window with a
/// code-owned activity source and no recorded activity. Components without
/// such an instrumented source remain `UNKNOWN`.
const USAGE_IDLE_WINDOW: Duration = Duration::hours(24);

/// Whether a component can change without rebuilding route/service ownership.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionMode {
	Hot,
	Restart,
}

impl TransitionMode {
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Hot => "HOT",
			Self::Restart => "RESTART",
		}
	}

	pub fn parse(value: &str) -> Self {
		if value.eq_ignore_ascii_case("HOT") {
			Self::Hot
		} else {
			Self::Restart
		}
	}
}

/// Whether activity evidence supports calling an enabled component used,
/// unused, or unknown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsageStatus {
	Used,
	Unused,
	Unknown,
}

impl UsageStatus {
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Used => "USED",
			Self::Unused => "UNUSED",
			Self::Unknown => "UNKNOWN",
		}
	}
}

/// A code-owned component descriptor registered by core or the server host.
#[derive(Clone, Debug)]
pub struct ComponentDefinition {
	pub key: String,
	pub label: String,
	pub description: String,
	pub category: String,
	pub compiled: bool,
	pub default_enabled: bool,
	pub transition_mode: TransitionMode,
	pub transition_reason: String,
	pub dependencies: Vec<String>,
	pub activity_source: Option<String>,
}

impl ComponentDefinition {
	pub fn new(
		key: impl Into<String>,
		label: impl Into<String>,
		category: impl Into<String>,
		compiled: bool,
		default_enabled: bool,
		transition_mode: TransitionMode,
		dependencies: impl IntoIterator<Item = impl Into<String>>,
	) -> Self {
		let key = key.into();
		let label = label.into();
		Self {
			description: component_description(&key).to_owned(),
			transition_reason: component_transition_reason(&key, transition_mode)
				.to_owned(),
			key,
			label,
			category: category.into(),
			compiled,
			default_enabled,
			transition_mode,
			dependencies: dependencies.into_iter().map(Into::into).collect(),
			activity_source: None,
		}
	}

	/// Marks the concrete source used to prove activity for this descriptor.
	/// Components without a source deliberately remain usage-unknown.
	pub fn with_activity_source(mut self, source: impl Into<String>) -> Self {
		self.activity_source = Some(source.into());
		self
	}
}

fn component_description(key: &str) -> &'static str {
	match key {
		COMPONENT_BACKGROUND_JOBS => "Runs durable background work on the server.",
		COMPONENT_WATCHER => "Watches configured libraries and queues scans.",
		COMPONENT_SCHEDULER => "Schedules durable jobs from persisted cron rows.",
		COMPONENT_PROVIDERS => {
			"Fetches metadata and materializes remote provider sources."
		},
		COMPONENT_CROSSPOINT_DELIVERY => {
			"Delivers queued reading-state updates to CrossPoint clients."
		},
		COMPONENT_INGEST => "Stages imported files for validation and library placement.",
		COMPONENT_WORKER => "Queues work for connected remote workers.",
		COMPONENT_DEVICES => "Stores registered clients, credentials, and device state.",
		COMPONENT_ANNOTATION_SYNC => "Synchronizes annotations to configured sinks.",
		COMPONENT_NOTIFICATIONS => {
			"Dispatches configured notification channels and rules."
		},
		COMPONENT_TRANSFORM => {
			"Converts media into requested reading and delivery formats."
		},
		COMPONENT_PDF => "Adds PDF parsing and page extraction support.",
		COMPONENT_RAR => "Adds RAR archive inspection and extraction support.",
		COMPONENT_KOBO => "Provides Kobo device synchronization endpoints.",
		COMPONENT_KOREADER => "Provides KOReader progress and catalog synchronization.",
		COMPONENT_CROSSPOINT => "Provides CrossPoint delivery and read-state endpoints.",
		COMPONENT_KOMGA => "Provides Komga-compatible catalog and reading endpoints.",
		COMPONENT_KAVITA => "Provides Kavita-compatible catalog and reading endpoints.",
		COMPONENT_ABS => "Provides Audiobookshelf-compatible audio endpoints.",
		COMPONENT_LISEUR_SYNC => "Provides Liseur work and edition synchronization.",
		COMPONENT_OPDS => "Publishes the OPDS catalog and acquisition feed.",
		COMPONENT_API => "Serves the native authenticated GraphQL and API surface.",
		COMPONENT_WEBUI => "Serves the browser applications and static web assets.",
		_ => "Runtime component.",
	}
}

fn component_transition_reason(key: &str, mode: TransitionMode) -> &'static str {
	match key {
		COMPONENT_BACKGROUND_JOBS => {
			"Restart rebuilds the executor and its shutdown ownership from startup configuration."
		},
		COMPONENT_WATCHER => {
			"Restart rebuilds filesystem watcher registrations owned by the background lifecycle."
		},
		COMPONENT_SCHEDULER => {
			"Restart rebuilds scheduler tasks from persisted configuration."
		},
		COMPONENT_PROVIDERS => {
			"Restart rebuilds the provider host, source registry, and provider-owned workers."
		},
		COMPONENT_CROSSPOINT_DELIVERY => {
			"Restart rebuilds the delivery coordinator and its durable queue workers."
		},
		COMPONENT_INGEST => {
			"Restart rebuilds ingest services from the configured provider and library wiring."
		},
		COMPONENT_WORKER => {
			"Restart rebuilds worker registrations and the remote-job claim lifecycle."
		},
		COMPONENT_ANNOTATION_SYNC => {
			"Restart rebuilds annotation sink workers and their lifecycle state."
		},
		COMPONENT_NOTIFICATIONS => {
			"Restart rebuilds notification channels and dispatch ownership."
		},
		COMPONENT_TRANSFORM => {
			"Restart applies transform configuration and rebuilds shared cache ownership."
		},
		COMPONENT_PDF => "PDF support is linked at compile time; restart loads the selected build.",
		COMPONENT_RAR => "RAR support is linked at compile time; restart loads the selected build.",
		COMPONENT_WEBUI => "Restart remounts the static web UI owner at the application root.",
		COMPONENT_DEVICES => "The device registry is lazy and its effective flag is read at request time.",
		COMPONENT_KOBO => "Mounted Kobo routes are guarded per request, so changes apply immediately.",
		COMPONENT_KOREADER => {
			"Mounted KOReader routes are guarded per request, so changes apply immediately."
		},
		COMPONENT_CROSSPOINT => {
			"Mounted CrossPoint routes are guarded per request, so changes apply immediately."
		},
		COMPONENT_KOMGA => "Mounted Komga routes are guarded per request, so changes apply immediately.",
		COMPONENT_KAVITA => {
			"Mounted Kavita routes are guarded per request, so changes apply immediately."
		},
		COMPONENT_ABS => {
			"Mounted Audiobookshelf routes are guarded per request, so changes apply immediately."
		},
		COMPONENT_LISEUR_SYNC => {
			"Mounted Liseur routes are guarded per request, so changes apply immediately."
		},
		COMPONENT_OPDS => "Mounted OPDS routes are guarded per request, so changes apply immediately.",
		COMPONENT_API => "Mounted native API routes are guarded per request, so changes apply immediately.",
		_ if mode == TransitionMode::Hot => "A live route or service gate applies changes immediately.",
		_ => "This service is owned during startup and must be rebuilt by restarting the server.",
	}
}

/// A byte/count gauge owned by one component. Process RSS is intentionally not
/// represented here: allocator, code, shared-library, and transient request
/// pages cannot be assigned to a component without explicit instrumentation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeGauge {
	pub name: String,
	pub kind: GaugeKind,
	pub value: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GaugeKind {
	Bytes,
	Count,
}

impl GaugeKind {
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Bytes => "BYTES",
			Self::Count => "COUNT",
		}
	}
}

/// One projected component, including its persisted state and live health.
#[derive(Clone, Debug)]
pub struct RuntimeComponent {
	pub key: String,
	pub label: String,
	pub description: String,
	pub category: String,
	pub compiled: bool,
	pub desired_enabled: bool,
	pub effective_enabled: bool,
	pub transition_mode: TransitionMode,
	pub transition_reason: String,
	pub dependencies: Vec<String>,
	pub health: String,
	pub restart_required: bool,
	pub usage_status: UsageStatus,
	pub usage_evidence: Option<String>,
	pub activity_count: Option<u64>,
	pub last_activity_at: Option<DateTime<Utc>>,
	pub last_transition_at: Option<DateTime<Utc>>,
	pub last_error: Option<String>,
	pub owned_gauges: Vec<RuntimeGauge>,
}

/// Exact process RSS when Linux exposes it, or an explicit unavailable state.
///
/// The PSS breakdown is process-wide evidence only. It is never attributed to
/// a component gauge: file-backed pages include executable/shared mappings and
/// anonymous pages include allocator/runtime state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessMemory {
	pub total_process_rss_bytes: Option<u64>,
	pub rss_available: bool,
	pub rss_unavailable_reason: Option<String>,
	pub anonymous_pss_bytes: Option<u64>,
	pub file_backed_pss_bytes: Option<u64>,
	pub private_dirty_bytes: Option<u64>,
	pub memory_breakdown_available: bool,
	pub memory_breakdown_unavailable_reason: Option<String>,
}

impl ProcessMemory {
	pub fn read() -> Self {
		#[cfg(target_os = "linux")]
		{
			let rss = read_linux_rss();
			let breakdown = read_linux_memory_breakdown();
			return match (rss, breakdown) {
				(Ok(bytes), Ok(breakdown)) => Self {
					total_process_rss_bytes: Some(bytes),
					rss_available: true,
					rss_unavailable_reason: None,
					anonymous_pss_bytes: breakdown.anonymous_pss_bytes,
					file_backed_pss_bytes: breakdown.file_backed_pss_bytes,
					private_dirty_bytes: breakdown.private_dirty_bytes,
					memory_breakdown_available: true,
					memory_breakdown_unavailable_reason: None,
				},
				(Ok(bytes), Err(reason)) => Self {
					total_process_rss_bytes: Some(bytes),
					rss_available: true,
					rss_unavailable_reason: None,
					anonymous_pss_bytes: None,
					file_backed_pss_bytes: None,
					private_dirty_bytes: None,
					memory_breakdown_available: false,
					memory_breakdown_unavailable_reason: Some(reason),
				},
				(Err(reason), Ok(breakdown)) => Self {
					total_process_rss_bytes: None,
					rss_available: false,
					rss_unavailable_reason: Some(reason),
					anonymous_pss_bytes: breakdown.anonymous_pss_bytes,
					file_backed_pss_bytes: breakdown.file_backed_pss_bytes,
					private_dirty_bytes: breakdown.private_dirty_bytes,
					memory_breakdown_available: true,
					memory_breakdown_unavailable_reason: None,
				},
				(Err(rss_reason), Err(breakdown_reason)) => Self {
					total_process_rss_bytes: None,
					rss_available: false,
					rss_unavailable_reason: Some(rss_reason),
					anonymous_pss_bytes: None,
					file_backed_pss_bytes: None,
					private_dirty_bytes: None,
					memory_breakdown_available: false,
					memory_breakdown_unavailable_reason: Some(breakdown_reason),
				},
			};
		}

		#[cfg(not(target_os = "linux"))]
		Self {
			total_process_rss_bytes: None,
			rss_available: false,
			rss_unavailable_reason: Some(
				"process RSS is available only on Linux via /proc/self/statm".to_owned(),
			),
			anonymous_pss_bytes: None,
			file_backed_pss_bytes: None,
			private_dirty_bytes: None,
			memory_breakdown_available: false,
			memory_breakdown_unavailable_reason: Some(
				"process memory breakdown is available only on Linux via /proc/self/smaps_rollup"
					.to_owned(),
			),
		}
	}
}

#[cfg(target_os = "linux")]
fn read_linux_rss() -> Result<u64, String> {
	let statm = std::fs::read_to_string("/proc/self/statm")
		.map_err(|error| format!("could not read /proc/self/statm: {error}"))?;
	let resident_pages = statm
		.split_whitespace()
		.nth(1)
		.ok_or_else(|| "missing resident page count in /proc/self/statm".to_owned())?
		.parse::<u64>()
		.map_err(|error| {
			format!("invalid resident page count in /proc/self/statm: {error}")
		})?;
	// Linux reports statm in pages; sysconf is the kernel's actual page size,
	// rather than a guessed 4096-byte constant.
	let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
	if page_size <= 0 {
		return Err("could not determine Linux page size".to_owned());
	}
	resident_pages
		.checked_mul(page_size as u64)
		.ok_or_else(|| "process RSS exceeds u64".to_owned())
}
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug)]
struct LinuxMemoryBreakdown {
	anonymous_pss_bytes: Option<u64>,
	file_backed_pss_bytes: Option<u64>,
	private_dirty_bytes: Option<u64>,
}

#[cfg(target_os = "linux")]
fn read_linux_memory_breakdown() -> Result<LinuxMemoryBreakdown, String> {
	let rollup = std::fs::read_to_string("/proc/self/smaps_rollup")
		.map_err(|error| format!("could not read /proc/self/smaps_rollup: {error}"))?;
	parse_linux_memory_breakdown(&rollup)
}

#[cfg(target_os = "linux")]
fn parse_linux_memory_breakdown(rollup: &str) -> Result<LinuxMemoryBreakdown, String> {
	let mut anonymous_pss_bytes = None;
	let mut file_backed_pss_bytes = None;
	let mut private_dirty_bytes = None;
	for line in rollup.lines() {
		let mut fields = line.split_whitespace();
		let Some(label) = fields.next() else {
			continue;
		};
		let Some(value) = fields.next() else {
			continue;
		};
		let Ok(kib) = value.parse::<u64>() else {
			continue;
		};
		let Some(bytes) = kib.checked_mul(1024) else {
			continue;
		};
		match label {
			"Pss_Anon:" => anonymous_pss_bytes = Some(bytes),
			"Pss_File:" => file_backed_pss_bytes = Some(bytes),
			"Private_Dirty:" => private_dirty_bytes = Some(bytes),
			_ => {},
		}
	}
	if anonymous_pss_bytes.is_none()
		&& file_backed_pss_bytes.is_none()
		&& private_dirty_bytes.is_none()
	{
		return Err(
			"smaps_rollup did not expose a PSS or private-dirty breakdown".to_owned(),
		);
	}
	Ok(LinuxMemoryBreakdown {
		anonymous_pss_bytes,
		file_backed_pss_bytes,
		private_dirty_bytes,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	fn definition(activity_source: Option<&str>) -> ComponentDefinition {
		let definition = ComponentDefinition::new(
			"test",
			"Test component",
			"test",
			true,
			true,
			TransitionMode::Hot,
			Vec::<&str>::new(),
		);
		match activity_source {
			Some(source) => definition.with_activity_source(source),
			None => definition,
		}
	}

	fn state(
		effective_enabled: bool,
		activity_count: u64,
		last_activity_at: Option<DateTime<Utc>>,
		observed_since: DateTime<Utc>,
	) -> RuntimeState {
		RuntimeState {
			desired_enabled: effective_enabled,
			effective_enabled,
			last_transition_at: None,
			last_error: None,
			activity_count,
			last_activity_at,
			observed_since,
		}
	}

	#[test]
	fn usage_status_requires_grounded_activity_evidence() {
		let now = Utc::now();
		let recent = now - Duration::minutes(5);
		let old = now - Duration::hours(25);

		assert_eq!(
			usage_projection(&definition(None), &state(true, 0, None, old)).0,
			UsageStatus::Unknown
		);
		assert_eq!(
			usage_projection(
				&definition(Some("request")),
				&state(false, 4, Some(recent), recent),
			)
			.0,
			UsageStatus::Unknown
		);
		assert_eq!(
			usage_projection(
				&definition(Some("request")),
				&state(true, 4, Some(recent), recent),
			)
			.0,
			UsageStatus::Used
		);
		assert_eq!(
			usage_projection(&definition(Some("request")), &state(true, 0, None, old)).0,
			UsageStatus::Unused
		);
		assert_eq!(
			usage_projection(&definition(Some("request")), &state(true, 0, None, recent))
				.0,
			UsageStatus::Unknown
		);
	}

	#[cfg(target_os = "linux")]
	#[test]
	fn linux_memory_breakdown_uses_pss_and_private_dirty_kib() {
		let parsed = parse_linux_memory_breakdown(
			"Pss_Anon: 12 kB\nPss_File: 34 kB\nPrivate_Dirty: 56 kB\n",
		)
		.expect("valid smaps rollup");

		assert_eq!(parsed.anonymous_pss_bytes, Some(12 * 1024));
		assert_eq!(parsed.file_backed_pss_bytes, Some(34 * 1024));
		assert_eq!(parsed.private_dirty_bytes, Some(56 * 1024));
		assert!(parse_linux_memory_breakdown("Rss: 99 kB\n").is_err());
	}
}

/// Runtime registry. The lock is intentionally synchronous because route
/// guards call [`is_effective`] on every request and must not await a database
/// round trip. Mutations persist after the in-memory state is serialized.
#[derive(Clone)]
pub struct ComponentRuntime {
	conn: Arc<DatabaseConnection>,
	definitions: Arc<RwLock<BTreeMap<String, ComponentDefinition>>>,
	states: Arc<RwLock<BTreeMap<String, RuntimeState>>>,
	loaded: Arc<RwLock<HashSet<String>>>,
	initialized: Arc<AtomicBool>,
}

#[derive(Clone, Debug)]
struct RuntimeState {
	desired_enabled: bool,
	effective_enabled: bool,
	last_transition_at: Option<DateTime<Utc>>,
	last_error: Option<String>,
	activity_count: u64,
	last_activity_at: Option<DateTime<Utc>>,
	observed_since: DateTime<Utc>,
}

impl ComponentRuntime {
	pub fn new(conn: Arc<DatabaseConnection>, config: &StumpConfig) -> Self {
		let definitions = builtin_definitions(config)
			.into_iter()
			.map(|definition| {
				let state = RuntimeState {
					desired_enabled: definition.default_enabled,
					effective_enabled: definition.compiled && definition.default_enabled,
					last_transition_at: None,
					last_error: (!definition.compiled)
						.then(|| "component is not compiled into this server".to_owned()),
					activity_count: 0,
					last_activity_at: None,
					observed_since: Utc::now(),
				};
				(definition.key.clone(), (definition, state))
			})
			.collect::<BTreeMap<_, _>>();
		let (definitions, states) = definitions.into_iter().fold(
			(BTreeMap::new(), BTreeMap::new()),
			|mut acc, (key, (definition, state))| {
				acc.0.insert(key.clone(), definition);
				acc.1.insert(key, state);
				acc
			},
		);
		Self {
			conn,
			definitions: Arc::new(RwLock::new(definitions)),
			states: Arc::new(RwLock::new(states)),
			loaded: Arc::new(RwLock::new(HashSet::new())),
			initialized: Arc::new(AtomicBool::new(false)),
		}
	}

	/// Adds server-owned descriptors (protocol routes and their compile flags).
	/// Existing persisted desired values win; newly seen keys use the supplied
	/// default. The call is idempotent and safe during startup.
	pub async fn register(
		&self,
		definitions: impl IntoIterator<Item = ComponentDefinition>,
	) -> CoreResult<()> {
		{
			let mut all = self
				.definitions
				.write()
				.unwrap_or_else(|poisoned| poisoned.into_inner());
			let mut states = self
				.states
				.write()
				.unwrap_or_else(|poisoned| poisoned.into_inner());
			for definition in definitions {
				let key = definition.key.clone();
				all.insert(key.clone(), definition.clone());
				states.entry(key).or_insert_with(|| RuntimeState {
					desired_enabled: definition.default_enabled,
					effective_enabled: definition.compiled && definition.default_enabled,
					last_transition_at: None,
					last_error: (!definition.compiled)
						.then(|| "component is not compiled into this server".to_owned()),
					activity_count: 0,
					last_activity_at: None,
					observed_since: Utc::now(),
				});
			}
		}
		self.initialize().await
	}

	/// Loads persisted desired state and synchronizes effective state at process
	/// startup. A restart is the ownership boundary for RESTART components, so a
	/// previously requested desired value becomes effective here when compiled
	/// and dependencies are available.
	pub async fn initialize(&self) -> CoreResult<()> {
		let rows = runtime_component::Entity::find()
			.all(self.conn.as_ref())
			.await?;
		let by_key = rows
			.into_iter()
			.map(|row| (row.key.clone(), row))
			.collect::<HashMap<_, _>>();
		let definitions = self
			.definitions
			.read()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
			.clone();
		let snapshots = {
			let mut states = self
				.states
				.write()
				.unwrap_or_else(|poisoned| poisoned.into_inner());
			let mut loaded = self
				.loaded
				.write()
				.unwrap_or_else(|poisoned| poisoned.into_inner());
			let mut pending = definitions
				.iter()
				.filter(|(key, _)| !loaded.contains(*key))
				.map(|(key, definition)| {
					(key.clone(), definition.clone(), by_key.get(key).cloned())
				})
				.collect::<Vec<_>>();
			let mut snapshots = Vec::with_capacity(pending.len());

			while !pending.is_empty() {
				let mut deferred = Vec::new();
				let mut progressed = false;
				for (key, definition, row) in pending {
					let dependencies_known =
						definition.dependencies.iter().all(|dependency| {
							loaded.contains(dependency)
								|| !definitions.contains_key(dependency)
						});
					if !dependencies_known {
						deferred.push((key, definition, row));
						continue;
					}

					let desired = row
						.as_ref()
						.map(|row| row.desired_enabled)
						.unwrap_or(definition.default_enabled);
					let dependencies_ok = dependencies_compiled_and_effective(
						&definition,
						&definitions,
						&states,
					);
					let effective = desired && definition.compiled && dependencies_ok;
					let last_error = if !definition.compiled {
						Some("component is not compiled into this server".to_owned())
					} else if desired && !dependencies_ok {
						Some(
							"one or more component dependencies are unavailable"
								.to_owned(),
						)
					} else {
						None
					};
					let (activity_count, last_activity_at, observed_since) = states
						.get(&key)
						.map(|state| {
							(
								state.activity_count,
								state.last_activity_at,
								state.observed_since,
							)
						})
						.unwrap_or((0, None, Utc::now()));
					let last_transition_at = row
						.as_ref()
						.and_then(|row| row.last_transition_at.map(|value| value.into()));
					states.insert(
						key.clone(),
						RuntimeState {
							desired_enabled: desired,
							effective_enabled: effective,
							last_transition_at,
							last_error: last_error.clone(),
							activity_count,
							last_activity_at,
							observed_since,
						},
					);
					loaded.insert(key.clone());
					snapshots.push((
						definition,
						desired,
						effective,
						last_transition_at,
						last_error,
					));
					progressed = true;
				}

				if progressed {
					pending = deferred;
				} else {
					for (key, definition, row) in deferred {
						let desired = row
							.as_ref()
							.map(|row| row.desired_enabled)
							.unwrap_or(definition.default_enabled);
						let (activity_count, last_activity_at, observed_since) = states
							.get(&key)
							.map(|state| {
								(
									state.activity_count,
									state.last_activity_at,
									state.observed_since,
								)
							})
							.unwrap_or((0, None, Utc::now()));
						let last_transition_at = row.as_ref().and_then(|row| {
							row.last_transition_at.map(|value| value.into())
						});
						let last_error = if !definition.compiled {
							Some("component is not compiled into this server".to_owned())
						} else {
							Some(
								"one or more component dependencies are unavailable"
									.to_owned(),
							)
						};
						states.insert(
							key.clone(),
							RuntimeState {
								desired_enabled: desired,
								effective_enabled: false,
								last_transition_at,
								last_error: last_error.clone(),
								activity_count,
								last_activity_at,
								observed_since,
							},
						);
						loaded.insert(key);
						snapshots.push((
							definition,
							desired,
							false,
							last_transition_at,
							last_error,
						));
					}
					break;
				}
			}
			snapshots
		};
		for (definition, desired, effective, last_transition_at, last_error) in snapshots
		{
			self.persist_definition(
				&definition,
				desired,
				effective,
				last_transition_at,
				last_error,
			)
			.await?;
		}
		self.initialized.store(true, Ordering::Release);
		Ok(())
	}

	pub fn initialized(&self) -> bool {
		self.initialized.load(Ordering::Acquire)
	}

	pub fn is_effective(&self, key: &str) -> bool {
		self.states
			.read()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
			.get(key)
			.map(|state| state.effective_enabled)
			.unwrap_or(false)
	}
	/// Records one event from a descriptor's explicit activity source.
	/// Activity is intentionally in-memory: it describes this process's
	/// observation window and is not a component-memory attribution.
	pub fn record_activity(&self, key: &str) {
		let definitions = self
			.definitions
			.read()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		if !definitions
			.get(key)
			.is_some_and(|definition| definition.activity_source.is_some())
		{
			return;
		}
		drop(definitions);
		let mut states = self
			.states
			.write()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		if let Some(state) = states.get_mut(key) {
			if !state.effective_enabled {
				return;
			}
			state.activity_count = state.activity_count.saturating_add(1);
			state.last_activity_at = Some(Utc::now());
		}
	}

	pub fn definition(&self, key: &str) -> Option<ComponentDefinition> {
		self.definitions
			.read()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
			.get(key)
			.cloned()
	}

	pub async fn components(&self) -> CoreResult<Vec<RuntimeComponent>> {
		if !self.initialized() {
			self.initialize().await?;
		}
		let definitions = self
			.definitions
			.read()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
			.clone();
		let states = self
			.states
			.read()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
			.clone();
		Ok(definitions
			.into_iter()
			.map(|(key, definition)| project(&key, &definition, states.get(&key)))
			.collect())
	}

	pub async fn set_enabled(
		&self,
		key: &str,
		enabled: bool,
	) -> CoreResult<RuntimeComponent> {
		if !self.initialized() {
			self.initialize().await?;
		}
		let before = ProcessMemory::read();
		let (
			definition,
			desired,
			effective,
			last_transition_at,
			last_error,
			activity_count,
			last_activity_at,
			observed_since,
		) = {
			let definitions = self
				.definitions
				.read()
				.unwrap_or_else(|poisoned| poisoned.into_inner());
			let definition = definitions.get(key).cloned().ok_or_else(|| {
				CoreError::NotFound(format!("unknown runtime component `{key}`"))
			})?;
			let mut states = self
				.states
				.write()
				.unwrap_or_else(|poisoned| poisoned.into_inner());
			if !states.contains_key(key) {
				return Err(CoreError::NotFound(format!(
					"unknown runtime component `{key}`"
				)));
			}
			if enabled && !definition.compiled {
				return Err(CoreError::BadRequest(format!(
					"component `{key}` is not compiled into this server"
				)));
			}
			if enabled
				&& !dependencies_compiled_and_effective(
					&definition,
					&definitions,
					&states,
				) {
				return Err(CoreError::BadRequest(format!(
					"component `{key}` has unavailable dependencies"
				)));
			}
			if !enabled {
				if let Some(dependent) = definitions.values().find(|candidate| {
					candidate
						.dependencies
						.iter()
						.any(|dependency| dependency == key)
						&& states.get(&candidate.key).is_some_and(|dependent_state| {
							dependent_state.effective_enabled
						})
				}) {
					return Err(CoreError::BadRequest(format!(
						"component `{key}` is required by enabled component `{}`",
						dependent.key
					)));
				}
			}
			let state = states
				.get_mut(key)
				.expect("runtime component state checked above");
			let old_effective = state.effective_enabled;
			state.desired_enabled = enabled;
			if definition.transition_mode == TransitionMode::Hot {
				state.effective_enabled = enabled;
			}
			let changed = old_effective != state.effective_enabled;
			if changed {
				state.last_transition_at = Some(Utc::now());
			}
			state.last_error = None;
			if !definition.compiled {
				state.last_error =
					Some("component is not compiled into this server".to_owned());
			}
			(
				definition,
				state.desired_enabled,
				state.effective_enabled,
				state.last_transition_at.clone(),
				state.last_error.clone(),
				state.activity_count,
				state.last_activity_at,
				state.observed_since,
			)
		};
		self.persist_definition(
			&definition,
			desired,
			effective,
			last_transition_at,
			last_error.clone(),
		)
		.await?;
		let after = ProcessMemory::read();
		let delta = match (
			before.total_process_rss_bytes,
			after.total_process_rss_bytes,
		) {
			(Some(before), Some(after)) => Some(after as i128 - before as i128),
			_ => None,
		};
		tracing::info!(
			component = key,
			enabled,
			before_total_rss_bytes = ?before.total_process_rss_bytes,
			after_total_rss_bytes = ?after.total_process_rss_bytes,
			unattributed_rss_delta_bytes = ?delta,
			"runtime component transition"
		);
		Ok(project(
			key,
			&definition,
			Some(&RuntimeState {
				desired_enabled: desired,
				effective_enabled: effective,
				last_transition_at,
				last_error,
				activity_count,
				last_activity_at,
				observed_since,
			}),
		))
	}

	async fn persist_definition(
		&self,
		definition: &ComponentDefinition,
		desired_enabled: bool,
		effective_enabled: bool,
		last_transition_at: Option<DateTime<Utc>>,
		last_error: Option<String>,
	) -> CoreResult<()> {
		let now = Utc::now().fixed_offset();
		let existing = runtime_component::Entity::find_by_id(definition.key.clone())
			.one(self.conn.as_ref())
			.await?;
		if let Some(existing) = existing {
			let mut active: runtime_component::ActiveModel = existing.into();
			active.label = Set(definition.label.clone());
			active.category = Set(definition.category.clone());
			active.compiled = Set(definition.compiled);
			active.desired_enabled = Set(desired_enabled);
			active.effective_enabled = Set(effective_enabled);
			active.transition_mode = Set(definition.transition_mode.as_str().to_owned());
			active.last_transition_at = Set(last_transition_at.map(Into::into));
			active.last_error = Set(last_error);
			active.updated_at = Set(now);
			active.update(self.conn.as_ref()).await?;
		} else {
			runtime_component::ActiveModel {
				key: Set(definition.key.clone()),
				label: Set(definition.label.clone()),
				category: Set(definition.category.clone()),
				compiled: Set(definition.compiled),
				desired_enabled: Set(desired_enabled),
				effective_enabled: Set(effective_enabled),
				transition_mode: Set(definition.transition_mode.as_str().to_owned()),
				last_transition_at: Set(last_transition_at.map(Into::into)),
				last_error: Set(last_error),
				created_at: Set(now),
				updated_at: Set(now),
			}
			.insert(self.conn.as_ref())
			.await?;
		}
		Ok(())
	}
}

fn project(
	key: &str,
	definition: &ComponentDefinition,
	state: Option<&RuntimeState>,
) -> RuntimeComponent {
	let state = state.cloned().unwrap_or(RuntimeState {
		desired_enabled: definition.default_enabled,
		effective_enabled: definition.compiled && definition.default_enabled,
		last_transition_at: None,
		last_error: None,
		activity_count: 0,
		last_activity_at: None,
		observed_since: Utc::now(),
	});
	let health = if !definition.compiled {
		"unavailable"
	} else if !state.effective_enabled {
		"disabled"
	} else {
		"active"
	};
	let (usage_status, usage_evidence) = usage_projection(definition, &state);
	RuntimeComponent {
		key: key.to_owned(),
		label: definition.label.clone(),
		description: definition.description.clone(),
		category: definition.category.clone(),
		compiled: definition.compiled,
		desired_enabled: state.desired_enabled,
		effective_enabled: state.effective_enabled,
		transition_mode: definition.transition_mode,
		transition_reason: definition.transition_reason.clone(),
		dependencies: definition.dependencies.clone(),
		health: health.to_owned(),
		restart_required: definition.transition_mode == TransitionMode::Restart
			&& state.desired_enabled != state.effective_enabled,
		usage_status,
		usage_evidence,
		activity_count: definition
			.activity_source
			.as_ref()
			.map(|_| state.activity_count),
		last_activity_at: definition
			.activity_source
			.as_ref()
			.and_then(|_| state.last_activity_at),
		last_transition_at: state.last_transition_at,
		last_error: state.last_error,
		owned_gauges: Vec::new(),
	}
}

fn usage_projection(
	definition: &ComponentDefinition,
	state: &RuntimeState,
) -> (UsageStatus, Option<String>) {
	let Some(source) = definition.activity_source.as_deref() else {
		return (
			UsageStatus::Unknown,
			Some(
				"Usage unknown: this component has no activity instrumented yet."
					.to_owned(),
			),
		);
	};
	if !state.effective_enabled {
		return (
			UsageStatus::Unknown,
			Some("Usage unknown: component is not currently effective.".to_owned()),
		);
	}
	let now = Utc::now();
	let last_activity = state.last_activity_at.unwrap_or(state.observed_since);
	let idle_for = now.signed_duration_since(last_activity);
	if idle_for >= USAGE_IDLE_WINDOW {
		let last = state
			.last_activity_at
			.map(|value| value.to_rfc3339())
			.unwrap_or_else(|| "never".to_owned());
		return (
			UsageStatus::Unused,
			Some(format!(
				"No {source} activity observed for the current {}-hour idle window; last activity {last}.",
				USAGE_IDLE_WINDOW.num_hours()
			)),
		);
	}
	if state.activity_count > 0 {
		let last = state
			.last_activity_at
			.map(|value| value.to_rfc3339())
			.unwrap_or_else(|| "time unavailable".to_owned());
		return (
			UsageStatus::Used,
			Some(format!(
				"Observed {source} activity {} time(s); last activity {last}.",
				state.activity_count
			)),
		);
	}
	(
		UsageStatus::Unknown,
		Some(format!(
			"Usage unknown: no {source} activity observed during the current {}-hour window.",
			USAGE_IDLE_WINDOW.num_hours()
		)),
	)
}

fn dependencies_compiled_and_effective(
	definition: &ComponentDefinition,
	definitions: &BTreeMap<String, ComponentDefinition>,
	states: &BTreeMap<String, RuntimeState>,
) -> bool {
	definition.dependencies.iter().all(|dependency| {
		definitions
			.get(dependency)
			.is_some_and(|dependency_definition| dependency_definition.compiled)
			&& states
				.get(dependency)
				.is_some_and(|dependency_state| dependency_state.effective_enabled)
	})
}

/// Core-owned components are known even in a headless/minimal server. The
/// server registers protocol/provider descriptors after it knows its Cargo
/// feature set.
pub fn builtin_definitions(config: &StumpConfig) -> Vec<ComponentDefinition> {
	vec![
		ComponentDefinition::new(
			COMPONENT_BACKGROUND_JOBS,
			"Background jobs",
			"lifecycle",
			true,
			config.jobs.enable_background_jobs,
			TransitionMode::Restart,
			Vec::<&str>::new(),
		)
		.with_activity_source("background job enqueue and execution"),
		ComponentDefinition::new(
			COMPONENT_WATCHER,
			"Library watcher",
			"lifecycle",
			cfg!(feature = "watcher"),
			config.jobs.enable_background_jobs,
			TransitionMode::Restart,
			[COMPONENT_BACKGROUND_JOBS],
		)
		.with_activity_source("library watcher initialization and events"),
		ComponentDefinition::new(
			COMPONENT_SCHEDULER,
			"Scheduled jobs",
			"lifecycle",
			true,
			config.jobs.enable_background_jobs,
			TransitionMode::Restart,
			[COMPONENT_BACKGROUND_JOBS],
		)
		.with_activity_source("scheduled-job initialization and dispatch"),
		ComponentDefinition::new(
			COMPONENT_PROVIDERS,
			"Metadata providers",
			"integration",
			cfg!(feature = "providers"),
			config.providers.enable_providers,
			TransitionMode::Restart,
			[COMPONENT_BACKGROUND_JOBS],
		),
		ComponentDefinition::new(
			COMPONENT_CROSSPOINT_DELIVERY,
			"CrossPoint delivery",
			"integration",
			cfg!(feature = "crosspoint"),
			true,
			TransitionMode::Restart,
			[COMPONENT_BACKGROUND_JOBS],
		),
		ComponentDefinition::new(
			COMPONENT_INGEST,
			"Staged ingest",
			"pipeline",
			cfg!(feature = "ingest"),
			true,
			TransitionMode::Restart,
			[COMPONENT_BACKGROUND_JOBS],
		)
		.with_activity_source("ingest service use"),
		ComponentDefinition::new(
			COMPONENT_WORKER,
			"Remote workers",
			"pipeline",
			true,
			true,
			TransitionMode::Restart,
			[COMPONENT_BACKGROUND_JOBS],
		)
		.with_activity_source("remote worker queue use"),
		ComponentDefinition::new(
			COMPONENT_DEVICES,
			"Device registry",
			"core",
			true,
			true,
			TransitionMode::Hot,
			Vec::<&str>::new(),
		),
		ComponentDefinition::new(
			COMPONENT_ANNOTATION_SYNC,
			"Annotation sync",
			"lifecycle",
			true,
			true,
			TransitionMode::Restart,
			Vec::<&str>::new(),
		),
		ComponentDefinition::new(
			COMPONENT_NOTIFICATIONS,
			"Notifications",
			"lifecycle",
			true,
			true,
			TransitionMode::Restart,
			Vec::<&str>::new(),
		),
		ComponentDefinition::new(
			COMPONENT_TRANSFORM,
			"Media transforms",
			"media",
			true,
			config.transform.transform_enabled,
			TransitionMode::Restart,
			Vec::<&str>::new(),
		),
		ComponentDefinition::new(
			COMPONENT_PDF,
			"PDF processing",
			"media",
			cfg!(feature = "pdf"),
			cfg!(feature = "pdf"),
			TransitionMode::Restart,
			Vec::<&str>::new(),
		),
		ComponentDefinition::new(
			COMPONENT_RAR,
			"RAR processing",
			"media",
			cfg!(feature = "rar"),
			cfg!(feature = "rar"),
			TransitionMode::Restart,
			Vec::<&str>::new(),
		),
	]
}
