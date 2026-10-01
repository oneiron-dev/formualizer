//! The XML document FILTERXML queries, loaded as MSXML loads it.
//!
//! The text first goes through the XML 1.0 steps that come before parsing:
//! end-of-line handling (§2.11), normalization of the white space written in
//! attribute values (§3.3.3), and the rejection of characters outside the
//! `Char` production (§2.2, a fatal error). quick-xml then splits it into
//! markup and character data and checks that tags nest; this module checks the
//! rest of what makes the text well-formed XML 1.0 with Namespaces in XML 1.0
//! (names, attributes, references, comments, processing instructions, the
//! declarations and where each may appear). Text that is not well-formed is
//! `None`, which FILTERXML reports as `#VALUE!`.
//!
//! The document is a flat list of nodes in document order, the XPath 1.0 data
//! model:
//! - white-space-only text is dropped (MSXML does not preserve white space
//!   unless `xml:space="preserve"`), and adjacent text (split by references
//!   and CDATA sections) is one text node;
//! - attributes keep their source order, as MSXML keeps them;
//! - names are resolved by the namespace declarations in scope (Namespaces in
//!   XML 1.0 §6): a declaration covers its element and the element's content,
//!   an inner one for the same prefix overrides it, and `xmlns=""` takes the
//!   default namespace away for the element's whole subtree;
//! - namespace nodes are made only for the elements the namespace axis
//!   reaches (see [`Document::namespaces`] for their order).

use std::cell::{Cell, OnceCell, RefCell};
use std::hash::{Hash, Hasher};
use std::ops::Range;

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};

/// The namespace of the predeclared `xml:` prefix (`xml:space`, `xml:lang`).
pub(super) const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

/// The namespace of `xmlns` and `xmlns:` attributes, which no prefix may be
/// bound to.
const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

/// XML white space: space, tab, carriage return and line feed.
pub(super) fn is_xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// Whether XML 1.0 allows `c` in a document (the `Char` production).
fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

/// The XML `NameStartChar` production, without the colon.
pub(super) fn is_name_start_char(c: char) -> bool {
    matches!(c,
        'A'..='Z' | '_' | 'a'..='z' | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}'
        | '\u{F8}'..='\u{2FF}' | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}'
        | '\u{200C}'..='\u{200D}' | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}'
        | '\u{3001}'..='\u{D7FF}' | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}'
        | '\u{10000}'..='\u{EFFFF}')
}

/// The XML `NameChar` production, without the colon.
pub(super) fn is_name_char(c: char) -> bool {
    is_name_start_char(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// An `NCName` (Namespaces in XML 1.0): a name without a colon.
fn is_ncname(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(is_name_start_char) && chars.all(is_name_char)
}

/// A `QName`: an `NCName`, or two joined by a colon (a prefix and a local part).
fn is_qname(name: &str) -> bool {
    match name.split_once(':') {
        Some((prefix, local)) => is_ncname(prefix) && is_ncname(local),
        None => is_ncname(name),
    }
}

/// An XML 1.0 `Name` (colons allowed): a processing instruction's target, the
/// name in a document type declaration.
fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c == ':' || is_name_start_char(c))
        && chars.all(|c| c == ':' || is_name_char(c))
}

/// The prefix and the local part of a `QName`.
fn split_qname(name: &str) -> (Option<&str>, &str) {
    match name.split_once(':') {
        Some((prefix, local)) => (Some(prefix), local),
        None => (None, name),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum Kind {
    Root,
    Element,
    Namespace,
    Attribute,
    Text,
    Comment,
    ProcessingInstruction,
}

/// A node of the document other than a namespace node.
#[derive(Debug)]
struct Node {
    kind: Kind,
    /// The parent node; an attribute's is its element. The root has none.
    parent: Option<usize>,
    /// One past the last node of this node's subtree: the nodes between a node
    /// and `end` are its attributes and descendants.
    end: usize,
    /// The child nodes (elements, text, comments, processing instructions).
    children: Vec<usize>,
    /// The position of this node in its parent's `children`.
    sibling_index: usize,
    /// An element's attributes, which directly follow it, in source order.
    attributes: Range<usize>,
    /// The namespace declarations written on an element, in source order.
    declarations: Range<usize>,
    /// The nearest element, this one included, that declares a namespace (for
    /// the root and elements).
    scope: Option<usize>,
    /// The declaration that binds the namespace of an element or attribute
    /// name; none when the name has no namespace.
    namespace: Option<usize>,
    /// The prefix of an element or attribute name in the source.
    prefix: Option<String>,
    /// The local part of an element or attribute name, a processing
    /// instruction's target; empty otherwise.
    local_name: String,
    /// The text of a text node or comment, an attribute's value, a processing
    /// instruction's data; empty otherwise.
    value: String,
}

impl Node {
    fn new(kind: Kind) -> Self {
        Node {
            kind,
            parent: None,
            end: 0,
            children: Vec::new(),
            sibling_index: 0,
            attributes: 0..0,
            declarations: 0..0,
            scope: None,
            namespace: None,
            prefix: None,
            local_name: String::new(),
            value: String::new(),
        }
    }
}

/// A namespace declaration: `xmlns:prefix="uri"`, or `xmlns="uri"` (an empty
/// prefix) for the default namespace, where an empty URI takes it away.
#[derive(Debug)]
struct Declaration {
    prefix: String,
    uri: String,
}

/// A namespace node: one prefix (or the default namespace) in scope for an
/// element.
#[derive(Debug, Clone, Copy)]
struct NamespaceNode {
    element: usize,
    declaration: usize,
    /// Its position among the element's namespace nodes.
    ordinal: usize,
}

/// The namespace nodes made so far, for the elements the namespace axis
/// reached.
#[derive(Debug, Default)]
struct NamespaceNodes {
    nodes: Vec<NamespaceNode>,
    /// The ids of each element's namespace nodes.
    of_element: FxHashMap<usize, Range<usize>>,
    /// The declarations in scope below each declaring element (`None`: below
    /// none, so only the `xml` prefix's), in document order.
    in_scope: FxHashMap<Option<usize>, Vec<usize>>,
}

/// An XML document as nodes in document order; node 0 is the root.
///
/// A node is named by an id: the nodes of the tree and the attributes by
/// their index in document order, the namespace nodes (made when first
/// reached) by ids from [`Document::len`] up. [`Document::order`] gives the
/// document order of any id.
#[derive(Debug)]
pub(super) struct Document {
    nodes: Vec<Node>,
    /// Every namespace declaration, in document order; 0 is the predeclared
    /// `xml` prefix.
    declarations: Vec<Declaration>,
    /// The text nodes, in document order.
    texts: Vec<usize>,
    /// For each index (and one past the last), the number of text nodes before
    /// it.
    texts_before: Vec<usize>,
    /// The string-values of elements whose text is in several text nodes,
    /// joined when first asked for.
    joined_text: Vec<OnceCell<Box<str>>>,
    namespace_nodes: RefCell<NamespaceNodes>,
    /// The kind of each node (other than namespace nodes), for the axis loops.
    kinds: Box<[Kind]>,
    /// The end of each tree node's subtree; `usize::MAX` for an attribute.
    tree_ends: Box<[usize]>,
    /// For each element and attribute, an id of its expanded name (namespace
    /// and local name); [`Document::NO_NAME`] for the other nodes.
    name_ids: Box<[u32]>,
    /// The expanded names' ids, by [`name_key`].
    names: FxHashMap<String, u32>,
    /// The ids of the nodes' string-values ([`Document::value_id`]).
    value_ids: ValueIds,
}

/// Ids for string-values: nodes with equal string-values get the same id,
/// given out when a node's is first asked for.
#[derive(Debug, Default)]
struct ValueIds {
    /// Each node's id, [`ValueIds::UNSET`] until asked for.
    of_node: Box<[Cell<u32>]>,
    /// The namespace nodes' ids.
    of_namespace_node: RefCell<FxHashMap<usize, u32>>,
    /// The ids given out, by the hash of the string-value: the id and a node
    /// with that string-value.
    by_hash: RefCell<FxHashMap<u64, Vec<(u32, usize)>>>,
    /// The number of ids given out.
    count: Cell<u32>,
}

impl ValueIds {
    const UNSET: u32 = u32::MAX;
}

impl Document {
    /// Loads `xml`; `None` when it is not well-formed XML 1.0 with namespaces.
    pub fn parse(xml: &str) -> Option<Document> {
        // A byte order mark belongs to an encoded file, not to text.
        if xml.starts_with('\u{FEFF}') {
            return None;
        }
        let text = normalize(xml)?;
        let mut builder = Builder::new();
        let mut reader = Reader::from_str(&text);
        let config = reader.config_mut();
        config.check_end_names = true;
        config.allow_unmatched_ends = false;
        config.allow_dangling_amp = false;
        config.expand_empty_elements = false;
        config.trim_text(false);
        loop {
            let start = usize::try_from(reader.buffer_position()).ok()?;
            let event = reader.read_event().ok()?;
            match event {
                Event::Eof => break,
                Event::Start(tag) => builder.start_tag(utf8(&tag)?, false)?,
                Event::Empty(tag) => builder.start_tag(utf8(&tag)?, true)?,
                // quick-xml has matched the name with the open element's.
                Event::End(_) => builder.end_tag()?,
                Event::Text(text) => builder.text(utf8(&text)?)?,
                Event::GeneralRef(reference) => builder.reference(utf8(&reference)?)?,
                Event::CData(text) => builder.cdata(utf8(&text)?)?,
                Event::Comment(text) => builder.comment(utf8(&text)?)?,
                Event::PI(content) => builder.processing_instruction(utf8(&content)?)?,
                Event::Decl(content) => builder.xml_declaration(utf8(&content)?)?,
                Event::DocType(_) => {
                    let end = usize::try_from(reader.buffer_position()).ok()?;
                    builder.document_type(text.get(start..end)?)?
                }
            }
        }
        builder.finish()
    }

    /// The number of nodes other than namespace nodes; the first namespace
    /// node's id.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    fn namespace_node(&self, id: usize) -> NamespaceNode {
        self.namespace_nodes.borrow().nodes[id - self.nodes.len()]
    }

    #[inline(always)]
    pub fn kind(&self, id: usize) -> Kind {
        match self.nodes.get(id) {
            Some(node) => node.kind,
            None => Kind::Namespace,
        }
    }

    /// Whether the node is in the tree proper: not an attribute or a namespace
    /// node, which no axis but `attribute` and `namespace` reaches.
    #[inline(always)]
    pub fn is_tree_node(&self, id: usize) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|node| node.kind != Kind::Attribute)
    }

    /// The parent node; an attribute's or a namespace node's is its element.
    #[inline(always)]
    pub fn parent(&self, id: usize) -> Option<usize> {
        match self.nodes.get(id) {
            Some(node) => node.parent,
            None => Some(self.namespace_node(id).element),
        }
    }

    /// The local part of an element or attribute name, a processing
    /// instruction's target, a namespace node's prefix; empty otherwise.
    #[inline(always)]
    pub fn local_name(&self, id: usize) -> &str {
        match self.nodes.get(id) {
            Some(node) => &node.local_name,
            None => &self.declarations[self.namespace_node(id).declaration].prefix,
        }
    }

    /// The prefix of an element or attribute name in the source.
    pub fn prefix(&self, id: usize) -> Option<&str> {
        self.nodes.get(id).and_then(|node| node.prefix.as_deref())
    }

    /// The namespace of an element or attribute name.
    #[inline(always)]
    pub fn namespace_uri(&self, id: usize) -> Option<&str> {
        let declaration = self.nodes.get(id)?.namespace?;
        Some(&self.declarations[declaration].uri)
    }

    /// The position of the node in document order: an element's namespace
    /// nodes come after it and before its attributes.
    pub fn order(&self, id: usize) -> (usize, usize) {
        if id < self.nodes.len() {
            (id, 0)
        } else {
            let node = self.namespace_node(id);
            (node.element, 1 + node.ordinal)
        }
    }

    #[inline(always)]
    pub fn children(&self, id: usize) -> &[usize] {
        self.nodes.get(id).map_or(&[], |node| &node.children)
    }

    pub fn attributes(&self, id: usize) -> Range<usize> {
        self.nodes
            .get(id)
            .map_or(0..0, |node| node.attributes.clone())
    }

    /// The descendants of the node in document order.
    pub fn descendants(&self, id: usize) -> impl DoubleEndedIterator<Item = usize> + '_ {
        let end = self.nodes.get(id).map_or(id, |node| node.end);
        (id + 1..end).filter(|&index| self.nodes[index].kind != Kind::Attribute)
    }

    /// The children of the parent of the node (when it is in the tree and not
    /// the root), and the node's position among them.
    pub fn siblings(&self, id: usize) -> (&[usize], usize) {
        match self.nodes.get(id) {
            Some(node) if node.kind != Kind::Attribute => match node.parent {
                Some(parent) => (&self.nodes[parent].children, node.sibling_index),
                None => (&[], 0),
            },
            _ => (&[], 0),
        }
    }

    /// The first index the `following` axis of the node may reach: past its
    /// subtree (an attribute's or a namespace node's element's children come
    /// after it).
    #[inline(always)]
    pub fn following_start(&self, id: usize) -> usize {
        match self.nodes.get(id) {
            Some(node) => node.end,
            None => self.namespace_node(id).element + 1,
        }
    }

    /// The `preceding` axis of the node holds the tree nodes whose subtree
    /// ends at or before this index (which leaves out the ancestors).
    #[inline(always)]
    pub fn preceding_end(&self, id: usize) -> usize {
        if id < self.nodes.len() {
            id
        } else {
            self.namespace_node(id).element
        }
    }

    /// One past the last node of the node's subtree (for a namespace node,
    /// one past itself: it has no descendants).
    #[inline(always)]
    pub fn end(&self, id: usize) -> usize {
        self.nodes.get(id).map_or(id + 1, |node| node.end)
    }

    /// The XPath string-value of the node: the text it contains for the root
    /// and elements, its value (a namespace node's URI) for the other nodes.
    /// Text in several text nodes is joined once and kept.
    #[inline(always)]
    pub fn string_value(&self, id: usize) -> &str {
        let Some(node) = self.nodes.get(id) else {
            return &self.declarations[self.namespace_node(id).declaration].uri;
        };
        if !matches!(node.kind, Kind::Root | Kind::Element) {
            return &node.value;
        }
        let texts = &self.texts[self.texts_before[id]..self.texts_before[node.end]];
        match texts {
            [] => "",
            [text] => &self.nodes[*text].value,
            _ => self.joined_text[id].get_or_init(|| {
                texts
                    .iter()
                    .map(|&text| self.nodes[text].value.as_str())
                    .collect::<String>()
                    .into_boxed_str()
            }),
        }
    }

    /// An id for the node's string-value: two nodes have the same id when
    /// their string-values are equal. It is worked out when first asked for.
    #[inline(always)]
    pub fn value_id(&self, id: usize) -> u32 {
        if let Some(cell) = self.value_ids.of_node.get(id) {
            let value_id = cell.get();
            if value_id != ValueIds::UNSET {
                return value_id;
            }
        }
        self.new_value_id(id)
    }

    fn new_value_id(&self, id: usize) -> u32 {
        if id >= self.nodes.len()
            && let Some(&value_id) = self.value_ids.of_namespace_node.borrow().get(&id)
        {
            return value_id;
        }
        let value = self.string_value(id);
        let mut hasher = FxHasher::default();
        value.hash(&mut hasher);
        let mut by_hash = self.value_ids.by_hash.borrow_mut();
        let ids = by_hash.entry(hasher.finish()).or_default();
        let value_id = match ids
            .iter()
            .find(|&&(_, node)| self.string_value(node) == value)
        {
            Some(&(value_id, _)) => value_id,
            None => {
                // One id per distinct string-value of a node reached, far
                // below UNSET.
                let value_id = self.value_ids.count.get();
                self.value_ids.count.set(value_id + 1);
                ids.push((value_id, id));
                value_id
            }
        };
        match self.value_ids.of_node.get(id) {
            Some(cell) => cell.set(value_id),
            None => {
                self.value_ids
                    .of_namespace_node
                    .borrow_mut()
                    .insert(id, value_id);
            }
        }
        value_id
    }

    /// The name id of the nodes without a name.
    pub const NO_NAME: u32 = u32::MAX;

    /// The id of the expanded name, as [`Document::name_ids`] holds it for the
    /// elements and attributes with that name; one no node has when none has
    /// the name.
    pub fn name_id(&self, namespace: Option<&str>, local_name: &str) -> u32 {
        self.names
            .get(&name_key(namespace, local_name))
            .copied()
            .unwrap_or(Self::NO_NAME - 1)
    }

    /// The expanded-name id of each node other than the namespace nodes, by
    /// index.
    #[inline(always)]
    pub fn name_ids(&self) -> &[u32] {
        &self.name_ids
    }

    /// The kind of each node other than the namespace nodes, by index.
    #[inline(always)]
    pub fn kinds(&self) -> &[Kind] {
        &self.kinds
    }

    /// The end of each tree node's subtree, by index ([`Document::end`]);
    /// `usize::MAX` for an attribute.
    #[inline(always)]
    pub fn tree_ends(&self) -> &[usize] {
        &self.tree_ends
    }

    /// The ids of the namespace nodes of `element`, made when first asked
    /// for: one for each prefix in scope (the `xml` prefix included) and one
    /// for the default namespace when one is in scope. Their order is the
    /// document order of the declarations in effect: the predeclared `xml`
    /// prefix first, then the outer elements' declarations before the inner
    /// ones', each element's in source order.
    pub fn namespaces(&self, element: usize) -> Range<usize> {
        if self.kind(element) != Kind::Element {
            return 0..0;
        }
        let mut cache = self.namespace_nodes.borrow_mut();
        let cache = &mut *cache;
        if let Some(ids) = cache.of_element.get(&element) {
            return ids.clone();
        }
        let scope = self.nodes[element].scope;
        let declarations = cache
            .in_scope
            .entry(scope)
            .or_insert_with(|| self.declarations_in_scope(scope));
        let first = self.nodes.len() + cache.nodes.len();
        cache.nodes.extend(
            declarations
                .iter()
                .enumerate()
                .map(|(ordinal, &declaration)| NamespaceNode {
                    element,
                    declaration,
                    ordinal,
                }),
        );
        let ids = first..self.nodes.len() + cache.nodes.len();
        cache.of_element.insert(element, ids.clone());
        ids
    }

    /// The declarations in effect below the declaring element `scope`: the
    /// innermost for each prefix, the default namespace's only when its URI is
    /// not empty, in document order.
    fn declarations_in_scope(&self, scope: Option<usize>) -> Vec<usize> {
        let mut prefixes = FxHashSet::default();
        let mut in_scope = Vec::new();
        let mut at = scope;
        while let Some(element) = at {
            for declaration in self.nodes[element].declarations.clone() {
                if prefixes.insert(self.declarations[declaration].prefix.as_str()) {
                    in_scope.push(declaration);
                }
            }
            at = self.nodes[element]
                .parent
                .and_then(|parent| self.nodes[parent].scope);
        }
        if prefixes.insert("xml") {
            in_scope.push(0);
        }
        in_scope.retain(|&declaration| !self.declarations[declaration].uri.is_empty());
        in_scope.sort_unstable();
        in_scope
    }

    /// The number of namespace nodes made so far.
    #[cfg(test)]
    pub fn namespace_nodes_made(&self) -> usize {
        self.namespace_nodes.borrow().nodes.len()
    }

    /// The number of string-values joined from several text nodes so far.
    #[cfg(test)]
    pub fn texts_joined(&self) -> usize {
        self.joined_text
            .iter()
            .filter(|text| text.get().is_some())
            .count()
    }
}

/// The key of an expanded name in [`Document::names`]: a namespace URI is
/// never empty and no name or URI holds U+0001, which XML does not allow.
fn name_key(namespace: Option<&str>, local_name: &str) -> String {
    format!("{}\u{1}{local_name}", namespace.unwrap_or(""))
}

/// The text of a part of the document; the parts quick-xml returns end at
/// ASCII delimiters, so they are whole UTF-8.
fn utf8(bytes: &[u8]) -> Option<&str> {
    std::str::from_utf8(bytes).ok()
}

/// Where in the document the parser is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Nothing read yet: only here may the XML declaration be.
    Start,
    /// In the prolog, after something.
    Prolog,
    /// Inside the root element.
    Root,
    /// After the root element.
    Epilog,
}

/// A pseudo-attribute of the XML declaration: its name, whether it is
/// required, and which values it takes.
type PseudoAttribute = (&'static str, bool, fn(&str) -> bool);

/// Builds the [`Document`] from the parts of the text in order.
struct Builder {
    nodes: Vec<Node>,
    declarations: Vec<Declaration>,
    /// For each prefix, the declarations in scope that bind it, innermost
    /// last.
    bindings: FxHashMap<String, Vec<usize>>,
    /// The open elements, innermost last, and whether white space is preserved
    /// in each.
    open: Vec<(usize, bool)>,
    /// Character data not yet made a text node.
    text: String,
    place: Place,
}

impl Builder {
    fn new() -> Self {
        let mut root = Node::new(Kind::Root);
        root.end = 1;
        let xml = Declaration {
            prefix: "xml".into(),
            uri: XML_NAMESPACE.into(),
        };
        let mut bindings = FxHashMap::default();
        bindings.insert("xml".to_string(), vec![0]);
        Builder {
            nodes: vec![root],
            declarations: vec![xml],
            bindings,
            open: Vec::new(),
            text: String::new(),
            place: Place::Start,
        }
    }

    /// The element (or the root) new nodes go into.
    fn parent(&self) -> usize {
        self.open.last().map_or(0, |&(element, _)| element)
    }

    /// Adds `node` as the last child of the current parent.
    fn add_child(&mut self, mut node: Node) -> usize {
        let index = self.nodes.len();
        let parent = self.parent();
        node.parent = Some(parent);
        node.sibling_index = self.nodes[parent].children.len();
        node.end = index + 1;
        self.nodes[parent].children.push(index);
        self.nodes.push(node);
        index
    }

    /// Makes the character data read so far a text node, unless it is only
    /// white space and white space is not preserved.
    fn flush_text(&mut self) {
        if self.text.is_empty() {
            return;
        }
        let preserve = self.open.last().is_some_and(|&(_, preserve)| preserve);
        if preserve || !self.text.chars().all(is_xml_space) {
            let mut node = Node::new(Kind::Text);
            node.value = std::mem::take(&mut self.text);
            self.add_child(node);
        }
        self.text.clear();
    }

    /// Before a comment or processing instruction: in the root element it ends
    /// the text before it; in the prolog it ends the start.
    fn before_misc(&mut self) {
        match self.place {
            Place::Start => self.place = Place::Prolog,
            Place::Root => self.flush_text(),
            Place::Prolog | Place::Epilog => {}
        }
    }

    /// Character data. Outside the root element only white space may be
    /// there; inside it `]]>` may not (XML 1.0 §2.4).
    fn text(&mut self, text: &str) -> Option<()> {
        if self.place == Place::Root {
            if text.contains("]]>") {
                return None;
            }
            self.text.push_str(text);
        } else {
            if !text.chars().all(is_xml_space) {
                return None;
            }
            if self.place == Place::Start {
                self.place = Place::Prolog;
            }
        }
        Some(())
    }

    /// An entity or character reference in content (`&name;`).
    fn reference(&mut self, name: &str) -> Option<()> {
        if self.place != Place::Root {
            return None;
        }
        self.text.push(reference(name)?);
        Some(())
    }

    /// A CDATA section: its text is character data.
    fn cdata(&mut self, text: &str) -> Option<()> {
        if self.place != Place::Root {
            return None;
        }
        self.text.push_str(text);
        Some(())
    }

    /// A comment: it may not hold `--` or end with `-` (XML 1.0 §2.5).
    fn comment(&mut self, text: &str) -> Option<()> {
        if text.contains("--") || text.ends_with('-') {
            return None;
        }
        self.before_misc();
        let mut node = Node::new(Kind::Comment);
        node.value = text.into();
        self.add_child(node);
        Some(())
    }

    /// A processing instruction (`<?target data?>`): the target is a name
    /// other than `xml` in any case (XML 1.0 §2.6).
    fn processing_instruction(&mut self, content: &str) -> Option<()> {
        let target_end = content.find(is_xml_space).unwrap_or(content.len());
        let target = &content[..target_end];
        if !is_name(target) || target.eq_ignore_ascii_case("xml") {
            return None;
        }
        self.before_misc();
        let mut node = Node::new(Kind::ProcessingInstruction);
        node.local_name = target.into();
        node.value = content[target_end..]
            .trim_start_matches(is_xml_space)
            .into();
        self.add_child(node);
        Some(())
    }

    /// The XML declaration (`<?xml version="1.0"?>`): only at the very start
    /// of the text, with the pseudo-attributes of XML 1.0 §2.8 in order.
    fn xml_declaration(&mut self, content: &str) -> Option<()> {
        if self.place != Place::Start {
            return None;
        }
        let mut rest = content.strip_prefix("xml")?;
        let checks: [PseudoAttribute; 3] = [
            ("version", true, |version| {
                version.strip_prefix("1.").is_some_and(|digits| {
                    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
                })
            }),
            ("encoding", false, |name| {
                let mut bytes = name.bytes();
                bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
                    && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            }),
            ("standalone", false, |value| matches!(value, "yes" | "no")),
        ];
        for (name, required, valid) in checks {
            match pseudo_attribute(rest, name) {
                Some((value, after)) if valid(value) => rest = after,
                Some(_) => return None,
                None if required => return None,
                None => {}
            }
        }
        if !rest.chars().all(is_xml_space) {
            return None;
        }
        self.place = Place::Prolog;
        Some(())
    }

    /// A document type declaration: read in the prolog after its start (an XML
    /// declaration or other prolog content), with a `SYSTEM` identifier at
    /// most and an internal subset without `]`. It is skipped: no declaration
    /// in it is read, so it defines no entity.
    fn document_type(&mut self, markup: &str) -> Option<()> {
        if self.place != Place::Prolog {
            return None;
        }
        let rest = markup.strip_prefix("<!DOCTYPE")?.strip_suffix('>')?;
        let name = rest.trim_start_matches(is_xml_space);
        if name.len() == rest.len() {
            return None;
        }
        let name_end = name
            .find(|c: char| c != ':' && !is_name_char(c))
            .unwrap_or(name.len());
        if !is_name(&name[..name_end]) {
            return None;
        }
        let mut rest = &name[name_end..];
        let space = rest.trim_start_matches(is_xml_space);
        if space.len() < rest.len()
            && let Some(system) = space.strip_prefix("SYSTEM")
        {
            let literal = system.trim_start_matches(is_xml_space);
            if literal.len() < system.len()
                && let Some(quote) = literal.chars().next().filter(|&q| q == '"' || q == '\'')
                && let Some(close) = literal[1..].find(['&', '<', quote])
                && close > 0
                && literal[1 + close..].starts_with(quote)
            {
                rest = &literal[close + 2..];
            }
        }
        rest = rest.trim_start_matches(is_xml_space);
        if let Some(subset) = rest.strip_prefix('[') {
            rest = subset[subset.find(']')? + 1..].trim_start_matches(is_xml_space);
        }
        rest.is_empty().then_some(())
    }

    /// The prefix's binding in scope.
    fn bound(&self, prefix: &str) -> Option<usize> {
        self.bindings.get(prefix)?.last().copied()
    }

    /// A start tag (`<name attributes>`, or `<name attributes/>` when
    /// `empty`).
    fn start_tag(&mut self, tag: &str, empty: bool) -> Option<()> {
        match self.place {
            Place::Epilog => return None,
            Place::Root => self.flush_text(),
            Place::Start | Place::Prolog => self.place = Place::Root,
        }
        let (name, attributes) = parse_tag(tag)?;
        // XML 1.0 WFC Unique Att Spec.
        let mut names = FxHashSet::default();
        if !attributes.iter().all(|(name, _)| names.insert(*name)) {
            return None;
        }
        // The namespace declarations come into scope for the element's own
        // name and attributes as well as its content.
        let first_declaration = self.declarations.len();
        for (attribute, value) in &attributes {
            let prefix = if *attribute == "xmlns" {
                ""
            } else if let Some(prefix) = attribute.strip_prefix("xmlns:") {
                prefix
            } else {
                continue;
            };
            if !is_declaration_allowed(prefix, value) {
                return None;
            }
            let index = self.declarations.len();
            self.declarations.push(Declaration {
                prefix: prefix.into(),
                uri: value.clone(),
            });
            self.bindings.entry(prefix.into()).or_default().push(index);
        }
        let declarations = first_declaration..self.declarations.len();
        let (prefix, local_name) = split_qname(name);
        let namespace = match prefix {
            // `xmlns` is bound to no namespace, so it is no prefix.
            Some(prefix) => Some(self.bound(prefix)?),
            None => self
                .bound("")
                .filter(|&declaration| !self.declarations[declaration].uri.is_empty()),
        };
        // The attributes' names; an unprefixed attribute is in no namespace.
        let mut resolved = Vec::with_capacity(attributes.len());
        for (attribute, value) in attributes {
            if attribute == "xmlns" || attribute.starts_with("xmlns:") {
                continue;
            }
            let (prefix, local_name) = split_qname(attribute);
            let namespace = match prefix {
                Some(prefix) => Some(self.bound(prefix)?),
                None => None,
            };
            resolved.push((prefix, local_name, namespace, value));
        }
        // Namespaces in XML 1.0 §6.3: no two attributes with the same
        // namespace and local name.
        let mut expanded = FxHashSet::default();
        for (_, local_name, namespace, _) in &resolved {
            if let Some(namespace) = namespace
                && !expanded.insert((self.declarations[*namespace].uri.as_str(), *local_name))
            {
                return None;
            }
        }
        let parent = self.parent();
        let inherited = self.open.last().is_some_and(|&(_, preserve)| preserve);
        let mut element = Node::new(Kind::Element);
        element.namespace = namespace;
        element.prefix = prefix.map(Into::into);
        element.local_name = local_name.into();
        element.scope = if declarations.is_empty() {
            self.nodes[parent].scope
        } else {
            Some(self.nodes.len())
        };
        element.declarations = declarations;
        let index = self.add_child(element);
        let mut preserve = inherited;
        for (prefix, local_name, namespace, value) in resolved {
            if namespace.is_some_and(|namespace| self.declarations[namespace].uri == XML_NAMESPACE)
                && local_name == "space"
            {
                preserve = match value.as_str() {
                    "preserve" => true,
                    "default" => false,
                    _ => preserve,
                };
            }
            let mut attribute = Node::new(Kind::Attribute);
            attribute.parent = Some(index);
            attribute.end = self.nodes.len() + 1;
            attribute.namespace = namespace;
            attribute.prefix = prefix.map(Into::into);
            attribute.local_name = local_name.into();
            attribute.value = value;
            self.nodes.push(attribute);
        }
        self.nodes[index].attributes = index + 1..self.nodes.len();
        if empty {
            self.close(index);
        } else {
            self.open.push((index, preserve));
        }
        Some(())
    }

    /// An end tag, which quick-xml has matched with the open element.
    fn end_tag(&mut self) -> Option<()> {
        self.flush_text();
        let (element, _) = self.open.pop()?;
        self.close(element);
        Some(())
    }

    /// Ends `element`'s subtree and its declarations' scope.
    fn close(&mut self, element: usize) {
        self.nodes[element].end = self.nodes.len();
        for declaration in self.nodes[element].declarations.clone() {
            if let Some(bindings) = self
                .bindings
                .get_mut(&self.declarations[declaration].prefix)
            {
                bindings.pop();
            }
        }
        if self.open.is_empty() {
            self.place = Place::Epilog;
        }
    }

    fn finish(mut self) -> Option<Document> {
        if self.place != Place::Epilog {
            return None;
        }
        self.nodes[0].end = self.nodes.len();
        let mut texts = Vec::new();
        let mut texts_before = Vec::with_capacity(self.nodes.len() + 1);
        for (index, node) in self.nodes.iter().enumerate() {
            texts_before.push(texts.len());
            if node.kind == Kind::Text {
                texts.push(index);
            }
        }
        texts_before.push(texts.len());
        let joined_text = std::iter::repeat_with(OnceCell::new)
            .take(self.nodes.len())
            .collect();
        let kinds = self.nodes.iter().map(|node| node.kind).collect();
        let tree_ends = self
            .nodes
            .iter()
            .map(|node| match node.kind {
                Kind::Attribute => usize::MAX,
                _ => node.end,
            })
            .collect();
        let mut names = FxHashMap::default();
        let name_ids = self
            .nodes
            .iter()
            .map(|node| match node.kind {
                Kind::Element | Kind::Attribute => {
                    let namespace = node
                        .namespace
                        .map(|declaration| self.declarations[declaration].uri.as_str());
                    let next = u32::try_from(names.len()).unwrap_or(Document::NO_NAME - 2);
                    *names
                        .entry(name_key(namespace, &node.local_name))
                        .or_insert(next)
                }
                _ => Document::NO_NAME,
            })
            .collect();
        let value_ids = ValueIds {
            of_node: std::iter::repeat_with(|| Cell::new(ValueIds::UNSET))
                .take(self.nodes.len())
                .collect(),
            ..ValueIds::default()
        };
        Some(Document {
            nodes: self.nodes,
            declarations: self.declarations,
            texts,
            texts_before,
            joined_text,
            namespace_nodes: RefCell::default(),
            kinds,
            tree_ends,
            name_ids,
            names,
            value_ids,
        })
    }
}

/// Whether `xmlns:prefix="uri"` (`xmlns="uri"` for an empty prefix) may be
/// declared (Namespaces in XML 1.0 §3): `xmlns` is never declared, `xml` only
/// for its own namespace, which no other prefix takes, nor the `xmlns`
/// namespace; only the default namespace may be undeclared (an empty URI).
fn is_declaration_allowed(prefix: &str, uri: &str) -> bool {
    match prefix {
        "xmlns" => false,
        "xml" => uri == XML_NAMESPACE,
        _ => {
            uri != XML_NAMESPACE && uri != XMLNS_NAMESPACE && (prefix.is_empty() || !uri.is_empty())
        }
    }
}

/// The name and attributes of a tag's content (between `<` and `>` or `/>`):
/// `QName (S QName S? = S? AttValue)* S?`, attribute values with their
/// references replaced.
fn parse_tag(tag: &str) -> Option<(&str, Vec<(&str, String)>)> {
    let name_end = tag.find(is_xml_space).unwrap_or(tag.len());
    let name = &tag[..name_end];
    if !is_qname(name) {
        return None;
    }
    let mut rest = &tag[name_end..];
    let mut attributes = Vec::new();
    loop {
        let attribute = rest.trim_start_matches(is_xml_space);
        if attribute.is_empty() {
            return Some((name, attributes));
        }
        // Attributes are separated by white space.
        if attribute.len() == rest.len() {
            return None;
        }
        let name_end = attribute
            .find(|c: char| c == '=' || is_xml_space(c))
            .unwrap_or(attribute.len());
        let attribute_name = &attribute[..name_end];
        if !is_qname(attribute_name) {
            return None;
        }
        let value = attribute[name_end..]
            .trim_start_matches(is_xml_space)
            .strip_prefix('=')?
            .trim_start_matches(is_xml_space);
        let quote = value.chars().next().filter(|&q| q == '"' || q == '\'')?;
        let close = 1 + value[1..].find(quote)?;
        attributes.push((attribute_name, attribute_value(&value[1..close])?));
        rest = &value[close + 1..];
    }
}

/// An attribute value as written between its quotes, with its references
/// replaced; `None` when it holds `<` or an `&` that starts no reference.
fn attribute_value(written: &str) -> Option<String> {
    let mut value = String::with_capacity(written.len());
    let mut rest = written;
    while let Some(at) = rest.find(['&', '<']) {
        if rest.as_bytes()[at] == b'<' {
            return None;
        }
        value.push_str(&rest[..at]);
        let end = at + rest[at..].find(';')?;
        value.push(reference(&rest[at + 1..end])?);
        rest = &rest[end + 1..];
    }
    value.push_str(rest);
    Some(value)
}

/// The character a reference (`&name;`, without `&` and `;`) stands for: a
/// predefined entity, or a character reference (`#N`, `#xH`) to a character
/// XML allows. No other entity is defined.
fn reference(name: &str) -> Option<char> {
    let c = if let Some(hex) = name.strip_prefix("#x") {
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        char::from_u32(u32::from_str_radix(hex, 16).ok()?)?
    } else if let Some(decimal) = name.strip_prefix('#') {
        if decimal.is_empty() || !decimal.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        char::from_u32(decimal.parse().ok()?)?
    } else {
        match name {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "apos" => '\'',
            "quot" => '"',
            _ => return None,
        }
    };
    is_xml_char(c).then_some(c)
}

/// `S name S? = S? quoted-value` at the start of `text`: the value and the
/// text after it.
fn pseudo_attribute<'t>(text: &'t str, name: &str) -> Option<(&'t str, &'t str)> {
    let after_space = text.trim_start_matches(is_xml_space);
    if after_space.len() == text.len() {
        return None;
    }
    let value = after_space
        .strip_prefix(name)?
        .trim_start_matches(is_xml_space)
        .strip_prefix('=')?
        .trim_start_matches(is_xml_space);
    let quote = value.chars().next().filter(|&q| q == '"' || q == '\'')?;
    let close = 1 + value[1..].find(quote)?;
    Some((&value[1..close], &value[close + 1..]))
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
