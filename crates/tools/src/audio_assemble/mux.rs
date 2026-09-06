//! The pure-Rust half of `audio-assemble`: turning several AAC-in-MP4 parts
//! into one seekable, faststart M4B without decoding a single sample.
//!
//! An audiobook that arrives as a folder of `.m4a`/`.m4b` parts already holds
//! exactly the bytes the canonical container needs. Decoding and re-encoding it
//! would cost an hour of CPU and a generation of quality to produce audio that
//! is, sample for sample, what was already on disk — so when every part
//! carries the same AAC stream shape, the parts are *remuxed*: the coded
//! samples are copied verbatim into one new container and only the sample
//! tables are rebuilt.
//!
//! # The three passes, and why there are three
//!
//! 1. [`remux`] writes `ftyp`, the media data, and a `moov` describing it. The
//!    `mp4` crate streams samples into `mdat` and can only finish `moov`
//!    afterwards, because a sample table is not known until the last sample is
//!    written.
//! 2. [`write_tags_and_chapters`] adds the iTunes metadata item list, the Nero
//!    `chpl` list, the QuickTime chapter track and the cover. It runs *before*
//!    the faststart pass on purpose: with `moov` still last, growing it moves
//!    no media data, so no chunk offset has to be rewritten and the riskiest
//!    edit is also the cheapest one.
//! 3. [`faststart`] moves the finished `moov` in front of the media data and
//!    adds the resulting shift to every chunk offset. This is the pass that
//!    makes the file streamable: a player that has to read to the end of a
//!    700 MB file to find the sample table cannot start playing until it has
//!    downloaded all of it, which is the difference between a book that starts
//!    instantly over HTTP and one that does not.
//!
//! Doing 2 before 3 is not an optimisation, it is the correctness argument:
//! each pass either moves media data or grows `moov`, never both.

use std::{
	collections::BTreeMap,
	fs::File,
	io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
	path::{Path, PathBuf},
	time::Duration,
};

use mp4::{
	AacConfig, AudioObjectType, Bytes, ChannelConfig, MediaConfig, Mp4Config, Mp4Sample,
	Mp4Writer, SampleFreqIndex, TrackConfig, TrackType,
};
use stump_media::ContentType;
use symphonia::core::{
	codecs::{audio::well_known::CODEC_ID_AAC, CodecParameters},
	formats::{probe::Hint, FormatOptions, FormatReader, TrackType as StreamType},
	io::MediaSourceStream,
	meta::MetadataOptions,
};

use crate::{util::write_atomic, ToolError, ToolResult};

/// The movie timescale of everything this module writes.
///
/// Milliseconds, which is the unit every audio value in Stump is expressed in
/// — `media_audio.duration_ms`, `reading_heads.position_ms`, a `chpl` mark —
/// so no time value crossing this module is ever rescaled.
const MOVIE_TIMESCALE: u32 = 1000;

/// Header of a box whose size fits in 32 bits: `size` + `type`.
const BOX_HEADER: u64 = 8;
/// `size == 1` means the real size follows the type as a `u64`.
const LARGE_SIZE_HEADER: u64 = 16;

/// Boxes [`faststart`] descends into looking for a chunk offset table. This is
/// the whole path to an `stco`/`co64` in a non-fragmented file, and nothing
/// else is opened: a metadata box that happened to contain those four bytes as
/// payload must not be mistaken for a table.
const CONTAINERS: [&[u8; 4]; 5] = [b"moov", b"trak", b"mdia", b"minf", b"stbl"];

/// The stream shape one AAC track has, and every part of a remuxable book must
/// agree on.
///
/// Sample rate and channel configuration are in the decoder configuration of
/// the output's sample description, so two parts that disagree cannot share
/// one track: a player would decode the second one at the first one's rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AacShape {
	/// The media timescale, which for AAC is the sample rate.
	pub timescale: u32,
	pub freq_index: SampleFreqIndex,
	pub chan_conf: ChannelConfig,
	pub profile: AudioObjectType,
	/// Highest advertised bitrate across the parts, for the output's `esds`.
	pub bitrate: u32,
}

/// The stream shape of one part's AAC track, or `None` when the part cannot be
/// remuxed at all.
///
/// `Ok(None)` is the ordinary answer for an MP3, an Opus file, an MP4 whose
/// AAC uses a sample rate or object type the canonical container's sample
/// description cannot express — and is not an error: the caller falls back to
/// the transcode path.
///
/// Demuxing goes through [`symphonia`] rather than the `mp4` crate on purpose.
/// Every M4B ffmpeg has ever produced with a cover carries the artwork as an
/// `attached_pic` video track, and the `mp4` crate refuses the whole file
/// because it cannot parse a PNG sample entry — which would send a book that
/// is *already* AAC down the lossy transcode path. symphonia demuxes the audio
/// track and ignores the rest, which is exactly the question being asked.
pub fn aac_shape(path: &Path) -> ToolResult<Option<AacShape>> {
	let format = match open(path) {
		Ok(format) => format,
		// An unreadable part is not remuxable; the transcode path and the
		// probe both report the real problem.
		Err(_) => return Ok(None),
	};
	let Some((_, params)) = audio_stream(format.as_ref()) else {
		return Ok(None);
	};
	if params.codec != CODEC_ID_AAC {
		return Ok(None);
	}

	// The `esds` decoder-specific info, which is the only authoritative
	// statement of what the samples are: object type, sample rate index and
	// channel configuration, in that order and bit-packed.
	let Some(config) = params.extra_data.as_deref().and_then(audio_specific_config)
	else {
		return Ok(None);
	};
	let (Ok(profile), Ok(freq_index), Ok(chan_conf)) = (
		AudioObjectType::try_from(config.object_type),
		SampleFreqIndex::try_from(config.freq_index),
		ChannelConfig::try_from(config.channels),
	) else {
		// An escape-coded object type or an explicit sample rate cannot be
		// written into an `AacConfig`, so the part is transcode-only rather
		// than silently mis-described.
		return Ok(None);
	};

	Ok(Some(AacShape {
		// For AAC in MP4 the media timescale is the sample rate, which is what
		// makes a sample's duration a frame count.
		timescale: params.sample_rate.unwrap_or_default(),
		freq_index,
		chan_conf,
		profile,
		bitrate: advertised_bitrate(path),
	}))
}

/// The `esds` average bitrate the part advertises, or zero.
///
/// Purely advisory: it lands in the output's `esds` so a player can show a
/// bitrate. Nothing depends on it — Stump's own probe derives a bitrate from
/// size over duration when the container states none — so a part that keeps
/// quiet about it simply carries the value forward.
fn advertised_bitrate(path: &Path) -> u32 {
	mp4ameta::Tag::read_with_path(path, &mp4ameta::ReadConfig::NONE)
		.ok()
		.and_then(|tag| tag.info.avg_bitrate)
		.unwrap_or_default()
}

/// Open a part with symphonia, hinted by its extension.
fn open(path: &Path) -> ToolResult<Box<dyn FormatReader>> {
	let file = File::open(path)?;
	let stream = MediaSourceStream::new(Box::new(file), Default::default());
	let mut hint = Hint::new();
	if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
		hint.with_extension(extension);
	}

	symphonia::default::get_probe()
		.probe(
			&hint,
			stream,
			FormatOptions::default(),
			MetadataOptions::default(),
		)
		.map_err(|error| ToolError::Invalid(format!("{}: {error}", path.display())))
}

/// The default audio stream's id and codec parameters.
fn audio_stream(
	format: &dyn FormatReader,
) -> Option<(u32, &symphonia::core::codecs::audio::AudioCodecParameters)> {
	let track = format.default_track(StreamType::Audio)?;
	let CodecParameters::Audio(params) = track.codec_params.as_ref()? else {
		return None;
	};
	Some((track.id, params))
}

/// The three fields of an `AudioSpecificConfig` the canonical container needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AudioSpecificConfig {
	object_type: u8,
	freq_index: u8,
	channels: u8,
}

/// Unpack the leading bits of an ISO 14496-3 `AudioSpecificConfig`:
/// 5 bits of object type, 4 bits of sampling frequency index, 4 bits of
/// channel configuration.
///
/// The two escape values are recognised and *rejected* rather than
/// misinterpreted: object type 31 means "six more bits follow" and frequency
/// index 15 means "an explicit 24-bit rate follows", and neither can be
/// expressed in the `esds` this module writes. Reading them as the literal 31
/// and 15 would describe the stream wrongly, which is the one failure that
/// produces a file that plays at the wrong pitch.
fn audio_specific_config(asc: &[u8]) -> Option<AudioSpecificConfig> {
	let mut bits = Bits::new(asc);
	let object_type = bits.take(5)? as u8;
	if object_type == 31 {
		return None;
	}
	let freq_index = bits.take(4)? as u8;
	if freq_index == 15 {
		return None;
	}
	let channels = bits.take(4)? as u8;
	Some(AudioSpecificConfig {
		object_type,
		freq_index,
		channels,
	})
}

/// A big-endian bit cursor over a byte slice.
struct Bits<'a> {
	bytes: &'a [u8],
	offset: usize,
}

impl<'a> Bits<'a> {
	fn new(bytes: &'a [u8]) -> Self {
		Self { bytes, offset: 0 }
	}

	/// The next `count` bits, or `None` when the slice is too short.
	fn take(&mut self, count: usize) -> Option<u32> {
		if self.offset + count > self.bytes.len() * 8 {
			return None;
		}
		let mut value = 0u32;
		for _ in 0..count {
			let byte = self.bytes[self.offset / 8];
			let bit = (byte >> (7 - (self.offset % 8))) & 1;
			value = (value << 1) | u32::from(bit);
			self.offset += 1;
		}
		Some(value)
	}
}

/// The one shape every part shares, or `None` when they disagree.
///
/// The bitrate is deliberately *not* part of the agreement: it is advisory
/// metadata in `esds`, not a decoding parameter, so parts mastered at 62 and
/// 64 kbps remux fine and the output advertises the larger.
pub fn common_shape(parts: &[AacShape]) -> Option<AacShape> {
	let mut shape = *parts.first()?;
	for part in parts {
		if (
			part.timescale,
			part.freq_index,
			part.chan_conf,
			part.profile,
		) != (
			shape.timescale,
			shape.freq_index,
			shape.chan_conf,
			shape.profile,
		) {
			return None;
		}
		shape.bitrate = shape.bitrate.max(part.bitrate);
	}
	Some(shape)
}

/// Copy every coded sample of `parts` into one new MP4 at `output`.
///
/// `parts` is the file list in playback order. Each part's *default audio*
/// stream is demuxed and its packets — coded AAC frames, byte for byte what
/// was on disk — are appended to one track, carrying their durations across
/// verbatim. Packets belonging to any other stream (a cover image, a chapter
/// text track) are dropped, which is the whole reason the output is one clean
/// audio track rather than a copy of the first part's structure.
///
/// No timestamp is rebased: the output's sample table is built from the
/// durations, so appending part *n+1*'s packets after part *n*'s is already
/// the right answer.
///
/// Returns the number of samples written. Zero is impossible for a real book
/// and is reported as an error rather than as an empty container.
pub fn remux(parts: &[PathBuf], output: &Path, shape: &AacShape) -> ToolResult<u64> {
	let config = Mp4Config {
		// What Apple, ffmpeg and every audiobook player agree an M4B is: an
		// `M4A ` audio MP4. `M4B ` is not a registered brand, and writing one
		// would only make the file look unfamiliar to a strict parser.
		major_brand: "M4A ".parse().map_err(brand_error)?,
		minor_version: 512,
		compatible_brands: vec![
			"M4A ".parse().map_err(brand_error)?,
			"mp42".parse().map_err(brand_error)?,
			"isom".parse().map_err(brand_error)?,
		],
		timescale: MOVIE_TIMESCALE,
	};

	let written = write_atomic(output, |file| {
		let mut writer = Mp4Writer::write_start(BufWriter::new(file), &config)?;
		writer.add_track(&TrackConfig {
			track_type: TrackType::Audio,
			timescale: shape.timescale,
			// The parts do not agree on a language often enough to be worth
			// reading, and `und` is what every muxer writes when it does not
			// know. The iTunes tags carry the facts a listener sees.
			language: "und".to_string(),
			media_conf: MediaConfig::AacConfig(AacConfig {
				bitrate: shape.bitrate,
				profile: shape.profile,
				freq_index: shape.freq_index,
				chan_conf: shape.chan_conf,
			}),
		})?;

		let mut samples = 0u64;
		for path in parts {
			let mut format = open(path)?;
			let stream_id =
				audio_stream(format.as_ref())
					.map(|(id, _)| id)
					.ok_or_else(|| {
						ToolError::Invalid(format!(
							"{} has no audio stream to copy",
							path.display()
						))
					})?;

			loop {
				let packet = format.next_packet().map_err(|error| {
					ToolError::Invalid(format!("{}: {error}", path.display()))
				})?;
				let Some(packet) = packet else { break };
				if packet.track_id != stream_id {
					continue;
				}
				// Every AAC frame is independently decodable, so every sample
				// is a sync sample and none carries a composition offset.
				let sample = Mp4Sample {
					start_time: packet.pts.get().max(0) as u64,
					duration: u32::try_from(packet.dur.get()).unwrap_or(u32::MAX),
					rendering_offset: 0,
					is_sync: true,
					bytes: Bytes::copy_from_slice(&packet.data),
				};
				// Track 1 is the only track this writer has; `add_track`
				// assigned it.
				writer.write_sample(1, &sample)?;
				samples += 1;
			}
		}

		writer.write_end()?;
		// The `BufWriter` wraps the temp file `write_atomic` will persist, so
		// its buffer has to reach the file before this closure returns.
		writer.into_writer().flush()?;
		Ok(samples)
	})?;

	if written == 0 {
		return Err(ToolError::Invalid(
			"the parts hold no audio samples".to_string(),
		));
	}
	repair_sl_config(output)?;
	Ok(written)
}

/// Correct the `SLConfigDescriptor` the muxer writes into `esds`.
///
/// `mp4` 0.14 emits the three bytes `06 00 00`: tag 6, **declared length 0**,
/// then one byte of payload it did not account for, and that payload is
/// `predefined = 0` ("custom", which obliges a reader to expect a full SL
/// packet header specification that is not there). ISO 14496-1 requires the
/// descriptor to carry `predefined`, and the MP4 default is `2`. So the
/// correct three bytes are `06 01 02`.
///
/// ffmpeg shrugs at the malformed version; [`symphonia`] does not, and
/// symphonia *is* `stump_media::audio::probe` — so without this repair
/// `audio-assemble` would write books Stump itself could not read. The fix is
/// exactly three bytes over three bytes, so no box, descriptor or chunk offset
/// changes size, which is why it can be done in place.
///
/// Returns whether anything was repaired, so the no-op case is honest if the
/// muxer is ever fixed upstream.
fn repair_sl_config(path: &Path) -> ToolResult<bool> {
	let boxes = {
		let mut file = BufReader::new(File::open(path)?);
		read_top_level(&mut file)?
	};
	let Some(moov) = boxes.iter().find(|item| &item.fourcc == b"moov") else {
		return Ok(false);
	};
	let bytes = read_box(path, *moov)?;

	let Some(esds) = find_esds(&bytes, 0, bytes.len())? else {
		return Ok(false);
	};
	let Some(at) = sl_config_at(&bytes[esds.0..esds.1]) else {
		return Ok(false);
	};

	// `at` indexes the length byte of the descriptor, inside the `esds`
	// payload, inside `moov`, inside the file.
	let offset = moov.offset + (esds.0 + at) as u64;
	let mut file = File::options().write(true).open(path)?;
	file.seek(SeekFrom::Start(offset))?;
	file.write_all(&[1, 2])?;
	file.sync_all()?;
	Ok(true)
}

/// The `esds` payload range inside a `moov` buffer, following the one path an
/// audio sample description takes: `trak/mdia/minf/stbl/stsd/mp4a/esds`.
fn find_esds(
	moov: &[u8],
	start: usize,
	end: usize,
) -> ToolResult<Option<(usize, usize)>> {
	/// `stsd` carries version/flags and an entry count before its entries.
	const STSD_PREAMBLE: usize = 8;
	/// A version-0 `AudioSampleEntry` payload before its child boxes: 6
	/// reserved bytes and a data reference index, then the 20-byte sound
	/// description. QuickTime's version 1 and 2 sound entries append 16 and 36
	/// further bytes, and the version is the first field of the description —
	/// so it is read rather than assumed, because guessing it puts the child
	/// walk in the middle of a field.
	const AUDIO_SAMPLE_ENTRY: usize = 28;

	let mut offset = start;
	while offset + BOX_HEADER as usize <= end {
		let (size, header, fourcc) = box_at(moov, offset, end)?;
		let content = offset + header;
		let child_end = offset + size;

		if &fourcc == b"esds" {
			// version/flags, then the descriptor chain.
			return Ok(Some((content + 4, child_end)));
		}
		let descend = if CONTAINERS.contains(&&fourcc) {
			Some(content)
		} else if &fourcc == b"stsd" {
			Some(content + STSD_PREAMBLE)
		} else if &fourcc == b"mp4a" {
			let version_at = content + 8;
			let version = u16::from_be_bytes([
				*moov.get(version_at).unwrap_or(&0),
				*moov.get(version_at + 1).unwrap_or(&0),
			]);
			let extra = match version {
				1 => 16,
				2 => 36,
				_ => 0,
			};
			Some(content + AUDIO_SAMPLE_ENTRY + extra)
		} else {
			None
		};
		if let Some(child) = descend {
			if child <= child_end {
				if let Some(found) = find_esds(moov, child, child_end)? {
					return Ok(Some(found));
				}
			}
		}

		offset += size;
	}
	Ok(None)
}

/// The index, inside an `esds` descriptor chain, of the length byte of a
/// zero-length `SLConfigDescriptor` — or `None` when there is nothing to fix.
fn sl_config_at(chain: &[u8]) -> Option<usize> {
	/// `ES_DescrTag`.
	const ES: u8 = 3;
	/// `SLConfigDescrTag`.
	const SL: u8 = 6;

	let (tag, len, mut at) = read_descriptor(chain, 0)?;
	if tag != ES {
		return None;
	}
	let es_end = (at + len).min(chain.len());

	// `ES_ID` and the flag byte; the three optional fields the flags select
	// are never written by this muxer, and a file that does carry them is one
	// this repair declines rather than mis-indexes.
	let flags = *chain.get(at + 2)?;
	if flags & 0xe0 != 0 {
		return None;
	}
	at += 3;

	while at + 2 <= es_end {
		let length_at = at + 1;
		let (tag, len, payload_at) = read_descriptor(chain, at)?;
		if tag == SL && len == 0 && payload_at < es_end {
			return Some(length_at);
		}
		at = payload_at + len;
	}
	None
}

/// One descriptor header: its tag, its declared payload length, and the index
/// its payload starts at.
///
/// Lengths are the MPEG-4 expandable encoding: seven bits per byte, high bit
/// set on every byte but the last.
fn read_descriptor(chain: &[u8], at: usize) -> Option<(u8, usize, usize)> {
	let tag = *chain.get(at)?;
	let mut length = 0usize;
	let mut index = at + 1;
	for _ in 0..4 {
		let byte = *chain.get(index)?;
		index += 1;
		length = (length << 7) | usize::from(byte & 0x7f);
		if byte & 0x80 == 0 {
			break;
		}
	}
	Some((tag, length, index))
}

/// The size, header length and type of the box at `offset`.
fn box_at(
	bytes: &[u8],
	offset: usize,
	end: usize,
) -> ToolResult<(usize, usize, [u8; 4])> {
	let declared = u32::from_be_bytes([
		bytes[offset],
		bytes[offset + 1],
		bytes[offset + 2],
		bytes[offset + 3],
	]);
	let fourcc: [u8; 4] = [
		bytes[offset + 4],
		bytes[offset + 5],
		bytes[offset + 6],
		bytes[offset + 7],
	];
	let (size, header) = match declared {
		1 => {
			if offset + LARGE_SIZE_HEADER as usize > end {
				return Err(ToolError::Invalid(format!(
					"box {} declares a 64-bit size it does not hold",
					fourcc_name(&fourcc)
				)));
			}
			let mut large = [0u8; 8];
			large.copy_from_slice(&bytes[offset + 8..offset + 16]);
			(
				usize::try_from(u64::from_be_bytes(large)).unwrap_or(usize::MAX),
				LARGE_SIZE_HEADER as usize,
			)
		},
		0 => (end - offset, BOX_HEADER as usize),
		other => (other as usize, BOX_HEADER as usize),
	};
	if size < header || offset + size > end {
		return Err(ToolError::Invalid(format!(
			"box {} at {offset} declares an impossible size {size}",
			fourcc_name(&fourcc)
		)));
	}
	Ok((size, header, fourcc))
}

fn brand_error(_: mp4::Error) -> ToolError {
	// Every brand here is a four-byte literal in this file.
	ToolError::Invalid("invalid MP4 brand literal".to_string())
}

/// The iTunes metadata item list an assembled book carries.
///
/// The atom each field lands in is named because that mapping *is* the
/// contract every audiobook player reads: `©wrt` (composer) is where every
/// publisher, Audiobookshelf and Plex all look for the narrator, and `©alb`
/// rather than `©nam` is what a player groups a series by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tags {
	/// `©nam`
	pub title: Option<String>,
	/// `©ART`
	pub author: Option<String>,
	/// `©wrt` — the narrator.
	pub narrator: Option<String>,
	/// `©alb`
	pub album: Option<String>,
	/// `©gen`
	pub genre: Option<String>,
	/// `©day`
	pub year: Option<String>,
	/// `©des`
	pub description: Option<String>,
	/// `covr`
	pub cover: Option<(ContentType, Vec<u8>)>,
}

/// One chapter mark to write, in publication milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mark {
	pub title: String,
	pub start_ms: i64,
}

/// Write the metadata item list, the `chpl` list, the QuickTime chapter track
/// and the cover into `target`.
///
/// Both chapter mechanisms are written, which is the one place this differs
/// from [`crate::audio_chapters`]: that tool edits a container a publisher
/// shipped and refuses to perform structural surgery on it, while this one is
/// *building* the container and can afford the track. Players split cleanly
/// between the two mechanisms — Apple's own read the track, most Android
/// players read `chpl` — so a book that carries only one is a book with no
/// chapters for half its audience.
pub fn write_tags_and_chapters(
	target: &Path,
	tags: &Tags,
	marks: &[Mark],
) -> ToolResult<()> {
	// The file was written by `remux` moments ago and carries no userdata, so
	// there is nothing to preserve and a fresh tag is the whole truth.
	let mut tag = mp4ameta::Tag::default();
	if let Some(title) = non_empty(&tags.title) {
		tag.set_title(title);
	}
	if let Some(author) = non_empty(&tags.author) {
		tag.set_artist(author);
		// A player that groups by album artist must not scatter one author's
		// books across two rows because only `©ART` was set.
		tag.set_album_artist(author);
	}
	if let Some(narrator) = non_empty(&tags.narrator) {
		tag.set_composer(narrator);
	}
	if let Some(album) = non_empty(&tags.album) {
		tag.set_album(album);
	}
	if let Some(genre) = non_empty(&tags.genre) {
		tag.set_custom_genre(genre);
	}
	if let Some(year) = non_empty(&tags.year) {
		tag.set_year(year);
	}
	if let Some(description) = non_empty(&tags.description) {
		tag.set_description(description);
	}
	if let Some((content_type, bytes)) = &tags.cover {
		let image = match content_type {
			ContentType::JPEG => Some(mp4ameta::Img::jpeg(bytes.clone())),
			ContentType::PNG => Some(mp4ameta::Img::png(bytes.clone())),
			// `covr` holds JPEG, PNG or BMP; anything else would be stored as
			// a format no player decodes, which is worse than no cover.
			_ => None,
		};
		if let Some(image) = image {
			tag.set_artwork(image);
		}
	}

	let chapters: Vec<mp4ameta::Chapter> = marks
		.iter()
		.map(|mark| {
			mp4ameta::Chapter::new(
				Duration::from_millis(mark.start_ms.max(0) as u64),
				mark.title.clone(),
			)
		})
		.collect();
	*tag.chapter_list_mut() = chapters.clone();
	*tag.chapter_track_mut() = chapters;

	let config = mp4ameta::WriteConfig {
		write_meta_items: true,
		write_chapter_list: true,
		write_chapter_track: true,
		..mp4ameta::WriteConfig::DEFAULT
	};
	let mut file = File::options().read(true).write(true).open(target)?;
	tag.write_with(&mut file, &config)?;
	file.sync_all()?;
	Ok(())
}

fn non_empty(value: &Option<String>) -> Option<&str> {
	value
		.as_deref()
		.map(str::trim)
		.filter(|value| !value.is_empty())
}

/// One top-level box of a file: where it starts, how long it is, what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TopLevelBox {
	offset: u64,
	size: u64,
	fourcc: [u8; 4],
}

impl TopLevelBox {
	fn end(self) -> u64 {
		self.offset + self.size
	}
}

/// Move `moov` ahead of the media data and correct every chunk offset.
///
/// Returns `true` when the file was rewritten and `false` when it was already
/// faststart, so a caller can report the no-op honestly instead of claiming a
/// rewrite it did not perform.
///
/// The rewrite is `ftyp`, then `moov`, then every other top-level box in its
/// original order. Each moved box shifts by its own delta and every chunk
/// offset is corrected by the delta of the box it points into — rather than by
/// one global shift — because a file with a `free` box between `mdat` and
/// `moov`, or with two `mdat`s, does not move uniformly, and an offset patched
/// with the wrong delta produces a file that opens and plays noise.
pub fn faststart(target: &Path) -> ToolResult<bool> {
	let boxes = {
		let mut file = BufReader::new(File::open(target)?);
		read_top_level(&mut file)?
	};

	let moov_index = boxes
		.iter()
		.position(|item| &item.fourcc == b"moov")
		.ok_or_else(|| {
			ToolError::Invalid(format!("{} has no moov box", target.display()))
		})?;
	let ftyp_index = boxes.iter().position(|item| &item.fourcc == b"ftyp");

	// Already streamable: `moov` is the first box that is not `ftyp`.
	let first_payload = if ftyp_index == Some(0) { 1 } else { 0 };
	if moov_index == first_payload {
		return Ok(false);
	}

	// New layout, and therefore the new start of every box.
	let mut order: Vec<usize> = Vec::with_capacity(boxes.len());
	if let Some(index) = ftyp_index {
		order.push(index);
	}
	order.push(moov_index);
	order.extend(
		(0..boxes.len())
			.filter(|index| Some(*index) != ftyp_index && *index != moov_index),
	);

	let mut deltas: BTreeMap<u64, i64> = BTreeMap::new();
	let mut cursor = 0u64;
	for index in &order {
		let item = boxes[*index];
		deltas.insert(item.offset, cursor as i64 - item.offset as i64);
		cursor += item.size;
	}

	let mut moov = read_box(target, boxes[moov_index])?;
	patch_chunk_offsets(&mut moov, &boxes, &deltas)?;

	let source = target.to_path_buf();
	write_atomic(target, |out| {
		let mut out = BufWriter::new(out);
		let mut input = BufReader::new(File::open(&source)?);
		for index in &order {
			if *index == moov_index {
				out.write_all(&moov)?;
				continue;
			}
			let item = boxes[*index];
			input.seek(SeekFrom::Start(item.offset))?;
			// Streamed, never buffered: `mdat` is the whole audiobook.
			let copied = std::io::copy(&mut input.by_ref().take(item.size), &mut out)?;
			if copied != item.size {
				return Err(ToolError::Invalid(format!(
					"{} ended inside its {} box",
					source.display(),
					fourcc_name(&item.fourcc)
				)));
			}
		}
		out.flush()?;
		Ok(())
	})?;

	Ok(true)
}

/// Every top-level box, in file order.
fn read_top_level(file: &mut BufReader<File>) -> ToolResult<Vec<TopLevelBox>> {
	let length = file.seek(SeekFrom::End(0))?;
	file.seek(SeekFrom::Start(0))?;

	let mut boxes = Vec::new();
	let mut offset = 0u64;
	while offset + BOX_HEADER <= length {
		file.seek(SeekFrom::Start(offset))?;
		let mut header = [0u8; 8];
		file.read_exact(&mut header)?;
		let declared = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
		let fourcc = [header[4], header[5], header[6], header[7]];

		let size = match declared {
			// `size == 1`: a 64-bit size follows the type.
			1 => {
				let mut large = [0u8; 8];
				file.read_exact(&mut large)?;
				u64::from_be_bytes(large)
			},
			// `size == 0`: the box runs to the end of the file.
			0 => length - offset,
			other => u64::from(other),
		};
		if size < BOX_HEADER || offset + size > length {
			return Err(ToolError::Invalid(format!(
				"box {} at {offset} declares an impossible size {size}",
				fourcc_name(&fourcc)
			)));
		}

		boxes.push(TopLevelBox {
			offset,
			size,
			fourcc,
		});
		offset += size;
	}

	Ok(boxes)
}

/// The whole box, header included, as bytes.
fn read_box(path: &Path, item: TopLevelBox) -> ToolResult<Vec<u8>> {
	let mut file = File::open(path)?;
	file.seek(SeekFrom::Start(item.offset))?;
	let mut bytes = vec![
		0u8;
		usize::try_from(item.size).map_err(|_| ToolError::Invalid(
			"moov is larger than this platform can address".to_string()
		))?
	];
	file.read_exact(&mut bytes)?;
	Ok(bytes)
}

/// Add each moved box's delta to every chunk offset that points into it.
fn patch_chunk_offsets(
	moov: &mut [u8],
	boxes: &[TopLevelBox],
	deltas: &BTreeMap<u64, i64>,
) -> ToolResult<()> {
	let tables = find_tables(moov, 0, moov.len())?;
	for table in tables {
		match table.kind {
			TableKind::Stco => {
				for index in 0..table.entries {
					let at = table.first_entry + index * 4;
					let old = u64::from(u32::from_be_bytes([
						moov[at],
						moov[at + 1],
						moov[at + 2],
						moov[at + 3],
					]));
					let new = shift(old, boxes, deltas)?;
					let new = u32::try_from(new).map_err(|_| {
						ToolError::Invalid(
							"the assembled book needs 64-bit chunk offsets: re-run with the ffmpeg path, which writes them"
								.to_string(),
						)
					})?;
					moov[at..at + 4].copy_from_slice(&new.to_be_bytes());
				}
			},
			TableKind::Co64 => {
				for index in 0..table.entries {
					let at = table.first_entry + index * 8;
					let mut wide = [0u8; 8];
					wide.copy_from_slice(&moov[at..at + 8]);
					let new = shift(u64::from_be_bytes(wide), boxes, deltas)?;
					moov[at..at + 8].copy_from_slice(&new.to_be_bytes());
				}
			},
		}
	}
	Ok(())
}

/// `offset` moved by the delta of the top-level box it falls inside.
fn shift(
	offset: u64,
	boxes: &[TopLevelBox],
	deltas: &BTreeMap<u64, i64>,
) -> ToolResult<u64> {
	let owner = boxes
		.iter()
		.find(|item| offset >= item.offset && offset < item.end())
		.ok_or_else(|| {
			ToolError::Invalid(format!(
				"chunk offset {offset} points outside every box in the file"
			))
		})?;
	let delta = deltas.get(&owner.offset).copied().unwrap_or(0);
	u64::try_from(offset as i64 + delta).map_err(|_| {
		ToolError::Invalid(format!("chunk offset {offset} moved before the file start"))
	})
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableKind {
	Stco,
	Co64,
}

/// One chunk offset table inside the `moov` byte buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Table {
	kind: TableKind,
	/// Index into the `moov` buffer of the first entry.
	first_entry: usize,
	entries: usize,
}

/// Every `stco`/`co64` reachable through [`CONTAINERS`], as buffer offsets.
fn find_tables(moov: &[u8], start: usize, end: usize) -> ToolResult<Vec<Table>> {
	let mut tables = Vec::new();
	let mut offset = start;
	while offset + BOX_HEADER as usize <= end {
		let declared = u32::from_be_bytes([
			moov[offset],
			moov[offset + 1],
			moov[offset + 2],
			moov[offset + 3],
		]);
		let fourcc: [u8; 4] = [
			moov[offset + 4],
			moov[offset + 5],
			moov[offset + 6],
			moov[offset + 7],
		];
		let (size, header) = match declared {
			1 => {
				if offset + LARGE_SIZE_HEADER as usize > end {
					break;
				}
				let mut large = [0u8; 8];
				large.copy_from_slice(&moov[offset + 8..offset + 16]);
				(u64::from_be_bytes(large), LARGE_SIZE_HEADER as usize)
			},
			0 => ((end - offset) as u64, BOX_HEADER as usize),
			other => (u64::from(other), BOX_HEADER as usize),
		};
		let size = usize::try_from(size).unwrap_or(usize::MAX);
		if size < header || offset + size > end {
			return Err(ToolError::Invalid(format!(
				"moov holds a {} box with an impossible size",
				fourcc_name(&fourcc)
			)));
		}

		let content = offset + header;
		if CONTAINERS.contains(&&fourcc) {
			tables.extend(find_tables(moov, content, offset + size)?);
		} else if &fourcc == b"stco" || &fourcc == b"co64" {
			// version/flags, then the entry count.
			if content + 8 > offset + size {
				return Err(ToolError::Invalid(format!(
					"{} is too short to hold an entry count",
					fourcc_name(&fourcc)
				)));
			}
			let entries = u32::from_be_bytes([
				moov[content + 4],
				moov[content + 5],
				moov[content + 6],
				moov[content + 7],
			]) as usize;
			let kind = if &fourcc == b"stco" {
				TableKind::Stco
			} else {
				TableKind::Co64
			};
			let width = if kind == TableKind::Stco { 4 } else { 8 };
			let first_entry = content + 8;
			if first_entry + entries * width > offset + size {
				return Err(ToolError::Invalid(format!(
					"{} declares {entries} entries it does not hold",
					fourcc_name(&fourcc)
				)));
			}
			tables.push(Table {
				kind,
				first_entry,
				entries,
			});
		}

		offset += size;
	}

	Ok(tables)
}

fn fourcc_name(fourcc: &[u8; 4]) -> String {
	String::from_utf8_lossy(fourcc).to_string()
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A 32-bit box header.
	fn header(fourcc: &[u8; 4], payload_len: usize) -> Vec<u8> {
		let mut bytes = ((payload_len + 8) as u32).to_be_bytes().to_vec();
		bytes.extend_from_slice(fourcc);
		bytes
	}

	fn container(fourcc: &[u8; 4], children: Vec<u8>) -> Vec<u8> {
		let mut bytes = header(fourcc, children.len());
		bytes.extend(children);
		bytes
	}

	fn stco(offsets: &[u32]) -> Vec<u8> {
		let mut payload = vec![0u8; 4];
		payload.extend((offsets.len() as u32).to_be_bytes());
		for offset in offsets {
			payload.extend(offset.to_be_bytes());
		}
		let mut bytes = header(b"stco", payload.len());
		bytes.extend(payload);
		bytes
	}

	fn co64(offsets: &[u64]) -> Vec<u8> {
		let mut payload = vec![0u8; 4];
		payload.extend((offsets.len() as u32).to_be_bytes());
		for offset in offsets {
			payload.extend(offset.to_be_bytes());
		}
		let mut bytes = header(b"co64", payload.len());
		bytes.extend(payload);
		bytes
	}

	/// A `moov` holding one table at the real depth.
	fn moov_with(table: Vec<u8>) -> Vec<u8> {
		container(
			b"moov",
			container(
				b"trak",
				container(b"mdia", container(b"minf", container(b"stbl", table))),
			),
		)
	}

	#[test]
	fn tables_are_found_at_the_real_depth() {
		let moov = moov_with(stco(&[100, 200]));
		let tables = find_tables(&moov, 0, moov.len()).expect("walk moov");

		assert_eq!(tables.len(), 1);
		assert_eq!(tables[0].kind, TableKind::Stco);
		assert_eq!(tables[0].entries, 2);
	}

	/// Four bytes spelling `stco` inside a metadata payload is not a table.
	#[test]
	fn a_fourcc_inside_an_unopened_box_is_not_a_table() {
		let mut payload = b"stco".to_vec();
		payload.extend([0u8; 12]);
		let moov = container(b"moov", container(b"udta", payload));

		assert!(find_tables(&moov, 0, moov.len())
			.expect("walk moov")
			.is_empty());
	}

	#[test]
	fn a_table_promising_entries_it_does_not_hold_is_refused() {
		let mut payload = vec![0u8; 4];
		payload.extend(9999u32.to_be_bytes());
		let mut table = header(b"stco", payload.len());
		table.extend(payload);
		let moov = moov_with(table);

		let error = find_tables(&moov, 0, moov.len()).expect_err("refuse the table");
		assert!(error.to_string().contains("9999 entries"), "{error}");
	}

	/// The whole point: an offset into `mdat` gains exactly the distance
	/// `mdat` moved, which is `moov`'s size.
	#[test]
	fn offsets_gain_the_delta_of_the_box_they_point_into() {
		let mut moov = moov_with(co64(&[64, 1064]));
		let boxes = [
			TopLevelBox {
				offset: 0,
				size: 24,
				fourcc: *b"ftyp",
			},
			TopLevelBox {
				offset: 24,
				size: 4000,
				fourcc: *b"mdat",
			},
			TopLevelBox {
				offset: 4024,
				size: moov.len() as u64,
				fourcc: *b"moov",
			},
		];
		// New layout: ftyp at 0, moov at 24, mdat after it.
		let deltas = BTreeMap::from([(0, 0), (4024, 24 - 4024), (24, moov.len() as i64)]);

		patch_chunk_offsets(&mut moov, &boxes, &deltas).expect("patch offsets");

		let tables = find_tables(&moov, 0, moov.len()).expect("walk moov");
		let at = tables[0].first_entry;
		let mut first = [0u8; 8];
		first.copy_from_slice(&moov[at..at + 8]);
		let mut second = [0u8; 8];
		second.copy_from_slice(&moov[at + 8..at + 16]);

		assert_eq!(u64::from_be_bytes(first), 64 + moov.len() as u64);
		assert_eq!(u64::from_be_bytes(second), 1064 + moov.len() as u64);
	}

	/// An offset the file does not contain is corruption, not something to
	/// silently shift.
	#[test]
	fn an_offset_outside_every_box_is_refused() {
		let boxes = [TopLevelBox {
			offset: 0,
			size: 100,
			fourcc: *b"ftyp",
		}];
		let error = shift(500, &boxes, &BTreeMap::new()).expect_err("refuse");
		assert!(error.to_string().contains("outside every box"), "{error}");
	}

	/// A 32-bit table that would need a 64-bit offset says so, instead of
	/// wrapping and producing a file that plays noise.
	#[test]
	fn a_32_bit_table_that_overflows_names_the_way_out() {
		let mut moov = moov_with(stco(&[u32::MAX - 10]));
		let boxes = [TopLevelBox {
			offset: 0,
			size: u64::from(u32::MAX),
			fourcc: *b"mdat",
		}];
		let deltas = BTreeMap::from([(0, 1000)]);

		let error = patch_chunk_offsets(&mut moov, &boxes, &deltas).expect_err("refuse");
		assert!(
			error.to_string().contains("64-bit chunk offsets"),
			"{error}"
		);
	}

	#[test]
	fn parts_that_disagree_on_the_stream_have_no_common_shape() {
		let stereo = AacShape {
			timescale: 44100,
			freq_index: SampleFreqIndex::Freq44100,
			chan_conf: ChannelConfig::Stereo,
			profile: AudioObjectType::AacLowComplexity,
			bitrate: 64_000,
		};
		let mono = AacShape {
			chan_conf: ChannelConfig::Mono,
			..stereo
		};

		assert_eq!(
			common_shape(&[stereo, stereo]),
			Some(stereo),
			"identical parts share a shape"
		);
		assert!(common_shape(&[stereo, mono]).is_none());
		assert!(common_shape(&[]).is_none());
	}

	/// A differing bitrate is advisory metadata, not a decoding parameter: the
	/// parts still remux and the output advertises the larger.
	#[test]
	fn a_differing_bitrate_still_remuxes() {
		let low = AacShape {
			timescale: 44100,
			freq_index: SampleFreqIndex::Freq44100,
			chan_conf: ChannelConfig::Stereo,
			profile: AudioObjectType::AacLowComplexity,
			bitrate: 62_000,
		};
		let high = AacShape {
			bitrate: 64_000,
			..low
		};

		assert_eq!(
			common_shape(&[low, high]).map(|shape| shape.bitrate),
			Some(64_000)
		);
	}
}
