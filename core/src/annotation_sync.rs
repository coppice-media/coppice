//! Annotation export wiring: the debounce hub, the settings-value encryption
//! helpers, and the per-user sink config rows.
//!
//! The canonical model and the concrete sinks live in
//! `stump_annotation_sync`; this module hosts them onto Stump:
//!
//! - every accepted annotation, bookmark, or reading-head change calls
//!   [`Ctx::note_annotation_activity`], which (re)arms a per-user deadline in
//!   the [`AnnotationSyncDebouncer`];
//! - a daemon loop (spawned by `StumpCore`) drains due users and enqueues
//!   [`StumpJob::AnnotationSync`], which runs
//!   [`crate::job::annotation_sync::AnnotationSyncJob`];
//! - the job loads the user's enabled `annotation_sink_configs` rows, builds
//!   the canonical batch (incremental by liseur CAS `seq`, full rebuild for
//!   native books), and runs each enabled sink, persisting the sink state and
//!   the last-run/last-error bookkeeping.
//!
//! Annotation changes deliberately do not become `CoreEvent`s: that stream is
//! broadcast to every GraphQL subscriber regardless of user (the same reason
//! reading heads use the separate `ReadingHeadChanged` channel).

use std::{
	collections::HashMap,
	sync::Mutex,
	time::{Duration, Instant},
};

use models::entity::annotation_sink_config;
use sea_orm::{prelude::*, ActiveValue::Set, QueryFilter, QueryOrder};
use serde_json::Value;
use stump_api_types::settings::{SettingDefinition, SettingValues};

use crate::{
	job::stump_job::StumpJob,
	utils::encryption::{decrypt_string, encrypt_string},
	CoreError, CoreResult, Ctx,
};

/// JSON marker wrapping encrypted secret setting values.
const ENCRYPTED_MARKER: &str = "__stump_encrypted";

// ---------------------------------------------------------------------------
// Debouncer
// ---------------------------------------------------------------------------

/// Per-user debounce deadlines for annotation export runs. An export is
/// scheduled once the user has been quiet for the configured delay; further
/// activity keeps pushing the deadline out.
pub struct AnnotationSyncDebouncer {
	deadlines: Mutex<HashMap<String, Instant>>,
	delay: Duration,
}

impl AnnotationSyncDebouncer {
	pub fn new(secs: u64) -> Self {
		Self {
			deadlines: Mutex::new(HashMap::new()),
			delay: Duration::from_secs(secs),
		}
	}

	/// (Re)arms the deadline for `user_id`.
	pub fn note(&self, user_id: &str) {
		self.note_at(user_id, Instant::now());
	}

	/// (Re)arms the deadline for `user_id` relative to `now`.
	pub fn note_at(&self, user_id: &str, now: Instant) {
		let mut deadlines = self
			.deadlines
			.lock()
			.expect("annotation sync debounce poisoned");
		deadlines.insert(user_id.to_owned(), now + self.delay);
	}

	/// Removes and returns every user whose deadline has passed, sorted.
	pub fn take_due(&self, now: Instant) -> Vec<String> {
		let mut deadlines = self
			.deadlines
			.lock()
			.expect("annotation sync debounce poisoned");
		let mut due: Vec<String> = deadlines
			.iter()
			.filter(|(_, deadline)| **deadline <= now)
			.map(|(user_id, _)| user_id.clone())
			.collect();
		for user_id in &due {
			deadlines.remove(user_id);
		}
		due.sort();
		due
	}

	/// Whether an export is currently scheduled for `user_id`.
	pub fn is_pending(&self, user_id: &str) -> bool {
		self.deadlines
			.lock()
			.expect("annotation sync debounce poisoned")
			.contains_key(user_id)
	}
}

/// Spawns the daemon loop that turns due debounce deadlines into
/// [`StumpJob::AnnotationSync`] enqueues. Called from `StumpCore`; harmless to
/// call without a running Tokio runtime (it logs and returns).
pub fn spawn_debounce_loop(ctx: Ctx) {
	if tokio::runtime::Handle::try_current().is_err() {
		tracing::debug!("annotation sync debounce loop not started: no tokio runtime");
		return;
	}

	tokio::spawn(async move {
		let mut interval = tokio::time::interval(Duration::from_secs(1));
		interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			interval.tick().await;
			if !ctx.background_jobs_enabled() {
				continue;
			}
			for user_id in ctx.annotation_debounce.take_due(Instant::now()) {
				tracing::debug!(user_id = %user_id, "Annotation sync debounce elapsed");
				if let Err(error) =
					ctx.enqueue(StumpJob::AnnotationSync { user_id }).await
				{
					tracing::error!(error = ?error, "Failed to enqueue annotation sync");
				}
			}
		}
	});
}

// ---------------------------------------------------------------------------
// Settings-value encryption helpers (host side; sinks see plaintext)
// ---------------------------------------------------------------------------

/// Encrypts every secret-marked string value in `values` with the server
/// encryption key. Encrypted values are stored as
/// `{"__stump_encrypted": "<base64>"}`.
pub fn encrypt_sink_values(
	definitions: &[SettingDefinition],
	values: &SettingValues,
	encryption_key: &str,
) -> CoreResult<SettingValues> {
	let encryption_key = encryption_key.to_owned();
	values
		.iter()
		.map(|(key, value)| {
			let is_secret = definitions
				.iter()
				.any(|definition| definition.key == key && definition.secret);
			match value.as_str().filter(|_| is_secret) {
				Some(plain) => {
					let encrypted = encrypt_string(plain, &encryption_key)?;
					Ok((
						key.clone(),
						serde_json::json!({ ENCRYPTED_MARKER: encrypted }),
					))
				},
				None => Ok((key.clone(), value.clone())),
			}
		})
		.collect()
}

/// Reverses [`encrypt_sink_values`]. Values without the marker pass through;
/// an encrypted value without a key is [`CoreError::EncryptionKeyNotSet`].
pub fn decrypt_sink_values(
	values: &SettingValues,
	encryption_key: Option<&str>,
) -> CoreResult<SettingValues> {
	let encryption_key = encryption_key.map(str::to_owned);
	values
		.iter()
		.map(|(key, value)| {
			let Some(encrypted) = value.get(ENCRYPTED_MARKER).and_then(Value::as_str)
			else {
				return Ok((key.clone(), value.clone()));
			};
			let encryption_key = encryption_key
				.as_ref()
				.ok_or(CoreError::EncryptionKeyNotSet)?;
			let plain = decrypt_string(encrypted, encryption_key)?;
			Ok((key.clone(), Value::String(plain)))
		})
		.collect()
}

// ---------------------------------------------------------------------------
// Sink config rows
// ---------------------------------------------------------------------------

/// One user's configuration row for a sink, for status rendering.
pub struct SinkStatusRow {
	pub sink_id: String,
	pub enabled: bool,
	pub last_run_at: Option<DateTimeWithTimeZone>,
	pub last_error: Option<String>,
}

/// Loads the per-sink status rows for a user (enabled and disabled), sorted
/// by sink id.
pub async fn sink_status_rows(
	conn: &DatabaseConnection,
	user_id: &str,
) -> CoreResult<Vec<SinkStatusRow>> {
	let rows = annotation_sink_config::Entity::find()
		.filter(annotation_sink_config::Column::UserId.eq(user_id))
		.order_by_asc(annotation_sink_config::Column::SinkId)
		.all(conn)
		.await?;
	Ok(rows
		.into_iter()
		.map(|row| SinkStatusRow {
			sink_id: row.sink_id,
			enabled: row.enabled,
			last_run_at: row.last_run_at,
			last_error: row.last_error,
		})
		.collect())
}

/// Upserts one sink config row for a user. Object settings are merged with
/// the existing row so omitting an encrypted secret does not erase it; values
/// must already be encrypted. Export state and last-run bookkeeping are
/// preserved.
pub async fn upsert_sink_config(
	conn: &DatabaseConnection,
	user_id: &str,
	sink_id: &str,
	settings: Option<Value>,
	enabled: bool,
) -> CoreResult<()> {
	let existing = annotation_sink_config::Entity::find()
		.filter(annotation_sink_config::Column::UserId.eq(user_id))
		.filter(annotation_sink_config::Column::SinkId.eq(sink_id))
		.one(conn)
		.await?;

	let now = chrono::Utc::now().fixed_offset();
	match existing {
		Some(row) => {
			let merged = merge_sink_settings(row.settings.clone(), settings);
			let mut active: annotation_sink_config::ActiveModel = row.into();
			active.settings = Set(merged);
			active.enabled = Set(enabled);
			active.updated_at = Set(now);
			active.update(conn).await?;
		},
		None => {
			annotation_sink_config::ActiveModel {
				user_id: Set(user_id.to_owned()),
				sink_id: Set(sink_id.to_owned()),
				settings: Set(settings),
				enabled: Set(enabled),
				last_run_at: Set(None),
				last_error: Set(None),
				state: Set(None),
				updated_at: Set(now),
			}
			.insert(conn)
			.await?;
		},
	}

	Ok(())
}

fn merge_sink_settings(
	existing: Option<Value>,
	incoming: Option<Value>,
) -> Option<Value> {
	match (existing, incoming) {
		(Some(Value::Object(mut existing)), Some(Value::Object(incoming))) => {
			for (key, value) in incoming {
				existing.insert(key, value);
			}
			Some(Value::Object(existing))
		},
		(_existing, Some(incoming)) => Some(incoming),
		(existing, None) => existing,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::utils::encryption::create_encryption_key;

	#[test]
	fn debounce_rearms_on_activity_and_drains_once() {
		let debouncer = AnnotationSyncDebouncer::new(30);
		let start = Instant::now();
		let secs = Duration::from_secs;

		debouncer.note_at("alice", start);
		assert!(debouncer.is_pending("alice"));
		assert!(debouncer.take_due(start + secs(29)).is_empty());

		// Activity at t=20 pushes the deadline out to t=50.
		debouncer.note_at("alice", start + secs(20));
		assert!(
			debouncer.take_due(start + secs(31)).is_empty(),
			"deadline must move with the latest activity"
		);

		assert_eq!(
			debouncer.take_due(start + secs(50)),
			vec!["alice".to_string()]
		);
		assert!(!debouncer.is_pending("alice"));
		assert!(debouncer.take_due(start + secs(60)).is_empty());
	}

	#[test]
	fn take_due_is_per_user_and_sorted() {
		let debouncer = AnnotationSyncDebouncer::new(0);
		debouncer.note("zoe");
		debouncer.note("bob");
		debouncer.note("alice");
		assert_eq!(
			debouncer.take_due(Instant::now()),
			vec!["alice".to_string(), "bob".to_string(), "zoe".to_string()]
		);
	}

	#[test]
	fn secret_values_round_trip_and_require_key() {
		let definitions = vec![SettingDefinition {
			key: "token",
			label: "Token",
			description: "",
			kind: stump_api_types::settings::SettingKind::String,
			default: Value::Null,
			required: false,
			secret: true,
			help_url: None,
		}];
		let key = create_encryption_key().unwrap();
		let values: SettingValues = [
			("token".to_string(), Value::String("s3cret".into())),
			("branch".to_string(), Value::String("main".into())),
		]
		.into_iter()
		.collect();

		let stored = encrypt_sink_values(&definitions, &values, &key).unwrap();
		assert_eq!(stored["branch"], Value::String("main".into()));
		assert!(stored["token"].get(ENCRYPTED_MARKER).is_some());
		assert_ne!(stored["token"], values["token"]);

		assert_eq!(decrypt_sink_values(&stored, Some(&key)).unwrap(), values);
		assert!(matches!(
			decrypt_sink_values(&stored, None),
			Err(CoreError::EncryptionKeyNotSet)
		));
		assert_eq!(decrypt_sink_values(&values, None).unwrap(), values);
	}
	#[test]
	fn omitted_sink_settings_are_merged_instead_of_erasing_secrets() {
		let encrypted = serde_json::json!({
			"token": { ENCRYPTED_MARKER: "ciphertext" },
			"branch": "main",
		});
		let updated = merge_sink_settings(
			Some(encrypted),
			Some(serde_json::json!({ "branch": "release" })),
		)
		.unwrap();
		assert_eq!(updated["token"][ENCRYPTED_MARKER], "ciphertext");
		assert_eq!(updated["branch"], "release");
	}
}
