//! The live DOM scripts work on: mb-qjs's `dom-live.js` calls these
//! operations on Blitz's document through [`mb_core::js_engine::Dom`].
//! Nodes are Blitz node ids; every change marks the page for a new frame.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use blitz_dom::{ns, LocalName, Namespace, NodeData, NodeId, QualName};
use blitz_html::{DocumentHtmlParser, HtmlDocument};
use mb_core::js_engine::Dom;
use serde_json::{json, Value};

pub struct BlitzDom {
    pub doc: Rc<RefCell<HtmlDocument>>,
    /// Set when a call changed the document.
    pub changed: Rc<Cell<bool>>,
}

fn id(v: Option<&Value>) -> Option<NodeId> {
    v.and_then(Value::as_u64).map(NodeId::from_u64)
}

/// A node id for JS (a number: index in the low 32 bits, version above).
fn js(n: Option<NodeId>) -> Value {
    n.map_or(Value::Null, |n| json!(n.as_u64()))
}

fn js_all(ns: impl IntoIterator<Item = NodeId>) -> Value {
    Value::Array(ns.into_iter().map(|n| json!(n.as_u64())).collect())
}

fn text(v: Option<&Value>) -> &str {
    v.and_then(Value::as_str).unwrap_or("")
}

fn attr_name(name: &str) -> QualName {
    QualName::new(None, ns!(), LocalName::from(name))
}

impl Dom for BlitzDom {
    fn call(&mut self, op: &str, a: &[Value]) -> Value {
        let mut doc = self.doc.borrow_mut();
        let n = id(a.first());
        let exists = |d: &HtmlDocument, n: Option<NodeId>| n.and_then(|n| d.get_node(n)).is_some();
        match op {
            // ---- reads
            "root" => js(Some(doc.root_node().id)),
            "viewport" => {
                let (w, h) = doc.viewport().window_size;
                json!([w, h])
            }
            "type" => match n.and_then(|n| doc.get_node(n)).map(|n| &n.data) {
                Some(NodeData::Document(_)) => json!(9),
                Some(NodeData::Element(_)) | Some(NodeData::AnonymousBlock(_)) => json!(1),
                Some(NodeData::Text(_)) => json!(3),
                Some(NodeData::Comment { .. }) => json!(8),
                _ => json!(0),
            },
            "tag" => json!(n.and_then(|n| doc.get_node(n)).and_then(|n| n.element_data()).map(|e| e.name.local.to_string())),
            "parent" => js(n.and_then(|n| doc.get_node(n)).and_then(|n| n.parent)),
            "kids" => js_all(n.and_then(|n| doc.get_node(n)).map(|n| n.children.to_vec()).unwrap_or_default()),
            "text" => json!(n.and_then(|n| doc.get_node(n)).map(|n| n.text_content()).unwrap_or_default()),
            "attr" => json!(n.and_then(|n| doc.get_node(n)).and_then(|n| n.attr(LocalName::from(text(a.get(1))))).map(str::to_string)),
            "attrs" => {
                let list: Vec<(String, String)> = n
                    .and_then(|n| doc.get_node(n))
                    .and_then(|n| n.element_data())
                    .map(|e| e.attrs.iter().map(|a| (a.name.local.to_string(), a.value.clone())).collect())
                    .unwrap_or_default();
                json!(list)
            }
            "byId" => js(doc.get_element_by_id(text(a.first()))),
            "query" => {
                let sel = text(a.get(1));
                let r = match n {
                    Some(scope) => doc.query_selector_in(scope, sel),
                    None => doc.query_selector(sel),
                };
                js(r.ok().flatten())
            }
            "queryAll" => {
                let sel = text(a.get(1));
                let r = match n {
                    Some(scope) => doc.query_selector_all_in(scope, sel),
                    None => doc.query_selector_all(sel),
                };
                js_all(r.map(|v| v.to_vec()).unwrap_or_default())
            }
            "matches" => {
                // Matches = found by the selector from the document root.
                let Some(n) = n else { return json!(false) };
                json!(doc.query_selector_all(text(a.get(1))).map(|v| v.contains(&n)).unwrap_or(false))
            }
            "html" => {
                let Some(node) = n.and_then(|n| doc.get_node(n)) else { return json!("") };
                let mut out = String::new();
                for &c in &node.children {
                    if let Some(c) = doc.get_node(c) {
                        c.write_outer_html(&mut out);
                    }
                }
                json!(out)
            }
            "outer" => json!(n.and_then(|n| doc.get_node(n)).map(|n| n.outer_html()).unwrap_or_default()),
            "rect" => {
                let Some(node) = n.and_then(|n| doc.get_node(n)) else { return Value::Null };
                let p = node.absolute_position(0.0, 0.0);
                let s = node.final_layout().size;
                json!({ "x": p.x, "y": p.y, "w": s.width, "h": s.height })
            }
            "fetch" => {
                // Web pages' fetch/XHR (files keep dom.js's reader): the URL
                // against the page's, synchronously.
                let url = text(a.first());
                let full = doc.base_url().join(url).map(|u| u.to_string()).unwrap_or_else(|_| url.to_string());
                drop(doc);
                if !(full.starts_with("http://") || full.starts_with("https://")) {
                    return json!({ "local": true });
                }
                match crate::net::http_get(&full) {
                    Some(body) => json!({ "ok": true, "status": 200, "url": full, "text": String::from_utf8_lossy(&body) }),
                    None => json!({ "ok": false, "status": 0, "url": full, "text": "" }),
                }
            }
            "computed" => {
                let Some(n) = n else { return json!("") };
                // Layout and styles of what scripts changed so far.
                doc.resolve(0.0);
                json!(doc.resolved_style_value(n, text(a.get(1))))
            }

            // ---- changes
            _ => {
                let v = match op {
                    "create" => {
                        let tag = text(a.first());
                        let space = match a.get(1).and_then(Value::as_str) {
                            Some("http://www.w3.org/2000/svg") => ns!(svg),
                            Some(other) => Namespace::from(other),
                            None => ns!(html),
                        };
                        js(Some(doc.mutate().create_element(QualName::new(None, space, LocalName::from(tag)), Vec::new())))
                    }
                    "createText" => js(Some(doc.mutate().create_text_node(text(a.first())))),
                    "setText" => {
                        let Some(n) = n.filter(|&n| exists(&doc, Some(n))) else { return Value::Null };
                        let is_text = matches!(doc.get_node(n).map(|n| &n.data), Some(NodeData::Text(_)));
                        let mut m = doc.mutate();
                        if is_text {
                            m.set_node_text(n, text(a.get(1)));
                        } else {
                            m.remove_and_drop_all_children(n);
                            let s = text(a.get(1));
                            if !s.is_empty() {
                                let t = m.create_text_node(s);
                                m.append_children(n, &[t]);
                            }
                        }
                        Value::Null
                    }
                    "setAttr" => {
                        let Some(n) = n.filter(|&n| exists(&doc, Some(n))) else { return Value::Null };
                        doc.mutate().set_attribute(n, attr_name(text(a.get(1))), text(a.get(2)));
                        Value::Null
                    }
                    "rmAttr" => {
                        let Some(n) = n.filter(|&n| exists(&doc, Some(n))) else { return Value::Null };
                        doc.mutate().clear_attribute(n, attr_name(text(a.get(1))));
                        Value::Null
                    }
                    "setStyle" => {
                        let Some(n) = n.filter(|&n| exists(&doc, Some(n))) else { return Value::Null };
                        let (prop, val) = (text(a.get(1)), text(a.get(2)));
                        let mut m = doc.mutate();
                        if val.is_empty() {
                            m.remove_style_property(n, prop);
                        } else {
                            m.set_style_property(n, prop, val);
                        }
                        Value::Null
                    }
                    "append" | "insertBefore" => {
                        let (Some(p), Some(c)) = (n, id(a.get(1))) else { return Value::Null };
                        if !exists(&doc, Some(p)) || !exists(&doc, Some(c)) || p == c {
                            return Value::Null;
                        }
                        let reference = id(a.get(2)).filter(|&r| doc.get_node(r).is_some_and(|r| r.parent == Some(p)));
                        let mut m = doc.mutate();
                        // A node elsewhere in the tree moves.
                        if m.node_has_parent(c) {
                            m.remove_node(c);
                        }
                        match (op, reference) {
                            ("insertBefore", Some(r)) => m.insert_nodes_before(r, &[c]),
                            _ => m.append_children(p, &[c]),
                        }
                        Value::Null
                    }
                    "remove" => {
                        let Some(n) = n.filter(|&n| exists(&doc, Some(n))) else { return Value::Null };
                        let mut m = doc.mutate();
                        if m.node_has_parent(n) {
                            m.remove_node(n);
                        }
                        Value::Null
                    }
                    "setHTML" => {
                        let Some(n) = n.filter(|&n| exists(&doc, Some(n))) else { return Value::Null };
                        let mut m = doc.mutate();
                        m.remove_and_drop_all_children(n);
                        DocumentHtmlParser::parse_inner_html_into_mutator(&mut m, n, text(a.get(1)));
                        Value::Null
                    }
                    "clone" => {
                        let Some(n) = n.filter(|&n| exists(&doc, Some(n))) else { return Value::Null };
                        let deep = a.get(1).and_then(Value::as_bool).unwrap_or(false);
                        let mut m = doc.mutate();
                        let c = m.deep_clone_node(n);
                        if !deep {
                            m.remove_and_drop_all_children(c);
                        }
                        js(Some(c))
                    }
                    other => {
                        log::warn!("dom: unknown op {other}");
                        return Value::Null;
                    }
                };
                self.changed.set(true);
                v
            }
        }
    }
}
