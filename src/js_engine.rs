//! JavaScript execution engine for MiniBrowser.
//!
//! Since rquickjs does not compile to wasm32-unknown-unknown, this module uses
//! the **host browser's** JS engine via `js_sys::eval()`.  We inject a small
//! mock DOM API that records mutations into a command queue, execute the user
//! script, then return the list of `DomCommand`s so that the caller can apply
//! them to the MiniBrowser DOM tree before rendering.

use crate::dom::{Document, Node, NodeType};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

// ---------------------------------------------------------------------------
// DomCommand — the mutations a script can request
// ---------------------------------------------------------------------------

/// A single DOM mutation recorded by the mock API.
#[derive(Debug, Clone)]
pub enum DomCommand {
    /// Set the text content of an element identified by `id`.
    SetTextById { id: String, text: String },
    /// Set one inline-style property on an element identified by `id`.
    SetStyleById { id: String, property: String, value: String },
    /// Create a new element and append it to a parent identified by `parent_id`.
    CreateElement { tag: String, parent_id: String },
    /// Set an attribute on an element identified by `id`.
    SetAttribute { id: String, name: String, value: String },
    /// Set innerHTML on an element — parsed as HTML, not plain text.
    SetInnerHtmlById { id: String, html: String },
    /// Console.log output (informational, not a DOM change).
    ConsoleLog { message: String },
    /// Canvas 2D: fill a rectangle.
    CanvasFillRect { id: String, x: f32, y: f32, w: f32, h: f32, color: String },
    /// Canvas 2D: stroke a rectangle.
    CanvasStrokeRect { id: String, x: f32, y: f32, w: f32, h: f32, color: String },
    /// Canvas 2D: fill text.
    CanvasFillText { id: String, text: String, x: f32, y: f32, color: String, font: String },
    /// Canvas 2D: clear a rectangle.
    CanvasClearRect { id: String, x: f32, y: f32, w: f32, h: f32 },
}

// ---------------------------------------------------------------------------
// Script extraction
// ---------------------------------------------------------------------------

/// Walk the DOM tree and collect the text content of every `<script>` element.
pub fn extract_scripts(doc: &Document) -> Vec<String> {
    let mut scripts = Vec::new();
    collect_scripts(&doc.root, &mut scripts);
    scripts
}

fn collect_scripts(node: &Node, out: &mut Vec<String>) {
    if let NodeType::Element(ref data) = node.node_type {
        if data.tag_name == "script" {
            let text = node.inner_text();
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
        }
    }
    for child in &node.children {
        collect_scripts(child, out);
    }
}

// ---------------------------------------------------------------------------
// Host-JS execution
// ---------------------------------------------------------------------------

/// Reset all JS state (call when loading a new page).
pub fn reset_js_state() {
    let _ = js_sys::eval(
        "window.__mb_init = false; window.__mb_elements = {}; \
         window.__mb_cmds = []; window.__mb_handlers = {};"
    );
}

/// Build the JS wrapper that:
/// 1. Sets up mock DOM API (once, persistent across calls)
/// 2. Patches real Document.prototype.getElementById so (0,eval) code works
/// 3. Runs user code at global scope via (0,eval)() for var persistence
/// 4. Restores patched APIs
/// 5. Returns JSON array of DOM commands
fn wrap_script(user_code: &str) -> String {
    format!(
        r#"(function() {{
  window.__mb_cmds = [];

  // Setup mock DOM API only once
  if (!window.__mb_init) {{
    window.__mb_init = true;
    window.__mb_elements = {{}};
    window.__mb_handlers = {{}};

    window.MockElement = function(id) {{
      this._id = id || '';
      this._text = '';
      this._html = '';
      this._styles = {{}};
      this._attrs = {{}};
      this._tag = 'div';
      this._children = [];
      this.style = new Proxy({{}}, {{
        set: function(target, prop, value) {{
          target[prop] = value;
          window.__mb_cmds.push({{ type: 'SetStyleById', id: this._id, property: prop, value: String(value) }});
          return true;
        }}.bind(this)
      }});
    }};

    Object.defineProperty(MockElement.prototype, 'textContent', {{
      get: function() {{ return this._text; }},
      set: function(v) {{
        this._text = String(v);
        window.__mb_cmds.push({{ type: 'SetTextById', id: this._id, text: this._text }});
      }}
    }});

    Object.defineProperty(MockElement.prototype, 'innerHTML', {{
      get: function() {{ return this._html || this._text; }},
      set: function(v) {{
        this._html = String(v);
        this._text = String(v);
        window.__mb_cmds.push({{ type: 'SetInnerHtmlById', id: this._id, html: this._html }});
      }}
    }});

    Object.defineProperty(MockElement.prototype, 'innerText', {{
      get: function() {{ return this._text; }},
      set: function(v) {{
        this._text = String(v);
        window.__mb_cmds.push({{ type: 'SetTextById', id: this._id, text: this._text }});
      }}
    }});

    // onclick property — store handler globally for later invocation
    Object.defineProperty(MockElement.prototype, 'onclick', {{
      get: function() {{ return window.__mb_handlers['click_' + this._id] || null; }},
      set: function(fn) {{
        if (typeof fn === 'function') {{
          // Sanitize ID for use as JS variable name (replace non-alphanumeric with _)
          var safeId = this._id.replace(/[^a-zA-Z0-9]/g, '_');
          var handlerId = '__mb_click_' + safeId;
          window[handlerId] = fn;
          window.__mb_handlers['click_' + this._id] = fn;
          window.__mb_cmds.push({{ type: 'SetAttribute', id: this._id, name: 'onclick', value: handlerId + '()' }});
        }}
      }}
    }});

    MockElement.prototype.addEventListener = function(event, fn) {{
      if (event === 'click' && typeof fn === 'function') {{
        this.onclick = fn;
      }}
    }};

    MockElement.prototype.setAttribute = function(name, value) {{
      this._attrs[name] = value;
      window.__mb_cmds.push({{ type: 'SetAttribute', id: this._id, name: name, value: String(value) }});
    }};
    MockElement.prototype.getAttribute = function(name) {{
      return this._attrs[name] || null;
    }};
    MockElement.prototype.appendChild = function(child) {{
      this._children.push(child);
      window.__mb_cmds.push({{ type: 'CreateElement', tag: child._tag || 'div', parent_id: this._id }});
      return child;
    }};
    MockElement.prototype.removeChild = function(child) {{ return child; }};
    MockElement.prototype.remove = function() {{}};

    // classList mock
    Object.defineProperty(MockElement.prototype, 'classList', {{
      get: function() {{
        var self = this;
        return {{
          add: function(cls) {{ /* no-op for now */ }},
          remove: function(cls) {{ /* no-op for now */ }},
          toggle: function(cls) {{ /* no-op for now */ }},
          contains: function(cls) {{ return false; }}
        }};
      }}
    }});

    // value property for input elements
    Object.defineProperty(MockElement.prototype, 'value', {{
      get: function() {{ return this._attrs['value'] || ''; }},
      set: function(v) {{
        this._attrs['value'] = String(v);
        window.__mb_cmds.push({{ type: 'SetAttribute', id: this._id, name: 'value', value: String(v) }});
      }}
    }});

    // Canvas 2D context mock
    MockElement.prototype.getContext = function(type) {{
      if (type !== '2d') return null;
      var self = this;
      var ctx = {{
        _cmds: [],
        fillStyle: '#000000',
        strokeStyle: '#000000',
        lineWidth: 1,
        font: '16px sans-serif',
        fillRect: function(x, y, w, h) {{
          window.__mb_cmds.push({{type:'CanvasFillRect', id:self._id, x:x, y:y, w:w, h:h, color:this.fillStyle}});
        }},
        strokeRect: function(x, y, w, h) {{
          window.__mb_cmds.push({{type:'CanvasStrokeRect', id:self._id, x:x, y:y, w:w, h:h, color:this.strokeStyle}});
        }},
        fillText: function(text, x, y) {{
          window.__mb_cmds.push({{type:'CanvasFillText', id:self._id, text:text, x:x, y:y, color:this.fillStyle, font:this.font}});
        }},
        clearRect: function(x, y, w, h) {{
          window.__mb_cmds.push({{type:'CanvasClearRect', id:self._id, x:x, y:y, w:w, h:h}});
        }},
        beginPath: function() {{}},
        closePath: function() {{}},
        moveTo: function() {{}},
        lineTo: function() {{}},
        arc: function() {{}},
        fill: function() {{}},
        stroke: function() {{}}
      }};
      return ctx;
    }};

    // localStorage mock
    if (!window.__mb_storage) window.__mb_storage = {{}};
    window.__mb_localStorage = {{
      getItem: function(key) {{ return window.__mb_storage[key] || null; }},
      setItem: function(key, value) {{ window.__mb_storage[key] = String(value); }},
      removeItem: function(key) {{ delete window.__mb_storage[key]; }},
      clear: function() {{ window.__mb_storage = {{}}; }},
      get length() {{ return Object.keys(window.__mb_storage).length; }},
      key: function(i) {{ return Object.keys(window.__mb_storage)[i] || null; }}
    }};
  }}

  // --- Temporarily patch real browser APIs (store originals on window) ---

  window.__mb_origGEBI = Document.prototype.getElementById;
  Document.prototype.getElementById = function(id) {{
    if (!window.__mb_elements[id]) {{
      window.__mb_elements[id] = new MockElement(id);
    }}
    return window.__mb_elements[id];
  }};

  window.__mb_origQS = Document.prototype.querySelector;
  Document.prototype.querySelector = function(sel) {{
    if (sel && sel[0] === '#') {{
      return Document.prototype.getElementById.call(this, sel.slice(1));
    }}
    return new MockElement('__qs_' + sel);
  }};

  window.__mb_origQSA = Document.prototype.querySelectorAll;
  Document.prototype.querySelectorAll = function(sel) {{
    return [];
  }};

  window.__mb_origLS = Object.getOwnPropertyDescriptor(window, 'localStorage');
  Object.defineProperty(window, 'localStorage', {{ get: function() {{ return window.__mb_localStorage; }} }});

  window.__mb_origCE = document.createElement.bind(document);
  document.createElement = function(tag) {{
    var el = new MockElement('');
    el._tag = tag;
    return el;
  }};

  window.__mb_origCL = console.log;
  window.__mb_origCW = console.warn;
  window.__mb_origCErr = console.error;
  console.log = function() {{
    var msg = Array.prototype.slice.call(arguments).map(function(a) {{
      return typeof a === 'object' ? JSON.stringify(a) : String(a);
    }}).join(' ');
    window.__mb_cmds.push({{ type: 'ConsoleLog', message: msg }});
  }};
  console.warn = console.log;
  console.error = console.log;

  // Track registered intervals/timeouts
  if (!window.__mb_intervals) {{ window.__mb_intervals = []; }}
  if (!window.__mb_timeouts) {{ window.__mb_timeouts = []; }}

  // Patch setTimeout/setInterval to capture callbacks
  window.__mb_origSetTimeout = window.setTimeout;
  window.__mb_origSetInterval = window.setInterval;
  window.setTimeout = function(fn, ms) {{
    if (typeof fn === 'function') {{
      window.__mb_timeouts.push(fn);
      fn(); // execute once immediately
    }}
    return 0;
  }};
  window.setInterval = function(fn, ms) {{
    if (typeof fn === 'function') {{
      window.__mb_intervals.push(fn);
      fn(); // execute once immediately
    }}
    return window.__mb_intervals.length - 1;
  }};
}})();

// Phase 2: Execute user code at TRUE global scope via (0,eval)().
// Variables declared with `var` become global and PERSIST across calls.
try {{
  (0, eval)({user_code_json});
}} catch(e) {{
  window.__mb_cmds.push({{ type: 'ConsoleLog', message: 'JS Error: ' + e.message }});
}}

// Phase 3: Restore real browser APIs
Document.prototype.getElementById = window.__mb_origGEBI;
Document.prototype.querySelector = window.__mb_origQS;
Document.prototype.querySelectorAll = window.__mb_origQSA;
if (window.__mb_origLS) Object.defineProperty(window, 'localStorage', window.__mb_origLS);
document.createElement = window.__mb_origCE;
console.log = window.__mb_origCL;
console.warn = window.__mb_origCW;
console.error = window.__mb_origCErr;
window.setTimeout = window.__mb_origSetTimeout;
window.setInterval = window.__mb_origSetInterval;

JSON.stringify(window.__mb_cmds)"#,
        user_code_json = serde_json::to_string(user_code).unwrap_or_else(|_| "\"\"".to_string())
    )
}

/// Execute a single script string via the host browser and return the resulting
/// DOM commands.
fn execute_one(script: &str) -> Vec<DomCommand> {
    let wrapped = wrap_script(script);
    let result = js_sys::eval(&wrapped);
    match result {
        Ok(val) => {
            if let Some(json_str) = val.as_string() {
                parse_commands(&json_str)
            } else {
                log::warn!("js_engine: eval returned non-string");
                Vec::new()
            }
        }
        Err(e) => {
            let msg = e
                .as_string()
                .unwrap_or_else(|| format!("{:?}", e));
            log::warn!("js_engine: eval error: {}", msg);
            Vec::new()
        }
    }
}

/// Parse the JSON array returned by the wrapper into `DomCommand`s.
fn parse_commands(json: &str) -> Vec<DomCommand> {
    let parsed = match js_sys::JSON::parse(json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let arr: js_sys::Array = match parsed.dyn_into() {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };

    let mut cmds = Vec::new();
    for i in 0..arr.length() {
        let obj = arr.get(i);
        let get = |key: &str| -> String {
            js_sys::Reflect::get(&obj, &JsValue::from_str(key))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default()
        };
        let get_f32 = |key: &str| -> f32 {
            js_sys::Reflect::get(&obj, &JsValue::from_str(key))
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as f32
        };

        let cmd_type = get("type");
        match cmd_type.as_str() {
            "SetTextById" => cmds.push(DomCommand::SetTextById {
                id: get("id"),
                text: get("text"),
            }),
            "SetStyleById" => cmds.push(DomCommand::SetStyleById {
                id: get("id"),
                property: get("property"),
                value: get("value"),
            }),
            "CreateElement" => cmds.push(DomCommand::CreateElement {
                tag: get("tag"),
                parent_id: get("parent_id"),
            }),
            "SetAttribute" => cmds.push(DomCommand::SetAttribute {
                id: get("id"),
                name: get("name"),
                value: get("value"),
            }),
            "SetInnerHtmlById" => cmds.push(DomCommand::SetInnerHtmlById {
                id: get("id"),
                html: get("html"),
            }),
            "ConsoleLog" => cmds.push(DomCommand::ConsoleLog {
                message: get("message"),
            }),
            "CanvasFillRect" => cmds.push(DomCommand::CanvasFillRect {
                id: get("id"), x: get_f32("x"), y: get_f32("y"),
                w: get_f32("w"), h: get_f32("h"), color: get("color"),
            }),
            "CanvasStrokeRect" => cmds.push(DomCommand::CanvasStrokeRect {
                id: get("id"), x: get_f32("x"), y: get_f32("y"),
                w: get_f32("w"), h: get_f32("h"), color: get("color"),
            }),
            "CanvasFillText" => cmds.push(DomCommand::CanvasFillText {
                id: get("id"), text: get("text"), x: get_f32("x"),
                y: get_f32("y"), color: get("color"), font: get("font"),
            }),
            "CanvasClearRect" => cmds.push(DomCommand::CanvasClearRect {
                id: get("id"), x: get_f32("x"), y: get_f32("y"),
                w: get_f32("w"), h: get_f32("h"),
            }),
            _ => {}
        }
    }
    cmds
}

// ---------------------------------------------------------------------------
// Apply commands to the DOM
// ---------------------------------------------------------------------------

/// Apply a list of DOM commands to the MiniBrowser DOM tree (mutating in place).
pub fn apply_commands(doc: &mut Document, commands: &[DomCommand]) {
    for cmd in commands {
        match cmd {
            DomCommand::SetTextById { id, text } => {
                set_text_by_id(&mut doc.root, id, text);
            }
            DomCommand::SetStyleById { id, property, value } => {
                set_style_by_id(&mut doc.root, id, property, value);
            }
            DomCommand::CreateElement { tag, parent_id } => {
                create_element_under(&mut doc.root, parent_id, tag);
            }
            DomCommand::SetAttribute { id, name, value } => {
                set_attribute_by_id(&mut doc.root, id, name, value);
            }
            DomCommand::SetInnerHtmlById { id, html } => {
                set_inner_html_by_id(&mut doc.root, id, html);
            }
            DomCommand::ConsoleLog { message } => {
                let _ = message; // used below in wasm target
                #[cfg(target_arch = "wasm32")]
                web_sys::console::log_1(&JsValue::from_str(
                    &format!("[minibrowser JS] {}", message),
                ));
            }
            DomCommand::CanvasFillRect { id, x, y, w, h, color } => {
                let json = format!(r#"{{"cmd":"fillRect","x":{},"y":{},"w":{},"h":{},"color":"{}"}}"#, x, y, w, h, color);
                append_canvas_cmd(&mut doc.root, id, &json);
            }
            DomCommand::CanvasStrokeRect { id, x, y, w, h, color } => {
                let json = format!(r#"{{"cmd":"strokeRect","x":{},"y":{},"w":{},"h":{},"color":"{}"}}"#, x, y, w, h, color);
                append_canvas_cmd(&mut doc.root, id, &json);
            }
            DomCommand::CanvasFillText { id, text, x, y, color, font } => {
                let json = format!(r#"{{"cmd":"fillText","text":"{}","x":{},"y":{},"color":"{}","font":"{}"}}"#, text, x, y, color, font);
                append_canvas_cmd(&mut doc.root, id, &json);
            }
            DomCommand::CanvasClearRect { id, x, y, w, h } => {
                let json = format!(r#"{{"cmd":"clearRect","x":{},"y":{},"w":{},"h":{}}}"#, x, y, w, h);
                append_canvas_cmd(&mut doc.root, id, &json);
            }
        }
    }
}

/// Find element by id and replace its children with a single text node.
fn set_text_by_id(node: &mut Node, id: &str, text: &str) {
    if let NodeType::Element(ref data) = node.node_type {
        if data.id() == Some(id) {
            node.children.clear();
            if !text.is_empty() {
                node.children.push(Node::new_text(text));
            }
            return;
        }
    }
    for child in &mut node.children {
        set_text_by_id(child, id, text);
    }
}

/// Find element by id and replace its children with parsed HTML fragment nodes.
fn set_inner_html_by_id(node: &mut Node, id: &str, html: &str) {
    if let NodeType::Element(ref data) = node.node_type {
        if data.id() == Some(id) {
            let parsed_doc = crate::html_parser::parse_html(html);
            let body_children = extract_body_children(&parsed_doc.root);
            node.children = body_children;
            return;
        }
    }
    for child in &mut node.children {
        set_inner_html_by_id(child, id, html);
    }
}

/// Extract children from the `<body>` element (or fall back to all descendants).
fn extract_body_children(root: &Node) -> Vec<Node> {
    if let NodeType::Element(ref data) = root.node_type {
        if data.tag_name == "body" {
            return root.children.clone();
        }
    }
    for child in &root.children {
        let result = extract_body_children(child);
        if !result.is_empty() {
            return result;
        }
        if let NodeType::Element(ref data) = child.node_type {
            if data.tag_name == "html" {
                for grandchild in &child.children {
                    if let NodeType::Element(ref gd) = grandchild.node_type {
                        if gd.tag_name == "body" {
                            return grandchild.children.clone();
                        }
                    }
                }
            }
        }
    }
    root.children.clone()
}

/// Find element by id and merge a style property into its `style` attribute.
fn set_style_by_id(node: &mut Node, id: &str, property: &str, value: &str) {
    if let NodeType::Element(ref mut data) = node.node_type {
        if data.id() == Some(id) {
            let css_prop = camel_to_kebab(property);
            let style_attr = data
                .attributes
                .entry("style".to_string())
                .or_insert_with(String::new);
            if !style_attr.is_empty() && !style_attr.ends_with(';') {
                style_attr.push(';');
            }
            style_attr.push_str(&format!("{}:{};", css_prop, value));
            return;
        }
    }
    for child in &mut node.children {
        set_style_by_id(child, id, property, value);
    }
}

/// Create a new element and append it under the parent with the given id.
fn create_element_under(node: &mut Node, parent_id: &str, tag: &str) {
    if let NodeType::Element(ref data) = node.node_type {
        if data.id() == Some(parent_id) {
            node.children
                .push(Node::new_element(tag, HashMap::new()));
            return;
        }
    }
    for child in &mut node.children {
        create_element_under(child, parent_id, tag);
    }
}

/// Set an attribute on an element by id.
fn set_attribute_by_id(node: &mut Node, id: &str, name: &str, value: &str) {
    if let NodeType::Element(ref mut data) = node.node_type {
        if data.id() == Some(id) {
            data.attributes
                .insert(name.to_string(), value.to_string());
            return;
        }
    }
    for child in &mut node.children {
        set_attribute_by_id(child, id, name, value);
    }
}

/// Append a canvas drawing command (as JSON) to the `data-canvas-cmds` attribute.
/// Stores commands as a JSON array string, e.g. `[{...},{...}]`.
fn append_canvas_cmd(node: &mut Node, id: &str, cmd_json: &str) {
    if let NodeType::Element(ref mut data) = node.node_type {
        if data.id() == Some(id) {
            let attr = data
                .attributes
                .entry("data-canvas-cmds".to_string())
                .or_insert_with(|| "[]".to_string());
            // Insert before the closing ']'
            if attr == "[]" {
                *attr = format!("[{}]", cmd_json);
            } else {
                // Remove trailing ']', append comma + new cmd + ']'
                attr.pop(); // remove ']'
                attr.push(',');
                attr.push_str(cmd_json);
                attr.push(']');
            }
            return;
        }
    }
    for child in &mut node.children {
        append_canvas_cmd(child, id, cmd_json);
    }
}

/// Convert a camelCase string to kebab-case (e.g. `marginTop` → `margin-top`).
fn camel_to_kebab(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for ch in s.chars() {
        if ch.is_ascii_uppercase() {
            result.push('-');
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Extract `<script>` tags from the document, execute them via the host
/// browser's JS engine, and apply the resulting DOM commands back to the tree.
///
/// Returns the number of scripts executed.
pub fn execute_scripts(doc: &mut Document) -> usize {
    let scripts = extract_scripts(doc);
    let count = scripts.len();
    if count == 0 {
        return 0;
    }

    log::info!("js_engine: executing {} script(s)", count);

    for script in &scripts {
        let commands = execute_one(script);
        log::info!("js_engine: script produced {} command(s)", commands.len());
        apply_commands(doc, &commands);
    }

    // Remove <script> elements from the DOM so they are not rendered.
    remove_script_elements(&mut doc.root);

    count
}

/// Execute an onclick handler and apply DOM commands.
/// Scripts are NOT re-executed — their state persists in window.* globals.
///
/// Returns the number of commands produced.
pub fn execute_onclick(doc: &mut Document, onclick_code: &str) -> usize {
    log::info!("js_engine: executing onclick ({} bytes)", onclick_code.len());

    let commands = execute_one(onclick_code);
    log::info!("js_engine: onclick produced {} command(s)", commands.len());
    apply_commands(doc, &commands);

    commands.len()
}

/// Execute all registered setInterval callbacks and apply DOM commands.
/// Called on re-renders to update dynamic content (clocks, etc.).
pub fn tick_timers(doc: &mut Document) -> usize {
    let tick_code = r#"
        window.__mb_cmds = [];
        var _gEBI = Document.prototype.getElementById;
        Document.prototype.getElementById = function(id) {
            if (!window.__mb_elements[id]) { window.__mb_elements[id] = new MockElement(id); }
            return window.__mb_elements[id];
        };
        var _qs = Document.prototype.querySelector;
        Document.prototype.querySelector = function(sel) {
            if (sel && sel[0] === '#') return Document.prototype.getElementById.call(this, sel.slice(1));
            return new MockElement('');
        };
        var _cl = console.log; var _cw = console.warn; var _ce = console.error;
        console.log = function() {
            var msg = Array.prototype.slice.call(arguments).map(function(a) {
                return typeof a === 'object' ? JSON.stringify(a) : String(a);
            }).join(' ');
            window.__mb_cmds.push({ type: 'ConsoleLog', message: msg });
        };
        console.warn = console.log; console.error = console.log;
        try {
            var intervals = window.__mb_intervals || [];
            for (var i = 0; i < intervals.length; i++) {
                if (typeof intervals[i] === 'function') intervals[i]();
            }
        } catch(e) {
            window.__mb_cmds.push({ type: 'ConsoleLog', message: 'Timer Error: ' + e.message });
        }
        Document.prototype.getElementById = _gEBI;
        Document.prototype.querySelector = _qs;
        console.log = _cl; console.warn = _cw; console.error = _ce;
        JSON.stringify(window.__mb_cmds)
    "#;

    let result = js_sys::eval(tick_code);
    match result {
        Ok(val) => {
            if let Some(json_str) = val.as_string() {
                let commands = parse_commands(&json_str);
                let count = commands.len();
                apply_commands(doc, &commands);
                count
            } else { 0 }
        }
        Err(_) => 0,
    }
}

/// Recursively remove `<script>` elements from the tree.
fn remove_script_elements(node: &mut Node) {
    node.children.retain(|child| {
        if let NodeType::Element(ref data) = child.node_type {
            data.tag_name != "script"
        } else {
            true
        }
    });
    for child in &mut node.children {
        remove_script_elements(child);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::{Document, Node, NodeType, ElementData};

    /// Helper: create a document with a body containing elements with IDs.
    fn make_doc_with_ids(ids: &[&str]) -> Document {
        let mut doc = Document::new();
        let mut body = Node::new_element("body", HashMap::new());
        for id in ids {
            let mut el = Node::new_element("div", HashMap::from([
                ("id".to_string(), id.to_string()),
            ]));
            el.children.push(Node::new_text("initial"));
            body.children.push(el);
        }
        doc.root.children.push(body);
        doc
    }

    /// Helper: find text content of element by id.
    fn get_text_by_id(doc: &Document, id: &str) -> Option<String> {
        fn find(node: &Node, id: &str) -> Option<String> {
            if let NodeType::Element(ref data) = node.node_type {
                if data.id() == Some(id) {
                    return Some(node.inner_text());
                }
            }
            for child in &node.children {
                if let Some(t) = find(child, id) {
                    return Some(t);
                }
            }
            None
        }
        find(&doc.root, id)
    }

    /// Helper: get attribute value by element id.
    fn get_attr_by_id(doc: &Document, id: &str, attr: &str) -> Option<String> {
        fn find(node: &Node, id: &str, attr: &str) -> Option<String> {
            if let NodeType::Element(ref data) = node.node_type {
                if data.id() == Some(id) {
                    return data.attributes.get(attr).cloned();
                }
            }
            for child in &node.children {
                if let Some(v) = find(child, id, attr) {
                    return Some(v);
                }
            }
            None
        }
        find(&doc.root, id, attr)
    }

    // -----------------------------------------------------------------------
    // camel_to_kebab
    // -----------------------------------------------------------------------

    #[test]
    fn camel_to_kebab_simple() {
        assert_eq!(camel_to_kebab("marginTop"), "margin-top");
    }

    #[test]
    fn camel_to_kebab_no_caps() {
        assert_eq!(camel_to_kebab("color"), "color");
    }

    #[test]
    fn camel_to_kebab_multiple_caps() {
        assert_eq!(camel_to_kebab("borderTopWidth"), "border-top-width");
    }

    #[test]
    fn camel_to_kebab_already_kebab() {
        assert_eq!(camel_to_kebab("background-color"), "background-color");
    }

    // -----------------------------------------------------------------------
    // SetTextById
    // -----------------------------------------------------------------------

    #[test]
    fn set_text_replaces_children() {
        let mut doc = make_doc_with_ids(&["count"]);
        assert_eq!(get_text_by_id(&doc, "count"), Some("initial".into()));

        let cmds = vec![DomCommand::SetTextById {
            id: "count".into(),
            text: "42".into(),
        }];
        apply_commands(&mut doc, &cmds);

        assert_eq!(get_text_by_id(&doc, "count"), Some("42".into()));
    }

    #[test]
    fn set_text_nonexistent_id_is_noop() {
        let mut doc = make_doc_with_ids(&["count"]);
        let cmds = vec![DomCommand::SetTextById {
            id: "missing".into(),
            text: "hello".into(),
        }];
        apply_commands(&mut doc, &cmds);
        // count unchanged
        assert_eq!(get_text_by_id(&doc, "count"), Some("initial".into()));
    }

    #[test]
    fn set_text_multiple_updates_same_element() {
        let mut doc = make_doc_with_ids(&["val"]);

        apply_commands(&mut doc, &[
            DomCommand::SetTextById { id: "val".into(), text: "1".into() },
        ]);
        assert_eq!(get_text_by_id(&doc, "val"), Some("1".into()));

        apply_commands(&mut doc, &[
            DomCommand::SetTextById { id: "val".into(), text: "2".into() },
        ]);
        assert_eq!(get_text_by_id(&doc, "val"), Some("2".into()));

        apply_commands(&mut doc, &[
            DomCommand::SetTextById { id: "val".into(), text: "3".into() },
        ]);
        assert_eq!(get_text_by_id(&doc, "val"), Some("3".into()));
    }

    // -----------------------------------------------------------------------
    // SetStyleById
    // -----------------------------------------------------------------------

    #[test]
    fn set_style_adds_css_property() {
        let mut doc = make_doc_with_ids(&["box"]);
        let cmds = vec![DomCommand::SetStyleById {
            id: "box".into(),
            property: "backgroundColor".into(),
            value: "red".into(),
        }];
        apply_commands(&mut doc, &cmds);

        let style = get_attr_by_id(&doc, "box", "style").unwrap();
        assert!(style.contains("background-color:red;"));
    }

    #[test]
    fn set_style_appends_to_existing() {
        let mut doc = Document::new();
        let mut body = Node::new_element("body", HashMap::new());
        let el = Node::new_element("div", HashMap::from([
            ("id".to_string(), "box".to_string()),
            ("style".to_string(), "color:blue".to_string()),
        ]));
        body.children.push(el);
        doc.root.children.push(body);

        apply_commands(&mut doc, &[DomCommand::SetStyleById {
            id: "box".into(),
            property: "marginTop".into(),
            value: "10px".into(),
        }]);

        let style = get_attr_by_id(&doc, "box", "style").unwrap();
        assert!(style.contains("color:blue"));
        assert!(style.contains("margin-top:10px;"));
    }

    // -----------------------------------------------------------------------
    // SetAttribute
    // -----------------------------------------------------------------------

    #[test]
    fn set_attribute_adds_new() {
        let mut doc = make_doc_with_ids(&["btn"]);
        apply_commands(&mut doc, &[DomCommand::SetAttribute {
            id: "btn".into(),
            name: "onclick".into(),
            value: "doClick()".into(),
        }]);

        assert_eq!(
            get_attr_by_id(&doc, "btn", "onclick"),
            Some("doClick()".into())
        );
    }

    #[test]
    fn set_attribute_overwrites_existing() {
        let mut doc = make_doc_with_ids(&["btn"]);
        apply_commands(&mut doc, &[DomCommand::SetAttribute {
            id: "btn".into(),
            name: "onclick".into(),
            value: "first()".into(),
        }]);
        apply_commands(&mut doc, &[DomCommand::SetAttribute {
            id: "btn".into(),
            name: "onclick".into(),
            value: "second()".into(),
        }]);

        assert_eq!(
            get_attr_by_id(&doc, "btn", "onclick"),
            Some("second()".into())
        );
    }

    // -----------------------------------------------------------------------
    // CreateElement
    // -----------------------------------------------------------------------

    #[test]
    fn create_element_appends_child() {
        let mut doc = make_doc_with_ids(&["container"]);
        let initial_count = doc.root.get_element_by_id("container")
            .unwrap().children.len();

        apply_commands(&mut doc, &[DomCommand::CreateElement {
            tag: "span".into(),
            parent_id: "container".into(),
        }]);

        let container = doc.root.get_element_by_id("container").unwrap();
        assert_eq!(container.children.len(), initial_count + 1);
        if let NodeType::Element(ref data) = container.children.last().unwrap().node_type {
            assert_eq!(data.tag_name, "span");
        } else {
            panic!("expected element node");
        }
    }

    // -----------------------------------------------------------------------
    // SetInnerHtmlById
    // -----------------------------------------------------------------------

    #[test]
    fn set_inner_html_replaces_children() {
        let mut doc = make_doc_with_ids(&["target"]);
        apply_commands(&mut doc, &[DomCommand::SetInnerHtmlById {
            id: "target".into(),
            html: "<span>hello</span><span>world</span>".into(),
        }]);

        let target = doc.root.get_element_by_id("target").unwrap();
        // Should have the parsed HTML fragment children
        assert!(target.inner_text().contains("hello"));
        assert!(target.inner_text().contains("world"));
    }

    // -----------------------------------------------------------------------
    // ConsoleLog (should not crash in test environment)
    // -----------------------------------------------------------------------

    #[test]
    fn console_log_does_not_crash() {
        let mut doc = make_doc_with_ids(&[]);
        apply_commands(&mut doc, &[DomCommand::ConsoleLog {
            message: "test log message".into(),
        }]);
        // No panic = pass
    }

    // -----------------------------------------------------------------------
    // Script extraction
    // -----------------------------------------------------------------------

    #[test]
    fn extract_scripts_finds_script_tags() {
        let html = r#"<html><body>
            <div>hello</div>
            <script>var x = 1;</script>
            <div>world</div>
            <script>var y = 2;</script>
        </body></html>"#;
        let doc = crate::html_parser::parse_html(html);
        let scripts = extract_scripts(&doc);
        assert_eq!(scripts.len(), 2);
        assert!(scripts[0].contains("var x = 1"));
        assert!(scripts[1].contains("var y = 2"));
    }

    #[test]
    fn extract_scripts_empty_script_ignored() {
        let html = "<html><body><script>  </script></body></html>";
        let doc = crate::html_parser::parse_html(html);
        let scripts = extract_scripts(&doc);
        assert_eq!(scripts.len(), 0);
    }

    #[test]
    fn extract_scripts_no_scripts() {
        let html = "<html><body><div>hello</div></body></html>";
        let doc = crate::html_parser::parse_html(html);
        let scripts = extract_scripts(&doc);
        assert_eq!(scripts.len(), 0);
    }

    #[test]
    fn extract_scripts_nested_in_div() {
        let html = "<html><body><div><script>alert(1)</script></div></body></html>";
        let doc = crate::html_parser::parse_html(html);
        let scripts = extract_scripts(&doc);
        assert_eq!(scripts.len(), 1);
        assert!(scripts[0].contains("alert(1)"));
    }

    // -----------------------------------------------------------------------
    // remove_script_elements
    // -----------------------------------------------------------------------

    #[test]
    fn remove_scripts_cleans_tree() {
        let html = r#"<html><body>
            <div>keep</div>
            <script>var x = 1;</script>
            <div>also keep</div>
        </body></html>"#;
        let mut doc = crate::html_parser::parse_html(html);

        // Before: scripts exist
        assert!(extract_scripts(&doc).len() > 0);

        remove_script_elements(&mut doc.root);

        // After: no scripts
        assert_eq!(extract_scripts(&doc).len(), 0);
        // But other content preserved
        assert!(doc.root.inner_text().contains("keep"));
    }

    // -----------------------------------------------------------------------
    // DOM persistence simulation (multiple command batches)
    // -----------------------------------------------------------------------

    #[test]
    fn simulate_counter_increment() {
        // Simulates: initial render sets "0", then 3 clicks increment to "3"
        let mut doc = make_doc_with_ids(&["count", "inc", "dec"]);

        // Initial render: script sets count to 0
        apply_commands(&mut doc, &[
            DomCommand::SetTextById { id: "count".into(), text: "0".into() },
            DomCommand::SetAttribute { id: "inc".into(), name: "onclick".into(), value: "__mb_click_inc()".into() },
            DomCommand::SetAttribute { id: "dec".into(), name: "onclick".into(), value: "__mb_click_dec()".into() },
        ]);
        assert_eq!(get_text_by_id(&doc, "count"), Some("0".into()));
        assert_eq!(get_attr_by_id(&doc, "inc", "onclick"), Some("__mb_click_inc()".into()));

        // Click +: count becomes 1
        apply_commands(&mut doc, &[
            DomCommand::SetTextById { id: "count".into(), text: "1".into() },
            DomCommand::SetStyleById { id: "count".into(), property: "color".into(), value: "#66bb6a".into() },
        ]);
        assert_eq!(get_text_by_id(&doc, "count"), Some("1".into()));

        // Click +: count becomes 2
        apply_commands(&mut doc, &[
            DomCommand::SetTextById { id: "count".into(), text: "2".into() },
        ]);
        assert_eq!(get_text_by_id(&doc, "count"), Some("2".into()));

        // Click -: count becomes 1
        apply_commands(&mut doc, &[
            DomCommand::SetTextById { id: "count".into(), text: "1".into() },
        ]);
        assert_eq!(get_text_by_id(&doc, "count"), Some("1".into()));

        // onclick attributes still present after all mutations
        assert_eq!(get_attr_by_id(&doc, "inc", "onclick"), Some("__mb_click_inc()".into()));
        assert_eq!(get_attr_by_id(&doc, "dec", "onclick"), Some("__mb_click_dec()".into()));
    }

    #[test]
    fn simulate_todo_app() {
        // Simulates a todo app: add items via innerHTML, update count text
        let mut doc = make_doc_with_ids(&["list", "count", "add-btn"]);

        // Initial: empty list
        apply_commands(&mut doc, &[
            DomCommand::SetTextById { id: "count".into(), text: "0 items".into() },
            DomCommand::SetAttribute { id: "add-btn".into(), name: "onclick".into(), value: "addItem()".into() },
        ]);
        assert_eq!(get_text_by_id(&doc, "count"), Some("0 items".into()));

        // Add first item
        apply_commands(&mut doc, &[
            DomCommand::SetInnerHtmlById { id: "list".into(), html: "<div>Item 1</div>".into() },
            DomCommand::SetTextById { id: "count".into(), text: "1 item".into() },
        ]);
        assert_eq!(get_text_by_id(&doc, "count"), Some("1 item".into()));
        assert!(doc.root.get_element_by_id("list").unwrap().inner_text().contains("Item 1"));

        // Add second item
        apply_commands(&mut doc, &[
            DomCommand::SetInnerHtmlById {
                id: "list".into(),
                html: "<div>Item 1</div><div>Item 2</div>".into(),
            },
            DomCommand::SetTextById { id: "count".into(), text: "2 items".into() },
        ]);
        assert_eq!(get_text_by_id(&doc, "count"), Some("2 items".into()));
        let list_text = doc.root.get_element_by_id("list").unwrap().inner_text();
        assert!(list_text.contains("Item 1"));
        assert!(list_text.contains("Item 2"));
    }

    #[test]
    fn simulate_style_toggle() {
        // Simulates toggling styles on click
        let mut doc = make_doc_with_ids(&["panel"]);

        // Show panel
        apply_commands(&mut doc, &[DomCommand::SetStyleById {
            id: "panel".into(),
            property: "display".into(),
            value: "block".into(),
        }]);
        let style = get_attr_by_id(&doc, "panel", "style").unwrap();
        assert!(style.contains("display:block;"));

        // Hide panel
        apply_commands(&mut doc, &[DomCommand::SetStyleById {
            id: "panel".into(),
            property: "display".into(),
            value: "none".into(),
        }]);
        let style = get_attr_by_id(&doc, "panel", "style").unwrap();
        // Both values present (CSS resolver takes last)
        assert!(style.contains("display:none;"));
    }

    #[test]
    fn dom_clone_preserves_mutations() {
        // Test that cloning a Document preserves all mutations
        let mut doc = make_doc_with_ids(&["a", "b"]);
        apply_commands(&mut doc, &[
            DomCommand::SetTextById { id: "a".into(), text: "mutated".into() },
            DomCommand::SetAttribute { id: "b".into(), name: "onclick".into(), value: "click()".into() },
        ]);

        // Clone
        let cloned = doc.clone();

        // Verify clone has mutations
        assert_eq!(get_text_by_id(&cloned, "a"), Some("mutated".into()));
        assert_eq!(get_attr_by_id(&cloned, "b", "onclick"), Some("click()".into()));

        // Mutate original further
        apply_commands(&mut doc, &[
            DomCommand::SetTextById { id: "a".into(), text: "changed again".into() },
        ]);

        // Clone should be independent
        assert_eq!(get_text_by_id(&cloned, "a"), Some("mutated".into()));
        assert_eq!(get_text_by_id(&doc, "a"), Some("changed again".into()));
    }

    #[test]
    fn mixed_commands_batch() {
        // Apply a batch of different command types at once
        let mut doc = make_doc_with_ids(&["title", "content", "btn"]);

        apply_commands(&mut doc, &[
            DomCommand::SetTextById { id: "title".into(), text: "Hello World".into() },
            DomCommand::SetStyleById { id: "title".into(), property: "color".into(), value: "#ff0000".into() },
            DomCommand::SetInnerHtmlById { id: "content".into(), html: "<p>paragraph</p>".into() },
            DomCommand::SetAttribute { id: "btn".into(), name: "disabled".into(), value: "true".into() },
            DomCommand::CreateElement { tag: "hr".into(), parent_id: "content".into() },
            DomCommand::ConsoleLog { message: "all done".into() },
        ]);

        assert_eq!(get_text_by_id(&doc, "title"), Some("Hello World".into()));
        assert!(get_attr_by_id(&doc, "title", "style").unwrap().contains("color:#ff0000"));
        assert!(doc.root.get_element_by_id("content").unwrap().inner_text().contains("paragraph"));
        assert_eq!(get_attr_by_id(&doc, "btn", "disabled"), Some("true".into()));
    }
}
