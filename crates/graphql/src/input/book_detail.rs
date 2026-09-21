//! Inputs for explicit book-detail metadata and review operations.
//!
//! Metadata writes carry both the destination scope and the selected fields;
//! omitted fields are never copied as a side effect. Provider search reuses the
//! canonical media search input used by the full metadata editor.

use async_graphql::InputObject;
use metadata_integrations::MetadataField;

use crate::{
	input::media::{MediaMetadataInput, MediaMetadataSearchInput},
	object::book_detail::BookMetadataScope,
};

#[derive(Debug, Clone, InputObject)]
pub struct BookMetadataApplyInput {
	pub scope: BookMetadataScope,
	pub selected_fields: Vec<MetadataField>,
	pub metadata: MediaMetadataInput,
}

#[derive(Debug, Clone, InputObject)]
pub struct BookReviewInput {
	/// Zero means unrated; values above five are rejected.
	pub rating: i32,
	pub content: Option<String>,
	pub is_private: bool,
}

pub type BookMetadataSearchInput = MediaMetadataSearchInput;
