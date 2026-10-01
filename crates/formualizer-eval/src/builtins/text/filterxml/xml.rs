//! The XML document FILTERXML queries, loaded as MSXML loads it.
//!
//! `sxd-document` parses the text. Before that, the text goes through the
//! XML 1.0 steps that parser leaves out: end-of-line handling (§2.11),
//! normalization of the white space written in attribute values (§3.3.3),
//! and the rejection of characters outside the `Char` production (§2.2, a
//! fatal error). The parsed tree is then copied into a flat list of nodes in
//! document order, the XPath 1.0 data model: white-space-only text is dropped
//! (MSXML does not preserve white space unless `xml:space="preserve"`), and
//! adjacent text (split by references and CDATA sections) is one text node.

use std::ops::Range;

use sxd_document::dom::{self, ChildOfElement, ChildOfRoot};

/// The namespace of the predeclared `xml:` prefix (`xml:space`, `xml:lang`).
pub(super) const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

/// XML white space: space, tab, carriage return and line feed.
pub(super) fn is_xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// Whether XML 1.0 allows `c` in a document (the `Char` production).
fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Root,
    Element,
    Namespace,
    Attribute,
    Text,
    Comment,
    ProcessingInstruction,
}

#[derive(Debug)]
pub(super) struct Node {
    pub kind: Kind,
    /// The parent node; an attribute's or a namespace node's is its element.
    /// The root has none.
    pub parent: Option<usize>,
    /// One past the last node of this node's subtree: the nodes between a node
    /// and `end` are its namespace nodes, attributes and descendants.
    pub end: usize,
    /// The child nodes (elements, text, comments, processing instructions).
    pub children: Vec<usize>,
    /// The position of this node in its parent's `children`.
    pub sibling_index: usize,
    /// An element's namespace nodes, then its attributes, directly follow it.
    pub namespaces: Range<usize>,
    pub attributes: Range<usize>,
    /// The namespace URI of an element or attribute name.
    pub namespace_uri: Option<String>,
    /// The prefix of an element or attribute name in the source.
    pub prefix: Option<String>,
    /// The local part of an element or attribute name, a processing
    /// instruction's target, a namespace node's prefix; empty otherwise.
    pub local_name: String,
    /// The text of a text node or comment, an attribute's value, a processing
    /// instruction's data, a namespace node's URI; empty otherwise.
    pub value: String,
}

impl Node {
    fn new(kind: Kind, parent: Option<usize>) -> Self {
        Node {
            kind,
            parent,
            end: 0,
            children: Vec::new(),
            sibling_index: 0,
            namespaces: 0..0,
            attributes: 0..0,
            namespace_uri: None,
            prefix: None,
            local_name: String::new(),
            value: String::new(),
        }
    }

    /// Whether the node is in the tree proper: not an attribute or a namespace
    /// node, which no axis but `attribute` and `namespace` reaches.
    pub fn is_tree_node(&self) -> bool {
        !matches!(self.kind, Kind::Attribute | Kind::Namespace)
    }
}

/// A node of the parsed document waiting for its place in document order.
enum Pending<'d> {
    /// An element, and whether white space is preserved in its parent.
    Element(dom::Element<'d>, bool),
    Text(String),
    Comment(&'d str),
    ProcessingInstruction(&'d str, Option<&'d str>),
}

/// An XML document as a list of nodes in document order; node 0 is the root.
#[derive(Debug)]
pub(super) struct Document {
    pub nodes: Vec<Node>,
}

impl Document {
    /// Loads `xml`; `None` when it is not well-formed XML 1.0. Namespace nodes
    /// are only made when `with_namespaces` (only the namespace axis reaches
    /// them).
    pub fn parse(xml: &str, with_namespaces: bool) -> Option<Document> {
        let xml = normalize(xml)?;
        let package = sxd_document::parser::parse(&xml).ok()?;
        let source = package.as_document();
        let mut nodes = vec![Node::new(Kind::Root, None)];
        let mut pending: Vec<(Pending, usize)> = source
            .root()
            .children()
            .into_iter()
            .rev()
            .map(|child| {
                let child = match child {
                    ChildOfRoot::Element(element) => Pending::Element(element, false),
                    ChildOfRoot::Comment(comment) => Pending::Comment(comment.text()),
                    ChildOfRoot::ProcessingInstruction(pi) => {
                        Pending::ProcessingInstruction(pi.target(), pi.value())
                    }
                };
                (child, 0)
            })
            .collect();
        // Depth first, so each node is added in document order.
        while let Some((child, parent)) = pending.pop() {
            let index = nodes.len();
            let sibling_index = nodes[parent].children.len();
            nodes[parent].children.push(index);
            let mut node = Node::new(Kind::Text, Some(parent));
            node.sibling_index = sibling_index;
            match child {
                Pending::Text(text) => node.value = text,
                Pending::Comment(text) => {
                    node.kind = Kind::Comment;
                    node.value = text.into();
                }
                Pending::ProcessingInstruction(target, value) => {
                    node.kind = Kind::ProcessingInstruction;
                    node.local_name = target.into();
                    node.value = value.unwrap_or("").into();
                }
                Pending::Element(element, inherited) => {
                    node.kind = Kind::Element;
                    let name = element.name();
                    node.namespace_uri = name.namespace_uri().map(Into::into);
                    node.prefix = element.preferred_prefix().map(Into::into);
                    node.local_name = name.local_part().into();
                    nodes.push(node);
                    add_namespaces_and_attributes(&mut nodes, index, element, with_namespaces);
                    let preserve = match element.attribute_value((XML_NAMESPACE, "space")) {
                        Some("preserve") => true,
                        Some("default") => false,
                        _ => inherited,
                    };
                    let first = pending.len();
                    for child in element_children(element, preserve) {
                        pending.push((child, index));
                    }
                    pending[first..].reverse();
                    continue;
                }
            }
            nodes.push(node);
        }
        // A subtree ends where the last of its descendants does; descendants
        // come after their ancestors.
        for index in (0..nodes.len()).rev() {
            nodes[index].end = nodes[index].end.max(index + 1);
            if let Some(parent) = nodes[index].parent {
                nodes[parent].end = nodes[parent].end.max(nodes[index].end);
            }
        }
        Some(Document { nodes })
    }

    /// The descendants of `node` in document order.
    pub fn descendants(&self, node: usize) -> impl DoubleEndedIterator<Item = usize> + '_ {
        (node + 1..self.nodes[node].end).filter(|&index| self.nodes[index].is_tree_node())
    }

    /// The children of the parent of `node`, when it is in the tree and not
    /// the root.
    pub fn siblings(&self, node: usize) -> &[usize] {
        match self.nodes[node].parent {
            Some(parent) if self.nodes[node].is_tree_node() => &self.nodes[parent].children,
            _ => &[],
        }
    }

    /// The XPath string-value of `node`: the text it contains for the root
    /// and elements, its value for the other nodes.
    pub fn string_value(&self, node: usize) -> String {
        match self.nodes[node].kind {
            Kind::Root | Kind::Element => self
                .descendants(node)
                .filter(|&index| self.nodes[index].kind == Kind::Text)
                .map(|index| self.nodes[index].value.as_str())
                .collect(),
            _ => self.nodes[node].value.clone(),
        }
    }
}

/// Adds the namespace nodes and attributes of `element`, the node at `index`.
fn add_namespaces_and_attributes(
    nodes: &mut Vec<Node>,
    index: usize,
    element: dom::Element,
    with_namespaces: bool,
) {
    let start = nodes.len();
    if with_namespaces {
        let default = element
            .recursive_default_namespace_uri()
            .map(|uri| ("", uri));
        let prefixed = element
            .namespaces_in_scope()
            .into_iter()
            .map(|namespace| (namespace.prefix(), namespace.uri()));
        for (prefix, uri) in default.into_iter().chain(prefixed) {
            let mut node = Node::new(Kind::Namespace, Some(index));
            node.local_name = prefix.into();
            node.value = uri.into();
            nodes.push(node);
        }
    }
    let middle = nodes.len();
    for attribute in element.attributes() {
        let name = attribute.name();
        let mut node = Node::new(Kind::Attribute, Some(index));
        node.namespace_uri = name.namespace_uri().map(Into::into);
        node.prefix = attribute.preferred_prefix().map(Into::into);
        node.local_name = name.local_part().into();
        node.value = attribute.value().into();
        nodes.push(node);
    }
    nodes[index].namespaces = start..middle;
    nodes[index].attributes = middle..nodes.len();
}

/// The children of `element`, adjacent text as one node, white-space-only
/// text dropped unless `preserve`.
fn element_children(element: dom::Element<'_>, preserve: bool) -> Vec<Pending<'_>> {
    let mut children = Vec::new();
    let mut text = String::new();
    let flush = |text: &mut String, children: &mut Vec<Pending>| {
        if !text.is_empty() && (preserve || !text.chars().all(is_xml_space)) {
            children.push(Pending::Text(std::mem::take(text)));
        }
        text.clear();
    };
    for child in element.children() {
        let child = match child {
            ChildOfElement::Text(part) => {
                text.push_str(part.text());
                continue;
            }
            ChildOfElement::Element(element) => Pending::Element(element, preserve),
            ChildOfElement::Comment(comment) => Pending::Comment(comment.text()),
            ChildOfElement::ProcessingInstruction(pi) => {
                Pending::ProcessingInstruction(pi.target(), pi.value())
            }
        };
        flush(&mut text, &mut children);
        children.push(child);
    }
    flush(&mut text, &mut children);
    children
}

/// `xml` as an XML 1.0 processor reads it: a CR LF pair or a lone CR is a
/// line feed (§2.11), and a tab or line feed written in an attribute value is
/// a space (§3.3.3; a character reference keeps its character). `None` when
/// the text holds a character outside the `Char` production, written or as a
/// character reference (§2.2, WFC Legal Character): a fatal error.
fn normalize(xml: &str) -> Option<String> {
    let mut text = String::with_capacity(xml.len());
    let mut chars = xml.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            chars.next_if_eq(&'\n');
            text.push('\n');
        } else if is_xml_char(c) {
            text.push(c);
        } else {
            return None;
        }
    }
    let mut bytes = text.into_bytes();
    let mut at = 0;
    loop {
        // Character data up to the next markup.
        let markup = bytes[at..]
            .iter()
            .position(|&b| b == b'<')
            .map_or(bytes.len(), |offset| at + offset);
        if !references_are_legal(&bytes[at..markup]) {
            return None;
        }
        if markup == bytes.len() {
            break;
        }
        let rest = &bytes[markup..];
        at = if rest.starts_with(b"<!--") {
            end_of(&bytes, markup + 4, b"-->")
        } else if rest.starts_with(b"<![CDATA[") {
            end_of(&bytes, markup + 9, b"]]>")
        } else if rest.starts_with(b"<?") {
            end_of(&bytes, markup + 2, b"?>")
        } else {
            // A tag, or a declaration (`<!DOCTYPE`, whose quoted strings are
            // not attribute values).
            let attributes = rest.get(1) != Some(&b'!');
            let end = end_of_tag(&mut bytes, markup + 1, attributes);
            if !references_are_legal(&bytes[markup..end]) {
                return None;
            }
            end
        };
    }
    // Only ASCII bytes were replaced, by ASCII bytes.
    String::from_utf8(bytes).ok()
}

/// Whether each character reference (`&#N;`, `&#xH;`) in `bytes` names a
/// character XML allows. A malformed one is left for the parser to reject.
fn references_are_legal(bytes: &[u8]) -> bool {
    let mut rest = bytes;
    while let Some(offset) = rest.windows(2).position(|window| window == b"&#") {
        rest = &rest[offset + 2..];
        let (radix, digits) = match rest.split_first() {
            Some((b'x', digits)) => (16, digits),
            _ => (10, rest),
        };
        let len = digits
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric())
            .count();
        if len == 0 || digits.get(len) != Some(&b';') {
            continue;
        }
        let named = std::str::from_utf8(&digits[..len])
            .ok()
            .and_then(|digits| u32::from_str_radix(digits, radix).ok())
            .and_then(char::from_u32);
        if !named.is_some_and(is_xml_char) {
            return false;
        }
    }
    true
}

/// The index just past the first `delimiter` at or after `from`.
fn end_of(bytes: &[u8], from: usize, delimiter: &[u8]) -> usize {
    bytes[from.min(bytes.len())..]
        .windows(delimiter.len())
        .position(|window| window == delimiter)
        .map_or(bytes.len(), |offset| from + offset + delimiter.len())
}

/// The index just past the `>` that closes the markup at `from`, outside
/// quotes. With `attributes`, a tab or line feed inside the quotes (an
/// attribute value) becomes a space.
fn end_of_tag(bytes: &mut [u8], from: usize, attributes: bool) -> usize {
    let mut quote = None;
    for (at, byte) in bytes.iter_mut().enumerate().skip(from) {
        match (quote, *byte) {
            (None, b'>') => return at + 1,
            (None, q @ (b'"' | b'\'')) => quote = Some(q),
            (Some(q), b) if b == q => quote = None,
            (Some(_), b'\t' | b'\n') if attributes => *byte = b' ',
            _ => {}
        }
    }
    bytes.len()
}
