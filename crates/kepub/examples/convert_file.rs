//! Convert one EPUB to KEPUB on disk, streaming the output to the file.
//! Usage: cargo run --release -p stump_kepub --example convert_file -- <in.epub> <out.kepub.epub>
use std::io::BufWriter;
fn main() {
	let args: Vec<String> = std::env::args().collect();
	let input = std::fs::read(&args[1]).expect("read input");
	let file = std::fs::File::create(&args[2]).expect("create output");
	let sink = BufWriter::with_capacity(1 << 16, file);
	stump_kepub::transform_epub_to(
		&input,
		&stump_kepub::TransformOptions::default(),
		&stump_kepub::WriteOptions::default(),
		sink,
	)
	.expect("transform");
}
