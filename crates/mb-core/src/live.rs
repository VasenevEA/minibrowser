//! The live DOM on mb-core's own tree: the operations mb-qjs's `dom-live.js`
//! calls (see [`crate::js_engine::Dom`]). Nodes are named by their `uid`.
//!
//! The page lends its document to [`CoreDom`] while a script runs (see
//! `Page::run_js`) and takes it back after. Changes that keep the tree's shape
//! (an attribute, a style, an element's text, on an element with an id) are
//! also logged as the by-id commands the page already knows, so its fast
//! repaint of SVG drawings still applies; any other change asks for a full one.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use serde_json::{json, Value};

use crate::css::{selector_matches_in, Selector};
use crate::dom::{Document, ElementData, Node, NodeType};
use crate::js_engine::{DomCommand, Dom};

pub struct CoreDom {
    /// The page's document while a script runs, else None.
    pub doc: Rc<RefCell<Option<Document>>>,
    /// By-id equivalents of the changes that keep the tree's shape.
    pub log: Rc<RefCell<Vec<DomCommand>>>,
    /// A change the log cannot express happened: render it all.
    pub structural: Rc<Cell<bool>>,
    /// Element boxes of the last render: uid → x, y, w, h.
    pub rects: Rc<RefCell<HashMap<u32, [f32; 4]>>>,
    pub viewport: (u32, u32),
}

fn uid(v: Option<&Value>) -> Option<u32> {
    v.and_then(Value::as_u64).map(|n| n as u32)
}

fn text(v: Option<&Value>) -> &str {
    v.and_then(Value::as_str).unwrap_or("")
}

/// Where a node is: in the tree (`None` root) or the n-th detached node, and
/// the path down from there.
#[derive(Clone)]
struct Loc {
    detached: Option<usize>,
    path: Vec<usize>,
}

fn locate(doc: &Document, n: u32) -> Option<Loc> {
    if let Some(path) = doc.root.path_to(n) {
        return Some(Loc { detached: None, path });
    }
    doc.detached.iter().enumerate().find_map(|(i, d)| d.path_to(n).map(|path| Loc { detached: Some(i), path }))
}

fn node(doc: &Document, n: u32) -> Option<&Node> {
    let l = locate(doc, n)?;
    let top = match l.detached {
        None => &doc.root,
        Some(i) => &doc.detached[i],
    };
    top.at_path(&l.path)
}

fn node_mut(doc: &mut Document, n: u32) -> Option<&mut Node> {
    let l = locate(doc, n)?;
    let top = match l.detached {
        None => &mut doc.root,
        Some(i) => &mut doc.detached[i],
    };
    top.at_path_mut(&l.path)
}

fn parent(doc: &Document, n: u32) -> Option<u32> {
    let l = locate(doc, n)?;
    let (_, up) = l.path.split_last()?;
    let top = match l.detached {
        None => &doc.root,
        Some(i) => &doc.detached[i],
    };
    top.at_path(up).map(|p| p.uid)
}

/// Takes node `n` out of wherever it is (its parent, or the detached list).
fn take(doc: &mut Document, n: u32) -> Option<Node> {
    let l = locate(doc, n)?;
    match l.path.split_last() {
        None => l.detached.map(|i| doc.detached.remove(i)),
        Some((&last, up)) => {
            let top = match l.detached {
                None => &mut doc.root,
                Some(i) => &mut doc.detached[i],
            };
            let p = top.at_path_mut(up)?;
            (last < p.children.len()).then(|| p.children.remove(last))
        }
    }
}

/// Whether `n` is `ancestor` or inside it.
fn within(doc: &Document, n: u32, ancestor: u32) -> bool {
    node(doc, ancestor).is_some_and(|a| a.path_to(n).is_some())
}

/// Element nodes under `scope` (not itself) matching a selector list, in
/// document order.
fn query(scope: &Node, ancestors_of_scope: &[&ElementData], selectors: &[Selector], first: bool) -> Vec<u32> {
    fn walk<'a>(n: &'a Node, chain: &mut Vec<&'a ElementData>, sels: &[Selector], first: bool, out: &mut Vec<u32>) -> bool {
        for c in &n.children {
            if let NodeType::Element(e) = &c.node_type {
                if sels.iter().any(|s| selector_matches_in(s, e, chain)) {
                    out.push(c.uid);
                    if first {
                        return true;
                    }
                }
                chain.push(e);
                let done = walk(c, chain, sels, first, out);
                chain.pop();
                if done {
                    return true;
                }
            }
        }
        false
    }
    let mut chain: Vec<&ElementData> = ancestors_of_scope.to_vec();
    if let NodeType::Element(e) = &scope.node_type {
        chain.push(e);
    }
    let mut out = Vec::new();
    walk(scope, &mut chain, selectors, first, &mut out);
    out
}

fn selectors(list: &str) -> Vec<Selector> {
    list.split(',').filter_map(|s| Selector::parse_complex(s.trim())).collect()
}

/// The elements above `n`, root first.
fn ancestors(doc: &Document, n: u32) -> Vec<&ElementData> {
    let Some(l) = locate(doc, n) else { return Vec::new() };
    let mut out = Vec::new();
    let mut at = match l.detached {
        None => &doc.root,
        Some(i) => &doc.detached[i],
    };
    for &i in &l.path {
        if let NodeType::Element(e) = &at.node_type {
            out.push(e);
        }
        at = &at.children[i];
    }
    out
}

fn escape(s: &str, attr: bool) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' if !attr => o.push_str("&lt;"),
            '>' if !attr => o.push_str("&gt;"),
            '"' if attr => o.push_str("&quot;"),
            c => o.push(c),
        }
    }
    o
}

const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"];

fn outer_html(n: &Node, out: &mut String) {
    match &n.node_type {
        NodeType::Text(t) => out.push_str(&escape(t, false)),
        NodeType::Document => n.children.iter().for_each(|c| outer_html(c, out)),
        NodeType::Element(e) => {
            out.push('<');
            out.push_str(&e.tag_name);
            let mut attrs: Vec<_> = e.attributes.iter().collect();
            attrs.sort();
            for (k, v) in attrs {
                out.push_str(&format!(" {k}=\"{}\"", escape(v, true)));
            }
            out.push('>');
            if VOID.contains(&e.tag_name.as_str()) {
                return;
            }
            n.children.iter().for_each(|c| outer_html(c, out));
            out.push_str(&format!("</{}>", e.tag_name));
        }
    }
}

impl CoreDom {
    fn id_of(doc: &Document, n: u32) -> Option<String> {
        node(doc, n).and_then(Node::element_data).and_then(|e| e.id()).map(str::to_string)
    }

    /// A change: by-id in the log when it can be, else a full render.
    fn changed(&self, by_id: Option<DomCommand>) {
        match by_id {
            Some(c) => self.log.borrow_mut().push(c),
            None => self.structural.set(true),
        }
    }
}

impl Dom for CoreDom {
    fn call(&mut self, op: &str, a: &[Value]) -> Value {
        if op == "viewport" {
            return json!([self.viewport.0, self.viewport.1]);
        }
        let slot = self.doc.clone();
        let mut guard = slot.borrow_mut();
        let Some(doc) = guard.as_mut() else { return Value::Null };
        let n = uid(a.first());
        match op {
            "root" => json!(doc.root.uid),
            "viewport" => json!([self.viewport.0, self.viewport.1]),
            "type" => json!(match n.and_then(|n| node(doc, n)).map(|n| &n.node_type) {
                Some(NodeType::Document) => 9,
                Some(NodeType::Element(_)) => 1,
                Some(NodeType::Text(_)) => 3,
                None => 0,
            }),
            "tag" => json!(n.and_then(|n| node(doc, n)).and_then(Node::element_data).map(|e| e.tag_name.clone())),
            "parent" => json!(n.and_then(|n| parent(doc, n))),
            "kids" => json!(n.and_then(|n| node(doc, n)).map(|n| n.children.iter().map(|c| c.uid).collect::<Vec<_>>()).unwrap_or_default()),
            "text" => json!(n.and_then(|n| node(doc, n)).map(Node::inner_text).unwrap_or_default()),
            "attr" => json!(n
                .and_then(|n| node(doc, n))
                .and_then(Node::element_data)
                .and_then(|e| e.attributes.get(&text(a.get(1)).to_ascii_lowercase()).cloned())),
            "attrs" => {
                let mut list: Vec<(String, String)> = n
                    .and_then(|n| node(doc, n))
                    .and_then(Node::element_data)
                    .map(|e| e.attributes.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();
                list.sort();
                json!(list)
            }
            "byId" => json!(doc.root.get_element_by_id(text(a.first())).map(|n| n.uid)),
            "query" | "queryAll" => {
                let sels = selectors(text(a.get(1)));
                let (scope, above) = match n.and_then(|n| node(doc, n).map(|s| (s, ancestors(doc, n)))) {
                    Some(x) => x,
                    None => (&doc.root, Vec::new()),
                };
                let found = query(scope, &above, &sels, op == "query");
                if op == "query" { json!(found.first()) } else { json!(found) }
            }
            "matches" => {
                let Some(n) = n else { return json!(false) };
                let sels = selectors(text(a.get(1)));
                let above = ancestors(doc, n);
                json!(node(doc, n)
                    .and_then(Node::element_data)
                    .is_some_and(|e| sels.iter().any(|s| selector_matches_in(s, e, &above))))
            }
            "html" => {
                let mut out = String::new();
                if let Some(n) = n.and_then(|n| node(doc, n)) {
                    n.children.iter().for_each(|c| outer_html(c, &mut out));
                }
                json!(out)
            }
            "outer" => {
                let mut out = String::new();
                if let Some(n) = n.and_then(|n| node(doc, n)) {
                    outer_html(n, &mut out);
                }
                json!(out)
            }
            "rect" => match n.and_then(|n| self.rects.borrow().get(&n).copied()) {
                Some([x, y, w, h]) => json!({ "x": x, "y": y, "w": w, "h": h }),
                None => Value::Null,
            },
            "computed" => {
                // Inline styles only: no computed styles kept per node.
                let prop = text(a.get(1));
                let v = n
                    .and_then(|n| node(doc, n))
                    .and_then(Node::element_data)
                    .and_then(|e| e.style())
                    .and_then(|s| {
                        s.split(';').find_map(|d| {
                            let (k, v) = d.split_once(':')?;
                            k.trim().eq_ignore_ascii_case(prop).then(|| v.trim().to_string())
                        })
                    });
                json!(v.unwrap_or_default())
            }
            "fetch" => json!({ "local": true }),

            "create" => {
                let el = Node::new_element(text(a.first()), HashMap::new());
                let id = el.uid;
                doc.detached.push(el);
                json!(id)
            }
            "createText" => {
                let t = Node::new_text(text(a.first()));
                let id = t.uid;
                doc.detached.push(t);
                json!(id)
            }
            "setText" => {
                let Some(n) = n else { return Value::Null };
                let s = text(a.get(1)).to_string();
                let id = Self::id_of(doc, n);
                let Some(target) = node_mut(doc, n) else { return Value::Null };
                let by_id = match &mut target.node_type {
                    NodeType::Text(t) => {
                        *t = s;
                        None
                    }
                    _ => {
                        target.children = if s.is_empty() { Vec::new() } else { vec![Node::new_text(&s)] };
                        id.map(|id| DomCommand::SetTextById { id, text: s })
                    }
                };
                self.changed(by_id);
                Value::Null
            }
            "setAttr" | "rmAttr" => {
                let Some(n) = n else { return Value::Null };
                let name = text(a.get(1)).to_ascii_lowercase();
                let value = text(a.get(2)).to_string();
                let id = Self::id_of(doc, n);
                let Some(NodeType::Element(e)) = node_mut(doc, n).map(|t| &mut t.node_type) else { return Value::Null };
                if op == "setAttr" {
                    e.attributes.insert(name.clone(), value.clone());
                } else {
                    e.attributes.remove(&name);
                }
                // An id or class change restyles more than this element.
                let simple = op == "setAttr" && name != "id" && name != "class";
                self.changed(id.filter(|_| simple).map(|id| DomCommand::SetAttribute { id, name, value }));
                Value::Null
            }
            "setStyle" => {
                let Some(n) = n else { return Value::Null };
                let (prop, val) = (text(a.get(1)).to_string(), text(a.get(2)).to_string());
                let id = Self::id_of(doc, n);
                let Some(NodeType::Element(e)) = node_mut(doc, n).map(|t| &mut t.node_type) else { return Value::Null };
                let style = e.attributes.entry("style".into()).or_default();
                crate::js_engine::set_style_decl(style, &prop, &val);
                self.changed(id.map(|id| DomCommand::SetStyleById { id, property: prop, value: val }));
                Value::Null
            }
            "append" | "insertBefore" => {
                let (Some(p), Some(c)) = (n, uid(a.get(1))) else { return Value::Null };
                // Not into itself or its own subtree.
                if within(doc, p, c) || node(doc, p).is_none() {
                    return Value::Null;
                }
                let Some(child) = take(doc, c) else { return Value::Null };
                let reference = uid(a.get(2));
                let Some(parent) = node_mut(doc, p) else { return Value::Null };
                let at = reference.and_then(|r| parent.children.iter().position(|k| k.uid == r));
                match (op, at) {
                    ("insertBefore", Some(i)) => parent.children.insert(i, child),
                    _ => parent.children.push(child),
                }
                self.changed(None);
                Value::Null
            }
            "remove" => {
                let Some(n) = n else { return Value::Null };
                if parent(doc, n).is_some() {
                    if let Some(gone) = take(doc, n) {
                        // Scripts may put it back: keep it (the latest few hundred).
                        doc.detached.push(gone);
                        if doc.detached.len() > 512 {
                            doc.detached.drain(..256);
                        }
                        self.changed(None);
                    }
                }
                Value::Null
            }
            "setHTML" => {
                let Some(n) = n else { return Value::Null };
                let parsed = crate::html_parser::parse_html(text(a.get(1)));
                let kids = crate::js_engine::extract_body_children(&parsed.root);
                if let Some(target) = node_mut(doc, n) {
                    target.children = kids;
                    self.changed(None);
                }
                Value::Null
            }
            "clone" => {
                let Some(src) = n.and_then(|n| node(doc, n)) else { return Value::Null };
                let mut copy = src.clone();
                if !a.get(1).and_then(Value::as_bool).unwrap_or(false) {
                    copy.children.clear();
                }
                copy.renumber();
                let id = copy.uid;
                doc.detached.push(copy);
                json!(id)
            }
            other => {
                log::warn!("dom: unknown op {other}");
                Value::Null
            }
        }
    }
}
