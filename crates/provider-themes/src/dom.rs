//! A flat, `Send` HTML document tree.
//!
//! `html5ever` gives a spec-compliant tree builder; its reference DOM is
//! `Rc`-based and therefore `!Send`, which cannot cross an `.await`. Every
//! document here is flattened into an index arena the moment it is parsed, so
//! parsing stays synchronous and the extracted data is what travels.
//!
//! The accessors mirror jsoup, because the selector tables and extraction rules
//! the definitions carry were written against jsoup: `text()` is normalised
//! whitespace over descendants, `own_text()` skips element children, `data()` is
//! the raw text of `<script>`/`<style>`, and `abs_attr()` resolves an attribute
//! against the document URL the way jsoup's `abs:` prefix does.

use std::rc::Rc;

use html5ever::{parse_document, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// Index into [`Document::nodes`]. Node 0 is always the document root.
pub type NodeId = u32;

const NO_NODE: NodeId = u32::MAX;

#[derive(Debug)]
pub struct Element {
	/// Lowercase local tag name.
	pub name: String,
	/// Attributes with lowercase names, in document order.
	pub attrs: Vec<(String, String)>,
	/// Lowercase class tokens; jsoup matches classes case-insensitively.
	pub classes: Vec<String>,
	/// 1-based position among the parent's element children.
	pub sibling_index: u32,
}

#[derive(Debug)]
pub enum NodeKind {
	Root,
	Element(Element),
	Text(String),
}

#[derive(Debug)]
pub struct Node {
	pub parent: NodeId,
	pub children: Vec<NodeId>,
	pub kind: NodeKind,
}

impl Node {
	pub fn element(&self) -> Option<&Element> {
		match &self.kind {
			NodeKind::Element(element) => Some(element),
			_ => None,
		}
	}
}

/// A parsed document plus the URL it was fetched from.
#[derive(Debug)]
pub struct Document {
	nodes: Vec<Node>,
	url: String,
	base: Option<url::Url>,
}

impl Document {
	/// Parse `html`, resolving relative links against `url`.
	///
	/// A `<base href>` in the document head wins over `url`, matching both
	/// browsers and jsoup.
	pub fn parse(html: &str, url: &str) -> Self {
		let dom = parse_document(RcDom::default(), Default::default()).one(html);
		let mut document = Self {
			nodes: Vec::with_capacity(html.len() / 32 + 8),
			url: url.to_string(),
			base: url::Url::parse(url).ok(),
		};
		document.nodes.push(Node {
			parent: NO_NODE,
			children: Vec::new(),
			kind: NodeKind::Root,
		});
		document.absorb(&dom.document, 0);
		if let Some(href) = document
			.select_first_tag("base")
			.and_then(|node| document.attr(node, "href"))
			.map(str::to_string)
		{
			if let Some(resolved) = document
				.base
				.as_ref()
				.and_then(|base| base.join(&href).ok())
				.or_else(|| url::Url::parse(&href).ok())
			{
				document.base = Some(resolved);
			}
		}
		document
	}

	/// Flatten one `RcDom` subtree, dropping comments, doctypes and
	/// processing instructions.
	fn absorb(&mut self, handle: &Handle, parent: NodeId) {
		for child in handle.children.borrow().iter() {
			match &child.data {
				NodeData::Element { name, attrs, .. } => {
					let sibling_index = self.nodes[parent as usize]
						.children
						.iter()
						.filter(|id| self.nodes[**id as usize].element().is_some())
						.count() as u32 + 1;
					let attrs: Vec<(String, String)> = attrs
						.borrow()
						.iter()
						.map(|attr| {
							(
								attr.name.local.to_ascii_lowercase().to_string(),
								attr.value.to_string(),
							)
						})
						.collect();
					let classes = attrs
						.iter()
						.find(|(name, _)| name == "class")
						.map(|(_, value)| {
							value
								.split_ascii_whitespace()
								.map(str::to_ascii_lowercase)
								.collect()
						})
						.unwrap_or_default();
					let id = self.push(
						parent,
						NodeKind::Element(Element {
							name: name.local.to_ascii_lowercase().to_string(),
							attrs,
							classes,
							sibling_index,
						}),
					);
					self.absorb(child, id);
				},
				NodeData::Text { contents } => {
					let text = contents.borrow().to_string();
					if !text.is_empty() {
						self.push(parent, NodeKind::Text(text));
					}
				},
				// `<template>` contents, comments, doctypes: no selector or
				// extraction rule in the three themes reaches them.
				_ => {
					if !Rc::ptr_eq(child, handle) {
						self.absorb(child, parent);
					}
				},
			}
		}
	}

	fn push(&mut self, parent: NodeId, kind: NodeKind) -> NodeId {
		let id = self.nodes.len() as NodeId;
		self.nodes.push(Node {
			parent,
			children: Vec::new(),
			kind,
		});
		self.nodes[parent as usize].children.push(id);
		id
	}

	pub fn root(&self) -> NodeId {
		0
	}

	/// The URL the document was fetched from (jsoup's `Document.location()`).
	pub fn url(&self) -> &str {
		&self.url
	}

	pub fn len(&self) -> usize {
		self.nodes.len()
	}

	pub fn is_empty(&self) -> bool {
		self.nodes.len() <= 1
	}

	pub fn node(&self, id: NodeId) -> &Node {
		&self.nodes[id as usize]
	}

	pub fn is_element(&self, id: NodeId) -> bool {
		self.node(id).element().is_some()
	}

	/// Every node id in document order, root excluded.
	pub fn ids(&self) -> impl Iterator<Item = NodeId> {
		1..self.nodes.len() as NodeId
	}

	pub fn parent(&self, id: NodeId) -> Option<NodeId> {
		let parent = self.node(id).parent;
		(parent != NO_NODE).then_some(parent)
	}

	pub fn element_children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
		self.node(id)
			.children
			.iter()
			.copied()
			.filter(|child| self.is_element(*child))
	}

	/// Element children count of `id`'s parent, for `:last-child`.
	pub fn parent_element_count(&self, id: NodeId) -> u32 {
		self.parent(id)
			.map(|parent| self.element_children(parent).count() as u32)
			.unwrap_or(1)
	}

	/// 1-based position of `id` among the element siblings that share its tag
	/// name, and how many of them there are — the `:*-of-type` family.
	///
	/// A node with no parent, and any non-element node, is `(1, 1)`: it is
	/// the only one of its type, which is what jsoup's `:first-of-type` and
	/// `:last-of-type` answer for a root element.
	pub fn type_position(&self, id: NodeId) -> (u32, u32) {
		let Some(name) = self.node(id).element().map(|element| &element.name) else {
			return (1, 1);
		};
		let Some(parent) = self.parent(id) else {
			return (1, 1);
		};
		let mut position = 0;
		let mut total = 0;
		for sibling in self.element_children(parent) {
			if self.node(sibling).element().map(|element| &element.name) != Some(name) {
				continue;
			}
			total += 1;
			if sibling == id {
				position = total;
			}
		}
		(position, total)
	}

	pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
		Ancestors {
			document: self,
			current: self.parent(id),
		}
	}

	/// Preceding element siblings, nearest first.
	pub fn preceding_siblings(&self, id: NodeId) -> Vec<NodeId> {
		let Some(parent) = self.parent(id) else {
			return Vec::new();
		};
		let siblings = &self.node(parent).children;
		let Some(position) = siblings.iter().position(|child| *child == id) else {
			return Vec::new();
		};
		siblings[..position]
			.iter()
			.rev()
			.copied()
			.filter(|child| self.is_element(*child))
			.collect()
	}

	/// Descendants of `id` in document order, `id` excluded.
	pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
		let mut out = Vec::new();
		let mut stack = vec![id];
		while let Some(current) = stack.pop() {
			let children = &self.node(current).children;
			for child in children.iter().rev() {
				stack.push(*child);
			}
			if current != id {
				out.push(current);
			}
		}
		out.sort_unstable();
		out
	}

	pub fn attr(&self, id: NodeId, name: &str) -> Option<&str> {
		self.node(id).element().and_then(|element| {
			element
				.attrs
				.iter()
				.find(|(key, _)| key == name)
				.map(|(_, value)| value.as_str())
		})
	}

	pub fn has_attr(&self, id: NodeId, name: &str) -> bool {
		self.attr(id, name).is_some()
	}

	/// jsoup's `abs:<name>`: the attribute resolved against the document base.
	/// Returns `None` for a missing, blank or unresolvable value.
	pub fn abs_attr(&self, id: NodeId, name: &str) -> Option<String> {
		let value = self.attr(id, name)?.trim();
		if value.is_empty() {
			return None;
		}
		self.resolve(value)
	}

	/// Resolve one URL-ish string against the document base.
	pub fn resolve(&self, value: &str) -> Option<String> {
		let value = value.trim();
		if value.is_empty() {
			return None;
		}
		match &self.base {
			Some(base) => base.join(value).ok().map(|url| url.to_string()),
			None => url::Url::parse(value).ok().map(|url| url.to_string()),
		}
	}

	/// jsoup `Element.text()`: whitespace-normalised text of the whole subtree,
	/// with block-level boundaries treated as whitespace.
	pub fn text(&self, id: NodeId) -> String {
		let mut out = String::new();
		self.write_text(id, &mut out, false);
		normalise(&out)
	}

	/// jsoup `Element.ownText()`: text children of this element only.
	pub fn own_text(&self, id: NodeId) -> String {
		let mut out = String::new();
		for child in &self.node(id).children {
			if let NodeKind::Text(text) = &self.node(*child).kind {
				out.push_str(text);
			}
		}
		normalise(&out)
	}

	/// MMRCMS' `textWithNewlines`: `<p>`/`<br>` become hard line breaks.
	pub fn text_with_newlines(&self, id: NodeId) -> String {
		let mut out = String::new();
		self.write_text(id, &mut out, true);
		out.lines()
			.map(|line| normalise(line))
			.collect::<Vec<_>>()
			.join("\n")
			.trim()
			.to_string()
	}

	/// jsoup `Element.data()`: raw text of `<script>`/`<style>` content.
	pub fn data(&self, id: NodeId) -> String {
		let mut out = String::new();
		for descendant in std::iter::once(id).chain(self.descendants(id)) {
			if let NodeKind::Text(text) = &self.node(descendant).kind {
				out.push_str(text);
			}
		}
		out
	}

	fn write_text(&self, id: NodeId, out: &mut String, hard_breaks: bool) {
		for child in &self.node(id).children {
			match &self.node(*child).kind {
				NodeKind::Text(text) => out.push_str(text),
				NodeKind::Element(element) => {
					if element.name == "script" || element.name == "style" {
						continue;
					}
					let breaks =
						hard_breaks && (element.name == "p" || element.name == "br");
					if breaks {
						out.push('\n');
					} else if is_block(&element.name) {
						out.push(' ');
					}
					self.write_text(*child, out, hard_breaks);
					if !breaks && is_block(&element.name) {
						out.push(' ');
					}
				},
				NodeKind::Root => {},
			}
		}
	}

	fn select_first_tag(&self, tag: &str) -> Option<NodeId> {
		self.ids().find(|id| {
			self.node(*id)
				.element()
				.is_some_and(|element| element.name == tag)
		})
	}
}

struct Ancestors<'a> {
	document: &'a Document,
	current: Option<NodeId>,
}

impl Iterator for Ancestors<'_> {
	type Item = NodeId;

	fn next(&mut self) -> Option<NodeId> {
		let current = self.current?;
		self.current = self.document.parent(current);
		Some(current)
	}
}

fn is_block(tag: &str) -> bool {
	matches!(
		tag,
		"address"
			| "article"
			| "aside" | "blockquote"
			| "br" | "dd"
			| "div" | "dl"
			| "dt" | "fieldset"
			| "figcaption"
			| "figure"
			| "footer"
			| "form" | "h1"
			| "h2" | "h3"
			| "h4" | "h5"
			| "h6" | "header"
			| "hr" | "li"
			| "main" | "nav"
			| "ol" | "p"
			| "pre" | "section"
			| "table" | "td"
			| "th" | "tr"
			| "ul"
	)
}

/// Collapse whitespace runs to one space and trim, as jsoup's `text()` does.
pub fn normalise(value: &str) -> String {
	let mut out = String::with_capacity(value.len());
	let mut pending_space = false;
	for character in value.chars() {
		if character.is_whitespace() {
			pending_space = !out.is_empty();
			continue;
		}
		if pending_space {
			out.push(' ');
			pending_space = false;
		}
		out.push(character);
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	const PAGE: &str = r#"
		<html><head><title>t</title></head><body>
			<div class="Wrap Outer" id="main">
				<p>Hello <b>world</b></p>
				<p>second</p>
				<a href="/manga/foo/" title="Foo">Foo</a>
				<img data-src="/img/a.jpg" src="placeholder.gif">
				<script>var chapter_id = 42;</script>
			</div>
		</body></html>
	"#;

	#[test]
	fn parsing_lowercases_names_and_indexes_siblings() {
		let document = Document::parse(PAGE, "https://site.test/read/x/");
		let div = document
			.ids()
			.find(|id| document.attr(*id, "id") == Some("main"))
			.unwrap();
		let element = document.node(div).element().unwrap();
		assert_eq!(element.name, "div");
		assert_eq!(element.classes, vec!["wrap", "outer"]);
		let paragraphs: Vec<_> = document
			.element_children(div)
			.filter(|id| document.node(*id).element().unwrap().name == "p")
			.collect();
		assert_eq!(paragraphs.len(), 2);
		assert_eq!(
			document
				.node(paragraphs[0])
				.element()
				.unwrap()
				.sibling_index,
			1
		);
		assert_eq!(
			document
				.node(paragraphs[1])
				.element()
				.unwrap()
				.sibling_index,
			2
		);
	}

	#[test]
	fn text_accessors_follow_jsoup_semantics() {
		let document = Document::parse(PAGE, "https://site.test/read/x/");
		let div = document
			.ids()
			.find(|id| document.attr(*id, "id") == Some("main"))
			.unwrap();
		assert_eq!(document.text(div), "Hello world second Foo");
		assert_eq!(document.own_text(div), "");
		let first = document.element_children(div).next().unwrap();
		assert_eq!(document.text(first), "Hello world");
		assert_eq!(document.own_text(first), "Hello");
		let script = document
			.ids()
			.find(|id| {
				document
					.node(*id)
					.element()
					.is_some_and(|e| e.name == "script")
			})
			.unwrap();
		assert_eq!(document.data(script), "var chapter_id = 42;");
	}

	#[test]
	fn abs_attr_resolves_against_the_document_url() {
		let document = Document::parse(PAGE, "https://site.test/read/x/");
		let anchor = document
			.ids()
			.find(|id| document.node(*id).element().is_some_and(|e| e.name == "a"))
			.unwrap();
		assert_eq!(
			document.abs_attr(anchor, "href").as_deref(),
			Some("https://site.test/manga/foo/")
		);
		let image = document
			.ids()
			.find(|id| {
				document
					.node(*id)
					.element()
					.is_some_and(|e| e.name == "img")
			})
			.unwrap();
		assert_eq!(
			document.abs_attr(image, "src").as_deref(),
			Some("https://site.test/read/x/placeholder.gif")
		);
		assert_eq!(document.abs_attr(image, "srcset"), None);
	}

	#[test]
	fn base_href_overrides_the_fetch_url() {
		let document = Document::parse(
			r#"<html><head><base href="https://cdn.test/app/"></head><body><a href="x.html">a</a></body></html>"#,
			"https://site.test/page",
		);
		let anchor = document
			.ids()
			.find(|id| document.node(*id).element().is_some_and(|e| e.name == "a"))
			.unwrap();
		assert_eq!(
			document.abs_attr(anchor, "href").as_deref(),
			Some("https://cdn.test/app/x.html")
		);
		// location() stays the fetch URL, as in jsoup.
		assert_eq!(document.url(), "https://site.test/page");
	}

	#[test]
	fn text_with_newlines_breaks_on_paragraphs_and_br() {
		let document = Document::parse(
			"<div><h5>Summary</h5><p>one</p><p>two<br>three</p></div>",
			"https://site.test/",
		);
		let div = document
			.ids()
			.find(|id| {
				document
					.node(*id)
					.element()
					.is_some_and(|e| e.name == "div")
			})
			.unwrap();
		assert_eq!(document.text_with_newlines(div), "Summary\none\ntwo\nthree");
	}
}
