use stump_kepub::{parts, transform_content, TransformOptions};

fn render_default(input: &str) -> String {
	String::from_utf8(parts::replacements(input.as_bytes(), &[]))
		.expect("fork output is UTF-8")
}

fn render_transformed(input: &str) -> String {
	String::from_utf8(
		transform_content(input.as_bytes(), &TransformOptions::default())
			.expect("transform"),
	)
	.expect("fork output is UTF-8")
}

#[test]
fn fork_lenient_self_closing_elements() {
	let cases = [
		(
			"title in head",
			"<!DOCTYPE html><html><head><title/></head><body><p>Test 1</p></body></html>",
			"<title></title>",
		),
		(
			"title in head with neighbours",
			"<!DOCTYPE html><html><head><base href=\"/\"/><title/><meta charset=\"asd\"/></head><body><p>Test 1</p></body></html>",
			"<title></title><meta charset=\"UTF-8\"/>",
		),
		(
			"title in body",
			"<!DOCTYPE html><html><head></head><body><title/><p>Test 1</p></body></html>",
			"<title></title><p><span",
		),
		(
			"script in head",
			"<!DOCTYPE html><html><head><title>Title</title><script src=\"script.js\"/></head><body><p>Test 1</p></body></html>",
			"<script src=\"script.js\" type=\"text/javascript\"></script>",
		),
		(
			"script in head with neighbours",
			"<!DOCTYPE html><html><head><title>Title</title><base href=\"/\"/><script src=\"script.js\"/><meta charset=\"asd\"/></head><body><p>Test 1</p></body></html>",
			"<script src=\"script.js\" type=\"text/javascript\"></script>",
		),
		(
			"script in body",
			"<!DOCTYPE html><html><head><title>Title</title></head><body><script src=\"script.js\"/><p>Test 1</p></body></html>",
			"<script src=\"script.js\" type=\"text/javascript\"></script>",
		),
		(
			"a in body",
			"<!DOCTYPE html><html><head><title>Title</title></head><body><p>Test <a id=\"test\"/> 1</p></body></html>",
			"<a id=\"test\"></a>",
		),
		(
			"multiple a in body",
			"<!DOCTYPE html><html><head><title>Title</title></head><body><p>Test <a id=\"test\"/><a id=\"test1\"/> 1</p></body></html>",
			"<a id=\"test\"></a><a id=\"test1\"></a>",
		),
		(
			"a with escaped text",
			"<!DOCTYPE html><html><head><title>Title</title></head><body><p>Test &gt;&lt;<a id=\"test\"/>&gt;&lt; 1<span>test</span></p><p>Test 2</p></body></html>",
			"<a id=\"test\"></a><span",
		),
		(
			"p and div",
			"<!DOCTYPE html><html><head><title>Title</title></head><body><p>Test</p><span>Test</span><p/><span>Test</span><p>Test</p><div/><span>After</span></body></html>",
			"<p></p><span><span",
		),
	];

	for (name, input, marker) in cases {
		let output = render_transformed(input);
		assert!(output.contains(marker), "fork parser case {name}: {output}");
	}
}

#[test]
fn fork_ignore_bom_and_preserve_xml_declaration() {
	let bom = "\u{feff}<!-- Comment Text --><!DOCTYPE html><html><head><title>Title</title></head><body><p>Test 1</p></body></html>";
	assert_eq!(
        render_default(bom),
        "<!-- Comment Text --><!DOCTYPE html><html><head><title>Title</title></head><body><p>Test 1</p></body></html>"
    );

	let declaration = "<?xml version='1.0' encoding='utf-8'?><html><head><title>Title</title></head><body><p>Test 1</p></body></html>";
	assert_eq!(
        render_default(declaration),
        "<?xml version='1.0' encoding='utf-8'?><html><head><title>Title</title></head><body><p>Test 1</p></body></html>"
    );

	let processing_instruction = "<?xml version='1.0' encoding='utf-8'?><html><head><title>Title</title></head><body><?xml-stylesheet type=\"text/xsl\" href=\"style.xsl\"?><p>Test 1</p></body></html>";
	assert_eq!(
		render_default(processing_instruction),
		"<?xml version='1.0' encoding='utf-8'?><html><head><title>Title</title></head><body><!--?xml-stylesheet type=\"text/xsl\" href=\"style.xsl\"?--><p>Test 1</p></body></html>"
	);
}

#[test]
fn fork_text_and_attribute_escaping() {
	let input = "<!DOCTYPE html><html><head><title>Tom's \"book\"</title></head><body><p>Tom's \"book\" &amp; more</p><div data-value=\"Tom's &amp; book\"></div></body></html>";
	let output = render_default(input);
	assert!(output.contains("<title>Tom&#39;s &#34;book&#34;</title>"));
	assert!(output.contains("<p>Tom&#39;s &#34;book&#34; &amp; more</p>"));
	assert!(output.contains("data-value=\"Tom&#39;s &amp; book\""));
}

#[test]
fn fork_polyglot_renderer_modifications() {
	let input = "<!DOCTYPE html><html><head><title>Title</title><style></style><script></script></head><body>&nbsp;<input type=\"text\" enabled/><math></math><svg></svg></body></html>";
	let output = String::from_utf8(
		transform_content(input.as_bytes(), &TransformOptions::default())
			.expect("transform"),
	)
	.expect("fork output is UTF-8");

	assert!(output.contains("<html xmlns=\"http://www.w3.org/1999/xhtml\">"));
	assert!(output.contains("<style type=\"text/css\">"));
	assert!(output.contains("<script type=\"text/javascript\">"));
	assert!(output.contains("&#160;"));
	assert!(output.contains("<input type=\"text\" enabled=\"\"/>"));
	assert!(output.contains("<math xmlns=\"http://www.w3.org/1998/Math/MathML\">"));
	assert!(output.contains("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\">") );
}
