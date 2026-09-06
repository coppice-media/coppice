//! This module defines the [`OpdsLink`] struct for representing an OPDS `atom:link` element as
//! specified at https://specs.opds.io/opds-1.2#the-atomlink-element
//!
//! It also defines the [`OpdsStreamLink`] struct for representing an OPDS page steaming extension
//! link element as specified at https://github.com/anansi-project/opds-pse/blob/master/v1.2.md

use std::borrow::Cow;

use xml::{writer::XmlEvent, EventWriter};

use crate::error::CoreResult;
use stump_media::ContentType;

use super::util::OpdsEnumStr;

#[derive(Debug, Clone, Copy)]
/// A struct for representing an OPDS `atom:link` element as specified at
/// https://specs.opds.io/opds-1.2#the-atomlink-element
pub enum OpdsLinkType {
	Acquisition, // "application/atom+xml;profile=opds-catalog;kind=acquisition",
	ImageJpeg,   // "image/jpeg",
	ImagePng,    // "image/png",
	ImageGif,    // "image/gif",
	Navigation,  // "application/atom+xml;profile=opds-catalog;kind=navigation",
	OctetStream, // "application/octet-stream",
	Zip,         // "application/zip"
	Epub,        // "application/epub+zip"
	Search,      // "application/opensearchdescription+xml"
	/// An audiobook container, carrying the [`ContentType`] that decides the
	/// MIME (`audio/mp4`, `audio/mpeg`, `audio/opus`, `audio/ogg`,
	/// `audio/flac`). `stump_media` already owns the extension → type table,
	/// so the feed defers to it instead of growing a second one.
	Audio(ContentType),
}

impl OpdsLinkType {
	pub fn from_extension(extension: &str) -> Option<Self> {
		// An audiobook is a first-class acquisition, and all six audio
		// containers are already mapped by `ContentType`.
		let content_type = ContentType::from_extension(extension);
		if content_type.is_audio() {
			return Some(OpdsLinkType::Audio(content_type));
		}

		match extension {
			"jpeg" | "jpg" => Some(OpdsLinkType::ImageJpeg),
			"png" => Some(OpdsLinkType::ImagePng),
			"gif" => Some(OpdsLinkType::ImageGif),
			"epub" => Some(OpdsLinkType::Epub),
			// TODO: RARs as ZIP??? Obviously for content type it's different, but does OPDS concern itself with that?
			"zip" | "cbz" | "rar" | "cbr" => Some(OpdsLinkType::Zip),
			_ => None,
		}
	}

	/// The value written into the Atom `link@type` attribute.
	///
	/// Every variant but [`OpdsLinkType::Audio`] is a fixed string and still
	/// borrows; audio renders through [`ContentType`]'s `Display`, which is
	/// why this returns a [`Cow`] rather than the `&'static str` the other
	/// v1.2 enums hand back through [`OpdsEnumStr`].
	pub fn mime(&self) -> Cow<'static, str> {
		match self {
			OpdsLinkType::Acquisition => Cow::Borrowed(
				"application/atom+xml;profile=opds-catalog;kind=acquisition",
			),
			OpdsLinkType::ImageJpeg => Cow::Borrowed("image/jpeg"),
			OpdsLinkType::ImagePng => Cow::Borrowed("image/png"),
			OpdsLinkType::ImageGif => Cow::Borrowed("image/gif"),
			OpdsLinkType::Navigation => {
				Cow::Borrowed("application/atom+xml;profile=opds-catalog;kind=navigation")
			},
			OpdsLinkType::OctetStream => Cow::Borrowed("application/octet-stream"),
			OpdsLinkType::Zip => Cow::Borrowed("application/zip"),
			OpdsLinkType::Epub => Cow::Borrowed("application/epub+zip"),
			OpdsLinkType::Search => {
				Cow::Borrowed("application/opensearchdescription+xml")
			},
			OpdsLinkType::Audio(content_type) => Cow::Owned(content_type.mime_type()),
		}
	}
}

impl TryFrom<ContentType> for OpdsLinkType {
	type Error = String;

	fn try_from(content_type: ContentType) -> Result<Self, Self::Error> {
		match content_type {
			ContentType::JPEG => Ok(OpdsLinkType::ImageJpeg),
			ContentType::PNG => Ok(OpdsLinkType::ImagePng),
			ContentType::GIF => Ok(OpdsLinkType::ImageGif),
			_ => Err(format!("Unsupported content type: {content_type}")),
		}
	}
}

#[derive(Debug)]
pub enum OpdsLinkRel {
	ItSelf,      // self
	Subsection,  // "subsection",
	Acquisition, // "http://opds-spec.org/acquisition",
	Start,       // start
	Next,        // next
	Previous,    // previous
	Thumbnail,   // "http://opds-spec.org/image/thumbnail"
	Image,       // "http://opds-spec.org/image"
	PageStream,  // "http://vaemendis.net/opds-pse/stream"
	Search,      // "search"
}

impl OpdsEnumStr for OpdsLinkRel {
	fn as_str(&self) -> &'static str {
		match self {
			OpdsLinkRel::ItSelf => "self",
			OpdsLinkRel::Subsection => "subsection",
			OpdsLinkRel::Acquisition => "http://opds-spec.org/acquisition",
			OpdsLinkRel::Start => "start",
			OpdsLinkRel::Next => "next",
			OpdsLinkRel::Previous => "previous",
			OpdsLinkRel::Thumbnail => "http://opds-spec.org/image/thumbnail",
			OpdsLinkRel::Image => "http://opds-spec.org/image",
			OpdsLinkRel::PageStream => "http://vaemendis.net/opds-pse/stream",
			OpdsLinkRel::Search => "search",
		}
	}
}

// TODO: this struct needs to be restructured.
// I need to be able to output the following:
// <link xmlns:wstxns3="http://vaemendis.net/opds-pse/ns" href="/opds/v1.2/books/<id>/pages/{pageNumber}?zero_based=true" wstxns3:count="309" type="image/jpeg" rel="http://vaemendis.net/opds-pse/stream"/>

#[derive(Debug)]
pub struct OpdsLink {
	pub link_type: OpdsLinkType,
	pub rel: OpdsLinkRel,
	pub href: String,
}

impl OpdsLink {
	pub fn new(link_type: OpdsLinkType, rel: OpdsLinkRel, href: String) -> Self {
		Self {
			link_type,
			rel,
			href,
		}
	}

	pub fn write(&self, writer: &mut EventWriter<Vec<u8>>) -> CoreResult<()> {
		let link_type = self.link_type.mime();
		let link = XmlEvent::start_element("link")
			.attr("type", link_type.as_ref())
			.attr("rel", self.rel.as_str())
			.attr("href", &self.href);

		writer.write(link)?;
		writer.write(XmlEvent::end_element())?; // end of link
		Ok(())
	}
}

// don't love this solution, but it works for now.
#[derive(Debug)]
pub struct OpdsStreamLink {
	pub book_id: String,
	pub count: String,
	pub mime_type: String,
	pub last_read: Option<String>,
	pub last_read_date: Option<String>,
}

impl OpdsStreamLink {
	pub fn new(
		book_id: String,
		count: String,
		mime_type: String,
		last_read: Option<String>,
		last_read_date: Option<String>,
	) -> Self {
		Self {
			book_id,
			count,
			mime_type,
			last_read,
			last_read_date,
		}
	}

	pub fn write(&self, writer: &mut EventWriter<Vec<u8>>) -> CoreResult<()> {
		let href = format!(
			"/opds/v1.2/books/{}/pages/{{pageNumber}}?zero_based=true",
			self.book_id
		);

		let mut link = XmlEvent::start_element("link")
			.attr("href", href.as_str())
			.attr("type", &self.mime_type)
			.attr("rel", "http://vaemendis.net/opds-pse/stream")
			// .attr("xmlns:pse", "http://vaemendis.net/opds-pse/ns")
			.attr("pse:count", &self.count);

		if let Some(last_read) = &self.last_read {
			link = link.attr("pse:lastRead", last_read);
		}

		if let Some(last_read_date) = &self.last_read_date {
			link = link.attr("pse:lastReadDate", last_read_date);
		}

		writer.write(link)?;
		writer.write(XmlEvent::end_element())?; // end of link
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::opds::v1_2::tests::normalize_xml;

	#[test]
	fn test_opds_link() {
		let link = OpdsLink::new(
			OpdsLinkType::ImageJpeg,
			OpdsLinkRel::Image,
			"https://www.example.com/thumbnail/image.jpg".to_string(),
		);

		let mut writer = EventWriter::new(Vec::new());
		link.write(&mut writer).unwrap();

		let result = String::from_utf8(writer.into_inner()).unwrap();
		let expected_result = normalize_xml(
			r#"
			<?xml version="1.0" encoding="UTF-8"?>
			<link type="image/jpeg"
						rel="http://opds-spec.org/image"
						href="https://www.example.com/thumbnail/image.jpg"
			/>
			"#,
		);

		assert_eq!(result, expected_result);
	}

	#[test]
	fn test_opds_stream_link() {
		let link = OpdsStreamLink::new(
			"123".to_string(),
			"35".to_string(),
			"image/jpeg".to_string(),
			Some("10".to_string()),
			Some("2010-01-10T10:01:11Z".to_string()),
		);

		let mut writer = EventWriter::new(Vec::new());
		link.write(&mut writer).unwrap();

		let result = String::from_utf8(writer.into_inner()).unwrap();
		let expected_result = normalize_xml(
			r#"
			<?xml version="1.0" encoding="UTF-8"?>
			<link href="/opds/v1.2/books/123/pages/{pageNumber}?zero_based=true"
						type="image/jpeg"
						rel="http://vaemendis.net/opds-pse/stream"
						pse:count="35"
						pse:lastRead="10"
						pse:lastReadDate="2010-01-10T10:01:11Z"
			/>
			"#,
		);

		assert_eq!(result, expected_result);
	}
}
