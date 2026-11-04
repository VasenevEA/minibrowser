//! [`JsHost`] on the browser's own JavaScript engine, through `js_sys::eval`.
//!
//! rquickjs does not compile to wasm32-unknown-unknown, so in the browser a
//! mock DOM API is patched over the real one for the duration of each run and
//! the mutations it records come back as JSON.

use mb_core::JsHost;

pub struct BrowserJs;

impl BrowserJs {
    fn eval_string(code: &str) -> Result<String, String> {
        match js_sys::eval(code) {
            Ok(v) => v.as_string().ok_or_else(|| "eval returned non-string".to_string()),
            Err(e) => Err(e.as_string().unwrap_or_else(|| format!("{:?}", e))),
        }
    }
}

impl JsHost for BrowserJs {
    fn reset(&mut self) {
        let _ = js_sys::eval(
            "window.__mb_init = false; window.__mb_elements = {}; \
             window.__mb_cmds = []; window.__mb_handlers = {}; window.__mb_intervals = [];"
        );
    }

    fn run(&mut self, code: &str, _now_ms: f64) -> Result<String, String> {
        Self::eval_string(&wrap_script(code))
    }

    /// Runs every registered interval: the page's JS drives ticks (`render_html_live`).
    fn tick(&mut self, _now_ms: f64) -> Result<String, String> {
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
        Self::eval_string(tick_code)
    }

    fn next_timer(&self, _now_ms: f64) -> Option<f64> {
        None
    }

    fn set_input_value(&mut self, id: &str, value: &str) {
        let id = serde_json::to_string(id).unwrap_or_default();
        let value = serde_json::to_string(value).unwrap_or_default();
        let code = format!(
            "(function() {{ if (!window.__mb_elements || !window.MockElement) return; \
             var el = window.__mb_elements[{id}] || (window.__mb_elements[{id}] = new MockElement({id})); \
             el._attrs['value'] = {value}; el._text = {value}; }})()"
        );
        let _ = js_sys::eval(&code);
    }
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

