//! Hand-built export fixtures shared by the sink tests.

use chrono::{DateTime, TimeZone, Utc};
use serde_json::json;

use crate::model::{
	AnnotationOrigin, BookSource, ExportAnnotation, ExportAnnotationKind, ExportBatch,
	ExportBook, ExportBookmark, ExportIdentifier, ReadingSummary,
};

pub(crate) fn at(secs: i64) -> DateTime<Utc> {
	Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
}

/// A native book with one liseur work folded in: a native highlight with a
/// note, a liseur highlight, a liseur note, a liseur bookmark, a tombstone,
/// a native bookmark, and a reading summary.
pub(crate) fn fixture_book() -> ExportBook {
	ExportBook {
		key: "native:m1".to_string(),
		source: BookSource::Native {
			media_id: "m1".to_string(),
		},
		title: "Dune \"Messiah\"".to_string(),
		authors: vec!["Frank Herbert".to_string()],
		identifiers: vec![ExportIdentifier {
			scheme: "isbn".to_string(),
			value: "9780441172696".to_string(),
		}],
		annotations: vec![
			ExportAnnotation {
				id: "a1".to_string(),
				kind: ExportAnnotationKind::Highlight,
				origin: AnnotationOrigin::Native,
				locator: Some(
					json!({"href": "ch1.xhtml", "locations": {"progression": 0.25}}),
				),
				progression: Some(0.25),
				color: None,
				excerpt: Some("The spice must flow.".to_string()),
				note: Some("Opening line".to_string()),
				created_at: Some(at(10)),
				updated_at: Some(at(10)),
				deleted: false,
				deleted_at: None,
			},
			ExportAnnotation {
				id: "l1".to_string(),
				kind: ExportAnnotationKind::Highlight,
				origin: AnnotationOrigin::Liseur { rev: 1, seq: 4 },
				locator: Some(json!({"cfi": "/6/4!/4/2"})),
				progression: Some(0.5),
				color: Some("yellow".to_string()),
				excerpt: Some(
					"Fear is the mind-killer.\nI will face my fear.".to_string(),
				),
				note: None,
				created_at: Some(at(20)),
				updated_at: Some(at(20)),
				deleted: false,
				deleted_at: None,
			},
			ExportAnnotation {
				id: "l2".to_string(),
				kind: ExportAnnotationKind::Note,
				origin: AnnotationOrigin::Liseur { rev: 1, seq: 5 },
				locator: None,
				progression: None,
				color: None,
				excerpt: None,
				note: Some("Re-read chapter two".to_string()),
				created_at: Some(at(30)),
				updated_at: Some(at(30)),
				deleted: false,
				deleted_at: None,
			},
			ExportAnnotation {
				id: "l3".to_string(),
				kind: ExportAnnotationKind::Highlight,
				origin: AnnotationOrigin::Liseur { rev: 2, seq: 7 },
				locator: None,
				progression: None,
				color: None,
				excerpt: Some("deleted text".to_string()),
				note: None,
				created_at: Some(at(40)),
				updated_at: Some(at(45)),
				deleted: true,
				deleted_at: Some(at(45)),
			},
			ExportAnnotation {
				id: "l4".to_string(),
				kind: ExportAnnotationKind::Bookmark,
				origin: AnnotationOrigin::Liseur { rev: 1, seq: 8 },
				locator: Some(json!({"cfi": "/6/8!/2"})),
				progression: Some(0.75),
				color: None,
				excerpt: None,
				note: None,
				created_at: Some(at(50)),
				updated_at: Some(at(50)),
				deleted: false,
				deleted_at: None,
			},
		],
		bookmarks: vec![ExportBookmark {
			id: "b1".to_string(),
			locator: Some(json!({"href": "ch3.xhtml"})),
			preview_content: Some("A beginning is the time".to_string()),
			page: Some(42),
			created_at: at(60),
		}],
		reading: Some(ReadingSummary {
			progression: Some(0.7512),
			page: Some(300),
			completed: false,
			last_read_at: Some(at(70)),
			source_protocol: Some("liseur".to_string()),
			session_count: 2,
			total_seconds: Some(5400),
			last_session_at: Some(at(70)),
			finished: false,
		}),
		review: None,
	}
}

pub(crate) fn fixture_batch(books: Vec<ExportBook>) -> ExportBatch {
	ExportBatch {
		user_id: "user-1".to_string(),
		liseur_from_seq: 0,
		liseur_high_water: 8,
		books,
	}
}
