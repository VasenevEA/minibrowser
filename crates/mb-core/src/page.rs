//! A loaded page: the DOM, its scripts' host, and the painted result, plus the
//! input handling (taps, keys, scrolling) that changes them.
//!
//! Platform frontends (wasm canvas, Linux framebuffer, headless PNG) own a `Page`
//! and only move pixels and events in and out of it.

use std::collections::HashMap;

use crate::js_engine::{self, JsHost};
use crate::{css, dom, html_parser, layout, paint};

/// A clickable region on the canvas, tied to a DOM element.
#[derive(Debug, Clone)]
pub struct HitRegion {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub onclick: String,
    /// Element ID (for focus tracking on inputs).
    pub id: String,
    /// Whether this is a focusable input element.
    pub is_input: bool,
    /// The DOM node's uid (live DOM: any element can listen for taps).
    pub uid: u32,
}

/// Which layout code positions the boxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutEngine {
    /// The original hand-written block/inline/flex/grid layout.
    Builtin,
    /// Taffy for block, flex and grid; the built-in code for inline content.
    #[default]
    Taffy,
}

impl LayoutEngine {
    /// From the `MB_LAYOUT` environment variable: "builtin" picks the original
    /// layout, anything else Taffy.
    pub fn from_env() -> Self {
        match std::env::var("MB_LAYOUT").as_deref() {
            Ok("builtin") => LayoutEngine::Builtin,
            _ => LayoutEngine::Taffy,
        }
    }
}

/// Numbers about the last render, for frontends to show or log.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub node_count: usize,
    pub box_count: usize,
    pub content_width: f32,
    pub content_height: f32,
}

pub struct Page {
    js: Box<dyn JsHost>,
    document: Option<dom::Document>,
    width: u32,
    height: u32,
    hit_regions: Vec<HitRegion>,
    focused_input: Option<String>,
    scroll_x: f32,
    scroll_y: f32,
    /// The whole painted page (`page_w * page_h` RGBA); the viewport is a window into it.
    pixels: Vec<u8>,
    page_w: u32,
    page_h: u32,
    images: HashMap<String, (Vec<u8>, u32, u32)>,
    pending_images: Vec<String>,
    stats: Stats,
    /// The page clock as of the last `tick`; timers that scripts set count from it.
    now_ms: f64,
    engine: LayoutEngine,
    layout: Option<layout::LayoutBox>,
    /// No doctype: lay out as browsers do in quirks mode.
    quirks: bool,
    /// Page rectangles repainted since the frontend last took them.
    damage: Vec<crate::damage::Rect>,
    /// Element ids already handed to the JS host (see `seed_js`).
    seeded: std::collections::HashSet<String>,
    svg: crate::svg::SvgCache,
    /// The styled tree of the last full render, for repaints that skip styling.
    styled: Option<css::StyledNode>,
    canvases: crate::canvas::Canvases,
    /// The live DOM scripts work on, if the JS host takes one.
    live: Option<Live>,
}

/// What the page shares with its live DOM (`live::CoreDom`).
struct Live {
    /// Holds the document while a script runs.
    slot: std::rc::Rc<std::cell::RefCell<Option<dom::Document>>>,
    log: std::rc::Rc<std::cell::RefCell<Vec<js_engine::DomCommand>>>,
    structural: std::rc::Rc<std::cell::Cell<bool>>,
    /// Element boxes of the last render (uid → x, y, w, h), for getBoundingClientRect.
    rects: std::rc::Rc<std::cell::RefCell<HashMap<u32, [f32; 4]>>>,
}

impl Page {
    pub fn new(mut js: Box<dyn JsHost>, width: u32, height: u32) -> Self {
        // MB_DOM=ids keeps the by-id elements and commands of v0.7.
        let live = (std::env::var("MB_DOM").map_or(true, |v| v != "ids")).then(|| Live {
            slot: Default::default(),
            log: Default::default(),
            structural: Default::default(),
            rects: Default::default(),
        });
        let live = live.filter(|l| {
            js.set_dom(std::rc::Rc::new(std::cell::RefCell::new(crate::live::CoreDom {
                doc: l.slot.clone(),
                log: l.log.clone(),
                structural: l.structural.clone(),
                rects: l.rects.clone(),
                viewport: (width, height),
            })))
        });
        Page {
            js,
            live,
            document: None,
            width,
            height,
            hit_regions: Vec::new(),
            focused_input: None,
            scroll_x: 0.0,
            scroll_y: 0.0,
            pixels: Vec::new(),
            page_w: 0,
            page_h: 0,
            images: HashMap::new(),
            pending_images: Vec::new(),
            stats: Stats::default(),
            now_ms: 0.0,
            engine: LayoutEngine::default(),
            layout: None,
            quirks: false,
            damage: Vec::new(),
            seeded: std::collections::HashSet::new(),
            svg: crate::svg::SvgCache::default(),
            styled: None,
            canvases: crate::canvas::Canvases::default(),
        }
    }

    /// Parses `html`, runs its scripts on a fresh JS state and paints it.
    pub fn load(&mut self, html: &str) {
        self.js.reset();
        self.layout = None; // a new page: no partial repaint against the old one
        self.quirks = is_quirks(html);
        let mut doc = html_parser::parse_html(html);
        self.seeded.clear();
        self.canvases.clear();
        if self.live.is_some() {
            let scripts = js_engine::page_scripts(&doc);
            js_engine::remove_script_elements(&mut doc.root);
            self.document = Some(doc);
            for (i, script) in scripts.into_iter().enumerate() {
                match script {
                    js_engine::Script::Classic(code) => self.run_js(&code),
                    js_engine::Script::Module(code) => {
                        let name = format!("./page-module-{i}.js");
                        self.run_js_with(move |js, now| js.run_module(&code, &name, now))
                    }
                };
            }
            self.run_js("__mb_loaded()");
        } else {
            self.seed_js(&doc);
            let script_count = js_engine::execute_scripts(&mut doc, self.js.as_mut(), self.now_ms);
            if script_count > 0 {
                log::info!("Executed {} script(s), DOM updated", script_count);
            }
            self.document = Some(doc);
        }
        self.draw_canvases();
        self.focused_input = None;
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
        self.render();
    }

    /// Re-parses `html` and runs its scripts again on the *current* JS state, keeping
    /// the scroll position. This is v0.7's "live" mode, for pages that redraw from a
    /// top-level script instead of setting a timer; new pages should use timers.
    pub fn reload(&mut self, html: &str) {
        self.quirks = is_quirks(html);
        let mut doc = html_parser::parse_html(html);
        js_engine::execute_scripts(&mut doc, self.js.as_mut(), self.now_ms);
        self.document = Some(doc);
        self.render();
    }

    /// Picks the layout code; takes effect on the next render.
    pub fn set_layout_engine(&mut self, engine: LayoutEngine) {
        self.engine = engine;
    }

    /// Changes the viewport size and repaints.
    pub fn resize(&mut self, width: u32, height: u32) {
        if (width, height) != (self.width, self.height) {
            self.width = width;
            self.height = height;
            self.render();
        }
    }

    /// Advances the page clock and runs the timers due by then. Returns true if they
    /// changed the DOM (it was repainted). Frontends call it before `load` too, so
    /// that the page's first timers are set on their clock.
    pub fn tick(&mut self, now_ms: f64) -> bool {
        self.now_ms = now_ms;
        if self.document.is_none() {
            return false;
        }
        if self.live.is_some() {
            let (cmds, structural) = self.run_js_with(|js, now| js.tick(now));
            return self.after_js(cmds, structural);
        }
        let Some(doc) = self.document.as_mut() else { return false };
        let cmds = js_engine::tick_timers(doc, self.js.as_mut(), now_ms);
        if cmds.is_empty() {
            return false;
        }
        self.seed_new();
        let canvases = self.draw_canvases();
        if !self.update_drawings_only(&cmds, &canvases) {
            self.render();
        }
        true
    }

    /// Milliseconds until the page's next timer, if it has any.
    pub fn next_timer(&self, now_ms: f64) -> Option<f64> {
        self.js.next_timer(now_ms)
    }

    /// Handles a tap or click at viewport coordinates. Returns true if it repainted.
    pub fn click(&mut self, x: f32, y: f32) -> bool {
        let page_x = x + self.scroll_x;
        let page_y = y + self.scroll_y;

        // Live DOM: a click event on the innermost element there, bubbling.
        if self.live.is_some() {
            let hit = self.hit_regions.iter().rev().find(|r| {
                page_x >= r.x && page_x <= r.x + r.w && page_y >= r.y && page_y <= r.y + r.h
            }).cloned();
            let Some(hit) = hit else { return false };
            if hit.is_input && !hit.id.is_empty() {
                self.focused_input = Some(hit.id.clone());
            }
            let code = format!(
                "__mb_dispatch({}, 'click', {{clientX: {x}, clientY: {y}, pageX: {page_x}, pageY: {page_y}, offsetX: {}, offsetY: {}}})",
                hit.uid, page_x - hit.x, page_y - hit.y
            );
            let (cmds, structural) = self.run_js(&code);
            self.after_js(cmds, structural);
            return true;
        }

        // Later (more deeply nested / on-top) elements win.
        let hit = self.hit_regions.iter().rev().find(|r| {
            page_x >= r.x && page_x <= r.x + r.w && page_y >= r.y && page_y <= r.y + r.h
                && (!r.onclick.is_empty() || r.is_input)
        }).cloned();

        let Some(hit) = hit else {
            // Click outside any region → clear focus
            return self.set_focus(None);
        };

        if hit.is_input && !hit.id.is_empty() {
            log::info!("click({}, {}): focusing input #{}", x, y, hit.id);
            return self.set_focus(Some(hit.id));
        }
        self.focused_input = None;
        if hit.onclick.is_empty() {
            return false;
        }

        log::info!("click({}, {}): onclick = {:?}", x, y, hit.onclick);
        let Some(doc) = self.document.as_mut() else { return false };
        // Handlers read input.value, so scripts must see what the user typed.
        for (id, value) in input_values(&doc.root) {
            self.js.set_input_value(&id, &value);
        }
        // Handlers get the tap as window.event (and their first argument).
        let code = format!(
            "window.event = {{type: 'click', clientX: {x}, clientY: {y}, pageX: {page_x}, pageY: {page_y}, offsetX: {}, offsetY: {}, \
             preventDefault: function () {{}}, stopPropagation: function () {{}}}};\n{}",
            page_x - hit.x, page_y - hit.y, hit.onclick
        );
        let cmds = js_engine::execute_onclick(doc, self.js.as_mut(), &code, self.now_ms);
        self.seed_new();
        let canvases = self.draw_canvases();
        if !self.update_drawings_only(&cmds, &canvases) {
            self.render();
        }
        true
    }

    /// Types a key (a character, "Backspace" or "Enter") into the focused input.
    /// Returns true if it repainted.
    pub fn key(&mut self, key: &str) -> bool {
        let Some(id) = self.focused_input.clone() else { return false };
        let Some(doc) = self.document.as_mut() else { return false };
        if !update_input_value(&mut doc.root, &id, key) {
            return false;
        }
        let value = get_input_value(&doc.root, &id).unwrap_or_default();
        self.js.set_input_value(&id, &value);
        self.render();
        true
    }

    /// Scrolls the viewport, clamped to the content. Returns true if it moved.
    pub fn scroll(&mut self, dx: f32, dy: f32) -> bool {
        let max_x = (self.stats.content_width - self.width as f32).max(0.0);
        let max_y = (self.stats.content_height - self.height as f32).max(0.0);
        let (x, y) = ((self.scroll_x + dx).clamp(0.0, max_x), (self.scroll_y + dy).clamp(0.0, max_y));
        let moved = (x, y) != (self.scroll_x, self.scroll_y);
        self.scroll_x = x;
        self.scroll_y = y;
        moved
    }

    /// The visible part of the page as `width * height` RGBA.
    pub fn viewport(&self) -> Vec<u8> {
        let (vw, vh) = (self.width, self.height);
        let mut out = vec![0u8; (vw * vh * 4) as usize];
        if self.pixels.is_empty() {
            return out;
        }
        let src_x = (self.scroll_x as u32).min(self.page_w.saturating_sub(vw));
        let src_y = (self.scroll_y as u32).min(self.page_h.saturating_sub(vh));
        let copy_w = vw.min(self.page_w - src_x) as usize * 4;
        let copy_h = vh.min(self.page_h - src_y);
        for row in 0..copy_h {
            let src = (((src_y + row) * self.page_w + src_x) * 4) as usize;
            let dst = (row * vw * 4) as usize;
            out[dst..dst + copy_w].copy_from_slice(&self.pixels[src..src + copy_w]);
        }
        out
    }

    /// The visible part of the page without copying, when it is one contiguous
    /// run of the page buffer (page as wide as the viewport, scrolled only
    /// vertically); else use `viewport`.
    pub fn viewport_slice(&self) -> Option<&[u8]> {
        if self.page_w != self.width || self.scroll_x != 0.0 || self.pixels.is_empty() {
            return None;
        }
        let row = self.width as usize * 4;
        let top = (self.scroll_y as usize).min(self.page_h.saturating_sub(self.height) as usize);
        self.pixels.get(top * row..(top + self.height as usize) * row)
    }

    /// The whole painted page (it may be taller or wider than the viewport).
    pub fn full_page(&self) -> (Vec<u8>, u32, u32) {
        (self.pixels.clone(), self.page_w, self.page_h)
    }

    /// The box tree of the last render.
    pub fn layout_tree(&self) -> Option<&layout::LayoutBox> { self.layout.as_ref() }

    /// Page rectangles `[x0, y0, x1, y1)` repainted since the last call (the whole
    /// page after a full paint): what a frontend has to copy to the screen.
    pub fn take_damage(&mut self) -> Vec<crate::damage::Rect> { std::mem::take(&mut self.damage) }

    pub fn size(&self) -> (u32, u32) { (self.width, self.height) }
    pub fn scroll_pos(&self) -> (f32, f32) { (self.scroll_x, self.scroll_y) }
    pub fn stats(&self) -> Stats { self.stats }
    pub fn focused_input(&self) -> Option<&str> { self.focused_input.as_deref() }
    pub fn document(&self) -> Option<&dom::Document> { self.document.as_ref() }
    pub fn hit_regions(&self) -> &[HitRegion] { &self.hit_regions }

    /// The box tree of the last render, one line per box (for debugging layout).
    pub fn dump_layout(&self) -> String {
        fn walk(b: &layout::LayoutBox, depth: usize, out: &mut String) {
            let what = match &b.content {
                layout::BoxContent::Text(t) => format!("text {:?}", t.chars().take(24).collect::<String>()),
                c => format!("{:?}", c).split_whitespace().next().unwrap_or("").to_string(),
            };
            out.push_str(&format!("{:indent$}{} x={:.0} y={:.0} w={:.0} h={:.0} m=({:.0} {:.0} {:.0} {:.0})\n",
                "", what, b.x, b.y, b.width, b.height, b.margin.top, b.margin.right, b.margin.bottom, b.margin.left, indent = depth * 2));
            for c in &b.children {
                walk(c, depth + 1, out);
            }
        }
        let mut out = String::new();
        if let Some(root) = &self.layout {
            walk(root, 0, &mut out);
        }
        for r in &self.hit_regions {
            out.push_str(&format!("hit #{} x={:.0} y={:.0} w={:.0} h={:.0} {}\n", r.id, r.x, r.y, r.w, r.h,
                if r.is_input { "input".to_string() } else { r.onclick.chars().take(40).collect() }));
        }
        out
    }

    /// Image URLs the last render needed but has no data for; the frontend fetches
    /// them and hands them over with `add_image`.
    pub fn pending_images(&self) -> &[String] { &self.pending_images }

    /// Decodes and caches a fetched image. Call `rerender` once the batch is in.
    pub fn add_image(&mut self, url: &str, data: &[u8]) -> bool {
        match image::load_from_memory(data) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (w, h) = (rgba.width(), rgba.height());
                log::info!("add_image: cached {}x{} for {}", w, h, url);
                self.images.insert(url.to_string(), (rgba.into_raw(), w, h));
                true
            }
            Err(e) => {
                log::warn!("add_image: failed to decode {}: {}", url, e);
                false
            }
        }
    }

    /// Repaints from the current DOM (after images arrived, for instance).
    pub fn rerender(&mut self) { self.render(); }

    /// Hands the JS host the elements with an id it has not seen yet (new ones
    /// come from innerHTML), with their text and attributes.
    fn seed_js(&mut self, doc: &dom::Document) {
        fn walk(n: &dom::Node, seeded: &mut std::collections::HashSet<String>, out: &mut Vec<serde_json::Value>) {
            if let dom::NodeType::Element(d) = &n.node_type {
                if let Some(id) = d.id() {
                    if seeded.insert(id.to_string()) {
                        out.push(serde_json::json!({ "id": id, "text": n.inner_text(), "attrs": d.attributes }));
                    }
                }
            }
            for c in &n.children {
                walk(c, seeded, out);
            }
        }
        let mut out = Vec::new();
        walk(&doc.root, &mut self.seeded, &mut out);
        if !out.is_empty() {
            self.js.seed(&serde_json::Value::Array(out).to_string());
        }
    }

    fn seed_new(&mut self) {
        if let Some(doc) = self.document.take() {
            self.seed_js(&doc);
            self.document = Some(doc);
        }
    }

    /// Runs `code` on the live DOM (the document lent to it meanwhile); applies
    /// the commands it still queues (canvas, console). Returns them with the
    /// live DOM's by-id log, and whether it changed the tree's shape.
    fn run_js(&mut self, code: &str) -> (Vec<js_engine::DomCommand>, bool) {
        let code = code.to_string();
        self.run_js_with(move |js, now| js.run(&code, now))
    }

    fn run_js_with(&mut self, f: impl FnOnce(&mut dyn JsHost, f64) -> Result<String, String>) -> (Vec<js_engine::DomCommand>, bool) {
        let Some(live) = self.live.as_ref() else { return (Vec::new(), false) };
        *live.slot.borrow_mut() = self.document.take();
        let out = f(self.js.as_mut(), self.now_ms);
        let live = self.live.as_ref().unwrap();
        self.document = live.slot.borrow_mut().take();
        let mut cmds = match out {
            Ok(json) => js_engine::parse_commands(&json),
            Err(e) => {
                log::warn!("JS: {e}");
                Vec::new()
            }
        };
        if let Some(doc) = self.document.as_mut() {
            js_engine::apply_commands(doc, &cmds);
        }
        cmds.append(&mut live.log.borrow_mut());
        (cmds, live.structural.replace(false))
    }

    /// Repaints for what a script did on the live DOM. Returns true if it did.
    fn after_js(&mut self, cmds: Vec<js_engine::DomCommand>, structural: bool) -> bool {
        let canvases = self.draw_canvases();
        if structural {
            self.render();
            return true;
        }
        let visible = cmds.iter().any(|c| !matches!(c, js_engine::DomCommand::ConsoleLog { .. }));
        if !visible && canvases.is_empty() {
            return false;
        }
        if !self.update_drawings_only(&cmds, &canvases) {
            self.render();
        }
        true
    }

    fn set_focus(&mut self, id: Option<String>) -> bool {
        if self.focused_input == id {
            return false;
        }
        self.focused_input = id;
        // The caret is drawn as part of the input's text.
        self.render();
        true
    }

    /// Commands that only change attributes inside inline `<svg>` drawings (a
    /// needle's transform, a level's height) change no style and no layout: the
    /// drawings are redrawn into their boxes and only those are repainted. Returns
    /// false, having changed nothing visible, when the commands need a full render.
    fn update_drawings_only(&mut self, cmds: &[js_engine::DomCommand], canvases: &[(String, Option<std::sync::Arc<Vec<u8>>>)]) -> bool {
        let (Some(styled), Some(layout)) = (self.styled.as_mut(), self.layout.as_mut()) else { return false };
        if cmds.is_empty() || self.pixels.is_empty() || std::env::var_os("MB_FULL_RENDER").is_some() {
            return false;
        }
        // Each command's drawing, by its path of child indices in the styled tree.
        let mut drawings: Vec<Vec<usize>> = Vec::new();
        for c in cmds {
            if let js_engine::DomCommand::Canvas { .. } = c {
                continue;
            }
            let js_engine::DomCommand::SetAttribute { id, name, .. } = c else {
                if std::env::var_os("MB_TRACE").is_some() {
                    eprintln!("full render for {c:?}");
                }
                return false;
            };
            if matches!(name.as_str(), "id" | "class" | "style") {
                return false;
            }
            let Some(path) = find_path(styled, id) else { return false };
            // The nearest <svg> above the element (not the element: its size is layout).
            let Some(svg) = (0..path.len()).rev().find(|&n| is_tag(node_at(styled, &path[..n]), "svg")) else { return false };
            if !drawings.iter().any(|d| d[..] == path[..svg]) {
                drawings.push(path[..svg].to_vec());
            }
        }
        let mut phases = Phases::start();
        // The pixels each drawing shows now, to find its box.
        let mut old = Vec::new();
        for d in &drawings {
            match crate::svg::render(node_at(styled, d), &mut self.svg) {
                Some(((px, _, _), _)) => old.push(px),
                None => return false,
            }
        }
        let mut boxes = Vec::new();
        for px in &old {
            let mut found = Vec::new();
            find_image_boxes(layout, px, &mut Vec::new(), &mut found);
            // Two identical drawings: which one changed is unclear.
            if found.len() != 1 {
                return false;
            }
            boxes.push(found.pop().unwrap());
        }
        phases.mark("svg-find");
        for c in cmds {
            if let js_engine::DomCommand::SetAttribute { id, name, value } = c {
                let path = find_path(styled, id).expect("found above");
                if let dom::NodeType::Element(d) = &mut node_at_mut(styled, &path).node_type {
                    d.attributes.insert(name.clone(), value.clone());
                }
            }
        }
        for ((d, b), before) in drawings.iter().zip(&boxes).zip(&old) {
            let Some(((px, iw, ih), _)) = crate::svg::render(node_at(styled, d), &mut self.svg) else { return false };
            phases.mark("svg-render");
            let lb = layout_at_mut(layout, b);
            let changed = changed_rect(before, &px, iw, ih);
            if let layout::BoxContent::Image { data, .. } = &mut lb.content {
                *data = px;
            }
            let Some(changed) = changed else { continue };
            // Drawn at its own size: repaint just the pixels that changed.
            let r = if (lb.width, lb.height) == (iw as f32, ih as f32) {
                let (x, y) = (lb.content_x().floor() as i32, lb.content_y().floor() as i32);
                [x + changed[0] - 1, y + changed[1] - 1, x + changed[2] + 1, y + changed[3] + 1]
            } else {
                crate::damage::own(lb)
            };
            paint::repaint(&mut self.pixels, self.page_w, self.page_h, layout, r);
            self.damage.push(r);
            phases.mark("svg-paint");
        }
        for (id, before) in canvases {
            let Some(before) = before else { return false };
            let mut found = Vec::new();
            find_image_boxes(layout, before, &mut Vec::new(), &mut found);
            let ([b], Some(c)) = (&found[..], self.canvases.get_mut(id)) else { return false };
            let (iw, ih) = c.size();
            let px = c.image();
            let lb = layout_at_mut(layout, b);
            let changed = changed_rect(before, &px, iw, ih);
            if let layout::BoxContent::Image { data, .. } = &mut lb.content {
                *data = px;
            }
            let Some(changed) = changed else { continue };
            let r = if (lb.width, lb.height) == (iw as f32, ih as f32) {
                let (x, y) = (lb.content_x().floor() as i32, lb.content_y().floor() as i32);
                [x + changed[0] - 1, y + changed[1] - 1, x + changed[2] + 1, y + changed[3] + 1]
            } else {
                crate::damage::own(lb)
            };
            paint::repaint(&mut self.pixels, self.page_w, self.page_h, layout, r);
            self.damage.push(r);
        }
        phases.mark("canvas");
        phases.report();
        true
    }

    /// Draws the canvas calls scripts made since the last time. Returns the
    /// canvases drawn on, each with the picture it showed before.
    fn draw_canvases(&mut self) -> Vec<(String, Option<std::sync::Arc<Vec<u8>>>)> {
        let Some(doc) = self.document.as_mut() else { return Vec::new() };
        let mut ops = std::mem::take(&mut doc.canvas_ops);
        for (_, op) in &mut ops {
            if op.blob != 0 && op.data.is_empty() {
                op.data = self.js.take_blob(op.blob).unwrap_or_default();
            }
        }
        let mut touched: Vec<(String, Option<std::sync::Arc<Vec<u8>>>, (u32, u32))> = Vec::new();
        for (id, op) in &ops {
            let size = match touched.iter().find(|t| &t.0 == id) {
                Some(t) => t.2,
                None => {
                    let size = canvas_size(&doc.root, id);
                    let before = self.canvases.get_mut(id).map(|c| c.image());
                    touched.push((id.clone(), before, size));
                    size
                }
            };
            self.canvases.apply(id, size, op);
        }
        touched.into_iter().map(|(id, before, _)| (id, before)).collect()
    }


    /// Styles, lays out and paints the whole page from the current DOM.
    fn render(&mut self) {
        let Some(document) = self.document.as_ref() else { return };
        self.pending_images.clear();
        let mut phases = Phases::start();

        let mut styled = css::resolve_styles(document);
        if self.quirks {
            css::apply_body_margin_quirk(&mut styled);
        }
        phases.mark("style");
        let mut cx = Cx {
            focused: self.focused_input.as_deref(),
            images: &self.images,
            pending: &mut self.pending_images,
            svg: &mut self.svg,
            svg_used: Vec::new(),
            canvases: &mut self.canvases,
        };
        let layout_input = convert_styled(&styled.root, &mut cx);
        let used = std::mem::take(&mut cx.svg_used);
        self.svg.retain_used(&used);
        phases.mark("convert+svg");
        // Lay out against the full content height, not the viewport.
        let layout_root = match self.engine {
            LayoutEngine::Builtin => layout::layout_tree(&layout_input, self.width as f32, 100000.0),
            LayoutEngine::Taffy => crate::layout_taffy::layout_tree(&layout_input, self.width as f32, self.height as f32),
        };

        phases.mark("layout");
        let total_height = layout_root.margin_box_height().max(self.height as f32);
        let total_width = layout_root.margin_box_width().max(self.width as f32);

        let mut regions = Vec::new();
        collect_hit_regions(&styled.root, &layout_root, &mut regions, self.live.is_some());
        if let Some(live) = &self.live {
            *live.rects.borrow_mut() = regions.iter().map(|r| (r.uid, [r.x, r.y, r.w, r.h])).collect();
        }
        self.hit_regions = regions;

        let page_w = (total_width as u32).max(self.width).min(4000);
        let page_h = (total_height as u32).min(8000).max(self.height);
        // Same page size and an earlier picture: repaint only what changed, unless
        // that is most of the page anyway.
        let partial = match &self.layout {
            Some(old) if (page_w, page_h) == (self.page_w, self.page_h) && !self.pixels.is_empty() => {
                let rects = crate::damage::damage(old, &layout_root);
                (crate::damage::area(&rects) * 2 < page_w as i64 * page_h as i64).then_some(rects)
            }
            _ => None,
        };
        match partial {
            Some(rects) => {
                for r in &rects {
                    paint::repaint(&mut self.pixels, page_w, page_h, &layout_root, *r);
                }
                if std::env::var_os("MB_CHECK_FRAMES").is_some() {
                    let fresh = paint::paint(&layout_root, page_w, page_h);
                    let bad = (0..fresh.len() / 4).filter(|&i| (0..3).any(|c| fresh[i * 4 + c].abs_diff(self.pixels[i * 4 + c]) > 2)).collect::<Vec<_>>();
                    if !bad.is_empty() {
                        let i = bad[0] as u32;
                        eprintln!("frame diverges: {} px, first {},{}; rects {:?}", bad.len(), i % page_w, i / page_w, rects);
                    }
                }
                self.damage.extend(rects);
            }
            None => {
                self.page_w = page_w;
                self.page_h = page_h;
                self.pixels = paint::paint(&layout_root, page_w, page_h);
                self.damage = vec![[0, 0, page_w as i32, page_h as i32]];
            }
        }
        phases.mark("paint");
        phases.report();
        self.layout = Some(layout_root);
        self.styled = Some(styled.root);
        self.stats = Stats {
            node_count: count_nodes(&document.root),
            box_count: layout::count_boxes(self.layout.as_ref().unwrap()),
            content_width: total_width,
            content_height: total_height,
        };
    }
}

/// A canvas's pixel size: its width/height attributes, else 300×150.
fn canvas_attr_size(d: &dom::ElementData) -> (u32, u32) {
    let get = |k: &str, def: u32| d.attributes.get(k).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(def).clamp(1, 4096);
    (get("width", 300), get("height", 150))
}

fn canvas_size(n: &dom::Node, id: &str) -> (u32, u32) {
    fn find<'a>(n: &'a dom::Node, id: &str) -> Option<&'a dom::ElementData> {
        if let dom::NodeType::Element(d) = &n.node_type {
            if d.attributes.get("id").map(String::as_str) == Some(id) {
                return Some(d);
            }
        }
        n.children.iter().find_map(|c| find(c, id))
    }
    find(n, id).map(canvas_attr_size).unwrap_or((300, 150))
}

fn is_tag(n: &css::StyledNode, tag: &str) -> bool {
    matches!(&n.node_type, dom::NodeType::Element(d) if d.tag_name.eq_ignore_ascii_case(tag))
}

fn node_at<'a>(mut n: &'a css::StyledNode, path: &[usize]) -> &'a css::StyledNode {
    for &i in path {
        n = &n.children[i];
    }
    n
}

fn node_at_mut<'a>(mut n: &'a mut css::StyledNode, path: &[usize]) -> &'a mut css::StyledNode {
    for &i in path {
        n = &mut n.children[i];
    }
    n
}

fn layout_at_mut<'a>(mut b: &'a mut layout::LayoutBox, path: &[usize]) -> &'a mut layout::LayoutBox {
    for &i in path {
        b = &mut b.children[i];
    }
    b
}

/// Child-index path to the element with `id`.
fn find_path(n: &css::StyledNode, id: &str) -> Option<Vec<usize>> {
    if matches!(&n.node_type, dom::NodeType::Element(d) if d.attributes.get("id").map(String::as_str) == Some(id)) {
        return Some(Vec::new());
    }
    n.children.iter().enumerate().find_map(|(i, c)| {
        find_path(c, id).map(|mut p| {
            p.insert(0, i);
            p
        })
    })
}

/// The bounding box `[x0, y0, x1, y1)` of the pixels that differ between two
/// `w`×`h` RGBA images (None: identical).
fn changed_rect(a: &[u8], b: &[u8], w: u32, h: u32) -> Option<[i32; 4]> {
    if a.len() != b.len() || a.len() != (w * h * 4) as usize {
        return Some([0, 0, w as i32, h as i32]);
    }
    let line = w as usize * 4;
    let rows: Vec<usize> = (0..h as usize).filter(|&y| a[y * line..(y + 1) * line] != b[y * line..(y + 1) * line]).collect();
    let (&y0, &y1) = (rows.first()?, rows.last()?);
    let (mut x0, mut x1) = (w as usize, 0);
    for &y in &rows {
        let (ra, rb) = (&a[y * line..(y + 1) * line], &b[y * line..(y + 1) * line]);
        if let Some(i) = (0..w as usize).find(|&x| ra[x * 4..x * 4 + 4] != rb[x * 4..x * 4 + 4]) {
            x0 = x0.min(i);
        }
        if let Some(i) = (0..w as usize).rev().find(|&x| ra[x * 4..x * 4 + 4] != rb[x * 4..x * 4 + 4]) {
            x1 = x1.max(i + 1);
        }
    }
    Some([x0 as i32, y0 as i32, x1 as i32, y1 as i32 + 1])
}

/// Paths to the image boxes showing exactly `px`.
fn find_image_boxes(b: &layout::LayoutBox, px: &std::sync::Arc<Vec<u8>>, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    if let layout::BoxContent::Image { data, .. } = &b.content {
        // The box holds the cached drawing itself; equal pixels are another copy of it.
        if std::sync::Arc::ptr_eq(data, px) || (data.len() == px.len() && data[..] == px[..]) {
            out.push(path.clone());
        }
    }
    for (i, c) in b.children.iter().enumerate() {
        path.push(i);
        find_image_boxes(c, px, path, out);
        path.pop();
    }
}

/// A page without `<!DOCTYPE ...>` before its first element is in quirks mode.
fn is_quirks(html: &str) -> bool {
    let head: String = html.chars().take(1024).collect::<String>().to_ascii_lowercase();
    match (head.find("<!doctype"), head.find("<html").or_else(|| head.find("<body"))) {
        (Some(d), Some(e)) => d > e,
        (Some(_), None) => false,
        (None, _) => true,
    }
}

/// Per-phase render timings, logged when MB_TRACE is set (native builds only:
/// wasm32 has no clock in std).
struct Phases {
    #[cfg(not(target_arch = "wasm32"))]
    at: Option<(std::time::Instant, Vec<(&'static str, f64)>)>,
}

impl Phases {
    fn start() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let on = std::env::var_os("MB_TRACE").is_some();
            Phases { at: on.then(|| (std::time::Instant::now(), Vec::new())) }
        }
        #[cfg(target_arch = "wasm32")]
        Phases {}
    }

    fn mark(&mut self, _name: &'static str) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((t, v)) = self.at.as_mut() {
            v.push((_name, t.elapsed().as_secs_f64() * 1000.0));
            *t = std::time::Instant::now();
        }
    }

    fn report(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((_, v)) = &self.at {
            let parts: Vec<String> = v.iter().map(|(n, ms)| format!("{n} {ms:.1}")).collect();
            eprintln!("render: {} ms", parts.join(", "));
        }
    }
}

/// What `convert_styled` needs from the page besides the styled tree.
struct Cx<'a> {
    focused: Option<&'a str>,
    images: &'a HashMap<String, (Vec<u8>, u32, u32)>,
    pending: &'a mut Vec<String>,
    svg: &'a mut crate::svg::SvgCache,
    /// Cache keys of the drawings this render used.
    svg_used: Vec<(String, u32, u32)>,
    canvases: &'a mut crate::canvas::Canvases,
}

impl Cx<'_> {
    /// Decodes an `<img src>`: a data: URI directly, anything else from the cache
    /// (or recorded as pending for the frontend to fetch). Empty if unavailable.
    fn image(&mut self, src: &str) -> (Vec<u8>, u32, u32) {
        if let Some(cached) = self.images.get(src) {
            return cached.clone();
        }
        if !src.starts_with("data:image/") {
            if !src.is_empty() && !self.pending.iter().any(|u| u == src) {
                self.pending.push(src.to_string());
            }
            return (Vec::new(), 0, 0);
        }
        // Parse data:[<mediatype>][;base64],<data>
        let Some(comma) = src.find(',') else { return (Vec::new(), 0, 0) };
        if !src[..comma].contains(";base64") {
            return (Vec::new(), 0, 0);
        }
        let Some(decoded) = base64_decode(&src[comma + 1..]) else { return (Vec::new(), 0, 0) };
        match image::load_from_memory(&decoded) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (w, h) = (rgba.width(), rgba.height());
                (rgba.into_raw(), w, h)
            }
            Err(_) => (Vec::new(), 0, 0),
        }
    }
}

/// The children that get boxes. Whitespace-only text next to block-level
/// siblings makes no line box in CSS (it is collapsible white space between
/// blocks), so it is dropped; among inline siblings it is a space and stays.
fn laid_out_children(sn: &css::StyledNode) -> impl Iterator<Item = &css::StyledNode> {
    // In a flex or grid container, blank text makes no item either.
    let has_blocks = matches!(sn.display(), "flex" | "inline-flex" | "grid" | "inline-grid")
        || sn.children.iter().any(|c| {
            matches!(c.node_type, dom::NodeType::Element(_)) && !matches!(c.display(), "inline" | "none")
        });
    sn.children.iter().filter(move |c| {
        !(has_blocks && matches!(&c.node_type, dom::NodeType::Text(t) if t.trim().is_empty()))
    })
}

/// (id, value) of every `<input>`/`<textarea>` that has an id.
fn input_values(node: &dom::Node) -> Vec<(String, String)> {
    fn collect(node: &dom::Node, out: &mut Vec<(String, String)>) {
        if let dom::NodeType::Element(ref data) = node.node_type {
            if matches!(data.tag_name.as_str(), "input" | "textarea") {
                if let Some(id) = data.id() {
                    out.push((id.to_string(), data.attributes.get("value").cloned().unwrap_or_default()));
                }
            }
        }
        for child in &node.children {
            collect(child, out);
        }
    }
    let mut out = Vec::new();
    collect(node, &mut out);
    out
}

/// Update an input element's value attribute based on key press.
fn update_input_value(node: &mut dom::Node, id: &str, key: &str) -> bool {
    if let dom::NodeType::Element(ref mut data) = node.node_type {
        if data.id() == Some(id) && matches!(data.tag_name.as_str(), "input" | "textarea") {
            let current = data.attributes.get("value").cloned().unwrap_or_default();
            let new_value = match key {
                "Backspace" => {
                    let mut chars: Vec<char> = current.chars().collect();
                    chars.pop();
                    chars.into_iter().collect()
                }
                "Enter" => current, // don't modify, but could trigger onsubmit later
                k if k.len() == 1 => format!("{}{}", current, k),
                _ => current, // ignore special keys
            };
            data.attributes.insert("value".to_string(), new_value.clone());
            // Also update placeholder display: remove placeholder text, set value as child
            node.children.clear();
            if !new_value.is_empty() {
                node.children.push(dom::Node::new_text(&new_value));
            }
            return true;
        }
    }
    for child in &mut node.children {
        if update_input_value(child, id, key) {
            return true;
        }
    }
    false
}

/// Get the current value of an input element.
fn get_input_value(node: &dom::Node, id: &str) -> Option<String> {
    if let dom::NodeType::Element(ref data) = node.node_type {
        if data.id() == Some(id) {
            return data.attributes.get("value").cloned();
        }
    }
    for child in &node.children {
        if let Some(v) = get_input_value(child, id) {
            return Some(v);
        }
    }
    None
}

/// Walk the css::StyledNode tree and layout::LayoutBox tree in parallel,
/// collecting elements that have `onclick` attributes.
fn collect_hit_regions(
    styled: &css::StyledNode,
    layout_box: &layout::LayoutBox,
    out: &mut Vec<HitRegion>,
    all: bool,
) {
    if let dom::NodeType::Element(ref data) = styled.node_type {
        let onclick = data.attributes.get("onclick").cloned().unwrap_or_default();
        let id = data.id().unwrap_or("").to_string();
        let is_input = matches!(data.tag_name.as_str(), "input" | "textarea");

        if !onclick.is_empty() || is_input || all {
            let bx = layout_box.x + layout_box.margin.left;
            let by = layout_box.y + layout_box.margin.top;
            let bw = layout_box.border.left
                + layout_box.padding.left
                + layout_box.width
                + layout_box.padding.right
                + layout_box.border.right;
            let bh = layout_box.border.top
                + layout_box.padding.top
                + layout_box.height
                + layout_box.padding.bottom
                + layout_box.border.bottom;

            out.push(HitRegion {
                x: bx,
                y: by,
                w: bw,
                h: bh,
                onclick,
                id,
                is_input,
                uid: styled.uid,
            });
        }
    }

    // Recurse into children that are visible
    let visible_styled: Vec<&css::StyledNode> = laid_out_children(styled)
        .filter(|c| {
            c.get("display").map(|v| {
                if let css::CSSValue::Keyword(s) = v { s != "none" } else { true }
            }).unwrap_or(true)
        })
        .collect();

    for (i, child_styled) in visible_styled.iter().enumerate() {
        if let Some(child_layout) = layout_box.children.get(i) {
            collect_hit_regions(child_styled, child_layout, out, all);
        }
    }
}

fn count_nodes(node: &dom::Node) -> usize {
    1 + node.children.iter().map(|c| count_nodes(c)).sum::<usize>()
}

/// Simple base64 decoder (no padding required).
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let table: [u8; 256] = {
        let mut t = [255u8; 256];
        for (i, &c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".iter().enumerate() {
            t[c as usize] = i as u8;
        }
        t[b'=' as usize] = 0;
        t
    };
    let bytes: Vec<u8> = input.bytes().filter(|&b| b != b'\n' && b != b'\r' && b != b' ').collect();
    if bytes.len() % 4 != 0 && bytes.len() % 4 != 2 && bytes.len() % 4 != 3 {
        // Try with padding
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let chunks = bytes.chunks(4);
    for chunk in chunks {
        let mut buf = [0u8; 4];
        let len = chunk.len();
        for (i, &b) in chunk.iter().enumerate() {
            let v = table[b as usize];
            if v == 255 { return None; }
            buf[i] = v;
        }
        out.push((buf[0] << 2) | (buf[1] >> 4));
        if len > 2 && chunk[2] != b'=' {
            out.push((buf[1] << 4) | (buf[2] >> 2));
        }
        if len > 3 && chunk[3] != b'=' {
            out.push((buf[2] << 6) | buf[3]);
        }
    }
    Some(out)
}

/// Convert css::StyledNode → layout::StyledNode
fn convert_styled(sn: &css::StyledNode, cx: &mut Cx) -> layout::StyledNode {
    use css::CSSValue;

    let get_str = |prop: &str| -> String {
        sn.get(prop).map(|v| match v {
            CSSValue::Keyword(s) => s.clone(),
            _ => String::new(),
        }).unwrap_or_default()
    };

    let get_length = |prop: &str, default: f32| -> f32 {
        sn.get(prop).map(|v| match v {
            CSSValue::Length(n, _) => *n,
            CSSValue::Number(n) => *n,
            _ => default,
        }).unwrap_or(default)
    };

    let get_length_or_shorthand = |longhand: &str, shorthand: &str, default: f32| -> f32 {
        if let Some(v) = sn.get(longhand) {
            return match v {
                CSSValue::Length(n, _) => *n,
                CSSValue::Number(n) => *n,
                _ => default,
            };
        }
        if let Some(v) = sn.get(shorthand) {
            return match v {
                CSSValue::Length(n, _) => *n,
                CSSValue::Number(n) => *n,
                _ => default,
            };
        }
        default
    };

    let get_color = |prop: &str, default: layout::Color| -> layout::Color {
        sn.get(prop).map(|v| match v {
            CSSValue::Color(r, g, b, a) => layout::Color { r: *r, g: *g, b: *b, a: *a },
            _ => default,
        }).unwrap_or(default)
    };

    let get_dim = |prop: &str| -> layout::Dimension {
        match sn.get(prop) {
            Some(CSSValue::Length(n, css::Unit::Percent)) => layout::Dimension::Percent(*n),
            Some(CSSValue::Length(n, _)) => layout::Dimension::Px(*n),
            Some(CSSValue::Number(n)) => layout::Dimension::Px(*n),
            Some(CSSValue::Auto) => layout::Dimension::Auto,
            _ => layout::Dimension::Auto,
        }
    };

    let display = match get_str("display").as_str() {
        "none" => layout::Display::None,
        "inline" => layout::Display::Inline,
        "flex" => layout::Display::Flex,
        "grid" => layout::Display::Grid,
        _ => layout::Display::Block,
    };

    let position = match get_str("position").as_str() {
        "absolute" => layout::Position::Absolute,
        "relative" => layout::Position::Relative,
        "fixed" => layout::Position::Fixed,
        _ => layout::Position::Static,
    };

    let bg = sn.get("background-color").map(|v| match v {
        CSSValue::Color(r, g, b, a) => layout::Color { r: *r, g: *g, b: *b, a: *a },
        _ => layout::Color { r: 0, g: 0, b: 0, a: 0 },
    });

    // An empty input shows its placeholder, dimmed like browsers' ::placeholder.
    let mut showing_placeholder = false;
    let content = match &sn.node_type {
        dom::NodeType::Text(s) => layout::NodeContent::Text(s.clone()),
        dom::NodeType::Element(data) => {
            match data.tag_name.as_str() {
                "input" => {
                    let input_id = data.id().unwrap_or("");
                    let value = data.attributes.get("value").cloned().unwrap_or_default();
                    let is_focused = cx.focused == Some(input_id) && !input_id.is_empty();

                    if !value.is_empty() {
                        // Show value with cursor if focused
                        let display = if is_focused {
                            format!("{}|", value)
                        } else {
                            value
                        };
                        layout::NodeContent::Text(display)
                    } else if is_focused {
                        layout::NodeContent::Text("|".to_string())
                    } else {
                        let placeholder = data.attributes.get("placeholder")
                            .cloned()
                            .unwrap_or_default();
                        if placeholder.is_empty() {
                            layout::NodeContent::Element
                        } else {
                            showing_placeholder = true;
                            layout::NodeContent::Text(placeholder)
                        }
                    }
                }
                "canvas" => {
                    let id = data.attributes.get("id").map(String::as_str).unwrap_or("");
                    let (w, h) = canvas_attr_size(data);
                    // A canvas never drawn on is transparent.
                    let px = match cx.canvases.get_mut(id) {
                        Some(c) if c.size() == (w, h) => c.image(),
                        _ => std::sync::Arc::new(vec![0; (w * h * 4) as usize]),
                    };
                    layout::NodeContent::Image { alt: String::new(), data: px, intrinsic_w: w, intrinsic_h: h }
                }
                "svg" => match crate::svg::render(sn, cx.svg) {
                    Some(((data, w, h), key)) => {
                        cx.svg_used.push(key);
                        layout::NodeContent::Image { alt: String::new(), data, intrinsic_w: w, intrinsic_h: h }
                    }
                    None => layout::NodeContent::Element,
                },
                "img" => {
                    let alt = data.attributes.get("alt").cloned().unwrap_or_default();
                    let src = data.attributes.get("src").cloned().unwrap_or_default();
                    let (img_data, iw, ih) = cx.image(&src);
                    layout::NodeContent::Image {
                        alt,
                        data: std::sync::Arc::new(img_data),
                        intrinsic_w: iw,
                        intrinsic_h: ih,
                    }
                }
                "select" => layout::NodeContent::Text("Select...".to_string()),
                "textarea" => {
                    let placeholder = data.attributes.get("placeholder")
                        .cloned()
                        .unwrap_or_default();
                    if placeholder.is_empty() {
                        layout::NodeContent::Element
                    } else {
                        layout::NodeContent::Text(placeholder)
                    }
                }
                _ => layout::NodeContent::Element,
            }
        }
        dom::NodeType::Document => layout::NodeContent::Element,
    };

    let font_size = get_length("font-size", 16.0);

    // Determine if element uses border-box sizing.
    // Browsers default to border-box for input, textarea, select, button.
    let is_border_box = match &sn.node_type {
        dom::NodeType::Element(data) => {
            let explicit_box_sizing = match sn.get("box-sizing") {
                Some(CSSValue::Keyword(k)) => k.as_str() == "border-box",
                _ => false,
            };
            explicit_box_sizing || matches!(data.tag_name.as_str(), "input" | "textarea" | "select" | "button")
        }
        _ => false,
    };

    // For border-box elements, convert explicit width/height from border-box to content-box
    // by subtracting padding and border widths.
    let raw_width = get_dim("width");
    let raw_height = get_dim("height");
    let (adj_width, adj_height) = if is_border_box {
        let pl = get_length_or_shorthand("padding-left", "padding", 0.0);
        let pr = get_length_or_shorthand("padding-right", "padding", 0.0);
        let pt = get_length_or_shorthand("padding-top", "padding", 0.0);
        let pb = get_length_or_shorthand("padding-bottom", "padding", 0.0);
        let bl = get_length_or_shorthand("border-left-width", "border-width", 0.0);
        let br = get_length_or_shorthand("border-right-width", "border-width", 0.0);
        let bt = get_length_or_shorthand("border-top-width", "border-width", 0.0);
        let bb = get_length_or_shorthand("border-bottom-width", "border-width", 0.0);
        let w = match raw_width {
            layout::Dimension::Px(w) => layout::Dimension::Px((w - pl - pr - bl - br).max(0.0)),
            d => d,
        };
        let h = match raw_height {
            layout::Dimension::Px(h) => layout::Dimension::Px((h - pt - pb - bt - bb).max(0.0)),
            d => d,
        };
        (w, h)
    } else {
        (raw_width, raw_height)
    };

    layout::StyledNode {
        display,
        position,
        width: adj_width,
        height: adj_height,
        margin: layout::EdgeSizes {
            top: get_length_or_shorthand("margin-top", "margin", 0.0),
            right: match sn.get("margin-right").or_else(|| sn.get("margin")) {
                // auto: the same sentinel as margin-left (only the Taffy layout reads it)
                Some(CSSValue::Auto) => f32::MAX / 2.0,
                _ => get_length_or_shorthand("margin-right", "margin", 0.0),
            },
            bottom: get_length_or_shorthand("margin-bottom", "margin", 0.0),
            left: {
                // margin-left: auto → sentinel value for flex auto-margin
                let ml_val = sn.get("margin-left").or_else(|| sn.get("margin"));
                match ml_val {
                    Some(CSSValue::Auto) => f32::MAX / 2.0,
                    _ => get_length_or_shorthand("margin-left", "margin", 0.0),
                }
            },
        },
        padding: layout::EdgeSizes {
            top: get_length_or_shorthand("padding-top", "padding", 0.0),
            right: get_length_or_shorthand("padding-right", "padding", 0.0),
            bottom: get_length_or_shorthand("padding-bottom", "padding", 0.0),
            left: get_length_or_shorthand("padding-left", "padding", 0.0),
        },
        border: layout::EdgeSizes {
            top: get_length_or_shorthand("border-top-width", "border-width", 0.0),
            right: get_length_or_shorthand("border-right-width", "border-width", 0.0),
            bottom: get_length_or_shorthand("border-bottom-width", "border-width", 0.0),
            left: get_length_or_shorthand("border-left-width", "border-width", 0.0),
        },
        background_color: bg,
        text_color: {
            let c = get_color("color", layout::Color { r: 0, g: 0, b: 0, a: 255 });
            if showing_placeholder { layout::Color { a: (c.a as f32 * 0.55) as u8, ..c } } else { c }
        },
        border_color: sn.get("border-color").map(|v| match v {
            CSSValue::Color(r, g, b, a) => layout::Color { r: *r, g: *g, b: *b, a: *a },
            _ => layout::Color::BLACK,
        }),
        font_size,
        text_align: match get_str("text-align").as_str() {
            "center" => layout::TextAlign::Center,
            "right" => layout::TextAlign::Right,
            _ => layout::TextAlign::Left,
        },
        opacity: get_length("opacity", 1.0),
        gap: get_length("gap", 0.0),
        flex_wrap: match get_str("flex-wrap").as_str() {
            "wrap" => layout::FlexWrap::Wrap,
            _ => layout::FlexWrap::NoWrap,
        },
        align_items: match get_str("align-items").as_str() {
            "center" => layout::AlignItems::Center,
            "flex-start" | "start" => layout::AlignItems::FlexStart,
            "flex-end" | "end" => layout::AlignItems::FlexEnd,
            _ => layout::AlignItems::Stretch,
        },
        min_height: get_dim("min-height"),
        line_height: {
            match sn.get("line-height") {
                Some(CSSValue::Length(n, _)) => *n,
                Some(CSSValue::Number(n)) => *n * font_size,
                Some(CSSValue::Keyword(k)) if k == "normal" => 0.0,
                _ => 0.0, // 0.0 means "use default 1.2 * font_size"
            }
        },
        white_space_nowrap: {
            match sn.get("white-space") {
                Some(CSSValue::Keyword(k)) => k == "nowrap" || k == "pre",
                _ => false,
            }
        },
        float_right: {
            match sn.get("float") {
                Some(CSSValue::Keyword(k)) => k == "right",
                _ => false,
            }
        },
        border_radius: get_length("border-radius", 0.0),
        text_decoration: match get_str("text-decoration").as_str() {
            "underline" => layout::TextDecoration::Underline,
            "line-through" => layout::TextDecoration::LineThrough,
            _ => layout::TextDecoration::None,
        },
        offset_top: get_length("top", 0.0),
        offset_left: get_length("left", 0.0),
        inset: ["top", "right", "bottom", "left"].map(|k| match sn.get(k) {
            Some(CSSValue::Length(n, _)) | Some(CSSValue::Number(n)) => Some(*n),
            _ => None,
        }),
        grid_template_columns: parse_grid_template_columns(sn),
        grid_gap: get_length("grid-gap", get_length("gap", 0.0)),
        overflow_hidden: matches!(get_str("overflow").as_str(), "hidden" | "clip" | "auto" | "scroll"),
        border_side_colors: ["top", "right", "bottom", "left"].map(|side| match sn.get(&format!("border-{side}-color")) {
            Some(CSSValue::Color(r, g, b, a)) => Some(layout::Color { r: *r, g: *g, b: *b, a: *a }),
            _ => None,
        }),
        bold: match sn.get("font-weight") {
            Some(CSSValue::Keyword(k)) => matches!(k.as_str(), "bold" | "bolder")
                || k.parse::<f32>().map_or(false, |w| w >= 600.0),
            Some(CSSValue::Number(w)) => *w >= 600.0,
            _ => false,
        },
        family: crate::font::Family::from_css(&get_str("font-family")),
        border_box: is_border_box,
        min_width: get_dim("min-width"),
        max_width: get_dim("max-width"),
        max_height: get_dim("max-height"),
        grid: layout::GridProps {
            columns: get_str("grid-template-columns"),
            rows: get_str("grid-template-rows"),
            column: get_str("grid-column"),
            row: get_str("grid-row"),
        },
        flex: {
            let mut f = layout::FlexProps::default();
            f.direction = match get_str("flex-direction").as_str() {
                "column" => layout::FlexDirection::Column,
                "column-reverse" => layout::FlexDirection::ColumnReverse,
                "row-reverse" => layout::FlexDirection::RowReverse,
                _ => layout::FlexDirection::Row,
            };
            f.justify = match get_str("justify-content").as_str() {
                "flex-end" | "end" | "right" => layout::JustifyContent::FlexEnd,
                "center" => layout::JustifyContent::Center,
                "space-between" => layout::JustifyContent::SpaceBetween,
                "space-around" => layout::JustifyContent::SpaceAround,
                "space-evenly" => layout::JustifyContent::SpaceEvenly,
                _ => layout::JustifyContent::FlexStart,
            };
            // `flex: <grow> [<shrink>] [<basis>]` or the keywords; longhands win.
            match sn.get("flex") {
                Some(CSSValue::Number(n)) => { f.grow = *n; f.basis = layout::Dimension::Percent(0.0); }
                Some(CSSValue::Keyword(k)) => {
                    let parts: Vec<&str> = k.split_whitespace().collect();
                    match parts.as_slice() {
                        ["none"] => { f.grow = 0.0; f.shrink = 0.0; }
                        ["auto"] => { f.grow = 1.0; }
                        [g, rest @ ..] if g.parse::<f32>().is_ok() => {
                            f.grow = g.parse().unwrap_or(0.0);
                            f.basis = layout::Dimension::Percent(0.0);
                            for p in rest {
                                if let Ok(v) = p.parse::<f32>() { f.shrink = v; }
                                else if let Some(v) = p.strip_suffix("px").and_then(|v| v.parse().ok()) { f.basis = layout::Dimension::Px(v); }
                                else if let Some(v) = p.strip_suffix('%').and_then(|v| v.parse().ok()) { f.basis = layout::Dimension::Percent(v); }
                            }
                        }
                        _ => {}
                    }
                }
                Some(CSSValue::Length(n, css::Unit::Percent)) => { f.grow = 1.0; f.basis = layout::Dimension::Percent(*n); }
                Some(CSSValue::Length(n, _)) => { f.grow = 1.0; f.basis = layout::Dimension::Px(*n); }
                _ => {}
            }
            let num = |p: &str| match sn.get(p) {
                Some(CSSValue::Number(n)) | Some(CSSValue::Length(n, _)) => Some(*n),
                Some(CSSValue::Keyword(k)) => k.parse().ok(),
                _ => None,
            };
            if let Some(g) = num("flex-grow") { f.grow = g; }
            if let Some(s) = num("flex-shrink") { f.shrink = s; }
            if sn.get("flex-basis").is_some() { f.basis = get_dim("flex-basis"); }
            f.align_self = match get_str("align-self").as_str() {
                "center" => Some(layout::AlignItems::Center),
                "flex-start" | "start" => Some(layout::AlignItems::FlexStart),
                "flex-end" | "end" => Some(layout::AlignItems::FlexEnd),
                "stretch" => Some(layout::AlignItems::Stretch),
                _ => None,
            };
            f
        },
        content,
        children: {
            // An <svg>'s children are its drawing, already rendered, not boxes.
            let is_svg = matches!(&sn.node_type, dom::NodeType::Element(d) if d.tag_name == "svg");
            let mut kids: Vec<layout::StyledNode> = if is_svg {
                Vec::new()
            } else {
                laid_out_children(sn).map(|c| convert_styled(c, cx)).collect()
            };
            // White space at the start and end of a block's (or button's) content
            // is at a line edge, where CSS removes it.
            let starts_lines = match &sn.node_type {
                dom::NodeType::Element(d) => sn.display() != "inline" || matches!(d.tag_name.as_str(), "button" | "select"),
                _ => false,
            };
            if starts_lines {
                if let Some(layout::StyledNode { content: layout::NodeContent::Text(t), .. }) = kids.first_mut() {
                    *t = t.trim_start().to_string();
                }
                if let Some(layout::StyledNode { content: layout::NodeContent::Text(t), .. }) = kids.last_mut() {
                    *t = t.trim_end().to_string();
                }
            }
            kids
        },
    }
}

fn parse_grid_template_columns(sn: &css::StyledNode) -> Vec<layout::GridTrack> {
    use css::CSSValue;
    let val = match sn.get("grid-template-columns") {
        Some(CSSValue::Keyword(s)) => s.clone(),
        _ => return Vec::new(),
    };
    val.split_whitespace()
        .filter_map(|part| {
            if let Some(v) = part.strip_suffix("fr") {
                v.parse::<f32>().ok().map(layout::GridTrack::Fr)
            } else if let Some(v) = part.strip_suffix("px") {
                v.parse::<f32>().ok().map(layout::GridTrack::Px)
            } else if let Ok(v) = part.parse::<f32>() {
                Some(layout::GridTrack::Px(v))
            } else {
                None
            }
        })
        .collect()
}
