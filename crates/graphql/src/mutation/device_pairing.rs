use async_graphql::{Context, Error, Object, Result, ID};
use chrono::Utc;
use models::{
	entity::device_pairing::{self, DevicePairingStatus, MAX_FAILED_ATTEMPTS},
	shared::enums::UserPermission,
};
use sea_orm::{prelude::*, DatabaseConnection};
use subtle::ConstantTimeEq;

use crate::{data::CoreContext, guard::PermissionGuard, object::device_pairing::DevicePairing};

#[derive(Default)]
pub struct DevicePairingMutation;

#[Object]
impl DevicePairingMutation {
	/// Approve a pending pairing and bind it to the calling user. Proof of
	/// possession is either the 6-digit `code` shown on the device or the `nonce`
	/// carried by its QR payload; exactly one must be given. After
	/// `MAX_FAILED_ATTEMPTS` wrong proofs the pairing is denied.
	#[graphql(guard = "PermissionGuard::one(UserPermission::AccessApiKeys)")]
	async fn approve_device_pairing(
		&self,
		ctx: &Context<'_>,
		pairing_id: ID,
		code: Option<String>,
		nonce: Option<String>,
	) -> Result<DevicePairing> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let now = DateTimeWithTimeZone::from(Utc::now());

		let pairing = load_pending(conn, pairing_id.as_str(), now).await?;

		// The poll that follows mints the credential as this user; refuse now
		// rather than leave the device polling an approved-but-forbidden pairing.
		let missing = stump_devices::required_permissions(pairing.kind)
			.into_iter()
			.filter(|permission| !user.has_permission(*permission))
			.collect::<Vec<_>>();
		if !missing.is_empty() {
			return Err(Error::new(format!(
				"Approving a {} device requires the {} permission{}",
				pairing.kind,
				missing
					.iter()
					.map(ToString::to_string)
					.collect::<Vec<_>>()
					.join(", "),
				if missing.len() == 1 { "" } else { "s" }
			)));
		}

		let proof_matches = match (code.as_deref(), nonce.as_deref()) {
			(Some(code), None) => bcrypt::verify(code.trim(), &pairing.code_hash)
				.map_err(|error| {
					tracing::error!(?error, "Failed to verify device pairing code");
					Error::new("Failed to verify the pairing code")
				})?,
			(None, Some(nonce)) => pairing
				.nonce
				.as_bytes()
				.ct_eq(nonce.trim().as_bytes())
				.into(),
			_ => {
				return Err(Error::new(
					"Provide exactly one of `code` or `nonce` to approve a pairing",
				))
			},
		};

		if !proof_matches {
			return Err(record_failed_attempt(conn, &pairing, &user.id).await?);
		}

		let approved = device_pairing::Entity::update_many()
			.col_expr(
				device_pairing::Column::Status,
				Expr::value(DevicePairingStatus::Approved),
			)
			.col_expr(
				device_pairing::Column::UserId,
				Expr::value(user.id.as_str()),
			)
			.col_expr(device_pairing::Column::ApprovedAt, Expr::value(now))
			.filter(pending_row(&pairing.id))
			.exec(conn)
			.await?
			.rows_affected;
		if approved != 1 {
			return Err(Error::new("Pairing is no longer pending"));
		}

		tracing::info!(
			pairing_id = %pairing.id,
			kind = ?pairing.kind,
			remote_ip = %pairing.remote_ip,
			user_id = %user.id,
			"Device pairing approved"
		);

		reload(conn, &pairing.id, now).await
	}

	/// Deny a pending pairing. Any user allowed to approve may deny, so a
	/// suspicious request can be shut down by whoever sees it first.
	#[graphql(guard = "PermissionGuard::one(UserPermission::AccessApiKeys)")]
	async fn deny_device_pairing(
		&self,
		ctx: &Context<'_>,
		pairing_id: ID,
	) -> Result<DevicePairing> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let now = DateTimeWithTimeZone::from(Utc::now());

		let pairing = load_pending(conn, pairing_id.as_str(), now).await?;

		let denied = deny(conn, &pairing.id, &user.id).await?;
		if !denied {
			return Err(Error::new("Pairing is no longer pending"));
		}

		tracing::info!(
			pairing_id = %pairing.id,
			kind = ?pairing.kind,
			remote_ip = %pairing.remote_ip,
			user_id = %user.id,
			"Device pairing denied"
		);

		reload(conn, &pairing.id, now).await
	}
}

fn pending_row(pairing_id: &str) -> sea_orm::Condition {
	sea_orm::Condition::all()
		.add(device_pairing::Column::Id.eq(pairing_id))
		.add(device_pairing::Column::Status.eq(DevicePairingStatus::Pending))
}

/// Fetch a pairing that can still be acted on, persisting expiry when the
/// deadline has passed so the audit row reads correctly afterwards.
async fn load_pending(
	conn: &DatabaseConnection,
	pairing_id: &str,
	now: DateTimeWithTimeZone,
) -> Result<device_pairing::Model> {
	let pairing = device_pairing::Entity::find_by_id(pairing_id)
		.one(conn)
		.await?
		.ok_or("Pairing not found")?;

	match pairing.effective_status(now) {
		DevicePairingStatus::Pending => Ok(pairing),
		DevicePairingStatus::Expired => {
			if pairing.status == DevicePairingStatus::Pending {
				device_pairing::Entity::update_many()
					.col_expr(
						device_pairing::Column::Status,
						Expr::value(DevicePairingStatus::Expired),
					)
					.filter(pending_row(&pairing.id))
					.exec(conn)
					.await?;
			}
			Err(Error::new("Pairing has expired"))
		},
		DevicePairingStatus::Approved => {
			Err(Error::new("Pairing has already been approved"))
		},
		DevicePairingStatus::Denied => Err(Error::new("Pairing has been denied")),
	}
}

/// Deny a pending pairing on behalf of `user_id`; `false` when it was no longer pending.
async fn deny(
	conn: &DatabaseConnection,
	pairing_id: &str,
	user_id: &str,
) -> Result<bool> {
	let rows = device_pairing::Entity::update_many()
		.col_expr(
			device_pairing::Column::Status,
			Expr::value(DevicePairingStatus::Denied),
		)
		.col_expr(device_pairing::Column::UserId, Expr::value(user_id))
		.filter(pending_row(pairing_id))
		.exec(conn)
		.await?
		.rows_affected;
	Ok(rows == 1)
}

/// Count a wrong proof and deny the pairing once the budget is spent. Returns
/// the error to hand back to the caller.
async fn record_failed_attempt(
	conn: &DatabaseConnection,
	pairing: &device_pairing::Model,
	user_id: &str,
) -> Result<Error> {
	device_pairing::Entity::update_many()
		.col_expr(
			device_pairing::Column::FailedAttempts,
			Expr::col(device_pairing::Column::FailedAttempts).add(1),
		)
		.filter(pending_row(&pairing.id))
		.exec(conn)
		.await?;

	let failed_attempts = device_pairing::Entity::find_by_id(pairing.id.as_str())
		.one(conn)
		.await?
		.map(|refreshed| refreshed.failed_attempts)
		.unwrap_or(pairing.failed_attempts + 1);

	tracing::warn!(
		pairing_id = %pairing.id,
		remote_ip = %pairing.remote_ip,
		user_id = %user_id,
		failed_attempts,
		"Device pairing approval failed: wrong code or nonce"
	);

	if failed_attempts >= MAX_FAILED_ATTEMPTS {
		deny(conn, &pairing.id, user_id).await?;
		return Ok(Error::new(
			"Too many failed attempts; the pairing has been denied",
		));
	}

	let remaining = MAX_FAILED_ATTEMPTS - failed_attempts;
	Ok(Error::new(format!(
		"Incorrect code or nonce ({remaining} attempt{} left)",
		if remaining == 1 { "" } else { "s" }
	)))
}

async fn reload(
	conn: &DatabaseConnection,
	pairing_id: &str,
	now: DateTimeWithTimeZone,
) -> Result<DevicePairing> {
	let pairing = device_pairing::Entity::find_by_id(pairing_id)
		.one(conn)
		.await?
		.ok_or("Pairing not found")?;
	Ok(DevicePairing::from_model(pairing, now))
}
