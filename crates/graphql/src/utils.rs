use sea_orm::{ConnectionTrait, DatabaseConnection, Statement, Value};

#[cfg(feature = "web")]
use models::entity::user::AuthUser;
#[cfg(feature = "web")]
use tower_sessions::Session;

/// Persist the viewer back into the session cookie after `updateViewerPreferences`
/// rewrites the cached preferences.
#[cfg(feature = "web")]
pub async fn save_user_session(session: &Session, user: AuthUser) {
	if let Err(error) = session.insert("user", user).await {
		tracing::error!(?error, "Failed to save user session");
	}
}

/// Build a raw SQL [`Statement`] using the actual database backend of `conn`.
/// Write SQL with `$1`, `$2`, … placeholders — they work for both PostgreSQL
/// and SQLite (SQLite treats them as named parameters).
pub fn db_statement(
	conn: &DatabaseConnection,
	sql: impl Into<String>,
	values: impl IntoIterator<Item = Value>,
) -> Statement {
	Statement::from_sql_and_values(conn.get_database_backend(), sql, values)
}
