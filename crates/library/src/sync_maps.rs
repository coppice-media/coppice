//! Validated tier-2 read-aloud maps and explicit alignment jobs.
//!
//! A map is accepted only for a confirmed ebook↔audiobook pair and only after
//! the worker validator has checked every cue against the probed track
//! durations. Imports are an idempotent cache write keyed by the map's exact
//! input and generator identity. Enqueueing is explicit and deduplicates every
//! non-terminal `align` job for the same pair and granularity.

use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
	sync::Arc,
};

use chrono::Utc;
use models::{
	domain::edition_pair::PairStatus,
	entity::{
		liseur_sync_media_link, media, media_audio_track, media_sync_map, worker_job,
	},
	txn::begin_write,
};
use sea_orm::{
	prelude::*,
	sea_query::{Condition, OnConflict},
	ActiveValue::Set,
	ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use stump_worker::{
	align_requires, AlignExecutionProvider, AlignGranularity, AlignInput, AlignPrecision,
	AlignResult, ResultValidator, SyncMapV1, SyncMapValidationContext,
	SyncMapValidationError, WorkerJob, WorkerJobs, ALIGN, BACKGROUND_PRIORITY,
};
use thiserror::Error;

const IMPORT_SOURCE: &str = "import";
const DEFAULT_ALIGNMENT_ALGORITHM: &str = "ctc";
const DEFAULT_ALIGNMENT_MODEL: &str = "default";
const DEFAULT_ALIGNMENT_MODEL_REVISION: &str = "default";
const DEFAULT_ALIGNMENT_LANGUAGE: &str = "und";

/// Failures returned by the shared map/enqueue path.
#[derive(Debug, Error)]
pub enum SyncMapError {
	#[error("database error: {0}")]
	Database(#[from] DbErr),
	#[error("invalid SyncMap: {0}")]
	Validation(#[from] SyncMapValidationError),
	#[error("worker enqueue failed: {0}")]
	Worker(#[from] stump_worker::WorkerError),
	#[error("media {media_id} was not found")]
	MediaNotFound { media_id: String },
	#[error("the ebook and audiobook are not a confirmed edition pair")]
	PairNotConfirmed,
	#[error("could not digest media {media_id}: {source}")]
	Digest {
		media_id: String,
		#[source]
		source: std::io::Error,
	},
	#[error("could not serialize the audio manifest: {0}")]
	ManifestSerialization(#[from] serde_json::Error),
	#[error("read-aloud EPUB is not usable: {0}")]
	ReadAloud(#[from] stump_media::ReadAloudError),
	#[error("the SyncMap contains too many cues")]
	CueCountOverflow,
	#[error("the audiobook must be one M4B file")]
	UnsupportedAudio,
	#[error("alignment jobs must carry their requesting user")]
	RequesterMissing,
}

/// Build the duration evidence used by `SyncMapV1::validate`.
///
/// Track indices are the persisted 0-based publication order, and each value
/// is the duration of that track alone. The validator deliberately does not use
/// `media_audio.duration_ms`: cue intervals are track-local, not publication-
/// relative.
pub async fn validation_context<C: ConnectionTrait>(
	conn: &C,
	audio_media_id: &str,
) -> Result<SyncMapValidationContext, DbErr> {
	let tracks = media_audio_track::Entity::find()
		.filter(media_audio_track::Column::MediaId.eq(audio_media_id))
		.order_by_asc(media_audio_track::Column::Index)
		.all(conn)
		.await?;

	Ok(tracks
		.into_iter()
		.map(|track| (track.index, track.duration_ms))
		.collect::<BTreeMap<_, _>>())
}

/// Validate and persist an operator-supplied `SyncMapV1`.
///
/// Server-side canonical-text and robust-audio digest producers do not exist
/// yet, so imports validate the map's own digest shape, media identity,
/// ordering, confidence and real track-duration bounds. The supplied digests
/// remain part of the immutable cache key and are not replaced by a weaker
/// media-row hint.
pub async fn import_sync_map(
	conn: &DatabaseConnection,
	ebook_media_id: &str,
	audio_media_id: &str,
	map: SyncMapV1,
) -> Result<media_sync_map::Model, SyncMapError> {
	if !confirmed_pair(conn, ebook_media_id, audio_media_id).await? {
		return Err(SyncMapError::PairNotConfirmed);
	}

	let context = validation_context(conn, audio_media_id).await?;
	map.validate(&context)?;
	if map.text_media_id != ebook_media_id {
		return Err(SyncMapError::Validation(
			SyncMapValidationError::InputMismatch {
				field: "text_media_id",
			},
		));
	}
	if map.audio_media_id != audio_media_id {
		return Err(SyncMapError::Validation(
			SyncMapValidationError::InputMismatch {
				field: "audio_media_id",
			},
		));
	}

	let cue_count =
		i32::try_from(map.cues.len()).map_err(|_| SyncMapError::CueCountOverflow)?;
	let map_json = serde_json::to_value(&map)?;
	let now = Utc::now().fixed_offset();
	let txn = begin_write(conn).await?;
	let persisted = media_sync_map::Entity::insert(media_sync_map::ActiveModel {
		id: Set(uuid::Uuid::new_v4().to_string()),
		ebook_media_id: Set(ebook_media_id.to_owned()),
		audio_media_id: Set(audio_media_id.to_owned()),
		granularity: Set(granularity_name(map.provenance.granularity).to_owned()),
		generator: Set(map.provenance.implementation.clone()),
		generator_version: Set(map.provenance.version.clone()),
		algorithm: Set(Some(map.provenance.algorithm.clone())),
		model: Set(Some(map.provenance.model.clone())),
		text_digest: Set(map.provenance.text_digest.clone()),
		audio_manifest_digest: Set(map.provenance.audio_manifest_digest.clone()),
		cue_count: Set(cue_count),
		// SyncMapV1 has no total-fragment denominator. Keep coverage nullable
		// until segmentation metadata becomes part of the accepted contract.
		coverage: Set(None),
		map: Set(map_json),
		source: Set(IMPORT_SOURCE.to_owned()),
		job_id: Set(None),
		created_at: Set(now),
	})
	.on_conflict(
		OnConflict::columns([
			media_sync_map::Column::EbookMediaId,
			media_sync_map::Column::AudioMediaId,
			media_sync_map::Column::Granularity,
			media_sync_map::Column::TextDigest,
			media_sync_map::Column::AudioManifestDigest,
			media_sync_map::Column::Generator,
			media_sync_map::Column::GeneratorVersion,
		])
		.update_columns([
			media_sync_map::Column::Algorithm,
			media_sync_map::Column::Model,
			media_sync_map::Column::CueCount,
			media_sync_map::Column::Coverage,
			media_sync_map::Column::Map,
			media_sync_map::Column::Source,
			media_sync_map::Column::JobId,
		])
		.to_owned(),
	)
	.exec_with_returning(&txn)
	.await?;
	txn.commit().await?;
	Ok(persisted)
}

/// Return the newest map for an explicitly oriented pair.
pub async fn latest_sync_map<C: ConnectionTrait>(
	conn: &C,
	ebook_media_id: &str,
	audio_media_id: &str,
	granularity: AlignGranularity,
) -> Result<Option<media_sync_map::Model>, DbErr> {
	Ok(media_sync_map::Entity::find()
		.filter(media_sync_map::Column::EbookMediaId.eq(ebook_media_id))
		.filter(media_sync_map::Column::AudioMediaId.eq(audio_media_id))
		.filter(media_sync_map::Column::Granularity.eq(granularity_name(granularity)))
		.order_by_desc(media_sync_map::Column::CreatedAt)
		.order_by_desc(media_sync_map::Column::Id)
		.one(conn)
		.await?)
}

/// Return the newest map where `media_id` is either side of the pair.
pub async fn latest_sync_map_for_media<C: ConnectionTrait>(
	conn: &C,
	media_id: &str,
	granularity: AlignGranularity,
) -> Result<Option<media_sync_map::Model>, DbErr> {
	Ok(media_sync_map::Entity::find()
		.filter(
			Condition::any()
				.add(media_sync_map::Column::EbookMediaId.eq(media_id))
				.add(media_sync_map::Column::AudioMediaId.eq(media_id)),
		)
		.filter(media_sync_map::Column::Granularity.eq(granularity_name(granularity)))
		.order_by_desc(media_sync_map::Column::CreatedAt)
		.order_by_desc(media_sync_map::Column::Id)
		.one(conn)
		.await?)
}

/// Build and route one explicit alignment job.
///
/// The default profile is deliberately stable (`ctc`/`default`/CPU/fp32), so
/// the capability requirement is deterministic until a future profile-selection
/// input is added to the public action. A worker advertising that exact `align`
/// capability is eligible; without one the queue parks the row as
/// `needs_worker` through the normal dispatcher.
pub async fn enqueue_alignment(
	conn: &DatabaseConnection,
	jobs: &WorkerJobs,
	ebook_media_id: &str,
	audio_media_id: &str,
	granularity: AlignGranularity,
) -> Result<worker_job::Model, SyncMapError> {
	if !confirmed_pair(conn, ebook_media_id, audio_media_id).await? {
		return Err(SyncMapError::PairNotConfirmed);
	}

	let requested_granularity = granularity_name(granularity);
	for job in jobs.active_of_kind(ALIGN).await? {
		if same_alignment_target(
			&job.input,
			ebook_media_id,
			audio_media_id,
			requested_granularity,
		) {
			return Ok(job);
		}
	}

	let input =
		alignment_input(conn, ebook_media_id, audio_media_id, granularity).await?;
	let input_json = serde_json::to_value(&input)?;
	let requires = align_requires(&input);
	Ok(jobs
		.enqueue(ALIGN, input_json, requires, BACKGROUND_PRIORITY)
		.await?)
}

fn same_alignment_target(
	input: &Value,
	ebook_media_id: &str,
	audio_media_id: &str,
	granularity: &str,
) -> bool {
	input.get("text_media_id").and_then(Value::as_str) == Some(ebook_media_id)
		&& input.get("audio_media_id").and_then(Value::as_str) == Some(audio_media_id)
		&& input.get("granularity").and_then(Value::as_str) == Some(granularity)
}

async fn alignment_input(
	conn: &DatabaseConnection,
	ebook_media_id: &str,
	audio_media_id: &str,
	granularity: AlignGranularity,
) -> Result<AlignInput, SyncMapError> {
	let ebook = media::Entity::find_by_id(ebook_media_id.to_owned())
		.one(conn)
		.await?
		.ok_or_else(|| SyncMapError::MediaNotFound {
			media_id: ebook_media_id.to_owned(),
		})?;
	media::Entity::find_by_id(audio_media_id.to_owned())
		.one(conn)
		.await?
		.ok_or_else(|| SyncMapError::MediaNotFound {
			media_id: audio_media_id.to_owned(),
		})?;

	let text_digest = text_digest(&ebook)?;
	let audio_manifest_digest = audio_manifest_digest(conn, audio_media_id).await?;
	Ok(AlignInput {
		text_media_id: ebook_media_id.to_owned(),
		text_digest,
		audio_media_id: audio_media_id.to_owned(),
		audio_manifest_digest,
		algorithm: DEFAULT_ALIGNMENT_ALGORITHM.to_owned(),
		model: DEFAULT_ALIGNMENT_MODEL.to_owned(),
		model_revision: DEFAULT_ALIGNMENT_MODEL_REVISION.to_owned(),
		language: DEFAULT_ALIGNMENT_LANGUAGE.to_owned(),
		granularity,
		execution_provider: AlignExecutionProvider::Cpu,
		precision: AlignPrecision::Fp32,
		options: BTreeMap::new(),
	})
}

fn text_digest(media: &media::Model) -> Result<String, SyncMapError> {
	if let Some(hash) = media.hash.as_deref().filter(|hash| is_sha256(hash)) {
		return Ok(hash.to_owned());
	}
	let bytes = std::fs::metadata(&media.path)
		.map_err(|source| SyncMapError::Digest {
			media_id: media.id.clone(),
			source,
		})?
		.len();
	stump_media::hash::generate(&media.path, bytes).map_err(|source| {
		SyncMapError::Digest {
			media_id: media.id.clone(),
			source,
		}
	})
}

async fn audio_manifest_digest(
	conn: &DatabaseConnection,
	audio_media_id: &str,
) -> Result<String, SyncMapError> {
	let tracks = media_audio_track::Entity::find()
		.filter(media_audio_track::Column::MediaId.eq(audio_media_id))
		.order_by_asc(media_audio_track::Column::Index)
		.all(conn)
		.await?;
	let manifest = tracks
		.into_iter()
		.map(|track| {
			json!({
				"index": track.index,
				"duration_ms": track.duration_ms,
				"start_offset_ms": track.start_offset_ms,
				"byte_size": track.byte_size,
				"mime": track.mime,
			})
		})
		.collect::<Vec<_>>();
	hex_sha256(&serde_json::to_vec(&manifest)?)
}

fn hex_sha256(bytes: &[u8]) -> Result<String, SyncMapError> {
	let mut hasher = Sha256::new();
	hasher.update(bytes);
	Ok(hasher
		.finalize()
		.iter()
		.map(|byte| format!("{byte:02x}"))
		.collect())
}

fn is_sha256(value: &str) -> bool {
	value.len() == 64
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn granularity_name(granularity: AlignGranularity) -> &'static str {
	match granularity {
		AlignGranularity::Sentence => "sentence",
		AlignGranularity::Word => "word",
	}
}

async fn confirmed_pair<C: ConnectionTrait>(
	conn: &C,
	ebook_media_id: &str,
	audio_media_id: &str,
) -> Result<bool, DbErr> {
	if ebook_media_id == audio_media_id {
		return Ok(false);
	}
	let links = liseur_sync_media_link::Entity::find()
		.filter(
			liseur_sync_media_link::Column::MediaId
				.is_in([ebook_media_id.to_owned(), audio_media_id.to_owned()]),
		)
		.filter(
			liseur_sync_media_link::Column::PairStatus
				.eq(PairStatus::Confirmed.to_string()),
		)
		.all(conn)
		.await?;

	let mut groups = BTreeMap::<(String, String), BTreeMap<String, ()>>::new();
	for link in links {
		groups
			.entry((link.user_id, link.work_id))
			.or_default()
			.insert(link.media_id, ());
	}
	Ok(groups.values().any(|media_ids| {
		media_ids.contains_key(ebook_media_id) && media_ids.contains_key(audio_media_id)
	}))
}

/// Import a map on behalf of one authenticated user. Unlike the historical
/// operator helper above, this path recomputes source identity and target
/// segmentation before accepting the map.
pub async fn import_sync_map_for_user(
	conn: &DatabaseConnection,
	user_id: &str,
	ebook_media_id: &str,
	audio_media_id: &str,
	map: SyncMapV1,
) -> Result<media_sync_map::Model, SyncMapError> {
	if !confirmed_pair_for_user(conn, user_id, ebook_media_id, audio_media_id).await? {
		return Err(SyncMapError::PairNotConfirmed);
	}
	let ebook = media::Entity::find_by_id(ebook_media_id.to_owned())
		.one(conn)
		.await?
		.ok_or_else(|| SyncMapError::MediaNotFound {
			media_id: ebook_media_id.to_owned(),
		})?;
	let audio = media::Entity::find_by_id(audio_media_id.to_owned())
		.one(conn)
		.await?
		.ok_or_else(|| SyncMapError::MediaNotFound {
			media_id: audio_media_id.to_owned(),
		})?;
	let context = validation_context(conn, audio_media_id).await?;
	map.validate(&context)?;
	if map.text_media_id != ebook_media_id {
		return Err(SyncMapError::Validation(
			SyncMapValidationError::InputMismatch {
				field: "text_media_id",
			},
		));
	}
	if map.audio_media_id != audio_media_id {
		return Err(SyncMapError::Validation(
			SyncMapValidationError::InputMismatch {
				field: "audio_media_id",
			},
		));
	}
	let prepared = stump_media::read_aloud::prepare_epub(&ebook.path)?;
	if map.provenance.text_digest != prepared.canonical_text_digest {
		return Err(SyncMapError::Validation(
			SyncMapValidationError::InputMismatch {
				field: "provenance.text_digest",
			},
		));
	}
	let audio_digest = strict_audio_manifest_digest(conn, &audio).await?;
	if map.provenance.audio_manifest_digest != audio_digest {
		return Err(SyncMapError::Validation(
			SyncMapValidationError::InputMismatch {
				field: "provenance.audio_manifest_digest",
			},
		));
	}
	validate_map_targets(&prepared, &map)?;
	persist_map(conn, &map, IMPORT_SOURCE, None).await
}

/// Return the newest map for a pair visible to `user_id`.
pub async fn latest_sync_map_for_user<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	ebook_media_id: &str,
	audio_media_id: &str,
	granularity: AlignGranularity,
) -> Result<Option<media_sync_map::Model>, SyncMapError> {
	if !confirmed_pair_for_user(conn, user_id, ebook_media_id, audio_media_id).await? {
		return Ok(None);
	}
	Ok(media_sync_map::Entity::find()
		.filter(media_sync_map::Column::EbookMediaId.eq(ebook_media_id))
		.filter(media_sync_map::Column::AudioMediaId.eq(audio_media_id))
		.filter(media_sync_map::Column::Granularity.eq(granularity_name(granularity)))
		.order_by_desc(media_sync_map::Column::CreatedAt)
		.order_by_desc(media_sync_map::Column::Id)
		.one(conn)
		.await?)
}

/// Return the newest accepted map where `media_id` is either side of a
/// confirmed pair owned by `user_id`.
pub async fn latest_sync_map_for_media_user<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	media_id: &str,
	granularity: AlignGranularity,
) -> Result<Option<media_sync_map::Model>, SyncMapError> {
	let candidates = media_sync_map::Entity::find()
		.filter(
			Condition::any()
				.add(media_sync_map::Column::EbookMediaId.eq(media_id))
				.add(media_sync_map::Column::AudioMediaId.eq(media_id)),
		)
		.filter(media_sync_map::Column::Granularity.eq(granularity_name(granularity)))
		.order_by_desc(media_sync_map::Column::CreatedAt)
		.order_by_desc(media_sync_map::Column::Id)
		.all(conn)
		.await?;
	for candidate in candidates {
		if confirmed_pair_for_user(
			conn,
			user_id,
			&candidate.ebook_media_id,
			&candidate.audio_media_id,
		)
		.await?
		{
			return Ok(Some(candidate));
		}
	}
	Ok(None)
}

/// Queue an alignment request with an authenticated owner and exact canonical
/// input. The owner id is carried as non-secret job metadata so finalization
/// can repeat the pair-visibility check after a worker returns.
pub async fn enqueue_alignment_for_user(
	conn: &DatabaseConnection,
	jobs: &WorkerJobs,
	user_id: &str,
	ebook_media_id: &str,
	audio_media_id: &str,
	granularity: AlignGranularity,
) -> Result<worker_job::Model, SyncMapError> {
	if !confirmed_pair_for_user(conn, user_id, ebook_media_id, audio_media_id).await? {
		return Err(SyncMapError::PairNotConfirmed);
	}
	let input =
		strict_alignment_input(conn, ebook_media_id, audio_media_id, granularity).await?;
	for job in jobs.active_of_kind(ALIGN).await? {
		if job_input_matches(&job.input, &input) {
			return Ok(job);
		}
	}
	let mut input_json = serde_json::to_value(&input)?;
	if let Value::Object(ref mut object) = input_json {
		object.insert(
			"requester_user_id".to_owned(),
			Value::String(user_id.to_owned()),
		);
	}
	Ok(jobs
		.enqueue(
			ALIGN,
			input_json,
			align_requires(&input),
			BACKGROUND_PRIORITY,
		)
		.await?)
}

/// Derive the path used by the accepted deterministic derivative.
pub fn read_aloud_cache_path(
	cache_dir: impl AsRef<Path>,
	map: &media_sync_map::Model,
) -> Result<PathBuf, SyncMapError> {
	let map_bytes = serde_json::to_vec(&map.map)?;
	Ok(cache_dir.as_ref().join("read-aloud").join(format!(
		"{}.epub",
		stump_media::read_aloud::cache_key(
			&map.text_digest,
			&map.audio_manifest_digest,
			&map_bytes,
		)
	)))
}

/// Install the server-side ALIGN completion gate. The host calls this before
/// accepting requests; `WorkerJobs` rejects a second installation.
pub fn alignment_result_validator(
	conn: Arc<DatabaseConnection>,
	cache_dir: impl Into<PathBuf>,
) -> Arc<dyn ResultValidator> {
	Arc::new(AlignmentResultValidator {
		conn,
		cache_dir: cache_dir.into(),
	})
}

struct AlignmentResultValidator {
	conn: Arc<DatabaseConnection>,
	cache_dir: PathBuf,
}

#[async_trait::async_trait]
impl ResultValidator for AlignmentResultValidator {
	async fn validate(
		&self,
		job: &WorkerJob,
		result: &Value,
		output_path: &Path,
	) -> Result<(), String> {
		if job.kind != ALIGN {
			return Ok(());
		}
		let input: AlignInput = serde_json::from_value(job.input.clone())
			.map_err(|error| format!("invalid ALIGN input: {error}"))?;
		let requester = job
			.input
			.get("requester_user_id")
			.and_then(Value::as_str)
			.ok_or_else(|| SyncMapError::RequesterMissing.to_string())?;
		let Some(ebook) = media::Entity::find_by_id(input.text_media_id.clone())
			.one(self.conn.as_ref())
			.await
			.map_err(|error| error.to_string())?
		else {
			return Err(SyncMapError::MediaNotFound {
				media_id: input.text_media_id.clone(),
			}
			.to_string());
		};
		let Some(audio) = media::Entity::find_by_id(input.audio_media_id.clone())
			.one(self.conn.as_ref())
			.await
			.map_err(|error| error.to_string())?
		else {
			return Err(SyncMapError::MediaNotFound {
				media_id: input.audio_media_id.clone(),
			}
			.to_string());
		};
		if !confirmed_pair_for_user(
			self.conn.as_ref(),
			requester,
			&input.text_media_id,
			&input.audio_media_id,
		)
		.await
		.map_err(|error| error.to_string())?
		{
			return Err(SyncMapError::PairNotConfirmed.to_string());
		}
		let canonical = strict_alignment_input(
			self.conn.as_ref(),
			&input.text_media_id,
			&input.audio_media_id,
			input.granularity,
		)
		.await
		.map_err(|error| error.to_string())?;
		if canonical != input {
			return Err(SyncMapError::Validation(
				SyncMapValidationError::InputMismatch {
					field: "queued AlignInput",
				},
			)
			.to_string());
		}
		let context = validation_context(self.conn.as_ref(), &input.audio_media_id)
			.await
			.map_err(|error| error.to_string())?;
		let align_result: AlignResult = serde_json::from_value(result.clone())
			.map_err(|error| format!("invalid ALIGN result: {error}"))?;
		let bytes = tokio::fs::read(output_path)
			.await
			.map_err(|error| format!("read ALIGN output: {error}"))?;
		if align_result.bytes != bytes.len() as u64 {
			return Err(format!(
				"ALIGN result byte count {} does not match uploaded {}",
				align_result.bytes,
				bytes.len()
			));
		}
		let digest = sha256_hex_bytes(&bytes);
		if align_result.sha256 != digest {
			return Err("ALIGN result digest does not match uploaded bytes".into());
		}
		let map: SyncMapV1 = serde_json::from_slice(&bytes)
			.map_err(|error| format!("invalid SyncMap output: {error}"))?;
		map.validate_for(&input, &context)
			.map_err(|error| error.to_string())?;
		let prepared = stump_media::read_aloud::prepare_epub(&ebook.path)
			.map_err(|error| error.to_string())?;
		let cues = map
			.cues
			.iter()
			.map(|cue| stump_media::read_aloud::RenderCue {
				spine_index: cue.text.spine_index,
				ordinal: cue.text.ordinal,
				element_id: cue.text.element_id.clone(),
				track_index: cue.audio.track_index,
				begin_ms: cue.audio.begin_ms,
				end_ms: cue.audio.end_ms,
			})
			.collect::<Vec<_>>();
		stump_media::read_aloud::validate_targets(&prepared, &cues)
			.map_err(|error| error.to_string())?;
		let tracks = media_audio_track::Entity::find()
			.filter(media_audio_track::Column::MediaId.eq(&audio.id))
			.order_by_asc(media_audio_track::Column::Index)
			.all(self.conn.as_ref())
			.await
			.map_err(|error| error.to_string())?;
		if tracks.len() != 1 || !audio.extension.eq_ignore_ascii_case("m4b") {
			return Err(SyncMapError::UnsupportedAudio.to_string());
		}
		let cache_path = read_aloud_cache_path(
			&self.cache_dir,
			&media_sync_map::Model {
				id: String::new(),
				ebook_media_id: input.text_media_id.clone(),
				audio_media_id: input.audio_media_id.clone(),
				granularity: granularity_name(input.granularity).to_owned(),
				generator: map.provenance.implementation.clone(),
				generator_version: map.provenance.version.clone(),
				algorithm: Some(map.provenance.algorithm.clone()),
				model: Some(map.provenance.model.clone()),
				text_digest: input.text_digest.clone(),
				audio_manifest_digest: input.audio_manifest_digest.clone(),
				cue_count: i32::try_from(map.cues.len())
					.map_err(|_| SyncMapError::CueCountOverflow.to_string())?,
				coverage: None,
				map: serde_json::to_value(&map).map_err(|error| error.to_string())?,
				source: "worker".to_owned(),
				job_id: Some(job.id.clone()),
				created_at: Utc::now().fixed_offset(),
			},
		)
		.map_err(|error| error.to_string())?;
		if tokio::fs::metadata(&cache_path).await.is_err() {
			let cache_parent = cache_path
				.parent()
				.ok_or_else(|| "cache path has no parent".to_owned())?;
			tokio::fs::create_dir_all(cache_parent)
				.await
				.map_err(|error| format!("create read-aloud cache: {error}"))?;
			let prepared_for_render = prepared.clone();
			let audio_path = tracks[0].path.clone();
			let cues_for_render = cues.clone();
			let cache_path_for_render = cache_path.clone();
			tokio::task::spawn_blocking(move || {
				stump_media::read_aloud::render_to_path(
					&prepared_for_render,
					audio_path,
					&cues_for_render,
					cache_path_for_render,
				)
			})
			.await
			.map_err(|error| format!("render read-aloud cache: {error}"))?
			.map_err(|error| format!("render read-aloud cache: {error}"))?;
		}
		let map_value = serde_json::to_value(&map).map_err(|error| error.to_string())?;
		let persisted = media_sync_map::ActiveModel {
			id: Set(uuid::Uuid::new_v4().to_string()),
			ebook_media_id: Set(input.text_media_id),
			audio_media_id: Set(input.audio_media_id),
			granularity: Set(granularity_name(input.granularity).to_owned()),
			generator: Set(map.provenance.implementation.clone()),
			generator_version: Set(map.provenance.version.clone()),
			algorithm: Set(Some(map.provenance.algorithm.clone())),
			model: Set(Some(map.provenance.model.clone())),
			text_digest: Set(map.provenance.text_digest.clone()),
			audio_manifest_digest: Set(map.provenance.audio_manifest_digest.clone()),
			cue_count: Set(i32::try_from(map.cues.len())
				.map_err(|_| SyncMapError::CueCountOverflow.to_string())?),
			coverage: Set(None),
			map: Set(map_value),
			source: Set("worker".to_owned()),
			job_id: Set(Some(job.id.clone())),
			created_at: Set(Utc::now().fixed_offset()),
		};
		let txn = begin_write(self.conn.as_ref())
			.await
			.map_err(|error| error.to_string())?;
		media_sync_map::Entity::insert(persisted)
			.on_conflict(
				OnConflict::columns([
					media_sync_map::Column::EbookMediaId,
					media_sync_map::Column::AudioMediaId,
					media_sync_map::Column::Granularity,
					media_sync_map::Column::TextDigest,
					media_sync_map::Column::AudioManifestDigest,
					media_sync_map::Column::Generator,
					media_sync_map::Column::GeneratorVersion,
				])
				.update_columns([
					media_sync_map::Column::Algorithm,
					media_sync_map::Column::Model,
					media_sync_map::Column::CueCount,
					media_sync_map::Column::Coverage,
					media_sync_map::Column::Map,
					media_sync_map::Column::Source,
					media_sync_map::Column::JobId,
				])
				.to_owned(),
			)
			.exec_with_returning(&txn)
			.await
			.map_err(|error| error.to_string())?;
		txn.commit().await.map_err(|error| error.to_string())?;
		Ok(())
	}
}
async fn persist_map(
	conn: &DatabaseConnection,
	map: &SyncMapV1,
	source: &str,
	job_id: Option<String>,
) -> Result<media_sync_map::Model, SyncMapError> {
	let cue_count =
		i32::try_from(map.cues.len()).map_err(|_| SyncMapError::CueCountOverflow)?;
	let map_json = serde_json::to_value(map)?;
	let txn = begin_write(conn).await?;
	let persisted = media_sync_map::Entity::insert(media_sync_map::ActiveModel {
		id: Set(uuid::Uuid::new_v4().to_string()),
		ebook_media_id: Set(map.text_media_id.clone()),
		audio_media_id: Set(map.audio_media_id.clone()),
		granularity: Set(granularity_name(map.provenance.granularity).to_owned()),
		generator: Set(map.provenance.implementation.clone()),
		generator_version: Set(map.provenance.version.clone()),
		algorithm: Set(Some(map.provenance.algorithm.clone())),
		model: Set(Some(map.provenance.model.clone())),
		text_digest: Set(map.provenance.text_digest.clone()),
		audio_manifest_digest: Set(map.provenance.audio_manifest_digest.clone()),
		cue_count: Set(cue_count),
		coverage: Set(None),
		map: Set(map_json),
		source: Set(source.to_owned()),
		job_id: Set(job_id),
		created_at: Set(Utc::now().fixed_offset()),
	})
	.on_conflict(
		OnConflict::columns([
			media_sync_map::Column::EbookMediaId,
			media_sync_map::Column::AudioMediaId,
			media_sync_map::Column::Granularity,
			media_sync_map::Column::TextDigest,
			media_sync_map::Column::AudioManifestDigest,
			media_sync_map::Column::Generator,
			media_sync_map::Column::GeneratorVersion,
		])
		.update_columns([
			media_sync_map::Column::Algorithm,
			media_sync_map::Column::Model,
			media_sync_map::Column::CueCount,
			media_sync_map::Column::Coverage,
			media_sync_map::Column::Map,
			media_sync_map::Column::Source,
			media_sync_map::Column::JobId,
		])
		.to_owned(),
	)
	.exec_with_returning(&txn)
	.await?;
	txn.commit().await?;
	Ok(persisted)
}

async fn strict_alignment_input<C: ConnectionTrait>(
	conn: &C,
	ebook_media_id: &str,
	audio_media_id: &str,
	granularity: AlignGranularity,
) -> Result<AlignInput, SyncMapError> {
	let ebook = media::Entity::find_by_id(ebook_media_id.to_owned())
		.one(conn)
		.await?
		.ok_or_else(|| SyncMapError::MediaNotFound {
			media_id: ebook_media_id.to_owned(),
		})?;
	let audio = media::Entity::find_by_id(audio_media_id.to_owned())
		.one(conn)
		.await?
		.ok_or_else(|| SyncMapError::MediaNotFound {
			media_id: audio_media_id.to_owned(),
		})?;
	let prepared = stump_media::read_aloud::prepare_epub(&ebook.path)?;
	let tracks = media_audio_track::Entity::find()
		.filter(media_audio_track::Column::MediaId.eq(audio_media_id))
		.order_by_asc(media_audio_track::Column::Index)
		.all(conn)
		.await?;
	if tracks.len() != 1 || !audio.extension.eq_ignore_ascii_case("m4b") {
		return Err(SyncMapError::UnsupportedAudio);
	}
	let audio_manifest_digest = strict_audio_manifest_digest_from_tracks(&tracks)?;
	Ok(AlignInput {
		text_media_id: ebook_media_id.to_owned(),
		text_digest: prepared.canonical_text_digest,
		audio_media_id: audio_media_id.to_owned(),
		audio_manifest_digest,
		algorithm: DEFAULT_ALIGNMENT_ALGORITHM.to_owned(),
		model: DEFAULT_ALIGNMENT_MODEL.to_owned(),
		model_revision: DEFAULT_ALIGNMENT_MODEL_REVISION.to_owned(),
		language: DEFAULT_ALIGNMENT_LANGUAGE.to_owned(),
		granularity,
		execution_provider: AlignExecutionProvider::Cpu,
		precision: AlignPrecision::Fp32,
		options: BTreeMap::new(),
	})
}

async fn strict_audio_manifest_digest(
	conn: &DatabaseConnection,
	audio: &media::Model,
) -> Result<String, SyncMapError> {
	let tracks = media_audio_track::Entity::find()
		.filter(media_audio_track::Column::MediaId.eq(&audio.id))
		.order_by_asc(media_audio_track::Column::Index)
		.all(conn)
		.await?;
	strict_audio_manifest_digest_from_tracks(&tracks)
}

fn strict_audio_manifest_digest_from_tracks(
	tracks: &[media_audio_track::Model],
) -> Result<String, SyncMapError> {
	let mut manifest = Vec::with_capacity(tracks.len());
	for track in tracks {
		let bytes =
			std::fs::read(&track.path).map_err(|source| SyncMapError::Digest {
				media_id: track.media_id.clone(),
				source,
			})?;
		let content_sha256 = sha256_hex_bytes(&bytes);
		if track.byte_size > 0 && track.byte_size != bytes.len() as i64 {
			return Err(SyncMapError::Digest {
				media_id: track.media_id.clone(),
				source: std::io::Error::new(
					std::io::ErrorKind::InvalidData,
					"audio track byte_size does not match source",
				),
			});
		}
		manifest.push(json!({
			"index": track.index,
			"duration_ms": track.duration_ms,
			"start_offset_ms": track.start_offset_ms,
			"byte_size": track.byte_size,
			"mime": track.mime,
			"content_sha256": content_sha256,
		}));
	}
	hex_sha256(&serde_json::to_vec(&manifest)?)
}

fn sha256_hex_bytes(bytes: &[u8]) -> String {
	let mut hasher = Sha256::new();
	hasher.update(bytes);
	hasher
		.finalize()
		.iter()
		.map(|byte| format!("{byte:02x}"))
		.collect()
}

fn validate_map_targets(
	prepared: &stump_media::read_aloud::PreparedEpub,
	map: &SyncMapV1,
) -> Result<(), SyncMapError> {
	let cues = map
		.cues
		.iter()
		.map(|cue| stump_media::read_aloud::RenderCue {
			spine_index: cue.text.spine_index,
			ordinal: cue.text.ordinal,
			element_id: cue.text.element_id.clone(),
			track_index: cue.audio.track_index,
			begin_ms: cue.audio.begin_ms,
			end_ms: cue.audio.end_ms,
		})
		.collect::<Vec<_>>();
	stump_media::read_aloud::validate_targets(prepared, &cues)?;
	Ok(())
}

async fn confirmed_pair_for_user<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	ebook_media_id: &str,
	audio_media_id: &str,
) -> Result<bool, DbErr> {
	if ebook_media_id == audio_media_id {
		return Ok(false);
	}
	let links = liseur_sync_media_link::Entity::find()
		.filter(
			liseur_sync_media_link::Column::MediaId
				.is_in([ebook_media_id.to_owned(), audio_media_id.to_owned()]),
		)
		.filter(liseur_sync_media_link::Column::UserId.eq(user_id))
		.filter(
			liseur_sync_media_link::Column::PairStatus
				.eq(PairStatus::Confirmed.to_string()),
		)
		.all(conn)
		.await?;
	let mut media_ids = BTreeMap::new();
	for link in links {
		media_ids
			.entry(link.work_id)
			.or_insert_with(BTreeMap::new)
			.insert(link.media_id, ());
	}
	Ok(media_ids
		.values()
		.any(|ids| ids.contains_key(ebook_media_id) && ids.contains_key(audio_media_id)))
}

fn job_input_matches(input: &Value, expected: &AlignInput) -> bool {
	serde_json::from_value::<AlignInput>(input.clone())
		.map(|actual| actual == *expected)
		.unwrap_or(false)
}

#[cfg(test)]
mod tests {
	use std::{collections::BTreeMap, sync::Arc};

	use chrono::Utc;
	use models::{
		domain::edition_pair::PairStatus,
		entity::{
			liseur_sync_media_link, liseur_sync_work, media, media_audio,
			media_audio_track, media_sync_map, worker_job,
		},
		shared::enums::{FileStatus, WorkerJobStatus},
	};
	use sea_orm::{
		ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DbBackend, DbConn,
		EntityTrait, Schema,
	};
	use stump_worker::{
		AlignExecutionProvider, AlignGranularity, AlignPrecision, AudioClip, SyncCue,
		SyncMapProvenance, SyncMapV1, TextFragment, WorkerJobs, ALIGN,
	};
	use tempfile::TempDir;
	use uuid::Uuid;

	use super::*;
	use ::tests::fake_data;

	async fn database() -> DbConn {
		let conn = ::tests::db::test_database().await;
		let schema = Schema::new(DbBackend::Sqlite);
		for statement in [
			schema.create_table_from_entity(liseur_sync_work::Entity),
			schema.create_table_from_entity(liseur_sync_media_link::Entity),
		] {
			conn.execute(conn.get_database_backend().build(&statement))
				.await
				.expect("create liseur sync table");
		}
		conn.execute_unprepared(
			"CREATE UNIQUE INDEX uq_media_sync_maps_identity
			 ON media_sync_maps (
			   ebook_media_id, audio_media_id, granularity, text_digest,
			   audio_manifest_digest, generator, generator_version
			 )",
		)
		.await
		.expect("create sync map unique index");
		conn
	}

	struct Pair {
		ebook: media::Model,
		audio: media::Model,
	}

	async fn pair(conn: &DbConn, status: PairStatus) -> Pair {
		let library = fake_data::Library::default().insert(conn).await;
		let series = fake_data::Series {
			library_id: Some(library.id.clone()),
			..Default::default()
		}
		.insert(conn)
		.await;
		let user = fake_data::User::new("owner").insert(conn).await;
		let ebook = fake_data::Media {
			series_id: series.id.clone(),
			id: Some("ebook".to_owned()),
			name: Some("Book".to_owned()),
			extension: Some("epub".to_owned()),
			..Default::default()
		}
		.insert(conn)
		.await;
		let audio = fake_data::Media {
			series_id: series.id,
			id: Some("audio".to_owned()),
			name: Some("Book".to_owned()),
			extension: Some("m4b".to_owned()),
			..Default::default()
		}
		.insert(conn)
		.await;

		media_audio::ActiveModel {
			media_id: Set(audio.id.clone()),
			duration_ms: Set(1_000),
			codec: Set("aac".to_owned()),
			chapter_source: Set(models::domain::audio::AudioChapterSource::Mp4Chpl),
			..Default::default()
		}
		.insert(conn)
		.await
		.expect("insert audio");
		media_audio_track::ActiveModel {
			id: Set("track".to_owned()),
			media_id: Set(audio.id.clone()),
			index: Set(0),
			path: Set("track.m4a".to_owned()),
			duration_ms: Set(1_000),
			start_offset_ms: Set(0),
			byte_size: Set(1),
			mime: Set("audio/mp4".to_owned()),
		}
		.insert(conn)
		.await
		.expect("insert track");

		let work_id = "work";
		liseur_sync_work::ActiveModel {
			id: Set(work_id.to_owned()),
			user_id: Set(user.id.clone()),
			title: Set("Book".to_owned()),
			author: Set("Author".to_owned()),
			pending: Set(false),
			created_at: Set(Utc::now().to_rfc3339()),
		}
		.insert(conn)
		.await
		.expect("insert work");
		for media_id in [&ebook.id, &audio.id] {
			liseur_sync_media_link::ActiveModel {
				id: Set(Uuid::new_v4().to_string()),
				user_id: Set(user.id.clone()),
				media_id: Set(media_id.clone()),
				work_id: Set(work_id.to_owned()),
				edition_sha: Set("sha".to_owned()),
				resolution_status: Set("verified".to_owned()),
				created_at: Set(Utc::now().to_rfc3339()),
				pair_status: Set(status.to_string()),
				pair_evidence: Set(None),
			}
			.insert(conn)
			.await
			.expect("insert pair link");
		}
		Pair { ebook, audio }
	}

	fn map(ebook_media_id: &str, audio_media_id: &str, end_ms: i64) -> SyncMapV1 {
		SyncMapV1 {
			schema: 1,
			text_media_id: ebook_media_id.to_owned(),
			audio_media_id: audio_media_id.to_owned(),
			provenance: SyncMapProvenance {
				text_digest: "a".repeat(64),
				audio_manifest_digest: "b".repeat(64),
				implementation: "importer".to_owned(),
				version: "1".to_owned(),
				algorithm: "ctc".to_owned(),
				model: "model".to_owned(),
				model_revision: "revision".to_owned(),
				language: "en".to_owned(),
				granularity: AlignGranularity::Sentence,
				execution_provider: AlignExecutionProvider::Cpu,
				precision: AlignPrecision::Fp32,
				options: BTreeMap::new(),
			},
			cues: vec![SyncCue {
				text: TextFragment {
					spine_index: 0,
					ordinal: 0,
					element_id: "s0".to_owned(),
				},
				audio: AudioClip {
					track_index: 0,
					begin_ms: 0,
					end_ms,
				},
				confidence: Some(1.0),
			}],
		}
	}

	#[tokio::test]
	async fn import_rejects_an_unconfirmed_pair() {
		let conn = database().await;
		let pair = pair(&conn, PairStatus::Suggested).await;
		let error = import_sync_map(
			&conn,
			&pair.ebook.id,
			&pair.audio.id,
			map(&pair.ebook.id, &pair.audio.id, 100),
		)
		.await
		.expect_err("suggested pair must be rejected");
		assert!(matches!(error, SyncMapError::PairNotConfirmed));
	}

	#[tokio::test]
	async fn import_rejects_a_cue_beyond_track_duration() {
		let conn = database().await;
		let pair = pair(&conn, PairStatus::Confirmed).await;
		let error = import_sync_map(
			&conn,
			&pair.ebook.id,
			&pair.audio.id,
			map(&pair.ebook.id, &pair.audio.id, 1_001),
		)
		.await
		.expect_err("cue beyond track must be rejected");
		assert!(matches!(
			error,
			SyncMapError::Validation(SyncMapValidationError::ClipOutOfBounds { .. })
		));
	}

	#[tokio::test]
	async fn import_upsert_is_idempotent_on_the_unique_identity() {
		let conn = database().await;
		let pair = pair(&conn, PairStatus::Confirmed).await;
		let first = import_sync_map(
			&conn,
			&pair.ebook.id,
			&pair.audio.id,
			map(&pair.ebook.id, &pair.audio.id, 100),
		)
		.await
		.expect("first import");
		let second = import_sync_map(
			&conn,
			&pair.ebook.id,
			&pair.audio.id,
			map(&pair.ebook.id, &pair.audio.id, 100),
		)
		.await
		.expect("second import");
		assert_eq!(first.id, second.id);
		assert_eq!(
			media_sync_map::Entity::find()
				.all(&conn)
				.await
				.unwrap()
				.len(),
			1
		);
	}

	async fn set_ebook_file(conn: &DbConn, ebook: &media::Model, dir: &TempDir) {
		let path = dir.path().join("book.epub");
		std::fs::write(&path, b"ebook bytes").expect("write ebook");
		let mut model: media::ActiveModel = ebook.clone().into();
		model.path = Set(path.to_string_lossy().into_owned());
		model.status = Set(FileStatus::Ready);
		model.update(conn).await.expect("update ebook path");
	}

	fn worker_jobs(conn: &Arc<DbConn>, dir: &TempDir) -> WorkerJobs {
		WorkerJobs::new(conn.clone(), dir.path().join("worker-output"))
	}

	#[tokio::test]
	async fn enqueue_reuses_an_active_identical_job() {
		let conn = Arc::new(database().await);
		let pair = pair(&conn, PairStatus::Confirmed).await;
		let dir = tempfile::tempdir().expect("tempdir");
		let input = serde_json::json!({
			"text_media_id": pair.ebook.id,
			"audio_media_id": pair.audio.id,
			"granularity": "sentence"
		});
		let active = worker_job::ActiveModel {
			id: Set("active".to_owned()),
			kind: Set(ALIGN.to_owned()),
			input: Set(input),
			requires: Set(json!({ "align": { "device": "cpu" } })),
			status: Set(WorkerJobStatus::Claimed),
			worker_id: Set(Some("worker".to_owned())),
			priority: Set(-10),
			progress: Set(0.0),
			progress_message: Set(None),
			result: Set(None),
			error: Set(None),
			created_at: Set(Utc::now().fixed_offset()),
			updated_at: Set(Utc::now().fixed_offset()),
			started_at: Set(None),
			finished_at: Set(None),
		}
		.insert(conn.as_ref())
		.await
		.expect("insert active job");
		let found = enqueue_alignment(
			&conn,
			&worker_jobs(&conn, &dir),
			&pair.ebook.id,
			&pair.audio.id,
			AlignGranularity::Sentence,
		)
		.await
		.expect("dedupe active job");
		assert_eq!(found.id, active.id);
		assert_eq!(
			worker_job::Entity::find()
				.all(conn.as_ref())
				.await
				.unwrap()
				.len(),
			1
		);
	}

	#[tokio::test]
	async fn enqueue_does_not_reuse_done_or_failed_jobs() {
		for terminal in [WorkerJobStatus::Done, WorkerJobStatus::Failed] {
			let conn = Arc::new(database().await);
			let pair = pair(&conn, PairStatus::Confirmed).await;
			let dir = tempfile::tempdir().expect("tempdir");
			set_ebook_file(&conn, &pair.ebook, &dir).await;
			let input = serde_json::json!({
				"text_media_id": pair.ebook.id,
				"audio_media_id": pair.audio.id,
				"granularity": "sentence"
			});
			worker_job::ActiveModel {
				id: Set(Uuid::new_v4().to_string()),
				kind: Set(ALIGN.to_owned()),
				input: Set(input),
				requires: Set(json!({ "align": { "device": "cpu" } })),
				status: Set(terminal),
				worker_id: Set(None),
				priority: Set(-10),
				progress: Set(0.0),
				progress_message: Set(None),
				result: Set(None),
				error: Set(None),
				created_at: Set(Utc::now().fixed_offset()),
				updated_at: Set(Utc::now().fixed_offset()),
				started_at: Set(None),
				finished_at: Set(Some(Utc::now().fixed_offset())),
			}
			.insert(conn.as_ref())
			.await
			.expect("insert terminal job");
			let created = enqueue_alignment(
				&conn,
				&worker_jobs(&conn, &dir),
				&pair.ebook.id,
				&pair.audio.id,
				AlignGranularity::Sentence,
			)
			.await
			.expect("enqueue after terminal job");
			assert_eq!(created.kind, ALIGN);
			assert_eq!(
				worker_job::Entity::find()
					.all(conn.as_ref())
					.await
					.unwrap()
					.len(),
				2
			);
		}
	}
}
