use chrono::Utc;
use sea_orm::{
	entity::prelude::*,
	prelude::async_trait::async_trait,
	sea_query::{Alias, Query, SelectStatement},
	ActiveValue, Condition, DeriveEntityModel, FromJsonQueryResult, FromQueryResult,
	Identity, JoinType, QueryOrder, QuerySelect, RelationDef,
};
use serde::{Deserialize, Serialize};

use crate::{
	entity::device,
	prefixer::{parse_query_to_model, parse_query_to_model_optional, Prefixer},
	shared::{enums::ReadingStatus, readium::ReadiumLocator},
};

use super::user::AuthUser;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
pub struct DeviceIds(pub Vec<String>);

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "ReadingSessionModel"))]
#[sea_orm(table_name = "reading_sessions")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = true)]
	pub id: i32,

	/// the "logical" date of this session, based on user prefs and start time
	pub session_date: Date,

	#[sea_orm(column_type = "Json", nullable)]
	pub start_locator: Option<ReadiumLocator>,
	#[sea_orm(column_type = "Json", nullable)]
	pub end_locator: Option<ReadiumLocator>,

	pub start_page: Option<i32>,
	pub end_page: Option<i32>,

	/// Where a listening session stopped, in milliseconds from the start of
	/// the publication. The time-addressed twin of `end_page`/`end_locator`.
	pub end_position_ms: Option<i64>,

	pub start_percentage: Option<Decimal>,
	pub end_percentage: Option<Decimal>,

	#[sea_orm(column_type = "Text", nullable)]
	pub koreader_progress: Option<String>,
	/// accumulated reading time for this session, updated via deltas (not overwritten)
	pub elapsed_seconds: Option<i64>,

	/// which read-through of this book this session belongs to (1-indexed)
	#[sea_orm(default_value = "1")]
	pub readthrough_number: i32,

	/// the status of this session. this might feel confusing when considering that sessions will
	/// remain in place even after completion/dnf, but the idea is that the status represents the
	/// state of the session when it was last updated
	#[sea_orm(column_type = "Text")]
	pub status: ReadingStatus,

	#[sea_orm(column_type = "Text", nullable)]
	pub notes: Option<String>,

	/// The latest native Kobo ReadingState request for this session.
	///
	/// Kobo's wire payload contains location fields and statistics that do not
	/// have a native column in the unified reading-session model. Keeping the
	/// request here lets the Kobo adapter replay those fields without treating
	/// the normalized Readium locator as lossless.
	#[sea_orm(column_type = "Json", nullable)]
	pub kobo_state: Option<serde_json::Value>,

	/// all device ids that contributed updates to this session
	#[cfg_attr(feature = "graphql", graphql(skip))]
	#[sea_orm(column_type = "Json", nullable)]
	pub device_ids: Option<DeviceIds>,

	/// Liseur's immutable closed-session id when this row was projected from
	/// `liseur_sync_sessions`; native sessions leave it unset.
	#[cfg_attr(feature = "graphql", graphql(skip))]
	#[sea_orm(column_type = "Text", nullable)]
	pub liseur_session_id: Option<String>,

	#[sea_orm(column_type = "Text")]
	pub media_id: String,
	#[sea_orm(column_type = "Text")]
	pub user_id: String,

	pub created_at: DateTimeWithTimeZone,
	pub updated_at: Option<DateTimeWithTimeZone>,
}

impl Model {
	/// whether this session is "finalized"
	pub fn is_finalized(&self) -> bool {
		matches!(
			self.status,
			ReadingStatus::Finished | ReadingStatus::Abandoned
		)
	}

	/// whether this session represents a completed readthrough (i.e. status = Finished)
	pub fn is_complete(&self) -> bool {
		self.status == ReadingStatus::Finished
	}
}

/// A reading-session/device pair returned by [`ModelWithDevice::find`].
///
/// `find().into_model::<ModelWithDevice>().all(...)` returns one row per
/// registered device in `model.device_ids`.
#[derive(Debug, Clone)]
pub struct ModelWithDevice {
	pub model: Model,
	pub device: Option<device::Model>,
}

impl ModelWithDevice {
	pub fn find() -> Select<Entity> {
		Prefixer::new(Entity::find().select_only())
			.add_columns(Entity)
			.add_columns(device::Entity)
			.selector
			// `json_each` turns every stored device id into a join candidate instead
			// of resolving only the first array element.
			.join(
				JoinType::LeftJoin,
				{
					let mut relation: RelationDef = Entity::belongs_to(device::Entity)
						.from(Column::Id)
						.to(device::Column::Id)
						.on_condition(|_left, _right| {
							Condition::all().add(Expr::cust(
								"EXISTS (SELECT 1 FROM json_each(reading_sessions.device_ids) WHERE json_each.value = devices.id)",
							))
						})
						.into();
					// Empty key identities suppress the generated equality; the
					// JSON membership predicate above is the complete join.
					relation.from_col = Identity::Many(Vec::new());
					relation.to_col = Identity::Many(Vec::new());
					relation
				},
			)
	}
}

impl FromQueryResult for ModelWithDevice {
	fn from_query_result(
		res: &sea_orm::QueryResult,
		_pre: &str,
	) -> Result<Self, sea_orm::DbErr> {
		let model = parse_query_to_model::<Model, Entity>(res)?;
		let device = parse_query_to_model_optional::<device::Model, device::Entity>(res)?;
		Ok(Self { model, device })
	}
}

// The JSON array cannot be represented by a SeaORM relation. The custom
// `json_each` join above expands it without requiring a junction table.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::media::Entity",
		from = "Column::MediaId",
		to = "super::media::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Media,
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::UserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	User,
}

impl Related<super::media::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Media.def()
	}
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::User.def()
	}
}

impl Entity {
	/// subquery used to detect if there is a newer session row for the same
	/// `(user_id, media_id)` as the outer `reading_sessions` row
	pub fn newer_session_exists_subquery() -> SelectStatement {
		let inner_alias = Alias::new("rs2");

		// select 1 from reading_sessions as rs2
		Query::select()
			.expr(Expr::val(1))
			.from_as(Entity, inner_alias.clone())
			// where the book/media is matching
			.and_where(
				Expr::col((inner_alias.clone(), Column::UserId))
					.eq(Expr::col((Entity, Column::UserId))),
			)
			.and_where(
				Expr::col((inner_alias.clone(), Column::MediaId))
					.eq(Expr::col((Entity, Column::MediaId))),
			)
			// some of these may feel odd, but there are some edge cases that are more plausible.
			// in particular, i know there are multiple places in the codebase where i set
			// updated_at/created_at to the same timestamp (e.g., for newly created ones)
			.cond_where(
				// and one of the following is true:
				Condition::any()
					// the updated_at is newer than outer row's (i.e., more recent session update)
					.add(
						Expr::col((inner_alias.clone(), Column::UpdatedAt))
							.gt(Expr::col((Entity, Column::UpdatedAt))),
					)
					// or, if updated_at is the same, the created_at is newer (i.e., a new session created after the outer row)
					// ^ realistically this feels like it should never happen
					.add(
						Expr::col((inner_alias.clone(), Column::UpdatedAt))
							.eq(Expr::col((Entity, Column::UpdatedAt)))
							.and(
								Expr::col((inner_alias.clone(), Column::CreatedAt))
									.gt(Expr::col((Entity, Column::CreatedAt))),
							),
					)
					// if timestamps are all equal fallback to ids, relying on insert order
					.add(
						Expr::col((inner_alias.clone(), Column::UpdatedAt))
							.eq(Expr::col((Entity, Column::UpdatedAt)))
							.and(
								Expr::col((inner_alias.clone(), Column::CreatedAt))
									.eq(Expr::col((Entity, Column::CreatedAt)))
									.and(
										Expr::col((inner_alias, Column::Id))
											.gt(Expr::col((Entity, Column::Id))),
									),
							),
					),
			)
			.to_owned()
	}

	pub fn find_for_user(user: &AuthUser) -> Select<Entity> {
		Entity::find().filter(Column::UserId.eq(&user.id))
	}

	pub fn find_for_user_and_media(user: &AuthUser, media_id: &str) -> Select<Entity> {
		Entity::find()
			.filter(Column::UserId.eq(&user.id))
			.filter(Column::MediaId.eq(media_id))
	}

	pub fn find_latest_for_user_and_media(
		user: &AuthUser,
		media_id: &str,
	) -> Select<Entity> {
		Entity::find_for_user_and_media(user, media_id)
			.order_by_desc(Column::CreatedAt)
			.limit(1)
	}
}

#[async_trait]
impl ActiveModelBehavior for ActiveModel {
	async fn before_save<C>(mut self, _db: &C, insert: bool) -> Result<Self, DbErr>
	where
		C: ConnectionTrait,
	{
		let now = Utc::now();
		if insert {
			self.created_at = ActiveValue::Set(DateTimeWithTimeZone::from(now));
			self.status = match self.status {
				ActiveValue::Set(s) => ActiveValue::Set(s),
				_ => ActiveValue::Set(ReadingStatus::Reading),
			};
		}
		self.updated_at = ActiveValue::Set(Some(DateTimeWithTimeZone::from(now)));

		Ok(self)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::shared::enums::{DeviceKind, ReadingStatus};
	use chrono::NaiveDate;
	use sea_orm::{
		ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, Database,
		DbBackend, QueryFilter, QueryOrder, Schema, Statement,
	};

	#[tokio::test]
	async fn model_with_device_expands_every_stored_device_id() {
		let db = Database::connect("sqlite::memory:").await.unwrap();
		let schema = Schema::new(DbBackend::Sqlite);
		for sql in [
			"CREATE TABLE users (id TEXT PRIMARY KEY)",
			"CREATE TABLE media (id TEXT PRIMARY KEY)",
		] {
			db.execute(Statement::from_string(DbBackend::Sqlite, sql))
				.await
				.unwrap();
		}
		db.execute(Statement::from_string(
			DbBackend::Sqlite,
			"INSERT INTO users (id) VALUES ('user-1')",
		))
		.await
		.unwrap();
		db.execute(Statement::from_string(
			DbBackend::Sqlite,
			"INSERT INTO media (id) VALUES ('media-1')",
		))
		.await
		.unwrap();
		for statement in [
			schema
				.create_table_from_entity(crate::entity::device::Entity)
				.if_not_exists()
				.to_owned(),
			schema
				.create_table_from_entity(Entity)
				.if_not_exists()
				.to_owned(),
		] {
			db.execute(db.get_database_backend().build(&statement))
				.await
				.unwrap();
		}

		for (id, name) in [("device-a", "Kobo"), ("device-b", "Phone")] {
			crate::entity::device::ActiveModel {
				id: Set(id.to_owned()),
				user_id: Set("user-1".to_owned()),
				name: Set(name.to_owned()),
				kind: Set(DeviceKind::Kobo),
				..Default::default()
			}
			.insert(&db)
			.await
			.unwrap();
		}

		let session = ActiveModel {
			session_date: Set(NaiveDate::from_ymd_opt(2026, 9, 11).unwrap()),
			readthrough_number: Set(1),
			status: Set(ReadingStatus::Reading),
			device_ids: Set(Some(DeviceIds(vec![
				"device-a".to_owned(),
				"device-b".to_owned(),
			]))),
			media_id: Set("media-1".to_owned()),
			user_id: Set("user-1".to_owned()),
			..Default::default()
		}
		.insert(&db)
		.await
		.unwrap();

		let rows = ModelWithDevice::find()
			.filter(Column::Id.eq(session.id))
			.order_by_asc(crate::entity::device::Column::Id)
			.into_model::<ModelWithDevice>()
			.all(&db)
			.await
			.unwrap();

		assert_eq!(rows.len(), 2);
		assert_eq!(
			rows.iter()
				.map(|row| row.device.as_ref().unwrap().id.as_str())
				.collect::<Vec<_>>(),
			vec!["device-a", "device-b"]
		);
	}
}
