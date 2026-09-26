pub(crate) mod annotation;
mod annotation_attachment;
mod api_key;
mod audiobook_narrators;
mod author;
#[cfg(feature = "web")]
mod book_club;
#[cfg(feature = "web")]
mod book_club_book;
#[cfg(feature = "web")]
mod book_club_discussion;
#[cfg(feature = "web")]
mod book_club_invitation;
#[cfg(feature = "web")]
mod book_club_suggestion;
mod book_detail;
mod book_request;
mod config;
#[cfg(feature = "crosspoint")]
pub(crate) mod crosspoint;
#[cfg(feature = "web")]
mod custom_emoji;
mod device;
mod device_pairing;
pub(crate) mod duplicate_page;
mod edition_pair;
mod email_device;
mod emailer;
mod epub;
mod filesystem;
mod hardcover;
mod ingest;
mod job;
mod kindle;
mod library;
mod log;
#[cfg(feature = "mam-acquisition")]
mod mam_acquisition;
pub(crate) mod media;
mod media_metadata_overview;
mod metadata_provider;
mod notification;
mod notifier;
#[cfg(feature = "providers")]
mod provider;
pub(crate) mod reading_list;
mod reading_stats;
mod runtime_component;
mod series;
mod server_config;
#[cfg(feature = "web")]
mod smart_list_view;
#[cfg(feature = "web")]
mod smart_lists;
#[cfg(feature = "web")]
pub(crate) mod smart_lists_builder;
mod tag;
mod unified_search;
pub(crate) mod user;
mod worker;

use crate::social::SocialQuery;
use annotation::AnnotationQuery;
use annotation_attachment::AnnotationAttachmentQuery;
use api_key::APIKeyQuery;
use audiobook_narrators::AudiobookNarratorsQuery;
use author::AuthorQuery;
#[cfg(feature = "web")]
use book_club::BookClubQuery;
#[cfg(feature = "web")]
use book_club_book::BookClubBookQuery;
#[cfg(feature = "web")]
use book_club_discussion::BookClubDiscussionQuery;
#[cfg(feature = "web")]
use book_club_invitation::BookClubInvitationQuery;
#[cfg(feature = "web")]
use book_club_suggestion::BookClubSuggestionQuery;
use book_detail::BookDetailQuery;
use book_request::BookRequestQuery;
use config::ConfigQuery;
#[cfg(feature = "crosspoint")]
use crosspoint::CrosspointQuery;
#[cfg(feature = "web")]
use custom_emoji::CustomEmojiQuery;
use device::DeviceQuery;
use device_pairing::DevicePairingQuery;
use duplicate_page::DuplicatePageQuery;
use edition_pair::EditionPairQuery;
use email_device::EmailDeviceQuery;
use emailer::EmailerQuery;
use epub::EpubQuery;
use filesystem::FilesystemQuery;
use hardcover::HardcoverQuery;
use ingest::IngestQuery;
use kindle::KindleQuery;
use library::LibraryQuery;
use log::LogQuery;
#[cfg(feature = "mam-acquisition")]
use mam_acquisition::MamAcquisitionQuery;
use media::MediaQuery;
use media_metadata_overview::MediaMetadataOverviewQuery;
use metadata_provider::MetadataProviderQuery;
use notification::NotificationQuery;
use notifier::NotifierQuery;
#[cfg(feature = "providers")]
use provider::ProviderQuery;
use reading_list::ReadingListQuery;
use reading_stats::ReadingStatsQuery;
use runtime_component::RuntimeComponentQuery;
use series::SeriesQuery;
use server_config::ServerConfigQuery;
#[cfg(feature = "web")]
use smart_list_view::SmartListViewQuery;
#[cfg(feature = "web")]
use smart_lists::SmartListsQuery;
use tag::TagQuery;
use unified_search::UnifiedSearchQuery;
use user::UserQuery;
use worker::WorkerQuery;

use crate::query::job::JobQuery;

// Note: I had to split the Query/Mutation root types into chunks to avoid a compiler
// overflow. It seems like a flat MergedObject creates a really large async block
// that is too deep for the compiler.

#[cfg(feature = "web")]
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
	EditionPairQuery,
	BookDetailQuery,
	UnifiedSearchQuery,
	AudiobookNarratorsQuery,
);
#[derive(async_graphql::MergedObject, Default)]
struct UserAndNotifsQueries(
	UserQuery,
	EmailerQuery,
	EmailDeviceQuery,
	NotifierQuery,
	SocialQuery,
	NotificationQuery,
	HardcoverQuery,
);
#[derive(async_graphql::MergedObject, Default)]
struct SystemQueries(
	APIKeyQuery,
	BookRequestQuery,
	#[cfg(feature = "mam-acquisition")] MamAcquisitionQuery,
	JobQuery,
	LogQuery,
	ConfigQuery,
	MetadataProviderQuery,
	ServerConfigQuery,
	FilesystemQuery,
	IngestQuery,
	DevicePairingQuery,
	WorkerQuery,
	RuntimeComponentQuery,
	#[cfg(feature = "providers")] ProviderQuery,
);

#[derive(async_graphql::MergedObject, Default)]
struct ListQueries(
	#[cfg(feature = "web")] SmartListsQuery,
	#[cfg(feature = "web")] SmartListViewQuery,
	ReadingListQuery,
	#[cfg(feature = "web")] CustomEmojiQuery,
);

#[derive(async_graphql::MergedObject, Default)]
struct DeviceQueries(
	DeviceQuery,
	#[cfg(feature = "crosspoint")] CrosspointQuery,
	KindleQuery,
	ReadingStatsQuery,
	AnnotationQuery,
	AnnotationAttachmentQuery,
);

#[derive(async_graphql::MergedObject, Default)]
pub struct Query(
	#[cfg(feature = "web")] BookClubQueries,
	ContentQueries,
	UserAndNotifsQueries,
	SystemQueries,
	ListQueries,
	DeviceQueries,
);
