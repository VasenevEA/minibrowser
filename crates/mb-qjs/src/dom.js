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
    op('putImageData', [x, y, img.width, img.height], { data: Array.prototype.slice.call(img.data) });
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
    var body = typeof __mb_read === 'function' ? __mb_read(String(url)) : null;
    if (body === null || body === undefined) {
      reject(new TypeError('fetch failed: ' + url));
      return;
    }
    resolve({
      ok: true, status: 200, url: String(url),
      text: function () { return Promise.resolve(body); },
      json: function () { return new Promise(function (ok) { ok(JSON.parse(body)); }); }
    });
  });
}
window.fetch = fetch;

function __mb_take() {
  var out = JSON.stringify(__mb_cmds);
  __mb_cmds = [];
  return out;
}
