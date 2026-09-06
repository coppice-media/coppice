mod annotation;
mod api_key;
#[cfg(feature = "web")]
mod book_club;
#[cfg(feature = "web")]
mod book_club_book;
#[cfg(feature = "web")]
mod book_club_discussion;
#[cfg(feature = "web")]
mod book_club_invitation;
#[cfg(feature = "web")]
mod book_club_member;
#[cfg(feature = "web")]
mod book_club_suggestion;
#[cfg(feature = "web")]
mod custom_emoji;
mod device;
mod device_pairing;
mod duplicate_page;
mod email_device;
mod emailer;
mod epub;
mod ingest;
mod job;
mod kindle;
mod library;
mod log;
mod media;
mod media_metadata;
mod metadata_provider;
mod notification;
mod notifier;
#[cfg(feature = "providers")]
mod provider;
mod reading_list;
pub mod reading_progress;
mod scheduled_job_config;
mod series;
mod series_metadata;
mod server_config;
#[cfg(feature = "web")]
mod smart_list_view;
#[cfg(feature = "web")]
mod smart_lists;
mod tag;
#[cfg(feature = "web")]
mod upload;
mod user;

use ingest::IngestMutation;

use annotation::AnnotationMutation;
use api_key::APIKeyMutation;
#[cfg(feature = "web")]
use book_club::BookClubMutation;
#[cfg(feature = "web")]
use book_club_book::BookClubBookMutation;
#[cfg(feature = "web")]
use book_club_discussion::BookClubDiscussionMutation;
#[cfg(feature = "web")]
use book_club_invitation::BookClubInvitationMutation;
#[cfg(feature = "web")]
use book_club_member::BookClubMemberMutation;
#[cfg(feature = "web")]
use book_club_suggestion::BookClubSuggestionMutation;
#[cfg(feature = "web")]
use custom_emoji::CustomEmojiMutation;
use device::DeviceMutation;
use device_pairing::DevicePairingMutation;
use duplicate_page::DuplicatePageMutation;
use email_device::EmailDeviceMutation;
use emailer::EmailerMutation;
use epub::EpubMutation;
use job::JobMutation;
use kindle::KindleMutation;
use library::LibraryMutation;
use log::LogMutation;
use media::MediaMutation;
use media_metadata::MediaMetadataMutation;
use metadata_provider::MetadataProviderMutation;
use notification::NotificationMutation;
use notifier::NotifierMutation;
#[cfg(feature = "providers")]
use provider::ProviderMutation;
use reading_list::ReadingListMutation;
use reading_progress::ReadProgressMutation;
use scheduled_job_config::ScheduledJobConfigMutation;
use series::SeriesMutation;
use series_metadata::SeriesMetadataMutation;
use server_config::ServerConfigMutation;
#[cfg(feature = "web")]
use smart_list_view::SmartListViewMutation;
#[cfg(feature = "web")]
use smart_lists::SmartListMutation;
use tag::TagMutation;
#[cfg(feature = "web")]
use upload::UploadMutation;
use user::UserMutation;

#[cfg(feature = "web")]
#[derive(async_graphql::MergedObject, Default)]
struct BookClubMutations(
	BookClubMutation,
	BookClubDiscussionMutation,
	BookClubInvitationMutation,
	BookClubMemberMutation,
	BookClubBookMutation,
	BookClubSuggestionMutation,
);

#[derive(async_graphql::MergedObject, Default)]
struct ContentMutations(
	MediaMutation,
	MediaMetadataMutation,
	SeriesMetadataMutation,
	LibraryMutation,
	SeriesMutation,
	EpubMutation,
	TagMutation,
	#[cfg(feature = "web")] UploadMutation,
	DuplicatePageMutation,
);

#[derive(async_graphql::MergedObject, Default)]
struct UserAndNotifsMutations(
	UserMutation,
	EmailerMutation,
	EmailDeviceMutation,
	KindleMutation,
	NotificationMutation,
);

#[derive(async_graphql::MergedObject, Default)]
struct SystemMutations(
	APIKeyMutation,
	JobMutation,
	LogMutation,
	NotifierMutation,
	ServerConfigMutation,
	ScheduledJobConfigMutation,
	MetadataProviderMutation,
	#[cfg(feature = "providers")] ProviderMutation,
	IngestMutation,
	DevicePairingMutation,
);

#[derive(async_graphql::MergedObject, Default)]
struct ListMutations(
	#[cfg(feature = "web")] SmartListMutation,
	#[cfg(feature = "web")] SmartListViewMutation,
	ReadingListMutation,
	#[cfg(feature = "web")] CustomEmojiMutation,
);

#[derive(async_graphql::MergedObject, Default)]
pub struct Mutation(
	#[cfg(feature = "web")] BookClubMutations,
	ContentMutations,
	UserAndNotifsMutations,
	SystemMutations,
	ListMutations,
	ReadProgressMutation,
	DeviceMutation,
	AnnotationMutation,
);
