//! HTML parser for the MiniBrowser engine using html5ever.
//!
//! Parses an HTML string into our `dom::Document` tree via a custom `TreeSink`.

use std::borrow::Cow;
use std::collections::HashMap;

use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{parse_document, Attribute, ExpandedName, QualName};

use crate::dom::{Document, Node};

/// Parse an HTML string and return a `Document`.
pub fn parse_html(html: &str) -> Document {
    let sink = DomSink::new();
    let parser = parse_document(sink, Default::default());
    let sink = parser.one(html);
    sink.into_document()
}

/// Count DOM nodes in a tree (utility).
pub fn count_nodes(node: &Node) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}

// ---------------------------------------------------------------------------
// Handle-based arena that html5ever populates
// ---------------------------------------------------------------------------

/// Lightweight handle for referencing nodes during parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Handle(usize);

/// Internal node representation during parsing.
#[derive(Debug, Clone)]
enum SinkNode {
    Document,
    Element {
        qual_name: QualName,
        attributes: HashMap<String, String>,
    },
    Text(String),
    /// Comments / PIs / doctype — dropped in final tree.
    Ignored,
}

/// A slot in the flat arena.
#[derive(Debug, Clone)]
struct SinkEntry {
    node: SinkNode,
    children: Vec<Handle>,
}

/// The custom `TreeSink`.
///
/// html5ever's `TreeSink` trait takes `&self` for mutation methods, so we use
/// `UnsafeCell` for interior mutability. This is safe because html5ever drives
/// the sink single-threaded during `parse_document().one()`.
struct DomSink {
    /// Flat node arena.  Mutated through `&self` — see safety note above.
    arena: std::cell::UnsafeCell<Vec<SinkEntry>>,
    document_handle: Handle,
}

// DomSink is only used single-threaded inside parse_document, but the trait
// bounds may require Send in some configurations.  We assert it is safe.
unsafe impl Send for DomSink {}

impl DomSink {
    fn new() -> Self {
        let document = SinkEntry {
            node: SinkNode::Document,
            children: Vec::new(),
        };
        DomSink {
            arena: std::cell::UnsafeCell::new(vec![document]),
            document_handle: Handle(0),
        }
    }

    /// Get a mutable reference to the arena.
    ///
    /// # Safety
    /// Only call during single-threaded parsing driven by html5ever.
    #[inline]
    fn arena_mut(&self) -> &mut Vec<SinkEntry> {
        unsafe { &mut *self.arena.get() }
    }

    /// Get a shared reference to the arena.
    #[inline]
    fn arena_ref(&self) -> &Vec<SinkEntry> {
        unsafe { &*self.arena.get() }
    }

    /// Allocate a new handle.
    fn alloc(&self, entry: SinkEntry) -> Handle {
        let arena = self.arena_mut();
        let id = arena.len();
        arena.push(entry);
        Handle(id)
    }

    /// Convert the flat arena into our owned `Document` tree.
    fn into_document(self) -> Document {
        let arena = self.arena.into_inner();
        let root = Self::build_node(&arena, self.document_handle);
        Document { root }
    }

    /// Recursively build an owned `Node` from the arena.
    fn build_node(arena: &[SinkEntry], handle: Handle) -> Node {
        let entry = &arena[handle.0];
        let mut node = match &entry.node {
            SinkNode::Document => Node::new_document(),
            SinkNode::Element { qual_name, attributes } => {
                Node::new_element(&qual_name.local, attributes.clone())
            }
            SinkNode::Text(text) => Node::new_text(text),
            SinkNode::Ignored => return Node::new_text(""),
        };
        for &child_handle in &entry.children {
            if matches!(arena[child_handle.0].node, SinkNode::Ignored) {
                continue;
            }
            let child = Self::build_node(arena, child_handle);
            node.children.push(child);
        }
        node
    }
}

// ---------------------------------------------------------------------------
// TreeSink implementation
// ---------------------------------------------------------------------------

impl TreeSink for DomSink {
    type Handle = Handle;
    type Output = Self;
    type ElemName<'a> = ExpandedName<'a>;

    fn finish(self) -> Self::Output {
        self
    }

    fn parse_error(&self, msg: Cow<'static, str>) {
        log::trace!("html5ever parse error: {}", msg);
    }

    fn get_document(&self) -> Handle {
        self.document_handle
    }

    fn elem_name<'a>(&'a self, target: &'a Handle) -> ExpandedName<'a> {
        let arena = self.arena_ref();
        match &arena[target.0].node {
            SinkNode::Element { qual_name, .. } => qual_name.expanded(),
            _ => panic!("elem_name called on non-element"),
        }
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        _flags: ElementFlags,
    ) -> Handle {
        let attributes: HashMap<String, String> = attrs
            .into_iter()
            .map(|a| (a.name.local.to_string(), a.value.to_string()))
            .collect();
        self.alloc(SinkEntry {
            node: SinkNode::Element {
                qual_name: name,
                attributes,
            },
            children: Vec::new(),
        })
    }

    fn create_comment(&self, _text: StrTendril) -> Handle {
        self.alloc(SinkEntry {
            node: SinkNode::Ignored,
            children: Vec::new(),
        })
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Handle {
        self.alloc(SinkEntry {
            node: SinkNode::Ignored,
            children: Vec::new(),
        })
    }

    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        let arena = self.arena_mut();
        match child {
            NodeOrText::AppendNode(handle) => {
                arena[parent.0].children.push(handle);
            }
            NodeOrText::AppendText(text) => {
                let text_str = text.to_string();
                // Merge with trailing text node if present.
                if let Some(&last_h) = arena[parent.0].children.last() {
                    if let SinkNode::Text(ref mut existing) = arena[last_h.0].node {
                        existing.push_str(&text_str);
                        return;
                    }
                }
                let id = arena.len();
                arena.push(SinkEntry {
                    node: SinkNode::Text(text_str),
                    children: Vec::new(),
                });
                arena[parent.0].children.push(Handle(id));
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        _prev_element: &Handle,
        child: NodeOrText<Handle>,
    ) {
        self.append(element, child);
    }

    fn append_doctype_to_document(
        &self,
        _name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        // Ignored.
    }

    fn get_template_contents(&self, target: &Handle) -> Handle {
        *target
    }

    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        x.0 == y.0
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &Handle, new_node: NodeOrText<Handle>) {
        let arena = self.arena_mut();
        let sibling_id = sibling.0;
        for i in 0..arena.len() {
            if let Some(pos) = arena[i].children.iter().position(|h| h.0 == sibling_id) {
                match new_node {
                    NodeOrText::AppendNode(handle) => {
                        arena[i].children.insert(pos, handle);
                    }
                    NodeOrText::AppendText(text) => {
                        let id = arena.len();
                        arena.push(SinkEntry {
                            node: SinkNode::Text(text.to_string()),
                            children: Vec::new(),
                        });
                        // After push, re-find position since arena may have reallocated
                        // but index i is still valid.
                        arena[i].children.insert(pos, Handle(id));
                    }
                }
                return;
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        let arena = self.arena_mut();
        if let SinkNode::Element { ref mut attributes, .. } = arena[target.0].node {
            for attr in attrs {
                let key = attr.name.local.to_string();
                attributes.entry(key).or_insert_with(|| attr.value.to_string());
            }
        }
    }

    fn remove_from_parent(&self, target: &Handle) {
        let arena = self.arena_mut();
        let target_id = target.0;
        for i in 0..arena.len() {
            arena[i].children.retain(|h| h.0 != target_id);
        }
    }

    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        let arena = self.arena_mut();
        let children: Vec<Handle> = arena[node.0].children.drain(..).collect();
        arena[new_parent.0].children.extend(children);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_html() {
        let doc = parse_html("<html><body><div id=\"main\"><p>Hello</p></div></body></html>");
        let main = doc.get_element_by_id("main");
        assert!(main.is_some(), "should find #main");
        assert_eq!(main.unwrap().element_data().unwrap().tag_name, "div");
    }

    #[test]
    fn test_parse_text_content() {
        let doc = parse_html("<p>Hello <b>World</b></p>");
        let p = doc.query_selector("p");
        assert!(p.is_some());
        let text = p.unwrap().inner_text();
        assert!(text.contains("Hello"), "text should contain Hello");
        assert!(text.contains("World"), "text should contain World");
    }

    #[test]
    fn test_parse_attributes() {
        let doc = parse_html(r#"<a href="https://example.com" class="link">Click</a>"#);
        let a = doc.query_selector("a.link");
        assert!(a.is_some());
        let data = a.unwrap().element_data().unwrap();
        assert_eq!(data.attributes.get("href").unwrap(), "https://example.com");
    }

    #[test]
    fn test_node_count() {
        let doc = parse_html("<div><span>A</span><span>B</span></div>");
        assert!(count_nodes(&doc.root) >= 5);
    }

    #[test]
    fn test_parse_nested_structure() {
        let html = r#"
            <html>
            <head><title>Test</title></head>
            <body>
                <div id="container">
                    <h1>Title</h1>
                    <p class="intro">Paragraph</p>
                </div>
            </body>
            </html>
        "#;
        let doc = parse_html(html);
        assert!(doc.get_element_by_id("container").is_some());
        assert!(doc.query_selector("h1").is_some());
        assert!(doc.query_selector("p.intro").is_some());
    }
}
