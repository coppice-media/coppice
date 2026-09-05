//! `ServerController.server-info-slim` and `HealthController`.

use std::sync::Arc;

use axum::{
	http::header,
	response::{IntoResponse, Response},
	routing::get,
	Extension, Json, Router,
};
use sea_orm::{ConnectionTrait, Statement};

use crate::{
	dto::{KavitaDateTime, ServerInfoSlimDto},
	errors::APIResult,
	KAVITA_VERSION,
};

use super::{route_ci, KavitaBackend};

pub(crate) fn public_routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	route_ci(Router::<S>::new(), "/api/Health", get(health))
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	route_ci(
		Router::<S>::new(),
		"/api/Server/server-info-slim",
		get(server_info_slim),
	)
}

async fn health() -> Response {
	([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], "Ok").into_response()
}

/// Kavita's `installId` is an eight-character token generated on first
/// start; Stump allocates one on first request and keeps it in `kavita_ids`
/// (`kind = "install"`), so it is stable for the life of the database.
pub(crate) async fn install_id(
	conn: &impl ConnectionTrait,
) -> Result<String, sea_orm::DbErr> {
	let existing = conn
		.query_one(Statement::from_sql_and_values(
			conn.get_database_backend(),
			"SELECT stump_id FROM kavita_ids WHERE kind = 'install' ORDER BY id LIMIT 1",
			vec![],
		))
		.await?;
	if let Some(row) = existing {
		return row.try_get::<String>("", "stump_id");
	}
	let generated = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();
	conn.execute(Statement::from_sql_and_values(
		conn.get_database_backend(),
		"INSERT INTO kavita_ids (kind, stump_id) VALUES ('install', $1)
         ON CONFLICT(kind, stump_id) DO NOTHING",
		vec![generated.clone().into()],
	))
	.await?;
	install_id_existing(conn)
		.await
		.map(|id| id.unwrap_or(generated))
}

async fn install_id_existing(
	conn: &impl ConnectionTrait,
) -> Result<Option<String>, sea_orm::DbErr> {
	conn.query_one(Statement::from_sql_and_values(
		conn.get_database_backend(),
		"SELECT stump_id FROM kavita_ids WHERE kind = 'install' ORDER BY id LIMIT 1",
		vec![],
	))
	.await?
	.map(|row| row.try_get::<String>("", "stump_id"))
	.transpose()
}

async fn server_info_slim(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
) -> APIResult<Json<ServerInfoSlimDto>> {
	let facts = ctx.server_facts();
	Ok(Json(ServerInfoSlimDto {
		install_id: install_id(ctx.conn()).await?,
		is_docker: facts.is_docker,
		kavita_version: KAVITA_VERSION.to_owned(),
		first_install_date: facts.first_install_date.map(KavitaDateTime::from),
		first_install_version: Some(KAVITA_VERSION.to_owned()),
	}))
}
