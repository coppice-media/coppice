use super::smart_list;
use sea_orm::{prelude::*, Condition};

use crate::entity::user::AuthUser;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[sea_orm(table_name = "smart_list_views")]
#[cfg_attr(feature = "graphql", graphql(name = "SmartListViewModel"))]
pub struct Model {
	#[sea_orm(primary_key)]
	pub id: i32,
	#[sea_orm(column_type = "Text")]
	pub name: String,
	#[sea_orm(column_type = "Text")]
	pub list_id: String,
	#[sea_orm(column_type = "Blob")]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub data: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::smart_list::Entity",
		from = "Column::ListId",
		to = "super::smart_list::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	SmartList,
}

impl Related<super::smart_list::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::SmartList.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}

impl Entity {
	pub fn find_by_list_id(list_id: &String) -> Select<Self> {
		Self::find().filter(Column::ListId.eq(list_id))
	}

	#[cfg(feature = "graphql")]
	pub fn find_by_user_list_id_name(
		user: &AuthUser,
		list_id: &async_graphql::ID,
		name: &String,
	) -> Select<Self> {
		Self::find_by_user(user)
			.filter(Column::ListId.eq(list_id.to_string()))
			.filter(Column::Name.eq(name))
	}

	#[cfg(not(feature = "graphql"))]
	pub fn find_by_user_list_id_name(
		user: &AuthUser,
		list_id: &String,
		name: &String,
	) -> Select<Self> {
		Self::find_by_user(user)
			.filter(Column::ListId.eq(list_id.to_string()))
			.filter(Column::Name.eq(name))
	}

	pub fn find_by_user(user: &AuthUser) -> Select<Self> {
		Self::find()
			.inner_join(smart_list::Entity)
			.filter(Condition::all().add_option(
				super::smart_list::get_access_condition_for_user(user, false, false),
			))
	}
}
