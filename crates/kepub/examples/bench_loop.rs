//! Convert one EPUB N times in-process (for profiling / timing without process overhead).
//! Usage: bench_loop <book.epub> [iterations]
use std::time::Instant;
fn main() {
	let args: Vec<String> = std::env::args().collect();
	let input = std::fs::read(&args[1]).expect("read input");
	let iterations: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(20);
	let options = stump_kepub::TransformOptions::default();
	let _ = stump_kepub::transform_epub(&input, &options).expect("warm-up");
	let started = Instant::now();
	let mut bytes = 0usize;
	for _ in 0..iterations {
		bytes += stump_kepub::transform_epub(&input, &options)
			.expect("transform")
			.len();
	}
	let per = started.elapsed().as_secs_f64() * 1000.0 / iterations as f64;
	println!("{per:.2} ms/iter ({bytes} bytes total)");
}
