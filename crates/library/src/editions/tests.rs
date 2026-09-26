use models::entity::{liseur_sync_media_link, liseur_sync_work, media_audio};
use sea_orm::{ActiveValue, DbBackend, DbConn, Schema, Statement};

use super::*;
use ::tests::fake_data;

/// The pairing tables the shared bootstrap does not create: `liseur_sync_*`
/// predates the entity-derived schema (the liseur lane writes it with raw
/// SQL) and `media_chapter_map` is new here. The unique index is created by
/// hand because `create_table_from_entity` does not carry indexes and
/// [`set_chapter_map_entry`] upserts on it.
async fn database() -> DbConn {
	let conn = ::tests::db::test_database().await;
	let schema = Schema::new(DbBackend::Sqlite);
	for statement in [
		schema.create_table_from_entity(liseur_sync_work::Entity),
		schema.create_table_from_entity(liseur_sync_media_link::Entity),
		schema.create_table_from_entity(media_chapter_map::Entity),
	] {
		conn.execute(conn.get_database_backend().build(&statement))
			.await
			.expect("create pairing table");
	}
	conn.execute_unprepared(
		"CREATE UNIQUE INDEX uq_media_chapter_map_pair_spine
		 ON media_chapter_map (ebook_media_id, audio_media_id, ebook_spine_index)",
	)
	.await
	.expect("create chapter map index");
	conn.execute_unprepared(
		"CREATE TABLE liseur_sync_aliases (
			id TEXT PRIMARY KEY, user_id TEXT NOT NULL, kind TEXT NOT NULL,
			value TEXT NOT NULL, work_id TEXT NOT NULL, edition_sha TEXT,
			created_at TEXT NOT NULL
		)",
	)
	.await
	.expect("create Liseur alias lookup table");
	conn.execute_unprepared(
		"CREATE TABLE liseur_sync_editions (
			id TEXT PRIMARY KEY, user_id TEXT NOT NULL, work_id TEXT NOT NULL,
			edition_sha TEXT NOT NULL, media_id TEXT, created_at TEXT
		)",
	)
	.await
	.expect("create Liseur edition lookup table");
	// A new link re-sequences the work's CAS records, so the feed tables the
	// liseur lane writes with raw SQL have to exist too.
	for sql in [
		"CREATE TABLE liseur_sync_counters (
			user_id TEXT PRIMARY KEY, op_seq BIGINT NOT NULL DEFAULT 0,
			annotation_seq BIGINT NOT NULL DEFAULT 0,
			projected_annotation_seq BIGINT NOT NULL DEFAULT 0
		)",
		"CREATE TABLE liseur_sync_annotations (
			row_id INTEGER PRIMARY KEY AUTOINCREMENT, user_id TEXT NOT NULL,
			annotation_id TEXT NOT NULL, rev BIGINT NOT NULL, seq BIGINT NOT NULL,
			work_id TEXT NOT NULL, deleted BOOLEAN NOT NULL DEFAULT FALSE,
			updated_at TEXT NOT NULL, device_id TEXT NOT NULL, payload TEXT NOT NULL
		)",
	] {
		conn.execute_unprepared(sql)
			.await
			.expect("create Liseur annotation feed table");
	}
	conn
}

struct Book {
	media: media::Model,
}

/// A book in a real series and library, so the visibility filter
/// [`media::ModelWithMetadata::find_for_user`] applies has rows to join.
async fn book(
	conn: &DbConn,
	series_id: &str,
	name: &str,
	extension: &str,
	title: Option<&str>,
	author: Option<&str>,
	isbn: Option<&str>,
	asin: Option<&str>,
) -> Book {
	let media = fake_data::Media {
		series_id: series_id.to_owned(),
		name: Some(name.to_owned()),
		extension: Some(extension.to_owned()),
		..Default::default()
	}
	.insert(conn)
	.await;

	media_metadata::ActiveModel {
		media_id: ActiveValue::Set(Some(media.id.clone())),
		title: ActiveValue::Set(title.map(str::to_owned)),
		writers: ActiveValue::Set(author.map(str::to_owned)),
		identifier_isbn: ActiveValue::Set(isbn.map(str::to_owned)),
		identifier_amazon: ActiveValue::Set(asin.map(str::to_owned)),
		..Default::default()
	}
	.insert(conn)
	.await
	.expect("insert metadata");

	Book { media }
}

/// Make a book an audio publication. The `media_audio` row is what
/// [`super::is_audio`] reads, and what makes a book the complement of an EPUB.
async fn make_audio(conn: &DbConn, media_id: &str, duration_ms: i64) {
	media_audio::ActiveModel {
		media_id: ActiveValue::Set(media_id.to_owned()),
		duration_ms: ActiveValue::Set(duration_ms),
		codec: ActiveValue::Set("aac".to_owned()),
		chapter_source: ActiveValue::Set(
			models::domain::audio::AudioChapterSource::Mp4Chpl,
		),
		..Default::default()
	}
	.insert(conn)
	.await
	.expect("insert media_audio");
}

async fn library_and_series(conn: &DbConn) -> String {
	let library = fake_data::Library::default().insert(conn).await;
	let series = fake_data::Series {
		library_id: Some(library.id.clone()),
		..Default::default()
	}
	.insert(conn)
	.await;
	series.id
}

fn owner(id: &str) -> AuthUser {
	AuthUser {
		id: id.to_owned(),
		is_server_owner: true,
		..Default::default()
	}
}

/// A link row exactly as the liseur lane writes it: no pairing columns, so
/// `pair_status` is the schema default and `pair_evidence` is null.
async fn liseur_link(conn: &DbConn, user_id: &str, media_id: &str, work_id: &str) {
	liseur_sync_media_link::ActiveModel {
		id: ActiveValue::Set(Uuid::new_v4().to_string()),
		user_id: ActiveValue::Set(user_id.to_owned()),
		media_id: ActiveValue::Set(media_id.to_owned()),
		work_id: ActiveValue::Set(work_id.to_owned()),
		edition_sha: ActiveValue::Set("sha".to_owned()),
		resolution_status: ActiveValue::Set("verified".to_owned()),
		created_at: ActiveValue::Set(Utc::now().to_rfc3339()),
		pair_status: ActiveValue::Set(PairStatus::Confirmed.to_string()),
		pair_evidence: ActiveValue::Set(None),
	}
	.insert(conn)
	.await
	.expect("insert link");
}

async fn work(conn: &DbConn, user_id: &str, id: &str) {
	liseur_sync_work::ActiveModel {
		id: ActiveValue::Set(id.to_owned()),
		user_id: ActiveValue::Set(user_id.to_owned()),
		title: ActiveValue::Set("The Lottery".to_owned()),
		author: ActiveValue::Set("Shirley Jackson".to_owned()),
		pending: ActiveValue::Set(false),
		created_at: ActiveValue::Set(Utc::now().to_rfc3339()),
	}
	.insert(conn)
	.await
	.expect("insert work");
}

/// Rule 1: two media rows already linked to the same work are editions with
/// nothing to compute. The links were written by the liseur lane, which knows
/// nothing about pairing, so they must read as confirmed rather than as
/// guesses awaiting review.
#[tokio::test]
async fn editions_pairs_media_sharing_a_work() {
	let conn = database().await;
	let user = fake_data::User::new("owner").insert(&conn).await;
	let user = owner(&user.id);
	let series_id = library_and_series(&conn).await;

	let audio = book(
		&conn,
		&series_id,
		"The Lottery",
		"m4b",
		Some("The Lottery"),
		Some("Shirley Jackson"),
		None,
		None,
	)
	.await;
	make_audio(&conn, &audio.media.id, 120_000).await;
	let ebook = book(
		&conn,
		&series_id,
		"The Lottery",
		"epub",
		Some("The Lottery"),
		Some("Shirley Jackson"),
		None,
		None,
	)
	.await;

	work(&conn, &user.id, "work-lottery").await;
	liseur_link(&conn, &user.id, &audio.media.id, "work-lottery").await;
	liseur_link(&conn, &user.id, &ebook.media.id, "work-lottery").await;

	let editions = pair_editions(&conn, &user, &audio.media.id, &[])
		.await
		.expect("pair");

	assert_eq!(editions.len(), 1);
	assert_eq!(editions[0].media_id, ebook.media.id);
	assert_eq!(editions[0].status, PairStatus::Confirmed);
	assert_eq!(editions[0].work_id, "work-lottery");
}

/// Rule 2: the identifiers do not match each other — an audiobook carries an
/// ASIN and the ebook an ISBN — and the titles do not match either. Only the
/// provider's edition list connects them, and it is the case the console
/// cannot solve by looking at titles.
#[tokio::test]
async fn editions_pairs_through_a_provider_edition_list() {
	let conn = database().await;
	let user = fake_data::User::new("owner").insert(&conn).await;
	let user = owner(&user.id);
	let series_id = library_and_series(&conn).await;

	let audio = book(
		&conn,
		&series_id,
		"The Lottery",
		"m4b",
		Some("The Lottery"),
		Some("Shirley Jackson"),
		None,
		Some("B002V02KPU"),
	)
	.await;
	make_audio(&conn, &audio.media.id, 120_000).await;
	// Deliberately retitled, so no title comparison could pair these two.
	let ebook = book(
		&conn,
		&series_id,
		"Loteria",
		"epub",
		Some("Loteria"),
		Some("Shirley Jackson"),
		Some("978-0-374-52953-6"),
		None,
	)
	.await;

	let server = metadata_integrations::mock_http::MockServer::spawn(vec![
		metadata_integrations::mock_http::render_ok(
			&serde_json::json!({
				"asin": "B002V02KPU",
				"title": "The Lottery",
				"isbn": "9780374529536"
			})
			.to_string(),
		),
	]);
	let lookups: Vec<Box<dyn EditionLookup + Send + Sync>> =
		vec![metadata_integrations::editions::create_edition_lookup_at(
			"AUDIBLE",
			&server.url,
		)
		.expect("audible lookup")];

	let editions = pair_editions(&conn, &user, &audio.media.id, &lookups)
		.await
		.expect("pair");

	assert_eq!(editions.len(), 1);
	assert_eq!(editions[0].media_id, ebook.media.id);
	assert_eq!(editions[0].status, PairStatus::Suggested);
	assert_eq!(
		editions[0].evidence,
		Some(PairEvidence::ProviderEditionList)
	);

	// The ASIN was resolved against Audnexus, not guessed.
	let requests = server.requests();
	assert_eq!(requests.len(), 1);
	assert!(
		requests[0].contains("/books/B002V02KPU"),
		"unexpected request: {}",
		requests[0]
	);
}

/// Rule 3: a normalised title and author match is a *suggestion*. It must not
/// appear as an edition until someone confirms it, because two different books
/// share a title far more often than they share an identifier.
#[tokio::test]
async fn editions_suggests_a_title_match_until_confirmed() {
	let conn = database().await;
	let user = fake_data::User::new("owner").insert(&conn).await;
	let user = owner(&user.id);
	let series_id = library_and_series(&conn).await;

	let audio = book(
		&conn,
		&series_id,
		"The Lottery",
		"m4b",
		Some("The Lottery"),
		Some("Shirley Jackson"),
		None,
		None,
	)
	.await;
	make_audio(&conn, &audio.media.id, 120_000).await;
	let ebook = book(
		&conn,
		&series_id,
		"Lottery",
		"epub",
		// A sort title with a subtitle: normalisation has to see through both.
		Some("Lottery, The: A Story"),
		Some("Jackson, Shirley"),
		None,
		None,
	)
	.await;

	let editions = pair_editions(&conn, &user, &audio.media.id, &[])
		.await
		.expect("pair");
	assert_eq!(editions.len(), 1);
	assert_eq!(editions[0].media_id, ebook.media.id);
	assert_eq!(editions[0].status, PairStatus::Suggested);
	assert_eq!(editions[0].evidence, Some(PairEvidence::TitleAuthor));

	// A suggestion is not an edition.
	let confirmed = edition_pair::linked_media(
		&conn,
		&user.id,
		&audio.media.id,
		Some(PairStatus::Confirmed),
	)
	.await
	.expect("confirmed");
	assert!(confirmed.is_empty());

	// Recomputing must not multiply the suggestion.
	let again = pair_editions(&conn, &user, &audio.media.id, &[])
		.await
		.expect("pair again");
	assert_eq!(again, editions);

	let outcome =
		confirm_edition_pair(&conn, &user.id, &audio.media.id, &ebook.media.id, None)
			.await
			.expect("confirm");
	assert!(matches!(
		outcome,
		PairOutcome::Written {
			status: PairStatus::Confirmed,
			..
		}
	));

	let editions = pair_editions(&conn, &user, &audio.media.id, &[])
		.await
		.expect("pair after confirm");
	assert_eq!(editions[0].status, PairStatus::Confirmed);
}

#[tokio::test]
async fn pairing_never_repoints_links_between_resolved_works() {
	let conn = database().await;
	let user = fake_data::User::new("owner").insert(&conn).await;
	let series_id = library_and_series(&conn).await;
	let first = book(
		&conn,
		&series_id,
		"Edition 0",
		"epub",
		Some("A Shared Work"),
		Some("Test Author"),
		None,
		None,
	)
	.await;
	let first_pair = book(
		&conn,
		&series_id,
		"Edition 1",
		"epub",
		Some("A Shared Work"),
		Some("Test Author"),
		None,
		None,
	)
	.await;
	let second = book(
		&conn,
		&series_id,
		"Edition 2",
		"epub",
		Some("A Shared Work"),
		Some("Test Author"),
		None,
		None,
	)
	.await;
	let second_pair = book(
		&conn,
		&series_id,
		"Edition 3",
		"epub",
		Some("A Shared Work"),
		Some("Test Author"),
		None,
		None,
	)
	.await;

	edition_pair::suggest_pair(
		&conn,
		&user.id,
		&first.media.id,
		&first_pair.media.id,
		PairEvidence::TitleAuthor,
	)
	.await
	.expect("first work suggestion");
	edition_pair::suggest_pair(
		&conn,
		&user.id,
		&second.media.id,
		&second_pair.media.id,
		PairEvidence::TitleAuthor,
	)
	.await
	.expect("second work suggestion");
	let second_link =
		edition_pair::link_for_media(&conn, &user.id, &second_pair.media.id)
			.await
			.expect("second work link")
			.expect("second work link exists");

	for outcome in [
		edition_pair::suggest_pair(
			&conn,
			&user.id,
			&first.media.id,
			&second_pair.media.id,
			PairEvidence::TitleAuthor,
		)
		.await
		.expect("conflicting suggestion"),
		edition_pair::confirm_pair(
			&conn,
			&user.id,
			&first.media.id,
			&second_pair.media.id,
		)
		.await
		.expect("conflicting confirmation"),
	] {
		assert_eq!(
			outcome,
			PairOutcome::Unchanged(PairUnchanged::WorkConflict {
				media_id: second_pair.media.id.clone(),
				work_id: second_link.work_id.clone(),
			})
		);
		let after = edition_pair::link_for_media(&conn, &user.id, &second_pair.media.id)
			.await
			.expect("link remains readable")
			.expect("existing link remains");
		assert_eq!(after.id, second_link.id);
		assert_eq!(after.work_id, second_link.work_id);
		assert_eq!(after.pair_status, PairStatus::Suggested.to_string());
		assert_eq!(
			edition_pair::link_for_media(&conn, &user.id, &first.media.id)
				.await
				.expect("anchor remains readable")
				.expect("anchor link remains")
				.work_id,
			edition_pair::link_for_media(&conn, &user.id, &first_pair.media.id)
				.await
				.expect("anchor pair remains readable")
				.expect("anchor pair link remains")
				.work_id
		);
	}
}

#[tokio::test]
async fn confirming_two_confirmed_work_identities_is_refused() {
	let conn = database().await;
	let user = fake_data::User::new("owner").insert(&conn).await;
	let series_id = library_and_series(&conn).await;
	let left = book(&conn, &series_id, "Left", "m4b", None, None, None, None).await;
	let right = book(&conn, &series_id, "Right", "epub", None, None, None, None).await;
	work(&conn, &user.id, "left-work").await;
	work(&conn, &user.id, "right-work").await;
	liseur_link(&conn, &user.id, &left.media.id, "left-work").await;
	liseur_link(&conn, &user.id, &right.media.id, "right-work").await;

	let outcome =
		edition_pair::confirm_pair(&conn, &user.id, &left.media.id, &right.media.id)
			.await
			.expect("conflicting confirmation is a business no-op");
	assert_eq!(
		outcome,
		PairOutcome::Unchanged(PairUnchanged::WorkConflict {
			media_id: right.media.id.clone(),
			work_id: "right-work".to_owned(),
		})
	);
	assert_eq!(
		edition_pair::link_for_media(&conn, &user.id, &left.media.id)
			.await
			.expect("left remains readable")
			.unwrap()
			.work_id,
		"left-work"
	);
	assert_eq!(
		edition_pair::link_for_media(&conn, &user.id, &right.media.id)
			.await
			.expect("right remains readable")
			.unwrap()
			.work_id,
		"right-work"
	);
}

#[tokio::test]
async fn alias_resolved_media_reuses_its_existing_work_without_a_link() {
	let conn = database().await;
	let user = fake_data::User::new("owner").insert(&conn).await;
	let series_id = library_and_series(&conn).await;
	let identified = book(
		&conn,
		&series_id,
		"Identified edition",
		"epub",
		Some("A Shared Work"),
		Some("Test Author"),
		None,
		None,
	)
	.await;
	let candidate = book(
		&conn,
		&series_id,
		"Candidate edition",
		"m4b",
		Some("A Shared Work"),
		Some("Test Author"),
		None,
		None,
	)
	.await;
	let identity_work = "alias-resolved-work";
	work(&conn, &user.id, identity_work).await;
	conn.execute(Statement::from_sql_and_values(
		DbBackend::Sqlite,
		"INSERT INTO liseur_sync_editions \
		 (id, user_id, work_id, edition_sha, media_id, created_at) \
		 VALUES ($1, $2, $3, $4, $5, $6)",
		vec![
			"resolved-edition".into(),
			user.id.clone().into(),
			identity_work.into(),
			"edition-sha".into(),
			identified.media.id.clone().into(),
			"2026-09-01T00:00:00Z".into(),
		],
	))
	.await
	.expect("insert the alias-resolved edition");
	conn.execute(Statement::from_sql_and_values(
		DbBackend::Sqlite,
		"INSERT INTO liseur_sync_aliases \
		 (id, user_id, kind, value, work_id, edition_sha, created_at) \
		 VALUES ($1, $2, 'sha256', $3, $4, $5, $6)",
		vec![
			"resolved-alias".into(),
			user.id.clone().into(),
			"edition-sha".into(),
			identity_work.into(),
			"edition-sha".into(),
			"2026-09-01T00:00:00Z".into(),
		],
	))
	.await
	.expect("insert the work alias");

	let outcome = edition_pair::suggest_pair(
		&conn,
		&user.id,
		&identified.media.id,
		&candidate.media.id,
		PairEvidence::TitleAuthor,
	)
	.await
	.expect("suggest the pairing");
	assert_eq!(
		outcome,
		PairOutcome::Written {
			work_id: identity_work.to_owned(),
			status: PairStatus::Suggested,
		}
	);
	for media_id in [&identified.media.id, &candidate.media.id] {
		assert_eq!(
			edition_pair::link_for_media(&conn, &user.id, media_id)
				.await
				.expect("link lookup succeeds")
				.expect("pair should have a media link")
				.work_id,
			identity_work
		);
	}
	assert_eq!(
		liseur_sync_work::Entity::find()
			.all(&conn)
			.await
			.expect("work lookup succeeds")
			.len(),
		1,
		"pairing must not create a parallel alias-less work"
	);
}

/// A user can keep a provider-confirmed EPUB and manually add another EPUB as
/// an edition of the same audiobook work.
#[tokio::test]
async fn confirming_a_second_ebook_in_an_existing_audio_work_succeeds() {
	let conn = database().await;
	let user_row = fake_data::User::new("owner").insert(&conn).await;
	let user = owner(&user_row.id);
	let series_id = library_and_series(&conn).await;

	let audio = book(
		&conn,
		&series_id,
		"Audio edition",
		"m4b",
		Some("A Shared Work"),
		Some("Test Author"),
		None,
		None,
	)
	.await;
	make_audio(&conn, &audio.media.id, 120_000).await;
	let provider_ebook = book(
		&conn,
		&series_id,
		"Provider EPUB",
		"epub",
		Some("A Shared Work"),
		Some("Test Author"),
		None,
		None,
	)
	.await;
	let manual_ebook = book(
		&conn,
		&series_id,
		"Manual EPUB",
		"epub",
		Some("A Shared Work"),
		Some("Test Author"),
		None,
		None,
	)
	.await;

	edition_pair::suggest_pair(
		&conn,
		&user.id,
		&audio.media.id,
		&provider_ebook.media.id,
		PairEvidence::ProviderEditionList,
	)
	.await
	.expect("suggest provider edition");
	let provider_outcome = confirm_edition_pair(
		&conn,
		&user.id,
		&audio.media.id,
		&provider_ebook.media.id,
		None,
	)
	.await
	.expect("confirm provider edition");
	assert!(matches!(
		provider_outcome,
		PairOutcome::Written {
			status: PairStatus::Confirmed,
			..
		}
	));

	let outcome = confirm_edition_pair(
		&conn,
		&user.id,
		&audio.media.id,
		&manual_ebook.media.id,
		None,
	)
	.await
	.expect("a second EPUB may join the audio's existing work");
	assert!(matches!(
		outcome,
		PairOutcome::Written {
			status: PairStatus::Confirmed,
			..
		}
	));

	let confirmed = edition_pair::linked_media(
		&conn,
		&user.id,
		&audio.media.id,
		Some(PairStatus::Confirmed),
	)
	.await
	.expect("load confirmed editions");
	assert_eq!(confirmed.len(), 2);
	assert!(confirmed
		.iter()
		.any(|link| link.media_id == provider_ebook.media.id));
	assert!(confirmed
		.iter()
		.any(|link| link.media_id == manual_ebook.media.id));
}

/// A rejected suggestion must stay rejected. Suggestions are recomputed on
/// every book-page query, so a rejection that was not persisted would come
/// straight back on the next page load.
#[tokio::test]
async fn editions_never_resurrect_a_rejected_suggestion() {
	let conn = database().await;
	let user = fake_data::User::new("owner").insert(&conn).await;
	let user = owner(&user.id);
	let series_id = library_and_series(&conn).await;

	let audio = book(
		&conn,
		&series_id,
		"The Lottery",
		"m4b",
		Some("The Lottery"),
		Some("Shirley Jackson"),
		None,
		None,
	)
	.await;
	make_audio(&conn, &audio.media.id, 120_000).await;
	let ebook = book(
		&conn,
		&series_id,
		"The Lottery",
		"epub",
		Some("The Lottery"),
		Some("Shirley Jackson"),
		None,
		None,
	)
	.await;

	let suggested = pair_editions(&conn, &user, &audio.media.id, &[])
		.await
		.expect("pair");
	assert_eq!(
		suggested.len(),
		1,
		"nothing was suggested, so the rejection below would prove nothing"
	);
	reject_edition_pair(&conn, &user.id, &audio.media.id, &ebook.media.id)
		.await
		.expect("reject");

	let editions = pair_editions(&conn, &user, &audio.media.id, &[])
		.await
		.expect("pair after reject");
	assert!(
		editions.is_empty(),
		"a rejected suggestion came back: {editions:?}"
	);
}

fn chapter(index: i32, title: &str) -> MapChapter {
	MapChapter {
		index,
		title: Some(title.to_owned()),
	}
}

/// Front and back matter has no counterpart on the other side, and forcing it
/// onto one would put the listener on the copyright page. Titles that do match
/// win over position.
#[test]
fn editions_chapter_map_skips_matter_and_prefers_titles() {
	let ebook = vec![
		chapter(0, "Cover"),
		chapter(1, "Copyright Page"),
		chapter(2, "The Lottery"),
		chapter(3, "The Daemon Lover"),
		chapter(4, "About the Author"),
	];
	let audio = vec![
		chapter(0, "Opening Credits"),
		chapter(1, "The Daemon Lover"),
		chapter(2, "The Lottery"),
		chapter(3, "End Credits"),
	];

	let entries = build_chapter_map(&ebook, &audio);

	assert_eq!(
		entries,
		vec![
			ChapterMapEntry {
				ebook_spine_index: 2,
				audio_chapter_index: 2,
				confidence: 1.0,
			},
			ChapterMapEntry {
				ebook_spine_index: 3,
				audio_chapter_index: 1,
				confidence: 1.0,
			},
		]
	);
}

/// "Chapter 12" ↔ "12" is the case the ordinal pass exists for; a chapter with
/// no usable title at all falls through to position, and the confidence says
/// which pass produced each entry.
#[test]
fn editions_chapter_map_falls_back_to_ordinal_then_position() {
	let ebook = vec![
		chapter(0, "Chapter 1"),
		chapter(1, "Chapter 2"),
		chapter(2, "Afterword"),
	];
	let audio = vec![
		chapter(0, "2"),
		chapter(1, "1"),
		MapChapter {
			index: 2,
			title: None,
		},
	];

	let entries = build_chapter_map(&ebook, &audio);

	assert_eq!(
		entries,
		vec![
			ChapterMapEntry {
				ebook_spine_index: 0,
				audio_chapter_index: 1,
				confidence: 0.85,
			},
			ChapterMapEntry {
				ebook_spine_index: 1,
				audio_chapter_index: 0,
				confidence: 0.85,
			},
			ChapterMapEntry {
				ebook_spine_index: 2,
				audio_chapter_index: 2,
				confidence: 0.5,
			},
		]
	);
}

/// A map is replaced wholesale when recomputed, and one entry can be edited
/// afterwards without disturbing the rest — the upsert is what makes an
/// operator's correction stick instead of duplicating the row.
#[tokio::test]
async fn editions_chapter_map_is_replaceable_and_editable() {
	let conn = database().await;
	let series_id = library_and_series(&conn).await;
	let ebook = book(&conn, &series_id, "Book", "epub", None, None, None, None).await;
	let audio = book(&conn, &series_id, "Book", "m4b", None, None, None, None).await;

	let entries = vec![
		ChapterMapEntry {
			ebook_spine_index: 1,
			audio_chapter_index: 1,
			confidence: 1.0,
		},
		ChapterMapEntry {
			ebook_spine_index: 2,
			audio_chapter_index: 2,
			confidence: 0.5,
		},
	];
	replace_chapter_map(&conn, &ebook.media.id, &audio.media.id, &entries)
		.await
		.expect("write map");
	replace_chapter_map(&conn, &ebook.media.id, &audio.media.id, &entries)
		.await
		.expect("rewrite map");

	let stored = chapter_map(&conn, &ebook.media.id, &audio.media.id)
		.await
		.expect("read map");
	assert_eq!(stored.len(), 2);

	set_chapter_map_entry(&conn, &ebook.media.id, &audio.media.id, 2, 3, None)
		.await
		.expect("edit entry");
	let stored = chapter_map(&conn, &ebook.media.id, &audio.media.id)
		.await
		.expect("read map again");
	assert_eq!(stored.len(), 2);
	assert_eq!(stored[1].audio_chapter_index, 3);
	assert_eq!(stored[1].confidence, 1.0);

	assert!(
		clear_chapter_map_entry(&conn, &ebook.media.id, &audio.media.id, 2)
			.await
			.expect("clear entry")
	);
	assert_eq!(
		chapter_map(&conn, &ebook.media.id, &audio.media.id)
			.await
			.expect("read map after clear")
			.len(),
		1
	);
}

/// The feed position of a work's CAS records is what every annotation
/// consumer reads by, so a link that is new must replay them; a suggestion
/// that is already recorded, or a confirmation of it, changes no edition and
/// must leave the feed exactly where it was.
#[tokio::test]
async fn a_new_pair_link_resequences_the_work_but_an_existing_one_does_not() {
	let conn = database().await;
	let user = fake_data::User::new("owner").insert(&conn).await;
	let series_id = library_and_series(&conn).await;
	let ebook = book(
		&conn,
		&series_id,
		"Annotated edition",
		"epub",
		Some("A Shared Work"),
		Some("Test Author"),
		None,
		None,
	)
	.await;
	let audio = book(
		&conn,
		&series_id,
		"Audio edition",
		"m4b",
		Some("A Shared Work"),
		Some("Test Author"),
		None,
		None,
	)
	.await;
	make_audio(&conn, &audio.media.id, 120_000).await;
	let work_id = "annotated-work";
	work(&conn, &user.id, work_id).await;
	liseur_link(&conn, &user.id, &ebook.media.id, work_id).await;
	conn.execute(Statement::from_sql_and_values(
		DbBackend::Sqlite,
		"INSERT INTO liseur_sync_counters (user_id, annotation_seq) VALUES ($1, 5)",
		vec![user.id.clone().into()],
	))
	.await
	.expect("seed the feed counter");
	conn.execute(Statement::from_sql_and_values(
		DbBackend::Sqlite,
		"INSERT INTO liseur_sync_annotations \
		 (user_id, annotation_id, rev, seq, work_id, deleted, updated_at, device_id, payload) \
		 VALUES ($1, 'kobo-note', 2, 5, $2, FALSE, '2026-09-11T12:00:00Z', 'kobo', '{}')",
		vec![user.id.clone().into(), work_id.into()],
	))
	.await
	.expect("seed a live CAS record");
	let feed_state = || async {
		let row = conn
			.query_one(Statement::from_sql_and_values(
				DbBackend::Sqlite,
				"SELECT a.rev AS rev, a.seq AS seq, a.updated_at AS updated_at, \
				        c.annotation_seq AS high_water \
				 FROM liseur_sync_annotations a \
				 JOIN liseur_sync_counters c ON c.user_id = a.user_id \
				 WHERE a.annotation_id = 'kobo-note'",
				Vec::<sea_orm::Value>::new(),
			))
			.await
			.expect("read the feed state")
			.expect("the record exists");
		(
			row.try_get::<i64>("", "rev").unwrap(),
			row.try_get::<i64>("", "seq").unwrap(),
			row.try_get::<String>("", "updated_at").unwrap(),
			row.try_get::<i64>("", "high_water").unwrap(),
		)
	};

	let suggested = edition_pair::suggest_pair(
		&conn,
		&user.id,
		&ebook.media.id,
		&audio.media.id,
		PairEvidence::TitleAuthor,
	)
	.await
	.expect("suggest the audiobook into the annotated work");
	assert_eq!(
		suggested,
		PairOutcome::Written {
			work_id: work_id.to_owned(),
			status: PairStatus::Suggested,
		}
	);
	// One new link (the audiobook's): the record moved above the old high
	// water and nothing but its position changed.
	assert_eq!(
		feed_state().await,
		(2, 6, "2026-09-11T12:00:00Z".to_owned(), 6)
	);

	let repeated = edition_pair::suggest_pair(
		&conn,
		&user.id,
		&ebook.media.id,
		&audio.media.id,
		PairEvidence::TitleAuthor,
	)
	.await
	.expect("repeat the suggestion");
	assert!(matches!(repeated, PairOutcome::Unchanged(_)));
	let confirmed =
		edition_pair::confirm_pair(&conn, &user.id, &ebook.media.id, &audio.media.id)
			.await
			.expect("confirm the existing pair");
	assert_eq!(
		confirmed,
		PairOutcome::Written {
			work_id: work_id.to_owned(),
			status: PairStatus::Confirmed,
		}
	);
	assert_eq!(
		feed_state().await,
		(2, 6, "2026-09-11T12:00:00Z".to_owned(), 6),
		"an existing link's verdict change must not replay the work"
	);
}
