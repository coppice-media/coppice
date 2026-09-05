pub mod analysis;
mod builder;
pub mod visible_pages;

pub(crate) use builder::{BuiltMedia, MediaBuilder};

#[cfg(test)]
pub(crate) mod tests {
	use std::path::PathBuf;

	fn fixture(name: &str) -> String {
		PathBuf::from(env!("CARGO_MANIFEST_DIR"))
			.join("integration-tests/data")
			.join(name)
			.to_string_lossy()
			.to_string()
	}

	pub fn get_test_zip_path() -> String {
		fixture("book.zip")
	}

	pub fn get_test_rar_path() -> String {
		fixture("book.rar")
	}

	pub fn get_test_epub_path() -> String {
		fixture("book.epub")
	}

	pub fn get_test_pdf_path() -> String {
		fixture("rust_book.pdf")
	}

	pub fn get_test_cbz_path() -> String {
		fixture("science_comics_001.cbz")
	}
}
