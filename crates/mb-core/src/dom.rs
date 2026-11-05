//! Simple DOM tree representation for the MiniBrowser engine.
//!
//! Provides `NodeType`, `Node`, `ElementData`, and `Document` types
//! with basic tree construction and traversal.

use std::collections::HashMap;
use std::fmt;

/// The kind of DOM node.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeType {
    Document,
    Element(ElementData),
    Text(String),
}

/// Data specific to element nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementData {
    pub tag_name: String,
    pub attributes: HashMap<String, String>,
}

impl ElementData {
    pub fn new(tag_name: String, attributes: HashMap<String, String>) -> Self {
        Self { tag_name, attributes }
    }

    /// Return the `id` attribute if present.
    pub fn id(&self) -> Option<&str> {
        self.attributes.get("id").map(|s| s.as_str())
    }

    /// Return the list of classes from the `class` attribute.
    pub fn classes(&self) -> Vec<&str> {
        self.attributes
            .get("class")
            .map(|c| c.split_whitespace().collect())
            .unwrap_or_default()
    }

    /// Return the inline `style` attribute if present.
    pub fn style(&self) -> Option<&str> {
        self.attributes.get("style").map(|s| s.as_str())
    }
}

/// A node in the DOM tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub node_type: NodeType,
    pub children: Vec<Node>,
    /// Stable identity for scripts (the live DOM): unique among nodes made in
    /// this process. A clone keeps it; `Node::renumber` gives fresh ones.
    pub uid: u32,
}

static NEXT_UID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

fn next_uid() -> u32 {
    NEXT_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl Node {
    /// Create a new element node.
    pub fn new_element(tag_name: &str, attributes: HashMap<String, String>) -> Self {
        Node {
            node_type: NodeType::Element(ElementData::new(tag_name.to_lowercase(), attributes)),
            children: Vec::new(),
            uid: next_uid(),
        }
    }

    /// Create a new text node.
    pub fn new_text(text: &str) -> Self {
        Node {
            node_type: NodeType::Text(text.to_string()),
            children: Vec::new(),
            uid: next_uid(),
        }
    }

    /// Create a new document node.
    pub fn new_document() -> Self {
        Node {
            node_type: NodeType::Document,
            children: Vec::new(),
            uid: next_uid(),
        }
    }

    /// Fresh uids for this node and its subtree (a copy made by cloneNode).
    pub fn renumber(&mut self) {
        self.uid = next_uid();
        for c in &mut self.children {
            c.renumber();
        }
    }

    /// Indices from this node down to the node with `uid`.
    pub fn path_to(&self, uid: u32) -> Option<Vec<usize>> {
        if self.uid == uid {
            return Some(Vec::new());
        }
        for (i, c) in self.children.iter().enumerate() {
            if let Some(mut p) = c.path_to(uid) {
                p.insert(0, i);
                return Some(p);
            }
        }
        None
    }

    pub fn at_path(&self, path: &[usize]) -> Option<&Node> {
        path.iter().try_fold(self, |n, &i| n.children.get(i))
    }

    pub fn at_path_mut(&mut self, path: &[usize]) -> Option<&mut Node> {
        path.iter().try_fold(self, |n, &i| n.children.get_mut(i))
    }

    /// Append a child node, returning a mutable reference to it.
    pub fn append_child(&mut self, child: Node) -> &mut Node {
        self.children.push(child);
        self.children.last_mut().unwrap()
    }

    /// Get element data if this is an element node.
    pub fn element_data(&self) -> Option<&ElementData> {
        match &self.node_type {
            NodeType::Element(data) => Some(data),
            _ => None,
        }
    }

    /// Get text content if this is a text node.
    pub fn text_content(&self) -> Option<&str> {
        match &self.node_type {
            NodeType::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Collect all text content from this node and descendants.
    pub fn inner_text(&self) -> String {
        let mut result = String::new();
        self.collect_text(&mut result);
        result
    }

    fn collect_text(&self, out: &mut String) {
        match &self.node_type {
            NodeType::Text(s) => out.push_str(s),
            _ => {
                for child in &self.children {
                    child.collect_text(out);
                }
            }
        }
    }

    /// Find the first element with the given `id` (depth-first).
    pub fn get_element_by_id(&self, id: &str) -> Option<&Node> {
        for node in self.iter_depth_first() {
            if let Some(data) = node.element_data() {
                if data.id() == Some(id) {
                    return Some(node);
                }
            }
        }
        None
    }

    /// Basic query_selector supporting:
    /// - tag name: `div`
    /// - id: `#main`
    /// - class: `.highlight`
    /// - tag.class: `div.highlight`
    /// - tag#id: `div#main`
    pub fn query_selector(&self, selector: &str) -> Option<&Node> {
        let sel = SimpleSelector::parse(selector);
        for node in self.iter_depth_first() {
            if sel.matches(node) {
                return Some(node);
            }
        }
        None
    }

    /// Return all matching nodes for a basic selector.
    pub fn query_selector_all(&self, selector: &str) -> Vec<&Node> {
        let sel = SimpleSelector::parse(selector);
        self.iter_depth_first()
            .filter(|node| sel.matches(node))
            .collect()
    }

    /// Return a depth-first iterator over this node and all descendants.
    pub fn iter_depth_first(&self) -> DepthFirstIter<'_> {
        DepthFirstIter {
            stack: vec![self],
        }
    }
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.node_type {
            NodeType::Document => write!(f, "#document"),
            NodeType::Text(t) => write!(f, "\"{}\"", t),
            NodeType::Element(data) => {
                write!(f, "<{}", data.tag_name)?;
                for (k, v) in &data.attributes {
                    write!(f, " {}=\"{}\"", k, v)?;
                }
                write!(f, ">")
            }
        }
    }
}

/// Depth-first iterator over DOM nodes.
pub struct DepthFirstIter<'a> {
    stack: Vec<&'a Node>,
}

impl<'a> Iterator for DepthFirstIter<'a> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;
        // Push children in reverse so the first child is visited first.
        for child in node.children.iter().rev() {
            self.stack.push(child);
        }
        Some(node)
    }
}

/// A very simple CSS selector for query_selector support.
#[derive(Debug, Default)]
struct SimpleSelector {
    tag_name: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
}

impl SimpleSelector {
    /// Parse a simple selector string like `div`, `#id`, `.class`, `div.class`, `div#id`.
    fn parse(input: &str) -> Self {
        let mut sel = SimpleSelector::default();
        let input = input.trim();
        if input.is_empty() {
            return sel;
        }

        // Split on `#` and `.` while keeping delimiters.
        let mut current = String::new();
        let mut kind = 't'; // 't' = tag, '#' = id, '.' = class

        for ch in input.chars() {
            if ch == '#' || ch == '.' {
                Self::apply_part(&mut sel, kind, &current);
                current.clear();
                kind = ch;
            } else {
                current.push(ch);
            }
        }
        Self::apply_part(&mut sel, kind, &current);
        sel
    }

    fn apply_part(sel: &mut SimpleSelector, kind: char, value: &str) {
        if value.is_empty() {
            return;
        }
        match kind {
            't' => sel.tag_name = Some(value.to_lowercase()),
            '#' => sel.id = Some(value.to_string()),
            '.' => sel.classes.push(value.to_string()),
            _ => {}
        }
    }

    fn matches(&self, node: &Node) -> bool {
        let data = match node.element_data() {
            Some(d) => d,
            None => return false,
        };

        if let Some(ref tag) = self.tag_name {
            if data.tag_name != *tag {
                return false;
            }
        }

        if let Some(ref id) = self.id {
            if data.id() != Some(id.as_str()) {
                return false;
            }
        }

        let node_classes = data.classes();
        for cls in &self.classes {
            if !node_classes.contains(&cls.as_str()) {
                return false;
            }
        }

        true
    }
}

/// Top-level Document wrapper around the root node.
#[derive(Debug, Clone)]
pub struct Document {
    pub root: Node,
    /// Canvas drawing calls from scripts not yet drawn by the page (canvas id, call).
    pub canvas_ops: Vec<(String, crate::canvas::Op)>,
    /// Nodes scripts made or took out of the tree and hold on to.
    pub detached: Vec<Node>,
}

impl Document {
    /// Create a new empty document.
    pub fn new() -> Self {
        Document {
            canvas_ops: Vec::new(),
            root: Node::new_document(),
            detached: Vec::new(),
        }
    }

    /// Find an element by id anywhere in the document.
    pub fn get_element_by_id(&self, id: &str) -> Option<&Node> {
        self.root.get_element_by_id(id)
    }

    /// Run a basic selector query on the document.
    pub fn query_selector(&self, selector: &str) -> Option<&Node> {
        self.root.query_selector(selector)
    }

    /// Run a basic selector query returning all matches.
    pub fn query_selector_all(&self, selector: &str) -> Vec<&Node> {
        self.root.query_selector_all(selector)
    }

    /// Iterate depth-first over all nodes.
    pub fn iter(&self) -> DepthFirstIter<'_> {
        self.root.iter_depth_first()
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_tree() {
        let mut doc = Document::new();
        let mut div = Node::new_element("div", HashMap::from([("id".into(), "main".into())]));
        div.append_child(Node::new_text("Hello"));
        doc.root.append_child(div);

        let found = doc.get_element_by_id("main").unwrap();
        assert_eq!(found.element_data().unwrap().tag_name, "div");
        assert_eq!(found.inner_text(), "Hello");
    }

    #[test]
    fn test_query_selector() {
        let mut doc = Document::new();
        let mut div = Node::new_element(
            "div",
            HashMap::from([("class".into(), "container highlight".into())]),
        );
        let span = Node::new_element("span", HashMap::new());
        div.append_child(span);
        doc.root.append_child(div);

        assert!(doc.query_selector("div.container").is_some());
        assert!(doc.query_selector(".highlight").is_some());
        assert!(doc.query_selector("span").is_some());
        assert!(doc.query_selector("p").is_none());
    }

    #[test]
    fn test_depth_first_iter() {
        let mut root = Node::new_element("div", HashMap::new());
        root.append_child(Node::new_text("A"));
        root.append_child(Node::new_text("B"));

        let texts: Vec<&str> = root
            .iter_depth_first()
            .filter_map(|n| n.text_content())
            .collect();
        assert_eq!(texts, vec!["A", "B"]);
    }
}
