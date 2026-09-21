//! GraphQL representation of one explicit work- or edition-scoped review.

use async_graphql::{SimpleObject, ID};

#[derive(Debug, Clone, SimpleObject)]
pub struct BookReview {
	pub id: ID,
	pub work_id: Option<ID>,
	pub media_id: Option<ID>,
	pub rating: i32,
	pub content: Option<String>,
	pub is_private: bool,
	pub created_at: String,
	pub updated_at: String,
}
