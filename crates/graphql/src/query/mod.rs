pub(crate) mod annotation;
mod annotation_attachment;
mod api_key;
mod author;
mod book_club;
mod book_club_book;
mod book_club_discussion;
mod book_club_invitation;
mod book_club_suggestion;
mod config;
mod custom_emoji;
mod device;
mod device_pairing;
pub(crate) mod duplicate_page;
mod email_device;
mod emailer;
mod epub;
mod filesystem;
mod ingest;
mod job;
mod library;
mod log;
pub(crate) mod media;
mod media_metadata_overview;
mod metadata_provider;
mod notification;
mod notifier;
#[cfg(feature = "providers")]
mod provider;
pub(crate) mod reading_list;
mod reading_stats;
mod series;
mod server_config;
mod smart_list_view;
mod smart_lists;
pub(crate) mod smart_lists_builder;
mod tag;
pub(crate) mod user;

use annotation::AnnotationQuery;
use annotation_attachment::AnnotationAttachmentQuery;
use api_key::APIKeyQuery;
use author::AuthorQuery;
use book_club::BookClubQuery;
use book_club_book::BookClubBookQuery;
use book_club_discussion::BookClubDiscussionQuery;
use book_club_invitation::BookClubInvitationQuery;
use book_club_suggestion::BookClubSuggestionQuery;
use config::ConfigQuery;
use custom_emoji::CustomEmojiQuery;
use device::DeviceQuery;
use device_pairing::DevicePairingQuery;
use duplicate_page::DuplicatePageQuery;
use email_device::EmailDeviceQuery;
use emailer::EmailerQuery;
use epub::EpubQuery;
use filesystem::FilesystemQuery;
use ingest::IngestQuery;
use library::LibraryQuery;
use log::LogQuery;
use media::MediaQuery;
use media_metadata_overview::MediaMetadataOverviewQuery;
use metadata_provider::MetadataProviderQuery;
use notification::NotificationQuery;
use notifier::NotifierQuery;
#[cfg(feature = "providers")]
use provider::ProviderQuery;
use reading_list::ReadingListQuery;
use reading_stats::ReadingStatsQuery;
use series::SeriesQuery;
use server_config::ServerConfigQuery;
use smart_list_view::SmartListViewQuery;
use smart_lists::SmartListsQuery;
use tag::TagQuery;
use user::UserQuery;

use crate::query::job::JobQuery;

// Note: I had to split the Query/Mutation root types into chunks to avoid a compiler
// overflow. It seems like a flat MergedObject creates a really large async block
// that is too deep for the compiler.

#[derive(async_graphql::MergedObject, Default)]
struct BookClubQueries(
	BookClubQuery,
	BookClubBookQuery,
	BookClubDiscussionQuery,
	BookClubInvitationQuery,
	BookClubSuggestionQuery,
);

#[derive(async_graphql::MergedObject, Default)]
struct ContentQueries(
	AuthorQuery,
	MediaQuery,
	LibraryQuery,
	SeriesQuery,
	EpubQuery,
	TagQuery,
	MediaMetadataOverviewQuery,
	DuplicatePageQuery,
);

#[derive(async_graphql::MergedObject, Default)]
struct UserAndNotifsQueries(
	UserQuery,
	EmailerQuery,
	EmailDeviceQuery,
	NotifierQuery,
	NotificationQuery,
);

#[derive(async_graphql::MergedObject, Default)]
struct SystemQueries(
	APIKeyQuery,
	JobQuery,
	LogQuery,
	ConfigQuery,
	MetadataProviderQuery,
	ServerConfigQuery,
	FilesystemQuery,
	IngestQuery,
	DevicePairingQuery,
	#[cfg(feature = "providers")] ProviderQuery,
);

#[derive(async_graphql::MergedObject, Default)]
struct ListQueries(
	SmartListsQuery,
	SmartListViewQuery,
	ReadingListQuery,
	CustomEmojiQuery,
);

#[derive(async_graphql::MergedObject, Default)]
struct DeviceQueries(
	DeviceQuery,
	ReadingStatsQuery,
	AnnotationQuery,
	AnnotationAttachmentQuery,
);

#[derive(async_graphql::MergedObject, Default)]
pub struct Query(
	BookClubQueries,
	ContentQueries,
	UserAndNotifsQueries,
	SystemQueries,
	ListQueries,
	DeviceQueries,
);
