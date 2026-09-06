use crate::{
	data::CoreContext,
	loader::{
		author::{AuthorMediaLoader, MetadataSeriesMediaLoader},
		favorite::FavoritesLoader,
		library::LibraryLoader,
		library_config::LibraryConfigLoader,
		log::JobAssociatedLogLoader,
		media::MediaLoader,
		media_analysis::MediaAnalysisLoader,
		reading_session::ReadingSessionLoader,
		series::SeriesLoader,
		series_count::SeriesCountLoader,
		series_finished_count::SeriesFinishedCountLoader,
	},
	mutation::Mutation,
	query::Query,
	subscription::Subscription,
};
use async_graphql::{dataloader::DataLoader, ObjectType, Schema, SchemaBuilder};
#[cfg(feature = "web")]
use models::shared::enums::AccessRole;
use sea_orm::DatabaseConnection;
use std::sync::Arc;

pub type AppSchema = Schema<Query, Mutation, Subscription>;

pub async fn build_schema(ctx: CoreContext) -> AppSchema {
	let conn = ctx.conn.clone();
	let schema_builder = Schema::build(
		Query::default(),
		Mutation::default(),
		Subscription::default(),
	)
	.limit_depth(15)
	.data(ctx);
	let schema_builder = register_web_only_types(schema_builder);

	add_data_loaders(schema_builder, conn).finish()
}

/// `AccessRole` only ever appears inside serialised SmartList JSON, so
/// async-graphql cannot discover it by walking the resolver graph. It is
/// registered by hand, and only when the smart-list resolvers exist.
fn register_web_only_types<
	QueryType: ObjectType + 'static,
	MutationType: ObjectType + 'static,
	SubscriptionType: 'static,
>(
	schema: SchemaBuilder<QueryType, MutationType, SubscriptionType>,
) -> SchemaBuilder<QueryType, MutationType, SubscriptionType> {
	#[cfg(feature = "web")]
	let schema = schema.register_output_type::<AccessRole>();
	schema
}

pub fn add_data_loaders<
	QueryType: ObjectType + 'static,
	MutationType: ObjectType + 'static,
	SubscriptionType: 'static,
>(
	schema: SchemaBuilder<QueryType, MutationType, SubscriptionType>,
	conn: Arc<DatabaseConnection>,
) -> SchemaBuilder<QueryType, MutationType, SubscriptionType> {
	schema
		.data(DataLoader::new(
			AuthorMediaLoader::new(conn.clone()),
			tokio::spawn,
		))
		.data(DataLoader::new(
			MetadataSeriesMediaLoader::new(conn.clone()),
			tokio::spawn,
		))
		.data(DataLoader::new(
			JobAssociatedLogLoader::new(conn.clone()),
			tokio::spawn,
		))
		.data(DataLoader::new(
			ReadingSessionLoader::new(conn.clone()),
			tokio::spawn,
		))
		.data(DataLoader::new(
			MediaLoader::new(conn.clone()),
			tokio::spawn,
		))
		.data(DataLoader::new(
			LibraryLoader::new(conn.clone()),
			tokio::spawn,
		))
		.data(DataLoader::new(
			LibraryConfigLoader::new(conn.clone()),
			tokio::spawn,
		))
		.data(DataLoader::new(
			SeriesLoader::new(conn.clone()),
			tokio::spawn,
		))
		.data(DataLoader::new(
			SeriesCountLoader::new(conn.clone()),
			tokio::spawn,
		))
		.data(DataLoader::new(
			SeriesFinishedCountLoader::new(conn.clone()),
			tokio::spawn,
		))
		.data(DataLoader::new(
			FavoritesLoader::new(conn.clone()),
			tokio::spawn,
		))
		.data(DataLoader::new(
			MediaAnalysisLoader::new(conn.clone()),
			tokio::spawn,
		))
}

pub fn build_schema_bare() -> AppSchema {
	register_web_only_types(Schema::build(
		Query::default(),
		Mutation::default(),
		Subscription::default(),
	))
	.finish()
}
