use std::io::{Cursor, Write};

use tempfile::TempDir;
use zip::{write::FileOptions, CompressionMethod, ZipWriter};

use super::*;

const AES_128: &str = "http://www.w3.org/2001/04/xmlenc#aes128-cbc";
const IDPF_OBFUSCATION: &str = "http://www.idpf.org/2008/embedding";
const ADOBE_OBFUSCATION: &str = "http://ns.adobe.com/pdf/enc#RC";

fn zip_with(members: &[(&str, &[u8])]) -> Vec<u8> {
	let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
	let options: FileOptions<()> =
		FileOptions::default().compression_method(CompressionMethod::Stored);
	for (name, bytes) in members {
		writer.start_file(*name, options).expect("start member");
		writer.write_all(bytes).expect("write member");
	}
	writer.finish().expect("finish zip").into_inner()
}

/// A minimal container with the EPUB mimetype plus whatever `META-INF` the
/// test needs.
fn epub_with(members: &[(&str, &[u8])]) -> Vec<u8> {
	let mut all: Vec<(&str, &[u8])> =
		vec![("mimetype", b"application/epub+zip".as_slice())];
	all.extend_from_slice(members);
	zip_with(&all)
}

fn encryption_xml(algorithms: &[&str]) -> Vec<u8> {
	let mut xml = String::from(
		"<?xml version=\"1.0\"?>\n<encryption \
		 xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\" \
		 xmlns:enc=\"http://www.w3.org/2001/04/xmlenc#\">",
	);
	for (index, algorithm) in algorithms.iter().enumerate() {
		xml.push_str(&format!(
			"<enc:EncryptedData><enc:EncryptionMethod \
			 Algorithm=\"{algorithm}\"/><enc:CipherData><enc:CipherReference \
			 URI=\"EPUB/res{index}.xhtml\"/></enc:CipherData></\
			 enc:EncryptedData>"
		));
	}
	xml.push_str("</encryption>");
	xml.into_bytes()
}

fn adept_rights(key_len: usize) -> Vec<u8> {
	format!(
		"<?xml version=\"1.0\"?><rights \
		 xmlns=\"http://ns.adobe.com/adept\"><licenseToken><encryptedKey>{}</\
		 encryptedKey></licenseToken></rights>",
		"A".repeat(key_len)
	)
	.into_bytes()
}

// ---------------------------------------------------------------------------
// EPUB
// ---------------------------------------------------------------------------

#[test]
fn font_obfuscation_only_is_not_drm() {
	for algorithm in [IDPF_OBFUSCATION, ADOBE_OBFUSCATION] {
		let epub = epub_with(&[(
			"META-INF/encryption.xml",
			encryption_xml(&[algorithm]).as_slice(),
		)]);
		assert_eq!(
			detect_epub_drm(Cursor::new(epub)).unwrap(),
			None,
			"{algorithm} is font obfuscation, not DRM"
		);
	}
}

#[test]
fn plain_epub_and_comic_archive_are_not_drm() {
	let epub = epub_with(&[("EPUB/package.opf", b"<package/>")]);
	assert_eq!(detect_epub_drm(Cursor::new(epub)).unwrap(), None);

	let cbz = zip_with(&[("0001.jpg", b"\xff\xd8\xff\xe0"), ("0002.jpg", b"x")]);
	assert_eq!(detect_epub_drm(Cursor::new(cbz)).unwrap(), None);
}

#[test]
fn content_encryption_without_a_vendor_marker_is_reported() {
	let epub = epub_with(&[(
		"META-INF/encryption.xml",
		encryption_xml(&[IDPF_OBFUSCATION, AES_128]).as_slice(),
	)]);
	let report = detect_epub_drm(Cursor::new(epub)).unwrap().expect("drm");
	assert_eq!(report.container, DrmContainer::Epub);
	assert_eq!(report.scheme, DrmScheme::EpubUnknownEncryption);
	// The obfuscation algorithm is not reported; the AES one is.
	assert_eq!(
		report.markers,
		vec![format!("META-INF/encryption.xml:{AES_128}")]
	);
	assert!(report.reason.contains(AES_128), "{}", report.reason);
}

#[test]
fn key_wrapping_algorithm_alone_is_not_content_encryption() {
	// `EncryptedKey` carries its own `EncryptionMethod`; only `EncryptedData`
	// algorithms describe an encrypted resource.
	let xml = b"<?xml version=\"1.0\"?><encryption \
		xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\" \
		xmlns:enc=\"http://www.w3.org/2001/04/xmlenc#\"><enc:EncryptedKey>\
		<enc:EncryptionMethod \
		Algorithm=\"http://www.w3.org/2001/04/xmlenc#rsa-1_5\"/></\
		enc:EncryptedKey><enc:EncryptedData><enc:EncryptionMethod \
		Algorithm=\"http://www.idpf.org/2008/embedding\"/></\
		enc:EncryptedData></encryption>";
	let epub = epub_with(&[("META-INF/encryption.xml", xml.as_slice())]);
	assert_eq!(detect_epub_drm(Cursor::new(epub)).unwrap(), None);
}

#[test]
fn adept_and_passhash_are_told_apart_by_key_length() {
	for (len, expected) in [
		(172usize, DrmScheme::AdobeAdept),
		// Some producers emit a 192-character key; it is still ADEPT.
		(192, DrmScheme::AdobeAdept),
		(64, DrmScheme::BarnesNoble),
		(12, DrmScheme::EpubUnknownEncryption),
	] {
		let epub = epub_with(&[
			(
				"META-INF/encryption.xml",
				encryption_xml(&[AES_128]).as_slice(),
			),
			("META-INF/rights.xml", adept_rights(len).as_slice()),
		]);
		let report = detect_epub_drm(Cursor::new(epub)).unwrap().expect("drm");
		assert_eq!(report.scheme, expected, "key length {len}");
		assert_eq!(
			report.markers,
			vec![format!("META-INF/rights.xml:encryptedKey[{len}]")]
		);
	}
}

#[test]
fn a_hardened_adept_key_type_is_recorded() {
	// keyType 0/1/2 are classic ADEPT; above 2 is Adobe's hardened DRM.
	for (key_type, hardened) in [(0u32, false), (2, false), (3, true)] {
		let rights = format!(
			"<rights xmlns=\"http://ns.adobe.com/adept\"><encryptedKey \
			 keyType=\"{key_type}\">{}</encryptedKey></rights>",
			"A".repeat(172)
		);
		let epub = epub_with(&[("META-INF/rights.xml", rights.as_bytes())]);
		let report = detect_epub_drm(Cursor::new(epub)).unwrap().expect("drm");
		assert_eq!(report.scheme, DrmScheme::AdobeAdept);
		assert_eq!(
			report
				.markers
				.contains(&format!("META-INF/rights.xml:keyType={key_type}")),
			hardened,
			"keyType {key_type}"
		);
		assert_eq!(
			report.reason.contains("hardened"),
			hardened,
			"keyType {key_type}: {}",
			report.reason
		);
	}
}

#[test]
fn the_adobe_content_algorithm_attributes_adept_without_rights_xml() {
	let epub = epub_with(&[(
		"META-INF/encryption.xml",
		encryption_xml(&["http://ns.adobe.com/adept/xmlenc#aes128-cbc-uncompressed"])
			.as_slice(),
	)]);
	let report = detect_epub_drm(Cursor::new(epub)).unwrap().expect("drm");
	assert_eq!(report.scheme, DrmScheme::AdobeAdept);

	// The generic W3C AES URI is used by several vendors, so it stays unnamed.
	let epub = epub_with(&[(
		"META-INF/encryption.xml",
		encryption_xml(&[AES_128]).as_slice(),
	)]);
	let report = detect_epub_drm(Cursor::new(epub)).unwrap().expect("drm");
	assert_eq!(report.scheme, DrmScheme::EpubUnknownEncryption);
}

#[test]
fn a_kfx_zip_package_is_detected_and_a_comic_archive_is_not_scanned() {
	let mut drmion = DRMION_MAGIC.to_vec();
	drmion.extend_from_slice(b"encrypted payload");
	let package = zip_with(&[
		("book/metadata.kfx", b"\xe0\x01\x00\xeaProtectedData"),
		("book/content.kfx", drmion.as_slice()),
	]);
	let report = detect_epub_drm(Cursor::new(package))
		.unwrap()
		.expect("kfx package");
	assert_eq!(report.container, DrmContainer::Kfx);
	assert_eq!(report.scheme, DrmScheme::AmazonKfx);
	assert_eq!(report.markers, vec!["book/content.kfx:DRMION".to_string()]);

	// A `.kfx`-free archive is never opened member by member.
	let cbz = zip_with(&[("0001.jpg", DRMION_MAGIC)]);
	assert_eq!(detect_epub_drm(Cursor::new(cbz)).unwrap(), None);
}

#[test]
fn adept_key_whitespace_is_stripped_before_measuring() {
	let key = format!("{}\n  {}", "A".repeat(100), "B".repeat(72));
	let rights = format!(
		"<rights xmlns=\"http://ns.adobe.com/adept\"><encryptedKey>{key}</encryptedKey></rights>"
	);
	let epub = epub_with(&[("META-INF/rights.xml", rights.as_bytes())]);
	let report = detect_epub_drm(Cursor::new(epub)).unwrap().expect("drm");
	assert_eq!(report.scheme, DrmScheme::AdobeAdept);
}

#[test]
fn vendor_markers_are_recognised() {
	let cases: Vec<(&str, Vec<(&str, Vec<u8>)>, DrmScheme)> = vec![
		(
			"apple",
			vec![("META-INF/sinf.xml", b"<sinf><fairplay/></sinf>".to_vec())],
			DrmScheme::AppleFairPlay,
		),
		(
			"kobo",
			vec![(
				"META-INF/rights.xml",
				b"<rights><kdrm>x</kdrm></rights>".to_vec(),
			)],
			DrmScheme::Kobo,
		),
		(
			"lcp",
			vec![
				("META-INF/license.lcpl", b"{\"id\":\"x\"}".to_vec()),
				(
					"META-INF/encryption.xml",
					b"<encryption><EncryptedContentKey/></encryption>".to_vec(),
				),
			],
			DrmScheme::ReadiumLcp,
		),
	];

	for (name, members, expected) in cases {
		let borrowed = members
			.iter()
			.map(|(path, bytes)| (*path, bytes.as_slice()))
			.collect::<Vec<_>>();
		let epub = epub_with(&borrowed);
		let report = detect_epub_drm(Cursor::new(epub))
			.unwrap()
			.unwrap_or_else(|| panic!("{name} should be detected"));
		assert_eq!(report.scheme, expected, "{name}");
		assert!(!report.reason.is_empty(), "{name} has no reason");
	}
}

#[test]
fn unparseable_encryption_file_is_reported_rather_than_ignored() {
	let epub = epub_with(&[(
		"META-INF/encryption.xml",
		b"<encryption><EncryptedData".as_slice(),
	)]);
	let report = detect_epub_drm(Cursor::new(epub)).unwrap().expect("drm");
	assert_eq!(report.scheme, DrmScheme::EpubUnknownEncryption);
	assert_eq!(
		report.markers,
		vec!["META-INF/encryption.xml:unparseable".to_string()]
	);
}

#[test]
fn a_truncated_zip_is_not_an_error() {
	assert_eq!(
		detect_epub_drm(Cursor::new(b"PK\x03\x04junk")).unwrap(),
		None
	);
}

// ---------------------------------------------------------------------------
// MOBI / AZW
// ---------------------------------------------------------------------------

/// MOBI header length used by the fixtures; record 0 is `16 + MOBI_HEADER_LEN`
/// bytes before any EXTH block.
const MOBI_HEADER_LEN: usize = 232;

#[derive(Default)]
struct MobiFixture {
	encryption_type: u16,
	drm_key_block: Option<(u32, u32)>,
	exth: Vec<(u32, Vec<u8>)>,
}

impl MobiFixture {
	fn build(&self) -> Vec<u8> {
		let mut record0 = vec![0u8; 16 + MOBI_HEADER_LEN];
		record0[0..2].copy_from_slice(&1u16.to_be_bytes()); // no compression
		record0[12..14].copy_from_slice(&self.encryption_type.to_be_bytes());
		record0[16..20].copy_from_slice(b"MOBI");
		record0[20..24].copy_from_slice(&(MOBI_HEADER_LEN as u32).to_be_bytes());
		let (drm_offset, drm_count) = self.drm_key_block.unwrap_or((u32::MAX, u32::MAX));
		record0[168..172].copy_from_slice(&drm_offset.to_be_bytes());
		record0[172..176].copy_from_slice(&drm_count.to_be_bytes());

		if !self.exth.is_empty() {
			record0[128..132].copy_from_slice(&0x40u32.to_be_bytes());
			let mut exth = Vec::new();
			for (record_type, data) in &self.exth {
				exth.extend_from_slice(&record_type.to_be_bytes());
				exth.extend_from_slice(&((data.len() + 8) as u32).to_be_bytes());
				exth.extend_from_slice(data);
			}
			record0.extend_from_slice(b"EXTH");
			record0.extend_from_slice(&((exth.len() + 12) as u32).to_be_bytes());
			record0.extend_from_slice(&(self.exth.len() as u32).to_be_bytes());
			record0.extend_from_slice(&exth);
		}

		// PDB header: 78 fixed bytes, one 8-byte record info entry, 2 gap
		// bytes.
		let record0_offset = 78 + 8 + 2u32;
		let mut file = vec![0u8; record0_offset as usize];
		file[0..5].copy_from_slice(b"Title");
		file[60..64].copy_from_slice(b"BOOK");
		file[64..68].copy_from_slice(b"MOBI");
		file[76..78].copy_from_slice(&1u16.to_be_bytes());
		file[78..82].copy_from_slice(&record0_offset.to_be_bytes());
		file.extend_from_slice(&record0);
		file
	}
}

#[test]
fn palmdoc_encryption_type_classifies_mobipocket_drm() {
	for (encryption_type, expected) in [
		(1u16, DrmScheme::MobipocketLegacy),
		(2, DrmScheme::Mobipocket),
	] {
		let bytes = MobiFixture {
			encryption_type,
			..Default::default()
		}
		.build();
		let report = detect_mobi_drm(Cursor::new(bytes))
			.unwrap()
			.expect("encrypted mobi");
		assert_eq!(report.container, DrmContainer::Mobipocket);
		assert_eq!(report.scheme, expected);
		assert_eq!(
			report.markers,
			vec![format!("PalmDOC:encryption_type={encryption_type}")]
		);
		assert!(report.reason.contains("ciphertext"), "{}", report.reason);
	}
}

#[test]
fn unencrypted_mobi_is_not_drm_even_with_tamper_proof_keys() {
	// EXTH 209 survives in DRM-free Amazon files; encryption type 0 proves the
	// text records are plaintext, so the file must be ingestible.
	let bytes = MobiFixture {
		encryption_type: 0,
		exth: vec![(209, vec![0xAB; 32]), (100, b"Author".to_vec())],
		..Default::default()
	}
	.build();
	assert_eq!(detect_mobi_drm(Cursor::new(bytes)).unwrap(), None);
}

#[test]
fn a_declared_drm_key_block_is_drm_and_records_exth_markers() {
	let bytes = MobiFixture {
		encryption_type: 0,
		drm_key_block: Some((0x1234, 1)),
		exth: vec![(209, vec![0xAB; 32]), (1, b"srv".to_vec())],
	}
	.build();
	let report = detect_mobi_drm(Cursor::new(bytes))
		.unwrap()
		.expect("drm key block");
	assert_eq!(report.scheme, DrmScheme::Mobipocket);
	// Markers keep EXTH document order, not numeric order.
	assert_eq!(
		report.markers,
		vec![
			"PalmDOC:encryption_type=0".to_string(),
			"EXTH:209".to_string(),
			"EXTH:1".to_string(),
			"MOBI:drm_offset=0x1234,drm_count=1".to_string(),
		]
	);
	assert!(report.reason.contains("DRM key block"), "{}", report.reason);
}

#[test]
fn a_clean_mobi_is_not_drm() {
	let bytes = MobiFixture::default().build();
	assert_eq!(detect_mobi_drm(Cursor::new(bytes)).unwrap(), None);
	// An empty EXTH 209 payload is not evidence of anything.
	let bytes = MobiFixture {
		exth: vec![(209, Vec::new())],
		..Default::default()
	}
	.build();
	assert_eq!(detect_mobi_drm(Cursor::new(bytes)).unwrap(), None);
}

#[test]
fn a_non_palm_file_is_ignored_by_the_mobi_detector() {
	assert_eq!(detect_mobi_drm(Cursor::new(vec![0u8; 200])).unwrap(), None);
	assert_eq!(detect_mobi_drm(Cursor::new(b"short")).unwrap(), None);
}

// ---------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------

fn pdf(trailer: &str) -> Vec<u8> {
	format!(
		"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n{trailer}\nstartxref\n9\n%%EOF\n"
	)
	.into_bytes()
}

#[test]
fn pdf_encrypt_dictionary_names_the_security_handler() {
	for (trailer, expected, marker) in [
		(
			"<< /Size 4 /Encrypt 3 0 R >>\n3 0 obj\n<< /Filter /Standard /V 2 /R 3 >>",
			DrmScheme::PdfStandardSecurity,
			"Standard",
		),
		(
			"<< /Size 4 /Encrypt 3 0 R >>\n3 0 obj\n<< /Filter /EBX_HANDLER /V 4 >>",
			DrmScheme::PdfAdobeEbx,
			"EBX_HANDLER",
		),
		(
			"<< /Size 4 /Encrypt 3 0 R >>\n3 0 obj\n<< /Filter /Adobe.APS >>",
			DrmScheme::PdfAdobeAps,
			"Adobe.APS",
		),
	] {
		let report = detect_pdf_drm(Cursor::new(pdf(trailer)))
			.unwrap()
			.expect("encrypted pdf");
		assert_eq!(report.container, DrmContainer::Pdf);
		assert_eq!(report.scheme, expected);
		assert_eq!(
			report.markers,
			vec![
				"trailer:/Encrypt".to_string(),
				format!("encrypt_dict:/Filter/{marker}"),
			]
		);
	}
}

#[test]
fn pdf_encrypt_with_an_inline_dictionary_is_detected() {
	let report = detect_pdf_drm(Cursor::new(pdf("<< /Encrypt << /V 1 >> /Size 4 >>")))
		.unwrap()
		.expect("encrypted pdf");
	assert_eq!(report.scheme, DrmScheme::PdfUnknownHandler);
	assert_eq!(report.markers, vec!["trailer:/Encrypt".to_string()]);
}

#[test]
fn a_plain_pdf_is_not_drm() {
	assert_eq!(
		detect_pdf_drm(Cursor::new(pdf("<< /Size 4 /Root 1 0 R >>"))).unwrap(),
		None
	);
}

#[test]
fn the_literal_text_encrypt_is_not_an_encrypt_entry() {
	// `/Encrypt` inside a content stream, with no dictionary or indirect
	// reference after it, must not fail an otherwise fine PDF.
	let bytes = pdf("<< /Size 4 /Root 1 0 R >>\n% see /Encrypt for details\n");
	assert_eq!(detect_pdf_drm(Cursor::new(bytes)).unwrap(), None);
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

#[test]
fn detect_drm_dispatches_on_container_magic_not_extension() {
	let dir = TempDir::new().expect("temp dir");

	let cases: Vec<(&str, Vec<u8>, Option<DrmScheme>)> = vec![
		(
			// A MOBI deliberately named `.epub`: classified by its bytes.
			"mislabelled.epub",
			MobiFixture {
				encryption_type: 2,
				..Default::default()
			}
			.build(),
			Some(DrmScheme::Mobipocket),
		),
		(
			"adobe.epub",
			epub_with(&[
				(
					"META-INF/encryption.xml",
					encryption_xml(&[AES_128]).as_slice(),
				),
				("META-INF/rights.xml", adept_rights(172).as_slice()),
			]),
			Some(DrmScheme::AdobeAdept),
		),
		(
			"locked.pdf",
			pdf("<< /Size 4 /Encrypt 3 0 R >>\n3 0 obj\n<< /Filter /Standard >>"),
			Some(DrmScheme::PdfStandardSecurity),
		),
		(
			"clean.cbz",
			zip_with(&[("0001.jpg", b"\xff\xd8\xff\xe0")]),
			None,
		),
		(
			"book.kfx",
			[DRMION_MAGIC, b"payload"].concat(),
			Some(DrmScheme::AmazonKfx),
		),
		(
			"old.azw1",
			[b"TPZ\x00".as_slice(), &[0u8; 64]].concat(),
			Some(DrmScheme::Topaz),
		),
		("random.bin", vec![7u8; 128], None),
	];

	for (name, bytes, expected) in cases {
		let path = dir.path().join(name);
		std::fs::write(&path, &bytes).expect("write fixture");
		let scheme = detect_drm(&path)
			.unwrap_or_else(|error| panic!("{name}: {error}"))
			.map(|report| report.scheme);
		assert_eq!(scheme, expected, "{name}");
	}
}

#[test]
fn detect_drm_reports_a_missing_file_as_an_error() {
	let error = detect_drm(std::path::Path::new("/nope/does/not/exist.epub"))
		.expect_err("missing file");
	assert!(matches!(error, FileError::FileIoError(_)), "{error}");
}
