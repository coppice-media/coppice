use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue};
use serde::{Deserialize, Serialize};

use crate::shared::enums::DeviceKind;

/// How long a pairing request stays approvable after it was started.
pub const PAIRING_TTL_SECS: i64 = 5 * 60;
/// Number of wrong codes accepted before a pending pairing is denied.
pub const MAX_FAILED_ATTEMPTS: i32 = 5;

/// The lifecycle state of a device-pairing request. `Expired` is derived from
/// `expires_at` for pending rows and persisted lazily once observed, so a row's
/// stored status may still read `Pending` after the deadline; always go through
/// [`Model::effective_status`].
#[derive(
	Eq,
	Copy,
	Hash,
	Debug,
	Clone,
	Default,
	EnumIter,
	PartialEq,
	Serialize,
	Deserialize,
	DeriveActiveEnum,
)]
#[cfg_attr(feature = "graphql", derive(async_graphql::Enum))]
#[sea_orm(
	rs_type = "String",
	rename_all = "SCREAMING_SNAKE_CASE",
	db_type = "String(StringLen::None)"
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DevicePairingStatus {
	#[default]
	Pending,
	Approved,
	Denied,
	Expired,
}

/// A device-pairing request: an unauthenticated device asks to be paired, a
/// signed-in user approves it with the code shown on the device (or the QR nonce),
/// and the device then collects its credential exactly once. Rows are kept as the
/// audit trail of who approved which device from where.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "device_pairings")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	pub kind: DeviceKind,
	#[sea_orm(column_type = "Text", nullable)]
	pub name: Option<String>,
	/// bcrypt hash of the 6-digit code displayed on the device
	#[sea_orm(column_type = "Text")]
	pub code_hash: String,
	/// Random secret known only to the device (and to whoever scanned its QR). It
	/// authenticates status polls and is the proof-of-possession for QR approvals.
	#[sea_orm(column_type = "Text")]
	pub nonce: String,
	#[sea_orm(column_type = "Text")]
	pub remote_ip: String,
	/// The user who approved (or denied) the pairing; unset while pending.
	#[sea_orm(column_type = "Text", nullable)]
	pub user_id: Option<String>,
	pub status: DevicePairingStatus,
	pub failed_attempts: i32,
	/// Set once the credential has been handed to the device; it is never returned again.
	pub credential_issued: bool,
	/// Whether approval explicitly granted this Komelia pairing the metadata
	/// edit permission. False for every new and pre-existing pairing by default.
	pub allow_komf_metadata_editing: bool,
	pub created_at: DateTimeWithTimeZone,
	pub expires_at: DateTimeWithTimeZone,
	pub approved_at: Option<DateTimeWithTimeZone>,
}

impl Model {
	pub fn is_expired_at(&self, now: DateTimeWithTimeZone) -> bool {
		self.expires_at <= now
	}

	/// The status a client should observe at `now`: a pending pairing past its
	/// deadline reads as expired even if the row has not been updated yet.
	pub fn effective_status(&self, now: DateTimeWithTimeZone) -> DevicePairingStatus {
		match self.status {
			DevicePairingStatus::Pending if self.is_expired_at(now) => {
				DevicePairingStatus::Expired
			},
			status => status,
		}
	}
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::UserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	User,
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::User.def()
	}
}

#[async_trait]
impl ActiveModelBehavior for ActiveModel {
	async fn before_save<C>(mut self, _db: &C, insert: bool) -> Result<Self, DbErr>
	where
		C: ConnectionTrait,
	{
		if insert {
			let now = DateTimeWithTimeZone::from(Utc::now());
			self.created_at = ActiveValue::Set(now);
			if self.id.is_not_set() {
				self.id = ActiveValue::Set(Uuid::new_v4().to_string());
			}
			if self.expires_at.is_not_set() {
				self.expires_at =
					ActiveValue::Set(now + chrono::Duration::seconds(PAIRING_TTL_SECS));
			}
		}

		Ok(self)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn pairing(status: DevicePairingStatus, expires_in_secs: i64) -> Model {
		let now = DateTimeWithTimeZone::from(Utc::now());
		Model {
			id: "pairing".to_string(),
			kind: DeviceKind::Kobo,
			name: None,
			code_hash: String::new(),
			nonce: String::new(),
			remote_ip: "127.0.0.1".to_string(),
			user_id: None,
			status,
			failed_attempts: 0,
			credential_issued: false,
			allow_komf_metadata_editing: false,
			created_at: now,
			expires_at: now + chrono::Duration::seconds(expires_in_secs),
			approved_at: None,
		}
	}

	#[test]
	fn pending_past_deadline_reads_as_expired() {
		let now = DateTimeWithTimeZone::from(Utc::now());
		assert_eq!(
			pairing(DevicePairingStatus::Pending, -1).effective_status(now),
			DevicePairingStatus::Expired
		);
		assert_eq!(
			pairing(DevicePairingStatus::Pending, 60).effective_status(now),
			DevicePairingStatus::Pending
		);
	}

	#[test]
	fn settled_statuses_do_not_expire() {
		let now = DateTimeWithTimeZone::from(Utc::now());
		for status in [
			DevicePairingStatus::Approved,
			DevicePairingStatus::Denied,
			DevicePairingStatus::Expired,
		] {
			assert_eq!(pairing(status, -1).effective_status(now), status);
		}
	}
}
