use std::{thread, time::Duration};

use clap::Subcommand;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Password};
use models::entity::{
	age_restriction, liseur_sync_token, refresh_token, session, user, user_preferences,
};

use models::txn::begin_write;
use sea_orm::{
	prelude::*, sqlx::types::chrono::Utc, ActiveValue::Set, ConnectionTrait,
	DatabaseTransaction, DbBackend, IntoActiveModel, QueryTrait, Statement,
};
use stump_core::{config::StumpConfig, database::connect};

use crate::{error::CliResult, CliError};

use super::default_progress_spinner;

/// Subcommands for interacting with Stump accounts
#[derive(Subcommand, Debug)]
pub enum Account {
	/// Lock an account, preventing any further logins until unlocked
	Lock {
		/// The username of the account to lock
		#[clap(long)]
		username: String,
	},
	/// Unlock an account, allowing logins again
	Unlock {
		/// The username of the account to unlock
		#[clap(long)]
		username: String,
	},
	/// List all accounts, optionally filtering by locked status
	List {
		/// Only list locked accounts
		#[clap(long)]
		locked: Option<bool>,
	},
	/// Reset the password for an account
	ResetPassword {
		/// The username of the account to reset the password for
		#[clap(long)]
		username: String,
	},
	/// Enter a flow to change the server owner to another account
	ResetOwner,
	/// Migrate a local user account to an OIDC account
	MigrateOidc {
		/// The username of the local account to migrate
		#[clap(long)]
		username: String,
		/// The email of the OIDC account to migrate to
		#[clap(long)]
		oidc_email: String,
	},
}

pub async fn handle_account_command(
	command: Account,
	config: &StumpConfig,
) -> CliResult<()> {
	match command {
		Account::Lock { username } => {
			set_account_lock_status(username, true, config).await
		},
		Account::Unlock { username } => {
			set_account_lock_status(username, false, config).await
		},
		Account::List { locked } => print_accounts(locked, config).await,
		Account::ResetPassword { username } => {
			reset_account_password(username, config.auth.password_hash_cost, config).await
		},
		Account::ResetOwner => change_server_owner(config).await,
		Account::MigrateOidc {
			username,
			oidc_email,
		} => migrate_oidc_account(config, username, oidc_email).await,
	}
}
async fn set_account_lock_status(
	username: String,
	lock: bool,
	config: &StumpConfig,
) -> CliResult<()> {
	let progress = default_progress_spinner();
	progress.set_message(if lock {
		"Locking account..."
	} else {
		"Unlocking account..."
	});

	let conn = connect(config).await?;

	let user = user::Entity::find()
		.filter(user::Column::Username.eq(username.clone()))
		.one(&conn)
		.await?
		.ok_or_else(|| {
			progress.abandon_with_message("No account with that username was found");
			CliError::OperationFailed(String::from(
				"No account with that username was found",
			))
		})?;

	let mut active_model = user.into_active_model();
	active_model.is_locked = Set(lock);
	let updated_user = active_model.update(&conn).await?;

	if lock {
		progress.set_message("Removing active login sessions...");

		let delete_sessions = session::Entity::delete_many()
			.filter(session::Column::UserId.eq(updated_user.id.clone()))
			.exec(&conn)
			.await?
			.rows_affected;

		progress.set_message(format!("Removed {} active session(s)", delete_sessions));
	}

	thread::sleep(Duration::from_millis(500));

	progress.finish_with_message(if lock {
		"Account locked successfully!"
	} else {
		"Account unlocked successfully!"
	});
	Ok(())
}

async fn reset_account_password(
	username: String,
	hash_cost: u32,
	config: &StumpConfig,
) -> CliResult<()> {
	let conn = connect(config).await?;

	let theme = &ColorfulTheme::default();
	let builder = Password::with_theme(theme)
		.with_prompt("Enter a new password")
		.with_confirmation("Confirm password", "Passwords don't match!");
	let password = builder.interact()?;

	let progress = default_progress_spinner();
	progress.set_message("Hashing and salting password...");
	let hashed_password =
		bcrypt::hash(password, hash_cost).expect("Failed to hash password");

	progress.set_message("Updating account...");

	let user = user::Entity::find()
		.filter(user::Column::Username.eq(username.clone()))
		.one(&conn)
		.await?
		.ok_or_else(|| {
			progress.abandon_with_message("No account with that username was found");
			CliError::OperationFailed(String::from(
				"No account with that username was found",
			))
		})?;

	let mut active_model = user.into_active_model();
	active_model.hashed_password = Set(hashed_password);

	let _updated_user = active_model.update(&conn).await?;

	thread::sleep(Duration::from_millis(500));

	progress.finish_with_message("Account password updated successfully!");
	Ok(())
}

async fn print_accounts(locked: Option<bool>, config: &StumpConfig) -> CliResult<()> {
	let progress = default_progress_spinner();
	progress.set_message("Fetching accounts...");

	let conn = connect(config).await?;

	let users = models::entity::user::Entity::find()
		.apply_if(locked, |query, locked| {
			query.filter(user::Column::IsLocked.eq(locked))
		})
		.all(&conn)
		.await?;

	if users.is_empty() {
		progress.finish_with_message("No accounts found.");
	} else {
		progress.finish_with_message("Accounts fetched successfully!");

		let mut table = prettytable::Table::new();
		table.add_row(prettytable::row!["Account", "Status"]);

		for user in users {
			table.add_row(prettytable::row![
				user.username,
				if user.is_locked { "locked" } else { "unlocked" }
			]);
		}

		table.printstd();
	}

	Ok(())
}

async fn change_server_owner(config: &StumpConfig) -> CliResult<()> {
	let conn = connect(config).await?;

	let all_accounts = models::entity::user::Entity::find()
		.filter(user::Column::IsLocked.eq(false))
		.all(&conn)
		.await?;

	let current_server_owner = all_accounts
		.iter()
		.find(|user| user.is_server_owner)
		.cloned();

	let username = Input::new()
		.with_prompt("Enter the username of the account to assign as server owner")
		.allow_empty(false)
		.validate_with(|input: &String| -> Result<(), &str> {
			let existing_user = all_accounts.iter().find(|user| user.username == *input);
			if existing_user.is_some() {
				Ok(())
			} else {
				Err("An account with that username does not exist or their account is locked")
			}
		})
		.interact_text()?;

	let confirmation = Confirm::new()
		.with_prompt("Are you sure you want to continue?")
		.interact()?;

	if !confirmation {
		println!("Exiting...");
		return Ok(());
	}

	let target_user = all_accounts
		.into_iter()
		.find(|user| user.username == username)
		.ok_or(CliError::OperationFailed(
			"Failed to reconcile users after validation".to_string(),
		))?;

	let progress = default_progress_spinner();
	if let Some(user) = current_server_owner {
		progress.set_message(format!("Removing owner status from {}", user.username));
		let mut active_model = user.into_active_model();
		active_model.is_server_owner = Set(false);
		let updated_user = active_model.update(&conn).await?;

		session::Entity::delete_many()
			.filter(session::Column::UserId.eq(updated_user.id))
			.exec(&conn)
			.await?;
	}

	progress.set_message(format!("Setting owner status for {}", target_user.username));
	let mut active_model = target_user.into_active_model();
	active_model.is_server_owner = Set(true);
	let _updated_user = active_model.update(&conn).await?;
	session::Entity::delete_many()
		.filter(session::Column::UserId.eq(_updated_user.id))
		.exec(&conn)
		.await?;
	progress.finish_with_message("Successfully changed the server owner!");

	Ok(())
}

async fn migrate_oidc_account(
	config: &StumpConfig,
	username: String,
	oidc_email: String,
) -> CliResult<()> {
	let conn = connect(config).await?;

	let progress = default_progress_spinner();
	progress.set_message("Finding accounts...");

	// Find the local user (must not have oidc_issuer_id)
	let local_user = user::Entity::find()
		.filter(user::Column::Username.eq(username.clone()))
		.filter(user::Column::OidcIssuerId.is_null())
		.filter(user::Column::DeletedAt.is_null())
		.one(&conn)
		.await?
		.ok_or_else(|| {
			CliError::OperationFailed(format!(
				"No local account found with username '{}' (or account is already an OIDC account)",
				username
			))
		})?;

	// Find the OIDC user (must have oidc_issuer_id)
	let oidc_users = user::Entity::find()
		.filter(user::Column::OidcEmail.eq(oidc_email.clone()))
		.filter(user::Column::OidcIssuerId.is_not_null())
		.filter(user::Column::DeletedAt.is_null())
		.all(&conn)
		.await?;
	let [oidc_user] = <[_; 1]>::try_from(oidc_users).map_err(|matches| {
		CliError::OperationFailed(format!(
			"Expected exactly one active OIDC account with email '{}' (found {}); migration is unsafe",
			oidc_email,
			matches.len()
		))
	})?;

	progress.finish_and_clear();

	let is_server_owner = if local_user.is_server_owner && !oidc_user.is_server_owner {
		let transfer = Confirm::new()
			.with_prompt(format!(
				"The local account '{}' is currently the server owner. Transfer server ownership to the OIDC account '{}'?",
				local_user.username, oidc_user.username
			))
			.default(false)
			.interact()?;
		if !transfer {
			println!("Migration cancelled: the server owner's account cannot be deleted without transferring ownership.");
			return Ok(());
		}
		true
	} else {
		oidc_user.is_server_owner
	};

	println!("\nMigration Summary:");
	println!(
		"  Local account: {} (ID: {})",
		local_user.username, local_user.id
	);
	println!(
		"  OIDC account:  {} (ID: {})",
		oidc_user.username, oidc_user.id
	);
	println!("\nThis will:");
	println!(
		"  1. Transfer compatible database data (including devices and reading history)"
	);
	println!("  2. Transfer local preferences, permissions, restrictions and username");
	println!("  3. Revoke login sessions and refresh tokens for both accounts");
	println!("  4. Delete local account '{}'", local_user.username);
	if is_server_owner {
		println!("  5. Transfer server ownership to OIDC account");
	}

	let confirmation = Confirm::new()
		.with_prompt("\nAre you sure you want to continue?")
		.default(false)
		.interact()?;

	if !confirmation {
		println!("Migration cancelled.");
		return Ok(());
	}

	let progress = default_progress_spinner();

	let result = do_migrate_oidc_account(
		local_user,
		oidc_user,
		&conn,
		|message| progress.set_message(message.to_string()),
		is_server_owner,
	)
	.await;

	match result {
		Ok(_) => {
			progress.finish_with_message(format!(
				"Successfully migrated local account '{}' to OIDC account!",
				username
			));
			Ok(())
		},
		Err(e) => {
			progress.abandon_with_message(format!("Migration failed: {}", e));
			Err(e)
		},
	}
}

// Account data has more than one user-id spelling. Discover actual SQLite columns
// rather than keeping a list that silently falls behind the schema (including the
// raw Liseur tables, audit references and tables without user foreign keys).
fn is_user_reference(table: &str, column: &str) -> bool {
	if matches!(
		table,
		"users" | "user_preferences" | "sessions" | "refresh_tokens"
	) || column == "remote_user_id"
	{
		return false;
	}
	column == "user_id"
		|| column.ends_with("_user_id")
		|| matches!(
			column,
			"requester_id"
				| "approver_id"
				| "approved_by"
				| "rejected_by"
				| "creator_id"
				| "created_by_id"
				| "created_by"
				| "owner_id" | "decision_actor_id"
		) || table == "ingest_metadata_applications" && column == "actor"
}

fn quoted(identifier: &str) -> String {
	format!("\"{}\"", identifier.replace('"', "\"\""))
}

async fn user_references(txn: &DatabaseTransaction) -> CliResult<Vec<(String, String)>> {
	let mut references = Vec::new();
	let tables = txn
		.query_all(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"
				.to_owned(),
		))
		.await?;
	for table in tables {
		let name: String = table.try_get("", "name")?;
		for column in txn
			.query_all(Statement::from_string(
				DbBackend::Sqlite,
				format!("PRAGMA table_info({})", quoted(&name)),
			))
			.await?
		{
			let field: String = column.try_get("", "name")?;
			if is_user_reference(&name, &field) {
				references.push((name.clone(), field));
			}
		}
		// A future FK to users cannot escape just because its name differs.
		for fk in txn
			.query_all(Statement::from_string(
				DbBackend::Sqlite,
				format!("PRAGMA foreign_key_list({})", quoted(&name)),
			))
			.await?
		{
			let target: String = fk.try_get("", "table")?;
			let target_field: String = fk.try_get("", "to")?;
			let field: String = fk.try_get("", "from")?;
			if name != "users"
				&& !matches!(
					name.as_str(),
					"user_preferences" | "sessions" | "refresh_tokens"
				) && target == "users"
				&& target_field == "id"
				&& !references.contains(&(name.clone(), field.clone()))
			{
				references.push((name.clone(), field));
			}
		}
	}
	// Annotation attachments reference the composite key on Liseur annotations.
	// Update the parent first so SQLite can cascade the child identity.
	references.sort_by_key(|(table, _)| {
		if table == "liseur_sync_annotations" {
			0
		} else {
			1
		}
	});
	Ok(references)
}

async fn has_user_row(
	txn: &DatabaseTransaction,
	table: &str,
	column: &str,
	id: &str,
) -> CliResult<bool> {
	Ok(txn
		.query_one(Statement::from_sql_and_values(
			DbBackend::Sqlite,
			format!(
				"SELECT 1 FROM {} WHERE {} = ? LIMIT 1",
				quoted(table),
				quoted(column)
			),
			[id.into()],
		))
		.await?
		.is_some())
}

async fn transfer_references(
	txn: &DatabaseTransaction,
	references: &[(String, String)],
	source: &str,
	destination: &str,
) -> CliResult<()> {
	for (table, column) in references {
		txn.execute(Statement::from_sql_and_values(
			DbBackend::Sqlite,
			format!(
				"UPDATE {} SET {} = ? WHERE {} = ?",
				quoted(table),
				quoted(column),
				quoted(column)
			),
			[destination.into(), source.into()],
		))
		.await
		.map_err(|error| {
			CliError::OperationFailed(format!(
				"Cannot merge {}.{} without losing or overwriting account data: {}",
				table, column, error
			))
		})?;
	}
	Ok(())
}

async fn do_migrate_oidc_account<F>(
	local_user: user::Model,
	oidc_user: user::Model,
	conn: &DatabaseConnection,
	post_message: F,
	is_server_owner: bool,
) -> CliResult<()>
where
	F: Fn(&str),
{
	if conn.get_database_backend() != DbBackend::Sqlite {
		return Err(CliError::OperationFailed(
			"OIDC migration requires SQLite".into(),
		));
	}
	let txn = begin_write(conn).await?;
	// The prompts happen before BEGIN IMMEDIATE. Consent does not cover edits
	// another process made to either identity while the operator was deciding.
	if user::Entity::find_by_id(&local_user.id)
		.one(&txn)
		.await?
		.as_ref()
		!= Some(&local_user)
		|| user::Entity::find_by_id(&oidc_user.id)
			.one(&txn)
			.await?
			.as_ref() != Some(&oidc_user)
		|| local_user.oidc_issuer_id.is_some()
		|| oidc_user.oidc_issuer_id.is_none()
		|| local_user.deleted_at.is_some()
		|| oidc_user.deleted_at.is_some()
		|| local_user.id == oidc_user.id
	{
		return Err(CliError::OperationFailed(
			"Accounts changed while confirming; retry the OIDC migration".into(),
		));
	}
	if local_user.is_server_owner && !oidc_user.is_server_owner && !is_server_owner {
		return Err(CliError::OperationFailed(
			"Refusing to delete the server owner without confirmed transfer".into(),
		));
	}
	let owner = oidc_user.is_server_owner || is_server_owner;
	let refs = user_references(&txn).await?;
	// Liseur uses per-user sequence namespaces. Distinct row IDs do not make
	// two populated sync logs safe to concatenate (ops, annotations, aliases).
	const LISEUR_STATE: &[&str] = &[
		"liseur_sync_counters",
		"liseur_sync_works",
		"liseur_sync_editions",
		"liseur_sync_aliases",
		"liseur_sync_media_links",
		"liseur_sync_ops",
		"liseur_sync_sessions",
		"liseur_sync_annotations",
		"liseur_sync_settings",
		"liseur_sync_series_names",
	];
	let mut local_state = false;
	let mut destination_state = false;
	for (table, field) in refs.iter().filter(|(table, field)| {
		LISEUR_STATE.contains(&table.as_str()) && field == "user_id"
	}) {
		local_state |= has_user_row(&txn, table, field, &local_user.id).await?;
		destination_state |= has_user_row(&txn, table, field, &oidc_user.id).await?;
	}
	if local_state && destination_state {
		return Err(CliError::OperationFailed(
			"Both accounts have Liseur sync state; their sequence namespaces cannot be merged safely".into(),
		));
	}

	// Long-lived destination credentials must not gain authority when the
	// inherited user permissions/ownership change. Login sessions are revoked
	// below, but durable keys require explicit operator intervention.
	if local_user.permissions != oidc_user.permissions
		|| owner != oidc_user.is_server_owner
	{
		if has_user_row(&txn, "api_keys", "user_id", &oidc_user.id).await?
			|| txn.query_one(Statement::from_sql_and_values(
				DbBackend::Sqlite,
				"SELECT 1 FROM liseur_sync_tokens WHERE user_id = ? AND revoked_at IS NULL AND token_kind = 'device' LIMIT 1",
				[oidc_user.id.as_str().into()],
			)).await?.is_some()
		{
			return Err(CliError::OperationFailed(
				"OIDC account has active durable credentials; revoke them before changing permissions or ownership".into(),
			));
		}
	}
	if owner && !local_user.is_server_owner
		&& (has_user_row(&txn, "api_keys", "user_id", &local_user.id).await?
			|| txn.query_one(Statement::from_sql_and_values(
				DbBackend::Sqlite,
				"SELECT 1 FROM liseur_sync_tokens WHERE user_id = ? AND revoked_at IS NULL AND token_kind = 'device' LIMIT 1",
				[local_user.id.as_str().into()],
			)).await?.is_some())
	{
		return Err(CliError::OperationFailed(
			"Local credentials cannot move to an already privileged OIDC server owner".into(),
		));
	}
	// A queued annotation export or worker payload can embed the old user ID
	// without a relational column. There is no safe general-purpose rewrite for
	// arbitrary saved job state, so require it to be drained/archived first.
	for (table, fields) in [
		("jobs", &["description", "save_state", "output_data"][..]),
		("worker_jobs", &["input", "requires", "result"][..]),
	] {
		for field in fields {
			let sql = format!(
				"SELECT 1 FROM {} WHERE instr(CAST({} AS TEXT), ?) > 0 LIMIT 1",
				quoted(table),
				quoted(field)
			);
			if txn
				.query_one(Statement::from_sql_and_values(
					DbBackend::Sqlite,
					sql,
					[local_user.id.as_str().into()],
				))
				.await?
				.is_some()
			{
				return Err(CliError::OperationFailed(format!(
					"Saved {}.{} contains the old account ID; drain or archive this job before migrating",
					table, field
				)));
			}
		}
	}
	// user_preferences.user_id has no FK to users. Validate both directions
	// before the source delete, including rows not reachable via the user FK.
	for (account, label) in [(&local_user, "local"), (&oidc_user, "OIDC")] {
		let linked = user_preferences::Entity::find()
			.filter(user_preferences::Column::UserId.eq(&account.id))
			.all(&txn)
			.await?;
		if linked.first().map(|row| row.id) != account.user_preferences_id
			|| linked.len() != usize::from(account.user_preferences_id.is_some())
		{
			return Err(CliError::OperationFailed(format!(
				"{} account has inconsistent preferences links; resolve them before migration",
				label
			)));
		}
	}
	// Combine age restrictions before reassignment: user_id is unique, and
	// preserving the stricter restriction avoids widening access.
	let source_age = age_restriction::Entity::find()
		.filter(age_restriction::Column::UserId.eq(&local_user.id))
		.one(&txn)
		.await?;
	let target_age = age_restriction::Entity::find()
		.filter(age_restriction::Column::UserId.eq(&oidc_user.id))
		.one(&txn)
		.await?;
	if let (Some(source), Some(target)) = (source_age, target_age) {
		let stricter_age = source.age.min(target.age);
		let restrict_on_unset = source.restrict_on_unset || target.restrict_on_unset;
		let mut target = target.into_active_model();
		target.age = Set(stricter_age);
		target.restrict_on_unset = Set(restrict_on_unset);
		target.update(&txn).await?;
		age_restriction::Entity::delete_by_id(source.id)
			.exec(&txn)
			.await?;
	}

	// An approved pairing without an issued credential is still a bearer
	// capability: after reassignment its holder could mint a credential as the
	// destination user. Invalidate it before transferring the rest of the account.
	models::entity::device_pairing::Entity::update_many()
		.col_expr(
			models::entity::device_pairing::Column::Status,
			sea_orm::sea_query::Expr::value("DENIED"),
		)
		.col_expr(
			models::entity::device_pairing::Column::UserId,
			sea_orm::sea_query::Expr::value(Option::<String>::None),
		)
		.col_expr(
			models::entity::device_pairing::Column::ApprovedAt,
			sea_orm::sea_query::Expr::value(Option::<DateTimeWithTimeZone>::None),
		)
		.col_expr(
			models::entity::device_pairing::Column::AllowKomfMetadataEditing,
			sea_orm::sea_query::Expr::value(false),
		)
		.filter(models::entity::device_pairing::Column::UserId.eq(&local_user.id))
		.filter(
			models::entity::device_pairing::Column::Status
				.eq(models::entity::device_pairing::DevicePairingStatus::Approved),
		)
		.filter(models::entity::device_pairing::Column::CredentialIssued.eq(false))
		.exec(&txn)
		.await?;
	// Check real UNIQUE/FK constraints while transferring in this write
	// transaction. Any conflict rolls back every change before source deletion.
	post_message("Transferring account data with conflict checks...");
	transfer_references(&txn, &refs, &local_user.id, &oidc_user.id).await?;

	post_message("Revoking login and refresh sessions...");
	// Session credential links are polymorphic, not FK-backed; clean these
	// while retaining device registrations and their durable credentials.
	txn.execute(Statement::from_sql_and_values(
		DbBackend::Sqlite,
		"DELETE FROM device_credentials WHERE credential_kind = 'session' AND credential_ref IN (SELECT session_id FROM sessions WHERE user_id IN (?, ?))",
		[local_user.id.as_str().into(), oidc_user.id.as_str().into()],
	)).await?;
	for id in [&local_user.id, &oidc_user.id] {
		session::Entity::delete_many()
			.filter(session::Column::UserId.eq(id))
			.exec(&txn)
			.await?;
		refresh_token::Entity::delete_many()
			.filter(refresh_token::Column::UserId.eq(id))
			.exec(&txn)
			.await?;
	}
	liseur_sync_token::Entity::update_many()
		.col_expr(
			liseur_sync_token::Column::RevokedAt,
			sea_orm::sea_query::Expr::value(Some(Utc::now().to_rfc3339())),
		)
		.filter(liseur_sync_token::Column::UserId.eq(&oidc_user.id))
		.filter(liseur_sync_token::Column::RevokedAt.is_null())
		.filter(
			sea_orm::Condition::any()
				.add(liseur_sync_token::Column::TokenKind.eq("session"))
				.add(liseur_sync_token::Column::TokenKind.is_null()),
		)
		.exec(&txn)
		.await?;

	post_message("Transferring preferences and account restrictions...");
	if let (Some(previous), Some(source)) = (
		oidc_user.user_preferences_id,
		local_user.user_preferences_id,
	) {
		if previous != source {
			user_preferences::Entity::update_many()
				.col_expr(
					user_preferences::Column::UserId,
					sea_orm::sea_query::Expr::value(Option::<String>::None),
				)
				.filter(user_preferences::Column::Id.eq(previous))
				.exec(&txn)
				.await?;
		}
	}
	if let Some(source) = local_user.user_preferences_id {
		user_preferences::Entity::update_many()
			.col_expr(
				user_preferences::Column::UserId,
				sea_orm::sea_query::Expr::value(Some(oidc_user.id.clone())),
			)
			.filter(user_preferences::Column::Id.eq(source))
			.exec(&txn)
			.await?;
	}

	post_message("Deleting local account...");
	user::Entity::delete_by_id(&local_user.id)
		.exec(&txn)
		.await?;
	let mut destination = user::Entity::find_by_id(&oidc_user.id)
		.one(&txn)
		.await?
		.ok_or_else(|| {
			CliError::OperationFailed("OIDC account disappeared during migration".into())
		})?
		.into_active_model();
	destination.username = Set(local_user.username.clone());
	destination.permissions = Set(local_user.permissions);
	destination.user_preferences_id = Set(
		match local_user
			.user_preferences_id
			.or(oidc_user.user_preferences_id)
		{
			Some(id) => Some(id),
			None => Some(
				user_preferences::ActiveModel {
					user_id: Set(Some(oidc_user.id.clone())),
					..Default::default()
				}
				.insert(&txn)
				.await?
				.id,
			),
		},
	);
	destination.is_server_owner = Set(owner);
	destination.is_locked = Set(local_user.is_locked || oidc_user.is_locked);
	destination.max_sessions_allowed = Set(
		match (
			local_user.max_sessions_allowed,
			oidc_user.max_sessions_allowed,
		) {
			(Some(local), Some(oidc)) => Some(local.min(oidc)),
			(local, oidc) => local.or(oidc),
		},
	);
	destination.update(&txn).await?;
	if let (Some(previous), Some(source)) = (
		oidc_user.user_preferences_id,
		local_user.user_preferences_id,
	) {
		if previous != source {
			user_preferences::Entity::delete_by_id(previous)
				.exec(&txn)
				.await?;
		}
	}
	for (table, column) in &refs {
		if has_user_row(&txn, table, column, &local_user.id).await? {
			return Err(CliError::OperationFailed(format!(
				"Account reference {}.{} still points at the removed user",
				table, column
			)));
		}
	}
	if txn
		.query_one(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT 1 FROM pragma_foreign_key_check LIMIT 1".to_owned(),
		))
		.await?
		.is_some()
	{
		return Err(CliError::OperationFailed(
			"Migration would leave invalid foreign keys; rolled back".into(),
		));
	}
	post_message("Committing changes...");
	txn.commit().await?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use migrations::{Migrator, MigratorTrait};
	use models::{
		entity::{
			api_key, bookmark, favorite_library, favorite_media, favorite_series,
			last_library_visit, library, library_config, library_exclusion, media,
			media_annotation, reading_session, refresh_token, review, series, session,
			user, user_login_activity, user_preferences,
		},
		shared::{
			api_key::APIKeyPermissions,
			enums::{
				FileStatus, LibraryPattern, LibraryViewMode, ReadingDirection,
				ReadingImageScaleFit, ReadingMode, ReadingStatus,
			},
			readium::ReadiumLocator,
		},
	};
	use sea_orm::{
		prelude::{Date, DateTimeWithTimeZone},
		sqlx::types::chrono::Utc,
		ActiveModelTrait, ColumnTrait, ConnectionTrait, Database, DbBackend, DbConn,
		EntityTrait, QueryFilter, Set, Statement,
	};

	use super::{do_migrate_oidc_account, quoted, user_references};

	async fn test_database() -> DbConn {
		let db = Database::connect("sqlite::memory:")
			.await
			.expect("failed to connect to test database");

		Migrator::up(&db, None).await.expect("Failed to migrate");

		db
	}

	#[derive(Default)]
	struct ExampleUser {}

	impl ExampleUser {
		fn active_model() -> user::ActiveModel {
			user::ActiveModel {
				username: sea_orm::Set("oromei".to_string()),
				hashed_password: sea_orm::Set("hashed_password".to_string()),
				is_server_owner: sea_orm::Set(true),
				is_locked: sea_orm::Set(false),
				..Default::default()
			}
		}

		async fn insert(
			&self,
			db: &DbConn,
			user: Option<user::ActiveModel>,
		) -> user::Model {
			let model = user.unwrap_or_else(Self::active_model);

			let user = model.insert(db).await.expect("could not insert user");
			let user_preferences = user_preferences::ActiveModel {
				user_id: Set(Some(user.id.clone())),
				..Default::default()
			}
			.insert(db)
			.await
			.expect("could not insert user preferences");

			user::Entity::update_many()
				.col_expr(
					user::Column::UserPreferencesId,
					sea_orm::sea_query::Expr::value(Some(user_preferences.id)),
				)
				.filter(user::Column::Id.eq(user.id.clone()))
				.exec(db)
				.await
				.expect("could not update user with preferences id");

			user::Entity::find_by_id(user.id)
				.one(db)
				.await
				.expect("could not find updated user")
				.expect("user should exist after update")
		}
	}

	fn library_config() -> library_config::ActiveModel {
		library_config::ActiveModel {
			convert_rar_to_zip: Set(false),
			hard_delete_conversions: Set(false),
			default_reading_dir: Set(ReadingDirection::Ltr),
			default_reading_mode: Set(ReadingMode::Paged),
			default_reading_image_scale_fit: Set(ReadingImageScaleFit::Height),
			generate_file_hashes: Set(false),
			generate_koreader_hashes: Set(false),
			process_metadata: Set(true),
			watch: Set(false),
			library_pattern: Set(LibraryPattern::SeriesBased),
			default_library_view_mode: Set(LibraryViewMode::Series),
			hide_series_view: Set(false),
			skip_book_overview: Set(false),
			process_thumbnail_colors_even_without_config: Set(false),
			..Default::default()
		}
	}

	async fn setup_oidc_migration_test(local_user: &user::Model, db: &DbConn) {
		let library_config_1 = library_config()
			.insert(db)
			.await
			.expect("could not insert library config");

		let library = library::ActiveModel {
			name: Set("Test Library".to_string()),
			path: Set("/test/library".to_string()),
			status: Set(FileStatus::Ready),
			config_id: Set(library_config_1.id),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("could not insert library");

		let library_config_2 = library_config()
			.insert(db)
			.await
			.expect("could not insert library config");

		let excluded_from_local_user_library = library::ActiveModel {
			name: Set("Test Library EXCLUDED".to_string()),
			path: Set("/test/library-excluded".to_string()),
			status: Set(FileStatus::Ready),
			config_id: Set(library_config_2.id),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("could not insert library");

		let series = series::ActiveModel {
			name: Set("Test Series".to_string()),
			library_id: Set(Some(library.id.clone())),
			path: Set("/test/series".to_string()),
			status: Set(FileStatus::Ready),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("could not insert series");

		for i in 0..5 {
			let _media = media::ActiveModel {
				name: Set(format!("Test Media {}", i + 1)),
				path: Set(format!("/test/series/media{}", i + 1)),
				series_id: Set(Some(series.id.clone())),
				extension: Set("cbz".to_string()),
				pages: Set(100 + i),
				status: Set(FileStatus::Ready),
				size: Set(1024),
				..Default::default()
			}
			.insert(db)
			.await
			.expect("could not insert media");
		}

		let media_list = media::Entity::find()
			.filter(media::Column::SeriesId.eq(series.id.clone()))
			.all(db)
			.await
			.expect("could not fetch media");

		// let's have an active session for books 0 and 2, and a finished reading session for book 1
		for (i, media) in media_list.iter().enumerate().take(3) {
			let mut active = reading_session::ActiveModel {
				user_id: Set(local_user.id.clone()),
				media_id: Set(media.id.clone()),
				end_page: Set(Some(10)),
				session_date: Set(Date::parse_from_str("2026-05-22", "%Y-%m-%d")
					.expect("failed to parse date")),
				..Default::default()
			};
			if i == 1 {
				active.status = Set(ReadingStatus::Finished)
			}
			active
				.insert(db)
				.await
				.expect("could not insert reading session for media");
		}

		// let's create a review for media 0 as well
		review::ActiveModel {
			user_id: Set(local_user.id.clone()),
			media_id: Set(media_list[0].id.clone()),
			rating: Set(4),
			content: Set(Some("Great book!".to_string())),
			is_private: Set(false),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("could not insert review for media 0");

		// bookmark for media 2
		bookmark::ActiveModel {
			user_id: Set(local_user.id.clone()),
			media_id: Set(media_list[2].id.clone()),
			locator: Set(Some(ReadiumLocator {
				chapter_title: "Chapter Foo".to_string(),
				href: "chapter1.html".to_string(),
				locations: None,
				text: None,
				title: Some("Chapter 1".to_string()),
				r#type: "application/xhtml+xml".to_string(),
				kobo_span: None,
			})),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("could not insert bookmark for media 2");

		// media annotation for media 3
		media_annotation::ActiveModel {
			user_id: Set(local_user.id.clone()),
			media_id: Set(media_list[3].id.clone()),
			locator: Set(ReadiumLocator {
				chapter_title: "Chapter 3".to_string(),
				href: "chapter3.html".to_string(),
				locations: None,
				text: None,
				title: Some("Chapter 3".to_string()),
				r#type: "application/xhtml+xml".to_string(),
				kobo_span: None,
			}),
			annotation_text: Set(Some("Important note!".to_string())),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("could not insert media annotation for media 3");

		favorite_library::ActiveModel {
			user_id: Set(local_user.id.clone()),
			library_id: Set(library.id.clone()),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("could not insert favorite library");

		favorite_media::ActiveModel {
			user_id: Set(local_user.id.clone()),
			media_id: Set(media_list[0].id.clone()),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("could not insert favorite media");

		favorite_series::ActiveModel {
			user_id: Set(local_user.id.clone()),
			series_id: Set(series.id.clone()),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("could not insert favorite series");

		last_library_visit::ActiveModel {
			user_id: Set(local_user.id.clone()),
			library_id: Set(library.id.clone()),
			timestamp: Set(DateTimeWithTimeZone::from(Utc::now())),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("could not insert last library visit");

		library_exclusion::ActiveModel {
			user_id: Set(local_user.id.clone()),
			library_id: Set(excluded_from_local_user_library.id.clone()),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("could not insert library exclusion for local user");

		api_key::ActiveModel {
			user_id: Set(local_user.id.clone()),
			long_token_hash: Set("hashed_token".to_string()),
			short_token: Set("short".to_string()),
			name: Set("Test API Key".to_string()),
			permissions: Set(APIKeyPermissions::inherit()),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("could not insert API key for local user");

		// give them an auth session, refresh token, and one login activity record each as well
		session::ActiveModel {
			user_id: Set(local_user.id.clone()),
			session_id: Set("session123".to_string()),
			// i know now = expired but its fine for this
			expiry_time: Set(DateTimeWithTimeZone::from(Utc::now())),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("could not insert session for local user");

		refresh_token::ActiveModel {
			id: Set("refresh123".to_string()),
			expires_at: Set(DateTimeWithTimeZone::from(Utc::now())),
			user_id: Set(local_user.id.clone()),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("could not insert refresh token for local user");

		for i in 0..2 {
			user_login_activity::ActiveModel {
				user_id: Set(local_user.id.clone()),
				authentication_successful: Set(i % 2 == 0),
				ip_address: Set("localhost".to_string()),
				timestamp: Set(DateTimeWithTimeZone::from(Utc::now())),
				user_agent: Set("TestAgent/1.0".to_string()),
				..Default::default()
			}
			.insert(db)
			.await
			.expect("could not insert login activity for local user");
		}
	}

	#[tokio::test]
	async fn test_oidc_user_migration() {
		let db = test_database().await;

		let local_user = ExampleUser {}.insert(&db, None).await;
		let oidc_user = ExampleUser {}
			.insert(
				&db,
				Some(user::ActiveModel {
					username: Set("oidc_user".to_string()),
					oidc_email: Set(Some("user@proton.me".to_string())),
					oidc_issuer_id: Set(Some("https://example.com/oidc".to_string())),
					is_server_owner: Set(true),
					..ExampleUser::active_model()
				}),
			)
			.await;

		setup_oidc_migration_test(&local_user, &db).await;

		models::entity::device_pairing::ActiveModel {
			id: Set("oidc-migration-unissued-pairing".to_owned()),
			kind: Set(models::shared::enums::DeviceKind::Api),
			name: Set(Some("unissued API pairing".to_owned())),
			code_hash: Set("unused".to_owned()),
			nonce: Set("unused".to_owned()),
			remote_ip: Set("127.0.0.1".to_owned()),
			user_id: Set(Some(local_user.id.clone())),
			status: Set(models::entity::device_pairing::DevicePairingStatus::Approved),
			failed_attempts: Set(0),
			credential_issued: Set(false),
			allow_komf_metadata_editing: Set(true),
			approved_at: Set(Some(Utc::now().into())),
			..Default::default()
		}
		.insert(&db)
		.await
		.expect("could not insert an unissued approved pairing");

		let result = do_migrate_oidc_account(
			local_user.clone(),
			oidc_user.clone(),
			&db,
			|_| {},
			true,
		)
		.await;

		assert!(result.is_ok(), "Migration failed: {:?}", result.err());
		let pairing = models::entity::device_pairing::Entity::find_by_id(
			"oidc-migration-unissued-pairing",
		)
		.one(&db)
		.await
		.unwrap()
		.expect("pairing remains as a denied audit row");
		assert_eq!(
			pairing.status,
			models::entity::device_pairing::DevicePairingStatus::Denied
		);
		assert!(pairing.user_id.is_none());
		assert!(!pairing.credential_issued);
		assert!(!pairing.allow_komf_metadata_editing);

		let local_user_check = user::Entity::find_by_id(&local_user.id)
			.one(&db)
			.await
			.expect("Failed to query local user");
		assert!(local_user_check.is_none(), "Local user should be deleted");

		let updated_oidc_user = user::Entity::find_by_id(&oidc_user.id)
			.one(&db)
			.await
			.expect("Failed to query OIDC user")
			.expect("OIDC user should exist");
		assert_eq!(
			updated_oidc_user.username, local_user.username,
			"OIDC user should have local user's username"
		);
		assert_eq!(
			updated_oidc_user.user_preferences_id, local_user.user_preferences_id,
			"OIDC user should have local user's preferences"
		);
		assert!(
			updated_oidc_user.is_server_owner,
			"OIDC user should be server owner"
		);

		if let Some(prefs_id) = updated_oidc_user.user_preferences_id {
			let preferences = user_preferences::Entity::find_by_id(prefs_id)
				.one(&db)
				.await
				.expect("Failed to query user preferences")
				.expect("User preferences should exist");
			// see https://discord.com/channels/972593831172272148/1490415985524609264/1491118111401705494
			assert_eq!(
				preferences.user_id,
				Some(oidc_user.id.clone()),
				"User preferences should point back to OIDC user"
			);
		} else {
			panic!("OIDC user should have preferences");
		}

		let active_sessions = reading_session::Entity::find()
			.filter(reading_session::Column::UserId.eq(&oidc_user.id))
			.filter(reading_session::Column::Status.eq(ReadingStatus::Reading))
			.all(&db)
			.await
			.expect("Failed to query reading sessions");
		assert_eq!(
			active_sessions.len(),
			2,
			"Should have 2 reading sessions transferred"
		);

		let finished_sessions = reading_session::Entity::find()
			.filter(reading_session::Column::UserId.eq(&oidc_user.id))
			.filter(reading_session::Column::Status.eq(ReadingStatus::Finished))
			.all(&db)
			.await
			.expect("Failed to query finished reading sessions");
		assert_eq!(
			finished_sessions.len(),
			1,
			"Should have 1 finished reading session transferred"
		);

		let reviews = review::Entity::find()
			.filter(review::Column::UserId.eq(&oidc_user.id))
			.all(&db)
			.await
			.expect("Failed to query reviews");
		assert_eq!(reviews.len(), 1, "Should have 1 review transferred");

		let bookmarks = bookmark::Entity::find()
			.filter(bookmark::Column::UserId.eq(&oidc_user.id))
			.all(&db)
			.await
			.expect("Failed to query bookmarks");
		assert_eq!(bookmarks.len(), 1, "Should have 1 bookmark transferred");

		let annotations = media_annotation::Entity::find()
			.filter(media_annotation::Column::UserId.eq(&oidc_user.id))
			.all(&db)
			.await
			.expect("Failed to query annotations");
		assert_eq!(annotations.len(), 1, "Should have 1 annotation transferred");

		let favorite_libraries = favorite_library::Entity::find()
			.filter(favorite_library::Column::UserId.eq(&oidc_user.id))
			.all(&db)
			.await
			.expect("Failed to query favorite libraries");
		assert_eq!(
			favorite_libraries.len(),
			1,
			"Should have 1 favorite library transferred"
		);

		let favorite_media_list = favorite_media::Entity::find()
			.filter(favorite_media::Column::UserId.eq(&oidc_user.id))
			.all(&db)
			.await
			.expect("Failed to query favorite media");
		assert_eq!(
			favorite_media_list.len(),
			1,
			"Should have 1 favorite media transferred"
		);

		let favorite_series_list = favorite_series::Entity::find()
			.filter(favorite_series::Column::UserId.eq(&oidc_user.id))
			.all(&db)
			.await
			.expect("Failed to query favorite series");
		assert_eq!(
			favorite_series_list.len(),
			1,
			"Should have 1 favorite series transferred"
		);

		let exclusions = library_exclusion::Entity::find()
			.filter(library_exclusion::Column::UserId.eq(&oidc_user.id))
			.all(&db)
			.await
			.expect("Failed to query library exclusions");
		assert_eq!(
			exclusions.len(),
			1,
			"Should have 1 library exclusion transferred"
		);

		let visits = last_library_visit::Entity::find()
			.filter(last_library_visit::Column::UserId.eq(&oidc_user.id))
			.all(&db)
			.await
			.expect("Failed to query library visits");
		assert_eq!(visits.len(), 1, "Should have 1 library visit transferred");

		let api_keys = api_key::Entity::find()
			.filter(api_key::Column::UserId.eq(&oidc_user.id))
			.all(&db)
			.await
			.expect("Failed to query API keys");
		assert_eq!(api_keys.len(), 1, "Should have 1 API key transferred");

		let login_activity = user_login_activity::Entity::find()
			.filter(user_login_activity::Column::UserId.eq(&oidc_user.id))
			.all(&db)
			.await
			.expect("Failed to query login activity");
		assert_eq!(
			login_activity.len(),
			2,
			"Should have 2 login activity records transferred"
		);

		let sessions = session::Entity::find()
			.filter(session::Column::UserId.eq(&local_user.id))
			.all(&db)
			.await
			.expect("Failed to query sessions");
		assert_eq!(sessions.len(), 0, "Local user's sessions should be deleted"); // not transferred

		let refresh_tokens = refresh_token::Entity::find()
			.filter(refresh_token::Column::UserId.eq(&local_user.id))
			.all(&db)
			.await
			.expect("Failed to query refresh tokens");
		assert_eq!(
			refresh_tokens.len(),
			0,
			"Local user's refresh tokens should be deleted"
		); // not transferred
	}
	#[tokio::test]
	async fn test_oidc_migration_denies_unissued_pairing_before_owner_transfer() {
		let db = test_database().await;
		let local_user = ExampleUser {}
			.insert(
				&db,
				Some(user::ActiveModel {
					is_server_owner: Set(false),
					..ExampleUser::active_model()
				}),
			)
			.await;
		let oidc_user = ExampleUser {}
			.insert(
				&db,
				Some(user::ActiveModel {
					username: Set("oidc_owner".to_owned()),
					oidc_email: Set(Some("owner@example.com".to_owned())),
					oidc_issuer_id: Set(Some("https://example.com/oidc".to_owned())),
					is_server_owner: Set(true),
					..ExampleUser::active_model()
				}),
			)
			.await;
		models::entity::device_pairing::ActiveModel {
			id: Set("owner-transfer-unissued-pairing".to_owned()),
			kind: Set(models::shared::enums::DeviceKind::Api),
			name: Set(Some("unissued API pairing".to_owned())),
			code_hash: Set("unused".to_owned()),
			nonce: Set("unused".to_owned()),
			remote_ip: Set("127.0.0.1".to_owned()),
			user_id: Set(Some(local_user.id.clone())),
			status: Set(models::entity::device_pairing::DevicePairingStatus::Approved),
			failed_attempts: Set(0),
			credential_issued: Set(false),
			allow_komf_metadata_editing: Set(true),
			approved_at: Set(Some(Utc::now().into())),
			..Default::default()
		}
		.insert(&db)
		.await
		.expect("insert pairing");

		do_migrate_oidc_account(local_user, oidc_user.clone(), &db, |_| {}, true)
			.await
			.expect("owner transfer");
		let pairing = models::entity::device_pairing::Entity::find_by_id(
			"owner-transfer-unissued-pairing",
		)
		.one(&db)
		.await
		.unwrap()
		.expect("denied audit row remains");
		assert_eq!(
			pairing.status,
			models::entity::device_pairing::DevicePairingStatus::Denied
		);
		assert!(pairing.user_id.is_none());
		let owner = user::Entity::find_by_id(oidc_user.id)
			.one(&db)
			.await
			.unwrap()
			.unwrap();
		assert!(owner.is_server_owner);
	}

	async fn temporary_database() -> (DbConn, std::path::PathBuf) {
		let path = std::env::temp_dir().join(format!(
			"coppice-oidc-{}.sqlite",
			sea_orm::prelude::Uuid::new_v4()
		));
		let db = Database::connect(format!("sqlite://{}?mode=rwc", path.display()))
			.await
			.expect("open disposable on-disk SQLite");
		Migrator::up(&db, None)
			.await
			.expect("migrate disposable DB");
		(db, path)
	}

	async fn count(db: &DbConn, table: &str, column: &str, id: &str) -> i64 {
		db.query_one(Statement::from_sql_and_values(
			DbBackend::Sqlite,
			format!(
				"SELECT count(*) AS n FROM {} WHERE {} = ?",
				quoted(table),
				quoted(column)
			),
			[id.into()],
		))
		.await
		.unwrap()
		.unwrap()
		.try_get("", "n")
		.unwrap()
	}

	async fn seed_current_account_data(
		db: &DbConn,
		source: &user::Model,
		target: &user::Model,
	) {
		setup_oidc_migration_test(source, db).await;
		let media = media::Entity::find().all(db).await.unwrap();
		let sql = r#"
INSERT INTO age_restrictions (user_id, age, restrict_on_unset) VALUES ('{source}',12,1),('{target}',18,0);
INSERT INTO devices (id,user_id,name,kind,created_at,library_scope) VALUES ('oidc-device','{source}','Reader','koreader',CURRENT_TIMESTAMP,'[]');
INSERT INTO device_credentials (device_id,protocol,credential_kind,credential_ref) VALUES
 ('oidc-device','koreader','api_key','short'),('oidc-device','koreader','session','session123');
INSERT INTO liseur_sync_tokens (id,user_id,device_id,secret_hash,scopes,created_at,expires_at,token_kind) VALUES
 ('oidc-device-token','{source}','oidc-device','oidc-hash-1','["sync"]','2026-01-01','2030-01-01','device'),
 ('oidc-login-token','{source}','oidc-device','oidc-hash-2','["sync"]','2026-01-01','2030-01-01','session'),
 ('oidc-target-login','{target}','target-device','oidc-hash-3','["sync"]','2026-01-01','2030-01-01','session');
INSERT INTO reading_head_events (user_id,media_id,protocol,raw_payload,timestamp_kind,source_updated_at,received_at,applied)
 VALUES ('{source}','{media}','stump','{"page":1}','server',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,1);
INSERT INTO reading_heads (user_id,media_id,progression,page,completed,updated_at,created_at,changed_at,source_protocol,revision,event_id)
 VALUES ('{source}','{media}',0.1,1,0,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,'stump',1,(SELECT max(id) FROM reading_head_events));
INSERT INTO reading_head_events (user_id,media_id,protocol,raw_payload,timestamp_kind,source_updated_at,received_at,applied)
 VALUES ('{target}','{target_media}','stump','{"page":2}','server',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,1);
INSERT INTO reading_heads (user_id,media_id,progression,page,completed,updated_at,created_at,changed_at,source_protocol,revision,event_id)
 VALUES ('{target}','{target_media}',0.2,2,0,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,'stump',1,(SELECT max(id) FROM reading_head_events));
INSERT INTO liseur_sync_works (id,user_id,title,author,pending,created_at)
 VALUES ('oidc-work','{source}','Migration work','A',0,'2026-01-01');
INSERT INTO liseur_sync_counters (user_id,op_seq,annotation_seq,projected_annotation_seq)
 VALUES ('{source}',1,1,1);
INSERT INTO liseur_sync_annotations
 (user_id,annotation_id,rev,seq,work_id,kind,excerpt,color,body,device_id,client_ts,updated_at,deleted,payload)
 VALUES ('{source}','oidc-annotation',1,1,'oidc-work','highlight','excerpt','yellow','note','oidc-device','2026-01-01','2026-01-01',0,'{}');
INSERT INTO annotation_attachments (id,user_id,annotation_id,kind,media_type,byte_size,sha256,storage_path,created_at)
 VALUES ('oidc-attachment','{source}','oidc-annotation','markup-svg','image/svg+xml',5,'abcd','test/markup.svg','2026-01-01');
INSERT INTO hardcover_connections (user_id,encrypted_api_token,credential_version,remote_user_id)
 VALUES ('{source}','synthetic-encrypted-credential',1,'external-user');
INSERT INTO hardcover_journal_provenance (id,user_id,remote_entry_id,status,created_at,updated_at)
 VALUES ('oidc-provenance','{source}','remote-entry','unresolved',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
INSERT INTO kindle_destinations (id,user_id,name,email,is_default,created_at,updated_at)
 VALUES ('oidc-kindle','{source}','Reader','reader@example.invalid',1,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
INSERT INTO book_requests (id,requester_id,title,format,status,approval_policy,approved_by) VALUES
 ('oidc-request','{source}','Requested','EBOOK','APPROVED','REQUIRED','{source}'),
 ('oidc-audit','{target}','Shared approval','EBOOK','APPROVED','REQUIRED','{source}');
INSERT INTO book_request_approvals (id,request_id,approver_id,decision)
 VALUES ('oidc-approval','oidc-request','{source}','APPROVED');
INSERT INTO notification_rules (user_id,event_kind,channel_id,enabled)
 VALUES ('{source}','request','email',1);
INSERT INTO book_clubs (id,name,slug,is_private,created_at)
 VALUES ('oidc-club','Migration club','oidc-club',0,CURRENT_TIMESTAMP);
INSERT INTO book_club_members (id,user_id,book_club_id,hide_progress,role,joined_at)
 VALUES ('oidc-member','{source}','oidc-club',0,0,CURRENT_TIMESTAMP);
INSERT INTO book_club_discussions (id,is_locked,is_archived,is_pinned,book_club_id)
 VALUES ('oidc-discussion',0,0,0,'oidc-club');
INSERT INTO book_club_discussion_message (id,content,timestamp,is_pinned_message,discussion_id,member_id,book_club_id)
 VALUES ('oidc-message','Preserved discussion',CURRENT_TIMESTAMP,0,'oidc-discussion','oidc-member','oidc-club');
"#
			.replace("{source}", &source.id)
			.replace("{target}", &target.id)
			.replace("{media}", &media[0].id)
			.replace("{target_media}", &media[1].id);
		db.execute_unprepared(&sql)
			.await
			.expect("seed real account records");
		user_preferences::Entity::update_many()
			.col_expr(
				user_preferences::Column::Locale,
				sea_orm::sea_query::Expr::value("fr"),
			)
			.filter(user_preferences::Column::Id.eq(source.user_preferences_id.unwrap()))
			.exec(db)
			.await
			.unwrap();
	}

	#[tokio::test]
	async fn test_oidc_current_data_survives_temporary_db_migration() {
		let (db, path) = temporary_database().await;
		let source = ExampleUser::default()
			.insert(
				&db,
				Some(user::ActiveModel {
					username: Set("current-local".into()),
					is_server_owner: Set(false),
					permissions: Set(Some("READ_USERS".into())),
					max_sessions_allowed: Set(Some(1)),
					..ExampleUser::active_model()
				}),
			)
			.await;
		let target = ExampleUser::default()
			.insert(
				&db,
				Some(user::ActiveModel {
					username: Set("current-oidc".into()),
					is_server_owner: Set(false),
					oidc_email: Set(Some("current@example.invalid".into())),
					oidc_issuer_id: Set(Some("https://example.invalid".into())),
					max_sessions_allowed: Set(Some(2)),
					..ExampleUser::active_model()
				}),
			)
			.await;
		// The user model deliberately initializes new accounts unlocked.
		user::Entity::update_many()
			.col_expr(
				user::Column::IsLocked,
				sea_orm::sea_query::Expr::value(true),
			)
			.filter(user::Column::Id.eq(&target.id))
			.exec(&db)
			.await
			.unwrap();
		let target = user::Entity::find_by_id(target.id)
			.one(&db)
			.await
			.unwrap()
			.unwrap();
		seed_current_account_data(&db, &source, &target).await;
		session::ActiveModel {
			user_id: Set(target.id.clone()),
			session_id: Set("oidc-target-session".into()),
			expiry_time: Set(DateTimeWithTimeZone::from(Utc::now())),
			..Default::default()
		}
		.insert(&db)
		.await
		.unwrap();
		refresh_token::ActiveModel {
			id: Set("oidc-target-refresh".into()),
			user_id: Set(target.id.clone()),
			expires_at: Set(DateTimeWithTimeZone::from(Utc::now())),
			..Default::default()
		}
		.insert(&db)
		.await
		.unwrap();
		let txn = models::txn::begin_write(&db).await.unwrap();
		let refs = user_references(&txn).await.unwrap();
		txn.rollback().await.unwrap();
		let mut before = Vec::new();
		for (table, column) in refs {
			let n = count(&db, &table, &column, &source.id).await;
			if n > 0 {
				before.push((
					table.clone(),
					column.clone(),
					n,
					count(&db, &table, &column, &target.id).await,
				));
			}
		}
		for table in [
			"reading_heads",
			"devices",
			"liseur_sync_annotations",
			"annotation_attachments",
			"hardcover_connections",
			"hardcover_journal_provenance",
			"kindle_destinations",
			"book_requests",
			"notification_rules",
			"book_club_members",
		] {
			assert!(
				before.iter().any(|(name, _, _, _)| name == table),
				"missing fixture: {table}"
			);
		}
		do_migrate_oidc_account(source.clone(), target.clone(), &db, |_| {}, false)
			.await
			.unwrap();
		assert!(user::Entity::find_by_id(&source.id)
			.one(&db)
			.await
			.unwrap()
			.is_none());
		let migrated = user::Entity::find_by_id(&target.id)
			.one(&db)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(migrated.username, source.username);
		assert_eq!(migrated.oidc_issuer_id, target.oidc_issuer_id);
		assert_eq!(migrated.oidc_email, target.oidc_email);
		assert_eq!(migrated.permissions, source.permissions);
		assert_eq!(migrated.max_sessions_allowed, Some(1));
		assert!(!migrated.is_server_owner);
		assert!(
			migrated.is_locked,
			"the destination's lock may not be lifted"
		);
		assert_eq!(migrated.user_preferences_id, source.user_preferences_id);
		assert_eq!(
			user_preferences::Entity::find_by_id(migrated.user_preferences_id.unwrap())
				.one(&db)
				.await
				.unwrap()
				.unwrap()
				.locale,
			"fr"
		);
		for (table, column, old, current) in before {
			assert_eq!(
				count(&db, &table, &column, &source.id).await,
				0,
				"orphaned {table}.{column}"
			);
			if table != "age_restrictions" {
				assert_eq!(
					count(&db, &table, &column, &target.id).await,
					old + current,
					"lost {table}.{column}"
				);
			}
		}
		assert!(
			db.query_one(Statement::from_string(
				DbBackend::Sqlite,
				"SELECT 1 FROM book_club_discussion_message WHERE id = 'oidc-message'"
			))
			.await
			.unwrap()
			.is_some(),
			"member-owned discussions must survive the account merge"
		);
		let restriction = db
			.query_one(Statement::from_sql_and_values(
				DbBackend::Sqlite,
				"SELECT age, restrict_on_unset FROM age_restrictions WHERE user_id = ?",
				[target.id.as_str().into()],
			))
			.await
			.unwrap()
			.unwrap();
		assert_eq!(restriction.try_get::<i64>("", "age").unwrap(), 12);
		assert_eq!(
			restriction.try_get::<i64>("", "restrict_on_unset").unwrap(),
			1
		);
		for id in [&source.id, &target.id] {
			assert_eq!(count(&db, "sessions", "user_id", id).await, 0);
			assert_eq!(count(&db, "refresh_tokens", "user_id", id).await, 0);
		}
		assert_eq!(
			count(&db, "device_credentials", "credential_kind", "session").await,
			0
		);
		assert_eq!(
			count(&db, "device_credentials", "credential_kind", "api_key").await,
			1
		);
		let token = db
			.query_one(Statement::from_string(
				DbBackend::Sqlite,
				"SELECT revoked_at FROM liseur_sync_tokens WHERE id = 'oidc-login-token'",
			))
			.await
			.unwrap()
			.unwrap();
		assert!(token
			.try_get::<Option<String>>("", "revoked_at")
			.unwrap()
			.is_some());
		let token = db.query_one(Statement::from_string(DbBackend::Sqlite,
			"SELECT revoked_at FROM liseur_sync_tokens WHERE id = 'oidc-device-token'")).await.unwrap().unwrap();
		assert!(token
			.try_get::<Option<String>>("", "revoked_at")
			.unwrap()
			.is_none());
		db.close().await.unwrap();
		std::fs::remove_file(path).unwrap();
	}

	#[tokio::test]
	async fn test_oidc_conflict_rolls_back_temporary_db() {
		let (db, path) = temporary_database().await;
		let source = ExampleUser::default()
			.insert(
				&db,
				Some(user::ActiveModel {
					username: Set("conflict-local".into()),
					is_server_owner: Set(false),
					..ExampleUser::active_model()
				}),
			)
			.await;
		let target = ExampleUser::default()
			.insert(
				&db,
				Some(user::ActiveModel {
					username: Set("conflict-oidc".into()),
					is_server_owner: Set(false),
					oidc_email: Set(Some("conflict@example.invalid".into())),
					oidc_issuer_id: Set(Some("https://example.invalid".into())),
					..ExampleUser::active_model()
				}),
			)
			.await;
		seed_current_account_data(&db, &source, &target).await;
		db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
			"INSERT INTO hardcover_connections (user_id,encrypted_api_token,credential_version) VALUES (?,'destination-credential',1)",
			[target.id.as_str().into()])).await.unwrap();
		let err =
			do_migrate_oidc_account(source.clone(), target.clone(), &db, |_| {}, false)
				.await
				.expect_err("conflicting provider connections must not overwrite");
		assert!(
			err.to_string().contains("hardcover_connections.user_id"),
			"{err}"
		);
		assert!(user::Entity::find_by_id(&source.id)
			.one(&db)
			.await
			.unwrap()
			.is_some());
		assert_eq!(
			count(&db, "hardcover_connections", "user_id", &source.id).await,
			1
		);
		assert_eq!(
			count(&db, "hardcover_connections", "user_id", &target.id).await,
			1
		);
		assert_eq!(
			count(&db, "liseur_sync_annotations", "user_id", &source.id).await,
			1
		);
		assert_eq!(
			count(&db, "book_requests", "requester_id", &source.id).await,
			1
		);
		assert_eq!(count(&db, "sessions", "user_id", &source.id).await, 1);
		assert_eq!(
			count(&db, "age_restrictions", "user_id", &source.id).await,
			1
		);
		assert_eq!(
			count(&db, "age_restrictions", "user_id", &target.id).await,
			1
		);
		let original_target_age = db
			.query_one(Statement::from_sql_and_values(
				DbBackend::Sqlite,
				"SELECT age FROM age_restrictions WHERE user_id = ?",
				[target.id.as_str().into()],
			))
			.await
			.unwrap()
			.unwrap();
		assert_eq!(
			original_target_age.try_get::<i64>("", "age").unwrap(),
			18,
			"an earlier age-restriction merge must roll back on conflict"
		);
		assert_eq!(
			user::Entity::find_by_id(&target.id)
				.one(&db)
				.await
				.unwrap()
				.unwrap()
				.username,
			target.username
		);
		db.close().await.unwrap();
		std::fs::remove_file(path).unwrap();
	}
	#[tokio::test]
	async fn test_oidc_conflicting_liseur_namespaces_are_rejected() {
		let db = test_database().await;
		let source = ExampleUser::default()
			.insert(
				&db,
				Some(user::ActiveModel {
					username: Set("liseur-local".into()),
					is_server_owner: Set(false),
					..ExampleUser::active_model()
				}),
			)
			.await;
		let target = ExampleUser::default()
			.insert(
				&db,
				Some(user::ActiveModel {
					username: Set("liseur-oidc".into()),
					is_server_owner: Set(false),
					oidc_issuer_id: Set(Some("https://example.invalid".into())),
					oidc_email: Set(Some("liseur@example.invalid".into())),
					..ExampleUser::active_model()
				}),
			)
			.await;
		for account in [&source, &target] {
			db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
				"INSERT INTO liseur_sync_counters (user_id, op_seq, annotation_seq) VALUES (?, 1, 1)",
				[account.id.as_str().into()])).await.unwrap();
		}
		let err =
			do_migrate_oidc_account(source.clone(), target.clone(), &db, |_| {}, false)
				.await
				.expect_err("two user-scoped sequence histories cannot be concatenated");
		assert!(err.to_string().contains("sequence namespaces"), "{err}");
		assert!(user::Entity::find_by_id(&source.id)
			.one(&db)
			.await
			.unwrap()
			.is_some());
		assert_eq!(
			count(&db, "liseur_sync_counters", "user_id", &source.id).await,
			1
		);
		assert_eq!(
			count(&db, "liseur_sync_counters", "user_id", &target.id).await,
			1
		);
	}

	#[tokio::test]
	async fn test_oidc_preserves_destination_preferences_without_local_preferences() {
		let db = test_database().await;
		let source = ExampleUser::default()
			.insert(
				&db,
				Some(user::ActiveModel {
					username: Set("no-prefs-local".into()),
					is_server_owner: Set(false),
					..ExampleUser::active_model()
				}),
			)
			.await;
		let target = ExampleUser::default()
			.insert(
				&db,
				Some(user::ActiveModel {
					username: Set("no-prefs-oidc".into()),
					is_server_owner: Set(false),
					oidc_issuer_id: Set(Some("https://example.invalid".into())),
					oidc_email: Set(Some("no-prefs@example.invalid".into())),
					..ExampleUser::active_model()
				}),
			)
			.await;
		db.execute(Statement::from_sql_and_values(
			DbBackend::Sqlite,
			"UPDATE users SET user_preferences_id = NULL WHERE id = ?",
			[source.id.as_str().into()],
		))
		.await
		.unwrap();
		user_preferences::Entity::delete_by_id(source.user_preferences_id.unwrap())
			.exec(&db)
			.await
			.unwrap();
		let source = user::Entity::find_by_id(source.id)
			.one(&db)
			.await
			.unwrap()
			.unwrap();
		do_migrate_oidc_account(source, target.clone(), &db, |_| {}, false)
			.await
			.unwrap();
		let migrated = user::Entity::find_by_id(&target.id)
			.one(&db)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(migrated.user_preferences_id, target.user_preferences_id);
		assert_eq!(
			count(&db, "user_preferences", "user_id", &target.id).await,
			1
		);
	}

	#[tokio::test]
	async fn test_oidc_rejects_queued_user_id_outside_foreign_keys() {
		let db = test_database().await;
		let source = ExampleUser::default()
			.insert(
				&db,
				Some(user::ActiveModel {
					username: Set("queued-local".into()),
					is_server_owner: Set(false),
					..ExampleUser::active_model()
				}),
			)
			.await;
		let target = ExampleUser::default()
			.insert(
				&db,
				Some(user::ActiveModel {
					username: Set("queued-oidc".into()),
					is_server_owner: Set(false),
					oidc_issuer_id: Set(Some("https://example.invalid".into())),
					oidc_email: Set(Some("queued@example.invalid".into())),
					..ExampleUser::active_model()
				}),
			)
			.await;
		db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
			"INSERT INTO worker_jobs (id,kind,input,requires,status,created_at,updated_at) VALUES ('queued-account-work','test',?,'{}','queued',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)",
			[serde_json::json!({"user_id": source.id}).to_string().into()])).await.unwrap();
		let err = do_migrate_oidc_account(source.clone(), target, &db, |_| {}, false)
			.await
			.expect_err("queued JSON cannot be remapped safely");
		assert!(err.to_string().contains("worker_jobs.input"), "{err}");
		assert!(user::Entity::find_by_id(&source.id)
			.one(&db)
			.await
			.unwrap()
			.is_some());
		assert!(db
			.query_one(Statement::from_string(
				DbBackend::Sqlite,
				"SELECT 1 FROM worker_jobs WHERE id = 'queued-account-work'"
			))
			.await
			.unwrap()
			.is_some());
	}
}
