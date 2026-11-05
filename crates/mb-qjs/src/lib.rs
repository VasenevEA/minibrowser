//! [`JsHost`] on QuickJS: real JavaScript for native builds (the panel, headless).
//!
//! Each page gets a fresh runtime with the DOM glue from `dom.js`. Page code runs
//! at global scope, so `var`s and functions persist between scripts, handlers and
//! timer callbacks, as in a browser.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[cfg(feature = "net")]
mod net;

use mb_core::js_engine::SharedDom;
use mb_core::JsHost;
use rquickjs::{CatchResultExt, Context, Function, Runtime};

const DOM_JS: &str = include_str!("dom.js");
/// The DOM on a live document (`set_dom`), over dom.js's by-id elements.
const DOM_LIVE_JS: &str = include_str!("dom-live.js");

pub struct QuickJs {
    rt: Runtime,
    ctx: Context,
    /// Where `fetch` finds relative URLs (the page's directory); None: no files.
    base: Option<PathBuf>,
    /// Binary data scripts hand over (putImageData pixels), by handle - 1.
    blobs: Blobs,
    /// The live document, if the engine has one.
    dom: Option<SharedDom>,
    /// Requests and sockets scripts started (feature `net`).
    #[cfg(feature = "net")]
    net: Rc<RefCell<net::Net>>,
}

type Blobs = Rc<RefCell<Vec<Vec<u8>>>>;

impl QuickJs {
    pub fn new() -> Self {
        let blobs = Blobs::default();
        let host = Host::default();
        let (rt, ctx) = fresh(None, blobs.clone(), None, &host);
        QuickJs { rt, ctx, base: None, blobs, dom: None, #[cfg(feature = "net")] net: host.net }
    }

    /// A host whose `fetch` reads files: relative URLs from `base` (the page's
    /// directory), absolute paths under `base` or `/tmp` (where the panel's data
    /// collector drops JSON).
    pub fn with_files(base: &Path) -> Self {
        let base = Some(base.to_path_buf());
        let blobs = Blobs::default();
        let host = Host::default();
        let (rt, ctx) = fresh(base.clone(), blobs.clone(), None, &host);
        QuickJs { rt, ctx, base, blobs, dom: None, #[cfg(feature = "net")] net: host.net }
    }

    /// Evaluates `code` and returns the commands it queued. If it threw, the commands
    /// queued before the throw are kept, as a browser keeps their effects, and the
    /// exception is logged.
    fn eval_take(&mut self, code: &str, now: f64) -> Result<String, String> {
        // The engine took the last run's blobs while applying its commands.
        self.blobs.borrow_mut().clear();
        self.ctx.with(|ctx| {
            let _ = ctx.globals().set("__mb_now", now);
            if let Err(e) = ctx.eval::<(), _>(code).catch(&ctx) {
                log::warn!("JS error: {}", e);
            }
        });
        // Promise reactions (fetch().then) run before the commands are collected.
        for _ in 0..1000 {
            match self.rt.execute_pending_job() {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => log::warn!("JS job error: {:?}", e),
            }
        }
        self.ctx.with(|ctx| ctx.eval::<String, _>("__mb_take()").catch(&ctx).map_err(|e| e.to_string()))
    }
}

impl Default for QuickJs {
    fn default() -> Self { Self::new() }
}

/// The file a `fetch` URL names, if it may be read: relative to `base`, or an
/// absolute path inside `base` or /tmp, without `..` escaping either.
fn resolve(base: &Path, url: &str) -> Option<PathBuf> {
    let url = url.strip_prefix("file://").unwrap_or(url);
    if url.contains("://") {
        return None;
    }
    let path = if url.starts_with('/') { PathBuf::from(url) } else { base.join(url) };
    let real = path.canonicalize().ok()?;
    let allowed = [base.canonicalize().ok(), Path::new("/tmp").canonicalize().ok()];
    allowed.iter().flatten().any(|root| real.starts_with(root)).then_some(real)
}

/// Where `import` finds modules: a module name is a path relative to the
/// page's directory (`lib/a.js`) or an http(s) URL.
struct Modules {
    base: Option<PathBuf>,
}

/// `name` imported from module `from`: the importing module's directory joined
/// with it, `..` folded; URLs as URLs.
fn join_module(from: &str, name: &str) -> String {
    if name.contains("://") {
        return name.to_string();
    }
    if let Some(i) = from.find("://") {
        // Against a URL: the origin for "/x", the directory otherwise.
        let rest = &from[i + 3..];
        let origin_end = i + 3 + rest.find('/').unwrap_or(rest.len());
        let origin = &from[..origin_end];
        if name.starts_with('/') {
            return format!("{origin}{name}");
        }
        let dir = &from[..from.rfind('/').filter(|&j| j >= origin_end).map_or(from.len(), |j| j + 1)];
        let dir = if dir.ends_with('/') { dir.to_string() } else { format!("{dir}/") };
        return fold(&format!("{dir}{name}"));
    }
    let dir = Path::new(from).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let joined = if name.starts_with('/') { name.trim_start_matches('/').to_string() } else if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") };
    fold(&joined)
}

/// Removes `.` and folds `..` segments (keeping a URL's scheme and host).
fn fold(path: &str) -> String {
    let (head, rest) = match path.find("://") {
        Some(i) => {
            let after = &path[i + 3..];
            let host_end = i + 3 + after.find('/').unwrap_or(after.len());
            (&path[..host_end], &path[host_end..])
        }
        None => ("", path),
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in rest.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    if head.is_empty() { parts.join("/") } else { format!("{head}/{}", parts.join("/")) }
}

impl rquickjs::loader::Resolver for Modules {
    fn resolve<'js>(&mut self, _ctx: &rquickjs::Ctx<'js>, base: &str, name: &str) -> rquickjs::Result<String> {
        if !(name.starts_with("./") || name.starts_with("../") || name.starts_with('/') || name.contains("://")) {
            return Err(rquickjs::Error::new_resolving_message(base, name, "bare module names need an import map (not supported)"));
        }
        Ok(join_module(base, name))
    }
}

impl rquickjs::loader::Loader for Modules {
    fn load<'js>(&mut self, ctx: &rquickjs::Ctx<'js>, name: &str) -> rquickjs::Result<rquickjs::Module<'js, rquickjs::module::Declared>> {
        let code = if name.contains("://") {
            fetch_module(name)
        } else {
            self.base.as_deref().and_then(|b| resolve(b, name)).and_then(|p| std::fs::read_to_string(p).ok())
        };
        let code = code.ok_or_else(|| rquickjs::Error::new_loading(name))?;
        rquickjs::Module::declare(ctx.clone(), name, code)
    }
}

#[cfg(feature = "net")]
fn fetch_module(url: &str) -> Option<String> {
    net::get_text(url)
}

#[cfg(not(feature = "net"))]
fn fetch_module(_url: &str) -> Option<String> {
    None
}

/// Pins down the closure's lifetimes: its value lives in the context it is given.
fn bytes_fn<F>(f: F) -> F
where
    F: for<'js> Fn(rquickjs::Ctx<'js>, String) -> rquickjs::Result<rquickjs::Value<'js>>,
{
    f
}

/// What a fresh context's natives share with the host.
#[derive(Default, Clone)]
struct Host {
    #[cfg(feature = "net")]
    net: Rc<RefCell<net::Net>>,
}

fn fresh(base: Option<PathBuf>, blobs: Blobs, dom: Option<SharedDom>, host: &Host) -> (Runtime, Context) {
    let _ = host;
    let rt = Runtime::new().expect("QuickJS runtime");
    // A page must not take the panel down with it.
    // Big enough for a game compiled to JS (Doom: ~40 MB with its heap).
    rt.set_memory_limit(64 << 20);
    rt.set_max_stack_size(512 << 10);
    rt.set_loader(Modules { base: base.clone() }, Modules { base: base.clone() });
    let ctx = Context::full(&rt).expect("QuickJS context");
    let base_bytes = base.clone();
    ctx.with(|ctx| {
        let read = Function::new(ctx.clone(), move |url: String| -> Option<String> {
            let path = resolve(base.as_deref()?, &url)?;
            std::fs::read_to_string(path).ok()
        });
        if let Ok(read) = read {
            let _ = ctx.globals().set("__mb_read", read);
        }
        let bytes_base = base_bytes;
        let read_bytes = Function::new(ctx.clone(), bytes_fn(move |ctx, url| {
            match bytes_base.as_deref().and_then(|b| resolve(b, &url)).and_then(|p| std::fs::read(p).ok()) {
                Some(data) => Ok(rquickjs::ArrayBuffer::new(ctx, data)?.into_value()),
                None => Ok(rquickjs::Value::new_null(ctx)),
            }
        }));
        if let Ok(f) = read_bytes {
            let _ = ctx.globals().set("__mb_read_bytes", f);
        }
        // __mb_blob(arrayBuffer) → a handle: bytes go to the engine without JSON.
        let blob = Function::new(ctx.clone(), move |buf: rquickjs::ArrayBuffer| -> u32 {
            let mut b = blobs.borrow_mut();
            b.push(buf.as_bytes().map(<[u8]>::to_vec).unwrap_or_default());
            b.len() as u32
        });
        if let Ok(blob) = blob {
            let _ = ctx.globals().set("__mb_blob", blob);
        }
        #[cfg(feature = "net")]
        {
            let net = host.net.clone();
            let f = Function::new(ctx.clone(), move |method: String, url: String, body: Option<String>, headers: String| -> u32 {
                let headers: Vec<(String, String)> = serde_json::from_str(&headers).unwrap_or_default();
                net.borrow_mut().request(method, url, body, headers)
            });
            if let Ok(f) = f {
                let _ = ctx.globals().set("__mb_http", f);
            }
            let net = host.net.clone();
            if let Ok(f) = Function::new(ctx.clone(), move |url: String| -> u32 { net.borrow_mut().ws_open(url) }) {
                let _ = ctx.globals().set("__mb_ws_open", f);
            }
            let net = host.net.clone();
            if let Ok(f) = Function::new(ctx.clone(), move |id: u32, text: String| net.borrow_mut().ws_send(id, text)) {
                let _ = ctx.globals().set("__mb_ws_send", f);
            }
            let net = host.net.clone();
            if let Ok(f) = Function::new(ctx.clone(), move |id: u32| net.borrow_mut().ws_close(id)) {
                let _ = ctx.globals().set("__mb_ws_close", f);
            }
        }
        ctx.eval::<(), _>(DOM_JS).catch(&ctx).map_err(|e| e.to_string()).expect("dom.js");
        if let Some(dom) = dom {
            // __mb_dom(op, argsJson) → resultJson: one call into the live document.
            let call = Function::new(ctx.clone(), move |op: String, args: String| -> String {
                let args: Vec<serde_json::Value> = serde_json::from_str(&args).unwrap_or_default();
                // A script running inside a DOM call (none do) would find it borrowed.
                match dom.try_borrow_mut() {
                    Ok(mut d) => d.call(&op, &args).to_string(),
                    Err(_) => "null".into(),
                }
            });
            if let Ok(call) = call {
                let _ = ctx.globals().set("__mb_dom", call);
            }
            ctx.eval::<(), _>(DOM_LIVE_JS).catch(&ctx).map_err(|e| e.to_string()).expect("dom-live.js");
        }
    });
    (rt, ctx)
}

impl JsHost for QuickJs {
    fn reset(&mut self) {
        self.blobs.borrow_mut().clear();
        #[cfg(feature = "net")]
        self.net.borrow_mut().reset();
        let host = Host {
            #[cfg(feature = "net")]
            net: self.net.clone(),
        };
        let (rt, ctx) = fresh(self.base.clone(), self.blobs.clone(), self.dom.clone(), &host);
        // Drop the old context before its runtime.
        self.ctx = ctx;
        self.rt = rt;
    }

    fn run(&mut self, code: &str, now_ms: f64) -> Result<String, String> {
        self.eval_take(code, now_ms)
    }

    fn run_module(&mut self, code: &str, name: &str, now_ms: f64) -> Result<String, String> {
        self.blobs.borrow_mut().clear();
        self.ctx.with(|ctx| {
            let _ = ctx.globals().set("__mb_now", now_ms);
            match rquickjs::Module::evaluate(ctx.clone(), name, code).catch(&ctx) {
                Ok(promise) => {
                    // A module's top-level throw rejects its promise.
                    let _ = ctx.globals().set("__mb_module_done", promise);
                    let _ = ctx.eval::<(), _>(
                        "__mb_module_done.catch(function (e) { console.error('module: ' + e + (e && e.stack ? '\\n' + e.stack : '')); });",
                    );
                }
                Err(e) => log::warn!("JS module {name}: {e}"),
            }
        });
        for _ in 0..1000 {
            match self.rt.execute_pending_job() {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => log::warn!("JS job error: {:?}", e),
            }
        }
        self.ctx.with(|ctx| ctx.eval::<String, _>("__mb_take()").catch(&ctx).map_err(|e| e.to_string()))
    }

    fn tick(&mut self, now_ms: f64) -> Result<String, String> {
        // What the network brought runs first, like a browser's task queue.
        self.eval_take(&format!("{}__mb_tick({})", self.net_events(), now_ms), now_ms)
    }

    fn next_timer(&self, now_ms: f64) -> Option<f64> {
        let t = self.ctx.with(|ctx| {
            let d: f64 = ctx.eval(format!("__mb_next({})", now_ms)).ok()?;
            (d >= 0.0).then_some(d)
        });
        // Requests and sockets running: look again soon.
        #[cfg(feature = "net")]
        if self.net.borrow().busy() {
            return Some(t.map_or(16.0, |t| t.min(16.0)));
        }
        t
    }

    fn set_dom(&mut self, dom: SharedDom) -> bool {
        self.dom = Some(dom);
        self.reset();
        true
    }

    fn take_blob(&mut self, handle: u32) -> Option<Vec<u8>> {
        let mut b = self.blobs.borrow_mut();
        b.get_mut((handle as usize).checked_sub(1)?).map(std::mem::take)
    }

    fn seed(&mut self, elements_json: &str) {
        let code = format!("__mb_seed({elements_json})");
        self.ctx.with(|ctx| {
            if let Err(e) = ctx.eval::<(), _>(code).catch(&ctx) {
                log::warn!("JS seed: {}", e);
            }
        });
    }

    fn set_input_value(&mut self, id: &str, value: &str) {
        let id = js_string(id);
        let value = js_string(value);
        self.ctx.with(|ctx| {
            let _ = ctx.eval::<(), _>(format!("__mb_el({id})._attrs.value = {value}; __mb_el({id})._text = {value};"));
        });
    }
}

impl QuickJs {
    /// JS that hands over the network's events since the last tick.
    fn net_events(&self) -> String {
        #[cfg(feature = "net")]
        {
            let events = self.net.borrow_mut().take();
            if events.is_empty() {
                return String::new();
            }
            let list: Vec<serde_json::Value> = events
                .into_iter()
                .map(|e| match e {
                    net::Event::Done { id, status, body, error } => serde_json::json!({
                        "t": "done", "id": id, "status": status, "body": String::from_utf8_lossy(&body), "error": error
                    }),
                    net::Event::Ws { id, kind, data } => serde_json::json!({ "t": "ws", "id": id, "kind": kind, "data": data }),
                })
                .collect();
            return format!("__mb_net_deliver({});\n", serde_json::Value::Array(list));
        }
        #[allow(unreachable_code)]
        String::new()
    }
}

/// `s` as a JavaScript string literal.
fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mb_core::Page;

    const COUNTER: &str = r#"<html><body>
      <div id="n">0</div><button id="inc" style="padding:10px">+</button>
      <script>
        var count = 0;
        document.getElementById('inc').onclick = function() {
          count++; document.getElementById('n').textContent = 'n=' + count;
        };
      </script></body></html>"#;

    impl QuickJs {
        fn run_at0(&mut self, code: &str) -> Result<String, String> { self.run(code, 0.0) }
    }

    fn text_of(page: &Page, id: &str) -> String {
        fn find(n: &mb_core::dom::Node, id: &str) -> Option<String> {
            if let mb_core::dom::NodeType::Element(d) = &n.node_type {
                if d.id() == Some(id) {
                    return Some(n.children.iter().filter_map(|c| match &c.node_type {
                        mb_core::dom::NodeType::Text(t) => Some(t.clone()),
                        _ => None,
                    }).collect());
                }
            }
            n.children.iter().find_map(|c| find(c, id))
        }
        find(&page.document().unwrap().root, id).unwrap_or_default()
    }

    #[test]
    fn click_runs_handler_and_state_persists() {
        let mut page = Page::new(Box::new(QuickJs::new()), 320, 200);
        page.load(COUNTER);
        let r = page.hit_regions().iter().find(|r| r.id == "inc").unwrap().clone();
        assert!(page.click(r.x + 2.0, r.y + 2.0));
        assert!(page.click(r.x + 2.0, r.y + 2.0));
        assert_eq!(text_of(&page, "n"), "n=2");
    }

    #[test]
    fn partial_repaint_matches_full_paint() {
        let html = r#"<html><body style="background:#123">
          <div style="display:flex;gap:8px"><div id="a" style="color:#fff;font-size:20px">0</div>
          <div id="b" style="background:#345;border-radius:6px;padding:4px">x</div></div>
          <div id="list" style="height:40px;overflow:hidden"></div>
          <div style="text-align:center"><span id="c">idle</span></div>
          <script>
            var n = 0;
            setInterval(function () {
              n++;
              document.getElementById('a').textContent = 'n=' + n;
              document.getElementById('b').style.backgroundColor = n % 2 ? '#a33' : '#3a3';
              document.getElementById('list').innerHTML = '<div>row ' + n + '</div><div>row</div><div>row</div><div>row</div>';
              if (n == 3) document.getElementById('c').textContent = 'busy, much longer text';
            }, 100);
          </script></body></html>"#;
        let mut page = Page::new(Box::new(QuickJs::new()), 320, 200);
        page.load(html);
        let mut partial = 0;
        for i in 1..=5 {
            assert!(page.tick(i as f64 * 100.0));
            let (w, h) = page.size();
            let (px, pw, ph) = page.full_page();
            assert_eq!((pw, ph), (w, h));
            let damage = page.take_damage();
            if damage.len() != 1 || damage[0] != [0, 0, w as i32, h as i32] {
                partial += 1;
            }
            let full = mb_core::paint::paint(page.layout_tree().unwrap(), pw, ph);
            // Antialiased rounded corners drawn through a clip mask may differ by a
            // level or two; anything more is a damage-tracking bug.
            let worst = px.iter().zip(&full).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            assert!(worst <= 2, "tick {i}: partial repaint differs from a full paint by up to {worst}");
        }
        assert!(partial > 0, "no tick was repainted partially");
    }

    #[test]
    fn scripts_read_the_document_and_toggle_classes() {
        let html = r#"<html><head><style>.alarm { color: #f00 }</style></head><body>
          <div id="v" class="card">41</div><div id="out"></div>
          <script>
            var v = document.getElementById('v');
            var n = parseInt(v.textContent) + 1;
            v.classList.add('alarm');
            document.getElementById('out').textContent = n + ' ' + v.classList.contains('card') + ' ' + v.className;
          </script></body></html>"#;
        let mut page = Page::new(Box::new(QuickJs::new()), 320, 200);
        page.load(html);
        assert_eq!(text_of(&page, "out"), "42 true card alarm");
        let v = page.document().unwrap().root.get_element_by_id("v").unwrap();
        assert_eq!(v.element_data().unwrap().attributes.get("class").map(String::as_str), Some("card alarm"));
    }

    #[test]
    fn fetch_reads_page_files_only() {
        let dir = std::env::temp_dir().join(format!("mbqjs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("data.json"), r#"{"t": 21.5}"#).unwrap();
        let mut js = QuickJs::with_files(&dir);
        let out = js.run(r#"
            fetch('data.json').then(function (r) { return r.json(); }).then(function (d) {
                document.getElementById('a').textContent = 't=' + d.t;
            });
            fetch('../../etc/passwd').then(function () { document.getElementById('b').textContent = 'leak'; },
                                           function () { document.getElementById('b').textContent = 'refused'; });
        "#, 0.0).unwrap();
        assert!(out.contains("t=21.5"), "{out}");
        assert!(out.contains("refused") && !out.contains("leak"), "{out}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn interval_fires_on_tick_only_when_due() {
        let mut js = QuickJs::new();
        js.run_at0("var k = 0; setInterval(function(){ k++; document.getElementById('a').textContent = k; }, 100);").unwrap();
        assert_eq!(js.tick(50.0).unwrap(), "[]");
        assert!(js.tick(120.0).unwrap().contains("\"text\":\"1\""));
        assert!(js.next_timer(120.0).unwrap() <= 100.0);
    }

    #[test]
    fn exception_keeps_earlier_commands() {
        let mut js = QuickJs::new();
        let out = js.run_at0("document.getElementById('a').textContent = 'x'; nope();").unwrap();
        assert!(out.contains("SetTextById"));
    }

    #[test]
    fn strings_survive_quoting() {
        let mut js = QuickJs::new();
        js.set_input_value("in", "a\"b\\c\n'");
        let out = js.run_at0("document.getElementById('o').textContent = document.getElementById('in').value;").unwrap();
        assert!(out.contains(r#"a\"b\\c\n'"#), "{}", out);
    }
}
