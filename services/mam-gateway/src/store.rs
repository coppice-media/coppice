use crate::{
	error::{GatewayError, GatewayResult},
	handoff::HandoffMetadata,
	mam::{NormalizedResult, PublicResult},
};
use serde::Serialize;
use std::{
	collections::HashMap,
	time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GrabState {
	Queued,
	Downloading,
	Completed,
	Failed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicGrab {
	pub grab_id: String,
	pub result_id: String,
	#[serde(rename = "status")]
	pub state: GrabState,
	pub progress: f64,
	pub created_at: u64,
	pub updated_at: u64,
	pub expires_at: u64,
	#[serde(rename = "failureCode", skip_serializing_if = "Option::is_none")]
	pub failure_code: Option<&'static str>,
	#[serde(rename = "failureMessage", skip_serializing_if = "Option::is_none")]
	pub failure_message: Option<&'static str>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub handoff: Option<HandoffMetadata>,
}

#[derive(Clone)]
pub struct StoredResult {
	pub normalized: NormalizedResult,
	pub public: PublicResult,
	pub expires_at: u64,
}

#[derive(Clone)]
pub struct GrabRecord {
	pub id: String,
	pub result_id: String,
	pub tag: String,
	pub state: GrabState,
	pub progress: f64,
	pub created_at: u64,
	pub updated_at: u64,
	pub expires_at: u64,
	pub failure_code: Option<&'static str>,
	pub failure_message: Option<&'static str>,
	pub handoff: Option<HandoffMetadata>,
}

impl GrabRecord {
	pub fn public(&self) -> PublicGrab {
		PublicGrab {
			grab_id: self.id.clone(),
			result_id: self.result_id.clone(),
			state: self.state,
			progress: self.progress,
			created_at: self.created_at,
			updated_at: self.updated_at,
			expires_at: self.expires_at,
			failure_code: self.failure_code,
			failure_message: self.failure_message,
			handoff: self.handoff.clone(),
		}
	}
}

pub enum GrabReservation {
	Existing(GrabRecord),
	New {
		record: GrabRecord,
		result: StoredResult,
	},
}

pub struct ResultStore {
	results: HashMap<String, StoredResult>,
	grabs: HashMap<String, GrabRecord>,
	idempotency: HashMap<String, String>,
}

impl ResultStore {
	pub fn new() -> Self {
		Self {
			results: HashMap::new(),
			grabs: HashMap::new(),
			idempotency: HashMap::new(),
		}
	}

	pub fn insert_results(
		&mut self,
		normalized: Vec<NormalizedResult>,
		ttl: Duration,
		now: u64,
	) -> Vec<PublicResult> {
		self.prune(now);
		normalized
			.into_iter()
			.map(|normalized| {
				let id = new_opaque_id(&self.results);
				let expires_at = now.saturating_add(ttl.as_secs());
				let public = normalized.public(id.clone());
				self.results.insert(
					id.clone(),
					StoredResult {
						normalized,
						public: public.clone(),
						expires_at,
					},
				);
				public
			})
			.collect()
	}

	pub fn get_result(&mut self, id: &str, now: u64) -> GatewayResult<StoredResult> {
		self.prune(now);
		if !is_opaque_id(id) {
			return Err(GatewayError::NotFound);
		}
		self.results.get(id).cloned().ok_or(GatewayError::Expired)
	}

	pub fn reserve_grab(
		&mut self,
		result_id: &str,
		requested_key: &str,
		ttl: Duration,
		now: u64,
	) -> GatewayResult<GrabReservation> {
		self.prune(now);
		let result = self.get_result(result_id, now)?;
		let key = requested_key;
		validate_idempotency_key(key)?;
		if let Some(existing_id) = self.idempotency.get(key) {
			let existing = self
				.grabs
				.get(existing_id)
				.cloned()
				.ok_or(GatewayError::Internal)?;
			if existing.result_id != result_id {
				return Err(GatewayError::IdempotencyConflict);
			}
			return Ok(GrabReservation::Existing(existing));
		}

		let id = new_opaque_id(&self.grabs);
		let expires_at = now.saturating_add(ttl.as_secs());
		let record = GrabRecord {
			id: id.clone(),
			result_id: result_id.to_owned(),
			tag: format!("coppice:{id}"),
			state: GrabState::Queued,
			progress: 0.0,
			created_at: now,
			updated_at: now,
			expires_at,
			failure_code: None,
			failure_message: None,
			handoff: None,
		};
		self.idempotency.insert(key.to_owned(), id.clone());
		self.grabs.insert(id, record.clone());
		Ok(GrabReservation::New { record, result })
	}

	pub fn get_grab(&mut self, id: &str, now: u64) -> GatewayResult<GrabRecord> {
		self.prune(now);
		if !is_opaque_id(id) {
			return Err(GatewayError::NotFound);
		}
		self.grabs.get(id).cloned().ok_or(GatewayError::Expired)
	}
	pub fn mark_downloading(
		&mut self,
		id: &str,
		progress: f64,
		now: u64,
	) -> GatewayResult<GrabRecord> {
		let record = self.grabs.get_mut(id).ok_or(GatewayError::NotFound)?;
		record.state = GrabState::Downloading;
		record.progress = progress.clamp(0.0, 0.99);
		record.updated_at = now;
		Ok(record.clone())
	}

	pub fn mark_completed(
		&mut self,
		id: &str,
		handoff: HandoffMetadata,
		now: u64,
	) -> GatewayResult<GrabRecord> {
		let record = self.grabs.get_mut(id).ok_or(GatewayError::NotFound)?;
		record.state = GrabState::Completed;
		record.progress = 1.0;
		record.updated_at = now;
		record.failure_code = None;
		record.failure_message = None;
		record.handoff = Some(handoff);
		Ok(record.clone())
	}

	pub fn mark_failed(
		&mut self,
		id: &str,
		error: &'static str,
		now: u64,
	) -> GatewayResult<GrabRecord> {
		let record = self.grabs.get_mut(id).ok_or(GatewayError::NotFound)?;
		record.state = GrabState::Failed;
		record.updated_at = now;
		record.failure_code = Some(error);
		record.failure_message = Some(failure_message(error));
		record.handoff = None;
		Ok(record.clone())
	}

	fn prune(&mut self, now: u64) {
		self.results.retain(|_, result| result.expires_at > now);
		self.grabs.retain(|_, grab| grab.expires_at > now);
		self.idempotency
			.retain(|_, grab_id| self.grabs.contains_key(grab_id));
	}
}

impl Default for ResultStore {
	fn default() -> Self {
		Self::new()
	}
}

pub fn now_seconds() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}

pub fn is_opaque_id(value: &str) -> bool {
	value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn new_opaque_id<T>(existing: &HashMap<String, T>) -> String {
	loop {
		let id = Uuid::new_v4().simple().to_string();
		if !existing.contains_key(&id) {
			return id;
		}
	}
}

fn validate_idempotency_key(value: &str) -> GatewayResult<()> {
	if value.is_empty()
		|| value.len() > 128
		|| !value.bytes().all(|byte| {
			byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
		}) {
		return Err(GatewayError::BadRequest);
	}
	Ok(())
}

fn failure_message(code: &str) -> &'static str {
	match code {
		"multiple_payloads" => "completed download contains multiple book payloads",
		"unsupported_payload" => "completed download is not a supported book payload",
		"handoff_symlink" | "handoff_root_escape" | "handoff_path_invalid" => {
			"completed download failed handoff path safety checks"
		},
		"handoff_too_large" => "completed download exceeds the handoff size limit",
		"handoff_changed" => "completed download changed while it was being hashed",
		"download_client_failed" => "download client reported a failed torrent",
		_ => "grab failed before a safe handoff was ready",
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use url::Url;

	fn result() -> NormalizedResult {
		NormalizedResult {
			title: "Fixture".to_owned(),
			authors: vec![],
			category: 7000,
			size_bytes: 1,
			seeders: 1,
			leechers: 0,
			published_at: None,
			info_hash: None,
			download_url: Url::parse("https://t.myanonamouse.net/fixture").expect("url"),
		}
	}

	#[test]
	fn repeated_key_reserves_the_same_grab_without_a_second_record() {
		let mut store = ResultStore::new();
		let result_id = store.insert_results(vec![result()], Duration::from_secs(60), 10)
			[0]
		.result_id
		.clone();
		let first = store
			.reserve_grab(&result_id, "request-1", Duration::from_secs(60), 10)
			.expect("reservation");
		let second = store
			.reserve_grab(&result_id, "request-1", Duration::from_secs(60), 10)
			.expect("reservation");
		let first_id = match first {
			GrabReservation::New { record, .. } => record.id,
			GrabReservation::Existing(_) => panic!("first reservation reused"),
		};
		let second_id = match second {
			GrabReservation::Existing(record) => record.id,
			GrabReservation::New { .. } => panic!("second reservation created a row"),
		};
		assert_eq!(first_id, second_id);
	}
}
