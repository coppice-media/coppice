use crate::book::KomgaBookId;
use crate::collection::KomgaCollectionId;
use crate::library::KomgaLibraryId;
use crate::read_list::KomgaReadListId;
use crate::series::KomgaSeriesId;
use crate::user::KomgaUserId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryAdded {
	pub library_id: KomgaLibraryId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryChanged {
	pub library_id: KomgaLibraryId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryDeleted {
	pub library_id: KomgaLibraryId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesAdded {
	pub series_id: KomgaSeriesId,
	pub library_id: KomgaLibraryId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesChanged {
	pub series_id: KomgaSeriesId,
	pub library_id: KomgaLibraryId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesDeleted {
	pub series_id: KomgaSeriesId,
	pub library_id: KomgaLibraryId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookAdded {
	pub book_id: KomgaBookId,
	pub series_id: KomgaSeriesId,
	pub library_id: KomgaLibraryId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookChanged {
	pub book_id: KomgaBookId,
	pub series_id: KomgaSeriesId,
	pub library_id: KomgaLibraryId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookDeleted {
	pub book_id: KomgaBookId,
	pub series_id: KomgaSeriesId,
	pub library_id: KomgaLibraryId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookImported {
	pub book_id: Option<KomgaBookId>,
	pub source_file: String,
	pub success: bool,
	pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadListAdded {
	pub read_list_id: KomgaReadListId,
	pub book_ids: Vec<KomgaBookId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadListChanged {
	pub read_list_id: KomgaReadListId,
	pub book_ids: Vec<KomgaBookId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadListDeleted {
	pub read_list_id: KomgaReadListId,
	pub book_ids: Vec<KomgaBookId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionAdded {
	pub collection_id: KomgaCollectionId,
	pub series_ids: Vec<KomgaSeriesId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionChanged {
	pub collection_id: KomgaCollectionId,
	pub series_ids: Vec<KomgaSeriesId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionDeleted {
	pub collection_id: KomgaCollectionId,
	pub series_ids: Vec<KomgaSeriesId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadProgressChanged {
	pub book_id: KomgaBookId,
	pub user_id: KomgaUserId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadProgressDeleted {
	pub book_id: KomgaBookId,
	pub user_id: KomgaUserId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadProgressSeriesChanged {
	pub series_id: KomgaSeriesId,
	pub user_id: KomgaUserId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadProgressSeriesDeleted {
	pub series_id: KomgaSeriesId,
	pub user_id: KomgaUserId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailBookAdded {
	pub book_id: KomgaBookId,
	pub series_id: KomgaSeriesId,
	pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailBookDeleted {
	pub book_id: KomgaBookId,
	pub series_id: KomgaSeriesId,
	pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailSeriesAdded {
	pub series_id: KomgaSeriesId,
	pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailSeriesDeleted {
	pub series_id: KomgaSeriesId,
	pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailSeriesCollectionAdded {
	pub collection_id: KomgaCollectionId,
	pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailSeriesCollectionDeleted {
	pub collection_id: KomgaCollectionId,
	pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailReadListAdded {
	pub read_list_id: KomgaReadListId,
	pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailReadListDeleted {
	pub read_list_id: KomgaReadListId,
	pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionExpired {
	pub user_id: KomgaUserId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskQueueStatus {
	pub count: i32,
	pub count_by_type: BTreeMap<String, i32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnknownEvent {
	pub event: Option<String>,
	pub data: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum KomgaEvent {
	LibraryAdded {
		library_id: KomgaLibraryId,
	},
	LibraryChanged {
		library_id: KomgaLibraryId,
	},
	LibraryDeleted {
		library_id: KomgaLibraryId,
	},
	SeriesAdded {
		series_id: KomgaSeriesId,
		library_id: KomgaLibraryId,
	},
	SeriesChanged {
		series_id: KomgaSeriesId,
		library_id: KomgaLibraryId,
	},
	SeriesDeleted {
		series_id: KomgaSeriesId,
		library_id: KomgaLibraryId,
	},
	BookAdded {
		book_id: KomgaBookId,
		series_id: KomgaSeriesId,
		library_id: KomgaLibraryId,
	},
	BookChanged {
		book_id: KomgaBookId,
		series_id: KomgaSeriesId,
		library_id: KomgaLibraryId,
	},
	BookDeleted {
		book_id: KomgaBookId,
		series_id: KomgaSeriesId,
		library_id: KomgaLibraryId,
	},
	BookImported {
		book_id: Option<KomgaBookId>,
		source_file: String,
		success: bool,
		message: Option<String>,
	},
	ReadListAdded {
		read_list_id: KomgaReadListId,
		book_ids: Vec<KomgaBookId>,
	},
	ReadListChanged {
		read_list_id: KomgaReadListId,
		book_ids: Vec<KomgaBookId>,
	},
	ReadListDeleted {
		read_list_id: KomgaReadListId,
		book_ids: Vec<KomgaBookId>,
	},
	CollectionAdded {
		collection_id: KomgaCollectionId,
		series_ids: Vec<KomgaSeriesId>,
	},
	CollectionChanged {
		collection_id: KomgaCollectionId,
		series_ids: Vec<KomgaSeriesId>,
	},
	CollectionDeleted {
		collection_id: KomgaCollectionId,
		series_ids: Vec<KomgaSeriesId>,
	},
	ReadProgressChanged {
		book_id: KomgaBookId,
		user_id: KomgaUserId,
	},
	ReadProgressDeleted {
		book_id: KomgaBookId,
		user_id: KomgaUserId,
	},
	ReadProgressSeriesChanged {
		series_id: KomgaSeriesId,
		user_id: KomgaUserId,
	},
	ReadProgressSeriesDeleted {
		series_id: KomgaSeriesId,
		user_id: KomgaUserId,
	},
	ThumbnailBookAdded {
		book_id: KomgaBookId,
		series_id: KomgaSeriesId,
		selected: bool,
	},
	ThumbnailBookDeleted {
		book_id: KomgaBookId,
		series_id: KomgaSeriesId,
		selected: bool,
	},
	ThumbnailSeriesAdded {
		series_id: KomgaSeriesId,
		selected: bool,
	},
	ThumbnailSeriesDeleted {
		series_id: KomgaSeriesId,
		selected: bool,
	},
	ThumbnailSeriesCollectionAdded {
		collection_id: KomgaCollectionId,
		selected: bool,
	},
	ThumbnailSeriesCollectionDeleted {
		collection_id: KomgaCollectionId,
		selected: bool,
	},
	ThumbnailReadListAdded {
		read_list_id: KomgaReadListId,
		selected: bool,
	},
	ThumbnailReadListDeleted {
		read_list_id: KomgaReadListId,
		selected: bool,
	},
	SessionExpired {
		user_id: KomgaUserId,
	},
	TaskQueueStatus {
		count: i32,
		count_by_type: BTreeMap<String, i32>,
	},
	UnknownEvent {
		#[serde(default)]
		event: Option<String>,
		#[serde(default)]
		data: Option<String>,
	},
}

impl KomgaEvent {
	pub fn event_name(&self) -> Option<&'static str> {
		Some(match self {
			Self::LibraryAdded { .. } => "LibraryAdded",
			Self::LibraryChanged { .. } => "LibraryChanged",
			Self::LibraryDeleted { .. } => "LibraryDeleted",
			Self::SeriesAdded { .. } => "SeriesAdded",
			Self::SeriesChanged { .. } => "SeriesChanged",
			Self::SeriesDeleted { .. } => "SeriesDeleted",
			Self::BookAdded { .. } => "BookAdded",
			Self::BookChanged { .. } => "BookChanged",
			Self::BookDeleted { .. } => "BookDeleted",
			Self::BookImported { .. } => "BookImported",
			Self::ReadListAdded { .. } => "ReadListAdded",
			Self::ReadListChanged { .. } => "ReadListChanged",
			Self::ReadListDeleted { .. } => "ReadListDeleted",
			Self::CollectionAdded { .. } => "CollectionAdded",
			Self::CollectionChanged { .. } => "CollectionChanged",
			Self::CollectionDeleted { .. } => "CollectionDeleted",
			Self::ReadProgressChanged { .. } => "ReadProgressChanged",
			Self::ReadProgressDeleted { .. } => "ReadProgressDeleted",
			Self::ReadProgressSeriesChanged { .. } => "ReadProgressSeriesChanged",
			Self::ReadProgressSeriesDeleted { .. } => "ReadProgressSeriesDeleted",
			Self::ThumbnailBookAdded { .. } => "ThumbnailBookAdded",
			Self::ThumbnailBookDeleted { .. } => "ThumbnailBookDeleted",
			Self::ThumbnailSeriesAdded { .. } => "ThumbnailSeriesAdded",
			Self::ThumbnailSeriesDeleted { .. } => "ThumbnailSeriesDeleted",
			Self::ThumbnailSeriesCollectionAdded { .. } => {
				"ThumbnailSeriesCollectionAdded"
			},
			Self::ThumbnailSeriesCollectionDeleted { .. } => {
				"ThumbnailSeriesCollectionDeleted"
			},
			Self::ThumbnailReadListAdded { .. } => "ThumbnailReadListAdded",
			Self::ThumbnailReadListDeleted { .. } => "ThumbnailReadListDeleted",
			Self::SessionExpired { .. } => "SessionExpired",
			Self::TaskQueueStatus { .. } => "TaskQueueStatus",
			Self::UnknownEvent { .. } => return None,
		})
	}
}

macro_rules! decode {
    ($data:expr, $ty:ty, $variant:ident, {$($field:ident),+ $(,)?}) => {
        serde_json::from_str::<$ty>($data).map(|payload| KomgaEvent::$variant {
            $($field: payload.$field),+
        })
    };
}

/// Decode an SSE event using the event-name-to-payload mapping used by
/// komga-client. The event name is carried by SSE, not by the JSON payload.
pub fn decode_event(
	event: Option<&str>,
	data: Option<&str>,
) -> Result<KomgaEvent, serde_json::Error> {
	let Some(data) = data else {
		return Ok(KomgaEvent::UnknownEvent {
			event: event.map(str::to_owned),
			data: None,
		});
	};

	match event {
		Some("LibraryAdded") => decode!(data, LibraryAdded, LibraryAdded, { library_id }),
		Some("LibraryChanged") => {
			decode!(data, LibraryChanged, LibraryChanged, { library_id })
		},
		Some("LibraryDeleted") => {
			decode!(data, LibraryDeleted, LibraryDeleted, { library_id })
		},
		Some("SeriesAdded") => {
			decode!(data, SeriesAdded, SeriesAdded, { series_id, library_id })
		},
		Some("SeriesChanged") => {
			decode!(data, SeriesChanged, SeriesChanged, { series_id, library_id })
		},
		Some("SeriesDeleted") => {
			decode!(data, SeriesDeleted, SeriesDeleted, { series_id, library_id })
		},
		Some("BookAdded") => {
			decode!(data, BookAdded, BookAdded, { book_id, series_id, library_id })
		},
		Some("BookChanged") => {
			decode!(data, BookChanged, BookChanged, { book_id, series_id, library_id })
		},
		Some("BookDeleted") => {
			decode!(data, BookDeleted, BookDeleted, { book_id, series_id, library_id })
		},
		Some("BookImported") => decode!(data, BookImported, BookImported, {
			book_id,
			source_file,
			success,
			message
		}),
		Some("ReadListAdded") => {
			decode!(data, ReadListAdded, ReadListAdded, { read_list_id, book_ids })
		},
		Some("ReadListChanged") => {
			decode!(data, ReadListChanged, ReadListChanged, { read_list_id, book_ids })
		},
		Some("ReadListDeleted") => {
			decode!(data, ReadListDeleted, ReadListDeleted, { read_list_id, book_ids })
		},
		Some("CollectionAdded") => {
			decode!(data, CollectionAdded, CollectionAdded, { collection_id, series_ids })
		},
		Some("CollectionChanged") => {
			decode!(data, CollectionChanged, CollectionChanged, { collection_id, series_ids })
		},
		Some("CollectionDeleted") => {
			decode!(data, CollectionDeleted, CollectionDeleted, { collection_id, series_ids })
		},
		Some("ReadProgressChanged") => {
			decode!(data, ReadProgressChanged, ReadProgressChanged, { book_id, user_id })
		},
		Some("ReadProgressDeleted") => {
			decode!(data, ReadProgressDeleted, ReadProgressDeleted, { book_id, user_id })
		},
		Some("ReadProgressSeriesChanged") => {
			decode!(data, ReadProgressSeriesChanged, ReadProgressSeriesChanged, { series_id, user_id })
		},
		Some("ReadProgressSeriesDeleted") => {
			decode!(data, ReadProgressSeriesDeleted, ReadProgressSeriesDeleted, { series_id, user_id })
		},
		Some("ThumbnailBookAdded") => {
			decode!(data, ThumbnailBookAdded, ThumbnailBookAdded, { book_id, series_id, selected })
		},
		Some("ThumbnailBookDeleted") => {
			decode!(data, ThumbnailBookDeleted, ThumbnailBookDeleted, { book_id, series_id, selected })
		},
		Some("ThumbnailSeriesAdded") => {
			decode!(data, ThumbnailSeriesAdded, ThumbnailSeriesAdded, { series_id, selected })
		},
		Some("ThumbnailSeriesDeleted") => {
			decode!(data, ThumbnailSeriesDeleted, ThumbnailSeriesDeleted, { series_id, selected })
		},
		Some("ThumbnailSeriesCollectionAdded") => {
			decode!(data, ThumbnailSeriesCollectionAdded, ThumbnailSeriesCollectionAdded, { collection_id, selected })
		},
		Some("ThumbnailSeriesCollectionDeleted") => {
			decode!(data, ThumbnailSeriesCollectionDeleted, ThumbnailSeriesCollectionDeleted, { collection_id, selected })
		},
		Some("ThumbnailReadListAdded") => {
			decode!(data, ThumbnailReadListAdded, ThumbnailReadListAdded, { read_list_id, selected })
		},
		Some("ThumbnailReadListDeleted") => {
			decode!(data, ThumbnailReadListDeleted, ThumbnailReadListDeleted, { read_list_id, selected })
		},
		Some("SessionExpired") => {
			decode!(data, SessionExpired, SessionExpired, { user_id })
		},
		Some("TaskQueueStatus") => {
			decode!(data, TaskQueueStatus, TaskQueueStatus, { count, count_by_type })
		},
		_ => Ok(KomgaEvent::UnknownEvent {
			event: event.map(str::to_owned),
			data: Some(data.to_owned()),
		}),
	}
}
