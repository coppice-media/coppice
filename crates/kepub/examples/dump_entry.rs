//! Debug helper: transform an EPUB and print one entry to stdout.
//! Usage: cargo run -p stump_kepub --example dump_entry -- <book.epub> <entry name>
use std::io::{Read, Write};

fn main() {
	let args: Vec<String> = std::env::args().collect();
	let input = std::fs::read(&args[1]).expect("read input epub");
	let out =
		stump_kepub::transform_epub(&input, &stump_kepub::TransformOptions::default())
			.expect("transform");
	let mut zip = zip::ZipArchive::new(std::io::Cursor::new(out)).expect("zip");
	let mut entry = zip.by_name(&args[2]).expect("entry");
	let mut bytes = Vec::new();
	entry.read_to_end(&mut bytes).expect("entry bytes");
	std::io::stdout().write_all(&bytes).expect("stdout");
}
