//! MiniBrowser on Blitz: a page whose DOM, styles (Stylo), layout (Taffy) and
//! text (Parley) come from Blitz, painted on the CPU by [`scene::TinySkiaScene`].
//!
//! Scripts run on the same [`JsHost`] as mb-core's page and answer with the
//! same DOM commands; they are applied to Blitz's DOM here. Each frame the
//! painted scene is recorded and compared with the last one: only the rect
//! around the commands that changed is painted again.

pub mod dom;
pub mod net;
pub mod scene;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use anyrender::recording::{RenderCommand, Scene};
use blitz_dom::{local_name, ns, DocumentConfig, LocalName, QualName};
use blitz_html::{DocumentHtmlParser, HtmlDocument};
use blitz_traits::shell::{ColorScheme, Viewport};
use kurbo::{Rect, Shape};
use mb_core::damage;
use mb_core::js_engine::{parse_commands, DomCommand};
use mb_core::JsHost;
use parley::fontique::{Blob, GenericFamily};

pub use scene::TinySkiaScene;

static SANS: &[u8] = include_bytes!("../../mb-core/assets/DejaVuSans-hmi.ttf");
static BOLD: &[u8] = include_bytes!("../../mb-core/assets/DejaVuSans-Bold-hmi.ttf");
static MONO: &[u8] = include_bytes!("../../mb-core/assets/DejaVuSansMono-hmi.ttf");

/// The engine's own fonts stand for every generic family: a panel has no
/// system fonts.
fn fonts() -> parley::FontContext {
    let mut cx = parley::FontContext::default();
    let mut reg = |data: &'static [u8]| cx.collection.register_fonts(Blob::new(Arc::new(data)), None)[0].0;
    let sans = reg(SANS);
    reg(BOLD);
    let mono = reg(MONO);
    for g in [GenericFamily::SansSerif, GenericFamily::Serif, GenericFamily::SystemUi, GenericFamily::UiSansSerif, GenericFamily::Cursive, GenericFamily::Fantasy] {
        cx.collection.set_generic_families(g, std::iter::once(sans));
    }
    for g in [GenericFamily::Monospace, GenericFamily::UiMonospace] {
        cx.collection.set_generic_families(g, std::iter::once(mono));
    }
    cx
}

pub struct BlitzPage {
    js: Box<dyn JsHost>,
    /// Shared with the scripts' live DOM (`dom::BlitzDom`).
    doc: Rc<RefCell<HtmlDocument>>,
    /// Scripts changed the live DOM since the last frame.
    changed: Rc<Cell<bool>>,
    /// The JS host works on the live DOM (else on by-id elements and commands).
    live: bool,
    width: u32,
    height: u32,
    now_ms: f64,
    /// RGBA, width x height.
    pixels: Vec<u8>,
    scene: TinySkiaScene,
    /// The scene painted last, to find what changed.
    last: Vec<RenderCommand>,
    damage: Vec<damage::Rect>,
    /// Painting is due: the DOM changed since the last frame.
    dirty: bool,
    /// MB_TRACE=1: log each frame's timings.
    trace: bool,
    /// Where the page's files are (images, stylesheets): its directory.
    base: Option<std::path::PathBuf>,
    /// The page's own URL when it came from the web.
    url: Option<String>,
}

impl BlitzPage {
    pub fn new(mut js: Box<dyn JsHost>, width: u32, height: u32) -> Self {
        let doc = Rc::new(RefCell::new(HtmlDocument::from_html("", Self::config(width, height))));
        let changed = Rc::new(Cell::new(false));
        // MB_DOM=ids keeps the old by-id elements, to compare.
        let live = std::env::var("MB_DOM").map_or(true, |v| v != "ids")
            && js.set_dom(Rc::new(RefCell::new(dom::BlitzDom { doc: doc.clone(), changed: changed.clone() })));
        Self {
            js,
            doc,
            changed,
            live,
            width,
            height,
            now_ms: 0.0,
            pixels: vec![0; (width * height * 4) as usize],
            scene: TinySkiaScene::new(width, height),
            last: Vec::new(),
            damage: Vec::new(),
            dirty: true,
            trace: std::env::var_os("MB_TRACE").is_some(),
            base: None,
            url: None,
        }
    }

    /// Images, stylesheets and other files the page names load from `dir`.
    pub fn with_base(mut self, dir: &std::path::Path) -> Self {
        self.base = dir.canonicalize().ok();
        self
    }

    fn config(width: u32, height: u32) -> DocumentConfig {
        DocumentConfig {
            viewport: Some(Viewport::new(width, height, 1.0, ColorScheme::Light)),
            font_ctx: Some(fonts()),
            ..Default::default()
        }
    }

    /// Loads the page at `url` (http, https; feature `http`): relative
    /// images, stylesheets and scripts come from the same place.
    pub fn load_url(&mut self, url: &str) -> bool {
        let Some(body) = net::http_get(url) else { return false };
        self.url = Some(url.to_string());
        self.load(&String::from_utf8_lossy(&body));
        true
    }

    fn page_config(&self) -> DocumentConfig {
        let mut c = Self::config(self.width, self.height);
        let files = self.base.clone().unwrap_or_else(|| std::path::PathBuf::from("/nonexistent"));
        if let Some(url) = &self.url {
            c.base_url = Some(url.clone());
            c.net_provider = Some(Arc::new(net::Files::new(files)));
        } else if let Some(base) = &self.base {
            c.base_url = Some(format!("file://{}/", base.display()));
            c.net_provider = Some(Arc::new(net::Files::new(files)));
        }
        c
    }

    pub fn load(&mut self, html: &str) {
        // The new document first: a live DOM host reads it as it resets.
        *self.doc.borrow_mut() = HtmlDocument::from_html(html, self.page_config());
        self.js.reset();
        self.last.clear();
        // Scripts run here: what a page shows without them goes.
        {
            let mut doc = self.doc.borrow_mut();
            let gone: Vec<_> = doc.query_selector_all("noscript").map(|v| v.to_vec()).unwrap_or_default();
            let mut m = doc.mutate();
            for n in gone {
                m.remove_node(n);
            }
        }
        if !self.live {
            self.seed_js();
        }
        for code in self.scripts() {
            let cmds = run(self.js.as_mut(), &code, self.now_ms);
            self.apply(&cmds);
        }
        if self.live {
            let cmds = run(self.js.as_mut(), "__mb_loaded()", self.now_ms);
            self.apply(&cmds);
        }
        self.dirty = true;
        self.render();
    }

    /// Runs the timers due at `now_ms`. Returns true if it repainted.
    pub fn tick(&mut self, now_ms: f64) -> bool {
        self.now_ms = now_ms;
        let cmds = match self.js.tick(now_ms) {
            Ok(json) => parse_commands(&json),
            Err(e) => {
                log::warn!("timer: {e}");
                Vec::new()
            }
        };
        if !cmds.is_empty() {
            self.apply(&cmds);
        }
        self.render()
    }

    pub fn next_timer(&self, now_ms: f64) -> Option<f64> {
        self.js.next_timer(now_ms)
    }

    /// A tap at viewport coordinates. On the live DOM: a click event on the
    /// element there, bubbling. Otherwise: the onclick of the element there
    /// or its nearest ancestor that has one. Returns true if it repainted.
    pub fn click(&mut self, x: f32, y: f32) -> bool {
        let code = {
            let doc = self.doc.borrow();
            let Some(hit) = doc.element_from_point(x, y) else { return false };
            let offset = |id| {
                doc.get_node(id).map(|n| {
                    let p = n.absolute_position(0.0, 0.0);
                    (x - p.x, y - p.y)
                }).unwrap_or((0.0, 0.0))
            };
            if self.live {
                let (ox, oy) = offset(hit);
                format!(
                    "__mb_dispatch({}, 'click', {{clientX: {x}, clientY: {y}, pageX: {x}, pageY: {y}, offsetX: {ox}, offsetY: {oy}}})",
                    hit.as_u64()
                )
            } else {
                let mut at = Some(hit);
                let mut handler = None;
                while let Some(id) = at {
                    let Some(node) = doc.get_node(id) else { break };
                    if let Some(code) = node.attr(local_name!("onclick")) {
                        handler = Some((id, code.to_string()));
                        break;
                    }
                    at = node.parent;
                }
                let Some((id, code)) = handler else { return false };
                let (ox, oy) = offset(id);
                format!(
                    "window.event = {{type: 'click', clientX: {x}, clientY: {y}, pageX: {x}, pageY: {y}, offsetX: {ox}, offsetY: {oy}, \
                     preventDefault: function () {{}}, stopPropagation: function () {{}}}};\n{code}"
                )
            }
        };
        let cmds = run(self.js.as_mut(), &code, self.now_ms);
        self.apply(&cmds);
        self.render()
    }

    /// The page's pixels, RGBA, width x height.
    pub fn viewport_slice(&self) -> Option<&[u8]> {
        Some(&self.pixels)
    }

    pub fn viewport(&self) -> Vec<u8> {
        self.pixels.clone()
    }

    pub fn take_damage(&mut self) -> Vec<damage::Rect> {
        std::mem::take(&mut self.damage)
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn node_count(&self) -> usize {
        self.doc.borrow().tree().len()
    }

    pub fn document(&self) -> std::cell::Ref<'_, HtmlDocument> {
        self.doc.borrow()
    }

    /// Scripts in document order; an external one as `__mb_include(src)`.
    fn scripts(&self) -> Vec<String> {
        let doc = self.doc.borrow();
        let mut out = Vec::new();
        let mut stack = vec![doc.root_node().id];
        while let Some(id) = stack.pop() {
            let Some(node) = doc.get_node(id) else { continue };
            if node.element_data().is_some_and(|e| e.name.local == local_name!("script")) {
                // Classic scripts only: not JSON data, templates or ES modules (yet).
                let ty = node.attr(local_name!("type")).unwrap_or("").trim().to_ascii_lowercase();
                if !matches!(ty.as_str(), "" | "text/javascript" | "application/javascript" | "text/ecmascript") {
                    continue;
                }
                match node.attr(local_name!("src")).map(str::trim).filter(|s| !s.is_empty()) {
                    // A web page's scripts come from the web.
                    Some(src) if self.url.is_some() => {
                        let full = doc.base_url().join(src).map(|u| u.to_string()).unwrap_or_else(|_| src.to_string());
                        match net::http_get(&full) {
                            Some(code) => out.push(String::from_utf8_lossy(&code).into_owned()),
                            None => log::warn!("script {full}: not loaded"),
                        }
                    }
                    Some(src) => out.push(format!("__mb_include({})", js_string(src))),
                    None => {
                        let text = node.text_content();
                        if !text.trim().is_empty() {
                            out.push(text);
                        }
                    }
                }
                continue;
            }
            stack.extend(node.children.iter().rev());
        }
        out
    }

    /// Hands the JS host every element with an id: its text and attributes.
    fn seed_js(&mut self) {
        let mut out = Vec::new();
        let doc = self.doc.borrow();
        for (_, node) in doc.tree().iter() {
            let Some(el) = node.element_data() else { continue };
            let Some(id) = el.id.as_ref() else { continue };
            let attrs: serde_json::Map<String, serde_json::Value> =
                el.attrs.iter().map(|a| (a.name.local.to_string(), serde_json::Value::String(a.value.clone()))).collect();
            out.push(serde_json::json!({ "id": id.to_string(), "text": node.text_content(), "attrs": attrs }));
        }
        if !out.is_empty() {
            self.js.seed(&serde_json::Value::Array(out).to_string());
        }
    }

    fn apply(&mut self, cmds: &[DomCommand]) {
        let mut seed = false;
        for cmd in cmds {
            match cmd {
                DomCommand::SetTextById { id, text } => {
                    let mut doc = self.doc.borrow_mut();
                    let Some(node) = doc.get_element_by_id(id) else { continue };
                    let mut m = doc.mutate();
                    let kids = m.child_ids(node);
                    // One text child: change it in place (cheaper restyle).
                    if kids.len() == 1 && m.element_name(kids[0]).is_none() {
                        m.set_node_text(kids[0], text);
                    } else {
                        m.remove_and_drop_all_children(node);
                        let t = m.create_text_node(text);
                        m.append_children(node, &[t]);
                    }
                }
                DomCommand::SetStyleById { id, property, value } => {
                    let mut doc = self.doc.borrow_mut();
                    let Some(node) = doc.get_element_by_id(id) else { continue };
                    let mut m = doc.mutate();
                    if value.is_empty() {
                        m.remove_style_property(node, property);
                    } else {
                        m.set_style_property(node, property, value);
                    }
                }
                DomCommand::SetAttribute { id, name, value } => {
                    let mut doc = self.doc.borrow_mut();
                    let Some(node) = doc.get_element_by_id(id) else { continue };
                    let mut m = doc.mutate();
                    m.set_attribute(node, QualName::new(None, ns!(), LocalName::from(name.as_str())), value);
                }
                DomCommand::SetInnerHtmlById { id, html } => {
                    let mut doc = self.doc.borrow_mut();
                    let Some(node) = doc.get_element_by_id(id) else { continue };
                    let mut m = doc.mutate();
                    m.remove_and_drop_all_children(node);
                    DocumentHtmlParser::parse_inner_html_into_mutator(&mut m, node, html);
                    seed = true;
                }
                DomCommand::CreateElement { tag, parent_id } => {
                    let mut doc = self.doc.borrow_mut();
                    let Some(parent) = doc.get_element_by_id(parent_id) else { continue };
                    let mut m = doc.mutate();
                    let el = m.create_element(QualName::new(None, ns!(html), LocalName::from(tag.as_str())), Vec::new());
                    m.append_children(parent, &[el]);
                }
                DomCommand::ConsoleLog { message } => log::info!("console: {message}"),
                // Canvas drawing is not wired to Blitz yet.
                DomCommand::Canvas { .. } => {}
            }
            self.dirty = true;
        }
        if seed {
            self.seed_js();
        }
    }

    /// Styles, layout and paint of what changed. Returns true if pixels changed.
    pub fn render(&mut self) -> bool {
        if !self.dirty && !self.changed.get() {
            return false;
        }
        self.dirty = false;
        self.changed.set(false);
        let mut doc = self.doc.borrow_mut();
        let t = std::time::Instant::now();
        doc.resolve(self.now_ms);
        let t_resolve = t.elapsed();
        let mut rec = Scene::new();
        blitz_paint::paint_scene(&mut rec, &mut doc, 1.0, self.width, self.height, 0, 0);
        drop(doc);
        let t_record = t.elapsed();
        let full = Rect::new(0.0, 0.0, self.width as f64, self.height as f64);
        let changed = if self.last.is_empty() { vec![full] } else { changed_rects(&self.last, &rec.commands) };
        let rects: Vec<_> = changed.into_iter().filter_map(|r| snap(r, self.width, self.height)).collect();
        for &r in &rects {
            self.scene.begin(r);
            // Page background: white, as the canvas is when the page sets none.
            self.scene.pixmap.fill(tiny_skia::Color::WHITE);
            self.scene.replay(&rec.commands);
            self.copy_out(r);
            self.damage.push([r.x(), r.y(), r.right(), r.bottom()]);
        }
        if self.trace {
            let area: i64 = rects.iter().map(|r| r.width() as i64 * r.height() as i64).sum();
            eprintln!(
                "render: resolve {:.1}, record {:.1} ({} cmds), paint {:.1} ms, {} rects {} px",
                t_resolve.as_secs_f64() * 1e3,
                (t_record - t_resolve).as_secs_f64() * 1e3,
                rec.commands.len(),
                (t.elapsed() - t_record).as_secs_f64() * 1e3,
                rects.len(),
                area
            );
        }
        self.last = rec.commands;
        !rects.is_empty()
    }

    pub fn scene_summary(&self) -> String {
        describe_scene(&self.last)
    }

    /// Paints the whole screen again (for checks); returns the pixels.
    pub fn repaint_full(&mut self) -> Vec<u8> {
        self.last.clear();
        self.dirty = true;
        self.render();
        self.damage.clear();
        self.pixels.clone()
    }

    /// Region pixmap (premultiplied, opaque after the white base) into `pixels`.
    fn copy_out(&mut self, r: tiny_skia::IntRect) {
        let src = self.scene.pixmap.data();
        let rw = r.width() as usize * 4;
        for row in 0..r.height() as usize {
            let s = &src[row * rw..row * rw + rw];
            let at = ((r.y() as usize + row) * self.width as usize + r.x() as usize) * 4;
            self.pixels[at..at + rw].copy_from_slice(s);
        }
    }
}

fn run(js: &mut dyn JsHost, code: &str, now_ms: f64) -> Vec<DomCommand> {
    match js.run(code, now_ms) {
        Ok(json) => parse_commands(&json),
        Err(e) => {
            log::warn!("script: {e}");
            Vec::new()
        }
    }
}

fn js_string(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

/// The page-pixel rect a command can paint (None: it paints nothing by itself).
fn bounds(cmd: &RenderCommand) -> Option<Rect> {
    let r = match cmd {
        RenderCommand::PushLayer(c) => c.transform.transform_rect_bbox(c.clip.bounding_box()),
        RenderCommand::PushClipLayer(c) => c.transform.transform_rect_bbox(c.clip.bounding_box()),
        RenderCommand::PopLayer => return None,
        RenderCommand::Stroke(c) => {
            let w = c.style.width / 2.0 + 1.0;
            c.transform.transform_rect_bbox(c.shape.bounding_box().inflate(w, w))
        }
        RenderCommand::Fill(c) => c.transform.transform_rect_bbox(c.shape.bounding_box()).inflate(1.0, 1.0),
        RenderCommand::GlyphRun(c) => {
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for g in &c.glyphs {
                x0 = x0.min(g.x as f64);
                y0 = y0.min(g.y as f64);
                x1 = x1.max(g.x as f64);
                y1 = y1.max(g.y as f64);
            }
            if c.glyphs.is_empty() {
                return None;
            }
            let s = c.font_size as f64;
            c.transform.transform_rect_bbox(Rect::new(x0 - s * 0.5, y0 - s * 1.5, x1 + s * 1.5, y1 + s * 0.5))
        }
        RenderCommand::BoxShadow(c) => c.transform.transform_rect_bbox(c.rect).inflate(3.0 * c.std_dev + 2.0, 3.0 * c.std_dev + 2.0),
    };
    Some(r)
}

/// Rects around what differs between two scenes. Same command count: each
/// changed command, old and new (an animation that moves things). Otherwise
/// the commands between the common start and the common end, in both.
fn changed_rects(old: &[RenderCommand], new: &[RenderCommand]) -> Vec<Rect> {
    let all = Rect::new(0.0, 0.0, f64::MAX / 4.0, f64::MAX / 4.0);
    let mut out = Vec::new();
    let add = |c: &RenderCommand, out: &mut Vec<Rect>| match bounds(c) {
        Some(b) => out.push(b),
        // A layer's end moved: everything may have changed.
        None => out.push(all),
    };
    if old.len() == new.len() {
        for (a, b) in old.iter().zip(new) {
            if a != b {
                add(a, &mut out);
                add(b, &mut out);
            }
        }
    } else {
        let pre = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let max_suf = old.len().min(new.len()) - pre;
        let suf = old.iter().rev().zip(new.iter().rev()).take(max_suf).take_while(|(a, b)| a == b).count();
        for c in old[pre..old.len() - suf].iter().chain(&new[pre..new.len() - suf]) {
            add(c, &mut out);
        }
    }
    merge(out)
}

/// Fewer, larger rects: merge any two whose union costs little more than the
/// two apart, and at most 8 in all.
fn merge(mut rs: Vec<Rect>) -> Vec<Rect> {
    let area = |r: &Rect| r.width().max(0.0) * r.height().max(0.0);
    loop {
        let mut best: Option<(usize, usize, f64)> = None;
        for i in 0..rs.len() {
            for j in i + 1..rs.len() {
                let u = rs[i].union(rs[j]);
                let waste = area(&u) - area(&rs[i]) - area(&rs[j]) + area(&rs[i].intersect(rs[j]));
                if best.is_none_or(|b| waste < b.2) {
                    best = Some((i, j, waste));
                }
            }
        }
        match best {
            Some((i, j, waste)) if waste <= 2000.0 || rs.len() > 8 => {
                let u = rs[i].union(rs[j]);
                rs.swap_remove(j);
                rs[i] = u;
            }
            _ => return rs,
        }
    }
}

fn snap(r: Rect, w: u32, h: u32) -> Option<tiny_skia::IntRect> {
    let x0 = r.x0.floor().max(0.0) as i32 - 1;
    let y0 = r.y0.floor().max(0.0) as i32 - 1;
    let x1 = (r.x1.ceil().min(w as f64) as i32 + 1).min(w as i32);
    let y1 = (r.y1.ceil().min(h as f64) as i32 + 1).min(h as i32);
    let (x0, y0) = (x0.max(0), y0.max(0));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    tiny_skia::IntRect::from_ltrb(x0, y0, x1, y1)
}

/// Kinds of the last recorded scene's commands, for tracing.
pub fn describe_scene(cmds: &[RenderCommand]) -> String {
    let mut n = [0usize; 7];
    for c in cmds {
        n[match c {
            RenderCommand::PushLayer(_) => 0,
            RenderCommand::PushClipLayer(_) => 1,
            RenderCommand::PopLayer => 2,
            RenderCommand::Stroke(_) => 3,
            RenderCommand::Fill(_) => 4,
            RenderCommand::GlyphRun(_) => 5,
            RenderCommand::BoxShadow(_) => 6,
        }] += 1;
    }
    format!("layers {} clips {} pops {} strokes {} fills {} text {} shadows {}", n[0], n[1], n[2], n[3], n[4], n[5], n[6])
}
