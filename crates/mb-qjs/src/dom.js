// The DOM a page's scripts see under QuickJS. It records mutations in
// __mb_cmds, which the engine applies to its own DOM tree after each run.
// Elements are looked up by id only; reads return what scripts wrote.
var window = globalThis;
var self = globalThis;
var __mb_cmds = [];
var __mb_elements = {};
var __mb_now = 0;
var __mb_timers = {};
var __mb_timer_seq = 0;
var __mb_storage = {};

function __mb_push(c) { __mb_cmds.push(c); }

function MockElement(id, tag) {
  var el = this;
  this._id = id || '';
  this._text = '';
  this._html = '';
  this._attrs = {};
  this._tag = tag || 'div';
  this._children = [];
  this.style = new Proxy({}, {
    set: function (target, prop, value) {
      target[prop] = value;
      __mb_push({ type: 'SetStyleById', id: el._id, property: String(prop), value: String(value) });
      return true;
    }
  });
}

Object.defineProperty(MockElement.prototype, 'id', {
  get: function () { return this._id; },
  set: function (v) { this._id = String(v); }
});
Object.defineProperty(MockElement.prototype, 'textContent', {
  get: function () { return this._text; },
  set: function (v) {
    this._text = String(v);
    __mb_push({ type: 'SetTextById', id: this._id, text: this._text });
  }
});
Object.defineProperty(MockElement.prototype, 'innerText', Object.getOwnPropertyDescriptor(MockElement.prototype, 'textContent'));
Object.defineProperty(MockElement.prototype, 'innerHTML', {
  get: function () { return this._html || this._text; },
  set: function (v) {
    this._html = this._text = String(v);
    __mb_push({ type: 'SetInnerHtmlById', id: this._id, html: this._html });
  }
});
Object.defineProperty(MockElement.prototype, 'value', {
  get: function () { return this._attrs.value || ''; },
  set: function (v) {
    this._attrs.value = String(v);
    __mb_push({ type: 'SetAttribute', id: this._id, name: 'value', value: String(v) });
  }
});
Object.defineProperty(MockElement.prototype, 'onclick', {
  get: function () { return window['__mb_click_' + this._id.replace(/[^a-zA-Z0-9]/g, '_')] || null; },
  set: function (fn) {
    if (typeof fn !== 'function') return;
    var name = '__mb_click_' + this._id.replace(/[^a-zA-Z0-9]/g, '_');
    window[name] = fn;
    __mb_push({ type: 'SetAttribute', id: this._id, name: 'onclick', value: name + '.call(document.getElementById(' + JSON.stringify(this._id) + '), window.event)' });
  }
});
Object.defineProperty(MockElement.prototype, 'className', {
  get: function () { return this._attrs['class'] || ''; },
  set: function (v) { this.setAttribute('class', String(v)); }
});
Object.defineProperty(MockElement.prototype, 'classList', {
  get: function () {
    var el = this;
    var list = function () { return (el._attrs['class'] || '').split(/\s+/).filter(function (c) { return c; }); };
    var save = function (l) { el.setAttribute('class', l.join(' ')); };
    return {
      add: function () { var l = list(); for (var i = 0; i < arguments.length; i++) if (l.indexOf(arguments[i]) < 0) l.push(arguments[i]); save(l); },
      remove: function () { var a = Array.prototype.slice.call(arguments); save(list().filter(function (c) { return a.indexOf(c) < 0; })); },
      toggle: function (c, force) {
        var l = list(), has = l.indexOf(c) >= 0, want = force === undefined ? !has : !!force;
        if (want && !has) l.push(c); else if (!want && has) l = l.filter(function (x) { return x !== c; });
        save(l);
        return want;
      },
      contains: function (c) { return list().indexOf(c) >= 0; },
      get length() { return list().length; }
    };
  }
});
MockElement.prototype.addEventListener = function (event, fn) {
  if (event === 'click') this.onclick = fn;
};
MockElement.prototype.removeEventListener = function () {};
MockElement.prototype.setAttribute = function (name, value) {
  this._attrs[name] = String(value);
  __mb_push({ type: 'SetAttribute', id: this._id, name: String(name), value: String(value) });
};
MockElement.prototype.getAttribute = function (name) {
  return name in this._attrs ? this._attrs[name] : null;
};
MockElement.prototype.appendChild = function (child) {
  this._children.push(child);
  __mb_push({ type: 'CreateElement', tag: child._tag || 'div', parent_id: this._id });
  return child;
};
MockElement.prototype.removeChild = function (child) { return child; };
MockElement.prototype.remove = function () {};
['width', 'height'].forEach(function (k) {
  Object.defineProperty(MockElement.prototype, k, {
    get: function () { var v = parseInt(this._attrs[k], 10); return isNaN(v) ? (k === 'width' ? 300 : 150) : v; },
    set: function (v) { this.setAttribute(k, String(v)); }
  });
});

// Canvas 2D: each call becomes a 'Canvas' command the engine draws; the
// context keeps the state (styles, font) that calls carry along.
MockElement.prototype.getContext = function (type) {
  if (type !== '2d') return null;
  if (this._ctx) return this._ctx;
  var id = this._id, el = this;
  var ctx = {
    canvas: el,
    fillStyle: '#000000', strokeStyle: '#000000', lineWidth: 1, font: '10px sans-serif',
    globalAlpha: 1, textAlign: 'start', textBaseline: 'alphabetic', lineCap: 'butt', lineJoin: 'miter'
  };
  function op(name, args, extra) {
    var c = { type: 'Canvas', id: id, op: name, args: args || [], alpha: ctx.globalAlpha };
    if (extra) for (var k in extra) c[k] = extra[k];
    __mb_push(c);
  }
  function nums(a) { return Array.prototype.slice.call(a).map(Number); }
  ['beginPath', 'closePath', 'save', 'restore', 'resetTransform'].forEach(function (n) {
    ctx[n] = function () { op(n); };
  });
  ['moveTo', 'lineTo', 'rect', 'quadraticCurveTo', 'bezierCurveTo', 'translate', 'rotate', 'scale', 'setTransform', 'clearRect'].forEach(function (n) {
    ctx[n] = function () { op(n, nums(arguments)); };
  });
  ctx.arc = function (x, y, r, a0, a1, ccw) { op('arc', [x, y, r, a0, a1, ccw ? 1 : 0]); };
  ctx.fillRect = function (x, y, w, h) { op('fillRect', [x, y, w, h], { style: String(ctx.fillStyle) }); };
  ctx.strokeRect = function (x, y, w, h) { op('strokeRect', [x, y, w, h], { style: String(ctx.strokeStyle), lineWidth: ctx.lineWidth }); };
  ctx.fill = function () { op('fill', [], { style: String(ctx.fillStyle) }); };
  ctx.stroke = function () { op('stroke', [], { style: String(ctx.strokeStyle), lineWidth: ctx.lineWidth }); };
  ctx.fillText = function (text, x, y) {
    op('fillText', [x, y], { text: String(text), style: String(ctx.fillStyle), font: String(ctx.font), align: ctx.textAlign });
  };
  ctx.strokeText = ctx.fillText;
  ctx.measureText = function (text) {
    var m = /(\d+(?:\.\d+)?)px/.exec(ctx.font), size = m ? Number(m[1]) : 10;
    return { width: String(text).length * size * 0.55 };
  };
  ctx.createImageData = function (w, h) {
    if (typeof w === 'object') { h = w.height; w = w.width; }
    return { width: w, height: h, data: new Uint8ClampedArray(w * h * 4) };
  };
  // Pixels are not read back from the engine: a fresh (transparent) image.
  ctx.getImageData = function (x, y, w, h) { return ctx.createImageData(w, h); };
  ctx.putImageData = function (img, x, y) {
    // Native hosts take the bytes directly (__mb_blob); others as a JSON array.
    var d = img.data;
    var extra = typeof __mb_blob === 'function' && d.buffer && d.byteOffset === 0 && d.byteLength === d.buffer.byteLength
      ? { blob: __mb_blob(d.buffer) } : { data: Array.prototype.slice.call(d) };
    op('putImageData', [x, y, img.width, img.height], extra);
  };
  ctx.setLineDash = function () {};
  ctx.drawImage = function () {};
  ctx.createLinearGradient = ctx.createRadialGradient = function () { return { addColorStop: function () {} }; };
  this._ctx = ctx;
  return ctx;
};

// Called by the engine with the document's elements that have an id: their
// current text and attributes, so scripts can read them.
function __mb_seed(list) {
  for (var i = 0; i < list.length; i++) {
    var e = list[i], el = __mb_el(e.id);
    el._text = e.text;
    el._attrs = e.attrs || {};
  }
}

function __mb_el(id) {
  if (!__mb_elements[id]) __mb_elements[id] = new MockElement(id);
  return __mb_elements[id];
}

var document = {
  getElementById: function (id) { return __mb_el(String(id)); },
  querySelector: function (sel) {
    return sel && sel[0] === '#' ? __mb_el(sel.slice(1)) : new MockElement('__qs_' + sel);
  },
  querySelectorAll: function () { return []; },
  createElement: function (tag) { return new MockElement('', String(tag)); },
  addEventListener: function () {},
  body: new MockElement('__body', 'body'),
  documentElement: new MockElement('__html', 'html')
};
window.document = document;
window.addEventListener = function () {};

var console = {};
console.log = console.info = console.warn = console.error = console.debug = function () {
  var msg = Array.prototype.slice.call(arguments).map(function (a) {
    return typeof a === 'object' ? JSON.stringify(a) : String(a);
  }).join(' ');
  __mb_push({ type: 'ConsoleLog', message: msg });
};
window.console = console;

var localStorage = {
  getItem: function (k) { return k in __mb_storage ? __mb_storage[k] : null; },
  setItem: function (k, v) { __mb_storage[k] = String(v); },
  removeItem: function (k) { delete __mb_storage[k]; },
  clear: function () { __mb_storage = {}; },
  key: function (i) { return Object.keys(__mb_storage)[i] || null; },
  get length() { return Object.keys(__mb_storage).length; }
};
window.localStorage = localStorage;
var sessionStorage = localStorage;
window.sessionStorage = sessionStorage;

function __mb_timer(fn, ms, repeat) {
  if (typeof fn !== 'function') return 0;
  ms = Math.max(0, Number(ms) || 0);
  var id = ++__mb_timer_seq;
  __mb_timers[id] = { fn: fn, ms: repeat ? Math.max(ms, 10) : ms, due: __mb_now + ms, repeat: repeat };
  return id;
}
var setTimeout = function (fn, ms) { return __mb_timer(fn, ms, false); };
var setInterval = function (fn, ms) { return __mb_timer(fn, ms, true); };
var clearTimeout = function (id) { delete __mb_timers[id]; };
var clearInterval = clearTimeout;
var requestAnimationFrame = function (fn) { return __mb_timer(function () { fn(__mb_now); }, 16, false); };
var cancelAnimationFrame = clearTimeout;

// Called by the engine: runs every timer due by `now`, oldest first.
function __mb_tick(now) {
  __mb_now = now;
  var due = Object.keys(__mb_timers).map(Number).filter(function (id) {
    return __mb_timers[id].due <= now;
  }).sort(function (a, b) { return __mb_timers[a].due - __mb_timers[b].due; });
  for (var i = 0; i < due.length; i++) {
    var t = __mb_timers[due[i]];
    if (!t) continue;
    if (t.repeat) t.due = Math.max(t.due + t.ms, now); else delete __mb_timers[due[i]];
    try { t.fn(); } catch (e) { console.error('Timer error: ' + e); }
  }
}

function __mb_next(now) {
  var best = -1;
  for (var id in __mb_timers) {
    var d = Math.max(0, __mb_timers[id].due - now);
    if (best < 0 || d < best) best = d;
  }
  return best;
}

// fetch() for the page's own files (the engine decides which may be read).
// Resolves on the same turn; there is no network.
function fetch(url) {
  return new Promise(function (resolve, reject) {
    url = String(url);
    var body = typeof __mb_read === 'function' ? __mb_read(url) : null;
    // Binary files are not text: they are only readable as bytes.
    var bytes = body === null || body === undefined ? (typeof __mb_read_bytes === 'function' ? __mb_read_bytes(url) : null) : null;
    if ((body === null || body === undefined) && !bytes) {
      reject(new TypeError('fetch failed: ' + url));
      return;
    }
    resolve({
      ok: true, status: 200, url: url,
      text: function () { return Promise.resolve(body || ''); },
      json: function () { return new Promise(function (ok) { ok(JSON.parse(body)); }); },
      arrayBuffer: function () { return Promise.resolve(bytes || __mb_read_bytes(url)); }
    });
  });
}
// ---- the web (the host's feature `net`): requests and sockets on threads,
// their results delivered on a later tick through __mb_net_deliver.

var __mb_base = null;
var __mb_net_pending = {};
var __mb_sockets = {};

// An absolute http(s)/ws(s) URL for `url`, against the page's own web
// address; null for the page's files.
function __mb_abs(url) {
  url = String(url);
  if (/^[a-z][a-z0-9+.-]*:/i.test(url)) return /^(https?|wss?):/i.test(url) ? url : null;
  var base = window.__mb_base;
  if (!base || !/^https?:/i.test(base)) return null;
  var m = /^([a-z]+:\/\/[^\/]+)(\/[^?#]*)?/i.exec(base);
  if (!m) return null;
  if (url.slice(0, 2) === '//') return base.split(':')[0] + ':' + url;
  if (url[0] === '/') return m[1] + url;
  var dir = (m[2] || '/').replace(/[^\/]*$/, '');
  return m[1] + dir + url;
}

function __mb_response(e, url) {
  var body = e.body || '';
  return {
    ok: e.status >= 200 && e.status < 300, status: e.status, statusText: '', url: url, redirected: false,
    headers: { get: function () { return null; }, has: function () { return false; }, forEach: function () {} },
    text: function () { return Promise.resolve(body); },
    json: function () { return new Promise(function (ok) { ok(JSON.parse(body)); }); },
    arrayBuffer: function () {
      var b = new Uint8Array(body.length);
      for (var i = 0; i < body.length; i++) b[i] = body.charCodeAt(i) & 255;
      return Promise.resolve(b.buffer);
    },
    clone: function () { return __mb_response(e, url); }
  };
}

function __mb_net_deliver(list) {
  for (var i = 0; i < list.length; i++) {
    var e = list[i];
    if (e.t === 'done') {
      var cb = __mb_net_pending[e.id];
      delete __mb_net_pending[e.id];
      if (cb) try { cb(e); } catch (x) { console.error('fetch callback: ' + x); }
    } else {
      var s = __mb_sockets[e.id];
      if (s) s._event(e.kind, e.data);
    }
  }
}

function __mb_request(method, url, body, headers, done) {
  var h = [];
  if (headers) {
    if (typeof headers.forEach === 'function' && !Array.isArray(headers)) headers.forEach(function (v, k) { h.push([k, String(v)]); });
    else for (var k in headers) h.push([k, String(headers[k])]);
  }
  var id = __mb_http(String(method || 'GET').toUpperCase(), url, body === undefined || body === null ? null : String(body), JSON.stringify(h));
  __mb_net_pending[id] = done;
}

var __mb_file_fetch_impl = fetch;
fetch = function (input, init) {
  var url = typeof input === 'object' && input && input.url ? input.url : String(input);
  var abs = typeof __mb_http === 'function' ? __mb_abs(url) : null;
  if (!abs) return __mb_file_fetch_impl(url, init);
  init = init || {};
  return new Promise(function (resolve, reject) {
    __mb_request(init.method, abs, init.body, init.headers, function (e) {
      if (e.status === 0) reject(new TypeError('fetch failed: ' + abs + (e.error ? ' (' + e.error + ')' : '')));
      else resolve(__mb_response(e, abs));
    });
  });
};
window.fetch = fetch;

function XMLHttpRequest() {
  this.readyState = 0; this.status = 0; this.responseText = ''; this.response = ''; this.responseType = '';
  this._headers = {};
}
XMLHttpRequest.prototype.open = function (method, url) { this._method = method; this._url = String(url); this.readyState = 1; };
XMLHttpRequest.prototype.setRequestHeader = function (k, v) { this._headers[k] = v; };
XMLHttpRequest.prototype.getResponseHeader = function () { return null; };
XMLHttpRequest.prototype.getAllResponseHeaders = function () { return ''; };
XMLHttpRequest.prototype.addEventListener = function (t, fn) { this['on' + t] = fn; };
XMLHttpRequest.prototype.removeEventListener = function () {};
XMLHttpRequest.prototype.abort = function () { this._aborted = true; };
XMLHttpRequest.prototype.send = function (body) {
  var x = this;
  var done = function (ok, status, text) {
    if (x._aborted) return;
    x.readyState = 4; x.status = status; x.responseText = text;
    x.response = x.responseType === 'json' ? (function () { try { return JSON.parse(text); } catch (e) { return null; } })() : text;
    var ev = { target: x, type: ok ? 'load' : 'error' };
    if (x.onreadystatechange) x.onreadystatechange(ev);
    if (ok && x.onload) x.onload(ev);
    if (!ok && x.onerror) x.onerror(ev);
    if (x.onloadend) x.onloadend(ev);
  };
  var abs = typeof __mb_http === 'function' ? __mb_abs(this._url) : null;
  if (abs) {
    __mb_request(this._method, abs, body, this._headers, function (e) { done(e.status !== 0, e.status, e.body || ''); });
    return;
  }
  var url = this._url;
  setTimeout(function () {
    var text = typeof __mb_read === 'function' ? __mb_read(url) : null;
    if (text === null || text === undefined) done(false, 404, ''); else done(true, 200, text);
  }, 0);
};
XMLHttpRequest.UNSENT = 0; XMLHttpRequest.OPENED = 1; XMLHttpRequest.HEADERS_RECEIVED = 2; XMLHttpRequest.LOADING = 3; XMLHttpRequest.DONE = 4;
window.XMLHttpRequest = XMLHttpRequest;

// WebSocket (ws:// with the host's feature `net`).
function WebSocket(url) {
  this.url = String(url);
  this.readyState = 0;
  this.protocol = '';
  this.bufferedAmount = 0;
  this._l = {};
  var abs = __mb_abs(this.url);
  if (typeof __mb_ws_open !== 'function' || !abs) {
    var me = this;
    setTimeout(function () { me._event('error', 'no WebSocket support'); me._event('close', ''); }, 0);
    return;
  }
  this._id = __mb_ws_open(abs);
  __mb_sockets[this._id] = this;
}
WebSocket.CONNECTING = 0; WebSocket.OPEN = 1; WebSocket.CLOSING = 2; WebSocket.CLOSED = 3;
WebSocket.prototype.send = function (data) { if (this.readyState === 1) __mb_ws_send(this._id, String(data)); };
WebSocket.prototype.close = function () {
  if (this.readyState >= 2) return;
  this.readyState = 2;
  if (this._id) __mb_ws_close(this._id);
};
WebSocket.prototype.addEventListener = function (t, fn) { (this._l[t] = this._l[t] || []).push(fn); };
WebSocket.prototype.removeEventListener = function (t, fn) { this._l[t] = (this._l[t] || []).filter(function (f) { return f !== fn; }); };
WebSocket.prototype._event = function (kind, data) {
  if (kind === 'open') this.readyState = 1;
  if (kind === 'close') { this.readyState = 3; delete __mb_sockets[this._id]; }
  var ev = { type: kind, target: this, data: data, code: 1000, reason: '', wasClean: kind === 'close' };
  if (kind === 'error') ev.message = data;
  var h = this['on' + kind];
  try { if (typeof h === 'function') h.call(this, ev); } catch (e) { console.error('WebSocket ' + kind + ': ' + e); }
  (this._l[kind] || []).slice().forEach(function (f) { try { f.call(this, ev); } catch (e) { console.error('WebSocket ' + kind + ': ' + e); } }, this);
};
window.WebSocket = WebSocket;

// <script src>: the file's code runs at global scope, like an inline script.
function __mb_include(src) {
  var code = typeof __mb_read === 'function' ? __mb_read(String(src)) : null;
  if (code === null || code === undefined) throw new Error('script not found: ' + src);
  (0, eval)(code);
}
window.fetch = fetch;

function __mb_take() {
  var out = JSON.stringify(__mb_cmds);
  __mb_cmds = [];
  return out;
}
