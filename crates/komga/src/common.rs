use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KomgaAuthor {
	pub name: String,
	pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KomgaWebLink {
	pub label: String,
	pub url: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KomgaReadingDirection {
	LeftToRight,
	RightToLeft,
	Vertical,
	Webtoon,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct KomgaThumbnailId(pub String);

impl KomgaThumbnailId {
	pub fn new(value: impl Into<String>) -> Self {
		Self(value.into())
	}
}

impl fmt::Display for KomgaThumbnailId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.0.fmt(f)
	}
}

impl From<String> for KomgaThumbnailId {
	fn from(value: String) -> Self {
		Self(value)
	}
}

impl From<&str> for KomgaThumbnailId {
	fn from(value: &str) -> Self {
		Self(value.to_owned())
	}
}

/// A patch field with the three states used by Komga's PATCH endpoints.
///
/// `Unset` is omitted by request DTOs, `None` is encoded as JSON `null`, and
/// `Some` is encoded as the contained value.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum PatchValue<T> {
	#[default]
	Unset,
	None,
	Some(T),
}

impl<T> PatchValue<T> {
	pub fn is_unset(&self) -> bool {
		matches!(self, Self::Unset)
	}

	pub fn is_none(&self) -> bool {
		matches!(self, Self::None)
	}

	pub fn some(value: T) -> Self {
		Self::Some(value)
	}

	pub fn none() -> Self {
		Self::None
	}
}

impl<T: Serialize> Serialize for PatchValue<T> {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		match self {
			Self::Unset => Err(serde::ser::Error::custom(
				"an unset patch value must be skipped by its containing request",
			)),
			Self::None => serializer.serialize_none(),
			Self::Some(value) => value.serialize(serializer),
		}
	}
}

impl<'de, T> Deserialize<'de> for PatchValue<T>
where
	T: DeserializeOwned,
{
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		Option::<T>::deserialize(deserializer).map(|value| match value {
			Some(value) => Self::Some(value),
			None => Self::None,
		})
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
	pub content: Vec<T>,
	pub pageable: Pageable,
	pub total_elements: i32,
	pub total_pages: i32,
	pub last: bool,
	pub number: i32,
	pub sort: Sort,
	pub first: bool,
	pub number_of_elements: i32,
	pub size: i32,
	pub empty: bool,
}

impl<T> Page<T> {
	/// Construct the Spring-style page envelope used by Komga list endpoints.
	pub fn new(content: Vec<T>, page: i32, size: i32, total: i32, unpaged: bool) -> Self {
		let paged = !unpaged;
		let (page_number, page_size, offset, total_pages) = if unpaged {
			let page_size = content.len() as i32;
			let total_pages = if total > 0 { 1 } else { 0 };
			(0, page_size, 0, total_pages)
		} else {
			let page_number = page.max(0);
			let page_size = size.max(0);
			let total_pages = if total <= 0 {
				0
			} else if page_size == 0 {
				1
			} else {
				(total + page_size - 1) / page_size
			};
			(
				page_number,
				page_size,
				page_number.saturating_mul(page_size),
				total_pages,
			)
		};
		let sort = Sort::default();
		let pageable = Pageable {
			sort: sort.clone(),
			page_number,
			page_size,
			offset,
			paged,
			unpaged,
		};
		let number_of_elements = content.len() as i32;
		let empty = content.is_empty();
		let first = page_number == 0;
		let last = total_pages == 0 || page_number.saturating_add(1) >= total_pages;

		Self {
			content,
			pageable,
			total_elements: total,
			total_pages,
			last,
			number: page_number,
			sort,
			first,
			number_of_elements,
			size: page_size,
			empty,
		}
	}

	pub fn empty() -> Self {
		Self {
			content: Vec::new(),
			pageable: Pageable::unpaged(),
			total_elements: 0,
			total_pages: 0,
			last: true,
			number: 0,
			sort: Sort::default(),
			first: true,
			number_of_elements: 0,
			size: 0,
			empty: true,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Pageable {
	pub sort: Sort,
	pub page_number: i32,
	pub page_size: i32,
	pub offset: i32,
	pub paged: bool,
	pub unpaged: bool,
}

impl Pageable {
	pub fn unpaged() -> Self {
		Self {
			sort: Sort::default(),
			page_number: 0,
			page_size: 0,
			offset: 0,
			paged: false,
			unpaged: true,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sort {
	pub sorted: bool,
	pub unsorted: bool,
	pub empty: bool,
}

impl Default for Sort {
	fn default() -> Self {
		Self {
			sorted: false,
			unsorted: true,
			empty: true,
		}
	}
}

impl Sort {
	pub fn sorted() -> Self {
		Self {
			sorted: true,
			unsorted: false,
			empty: false,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{Page, Pageable, Sort};
	use serde_json::json;

	#[test]
	fn spring_page_serializes_every_field_in_camel_case() {
		let page = Page::new(vec!["book"], 0, 2, 3, false);
		let value = serde_json::to_value(page).expect("page serializes");
		assert_eq!(
			value,
			json!({
				"content": ["book"],
				"pageable": {
					"sort": {"sorted": false, "unsorted": true, "empty": true},
					"pageNumber": 0,
					"pageSize": 2,
					"offset": 0,
					"paged": true,
					"unpaged": false
				},
				"totalElements": 3,
				"totalPages": 2,
				"last": false,
				"number": 0,
				"sort": {"sorted": false, "unsorted": true, "empty": true},
				"first": true,
				"numberOfElements": 1,
				"size": 2,
				"empty": false
			})
		);
	}

	#[test]
	fn empty_page_matches_spring_empty_shape() {
		let page: Page<String> = Page::empty();
		assert_eq!(page.pageable, Pageable::unpaged());
		assert_eq!(page.sort, Sort::default());
		assert!(page.empty && page.first && page.last);
	}
}
