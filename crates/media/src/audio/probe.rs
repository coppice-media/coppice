//! The probe itself. See the module docs in [`super`] for why two libraries
//! are involved and what unit every time value is in.

use std::{
	fs::File,
	path::{Path, PathBuf},
};

use symphonia::core::{
	codecs::{audio::well_known::*, CodecParameters},
	formats::{probe::Hint, FormatOptions, FormatReader, TrackType},
	io::MediaSourceStream,
	meta::{
		ChapterGroup, ChapterGroupItem, MetadataOptions, MetadataRevision, StandardTag,
		StandardVisualKey, Visual,
	},
};

use crate::{
	audio::{ChapterSource, ProbedAudio, ProbedChapter, ProbedTrack},
	content_type::ContentType,
	error::FileError,
	PathUtils,
};

/// Probe an audiobook, dispatching on whether `path` is a folder.
pub fn probe(path: &Path) -> Result<ProbedAudio, FileError> {
	if path.is_dir() {
		probe_folder(path)
	} else {
		probe_file(path)
	}
}

/// Probe a single-container audiobook.
///
/// The result always has exactly one track, so a single-file and a folder
/// book are the same thing to every consumer downstream.
pub fn probe_file(path: &Path) -> Result<ProbedAudio, FileError> {
	let content_type = path.naive_content_type();
	if !content_type.is_audio() {
		return Err(FileError::UnsupportedFileType(
			path.to_string_lossy().to_string(),
		));
	}

	let track = probe_track(path, content_type)?;
	// MP4 carries chapters in atoms symphonia does not read; every other
	// container hands them to us through the demuxer.
	let (chapter_source, mut chapters) = if is_mp4(content_type) {
		mp4_chapters(path)
	} else {
		container_chapters(path, content_type)?
	};
	let tags = read_tags(path, content_type)?;

	clamp_chapters(&mut chapters, track.duration_ms);

	Ok(ProbedAudio {
		duration_ms: track.duration_ms,
		codec: track.codec.clone(),
		sample_rate: track.sample_rate,
		channels: track.channels,
		bitrate: track.bitrate,
		chapter_source: if chapters.is_empty() {
			ChapterSource::None
		} else {
			chapter_source
		},
		chapters,
		cover: tags.cover,
		title: tags.title,
		author: tags.author,
		narrator: tags.narrator,
		album: tags.album,
		description: tags.description,
		genre: tags.genre,
		year: tags.year,
		tracks: vec![track],
	})
}

/// Probe a folder audiobook: one file per part, in playback order.
///
/// Order comes from the `TRCK`/`trkn` tag when *every* file carries a
/// distinct one, because a publisher who numbered the files meant that order
/// even when the filenames sort differently. Otherwise it falls back to the
/// same natural filename sort the rest of the scanner uses, so `2` sorts
/// before `10`.
///
/// Chapters are the union of the files' own marks, shifted by each file's
/// start offset. A folder whose files carry no marks at all gets one
/// synthesized chapter per file — recorded as [`ChapterSource::PerTrack`] so
/// it can never be mistaken for a publisher's chapter list.
pub fn probe_folder(path: &Path) -> Result<ProbedAudio, FileError> {
	let files = audio_files_in(path)?;
	if files.is_empty() {
		return Err(FileError::UnsupportedFileType(
			path.to_string_lossy().to_string(),
		));
	}

	let mut parts = Vec::with_capacity(files.len());
	for file in files {
		let content_type = file.naive_content_type();
		let track = probe_track(&file, content_type)?;
		let (chapter_source, chapters) = if is_mp4(content_type) {
			mp4_chapters(&file)
		} else {
			container_chapters(&file, content_type)?
		};
		let tags = read_tags(&file, content_type)?;
		parts.push((track, chapter_source, chapters, tags));
	}

	// A publisher-assigned order wins over the filename order, but only when
	// it is complete and unambiguous: a partial or duplicated numbering would
	// silently shuffle the book.
	let numbers = parts
		.iter()
		.map(|(track, ..)| track.track_number)
		.collect::<Option<Vec<_>>>();
	if let Some(mut numbers) = numbers.clone() {
		numbers.sort_unstable();
		numbers.dedup();
		if numbers.len() == parts.len() {
			parts.sort_by_key(|(track, ..)| track.track_number);
		}
	}

	let mut duration_ms = 0_i64;
	let mut tracks = Vec::with_capacity(parts.len());
	let mut chapters = Vec::new();
	let mut chapter_source = ChapterSource::None;
	let mut cover = None;
	let mut book_tags = None;

	for (track, source, part_chapters, tags) in parts {
		let offset = duration_ms;
		duration_ms += track.duration_ms;

		if !part_chapters.is_empty() {
			if chapter_source == ChapterSource::None {
				chapter_source = source;
			}
			chapters.extend(part_chapters.into_iter().map(|chapter| ProbedChapter {
				title: chapter.title,
				start_ms: chapter.start_ms + offset,
				end_ms: chapter.end_ms.map(|end| end + offset),
			}));
		}

		if cover.is_none() {
			cover = tags.cover.clone();
		}
		if book_tags.is_none() {
			book_tags = Some(tags);
		}
		tracks.push(track);
	}

	if chapters.is_empty() {
		// No file carried a mark: the file list *is* the chapter list.
		chapter_source = ChapterSource::PerTrack;
		let mut offset = 0_i64;
		for track in &tracks {
			let end = offset + track.duration_ms;
			chapters.push(ProbedChapter {
				title: track.title.clone().or_else(|| {
					track
						.path
						.file_stem()
						.map(|stem| stem.to_string_lossy().to_string())
				}),
				start_ms: offset,
				end_ms: Some(end),
			});
			offset = end;
		}
	}

	clamp_chapters(&mut chapters, duration_ms);

	let codecs = tracks
		.iter()
		.map(|track| track.codec.as_str())
		.collect::<std::collections::BTreeSet<_>>();
	let codec = if codecs.len() == 1 {
		tracks[0].codec.clone()
	} else {
		"mixed".to_string()
	};

	let first = &tracks[0];
	let sample_rate = first.sample_rate;
	let channels = first.channels;
	// A folder book's bitrate is the whole publication's average, not the
	// first file's: parts are often encoded at different rates.
	let total_bytes = tracks.iter().map(|track| track.byte_size).sum::<i64>();
	let bitrate = derive_bitrate(total_bytes, duration_ms);
	let tags = book_tags.unwrap_or_default();

	Ok(ProbedAudio {
		duration_ms,
		codec,
		sample_rate,
		channels,
		bitrate,
		chapter_source,
		tracks,
		chapters,
		cover,
		title: tags.title,
		author: tags.author,
		narrator: tags.narrator,
		album: tags.album,
		description: tags.description,
		genre: tags.genre,
		year: tags.year,
	})
}

fn is_mp4(content_type: ContentType) -> bool {
	matches!(content_type, ContentType::M4B | ContentType::M4A)
}

/// Trim marks that fall outside the publication and give the last chapter an
/// end. A container is free to write a mark past its own duration; a client
/// that seeks there gets silence, so the probe is the place to fix it.
fn clamp_chapters(chapters: &mut Vec<ProbedChapter>, duration_ms: i64) {
	chapters.retain(|chapter| chapter.start_ms >= 0 && chapter.start_ms < duration_ms);
	chapters.sort_by_key(|chapter| chapter.start_ms);
	chapters.dedup_by_key(|chapter| chapter.start_ms);

	let count = chapters.len();
	for index in 0..count {
		let next_start = chapters.get(index + 1).map(|next| next.start_ms);
		let chapter = &mut chapters[index];
		let end = chapter
			.end_ms
			.filter(|end| *end > chapter.start_ms)
			.unwrap_or(next_start.unwrap_or(duration_ms));
		chapter.end_ms = Some(end.min(duration_ms).max(chapter.start_ms));
	}
}

/// Bits per second derived from size over duration, for containers that state
/// no bitrate of their own.
fn derive_bitrate(byte_size: i64, duration_ms: i64) -> Option<i32> {
	(duration_ms > 0).then(|| {
		i32::try_from(byte_size.saturating_mul(8_000) / duration_ms).unwrap_or(i32::MAX)
	})
}

fn open(
	path: &Path,
	content_type: ContentType,
) -> Result<Box<dyn FormatReader>, FileError> {
	let file = File::open(path)?;
	let mss = MediaSourceStream::new(Box::new(file), Default::default());

	let mut hint = Hint::new();
	if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
		hint.with_extension(extension);
	}
	hint.mime_type(&content_type.mime_type());

	symphonia::default::get_probe()
		.probe(
			&hint,
			mss,
			FormatOptions::default(),
			MetadataOptions::default(),
		)
		.map_err(|error| FileError::UnknownError(format!("{path:?}: {error}")))
}

/// Probe one file for its own facts. `duration_ms` is this file's duration,
/// not the publication's.
fn probe_track(path: &Path, content_type: ContentType) -> Result<ProbedTrack, FileError> {
	let byte_size = i64::try_from(std::fs::metadata(path)?.len()).unwrap_or(i64::MAX);
	let mut format = open(path, content_type)?;

	let duration_ms = duration_ms(format.as_ref());
	let (codec, sample_rate, channels) = codec_facts(format.as_ref());

	let revision = format.metadata().skip_to_latest().cloned();
	let tags = revision.as_ref().map(tags_of).unwrap_or_default();

	// MP4 states an average bitrate in `esds`; nothing else Stump reads does,
	// so the rest is derived from size over duration.
	let bitrate = if is_mp4(content_type) {
		mp4ameta::Tag::read_with_path(path, &mp4ameta::ReadConfig::NONE)
			.ok()
			.and_then(|tag| tag.info.avg_bitrate)
			.and_then(|bitrate| i32::try_from(bitrate).ok())
			.or_else(|| derive_bitrate(byte_size, duration_ms))
	} else {
		derive_bitrate(byte_size, duration_ms)
	};

	Ok(ProbedTrack {
		path: path.to_path_buf(),
		duration_ms,
		byte_size,
		mime: content_type.mime_type(),
		codec,
		sample_rate,
		channels,
		bitrate,
		title: tags.title,
		track_number: tags.track_number,
	})
}

/// The file's duration in milliseconds.
///
/// A container states the media duration in its own timebase units; a
/// container that does not is asked for its audio track's duration instead.
/// A file whose duration cannot be established is 0, never an error: a book
/// with one unreadable part should still be listed.
fn duration_ms(format: &dyn FormatReader) -> i64 {
	let media = format.media_info();
	let from_media = media
		.time_base
		.zip(media.duration)
		.and_then(|(time_base, duration)| time_base.calc_duration(duration));

	let from_track = || {
		let track = format.default_track(TrackType::Audio)?;
		let time_base = track.time_base?;
		let duration = track
			.duration
			.or_else(|| track.num_frames.map(Into::into))?;
		time_base.calc_duration(duration)
	};

	from_media
		.or_else(from_track)
		.map(|time| i64::try_from(time.as_millis()).unwrap_or(i64::MAX))
		.unwrap_or_default()
}

fn codec_facts(format: &dyn FormatReader) -> (String, Option<i32>, Option<i32>) {
	let Some(track) = format.default_track(TrackType::Audio) else {
		return ("unknown".to_string(), None, None);
	};
	let Some(CodecParameters::Audio(params)) = track.codec_params.as_ref() else {
		return ("unknown".to_string(), None, None);
	};

	let codec = match params.codec {
		CODEC_ID_AAC => "aac",
		CODEC_ID_MP3 => "mp3",
		CODEC_ID_MP2 => "mp2",
		CODEC_ID_MP1 => "mp1",
		CODEC_ID_OPUS => "opus",
		CODEC_ID_VORBIS => "vorbis",
		CODEC_ID_FLAC => "flac",
		CODEC_ID_ALAC => "alac",
		_ => "unknown",
	};

	(
		codec.to_string(),
		params.sample_rate.and_then(|rate| i32::try_from(rate).ok()),
		params
			.channels
			.as_ref()
			.map(|channels| channels.count())
			.and_then(|count| i32::try_from(count).ok()),
	)
}

/// Chapters read by the demuxer: ID3v2 `CHAP`/`CTOC` for MP3, `CHAPTERxxx`
/// Vorbis comments for Ogg/Opus/FLAC.
fn container_chapters(
	path: &Path,
	content_type: ContentType,
) -> Result<(ChapterSource, Vec<ProbedChapter>), FileError> {
	let format = open(path, content_type)?;
	let Some(group) = format.chapters() else {
		return Ok((ChapterSource::None, Vec::new()));
	};

	let mut chapters = Vec::new();
	flatten_chapters(group, &mut chapters);
	if chapters.is_empty() {
		return Ok((ChapterSource::None, Vec::new()));
	}

	let source = match content_type {
		ContentType::MP3 => ChapterSource::Id3Chap,
		ContentType::OPUS | ContentType::OGG | ContentType::FLAC => {
			ChapterSource::VorbisComment
		},
		// Unreachable for the types `is_audio` accepts; a new container
		// format must name its own mechanism rather than inherit one.
		_ => ChapterSource::None,
	};
	Ok((source, chapters))
}

/// A `ChapterGroup` is a tree (a container may nest chapter groups); a
/// reading position is flat, so the tree is flattened in document order and
/// re-sorted by start time afterwards.
fn flatten_chapters(group: &ChapterGroup, into: &mut Vec<ProbedChapter>) {
	for item in &group.items {
		match item {
			ChapterGroupItem::Group(group) => flatten_chapters(group, into),
			ChapterGroupItem::Chapter(chapter) => into.push(ProbedChapter {
				// ID3v2 repurposes `TIT2` and Vorbis uses `CHAPTERxxxNAME`;
				// symphonia normalizes both to `ChapterTitle`. `TrackTitle`
				// is the fallback for a container that writes the chapter
				// name as a plain track title.
				title: chapter
					.tags
					.iter()
					.find_map(|tag| match tag.std.as_ref() {
						Some(StandardTag::ChapterTitle(title))
						| Some(StandardTag::TrackTitle(title)) => Some(title.to_string()),
						_ => None,
					})
					.filter(|title| !title.is_empty()),
				start_ms: i64::try_from(chapter.start_time.as_millis())
					.unwrap_or_default(),
				end_ms: chapter
					.end_time
					.and_then(|end| i64::try_from(end.as_millis()).ok()),
			}),
		}
	}
}

/// MP4 chapters, and which of the two MP4 mechanisms carried them.
///
/// A container is free to write both a `chpl` atom and a chapter track; the
/// atom wins because that is what every player that reads only one of them
/// reads, and [`mp4ameta::Userdata::chapters`] resolves the same way. A file
/// that cannot be parsed at all yields no chapters rather than an error: the
/// duration and codec came from symphonia and the book is still listable.
fn mp4_chapters(path: &Path) -> (ChapterSource, Vec<ProbedChapter>) {
	let config = mp4ameta::ReadConfig {
		read_chapter_list: true,
		read_chapter_track: true,
		..mp4ameta::ReadConfig::NONE
	};
	let Ok(tag) = mp4ameta::Tag::read_with_path(path, &config) else {
		return (ChapterSource::None, Vec::new());
	};

	let (source, marks) = if !tag.chapter_list().is_empty() {
		(ChapterSource::Mp4Chpl, tag.chapter_list())
	} else if !tag.chapter_track().is_empty() {
		(ChapterSource::Mp4ChapterTrack, tag.chapter_track())
	} else {
		return (ChapterSource::None, Vec::new());
	};

	let chapters = marks
		.iter()
		.map(|chapter| ProbedChapter {
			title: (!chapter.title.is_empty()).then(|| chapter.title.clone()),
			start_ms: i64::try_from(chapter.start.as_millis()).unwrap_or_default(),
			end_ms: None,
		})
		.collect();

	(source, chapters)
}

/// The tags and cover of one file.
#[derive(Clone, Debug, Default)]
struct AudioTags {
	title: Option<String>,
	author: Option<String>,
	narrator: Option<String>,
	album: Option<String>,
	description: Option<String>,
	genre: Option<String>,
	year: Option<i32>,
	track_number: Option<u32>,
	cover: Option<(ContentType, Vec<u8>)>,
}

fn read_tags(path: &Path, content_type: ContentType) -> Result<AudioTags, FileError> {
	let mut format = open(path, content_type)?;
	let revision = format.metadata().skip_to_latest().cloned();
	let mut tags = revision.as_ref().map(tags_of).unwrap_or_default();

	// symphonia's isomp4 reader surfaces `ilst` text tags but not `covr`, so
	// an MP4 cover comes from mp4ameta.
	if tags.cover.is_none() && is_mp4(content_type) {
		let config = mp4ameta::ReadConfig {
			read_meta_items: true,
			read_image_data: true,
			..mp4ameta::ReadConfig::NONE
		};
		if let Ok(tag) = mp4ameta::Tag::read_with_path(path, &config) {
			tags.cover = tag.artwork().and_then(|artwork| {
				let content_type = match artwork.fmt {
					mp4ameta::ImgFmt::Jpeg => ContentType::JPEG,
					mp4ameta::ImgFmt::Png => ContentType::PNG,
					// Stump's thumbnail pipeline decodes GIF/JPEG/PNG/WebP
					// only; a BMP cover would fail every consumer, so it is
					// reported as no cover rather than as an unusable one.
					mp4ameta::ImgFmt::Bmp => return None,
				};
				Some((content_type, artwork.data.to_vec()))
			});
			tags.title = tags.title.or_else(|| tag.title().map(str::to_string));
			tags.author = tags.author.or_else(|| tag.artist().map(str::to_string));
			tags.narrator = tags.narrator.or_else(|| tag.composer().map(str::to_string));
			tags.album = tags.album.or_else(|| tag.album().map(str::to_string));
			tags.description = tags
				.description
				.or_else(|| tag.description().map(str::to_string));
			tags.genre = tags.genre.or_else(|| tag.genre().map(str::to_string));
			tags.track_number =
				tags.track_number.or_else(|| tag.track().0.map(u32::from));
		}
	}

	Ok(tags)
}

fn tags_of(revision: &MetadataRevision) -> AudioTags {
	let mut tags = AudioTags::default();

	for tag in &revision.media.tags {
		match tag.std.as_ref() {
			Some(StandardTag::TrackTitle(value)) => {
				tags.title.get_or_insert_with(|| value.to_string());
			},
			Some(StandardTag::Artist(value)) | Some(StandardTag::AlbumArtist(value)) => {
				tags.author.get_or_insert_with(|| value.to_string());
			},
			// A publisher writes the reader into `narrator` when the format
			// has one and into `composer` otherwise; both mean the reader for
			// an audiobook.
			Some(StandardTag::Narrator(value)) | Some(StandardTag::Composer(value)) => {
				tags.narrator.get_or_insert_with(|| value.to_string());
			},
			Some(StandardTag::Album(value)) => {
				tags.album.get_or_insert_with(|| value.to_string());
			},
			Some(StandardTag::Description(value)) | Some(StandardTag::Comment(value)) => {
				tags.description.get_or_insert_with(|| value.to_string());
			},
			Some(StandardTag::Genre(value)) => {
				tags.genre.get_or_insert_with(|| value.to_string());
			},
			Some(StandardTag::TrackNumber(value)) => {
				let value = u32::try_from(*value).ok();
				if tags.track_number.is_none() {
					tags.track_number = value;
				}
			},
			Some(StandardTag::ReleaseYear(value))
			| Some(StandardTag::RecordingYear(value))
			| Some(StandardTag::OriginalReleaseYear(value)) => {
				tags.year.get_or_insert(i32::from(*value));
			},
			Some(StandardTag::ReleaseDate(value))
			| Some(StandardTag::RecordingDate(value)) => {
				if let Some(year) = year_of(value) {
					tags.year.get_or_insert(year);
				}
			},
			_ => {},
		}
	}

	tags.cover = cover_of(&revision.media.visuals);
	tags
}

/// The leading four digits of an ISO-8601-ish date string.
fn year_of(value: &str) -> Option<i32> {
	let year = value.get(..4)?;
	year.chars().all(|c| c.is_ascii_digit()).then(|| ())?;
	year.parse().ok()
}

/// The front cover, or the first visual when nothing declares itself the
/// front cover. A book's back cover or a band logo is a worse thumbnail than
/// nothing only in theory: a listener recognizes any of them.
fn cover_of(visuals: &[Visual]) -> Option<(ContentType, Vec<u8>)> {
	let visual = visuals
		.iter()
		.find(|visual| visual.usage == Some(StandardVisualKey::FrontCover))
		.or_else(|| visuals.first())?;

	let content_type = visual
		.media_type
		.as_deref()
		.map(ContentType::from)
		.filter(|content_type| content_type.is_image())
		.or_else(|| {
			let inferred = ContentType::from_bytes(&visual.data);
			inferred.is_image().then_some(inferred)
		})?;

	Some((content_type, visual.data.to_vec()))
}

/// Every audio file directly under `path`, in natural filename order. The
/// scanner and the `audio-*` tools use it to decide what a folder book holds
/// without probing every file first.
pub fn audio_files_in(path: &Path) -> Result<Vec<PathBuf>, FileError> {
	let mut files = Vec::new();
	for entry in std::fs::read_dir(path)?.filter_map(Result::ok) {
		let entry = entry.path();
		if entry.is_file() && !entry.is_hidden_file() && entry.is_audio() {
			files.push(entry);
		}
	}
	alphanumeric_sort::sort_path_slice(&mut files);
	Ok(files)
}
