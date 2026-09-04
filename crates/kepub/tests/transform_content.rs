//! Verbatim ports of kepubify's `TestTransformContent` and
//! `TestTransformContentParts` from `kepub/transform_test.go` at
//! 9546034bc023891af5ce30709de6ae2dcf264628.
//!
//! TransformOptions fields referenced here: `smarten_punctuation`, `charset`,
//! `extra_css`, and `find_replace`.

use stump_kepub::{parts, transform_content, TransformOptions};

// Mirrors transformContentCase.Run's fragment wrapping (transform_test.go:941-944).
fn fragment_input(input: &str, fragment: bool) -> Vec<u8> {
	if fragment {
		format!(
            "<!DOCTYPE html><head><title>Kepubify Test Case</title><meta charset=\"utf-8\"/></head><body>{input}</body></html>"
        )
        .into_bytes()
	} else {
		input.as_bytes().to_vec()
	}
}

// Mirrors the runner's body extraction (transform_test.go:978-981).
fn body_fragment(rendered: &[u8], fragment: bool) -> String {
	let rendered = String::from_utf8(rendered.to_vec()).expect("rendered HTML is UTF-8");
	if !fragment {
		return rendered;
	}
	let start =
		rendered.find("<body>").expect("rendered document has body") + "<body>".len();
	let end = rendered[start..]
		.find("</body>")
		.map(|offset| start + offset)
		.expect("rendered document has body close");
	rendered[start..end].to_owned()
}

fn run_part_case<F>(
	name: &str,
	fragment: bool,
	contains: bool,
	input: &str,
	expected: &str,
	f: F,
) where
	F: Fn(&[u8]) -> Vec<u8>,
{
	let rendered = f(&fragment_input(input, fragment));
	let actual = body_fragment(&rendered, fragment);
	if contains {
		assert!(
			actual.contains(expected),
			"case {name:?}: got `{actual}` does not contain `{expected}`"
		);
	} else {
		assert_eq!(actual, expected, "case {name:?}");
	}
}

#[test]
fn test_transform_content() {
	let options = TransformOptions {
		extra_css: vec![(
			"kepubify-test".to_owned(),
			"body { color: black; }".to_owned(),
		)],
		find_replace: vec![
			(
				"Test sentence 2.".to_owned(),
				"Replaced sentence 2.".to_owned(),
			),
			("<a id=\"test\">".to_owned(), "<a id=\"test1\">".to_owned()),
			("sdfsdfsdf".to_owned(), "dfgdfgdfg".to_owned()),
			(" sentence 2".to_owned(), String::new()),
		],
		smarten_punctuation: true,
		..TransformOptions::default()
	};
	let input = r##"
<?xml version="1.0" charset="utf-8"?>
<!DOCTYPE html>
<html>
<head>
    <title>Kepubify Test</title>
    <meta charset="utf-8">
    <!-- Note: this tests a few of the extended features from my fork of
         x/net/html, but this isn't the focus. Those features are fully tested
         in my tests for that library. -->
</head>
<body>
    <p>Test sentence 1. <a id="test"/> Test sentence 2. <b>Test sentence 3<i>Test sentence 4</p>
    <p>Test sentence 5. <i>"This is quoted"</i> -- <b>and this is not</b>.</p>
    <p>Sentence.<ul><li>Another sentence.</li><li>Another sentence.<ul><li>Another sentence.</li><li>Another sentence.</li></ul></li><li>Another sentence.</li></ul> Another sentence.</p>
    <pre>Test
</pre>
    <table borders><tr><td>test</td></tr></table>
    <p>  </p>
    <p></p>
    <img src="test">
    <p>&nbsp;</p>
    <svg></svg>
</body>
</html>"##
    .trim();
	let expected = r##"
<?xml version="1.0" charset="utf-8"?><!DOCTYPE html><html xmlns="http://www.w3.org/1999/xhtml"><head>
    <title>Kepubify Test</title>
    <meta charset="utf-8"/>
    <!-- Note: this tests a few of the extended features from my fork of
         x/net/html, but this isn't the focus. Those features are fully tested
         in my tests for that library. -->
<style type="text/css" class="kobostylehacks">div#book-inner { margin-top: 0; margin-bottom: 0;}</style><style type="text/css" class="kepubify-test">body { color: black; }</style></head>
<body><div id="book-columns"><div id="book-inner">
    <p><span class="koboSpan" id="kobo.1.1">Test sentence 1. </span><a id="test1"></a><span class="koboSpan" id="kobo.1.2"> Replaced. </span><b><span class="koboSpan" id="kobo.1.3">Test sentence 3</span><i><span class="koboSpan" id="kobo.1.4">Test sentence 4</span></i></b></p><b><i>
    <p><span class="koboSpan" id="kobo.2.1">Test sentence 5. </span><i><span class="koboSpan" id="kobo.2.2">“This is quoted”</span></i><span class="koboSpan" id="kobo.2.3"> – </span><b><span class="koboSpan" id="kobo.2.4">and this is not</span></b><span class="koboSpan" id="kobo.2.5">.</span></p>
    <p><span class="koboSpan" id="kobo.3.1">Sentence.</span></p><ul><li><span class="koboSpan" id="kobo.4.1">Another sentence.</span></li><li><span class="koboSpan" id="kobo.4.2">Another sentence.</span><ul><li><span class="koboSpan" id="kobo.5.1">Another sentence.</span></li><li><span class="koboSpan" id="kobo.5.2">Another sentence.</span></li></ul></li><li><span class="koboSpan" id="kobo.5.3">Another sentence.</span></li></ul><span class="koboSpan" id="kobo.5.4"> Another sentence.</span><p></p>
    <pre>Test
</pre>
    <table borders=""><tbody><tr><td><span class="koboSpan" id="kobo.6.1">test</span></td></tr></tbody></table>
    <p><span class="koboSpan" id="kobo.7.1">  </span></p>
    <p></p>
    <span class="koboSpan" id="kobo.8.1"><img src="test"/></span>
    <p><span class="koboSpan" id="kobo.9.1">&#160;</span></p>
    <svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"></svg>

</i></b></div></div></body></html>"##
    .trim();
	let actual = String::from_utf8(
		transform_content(input.as_bytes(), &options).expect("transform"),
	)
	.expect("transformed HTML is UTF-8");
	assert_eq!(actual.trim(), expected, "TestTransformContent");
}

#[test]
fn test_transform_content_parts() {
	// Charset
	run_part_case(
		"charset meta element",
		false,
		true,
		r##"<!DOCTYPE html><head><title>Kepubify Test Case</title><meta charset="iso-8859-1"/></head><body></body></html>"##,
		r##"meta charset="UTF-8""##,
		parts::charset_utf8,
	);
	run_part_case(
		"charset meta element (case-insensitive, unchanged)",
		false,
		true,
		r##"<!DOCTYPE html><head><title>Kepubify Test Case</title><meta charset="UTf-8"/></head><body></body></html>"##,
		r##"meta charset="UTf-8""##,
		parts::charset_utf8,
	);
	run_part_case(
		"http-equiv content-type charset meta element",
		false,
		true,
		r##"<!DOCTYPE html><head><title>Kepubify Test Case</title><meta http-equiv="content-type" content="application/xhtml+xml; charset=iso-8859-1"/></head><body></body></html>"##,
		r##"meta http-equiv="content-type" content="application/xhtml+xml; charset=utf-8""##,
		parts::charset_utf8,
	);
	run_part_case(
		"http-equiv content-type charset meta element (case-insensitive, unchanged)",
		false,
		true,
		r##"<!DOCTYPE html><head><title>Kepubify Test Case</title><meta http-equiv="content-type" content="application/xhtml+xml; charset=UTf-8"/></head><body></body></html>"##,
		r##"meta http-equiv="content-type" content="application/xhtml+xml; charset=UTf-8""##,
		parts::charset_utf8,
	);

	// KoboStyles
	run_part_case(
		"add kobo style hacks",
		false,
		true,
		r##"<!DOCTYPE html><head><title>Kepubify Test Case</title><meta charset="utf-8"/></head><body></body></html>"##,
		"kobostylehacks",
		parts::kobo_styles,
	);

	// KoboDivs
	run_part_case(
		"no content",
		true,
		false,
		"",
		r##"<div id="book-columns"><div id="book-inner"></div></div>"##,
		parts::kobo_divs,
	);
	run_part_case(
		"already has divs",
		true,
		false,
		r##"<div id="book-columns"><div id="book-inner"></div></div>"##,
		r##"<div id="book-columns"><div id="book-inner"></div></div>"##,
		parts::kobo_divs,
	);
	run_part_case(
		"single text node",
		true,
		false,
		"test",
		r##"<div id="book-columns"><div id="book-inner">test</div></div>"##,
		parts::kobo_divs,
	);
	run_part_case(
		"multiple elements and children",
		true,
		false,
		r##"<p>Test 1</p><p>Test <b>2</b></p><p>Test 3</p>"##,
		r##"<div id="book-columns"><div id="book-inner"><p>Test 1</p><p>Test <b>2</b></p><p>Test 3</p></div></div>"##,
		parts::kobo_divs,
	);

	// KoboSpans
	run_part_case("no content", true, false, "", "", parts::kobo_spans);
	run_part_case(
		"already has spans",
		true,
		false,
		r##"<p><span class="koboSpan" id="kobo.1.1">Test</span></p>"##,
		r##"<p><span class="koboSpan" id="kobo.1.1">Test</span></p>"##,
		parts::kobo_spans,
	);
	run_part_case(
		"increment segment counter from 1 for every sentence",
		true,
		false,
		r##"<p>Sentence 1. Sentence 2. Sentence 3.</p>"##,
		r##"<p><span class="koboSpan" id="kobo.1.1">Sentence 1. </span><span class="koboSpan" id="kobo.1.2">Sentence 2. </span><span class="koboSpan" id="kobo.1.3">Sentence 3.</span></p>"##,
		parts::kobo_spans,
	);
	run_part_case(
        "increment paragraph counter from 1 and reset segment counter for every p, ul, ol, or table",
        true,
        false,
        r##"<p>Sentence 1. Sentence 2.</p><p>Sentence 3.</p><ul><li>Sentence 4</li><li>Sentence 5</li></ul><ol><li>Sentence 6</li><li>Sentence 7</li></ol><table><tbody><tr><td>Test</td></tr><tr><td>Test</td></tr></tbody></table>"##,
        r##"<p><span class="koboSpan" id="kobo.1.1">Sentence 1. </span><span class="koboSpan" id="kobo.1.2">Sentence 2.</span></p><p><span class="koboSpan" id="kobo.2.1">Sentence 3.</span></p><ul><li><span class="koboSpan" id="kobo.3.1">Sentence 4</span></li><li><span class="koboSpan" id="kobo.3.2">Sentence 5</span></li></ul><ol><li><span class="koboSpan" id="kobo.4.1">Sentence 6</span></li><li><span class="koboSpan" id="kobo.4.2">Sentence 7</span></li></ol><table><tbody><tr><td><span class="koboSpan" id="kobo.5.1">Test</span></td></tr><tr><td><span class="koboSpan" id="kobo.5.2">Test</span></td></tr></tbody></table>"##,
        parts::kobo_spans,
    );
	run_part_case(
        "merge stray text at the end of lines into the next sentence (between regexp matches)",
        true,
        false,
        concat!(r##"<p>Sentence 1. Sentence 2. Stray text"##, "\n", r##"Another sentence.</p>"##),
        concat!(r##"<p><span class="koboSpan" id="kobo.1.1">Sentence 1. </span><span class="koboSpan" id="kobo.1.2">Sentence 2. </span><span class="koboSpan" id="kobo.1.3">Stray text"##, "\n", r##"Another sentence.</span></p>"##),
        parts::kobo_spans,
    );
	run_part_case(
		"don't lose stray text not part of a sentence (after the last regexp match)",
		true,
		false,
		r##"<p>Sentence 1. Sentence 2. Stray text</p>"##,
		r##"<p><span class="koboSpan" id="kobo.1.1">Sentence 1. </span><span class="koboSpan" id="kobo.1.2">Sentence 2. </span><span class="koboSpan" id="kobo.1.3">Stray text</span></p>"##,
		parts::kobo_spans,
	);
	run_part_case(
		"preserve but don't wrap extra whitespace outside of P elements",
		true,
		false,
		concat!(
			r##"<p>This is a test."##,
			"\n",
			r##"    This is another sentence on the next line.<span> </span>Another sentence.</p>"##,
			"\n",
			"    <p>Another paragraph.</p><p> </p><p></p>"
		),
		concat!(
			r##"<p><span class="koboSpan" id="kobo.1.1">This is a test."##,
			"\n",
			r##"    </span><span class="koboSpan" id="kobo.1.2">This is another sentence on the next line.</span><span> </span><span class="koboSpan" id="kobo.1.3">Another sentence.</span></p>"##,
			"\n",
			r##"    <p><span class="koboSpan" id="kobo.2.1">Another paragraph.</span></p><p><span class="koboSpan" id="kobo.3.1"> </span></p><p></p>"##
		),
		parts::kobo_spans,
	);
	run_part_case(
		"preserve and split segments on formatting and links",
		true,
		false,
		r##"<p>Sentence<b> 1. </b>Sentence <span>2. Se</span>nten<a href="test.html">ce 3. Another word</a></p>"##,
		r##"<p><span class="koboSpan" id="kobo.1.1">Sentence</span><b><span class="koboSpan" id="kobo.1.2"> 1. </span></b><span class="koboSpan" id="kobo.1.3">Sentence </span><span><span class="koboSpan" id="kobo.1.4">2. </span><span class="koboSpan" id="kobo.1.5">Se</span></span><span class="koboSpan" id="kobo.1.6">nten</span><a href="test.html"><span class="koboSpan" id="kobo.1.7">ce 3. </span><span class="koboSpan" id="kobo.1.8">Another word</span></a></p>"##,
		parts::kobo_spans,
	);
	run_part_case(
		"preserve and split segments on nested formatting and links",
		true,
		false,
		r##"<p>Sentence<b> 1. Sente<i>nce <span>2. Se</span>nt</i>en<a href="test.html">ce 3. Another word</a></b></p>"##,
		r##"<p><span class="koboSpan" id="kobo.1.1">Sentence</span><b><span class="koboSpan" id="kobo.1.2"> 1. </span><span class="koboSpan" id="kobo.1.3">Sente</span><i><span class="koboSpan" id="kobo.1.4">nce </span><span><span class="koboSpan" id="kobo.1.5">2. </span><span class="koboSpan" id="kobo.1.6">Se</span></span><span class="koboSpan" id="kobo.1.7">nt</span></i><span class="koboSpan" id="kobo.1.8">en</span><a href="test.html"><span class="koboSpan" id="kobo.1.9">ce 3. </span><span class="koboSpan" id="kobo.1.10">Another word</span></a></b></p>"##,
		parts::kobo_spans,
	);
	run_part_case(
        "nested lists/paragraphs should not reset numbering once out of scope (i.e. <p [para1]>[span1.1]<ul [para2]><li>[span2.1]</li></ul>[span2.2]</p>)",
        true,
        false,
        r##"<p>Sentence.<ul><li>Another sentence.</li><li>Another sentence.<ul><li>Another sentence.</li><li>Another sentence.</li></ul></li><li>Another sentence.</li></ul> Another sentence.</p>"##,
        r##"<p><span class="koboSpan" id="kobo.1.1">Sentence.</span></p><ul><li><span class="koboSpan" id="kobo.2.1">Another sentence.</span></li><li><span class="koboSpan" id="kobo.2.2">Another sentence.</span><ul><li><span class="koboSpan" id="kobo.3.1">Another sentence.</span></li><li><span class="koboSpan" id="kobo.3.2">Another sentence.</span></li></ul></li><li><span class="koboSpan" id="kobo.3.3">Another sentence.</span></li></ul><span class="koboSpan" id="kobo.3.4"> Another sentence.</span><p></p>"##,
        parts::kobo_spans,
    );
	run_part_case(
		"don't touch the contents of script, style, pre, audio, video tags",
		true,
		false,
		r##"<p>Touch this.</p><script>not this</script><style>or this</style><pre>or this</pre><audio>or this</audio><video>or this</video><p>Touch this.</p>"##,
		r##"<p><span class="koboSpan" id="kobo.1.1">Touch this.</span></p><script>not this</script><style>or this</style><pre>or this</pre><audio>or this</audio><video>or this</video><p><span class="koboSpan" id="kobo.2.1">Touch this.</span></p>"##,
		parts::kobo_spans,
	);
	run_part_case(
		"treat an img as a new paragraph and add a span around it",
		true,
		false,
		r##"<p>One.</p><img src="test"><p>Three.</p>"##,
		r##"<p><span class="koboSpan" id="kobo.1.1">One.</span></p><span class="koboSpan" id="kobo.2.1"><img src="test"/></span><p><span class="koboSpan" id="kobo.3.1">Three.</span></p>"##,
		parts::kobo_spans,
	);
	run_part_case(
		"don't increment paragraph counter if no spans were added",
		true,
		false,
		r##"<p>One.</p><p> </p><p><!-- comment --></p><p>Two.</p><p><b>Three.</b></p>"##,
		r##"<p><span class="koboSpan" id="kobo.1.1">One.</span></p><p><span class="koboSpan" id="kobo.2.1"> </span></p><p><!-- comment --></p><p><span class="koboSpan" id="kobo.3.1">Two.</span></p><p><b><span class="koboSpan" id="kobo.4.1">Three.</span></b></p>"##,
		parts::kobo_spans,
	);
	run_part_case(
		"don't add spans to svg and math elements",
		true,
		false,
		r##"<svg xmlns="http://www.w3.org/2000/svg"><g><text font-size="24" y="20" x="0">kepubify</text></g></svg><math xmlns="http://www.w3.org/1998/Math/MathML"><mi>x</mi><mo>=</mo><mfrac><mrow><mo>-</mo><mi>b</mi><mo>±</mo><msqrt><msup><mi>b</mi><mn>2</mn></msup><mo>-</mo><mn>4</mn><mi>a</mi><mi>c</mi></msqrt></mrow><mrow><mn>2</mn><mi>a</mi></mrow></mfrac></math>"##,
		r##"<svg xmlns="http://www.w3.org/2000/svg"><g><text font-size="24" y="20" x="0">kepubify</text></g></svg><math xmlns="http://www.w3.org/1998/Math/MathML"><mi>x</mi><mo>=</mo><mfrac><mrow><mo>-</mo><mi>b</mi><mo>±</mo><msqrt><msup><mi>b</mi><mn>2</mn></msup><mo>-</mo><mn>4</mn><mi>a</mi><mi>c</mi></msqrt></mrow><mrow><mn>2</mn><mi>a</mi></mrow></mfrac></math>"##,
		parts::kobo_spans,
	);
	run_part_case(
        "also increment paragraph counter on heading tags and don't split sentences after colons or if there isn't any spaces after a period",
        true,
        false,
        concat!(r##"<div class="img_container"><p class="ad_image"><img src="bookwire_ad_cover1.jpg" alt="image"/></p></div>"##, "\n", r##"    <h2 class="subheadline">The Christmas Collection: All Of Your Favourite Classic Christmas Stories, Novels, Poems, Carols in One Ebook</h2>"##, "\n", r##"    <p class="subheadline2"></p>"##, "\n", r##"    <p class="metadata">Carr, Annie Roe</p>"##),
        concat!(r##"<div class="img_container"><p class="ad_image"><span class="koboSpan" id="kobo.1.1"><img src="bookwire_ad_cover1.jpg" alt="image"/></span></p></div>"##, "\n", r##"    <h2 class="subheadline"><span class="koboSpan" id="kobo.2.1">The Christmas Collection: All Of Your Favourite Classic Christmas Stories, Novels, Poems, Carols in One Ebook</span></h2>"##, "\n", r##"    <p class="subheadline2"></p>"##, "\n", r##"    <p class="metadata"><span class="koboSpan" id="kobo.3.1">Carr, Annie Roe</span></p>"##),
        parts::kobo_spans,
    );

	// AddStyle
	run_part_case(
		"add style to head",
		false,
		false,
		r##"<!DOCTYPE html><html><head><title>Kepubify Test</title></head><body></body></html>"##,
		r##"<!DOCTYPE html><html><head><title>Kepubify Test</title><style type="text/css" class="kepubify-test">div > div { color: black; }</style></head><body></body></html>"##,
		|input| parts::add_style(input, "kepubify-test", "div > div { color: black; }"),
	);

	// SmartyPants
	run_part_case(
		"smart punctuation",
		true,
		false,
		r##"<p>This is a test sentence to test smartypants' conversion of "quotation marks", dashes like - / -- / ---, and symbols like (c).</p>"##,
		r##"<p>This is a test sentence to test smartypants’ conversion of “quotation marks”, dashes like - / – / —, and symbols like ©.</p>"##,
		parts::smarten_punctuation,
	);
	run_part_case(
		"skip pre, code, style, and script elements",
		true,
		false,
		r##"<p>This is a test sentence to test smartypants' conversion of <code>"quotation marks"</code>, dashes like <pre>- / -- / ---</pre>, and symbols like (c).</p><style>div{font-family:"Test"}</style><script>var a="test"</script>"##,
		r##"<p>This is a test sentence to test smartypants’ conversion of <code>&#34;quotation marks&#34;</code>, dashes like </p><pre>- / -- / ---</pre>, and symbols like ©.<p></p><style>div{font-family:"Test"}</style><script>var a="test"</script>"##,
		parts::smarten_punctuation,
	);
	run_part_case(
		"properly handle entity escaping",
		true,
		false,
		r##"<p>&amp;&quot;&lt;&gt;&quot;</p><pre>&quot;</pre>"##,
		r##"<p>&amp;“&lt;&gt;”</p><pre>&#34;</pre>"##,
		parts::smarten_punctuation,
	);

	// CleanHTML
	run_part_case(
		"remove adobe adept metadata",
		true,
		false,
		r##"<meta charset="utf-8"/><meta name="Adept.expected.resource"/>"##,
		r##"<meta charset="utf-8"/>"##,
		parts::clean_html,
	);
	run_part_case(
		"remove useless MS Word tags",
		true,
		false,
		r##"<o:p></o:p><st1:test></st1:test><div><o:p> dfg </o:p><o:p></o:p></div>"##,
		r##"<div><o:p> dfg </o:p></div>"##,
		parts::clean_html,
	);
	run_part_case(
		"remove unicode replacement chars",
		true,
		false,
		r##"�<p>test�ing</p><p>asd<b>fgh</b></p>"##,
		r##"<p>testing</p><p>asd<b>fgh</b></p>"##,
		parts::clean_html,
	);

	// Replacements
	let corpus = r##"<!DOCTYPE html><html><head><title></title></head><body><b>Lorem ipsum</b> dolor sit amet, <a href="https://example.com">consectetur adipiscing elit</a>, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.</body></html>"##;
	let replacement_cases: &[(&str, &[&str])] = &[
		("simple removal", &[" ipsum", ""]),
		("simple replacement", &[". ", "_ "]),
		("complex removal", &["<b>Lorem ipsum</b>", ""]),
		(
			"simple chained",
			&["ipsum", "test1", "amet", "test2", "commodo", ""],
		),
		(
			"ordered chained",
			&["ipsum", "Lorem", "Lorem", "test1", "ipsum", ""],
		),
		(
			"overlapping chained",
			&[
				"Lorem",
				"test1",
				"test1",
				"test2",
				"ipsum",
				"test2 ipsum",
				"test2",
				"test3",
			],
		),
		(
			"complex chained",
			&[
				"Lorem",
				"ipsum",
				"or",
				"ar",
				"dolar",
				"dolor",
				"</",
				"__________",
				"________",
				"__</",
				"_",
				" ",
				". ",
				"; ",
			],
		),
	];
	for (name, replacements) in replacement_cases {
		let mut expected = corpus.to_owned();
		for pair in replacements.chunks_exact(2) {
			expected = expected.replace(pair[0], pair[1]);
		}
		assert_ne!(expected, corpus, "strings don't differ: {name}");
		let options = replacements
			.chunks_exact(2)
			.map(|pair| (pair[0].to_owned(), pair[1].to_owned()))
			.collect::<Vec<_>>();
		let actual = parts::replacements(corpus.as_bytes(), &options);
		assert_eq!(actual, expected.as_bytes(), "case {name:?}");
	}
	let long_replacement = ".".repeat(4096);
	let expected = corpus.replace("ipsum", &long_replacement);
	let actual =
		parts::replacements(corpus.as_bytes(), &[("ipsum".to_owned(), long_replacement)]);
	assert_eq!(actual, expected.as_bytes(), "case {:?}", "long replacement");
}
