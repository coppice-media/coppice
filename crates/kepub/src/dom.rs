use std::{borrow::Cow, collections::HashSet, fmt::Write as _};

use html5ever::{
	tendril::StrTendril,
	tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink},
};
use markup5ever::{namespace_url, ns, Attribute, ExpandedName, LocalName, QualName};

/// A stable index into an [`Arena`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct NodeId(u32);

/// The payload stored by one arena node.
#[derive(Debug)]
pub(crate) enum NodeKind {
	Document,
	Doctype {
		name: StrTendril,
		public_id: StrTendril,
		system_id: StrTendril,
	},
	Text(StrTendril),
	Comment(StrTendril),
	ProcessingInstruction {
		target: StrTendril,
		data: StrTendril,
	},
	Element {
		name: QualName,
		attrs: Vec<Attribute>,
		template_contents: Option<NodeId>,
		mathml_annotation_xml_integration_point: bool,
	},
}

/// An arena-backed HTML tree.
///
/// Node indices never change after allocation.  Parent and sibling links make
/// the parser's insertion operations and the content transforms independent of
/// reference-counted tree nodes.
pub(crate) struct Arena {
	pub(crate) nodes: Vec<Node>,
	document: NodeId,
	errors: Vec<Cow<'static, str>>,
	quirks_mode: QuirksMode,
}

#[derive(Debug)]
pub(crate) struct Node {
	pub(crate) parent: Option<NodeId>,
	pub(crate) first_child: Option<NodeId>,
	pub(crate) last_child: Option<NodeId>,
	pub(crate) prev: Option<NodeId>,
	pub(crate) next: Option<NodeId>,
	pub(crate) data: NodeKind,
}

impl Default for Arena {
	fn default() -> Self {
		Self::new()
	}
}

impl Arena {
	pub(crate) fn new() -> Self {
		let mut arena = Self {
			nodes: Vec::with_capacity(1024),
			document: NodeId(0),
			errors: Vec::new(),
			quirks_mode: html5ever::tree_builder::NoQuirks,
		};
		arena.document = arena.alloc(NodeKind::Document);
		arena
	}

	pub(crate) fn document(&self) -> NodeId {
		self.document
	}

	fn alloc(&mut self, data: NodeKind) -> NodeId {
		let index =
			u32::try_from(self.nodes.len()).expect("DOM arena exceeded u32::MAX nodes");
		let id = NodeId(index);
		self.nodes.push(Node {
			parent: None,
			first_child: None,
			last_child: None,
			prev: None,
			next: None,
			data,
		});
		id
	}

	fn node(&self, id: NodeId) -> &Node {
		&self.nodes[id.0 as usize]
	}

	fn node_mut(&mut self, id: NodeId) -> &mut Node {
		&mut self.nodes[id.0 as usize]
	}

	fn child_ids(&self, parent: NodeId) -> Vec<NodeId> {
		let mut children = Vec::new();
		let mut child = self.node(parent).first_child;
		while let Some(id) = child {
			children.push(id);
			child = self.node(id).next;
		}
		children
	}

	fn append_node_id(&mut self, parent: NodeId, child: NodeId) {
		assert!(
			self.node(child).parent.is_none(),
			"appended node already has a parent"
		);
		let previous = self.node(parent).last_child;
		self.node_mut(child).parent = Some(parent);
		self.node_mut(child).prev = previous;
		self.node_mut(child).next = None;
		if let Some(previous) = previous {
			self.node_mut(previous).next = Some(child);
		} else {
			self.node_mut(parent).first_child = Some(child);
		}
		self.node_mut(parent).last_child = Some(child);
	}

	fn append_text(&mut self, parent: NodeId, text: StrTendril) {
		if let Some(last) = self.node(parent).last_child {
			if let NodeKind::Text(contents) = &mut self.node_mut(last).data {
				contents.push_tendril(&text);
				return;
			}
		}
		let child = self.alloc(NodeKind::Text(text));
		self.append_node_id(parent, child);
	}

	fn detach(&mut self, target: NodeId) -> Option<NodeId> {
		let parent = self.node(target).parent?;
		let previous = self.node(target).prev;
		let next = self.node(target).next;
		if let Some(previous) = previous {
			self.node_mut(previous).next = next;
		} else {
			self.node_mut(parent).first_child = next;
		}
		if let Some(next) = next {
			self.node_mut(next).prev = previous;
		} else {
			self.node_mut(parent).last_child = previous;
		}
		let node = self.node_mut(target);
		node.parent = None;
		node.prev = None;
		node.next = None;
		Some(parent)
	}

	fn insert_before_detached(&mut self, sibling: NodeId, child: NodeId) {
		assert!(
			self.node(child).parent.is_none(),
			"inserted node already has a parent"
		);
		let parent = self
			.node(sibling)
			.parent
			.expect("append_before_sibling called on node without parent");
		let previous = self.node(sibling).prev;
		self.node_mut(child).parent = Some(parent);
		self.node_mut(child).prev = previous;
		self.node_mut(child).next = Some(sibling);
		self.node_mut(sibling).prev = Some(child);
		if let Some(previous) = previous {
			self.node_mut(previous).next = Some(child);
		} else {
			self.node_mut(parent).first_child = Some(child);
		}
	}

	fn append_before_text(&mut self, sibling: NodeId, text: StrTendril) {
		let previous = self.node(sibling).prev;
		if let Some(previous) = previous {
			if let NodeKind::Text(contents) = &mut self.node_mut(previous).data {
				contents.push_tendril(&text);
				return;
			}
		}
		let child = self.alloc(NodeKind::Text(text));
		self.insert_before_detached(sibling, child);
	}

	fn append_before_node(&mut self, sibling: NodeId, child: NodeId) {
		self.detach(child);
		self.insert_before_detached(sibling, child);
	}

	pub(crate) fn append_child(&mut self, parent: NodeId, child: NodeId) {
		self.append_node_id(parent, child);
	}

	pub(crate) fn remove_from_parent(&mut self, target: NodeId) {
		self.detach(target);
	}

	pub(crate) fn move_children(&mut self, from: NodeId, to: NodeId) {
		if from == to {
			return;
		}
		let first = self.node(from).first_child;
		let last = self.node(from).last_child;
		let (Some(first), Some(last)) = (first, last) else {
			return;
		};
		self.node_mut(from).first_child = None;
		self.node_mut(from).last_child = None;

		let mut child = Some(first);
		while let Some(id) = child {
			self.node_mut(id).parent = Some(to);
			if id == last {
				break;
			}
			child = self.node(id).next;
		}

		let previous = self.node(to).last_child;
		if let Some(previous) = previous {
			self.node_mut(previous).next = Some(first);
			self.node_mut(first).prev = Some(previous);
		} else {
			self.node_mut(to).first_child = Some(first);
			self.node_mut(first).prev = None;
		}
		self.node_mut(last).next = None;
		self.node_mut(to).last_child = Some(last);
	}
}

impl TreeSink for Arena {
	type Handle = NodeId;
	type Output = Self;

	fn finish(self) -> Self::Output {
		self
	}

	fn parse_error(&mut self, msg: Cow<'static, str>) {
		self.errors.push(msg);
	}

	fn get_document(&mut self) -> Self::Handle {
		self.document
	}

	fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> ExpandedName<'a> {
		match &self.node(*target).data {
			NodeKind::Element { name, .. } => name.expanded(),
			_ => panic!("not an element"),
		}
	}

	fn create_element(
		&mut self,
		name: QualName,
		attrs: Vec<Attribute>,
		flags: ElementFlags,
	) -> Self::Handle {
		let template_contents = flags.template.then(|| self.alloc(NodeKind::Document));
		self.alloc(NodeKind::Element {
			name,
			attrs,
			template_contents,
			mathml_annotation_xml_integration_point: flags
				.mathml_annotation_xml_integration_point,
		})
	}

	fn create_comment(&mut self, text: StrTendril) -> Self::Handle {
		self.alloc(NodeKind::Comment(text))
	}

	fn create_pi(&mut self, target: StrTendril, data: StrTendril) -> Self::Handle {
		self.alloc(NodeKind::ProcessingInstruction { target, data })
	}

	fn append(&mut self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
		match child {
			NodeOrText::AppendText(text) => self.append_text(*parent, text),
			NodeOrText::AppendNode(child) => self.append_node_id(*parent, child),
		}
	}

	fn append_based_on_parent_node(
		&mut self,
		element: &Self::Handle,
		prev_element: &Self::Handle,
		child: NodeOrText<Self::Handle>,
	) {
		if self.node(*element).parent.is_some() {
			self.append_before_sibling(element, child);
		} else {
			self.append(prev_element, child);
		}
	}

	fn append_doctype_to_document(
		&mut self,
		name: StrTendril,
		public_id: StrTendril,
		system_id: StrTendril,
	) {
		let doctype = self.alloc(NodeKind::Doctype {
			name,
			public_id,
			system_id,
		});
		self.append_node_id(self.document, doctype);
	}

	fn mark_script_already_started(&mut self, _node: &Self::Handle) {}

	fn pop(&mut self, _node: &Self::Handle) {}

	fn get_template_contents(&mut self, target: &Self::Handle) -> Self::Handle {
		match &self.node(*target).data {
			NodeKind::Element {
				template_contents: Some(contents),
				..
			} => *contents,
			_ => panic!("not a template element"),
		}
	}

	fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
		x == y
	}

	fn set_quirks_mode(&mut self, mode: QuirksMode) {
		self.quirks_mode = mode;
	}

	fn append_before_sibling(
		&mut self,
		sibling: &Self::Handle,
		child: NodeOrText<Self::Handle>,
	) {
		match child {
			NodeOrText::AppendText(text) => self.append_before_text(*sibling, text),
			NodeOrText::AppendNode(child) => self.append_before_node(*sibling, child),
		}
	}

	fn add_attrs_if_missing(&mut self, target: &Self::Handle, attrs: Vec<Attribute>) {
		let existing_names: HashSet<QualName> = match &self.node(*target).data {
			NodeKind::Element { attrs, .. } => {
				attrs.iter().map(|attr| attr.name.clone()).collect()
			},
			_ => panic!("not an element"),
		};
		let additions: Vec<_> = attrs
			.into_iter()
			.filter(|attr| !existing_names.contains(&attr.name))
			.collect();
		match &mut self.node_mut(*target).data {
			NodeKind::Element { attrs, .. } => attrs.extend(additions),
			_ => unreachable!("element checked above"),
		}
	}

	fn remove_from_parent(&mut self, target: &Self::Handle) {
		self.remove_from_parent(*target);
	}

	fn reparent_children(&mut self, node: &Self::Handle, new_parent: &Self::Handle) {
		self.move_children(*node, *new_parent);
	}

	fn is_mathml_annotation_xml_integration_point(&self, target: &Self::Handle) -> bool {
		match &self.node(*target).data {
			NodeKind::Element {
				mathml_annotation_xml_integration_point,
				..
			} => *mathml_annotation_xml_integration_point,
			_ => panic!("not an element"),
		}
	}
}

pub(crate) fn normalize_document_whitespace(arena: &mut Arena, document: NodeId) {
	while let Some(first) = arena.node(document).first_child {
		if !is_whitespace_node(arena, first) {
			break;
		}
		arena.remove_from_parent(first);
	}
	let Some(html) = find_element(arena, document, "html") else {
		return;
	};
	let children = arena.child_ids(html);
	let mut saw_head = false;
	for child in children {
		if is_element_named(arena, child, "head") {
			saw_head = true;
		} else if is_element_named(arena, child, "body") {
			continue;
		} else if !saw_head && is_whitespace_node(arena, child) {
			arena.remove_from_parent(child);
		}
	}
}

pub(crate) fn is_whitespace_node(arena: &Arena, node: NodeId) -> bool {
	match &arena.node(node).data {
		NodeKind::Text(contents) => contents.chars().all(char::is_whitespace),
		_ => false,
	}
}

pub(crate) fn children(arena: &Arena, parent: NodeId) -> Vec<NodeId> {
	arena.child_ids(parent)
}

pub(crate) fn text_contents(arena: &Arena, node: NodeId) -> Option<&str> {
	match &arena.node(node).data {
		NodeKind::Text(contents) => Some(contents),
		_ => None,
	}
}

pub(crate) fn find_element(arena: &Arena, root: NodeId, wanted: &str) -> Option<NodeId> {
	let mut stack = vec![root];
	while let Some(node) = stack.pop() {
		if is_element_named(arena, node, wanted) {
			return Some(node);
		}
		let children = arena.child_ids(node);
		stack.extend(children.into_iter().rev());
	}
	None
}

pub(crate) fn is_element_named(arena: &Arena, node: NodeId, wanted: &str) -> bool {
	match &arena.node(node).data {
		NodeKind::Element { name, .. } => {
			name.local.as_ref().eq_ignore_ascii_case(wanted)
		},
		_ => false,
	}
}

pub(crate) fn element_name(arena: &Arena, node: NodeId) -> Option<String> {
	match &arena.node(node).data {
		NodeKind::Element { name, .. } => Some(name.local.as_ref().to_ascii_lowercase()),
		_ => None,
	}
}

pub(crate) fn new_text(arena: &mut Arena, value: &str) -> NodeId {
	arena.alloc(NodeKind::Text(StrTendril::from_slice(value)))
}

pub(crate) fn new_element(
	arena: &mut Arena,
	name: &str,
	attrs: Vec<Attribute>,
) -> NodeId {
	arena.alloc(NodeKind::Element {
		name: QualName::new(None, ns!(html), LocalName::from(name)),
		attrs,
		template_contents: None,
		mathml_annotation_xml_integration_point: false,
	})
}

pub(crate) fn html_attribute(name: &str, value: &str) -> Attribute {
	Attribute {
		name: QualName::new(None, ns!(), LocalName::from(name)),
		value: StrTendril::from_slice(value),
	}
}

pub(crate) fn transform_divs(arena: &mut Arena, body: NodeId) {
	let direct = arena.child_ids(body);
	if let Some(columns) = direct.into_iter().find(|node| {
		is_element_named(arena, *node, "div")
			&& has_attr(arena, *node, "id", "book-columns")
	}) {
		if arena.child_ids(columns).into_iter().any(|node| {
			is_element_named(arena, node, "div")
				&& has_attr(arena, node, "id", "book-inner")
		}) {
			return;
		}
		let inner = new_element(arena, "div", vec![html_attribute("id", "book-inner")]);
		arena.move_children(columns, inner);
		arena.append_child(columns, inner);
		return;
	}
	let inner = new_element(arena, "div", vec![html_attribute("id", "book-inner")]);
	arena.move_children(body, inner);
	let columns = new_element(arena, "div", vec![html_attribute("id", "book-columns")]);
	arena.append_child(columns, inner);
	arena.append_child(body, columns);
}

pub(crate) fn transform_content_charset(arena: &mut Arena, head: NodeId) {
	for node in descendants(arena, head) {
		if !is_element_named(arena, node, "meta") {
			continue;
		}
		let NodeKind::Element { attrs, .. } = &mut arena.node_mut(node).data else {
			continue;
		};
		for attr in attrs.iter_mut() {
			if attr.name.local.as_ref().eq_ignore_ascii_case("charset")
				&& !attr.value.eq_ignore_ascii_case("utf-8")
			{
				attr.value = StrTendril::from_slice("UTF-8");
			}
		}
		let http_equiv = attrs.iter().find_map(|attr| {
			attr.name
				.local
				.as_ref()
				.eq_ignore_ascii_case("http-equiv")
				.then(|| attr.value.to_string())
		});
		if http_equiv
			.as_deref()
			.is_some_and(|value| value.eq_ignore_ascii_case("content-type"))
		{
			if let Some(content) = attrs
				.iter_mut()
				.find(|attr| attr.name.local.as_ref().eq_ignore_ascii_case("content"))
			{
				let lower = content.value.to_ascii_lowercase();
				if let Some(charset_start) = lower.find("charset=") {
					let value_start = charset_start + "charset=".len();
					let value_end = lower[value_start..]
						.find(';')
						.map(|offset| value_start + offset)
						.unwrap_or(lower.len());
					let current = content.value[value_start..value_end]
						.trim_matches([' ', '\t', '"']);
					if !current.eq_ignore_ascii_case("utf-8") {
						let mut value = content.value.to_string();
						value.replace_range(value_start..value_end, "utf-8");
						content.value = StrTendril::from_slice(&value);
					}
				}
			}
		}
	}
}

pub(crate) fn add_style(arena: &mut Arena, head: NodeId, class: &str, css: &str) {
	let style = new_element(
		arena,
		"style",
		vec![
			html_attribute("type", "text/css"),
			html_attribute("class", class),
		],
	);
	let text = new_text(arena, css);
	arena.append_child(style, text);
	arena.append_child(head, style);
}

pub(crate) fn has_attr(arena: &Arena, node: NodeId, wanted: &str, value: &str) -> bool {
	let NodeKind::Element { attrs, .. } = &arena.node(node).data else {
		return false;
	};
	attrs.iter().any(|attr| {
		attr.name.local.as_ref().eq_ignore_ascii_case(wanted)
			&& attr.value.as_ref() == value
	})
}

pub(crate) fn has_kobo_span(arena: &Arena, body: NodeId) -> bool {
	descendants(arena, body).into_iter().any(|node| {
		let NodeKind::Element { attrs, .. } = &arena.node(node).data else {
			return false;
		};
		attrs.iter().any(|attr| {
			attr.name.local.as_ref().eq_ignore_ascii_case("class")
				&& attr
					.value
					.split_ascii_whitespace()
					.any(|token| token == "koboSpan")
		})
	})
}

pub(crate) fn descendants(arena: &Arena, root: NodeId) -> Vec<NodeId> {
	let mut output = Vec::new();
	let mut stack = vec![root];
	while let Some(node) = stack.pop() {
		output.push(node);
		let children = arena.child_ids(node);
		stack.extend(children.into_iter().rev());
	}
	output
}

fn is_paragraph_element(name: &str) -> bool {
	matches!(
		name,
		"p" | "ol" | "ul" | "table" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
	)
}

pub(crate) fn smarten_body(arena: &mut Arena, body: NodeId) {
	smarten_node(arena, body);
}

fn smarten_node(arena: &mut Arena, node: NodeId) {
	if let NodeKind::Text(contents) = &arena.node(node).data {
		let transformed = smartypants(contents);
		if let NodeKind::Text(contents) = &mut arena.node_mut(node).data {
			*contents = StrTendril::from_slice(&transformed);
		}
		return;
	}
	if let Some(name) = element_name(arena, node) {
		if matches!(name.as_str(), "pre" | "code" | "style" | "script") {
			return;
		}
	}
	for child in arena.child_ids(node) {
		smarten_node(arena, child);
	}
}

fn smartypants(text: &str) -> String {
	let mut output = String::with_capacity(text.len());
	let chars: Vec<char> = text.chars().collect();
	let mut index = 0;
	while index < chars.len() {
		if index + 3 <= chars.len() {
			let token: String = chars[index..index + 3].iter().collect();
			match token.to_ascii_lowercase().as_str() {
				"(c)" => {
					output.push('©');
					index += 3;
					continue;
				},
				"(r)" => {
					output.push('®');
					index += 3;
					continue;
				},
				"(tm" if index + 3 < chars.len() && chars[index + 3] == ')' => {
					output.push('™');
					index += 4;
					continue;
				},
				_ => {},
			}
		}
		if index + 3 <= chars.len()
			&& chars[index] == '.'
			&& chars[index + 1] == '.'
			&& chars[index + 2] == '.'
		{
			output.push('…');
			index += 3;
			continue;
		}
		if index + 3 <= chars.len()
			&& chars[index] == '-'
			&& chars[index + 1] == '-'
			&& chars[index + 2] == '-'
		{
			output.push('—');
			index += 3;
			continue;
		}
		if index + 2 <= chars.len() && chars[index] == '-' && chars[index + 1] == '-' {
			output.push('–');
			index += 2;
			continue;
		}
		if index + 3 <= chars.len() {
			let fraction = match chars[index..index + 3] {
				['1', '/', '2'] => Some('½'),
				['1', '/', '4'] => Some('¼'),
				['3', '/', '4'] => Some('¾'),
				_ => None,
			};
			if let Some(fraction) = fraction {
				output.push(fraction);
				index += 3;
				continue;
			}
		}
		let ch = chars[index];
		if ch == '\'' || ch == '"' {
			let previous = output.chars().next_back();
			let opening = previous
				.is_none_or(|value| value.is_whitespace() || "([{<&".contains(value));
			output.push(match (ch, opening) {
				('\'', true) => '‘',
				('\'', false) => '’',
				(_, true) => '“',
				(_, false) => '”',
			});
		} else {
			output.push(ch);
		}
		index += 1;
	}
	output
}

pub(crate) fn clean_node(arena: &mut Arena, node: NodeId) {
	if let NodeKind::Text(contents) = &arena.node(node).data {
		if contents.contains('\u{fffd}') {
			let cleaned: String =
				contents.chars().filter(|ch| *ch != '\u{fffd}').collect();
			if let NodeKind::Text(contents) = &mut arena.node_mut(node).data {
				*contents = StrTendril::from_slice(&cleaned);
			}
		}
		return;
	}
	for child in arena.child_ids(node) {
		if should_clean_away(arena, child) {
			arena.remove_from_parent(child);
		} else {
			clean_node(arena, child);
		}
	}
}

fn should_clean_away(arena: &Arena, node: NodeId) -> bool {
	let NodeKind::Element { name, attrs, .. } = &arena.node(node).data else {
		return false;
	};
	let local = name.local.as_ref();
	if local == "meta" {
		return attrs.iter().any(|attr| {
			attr.name.local.as_ref() == "name"
				&& (attr.value.as_ref() == "Adept.expected.resource"
					|| attr.value.as_ref() == "Adept.resource")
		});
	}
	(local == "o:p" || local.starts_with("st1:"))
		&& arena.child_ids(node).into_iter().all(|child| {
			is_whitespace_node(arena, child)
				|| matches!(arena.node(child).data, NodeKind::Comment(_))
		})
}

const VOID_ELEMENTS: &[&str] = &[
	"area", "base", "br", "col", "embed", "hr", "img", "input", "keygen", "link", "meta",
	"param", "source", "track", "wbr",
];

const SPAN_SKIP_ELEMENTS: &[&str] =
	&["script", "style", "pre", "audio", "video", "svg", "math"];

const RAW_TEXT_ELEMENTS: &[&str] = &[
	"iframe",
	"noembed",
	"noframes",
	"noscript",
	"plaintext",
	"script",
	"style",
	"xmp",
];

#[allow(clippy::too_many_arguments)]
pub(crate) fn serialize_node(
	arena: &Arena,
	node: NodeId,
	output: &mut String,
	raw_text: bool,
	polyglot: bool,
	span_body: Option<NodeId>,
	emit_spans: bool,
	smarten: bool,
) {
	let mut state = SerializeState {
		para: 0,
		segment: 0,
		increment_para: false,
	};
	serialize_node_inner(
		arena, node, output, raw_text, polyglot, span_body, emit_spans, false, false,
		smarten, &mut state,
	);
}

#[derive(Debug, Default)]
struct SerializeState {
	para: usize,
	segment: usize,
	increment_para: bool,
}

#[allow(clippy::too_many_arguments)]
fn serialize_node_inner(
	arena: &Arena,
	node: NodeId,
	output: &mut String,
	raw_text: bool,
	polyglot: bool,
	span_body: Option<NodeId>,
	emit_spans: bool,
	in_body: bool,
	span_active: bool,
	smarten_allowed: bool,
	state: &mut SerializeState,
) {
	match &arena.node(node).data {
		NodeKind::Document => {
			let mut child = arena.node(node).first_child;
			while let Some(id) = child {
				let next = arena.node(id).next;
				serialize_node_inner(
					arena,
					id,
					output,
					false,
					polyglot,
					span_body,
					emit_spans,
					false,
					false,
					smarten_allowed,
					state,
				);
				child = next;
			}
		},
		NodeKind::Doctype {
			name,
			public_id,
			system_id,
		} => {
			output.push_str("<!DOCTYPE ");
			output.push_str(name);
			if !public_id.is_empty() {
				output.push_str(" PUBLIC ");
				write_quoted(output, public_id);
				if !system_id.is_empty() {
					output.push(' ');
					write_quoted(output, system_id);
				}
			} else if !system_id.is_empty() {
				output.push_str(" SYSTEM ");
				write_quoted(output, system_id);
			}
			output.push('>');
		},
		NodeKind::Text(contents) => {
			if span_active && in_body {
				emit_text_spans(
					contents,
					output,
					polyglot,
					raw_text,
					smarten_allowed,
					false,
					state,
				);
			} else if smarten_allowed && in_body {
				let transformed = smarten_text(contents);
				if raw_text {
					output.push_str(&transformed);
				} else {
					escape_text(output, &transformed, polyglot);
				}
			} else if raw_text {
				output.push_str(contents);
			} else {
				escape_text(output, contents, polyglot);
			}
		},
		NodeKind::Comment(contents) => {
			output.push_str("<!--");
			output.push_str(contents);
			output.push_str("-->");
		},
		NodeKind::ProcessingInstruction { target, data } => {
			output.push_str("<?");
			output.push_str(target);
			output.push_str(data);
			output.push_str("?>");
		},
		NodeKind::Element { name, attrs, .. } => {
			let tag = name.local.as_ref();
			let this_in_body = in_body || span_body == Some(node);
			let this_span_active = span_active || (emit_spans && span_body == Some(node));
			output.push('<');
			output.push_str(tag);
			for attr in attrs {
				write_attribute(output, attr);
			}
			if polyglot {
				polyglot_attributes(output, tag, attrs);
			}
			if VOID_ELEMENTS.contains(&tag) {
				output.push_str("/>");
				return;
			}
			output.push('>');
			let child_raw = RAW_TEXT_ELEMENTS.contains(&tag);
			let child_span_active =
				this_span_active && !SPAN_SKIP_ELEMENTS.contains(&tag);
			let child_smarten_allowed =
				smarten_allowed && !matches!(tag, "pre" | "code" | "style" | "script");
			let parent_is_p = tag == "p";
			if child_span_active && is_paragraph_element(tag) {
				state.increment_para = true;
			}
			let mut child = arena.node(node).first_child;
			while let Some(id) = child {
				let next = arena.node(id).next;
				if child_span_active {
					match &arena.node(id).data {
						NodeKind::Text(contents) => emit_text_spans(
							contents,
							output,
							polyglot,
							child_raw,
							child_smarten_allowed,
							parent_is_p,
							state,
						),
						NodeKind::Element { name, .. }
							if name.local.as_ref() == "img" =>
						{
							emit_image_span(
								arena,
								id,
								output,
								polyglot,
								this_in_body,
								state,
							);
						},
						_ => serialize_node_inner(
							arena,
							id,
							output,
							child_raw,
							polyglot,
							span_body,
							emit_spans,
							this_in_body,
							child_span_active,
							child_smarten_allowed,
							state,
						),
					}
				} else {
					serialize_node_inner(
						arena,
						id,
						output,
						child_raw,
						polyglot,
						span_body,
						emit_spans,
						this_in_body,
						false,
						child_smarten_allowed,
						state,
					);
				}
				child = next;
			}
			output.push_str("</");
			output.push_str(tag);
			output.push('>');
		},
	}
}

fn write_quoted(output: &mut String, value: &str) {
	let quote = if value.contains('"') { '\'' } else { '"' };
	output.push(quote);
	output.push_str(value);
	output.push(quote);
}

fn emit_text_spans(
	text: &str,
	output: &mut String,
	polyglot: bool,
	raw_text: bool,
	smarten: bool,
	parent_is_p: bool,
	state: &mut SerializeState,
) {
	for sentence in crate::split_sentences(text) {
		if !parent_is_p && sentence.chars().all(char::is_whitespace) {
			if raw_text {
				output.push_str(sentence);
			} else {
				escape_text(output, sentence, polyglot);
			}
			continue;
		}
		if state.increment_para {
			state.para += 1;
			state.segment = 0;
			state.increment_para = false;
		}
		state.segment += 1;
		let _ = write!(
			output,
			"<span class=\"koboSpan\" id=\"kobo.{}.{}\">",
			state.para, state.segment
		);
		if smarten {
			let transformed = smarten_text(sentence);
			escape_text(output, &transformed, polyglot);
		} else {
			escape_text(output, sentence, polyglot);
		}
		output.push_str("</span>");
	}
}

fn emit_image_span(
	arena: &Arena,
	image: NodeId,
	output: &mut String,
	polyglot: bool,
	in_body: bool,
	state: &mut SerializeState,
) {
	state.para += 1;
	state.segment = 1;
	state.increment_para = false;
	let _ = write!(
		output,
		"<span class=\"koboSpan\" id=\"kobo.{}.{}\">",
		state.para, state.segment
	);
	serialize_node_inner(
		arena, image, output, false, polyglot, None, false, in_body, false, false, state,
	);
	output.push_str("</span>");
}

fn smarten_text(text: &str) -> String {
	smartypants(text)
}

fn write_attribute(output: &mut String, attr: &Attribute) {
	output.push(' ');
	if let Some(prefix) = &attr.name.prefix {
		if !prefix.as_ref().is_empty() {
			output.push_str(prefix.as_ref());
			output.push(':');
		}
	}
	output.push_str(attr.name.local.as_ref());
	output.push_str("=\"");
	escape_attribute(output, &attr.value);
	output.push('"');
}

fn write_raw_attribute(output: &mut String, name: &str, value: &str) {
	output.push(' ');
	output.push_str(name);
	output.push_str("=\"");
	escape_attribute(output, value);
	output.push('"');
}

fn polyglot_attributes(output: &mut String, tag: &str, attrs: &[Attribute]) {
	match tag {
		"html" if !has_rendered_attr(attrs, "xmlns", None) => {
			write_raw_attribute(output, "xmlns", "http://www.w3.org/1999/xhtml");
		},
		"svg" => {
			if !has_rendered_attr(attrs, "xmlns", None) {
				write_raw_attribute(output, "xmlns", "http://www.w3.org/2000/svg");
			}
			if !has_rendered_attr(attrs, "xlink", Some("xmlns")) {
				write_raw_attribute(
					output,
					"xmlns:xlink",
					"http://www.w3.org/1999/xlink",
				);
			}
		},
		"math" if !has_rendered_attr(attrs, "xmlns", None) => {
			write_raw_attribute(output, "xmlns", "http://www.w3.org/1998/Math/MathML");
		},
		"script" if !has_rendered_attr(attrs, "type", None) => {
			write_raw_attribute(output, "type", "text/javascript");
		},
		"style" if !has_rendered_attr(attrs, "type", None) => {
			write_raw_attribute(output, "type", "text/css");
		},
		_ => {},
	}
}

fn has_rendered_attr(attrs: &[Attribute], local: &str, prefix: Option<&str>) -> bool {
	attrs.iter().any(|attr| {
		attr.name.local.as_ref().eq_ignore_ascii_case(local)
			&& attr
				.name
				.prefix
				.as_ref()
				.map(|value| value.as_ref())
				.filter(|value| !value.is_empty())
				== prefix
	})
}

fn escape_text(output: &mut String, value: &str, polyglot: bool) {
	let bytes = value.as_bytes();
	let mut last = 0;
	for (index, byte) in bytes.iter().enumerate() {
		let escaped = match byte {
			b'&' => "&amp;",
			b'\'' => "&#39;",
			b'<' => "&lt;",
			b'>' => "&gt;",
			b'"' => "&#34;",
			b'\r' => "&#13;",
			0xC2 if polyglot && bytes.get(index + 1) == Some(&0xA0) => {
				output.push_str(&value[last..index]);
				output.push_str("&#160;");
				last = index + 2;
				continue;
			},
			_ => continue,
		};
		output.push_str(&value[last..index]);
		output.push_str(escaped);
		last = index + 1;
	}
	output.push_str(&value[last..]);
}

fn escape_attribute(output: &mut String, value: &str) {
	escape_text(output, value, false);
}
