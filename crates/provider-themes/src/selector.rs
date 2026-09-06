//! A jsoup-compatible CSS selector subset.
//!
//! The selector strings in the definitions were written for jsoup, so plain CSS
//! is not enough: jsoup adds `:contains(text)`, `:containsOwn(text)` and
//! `:containsData(text)`, matches attribute values and class names
//! case-insensitively, and allows `:has(> a)`. Everything the three themes and
//! their ~470 subclasses actually use at `keiyoushi/extensions-source`
//! `064c1a0e` is supported:
//!
//! * groups (`a, b`), descendant, `>`, `+`, `~`
//! * `*`, tag, `#id`, `.class`
//! * `[attr]`, `[attr=v]`, `[attr^=v]`, `[attr$=v]`, `[attr*=v]`, `[attr~=v]`,
//!   `[attr!=v]`, with `\` escapes inside values
//! * `:not(...)`, `:has(...)`, `:contains(...)`, `:containsOwn(...)`,
//!   `:containsData(...)`, `:matches(...)`, `:first-child`, `:last-child`,
//!   `:only-child`, `:nth-child(n|odd|even)`, `:first-of-type`,
//!   `:last-of-type`, `:nth-of-type(n|odd|even)`, `:empty`, `:root`, `:eq(n)`,
//!   `:gt(n)`, `:lt(n)`
//!
//! Anything else is a parse error rather than a silent mismatch, so a
//! definition carrying an unsupported selector fails loudly at engine
//! construction instead of returning an empty series list at runtime.

use std::fmt;

use regex::Regex;

use crate::dom::{normalise, Document, NodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Combinator {
	Descendant,
	Child,
	Adjacent,
	Sibling,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AttrOp {
	Exists,
	Equals(String),
	NotEquals(String),
	StartsWith(String),
	EndsWith(String),
	Contains(String),
	Word(String),
}

#[derive(Debug)]
enum Simple {
	Any,
	Tag(String),
	Id(String),
	Class(String),
	Attr {
		name: String,
		op: AttrOp,
	},
	Not(Selector),
	Has(Selector),
	Contains(String),
	ContainsOwn(String),
	ContainsData(String),
	Matches(Regex),
	FirstChild,
	LastChild,
	OnlyChild,
	NthChild(Nth),
	/// The `-of-type` family counts only siblings with the same tag name.
	FirstOfType,
	LastOfType,
	NthOfType(Nth),
	Empty,
	Root,
	/// jsoup's positional `:eq`/`:gt`/`:lt` over the sibling index.
	IndexEquals(u32),
	IndexGreater(u32),
	IndexLess(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Nth {
	Exact(u32),
	Odd,
	Even,
}

#[derive(Debug, Default)]
struct Compound {
	simples: Vec<Simple>,
}

/// One comma-separated alternative: compounds joined by combinators.
#[derive(Debug)]
struct Complex {
	/// A leading combinator, which only `:has(> a)` style selectors carry.
	leading: Option<Combinator>,
	compounds: Vec<Compound>,
	/// `combinators[i]` joins `compounds[i]` and `compounds[i + 1]`.
	combinators: Vec<Combinator>,
}

/// A parsed selector. Parse once, evaluate many times.
#[derive(Debug)]
pub struct Selector {
	source: String,
	alternatives: Vec<Complex>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid selector `{selector}`: {reason}")]
pub struct SelectorError {
	pub selector: String,
	pub reason: String,
}

impl fmt::Display for Selector {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(&self.source)
	}
}

impl Selector {
	pub fn parse(selector: &str) -> Result<Self, SelectorError> {
		let mut parser = Parser {
			input: selector.as_bytes(),
			position: 0,
			original: selector,
		};
		let alternatives = parser.parse_group()?;
		if alternatives.is_empty() {
			return Err(parser.error("selector is empty"));
		}
		Ok(Self {
			source: selector.to_string(),
			alternatives,
		})
	}

	pub fn source(&self) -> &str {
		&self.source
	}

	/// Whether `node` itself matches.
	pub fn matches(&self, document: &Document, node: NodeId) -> bool {
		document.is_element(node)
			&& self
				.alternatives
				.iter()
				.any(|complex| match_complex(document, node, complex, None))
	}

	/// Every descendant of `root` that matches, in document order.
	pub fn select(&self, document: &Document, root: NodeId) -> Vec<NodeId> {
		let candidates: Vec<NodeId> = if root == document.root() {
			document.ids().collect()
		} else {
			document.descendants(root)
		};
		candidates
			.into_iter()
			.filter(|node| self.matches(document, *node))
			.collect()
	}

	/// The first matching descendant of `root`.
	pub fn select_first(&self, document: &Document, root: NodeId) -> Option<NodeId> {
		if root == document.root() {
			document.ids().find(|node| self.matches(document, *node))
		} else {
			document
				.descendants(root)
				.into_iter()
				.find(|node| self.matches(document, *node))
		}
	}
}

/// Match `complex` with `node` as the subject (rightmost compound).
///
/// `subject_parent` restricts the leftmost compound's parent, which is how
/// `:has(> a)` is evaluated.
fn match_complex(
	document: &Document,
	node: NodeId,
	complex: &Complex,
	subject_parent: Option<NodeId>,
) -> bool {
	let last = complex.compounds.len() - 1;
	if !match_at(document, node, complex, last) {
		return false;
	}
	match (complex.leading, subject_parent) {
		(Some(Combinator::Child), Some(parent)) => {
			// The leftmost compound must be a direct child of `parent`. With a
			// single compound that is `node` itself; otherwise walk left.
			leftmost_candidates(document, node, complex)
				.into_iter()
				.any(|left| document.parent(left) == Some(parent))
		},
		_ => true,
	}
}

/// Nodes that could satisfy `complex`'s leftmost compound given that `node`
/// satisfies the rightmost one.
fn leftmost_candidates(
	document: &Document,
	node: NodeId,
	complex: &Complex,
) -> Vec<NodeId> {
	if complex.compounds.len() == 1 {
		return vec![node];
	}
	// Only the compound-only and single-child forms occur upstream; for longer
	// chains any ancestor satisfying compound 0 is accepted.
	document
		.ancestors(node)
		.filter(|ancestor| match_compound(document, *ancestor, &complex.compounds[0]))
		.collect()
}

fn match_at(document: &Document, node: NodeId, complex: &Complex, index: usize) -> bool {
	if !match_compound(document, node, &complex.compounds[index]) {
		return false;
	}
	if index == 0 {
		return true;
	}
	match complex.combinators[index - 1] {
		Combinator::Descendant => document
			.ancestors(node)
			.any(|ancestor| match_at(document, ancestor, complex, index - 1)),
		Combinator::Child => document
			.parent(node)
			.is_some_and(|parent| match_at(document, parent, complex, index - 1)),
		Combinator::Adjacent => document
			.preceding_siblings(node)
			.first()
			.is_some_and(|sibling| match_at(document, *sibling, complex, index - 1)),
		Combinator::Sibling => document
			.preceding_siblings(node)
			.into_iter()
			.any(|sibling| match_at(document, sibling, complex, index - 1)),
	}
}

fn match_compound(document: &Document, node: NodeId, compound: &Compound) -> bool {
	document.is_element(node)
		&& compound
			.simples
			.iter()
			.all(|simple| match_simple(document, node, simple))
}

fn match_simple(document: &Document, node: NodeId, simple: &Simple) -> bool {
	let element = match document.node(node).element() {
		Some(element) => element,
		None => return false,
	};
	match simple {
		Simple::Any => true,
		Simple::Tag(tag) => element.name == *tag,
		Simple::Id(id) => document.attr(node, "id") == Some(id.as_str()),
		Simple::Class(class) => element.classes.iter().any(|value| value == class),
		Simple::Attr { name, op } => match_attr(document, node, name, op),
		Simple::Not(selector) => !selector.matches(document, node),
		Simple::Has(selector) => selector
			.alternatives
			.iter()
			.any(|complex| match_has(document, node, complex)),
		Simple::Contains(needle) => {
			normalise(&document.text(node).to_lowercase()).contains(needle)
		},
		Simple::ContainsOwn(needle) => {
			normalise(&document.own_text(node).to_lowercase()).contains(needle)
		},
		Simple::ContainsData(needle) => {
			document.data(node).to_lowercase().contains(needle)
		},
		Simple::Matches(regex) => regex.is_match(&document.text(node)),
		Simple::FirstChild => element.sibling_index == 1,
		Simple::LastChild => element.sibling_index == document.parent_element_count(node),
		Simple::OnlyChild => {
			element.sibling_index == 1 && document.parent_element_count(node) == 1
		},
		Simple::NthChild(nth) => match nth {
			Nth::Exact(value) => element.sibling_index == *value,
			Nth::Odd => element.sibling_index % 2 == 1,
			Nth::Even => element.sibling_index % 2 == 0,
		},
		Simple::FirstOfType => document.type_position(node).0 == 1,
		Simple::LastOfType => {
			let (position, total) = document.type_position(node);
			position == total
		},
		Simple::NthOfType(nth) => {
			let position = document.type_position(node).0;
			match nth {
				Nth::Exact(value) => position == *value,
				Nth::Odd => position % 2 == 1,
				Nth::Even => position % 2 == 0,
			}
		},
		Simple::Empty => {
			document.element_children(node).next().is_none()
				&& document.text(node).is_empty()
		},
		Simple::Root => document.parent(node) == Some(document.root()),
		Simple::IndexEquals(value) => element.sibling_index == value + 1,
		Simple::IndexGreater(value) => element.sibling_index > value + 1,
		Simple::IndexLess(value) => element.sibling_index < value + 1,
	}
}

/// `:has(sel)`: a descendant matches, or — when `sel` starts with a
/// combinator — the relation that combinator names holds.
fn match_has(document: &Document, node: NodeId, complex: &Complex) -> bool {
	let candidates = match complex.leading {
		Some(Combinator::Child) => document.element_children(node).collect::<Vec<_>>(),
		Some(Combinator::Adjacent) => following_siblings(document, node)
			.into_iter()
			.take(1)
			.collect(),
		Some(Combinator::Sibling) => following_siblings(document, node),
		_ => document.descendants(node),
	};
	candidates
		.into_iter()
		.any(|candidate| match_complex(document, candidate, complex, Some(node)))
}

fn following_siblings(document: &Document, node: NodeId) -> Vec<NodeId> {
	let Some(parent) = document.parent(node) else {
		return Vec::new();
	};
	let siblings = &document.node(parent).children;
	let Some(position) = siblings.iter().position(|child| *child == node) else {
		return Vec::new();
	};
	siblings[position + 1..]
		.iter()
		.copied()
		.filter(|child| document.is_element(*child))
		.collect()
}

/// jsoup compares attribute values case-insensitively.
fn match_attr(document: &Document, node: NodeId, name: &str, op: &AttrOp) -> bool {
	let value = document.attr(node, name);
	match op {
		AttrOp::Exists => value.is_some(),
		AttrOp::NotEquals(expected) => {
			value.is_none_or(|value| !value.eq_ignore_ascii_case(expected))
		},
		_ => {
			let Some(value) = value else {
				return false;
			};
			let lower = value.to_lowercase();
			match op {
				AttrOp::Equals(expected) => lower == *expected,
				AttrOp::StartsWith(expected) => lower.starts_with(expected.as_str()),
				AttrOp::EndsWith(expected) => lower.ends_with(expected.as_str()),
				AttrOp::Contains(expected) => lower.contains(expected.as_str()),
				AttrOp::Word(expected) => {
					lower.split_ascii_whitespace().any(|word| word == expected)
				},
				AttrOp::Exists | AttrOp::NotEquals(_) => unreachable!(),
			}
		},
	}
}

struct Parser<'a> {
	input: &'a [u8],
	position: usize,
	original: &'a str,
}

impl<'a> Parser<'a> {
	fn error(&self, reason: impl Into<String>) -> SelectorError {
		SelectorError {
			selector: self.original.to_string(),
			reason: reason.into(),
		}
	}

	fn peek(&self) -> Option<u8> {
		self.input.get(self.position).copied()
	}

	fn skip_whitespace(&mut self) -> bool {
		let start = self.position;
		while matches!(self.peek(), Some(b) if b.is_ascii_whitespace()) {
			self.position += 1;
		}
		self.position != start
	}

	fn parse_group(&mut self) -> Result<Vec<Complex>, SelectorError> {
		let mut alternatives = Vec::new();
		loop {
			let complex = self.parse_complex()?;
			alternatives.push(complex);
			self.skip_whitespace();
			match self.peek() {
				Some(b',') => {
					self.position += 1;
					self.skip_whitespace();
				},
				_ => break,
			}
		}
		Ok(alternatives)
	}

	fn parse_complex(&mut self) -> Result<Complex, SelectorError> {
		self.skip_whitespace();
		let leading = match self.peek() {
			Some(b'>') => {
				self.position += 1;
				Some(Combinator::Child)
			},
			Some(b'+') => {
				self.position += 1;
				Some(Combinator::Adjacent)
			},
			Some(b'~') => {
				self.position += 1;
				Some(Combinator::Sibling)
			},
			_ => None,
		};
		self.skip_whitespace();
		let mut compounds = vec![self.parse_compound()?];
		let mut combinators = Vec::new();
		loop {
			let had_space = self.skip_whitespace();
			let combinator = match self.peek() {
				Some(b'>') => {
					self.position += 1;
					Combinator::Child
				},
				Some(b'+') => {
					self.position += 1;
					Combinator::Adjacent
				},
				Some(b'~') => {
					self.position += 1;
					Combinator::Sibling
				},
				Some(b',') | None => break,
				Some(b')') => break,
				Some(_) if had_space => Combinator::Descendant,
				Some(byte) => {
					return Err(self.error(format!(
						"unexpected `{}` at byte {}",
						byte as char, self.position
					)))
				},
			};
			self.skip_whitespace();
			compounds.push(self.parse_compound()?);
			combinators.push(combinator);
		}
		Ok(Complex {
			leading,
			compounds,
			combinators,
		})
	}

	fn parse_compound(&mut self) -> Result<Compound, SelectorError> {
		let mut compound = Compound::default();
		loop {
			match self.peek() {
				Some(b'*') => {
					self.position += 1;
					compound.simples.push(Simple::Any);
				},
				Some(b'#') => {
					self.position += 1;
					let id = self.parse_identifier()?;
					compound.simples.push(Simple::Id(id));
				},
				Some(b'.') => {
					self.position += 1;
					let class = self.parse_identifier()?.to_lowercase();
					compound.simples.push(Simple::Class(class));
				},
				Some(b'[') => {
					self.position += 1;
					compound.simples.push(self.parse_attr()?);
				},
				Some(b':') => {
					self.position += 1;
					compound.simples.push(self.parse_pseudo()?);
				},
				Some(byte) if is_identifier_byte(byte) => {
					let tag = self.parse_identifier()?.to_lowercase();
					compound.simples.push(Simple::Tag(tag));
				},
				_ => break,
			}
		}
		if compound.simples.is_empty() {
			return Err(self.error(format!("empty compound at byte {}", self.position)));
		}
		// Cheap structural tests first: a pseudo-class can walk the subtree.
		compound.simples.sort_by_key(|simple| match simple {
			Simple::Tag(_) => 0,
			Simple::Id(_) => 1,
			Simple::Class(_) => 2,
			Simple::Attr { .. } => 3,
			Simple::Any => 4,
			Simple::FirstChild
			| Simple::LastChild
			| Simple::OnlyChild
			| Simple::NthChild(_)
			| Simple::Root
			| Simple::IndexEquals(_)
			| Simple::IndexGreater(_)
			| Simple::IndexLess(_) => 5,
			_ => 6,
		});
		Ok(compound)
	}

	fn parse_identifier(&mut self) -> Result<String, SelectorError> {
		let mut out = String::new();
		while let Some(byte) = self.peek() {
			if byte == b'\\' {
				self.position += 1;
				if let Some(escaped) = self.peek() {
					out.push(escaped as char);
					self.position += 1;
				}
				continue;
			}
			if !is_identifier_byte(byte) {
				break;
			}
			out.push(byte as char);
			self.position += 1;
		}
		if out.is_empty() {
			return Err(self.error(format!("expected a name at byte {}", self.position)));
		}
		Ok(out)
	}

	fn parse_attr(&mut self) -> Result<Simple, SelectorError> {
		self.skip_whitespace();
		let negated = if self.peek() == Some(b'^') {
			// jsoup's `[^data-]` prefix form is not used by these themes.
			return Err(self.error("attribute name prefix matching is unsupported"));
		} else {
			false
		};
		let _ = negated;
		let name = self.parse_identifier()?.to_lowercase();
		self.skip_whitespace();
		let op = match self.peek() {
			Some(b']') => AttrOp::Exists,
			Some(b'=') => {
				self.position += 1;
				AttrOp::Equals(self.parse_attr_value()?)
			},
			Some(b'^') => {
				self.position += 1;
				self.expect(b'=')?;
				AttrOp::StartsWith(self.parse_attr_value()?)
			},
			Some(b'$') => {
				self.position += 1;
				self.expect(b'=')?;
				AttrOp::EndsWith(self.parse_attr_value()?)
			},
			Some(b'*') => {
				self.position += 1;
				self.expect(b'=')?;
				AttrOp::Contains(self.parse_attr_value()?)
			},
			Some(b'~') => {
				self.position += 1;
				self.expect(b'=')?;
				AttrOp::Word(self.parse_attr_value()?)
			},
			Some(b'!') => {
				self.position += 1;
				self.expect(b'=')?;
				AttrOp::NotEquals(self.parse_attr_value()?)
			},
			other => {
				return Err(self.error(format!(
					"unexpected attribute operator `{}`",
					other.map(|b| b as char).unwrap_or('\0')
				)))
			},
		};
		self.skip_whitespace();
		self.expect(b']')?;
		Ok(Simple::Attr { name, op })
	}

	/// Attribute values may be quoted, and unquoted values may escape `]`,
	/// `=` and whitespace with a backslash (`a[href*=type\=]`).
	fn parse_attr_value(&mut self) -> Result<String, SelectorError> {
		self.skip_whitespace();
		let mut out = String::new();
		match self.peek() {
			Some(quote @ (b'"' | b'\'')) => {
				self.position += 1;
				while let Some(byte) = self.peek() {
					self.position += 1;
					if byte == b'\\' {
						if let Some(escaped) = self.peek() {
							out.push(escaped as char);
							self.position += 1;
						}
						continue;
					}
					if byte == quote {
						return Ok(out.to_lowercase());
					}
					out.push(byte as char);
				}
				Err(self.error("unterminated quoted attribute value"))
			},
			_ => {
				while let Some(byte) = self.peek() {
					if byte == b'\\' {
						self.position += 1;
						if let Some(escaped) = self.peek() {
							out.push(escaped as char);
							self.position += 1;
						}
						continue;
					}
					if byte == b']' || byte.is_ascii_whitespace() {
						break;
					}
					out.push(byte as char);
					self.position += 1;
				}
				Ok(out.to_lowercase())
			},
		}
	}

	fn parse_pseudo(&mut self) -> Result<Simple, SelectorError> {
		let name = self.parse_pseudo_name()?;
		let takes_argument =
			matches!(
				name.as_str(),
				"not"
					| "has" | "contains"
					| "containsown" | "containsdata"
					| "matches" | "nth-child"
					| "nth-of-type" | "eq"
					| "gt" | "lt"
			);
		let argument = if takes_argument {
			self.expect(b'(')?;
			Some(self.parse_balanced()?)
		} else {
			None
		};
		match (name.as_str(), argument) {
			("not", Some(inner)) => Ok(Simple::Not(Selector::parse(&inner)?)),
			("has", Some(inner)) => Ok(Simple::Has(Selector::parse(&inner)?)),
			("contains", Some(text)) => {
				Ok(Simple::Contains(normalise(&unquote(&text).to_lowercase())))
			},
			("containsown", Some(text)) => Ok(Simple::ContainsOwn(normalise(
				&unquote(&text).to_lowercase(),
			))),
			("containsdata", Some(text)) => {
				Ok(Simple::ContainsData(unquote(&text).to_lowercase()))
			},
			("matches", Some(pattern)) => Regex::new(&unquote(&pattern))
				.map(Simple::Matches)
				.map_err(|error| self.error(error.to_string())),
			("nth-child", Some(argument)) => {
				self.nth_pseudo("nth-child", &argument, Simple::NthChild)
			},
			("nth-of-type", Some(argument)) => {
				self.nth_pseudo("nth-of-type", &argument, Simple::NthOfType)
			},
			("eq", Some(argument)) => self.index_pseudo(&argument, Simple::IndexEquals),
			("gt", Some(argument)) => self.index_pseudo(&argument, Simple::IndexGreater),
			("lt", Some(argument)) => self.index_pseudo(&argument, Simple::IndexLess),
			("first-child", None) => Ok(Simple::FirstChild),
			("last-child", None) => Ok(Simple::LastChild),
			("only-child", None) => Ok(Simple::OnlyChild),
			("empty", None) => Ok(Simple::Empty),
			("root", None) => Ok(Simple::Root),
			("first-of-type", None) => Ok(Simple::FirstOfType),
			("last-of-type", None) => Ok(Simple::LastOfType),
			(other, _) => Err(self.error(format!("unsupported pseudo-class `:{other}`"))),
		}
	}

	/// `:nth-child`/`:nth-of-type` share jsoup's `n | odd | even` argument.
	/// The `an+b` forms no theme uses stay a parse error.
	fn nth_pseudo(
		&self,
		name: &str,
		argument: &str,
		build: fn(Nth) -> Simple,
	) -> Result<Simple, SelectorError> {
		match argument.trim().to_ascii_lowercase().as_str() {
			"odd" => Ok(build(Nth::Odd)),
			"even" => Ok(build(Nth::Even)),
			value => value
				.parse()
				.map(|value| build(Nth::Exact(value)))
				.map_err(|_| self.error(format!("unsupported :{name}({value})"))),
		}
	}

	fn index_pseudo(
		&self,
		argument: &str,
		build: fn(u32) -> Simple,
	) -> Result<Simple, SelectorError> {
		argument
			.trim()
			.parse()
			.map(build)
			.map_err(|_| self.error(format!("expected an index, got `{argument}`")))
	}

	fn parse_pseudo_name(&mut self) -> Result<String, SelectorError> {
		let mut out = String::new();
		while let Some(byte) = self.peek() {
			if byte.is_ascii_alphanumeric() || byte == b'-' {
				out.push(byte.to_ascii_lowercase() as char);
				self.position += 1;
				continue;
			}
			break;
		}
		if out.is_empty() {
			return Err(self.error("expected a pseudo-class name"));
		}
		Ok(out)
	}

	/// Consume up to the parenthesis that closes the one already consumed.
	fn parse_balanced(&mut self) -> Result<String, SelectorError> {
		let mut depth = 1usize;
		let start = self.position;
		while let Some(byte) = self.peek() {
			match byte {
				b'\\' => {
					self.position += 1;
					if self.peek().is_some() {
						self.position += 1;
					}
					continue;
				},
				b'(' => depth += 1,
				b')' => {
					depth -= 1;
					if depth == 0 {
						let text = std::str::from_utf8(&self.input[start..self.position])
							.map_err(|_| self.error("selector is not valid UTF-8"))?
							.to_string();
						self.position += 1;
						return Ok(text);
					}
				},
				_ => {},
			}
			self.position += 1;
		}
		Err(self.error("unbalanced `(`"))
	}

	fn expect(&mut self, byte: u8) -> Result<(), SelectorError> {
		if self.peek() == Some(byte) {
			self.position += 1;
			return Ok(());
		}
		Err(self.error(format!(
			"expected `{}` at byte {}",
			byte as char, self.position
		)))
	}
}

fn is_identifier_byte(byte: u8) -> bool {
	byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'|') || byte >= 0x80
}

fn unquote(value: &str) -> String {
	let trimmed = value.trim();
	let bytes = trimmed.as_bytes();
	if bytes.len() >= 2
		&& (bytes[0] == b'"' || bytes[0] == b'\'')
		&& bytes[bytes.len() - 1] == bytes[0]
	{
		return trimmed[1..trimmed.len() - 1].to_string();
	}
	trimmed.to_string()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn document(html: &str) -> Document {
		Document::parse(html, "https://site.test/page/")
	}

	fn texts(selector: &str, html: &str) -> Vec<String> {
		let document = document(html);
		let selector = Selector::parse(selector).expect("selector parses");
		selector
			.select(&document, document.root())
			.into_iter()
			.map(|node| document.text(node))
			.collect()
	}

	#[test]
	fn groups_combinators_and_compounds() {
		let html = r#"<ul>
			<li class="a">one</li>
			<li class="b">two</li>
			<li class="a b">three</li>
		</ul><ol><li class="a">four</li></ol>"#;
		assert_eq!(texts("li.a", html), ["one", "three", "four"]);
		assert_eq!(texts("ul > li.a", html), ["one", "three"]);
		assert_eq!(texts("li.a + li.b", html), ["two"]);
		assert_eq!(texts("li.b ~ li", html), ["three"]);
		// `li.a b` also carries `b`, so both `ul` items match the group.
		assert_eq!(texts("ol li, ul li.b", html), ["two", "three", "four"]);
		assert_eq!(texts("ul li:last-child", html), ["three"]);
		assert_eq!(texts("ul li:nth-child(2)", html), ["two"]);
		assert_eq!(texts("*.b", html), ["two", "three"]);
	}

	#[test]
	fn attribute_operators_are_case_insensitive_and_escapable() {
		let html = r#"<div>
			<a id="x" href="/Manga/Foo/" rel="next nofollow">a</a>
			<a href="https://other.test/type=1" rel="next">b</a>
			<a>c</a>
		</div>"#;
		assert_eq!(texts("a[href]", html), ["a", "b"]);
		assert_eq!(texts("a[href^=/manga]", html), ["a"]);
		assert_eq!(texts("a[href$=foo/]", html), ["a"]);
		assert_eq!(texts(r"a[href*=type\=]", html), ["b"]);
		assert_eq!(texts("a[rel~=next]", html), ["a", "b"]);
		assert_eq!(texts("a[href='/MANGA/foo/']", html), ["a"]);
		assert_eq!(texts("#x", html), ["a"]);
		// jsoup's `!=` compares the whole value, so the multi-token `rel` on
		// `a` is "not next" while the single-token one on `b` is not.
		assert_eq!(texts("a[rel!=next]", html), ["a", "c"]);
	}

	#[test]
	fn jsoup_contains_and_not_and_has() {
		let html = r#"<div class="row">
			<div class="post-content_item"><h5>Type</h5><div class="summary-content">Manga</div></div>
			<div class="post-content_item"><h5>Alt</h5><div class="summary-content">Other Name</div></div>
			<ul>
				<li><div class="chbox"></div><div class="eph-num">1</div></li>
				<li><div class="chbox"></div></li>
			</ul>
			<img class="thumb" alt="skip"><img alt="keep">
		</div>"#;
		assert_eq!(
			texts(".post-content_item:contains(Type) .summary-content", html),
			["Manga"]
		);
		assert_eq!(
			texts(".post-content_item:contains(alt) .summary-content", html),
			["Other Name"]
		);
		assert_eq!(
			texts("ul li:has(div.chbox):has(div.eph-num)", html).len(),
			1
		);
		let document = document(html);
		let selector = Selector::parse("img:not(.thumb)").unwrap();
		let matched = selector.select(&document, document.root());
		assert_eq!(matched.len(), 1);
		assert_eq!(document.attr(matched[0], "alt"), Some("keep"));
	}

	#[test]
	fn has_supports_a_leading_child_combinator() {
		let html = r#"<div><span><a href="/x">deep</a></span></div>
			<div><a href="/y">shallow</a></div>"#;
		let document = document(html);
		let direct = Selector::parse("div:has(> a)").unwrap();
		let matched = direct.select(&document, document.root());
		assert_eq!(matched.len(), 1);
		assert_eq!(document.text(matched[0]), "shallow");

		let descendant = Selector::parse("div:has(a)").unwrap();
		assert_eq!(descendant.select(&document, document.root()).len(), 2);
	}

	#[test]
	fn contains_data_reads_script_bodies() {
		let html = r#"<script>var x = 1;</script>
			<script>ts_reader.run({"sources":[{"images":["a.jpg"]}]});</script>"#;
		let document = document(html);
		let selector = Selector::parse("script:containsData(ts_reader)").unwrap();
		let matched = selector.select(&document, document.root());
		assert_eq!(matched.len(), 1);
		assert!(document.data(matched[0]).contains("images"));
	}

	#[test]
	fn real_theme_selectors_parse() {
		// Verbatim defaults from the three base classes at 064c1a0e.
		for selector in [
			"div.page-item-detail, .manga__item, .c-tabs-item__content",
			"div.post-title h3, div.post-title h1, #manga-title > h1",
			"div.summary-content, div.summary-heading:contains(Status) + div",
			"div.description-summary div.summary__content, div.summary_content div.post-content_item > h5 + div, div.summary_content div.manga-excerpt",
			"div.page-break, li.blocks-gallery-item, .reading-content .text-left:not(:has(.blocks-gallery-item)) img",
			"div.page-item-detail:not(:has(a[href*='bilibilicomics.com'])) , .manga__item",
			".utao .uta .imgu, .listupd .bs .bsx, .listo .bs .bsx",
			"div.bxcl li, div.cl li, #chapterlist li, ul li:has(div.chbox):has(div.eph-num)",
			".infotable tr:contains(artist) td:last-child, .tsinfo .imptdt:contains(artist) i, .fmed b:contains(artist)+span, span:contains(artist)",
			"ul.chapters > li:not(.btn)",
			"#all > img.img-responsive",
			".pagination a[rel=next]",
			"div.comic-list-layout .grid > .group",
			"div a:has(img)",
			"dl div:contains(Genres:) a",
			"div:has(span:contains(Author:)) > a",
			"[id^=manga-chapters-holder]",
			"script:containsData(dynamic_view_ajax)",
			"select[name='categories[]'] option",
			"#sort-types label:has(input)",
			"a[href*='/manga-genre/']",
			"link[rel=shortlink]",
			"input[type=checkbox]",
			"time[itemprop=dateModified]",
		] {
			Selector::parse(selector)
				.unwrap_or_else(|error| panic!("{selector}: {error}"));
		}
	}

	/// `all.allporncomicsco` selects its archive link with
	/// `h3 > a:not([target=_self]):last-of-type`, so the family has to count
	/// siblings by tag name rather than by position among all elements.
	#[test]
	fn of_type_counts_only_siblings_with_the_same_tag() {
		let html = r#"<div>
			<h3><span>x</span><a target="_self">self</a><a href="/1">mid</a><a href="/2">last</a></h3>
			<h3><a href="/3">only</a></h3>
		</div>"#;
		assert_eq!(
			texts("h3 > a:not([target=_self]):last-of-type", html),
			["last", "only"]
		);
		// The leading `span` does not shift the count, and `:first-of-type`
		// is not "the first link that is not `_self`".
		assert_eq!(texts("h3 > a:first-of-type", html), ["self", "only"]);
		assert_eq!(texts("h3 > span:last-of-type", html), ["x"]);
		assert_eq!(texts("h3 a:nth-of-type(2)", html), ["mid"]);
		assert_eq!(
			texts("h3 a:nth-of-type(odd)", html),
			["self", "last", "only"]
		);
	}

	#[test]
	fn unsupported_syntax_is_rejected_loudly() {
		assert!(Selector::parse(":nth-last-of-type(2)").is_err());
		assert!(Selector::parse("li:nth-child(2n+1)").is_err());
		assert!(Selector::parse("div:has(").is_err());
		assert!(Selector::parse("").is_err());
		assert!(Selector::parse("a[href").is_err());
		assert!(Selector::parse("a[href@=x]").is_err());
	}
}
