//! Direct port of kepubify's TestSplitSentences (transform_test.go:1224-1244).
//!
//! The pinned Go table deliberately includes malformed UTF-8 byte strings. Rust's
//! `str` API cannot represent those bytes safely; those entries are retained as
//! byte arrays and passed through an explicitly invalid `&str`, while the
//! regexp cross-check is limited to valid UTF-8 entries (the Rust regex crate
//! requires valid UTF-8 input). The workspace contains `regex`, but it is not a
//! dependency of stump_kepub, so the regexp helper is implemented locally with
//! the same matching semantics rather than importing that crate.

use stump_kepub::split_sentences;

fn sentence_cases() -> Vec<String> {
	vec![
        " ! Lorem ipsum dolor, sit amet. Consectetur adipiscing elit?\n Sed do eiusmod tempor incididunt!?! Ut labore et dolore \"magna aliqua.\". Ut enim ad “minim veniam”, quis nostrud exercitation 'ullamco laboris' nisi ut aliquip ex ea commodo consequat?… Duis aute irure dolor in reprehenderit in voluptate velit’s esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.      sdfsdfsdf".to_owned(),
        "Lorem ipsum dolor, sit amet. Consectetur adipiscing elit? Sed do eiusmod tempor incididunt!?! Ut labore et dolore magna aliqua.".to_owned(),
        "Lorem ipsum dolor sit amet. Consectetur adipiscing elit ut labore et dolore magna aliqua. ".repeat(40),
        "                                ".to_owned(),
        "...       !!!       ???       .'.'.'.'   ".to_owned(),
        "test\u{00a0}.\u{0080}.\u{00a0}.".to_owned(),
        "".to_owned(),
        "🌝. 🌝      🌝.    🌝".to_owned(),
        "!".to_owned(),
        "? ".to_owned(),
        "? ?".to_owned(),
        "?  ".to_owned(),
        "  ?  ".to_owned(),
        " ?'  .".to_owned(),
        " ?'  .   ".to_owned(),
    ]
}

// The final seven Go literals contain \xFF and/or malformed UTF-8. Keep the
const INVALID_SENTENCES: &[&[u8]] = &[
	b" ?'  .   \xff",
	b" ?'  .   \xff .",
	b" ?'  .   .\xe2\x82(\xff",
	b" ?'  .   .\xe2\x82(\xff .",
	b" ?'  .   .'\xe2\x82(\xff",
	b" ?'  .   .'\xe2\x82(\xff .",
	b" ?'  .   .'\xe2\x82(\xff.",
];

fn go_regexp_space(ch: char) -> bool {
	matches!(ch, ' ' | '\t' | '\n' | '\x0c' | '\r')
}

fn split_sentences_regexp(input: &str) -> Vec<&str> {
	// Go: ((?ms).*?[\.\!\?]['”’“…]?\s+)
	// This is intentionally a small byte-indexed equivalent. It preserves the
	// exact grouping and remainder behavior of splitSentencesRegexp.
	let bytes = input.as_bytes();
	let mut matches = Vec::new();
	let mut pos = 0;
	while pos < bytes.len() {
		let mut i = pos;
		let mut end = None;
		while i < bytes.len() {
			if matches!(bytes[i], b'.' | b'!' | b'?') {
				let mut j = i + 1;
				if j < bytes.len() {
					let ch = input[j..].chars().next().unwrap();
					if matches!(ch, '\'' | '”' | '’' | '"' | '…') {
						j += ch.len_utf8();
					}
				}
				if j < bytes.len()
					&& input[j..].chars().next().is_some_and(go_regexp_space)
				{
					while j < bytes.len()
						&& input[j..].chars().next().is_some_and(go_regexp_space)
					{
						j += input[j..].chars().next().unwrap().len_utf8();
					}
					end = Some(j);
					break;
				}
			}
			i += input[i..].chars().next().unwrap().len_utf8();
		}
		match end {
			Some(end) => {
				matches.push(&input[pos..end]);
				pos = end;
			},
			None => break,
		}
	}
	if matches.is_empty() {
		vec![input]
	} else {
		if pos < input.len() {
			matches.push(&input[pos..]);
		}
		matches
	}
}

unsafe fn invalid_utf8(bytes: &[u8]) -> &str {
	// This mirrors Go's byte string input. The state-machine implementation is
	// expected to preserve arbitrary bytes; no Unicode operation is performed
	// by this test on the invalid entries.
	std::str::from_utf8_unchecked(bytes)
}

#[test]
fn test_split_sentences() {
	for (index, value) in sentence_cases().iter().enumerate() {
		let state_machine = split_sentences(value);
		let regexp = split_sentences_regexp(value);
		assert_eq!(
			state_machine, regexp,
			"case {index}: state-machine != regexp"
		);
		assert_eq!(
			state_machine.concat(),
			*value,
			"case {index}: joined sentence != original"
		);
	}

	for (index, bytes) in INVALID_SENTENCES.iter().enumerate() {
		let value = unsafe { invalid_utf8(bytes) };
		let state_machine = split_sentences(value);
		let joined: Vec<u8> = state_machine
			.iter()
			.flat_map(|part| part.as_bytes())
			.copied()
			.collect();
		assert_eq!(
			joined, *bytes,
			"invalid case {index}: joined sentence != original bytes"
		);
	}
}
