// The DOM on a live document: loaded after dom.js when the engine has one
// (Blitz). Every node is a wrapper around the engine's node id, and each
// property or method asks the engine through __mb_dom(op, argsJson), so
// reads see what the page really holds and changes show at once. Events
// bubble through the real tree. dom.js's timers, fetch and console stay.

function __mb_d(op) {
  var args = Array.prototype.slice.call(arguments, 1);
  return JSON.parse(__mb_dom(op, JSON.stringify(args)));
}

var __mb_nodes = {};
var __mb_listeners = {};
var __mb_inline = {};

function __mb_wrap(nid) {
  if (nid === null || nid === undefined) return null;
  var n = __mb_nodes[nid];
  if (n) return n;
  var t = __mb_d('type', nid);
  n = t === 1 ? new Element(nid) : t === 3 ? new Text(nid) : t === 9 ? document : new Node(nid);
  n.nodeType = t;
  __mb_nodes[nid] = n;
  return n;
}
function __mb_wrapAll(ids) { return (ids || []).map(__mb_wrap); }
function __mb_nid(n) { return n && n._nid !== undefined ? n._nid : null; }

// ---- events ---------------------------------------------------------------

function Event(type, init) {
  init = init || {};
  this.type = String(type);
  this.bubbles = !!init.bubbles;
  this.cancelable = !!init.cancelable;
  this.defaultPrevented = false;
  this.target = this.currentTarget = null;
  this.timeStamp = __mb_now;
  this.eventPhase = 0;
  this._stop = this._stopNow = false;
}
Event.prototype.preventDefault = function () { if (this.cancelable) this.defaultPrevented = true; };
Event.prototype.stopPropagation = function () { this._stop = true; };
Event.prototype.stopImmediatePropagation = function () { this._stop = this._stopNow = true; };
function CustomEvent(type, init) { Event.call(this, type, init); this.detail = init && init.detail !== undefined ? init.detail : null; }
CustomEvent.prototype = Object.create(Event.prototype);
function MouseEvent(type, init) {
  Event.call(this, type, init);
  init = init || {};
  var self = this;
  ['clientX', 'clientY', 'pageX', 'pageY', 'offsetX', 'offsetY', 'screenX', 'screenY', 'button'].forEach(function (k) { self[k] = init[k] || 0; });
}
MouseEvent.prototype = Object.create(Event.prototype);
var PointerEvent = MouseEvent, TouchEvent = MouseEvent, KeyboardEvent = CustomEvent, InputEvent = CustomEvent, FocusEvent = Event;
window.Event = Event; window.CustomEvent = CustomEvent; window.MouseEvent = MouseEvent;

function __mb_listen(key, type, fn, opts) {
  if (typeof fn !== 'function' && !(fn && typeof fn.handleEvent === 'function')) return;
  var m = __mb_listeners[key] || (__mb_listeners[key] = {});
  var l = m[type] || (m[type] = []);
  var capture = typeof opts === 'boolean' ? opts : !!(opts && opts.capture);
  for (var i = 0; i < l.length; i++) if (l[i].fn === fn && l[i].capture === capture) return;
  l.push({ fn: fn, capture: capture, once: !!(opts && opts.once) });
}
function __mb_unlisten(key, type, fn, opts) {
  var l = __mb_listeners[key] && __mb_listeners[key][type];
  if (!l) return;
  var capture = typeof opts === 'boolean' ? opts : !!(opts && opts.capture);
  __mb_listeners[key][type] = l.filter(function (x) { return x.fn !== fn || x.capture !== capture; });
}

// The listeners of `node` for `ev` in one phase, then its on<type> handler.
function __mb_fire(node, ev, capture) {
  var key = node === window ? 'window' : node._nid;
  ev.currentTarget = node;
  var l = (__mb_listeners[key] && __mb_listeners[key][ev.type]) || [];
  l = l.slice();
  for (var i = 0; i < l.length && !ev._stopNow; i++) {
    if (l[i].capture !== capture && node !== ev.target) continue;
    if (l[i].once) __mb_unlisten(key, ev.type, l[i].fn, l[i].capture);
    try {
      if (typeof l[i].fn === 'function') l[i].fn.call(node, ev); else l[i].fn.handleEvent(ev);
    } catch (e) { console.error('Event handler error: ' + e + (e && e.stack ? '\n' + e.stack : '')); }
  }
  if (capture || ev._stopNow) return;
  var h = node['on' + ev.type];
  if (typeof h === 'function') {
    try { if (h.call(node, ev) === false) ev.preventDefault(); } catch (e) { console.error('Event handler error: ' + e); }
  }
}

function __mb_dispatch_on(target, ev) {
  ev.target = target;
  var path = [];
  for (var n = target; n; n = n.parentNode) path.push(n);
  if (path[path.length - 1] === document) path.push(window);
  var prev = window.event;
  window.event = ev;
  for (var i = path.length - 1; i > 0 && !ev._stop; i--) { ev.eventPhase = 1; __mb_fire(path[i], ev, true); }
  if (!ev._stop) { ev.eventPhase = 2; __mb_fire(target, ev, false); }
  if (ev.bubbles) for (var j = 1; j < path.length && !ev._stop; j++) { ev.eventPhase = 3; __mb_fire(path[j], ev, false); }
  window.event = prev;
  ev.currentTarget = null;
  ev.eventPhase = 0;
  return !ev.defaultPrevented;
}

// Called by the engine: a tap on node `nid`. Returns whether anything listened.
function __mb_dispatch(nid, type, init) {
  var t = __mb_wrap(nid);
  if (!t) return false;
  init = init || {};
  init.bubbles = true;
  init.cancelable = true;
  var ev = new MouseEvent(type, init);
  __mb_dispatch_on(t, ev);
  return true;
}

// ---- nodes ----------------------------------------------------------------

function Node(nid) { this._nid = nid; }
Node.ELEMENT_NODE = 1; Node.TEXT_NODE = 3; Node.COMMENT_NODE = 8; Node.DOCUMENT_NODE = 9; Node.DOCUMENT_FRAGMENT_NODE = 11;
function __mb_prop(proto, name, get, set) {
  Object.defineProperty(proto, name, { get: get, set: set || function () {}, configurable: true });
}
__mb_prop(Node.prototype, 'parentNode', function () { return __mb_wrap(__mb_d('parent', this._nid)); });
__mb_prop(Node.prototype, 'parentElement', function () { var p = this.parentNode; return p && p.nodeType === 1 ? p : null; });
__mb_prop(Node.prototype, 'childNodes', function () { return __mb_wrapAll(__mb_d('kids', this._nid)); });
__mb_prop(Node.prototype, 'firstChild', function () { var k = __mb_d('kids', this._nid); return k.length ? __mb_wrap(k[0]) : null; });
__mb_prop(Node.prototype, 'lastChild', function () { var k = __mb_d('kids', this._nid); return k.length ? __mb_wrap(k[k.length - 1]) : null; });
function __mb_sibling(n, step, elementsOnly) {
  var p = __mb_d('parent', n._nid);
  if (p === null) return null;
  var k = __mb_d('kids', p), i = k.indexOf(n._nid);
  for (i += step; i >= 0 && i < k.length; i += step) {
    var s = __mb_wrap(k[i]);
    if (!elementsOnly || s.nodeType === 1) return s;
  }
  return null;
}
__mb_prop(Node.prototype, 'nextSibling', function () { return __mb_sibling(this, 1, false); });
__mb_prop(Node.prototype, 'previousSibling', function () { return __mb_sibling(this, -1, false); });
__mb_prop(Node.prototype, 'nextElementSibling', function () { return __mb_sibling(this, 1, true); });
__mb_prop(Node.prototype, 'previousElementSibling', function () { return __mb_sibling(this, -1, true); });
__mb_prop(Node.prototype, 'textContent',
  function () { return __mb_d('text', this._nid); },
  function (v) { __mb_d('setText', this._nid, v === null || v === undefined ? '' : String(v)); });
__mb_prop(Node.prototype, 'nodeValue', function () { return this.nodeType === 3 ? this.textContent : null; },
  function (v) { if (this.nodeType === 3) this.textContent = v; });
__mb_prop(Node.prototype, 'ownerDocument', function () { return document; });
__mb_prop(Node.prototype, 'isConnected', function () {
  for (var n = this; n; n = n.parentNode) if (n === document) return true;
  return false;
});
__mb_prop(Node.prototype, 'nodeName', function () {
  return this.nodeType === 1 ? this.tagName : this.nodeType === 3 ? '#text' : this.nodeType === 9 ? '#document' : '#node';
});

// A fragment's children move into the parent; a node elsewhere is moved.
function __mb_insert(parent, child, ref) {
  if (child && child._fragment) {
    var kids = child.childNodes;
    for (var i = 0; i < kids.length; i++) __mb_insert(parent, kids[i], ref);
    return child;
  }
  if (typeof child === 'string') child = document.createTextNode(child);
  if (ref) __mb_d('insertBefore', parent._nid, child._nid, ref._nid);
  else __mb_d('append', parent._nid, child._nid);
  return child;
}
Node.prototype.appendChild = function (c) { return __mb_insert(this, c, null); };
Node.prototype.insertBefore = function (c, ref) { return __mb_insert(this, c, ref || null); };
Node.prototype.removeChild = function (c) { __mb_d('remove', c._nid); return c; };
Node.prototype.replaceChild = function (n, old) { __mb_insert(this, n, old); __mb_d('remove', old._nid); return old; };
Node.prototype.remove = function () { __mb_d('remove', this._nid); };
Node.prototype.hasChildNodes = function () { return __mb_d('kids', this._nid).length > 0; };
Node.prototype.contains = function (o) { for (var n = o; n; n = n.parentNode) if (n === this) return true; return false; };
Node.prototype.cloneNode = function (deep) { return __mb_wrap(__mb_d('clone', this._nid, !!deep)); };
Node.prototype.append = function () { for (var i = 0; i < arguments.length; i++) __mb_insert(this, arguments[i], null); };
Node.prototype.prepend = function () {
  var first = this.firstChild;
  for (var i = 0; i < arguments.length; i++) __mb_insert(this, arguments[i], first);
};
Node.prototype.before = function () { var p = this.parentNode; for (var i = 0; i < arguments.length; i++) __mb_insert(p, arguments[i], this); };
Node.prototype.after = function () {
  var p = this.parentNode, next = this.nextSibling;
  for (var i = 0; i < arguments.length; i++) __mb_insert(p, arguments[i], next);
};
Node.prototype.replaceWith = function (n) { this.before(n); this.remove(); };
Node.prototype.addEventListener = function (t, fn, o) { __mb_listen(this._nid, String(t), fn, o); };
Node.prototype.removeEventListener = function (t, fn, o) { __mb_unlisten(this._nid, String(t), fn, o); };
Node.prototype.dispatchEvent = function (ev) { return __mb_dispatch_on(this, ev); };

function Text(nid) { Node.call(this, nid); }
Text.prototype = Object.create(Node.prototype);
__mb_prop(Text.prototype, 'data', function () { return this.textContent; }, function (v) { this.textContent = v; });
__mb_prop(Text.prototype, 'length', function () { return this.textContent.length; });

function Element(nid) { Node.call(this, nid); }
Element.prototype = Object.create(Node.prototype);
var HTMLElement = Element, SVGElement = Element, HTMLCanvasElement = Element, HTMLInputElement = Element;
window.Node = Node; window.Element = Element; window.HTMLElement = Element; window.Text = Text;

__mb_prop(Element.prototype, 'tagName', function () { return __mb_d('tag', this._nid).toUpperCase(); });
__mb_prop(Element.prototype, 'localName', function () { return __mb_d('tag', this._nid); });
Element.prototype.getAttribute = function (k) { return __mb_d('attr', this._nid, String(k)); };
Element.prototype.setAttribute = function (k, v) { __mb_d('setAttr', this._nid, String(k), String(v)); };
Element.prototype.removeAttribute = function (k) { __mb_d('rmAttr', this._nid, String(k)); };
Element.prototype.hasAttribute = function (k) { return __mb_d('attr', this._nid, String(k)) !== null; };
Element.prototype.toggleAttribute = function (k, force) {
  var has = this.hasAttribute(k), want = force === undefined ? !has : !!force;
  if (want && !has) this.setAttribute(k, ''); else if (!want && has) this.removeAttribute(k);
  return want;
};
Element.prototype.setAttributeNS = function (ns, k, v) { this.setAttribute(k, v); };
Element.prototype.getAttributeNS = function (ns, k) { return this.getAttribute(k); };
__mb_prop(Element.prototype, 'attributes', function () {
  return __mb_d('attrs', this._nid).map(function (a) { return { name: a[0], value: a[1], nodeName: a[0], nodeValue: a[1] }; });
});
// Attributes reflected as properties.
['id', 'title', 'href', 'src', 'alt', 'type', 'name', 'placeholder', 'lang', 'dir', 'role', 'tabIndex', 'htmlFor'].forEach(function (p) {
  var a = p === 'htmlFor' ? 'for' : p.toLowerCase();
  __mb_prop(Element.prototype, p, function () { var v = this.getAttribute(a); return v === null ? '' : v; },
    function (v) { this.setAttribute(a, v); });
});
['disabled', 'checked', 'hidden', 'selected', 'readOnly', 'required', 'multiple'].forEach(function (p) {
  var a = p.toLowerCase();
  __mb_prop(Element.prototype, p, function () { return this.hasAttribute(a); },
    function (v) { if (v) this.setAttribute(a, ''); else this.removeAttribute(a); });
});
__mb_prop(Element.prototype, 'value',
  function () { var v = this.getAttribute('value'); return v === null ? (this.localName === 'textarea' ? this.textContent : '') : v; },
  function (v) { this.setAttribute('value', v); if (this.localName === 'textarea') this.textContent = v; });
__mb_prop(Element.prototype, 'className',
  function () { var v = this.getAttribute('class'); return v === null ? '' : v; },
  function (v) { this.setAttribute('class', v); });

function __mb_classes(el) { return el.className.split(/\s+/).filter(function (c) { return c; }); }
__mb_prop(Element.prototype, 'classList', function () {
  var el = this;
  var save = function (l) { el.setAttribute('class', l.join(' ')); };
  return {
    add: function () { var l = __mb_classes(el); for (var i = 0; i < arguments.length; i++) if (l.indexOf(arguments[i]) < 0) l.push(arguments[i]); save(l); },
    remove: function () { var a = Array.prototype.slice.call(arguments); save(__mb_classes(el).filter(function (c) { return a.indexOf(c) < 0; })); },
    toggle: function (c, force) {
      var l = __mb_classes(el), has = l.indexOf(c) >= 0, want = force === undefined ? !has : !!force;
      if (want && !has) l.push(c); else if (!want && has) l = l.filter(function (x) { return x !== c; });
      save(l);
      return want;
    },
    replace: function (a, b) { save(__mb_classes(el).map(function (c) { return c === a ? b : c; })); },
    contains: function (c) { return __mb_classes(el).indexOf(c) >= 0; },
    item: function (i) { return __mb_classes(el)[i] || null; },
    forEach: function (f) { __mb_classes(el).forEach(f); },
    get length() { return __mb_classes(el).length; },
    toString: function () { return el.className; }
  };
});
__mb_prop(Element.prototype, 'dataset', function () {
  var el = this;
  var attr = function (k) { return 'data-' + String(k).replace(/[A-Z]/g, function (c) { return '-' + c.toLowerCase(); }); };
  return new Proxy({}, {
    get: function (t, k) { if (typeof k !== 'string') return undefined; var v = el.getAttribute(attr(k)); return v === null ? undefined : v; },
    set: function (t, k, v) { el.setAttribute(attr(k), v); return true; },
    deleteProperty: function (t, k) { el.removeAttribute(attr(k)); return true; },
    has: function (t, k) { return el.hasAttribute(attr(k)); }
  });
});

// style: properties go to the engine's inline style; reads parse the
// style attribute.
function __mb_kebab(p) { return p === 'cssFloat' ? 'float' : String(p).replace(/[A-Z]/g, function (c) { return '-' + c.toLowerCase(); }); }
function __mb_styleMap(el) {
  var out = {}, s = el.getAttribute('style') || '';
  s.split(';').forEach(function (d) {
    var i = d.indexOf(':');
    if (i > 0) out[d.slice(0, i).trim().toLowerCase()] = d.slice(i + 1).trim();
  });
  return out;
}
__mb_prop(Element.prototype, 'style', function () {
  var el = this;
  var api = {
    setProperty: function (k, v) { __mb_d('setStyle', el._nid, String(k), v === null || v === undefined ? '' : String(v)); },
    removeProperty: function (k) { var old = __mb_styleMap(el)[k] || ''; __mb_d('setStyle', el._nid, String(k), ''); return old; },
    getPropertyValue: function (k) { return __mb_styleMap(el)[String(k)] || ''; }
  };
  return new Proxy(api, {
    get: function (t, k) {
      if (k in t) return t[k];
      if (k === 'cssText') return el.getAttribute('style') || '';
      if (typeof k !== 'string') return undefined;
      return __mb_styleMap(el)[__mb_kebab(k)] || '';
    },
    set: function (t, k, v) {
      if (k === 'cssText') el.setAttribute('style', v);
      else t.setProperty(__mb_kebab(k), v);
      return true;
    }
  });
});

__mb_prop(Element.prototype, 'innerHTML', function () { return __mb_d('html', this._nid); },
  function (v) { __mb_d('setHTML', this._nid, String(v)); });
__mb_prop(Element.prototype, 'outerHTML', function () { return __mb_d('outer', this._nid); });
__mb_prop(Element.prototype, 'innerText', function () { return this.textContent; }, function (v) { this.textContent = v; });
__mb_prop(Element.prototype, 'children', function () { return this.childNodes.filter(function (n) { return n.nodeType === 1; }); });
__mb_prop(Element.prototype, 'childElementCount', function () { return this.children.length; });
__mb_prop(Element.prototype, 'firstElementChild', function () { return this.children[0] || null; });
__mb_prop(Element.prototype, 'lastElementChild', function () { var c = this.children; return c[c.length - 1] || null; });
Element.prototype.querySelector = function (s) { return __mb_wrap(__mb_d('query', this._nid, String(s))); };
Element.prototype.querySelectorAll = function (s) { return __mb_wrapAll(__mb_d('queryAll', this._nid, String(s))); };
Element.prototype.getElementsByTagName = function (t) { return this.querySelectorAll(t); };
Element.prototype.getElementsByClassName = function (c) {
  return this.querySelectorAll(String(c).trim().split(/\s+/).map(function (x) { return '.' + x; }).join(''));
};
Element.prototype.matches = function (s) { return __mb_d('matches', this._nid, String(s)); };
Element.prototype.closest = function (s) {
  for (var n = this; n && n.nodeType === 1; n = n.parentNode) if (n.matches(s)) return n;
  return null;
};
Element.prototype.getBoundingClientRect = function () {
  var r = __mb_d('rect', this._nid) || { x: 0, y: 0, w: 0, h: 0 };
  return { x: r.x, y: r.y, left: r.x, top: r.y, width: r.w, height: r.h, right: r.x + r.w, bottom: r.y + r.h };
};
['offsetWidth', 'clientWidth', 'scrollWidth'].forEach(function (p) {
  __mb_prop(Element.prototype, p, function () { return Math.round(this.getBoundingClientRect().width); });
});
['offsetHeight', 'clientHeight', 'scrollHeight'].forEach(function (p) {
  __mb_prop(Element.prototype, p, function () { return Math.round(this.getBoundingClientRect().height); });
});
__mb_prop(Element.prototype, 'offsetLeft', function () { return Math.round(this.getBoundingClientRect().left); });
__mb_prop(Element.prototype, 'offsetTop', function () { return Math.round(this.getBoundingClientRect().top); });
['scrollTop', 'scrollLeft'].forEach(function (p) { __mb_prop(Element.prototype, p, function () { return 0; }); });
Element.prototype.focus = Element.prototype.blur = Element.prototype.scrollIntoView = function () {};
Element.prototype.click = function () { __mb_dispatch(this._nid, 'click', {}); };
Element.prototype.insertAdjacentHTML = function (where, html) {
  var tmp = document.createElement('div');
  tmp.innerHTML = html;
  var nodes = tmp.childNodes, w = String(where).toLowerCase();
  for (var i = 0; i < nodes.length; i++) {
    if (w === 'beforeend') this.appendChild(nodes[i]);
    else if (w === 'afterbegin') this.insertBefore(nodes[i], this.firstChild);
    else if (w === 'beforebegin') this.parentNode.insertBefore(nodes[i], this);
    else if (w === 'afterend') this.parentNode.insertBefore(nodes[i], this.nextSibling);
  }
};
// Canvas: dom.js's command-recording context, by the element's id.
Element.prototype.getContext = function (type) {
  if (!this._mock) {
    if (!this.id) this.id = '__canvas' + this._nid;
    this._mock = __mb_el(this.id);
  }
  return this._mock.getContext(type);
};
['width', 'height'].forEach(function (k) {
  __mb_prop(Element.prototype, k, function () {
    var v = parseInt(this.getAttribute(k), 10);
    return isNaN(v) ? (this.localName === 'canvas' ? (k === 'width' ? 300 : 150) : Math.round(this.getBoundingClientRect()[k])) : v;
  }, function (v) { this.setAttribute(k, String(v)); });
});
// on<event> properties: kept per node, fired by __mb_fire; inline on<event>
// attributes compile to the same on first use.
['click', 'input', 'change', 'submit', 'pointerdown', 'pointerup', 'mousedown', 'mouseup', 'touchstart', 'touchend', 'keydown', 'keyup', 'load', 'focus', 'blur'].forEach(function (t) {
  __mb_prop(Element.prototype, 'on' + t, function () {
    var key = this._nid + ':' + t, code = this.getAttribute('on' + t);
    var cached = __mb_inline[key];
    // A handler set from script wins; an inline one is recompiled if the attribute changed.
    if (cached && (cached.code === undefined || cached.code === code)) return cached.fn;
    if (!code) return null;
    var fn;
    try { fn = new Function('event', code); } catch (e) { console.error('on' + t + ': ' + e); fn = null; }
    __mb_inline[key] = { fn: fn, code: code };
    return fn;
  }, function (fn) { __mb_inline[this._nid + ':' + t] = { fn: typeof fn === 'function' ? fn : null }; });
});

// ---- document and window --------------------------------------------------

var __mb_docListeners = 'doc';
var document = Object.create(Node.prototype);
document._nid = __mb_d('root');
document.nodeType = 9;
__mb_nodes[document._nid] = document;
document.getElementById = function (id) { return __mb_wrap(__mb_d('byId', String(id))); };
document.querySelector = function (s) { return __mb_wrap(__mb_d('query', null, String(s))); };
document.querySelectorAll = function (s) { return __mb_wrapAll(__mb_d('queryAll', null, String(s))); };
document.getElementsByTagName = function (t) { return document.querySelectorAll(t); };
document.getElementsByClassName = function (c) { return document.documentElement.getElementsByClassName(c); };
document.getElementsByName = function (n) { return document.querySelectorAll('[name="' + n + '"]'); };
document.createElement = function (t) { return __mb_wrap(__mb_d('create', String(t).toLowerCase(), null)); };
document.createElementNS = function (ns, t) { return __mb_wrap(__mb_d('create', String(t), ns ? String(ns) : null)); };
document.createTextNode = function (s) { return __mb_wrap(__mb_d('createText', String(s))); };
document.createComment = function () { return document.createTextNode(''); };
document.createDocumentFragment = function () { var f = document.createElement('div'); f._fragment = true; return f; };
document.createEvent = function () { var e = new CustomEvent(''); e.initEvent = function (t, b, c) { e.type = t; e.bubbles = b; e.cancelable = c; }; return e; };
__mb_prop(document, 'documentElement', function () { return __mb_wrap(__mb_d('query', null, 'html')); });
__mb_prop(document, 'body', function () { return __mb_wrap(__mb_d('query', null, 'body')); });
__mb_prop(document, 'head', function () { return __mb_wrap(__mb_d('query', null, 'head')); });
__mb_prop(document, 'title', function () { var t = document.querySelector('title'); return t ? t.textContent : ''; },
  function (v) { var t = document.querySelector('title'); if (t) t.textContent = v; });
document.readyState = 'loading';
document.defaultView = window;
document.activeElement = null;
document.hasFocus = function () { return true; };
window.document = document;
window.HTMLDocument = Node;

window.addEventListener = function (t, fn, o) { __mb_listen('window', String(t), fn, o); };
window.removeEventListener = function (t, fn, o) { __mb_unlisten('window', String(t), fn, o); };
window.dispatchEvent = function (ev) { ev.target = window; __mb_fire(window, ev, false); return !ev.defaultPrevented; };
var __mb_vp = __mb_d('viewport');
window.innerWidth = window.outerWidth = __mb_vp[0];
window.innerHeight = window.outerHeight = __mb_vp[1];
window.devicePixelRatio = 1;
window.screen = { width: __mb_vp[0], height: __mb_vp[1], availWidth: __mb_vp[0], availHeight: __mb_vp[1] };
window.scrollX = window.scrollY = window.pageXOffset = window.pageYOffset = 0;
window.scrollTo = window.scroll = function () {};
window.location = { href: 'file:///', protocol: 'file:', host: '', hostname: '', pathname: '/', search: '', hash: '', reload: function () {} };
window.navigator = { userAgent: 'MiniBrowser/0.8 (Blitz; QuickJS)', language: 'ru-RU', languages: ['ru-RU', 'en'], platform: 'Linux', onLine: true, maxTouchPoints: 1 };
window.getComputedStyle = function (el) {
  return new Proxy({}, {
    get: function (t, k) {
      if (k === 'getPropertyValue') return function (p) { return __mb_d('computed', el._nid, String(p)) || ''; };
      if (typeof k !== 'string') return undefined;
      return __mb_d('computed', el._nid, __mb_kebab(k)) || '';
    }
  });
};
window.matchMedia = function (q) { return { matches: false, media: q, addListener: function () {}, removeListener: function () {}, addEventListener: function () {}, removeEventListener: function () {} }; };
window.requestIdleCallback = function (fn) { return setTimeout(function () { fn({ timeRemaining: function () { return 10; }, didTimeout: false }); }, 1); };
window.queueMicrotask = function (fn) { Promise.resolve().then(fn); };
window.performance = { now: function () { return __mb_now; } };
window.alert = window.confirm = function (m) { console.log('[alert] ' + m); return true; };

// Called by the engine after the page's scripts ran.
function __mb_loaded() {
  document.readyState = 'interactive';
  __mb_dispatch_on(document, new Event('DOMContentLoaded', { bubbles: true }));
  document.readyState = 'complete';
  __mb_fire(window, (function () { var e = new Event('load'); e.target = document; return e; })(), false);
}

// fetch and XMLHttpRequest for web pages: the engine loads http(s) URLs
// (synchronously, so a slow server holds the page); files as before.
var __mb_file_fetch = fetch;
function __mb_web(url) {
  var r = __mb_d('fetch', String(url));
  return r && !r.local ? r : null;
}
fetch = window.fetch = function (input, init) {
  var url = typeof input === 'object' && input.url ? input.url : String(input);
  var r = __mb_web(url);
  if (!r) return __mb_file_fetch(url, init);
  return new Promise(function (resolve, reject) {
    if (!r.ok && r.status === 0) { reject(new TypeError('fetch failed: ' + url)); return; }
    resolve({
      ok: r.ok, status: r.status, statusText: r.ok ? 'OK' : '', url: r.url, redirected: false,
      headers: { get: function () { return null; }, has: function () { return false; } },
      text: function () { return Promise.resolve(r.text); },
      json: function () { return new Promise(function (ok) { ok(JSON.parse(r.text)); }); },
      clone: function () { return this; }
    });
  });
};
function XMLHttpRequest() {
  this.readyState = 0; this.status = 0; this.responseText = ''; this.response = ''; this.responseType = '';
  this._h = {};
}
XMLHttpRequest.prototype.open = function (method, url) { this._url = String(url); this.readyState = 1; };
XMLHttpRequest.prototype.setRequestHeader = function () {};
XMLHttpRequest.prototype.getResponseHeader = function () { return null; };
XMLHttpRequest.prototype.getAllResponseHeaders = function () { return ''; };
XMLHttpRequest.prototype.addEventListener = function (t, fn) { this['on' + t] = fn; };
XMLHttpRequest.prototype.abort = function () {};
XMLHttpRequest.prototype.send = function () {
  var x = this, url = this._url;
  var done = function (ok, status, text) {
    x.readyState = 4; x.status = status; x.responseText = text;
    x.response = x.responseType === 'json' ? (function () { try { return JSON.parse(text); } catch (e) { return null; } })() : text;
    if (x.onreadystatechange) x.onreadystatechange();
    if (ok && x.onload) x.onload({ target: x });
    if (!ok && x.onerror) x.onerror({ target: x });
    if (x.onloadend) x.onloadend({ target: x });
  };
  // Answers after the current script, as a real request would.
  setTimeout(function () {
    var r = __mb_web(url);
    if (r) { done(r.ok, r.status, r.text); return; }
    var body = typeof __mb_read === 'function' ? __mb_read(url) : null;
    if (body === null || body === undefined) done(false, 404, ''); else done(true, 200, body);
  }, 0);
};
XMLHttpRequest.UNSENT = 0; XMLHttpRequest.OPENED = 1; XMLHttpRequest.DONE = 4;
window.XMLHttpRequest = XMLHttpRequest;
