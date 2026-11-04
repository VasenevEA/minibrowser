//! MiniBrowser for the web: the mb-core engine drawing into a `<canvas>`.
//!
//! The exported functions keep the v0.7 API that `www/` uses; all state lives
//! in one [`Page`] per tab.

mod browser_js;

use std::cell::RefCell;

use mb_core::Page;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, RequestMode, Response};

pub use mb_core::{css, dom, html_parser, js_engine, layout, paint};
pub use browser_js::BrowserJs;

thread_local! {
    static PAGE: RefCell<Option<Page>> = RefCell::new(None);
    /// The HTML of the loaded page: rendering the same HTML again keeps its DOM and JS state.
    static LAST_HTML: RefCell<String> = RefCell::new(String::new());
}

#[wasm_bindgen]
pub fn init() {
    wasm_logger::init(wasm_logger::Config::default());
    log::info!("minibrowser v0.8 initialized");
}

fn canvas(canvas_id: &str) -> web_sys::HtmlCanvasElement {
    web_sys::window().unwrap().document().unwrap()
        .get_element_by_id(canvas_id).unwrap()
        .dyn_into::<web_sys::HtmlCanvasElement>().unwrap()
}

fn now_ms() -> f64 {
    web_sys::window().and_then(|w| w.performance()).map(|p| p.now()).unwrap_or(0.0)
}

/// Runs `f` on the page (creating it at the canvas size) and returns its result.
fn with_page<R>(canvas_id: &str, f: impl FnOnce(&mut Page) -> R) -> R {
    let c = canvas(canvas_id);
    PAGE.with(|p| {
        let mut p = p.borrow_mut();
        let page = p.get_or_insert_with(|| Page::new(Box::new(BrowserJs), c.width(), c.height()));
        page.resize(c.width(), c.height());
        f(page)
    })
}

/// Puts the page's viewport on the canvas.
fn blit(canvas_id: &str, page: &Page) {
    let (w, h) = page.size();
    let ctx = canvas(canvas_id).get_context("2d").unwrap().unwrap()
        .dyn_into::<web_sys::CanvasRenderingContext2d>().unwrap();
    let pixels = page.viewport();
    let data = web_sys::ImageData::new_with_u8_clamped_array_and_sh(wasm_bindgen::Clamped(&pixels), w, h).unwrap();
    ctx.put_image_data(&data, 0.0, 0.0).unwrap();
}

fn stats(page: &Page, t0: f64) -> JsValue {
    let s = page.stats();
    let o = js_sys::Object::new();
    let set = |k: &str, v: JsValue| { js_sys::Reflect::set(&o, &k.into(), &v).unwrap(); };
    set("renderTime", (now_ms() - t0).into());
    set("nodeCount", (s.node_count as u32).into());
    set("boxCount", (s.box_count as u32).into());
    set("contentHeight", (s.content_height as u32).into());
    o.into()
}

#[wasm_bindgen]
pub async fn render_url(canvas_id: &str, url: &str) -> Result<JsValue, JsValue> {
    let window = web_sys::window().unwrap();

    let opts = RequestInit::new();
    opts.set_method("GET");
    opts.set_mode(RequestMode::Cors);

    let request = Request::new_with_str_and_init(url, &opts)
        .map_err(|e| JsValue::from_str(&format!("Request error: {:?}", e)))?;

    let resp_val = JsFuture::from(window.fetch_with_request(&request)).await?;
    let resp: Response = resp_val.dyn_into()?;
    let text_val = JsFuture::from(resp.text()?).await?;
    let html = text_val.as_string().unwrap_or_default();

    Ok(render_html(canvas_id, &html))
}

/// Renders `html`. The same HTML as last time keeps the page (its DOM and JS state)
/// and only repaints; new HTML loads a fresh page.
#[wasm_bindgen]
pub fn render_html(canvas_id: &str, html: &str) -> JsValue {
    let t0 = now_ms();
    let same = LAST_HTML.with(|h| h.borrow().as_str() == html);
    with_page(canvas_id, |page| {
        if same {
            page.tick(now_ms());
            page.rerender();
        } else {
            LAST_HTML.with(|h| *h.borrow_mut() = html.to_string());
            page.tick(now_ms());
            page.load(html);
        }
        blit(canvas_id, page);
        stats(page, t0)
    })
}

/// Live re-render (the page's timer tick): re-runs its scripts on the kept JS state,
/// as v0.7 did, so pages that redraw from a top-level script stay current.
#[wasm_bindgen]
pub fn render_html_live(canvas_id: &str, html: &str) -> JsValue {
    if LAST_HTML.with(|h| h.borrow().as_str() != html) {
        return render_html(canvas_id, html);
    }
    let t0 = now_ms();
    with_page(canvas_id, |page| {
        page.tick(now_ms());
        page.reload(html);
        blit(canvas_id, page);
        stats(page, t0)
    })
}

#[wasm_bindgen]
pub fn click_at(canvas_id: &str, x: f32, y: f32) -> JsValue {
    let t0 = now_ms();
    with_page(canvas_id, |page| {
        if !page.click(x, y) {
            return JsValue::NULL;
        }
        blit(canvas_id, page);
        stats(page, t0)
    })
}

#[wasm_bindgen]
pub fn key_press(canvas_id: &str, key: &str) -> JsValue {
    let t0 = now_ms();
    with_page(canvas_id, |page| {
        if !page.key(key) {
            return JsValue::NULL;
        }
        blit(canvas_id, page);
        stats(page, t0)
    })
}

#[wasm_bindgen]
pub fn scroll(canvas_id: &str, delta_x: f32, delta_y: f32) -> JsValue {
    with_page(canvas_id, |page| {
        page.scroll(delta_x, delta_y);
        blit(canvas_id, page);
        let (x, y) = page.scroll_pos();
        let o = js_sys::Object::new();
        js_sys::Reflect::set(&o, &"scrollX".into(), &x.into()).unwrap();
        js_sys::Reflect::set(&o, &"scrollY".into(), &y.into()).unwrap();
        o.into()
    })
}

#[wasm_bindgen]
pub fn get_focused_input() -> JsValue {
    PAGE.with(|p| match p.borrow().as_ref().and_then(|p| p.focused_input()) {
        Some(id) => JsValue::from_str(id),
        None => JsValue::NULL,
    })
}

/// Image URLs the last render needs; JS fetches them and calls `preload_image`.
#[wasm_bindgen]
pub fn get_image_urls() -> JsValue {
    let arr = js_sys::Array::new();
    PAGE.with(|p| {
        if let Some(page) = p.borrow().as_ref() {
            for url in page.pending_images() {
                arr.push(&JsValue::from_str(url));
            }
        }
    });
    arr.into()
}

#[wasm_bindgen]
pub fn preload_image(url: &str, data: &[u8]) -> bool {
    PAGE.with(|p| p.borrow_mut().as_mut().map(|page| page.add_image(url, data)).unwrap_or(false))
}
