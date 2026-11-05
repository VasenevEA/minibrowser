//! [`JsHost`] on QuickJS: real JavaScript for native builds (the panel, headless).
//!
//! Each page gets a fresh runtime with the DOM glue from `dom.js`. Page code runs
//! at global scope, so `var`s and functions persist between scripts, handlers and
//! timer callbacks, as in a browser.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mb_core::JsHost;
use rquickjs::{CatchResultExt, Context, Function, Runtime};

const DOM_JS: &str = include_str!("dom.js");

pub struct QuickJs {
    rt: Runtime,
    ctx: Context,
    /// Where `fetch` finds relative URLs (the page's directory); None: no files.
    base: Option<PathBuf>,
    /// Binary data scripts hand over (putImageData pixels), by handle - 1.
    blobs: Blobs,
}

type Blobs = Rc<RefCell<Vec<Vec<u8>>>>;

impl QuickJs {
    pub fn new() -> Self {
        let blobs = Blobs::default();
        let (rt, ctx) = fresh(None, blobs.clone());
        QuickJs { rt, ctx, base: None, blobs }
    }

    /// A host whose `fetch` reads files: relative URLs from `base` (the page's
    /// directory), absolute paths under `base` or `/tmp` (where the panel's data
    /// collector drops JSON).
    pub fn with_files(base: &Path) -> Self {
        let base = Some(base.to_path_buf());
        let blobs = Blobs::default();
        let (rt, ctx) = fresh(base.clone(), blobs.clone());
        QuickJs { rt, ctx, base, blobs }
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

fn fresh(base: Option<PathBuf>, blobs: Blobs) -> (Runtime, Context) {
    let rt = Runtime::new().expect("QuickJS runtime");
    // A page must not take the panel down with it.
    rt.set_memory_limit(32 << 20);
    rt.set_max_stack_size(512 << 10);
    let ctx = Context::full(&rt).expect("QuickJS context");
    ctx.with(|ctx| {
        let read = Function::new(ctx.clone(), move |url: String| -> Option<String> {
            let path = resolve(base.as_deref()?, &url)?;
            std::fs::read_to_string(path).ok()
        });
        if let Ok(read) = read {
            let _ = ctx.globals().set("__mb_read", read);
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
        ctx.eval::<(), _>(DOM_JS).catch(&ctx).map_err(|e| e.to_string()).expect("dom.js");
    });
    (rt, ctx)
}

impl JsHost for QuickJs {
    fn reset(&mut self) {
        self.blobs.borrow_mut().clear();
        let (rt, ctx) = fresh(self.base.clone(), self.blobs.clone());
        // Drop the old context before its runtime.
        self.ctx = ctx;
        self.rt = rt;
    }

    fn run(&mut self, code: &str, now_ms: f64) -> Result<String, String> {
        self.eval_take(code, now_ms)
    }

    fn tick(&mut self, now_ms: f64) -> Result<String, String> {
        self.eval_take(&format!("__mb_tick({})", now_ms), now_ms)
    }

    fn next_timer(&self, now_ms: f64) -> Option<f64> {
        self.ctx.with(|ctx| {
            let d: f64 = ctx.eval(format!("__mb_next({})", now_ms)).ok()?;
            (d >= 0.0).then_some(d)
        })
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
