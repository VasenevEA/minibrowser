//! MiniBrowser — modular browser engine in Rust/WASM.
//! WASM entry point.

pub mod dom;
pub mod html_parser;
pub mod css;
pub mod font;
pub mod js_engine;
pub mod layout;
pub mod paint;

use std::cell::RefCell;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, RequestMode, Response};

// ---------------------------------------------------------------------------
// Interactivity: hit-testing state
// ---------------------------------------------------------------------------

/// A clickable region on the canvas, tied to a DOM element.
#[derive(Debug, Clone)]
struct HitRegion {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    onclick: String,
    /// Element ID (for focus tracking on inputs).
    id: String,
    /// Whether this is a focusable input element.
    is_input: bool,
}

thread_local! {
    /// The last raw HTML string (for detecting page changes).
    static LAST_HTML: RefCell<String> = RefCell::new(String::new());
    /// The DOM *after* initial script execution — persists across re-renders and clicks.
    static LAST_DOCUMENT: RefCell<Option<dom::Document>> = RefCell::new(None);
    /// Clickable regions collected after the most recent render.
    static HIT_REGIONS: RefCell<Vec<HitRegion>> = RefCell::new(Vec::new());
    /// ID of the currently focused input element (if any).
    static FOCUSED_INPUT: RefCell<Option<String>> = RefCell::new(None);
    /// Last canvas ID used (for re-rendering after key input).
    static LAST_CANVAS_ID: RefCell<String> = RefCell::new(String::new());
    /// Scroll offset (Y position of viewport).
    static SCROLL_Y: RefCell<f32> = RefCell::new(0.0);
    /// Scroll offset (X position of viewport).
    static SCROLL_X: RefCell<f32> = RefCell::new(0.0);
    /// Total content height (for scroll bounds).
    static CONTENT_HEIGHT: RefCell<f32> = RefCell::new(0.0);
    /// Total content width (for scroll bounds).
    static CONTENT_WIDTH: RefCell<f32> = RefCell::new(0.0);
    /// Cached full-page pixel buffer.
    static LAST_PIXELS: RefCell<Option<Vec<u8>>> = RefCell::new(None);
    /// Cached full-page dimensions.
    static LAST_PAGE_SIZE: RefCell<(u32, u32)> = RefCell::new((0, 0));
    /// Image cache: maps src URL to decoded RGBA pixel data + (width, height).
    static IMAGE_CACHE: RefCell<HashMap<String, (Vec<u8>, u32, u32)>> = RefCell::new(HashMap::new());
    /// Image URLs found during the last render (for JS to fetch).
    static PENDING_IMAGE_URLS: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

#[wasm_bindgen]
pub fn init() {
    wasm_logger::init(wasm_logger::Config::default());
    log::info!("minibrowser v0.7 initialized");
}

/// Returns a JS array of image URLs that need fetching (found during last render).
#[wasm_bindgen]
pub fn get_image_urls() -> JsValue {
    PENDING_IMAGE_URLS.with(|urls| {
        let urls = urls.borrow();
        let arr = js_sys::Array::new();
        for url in urls.iter() {
            arr.push(&JsValue::from_str(url));
        }
        arr.into()
    })
}

/// Preload an image from raw bytes (fetched by JS). Decodes and caches it.
#[wasm_bindgen]
pub fn preload_image(url: &str, data: &[u8]) -> bool {
    match image::load_from_memory(data) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let w = rgba.width();
            let h = rgba.height();
            log::info!("preload_image: cached {}x{} for {}", w, h, url);
            IMAGE_CACHE.with(|cache| {
                cache.borrow_mut().insert(url.to_string(), (rgba.into_raw(), w, h));
            });
            true
        }
        Err(e) => {
            log::warn!("preload_image: failed to decode {}: {}", url, e);
            false
        }
    }
}

#[wasm_bindgen]
pub fn render_html(canvas_id: &str, html: &str) -> JsValue {
    render_to_canvas(canvas_id, html, false)
}

/// Re-render for live timer pages: always re-parses HTML and re-executes scripts.
/// JS variables persist via (0,eval) global scope, so counters/state survive.
#[wasm_bindgen]
pub fn render_html_live(canvas_id: &str, html: &str) -> JsValue {
    render_to_canvas(canvas_id, html, true)
}

/// Handle a key press while an input element is focused.
/// Updates the input's value in the DOM and re-renders.
#[wasm_bindgen]
pub fn key_press(canvas_id: &str, key: &str) -> JsValue {
    let focused_id = FOCUSED_INPUT.with(|f| f.borrow().clone());
    let focused_id = match focused_id {
        Some(id) => id,
        None => return JsValue::NULL,
    };

    let mut document = match LAST_DOCUMENT.with(|d| d.borrow().clone()) {
        Some(doc) => doc,
        None => return JsValue::NULL,
    };

    // Find the input element and update its value
    let updated = update_input_value(&mut document.root, &focused_id, key);
    if !updated {
        return JsValue::NULL;
    }

    // Also update the mock element in JS so scripts can read input.value
    let sync_code = format!(
        "if (window.__mb_elements && window.__mb_elements['{}']) {{ window.__mb_elements['{}']._attrs['value'] = '{}'; }}",
        focused_id,
        focused_id,
        get_input_value(&document.root, &focused_id).unwrap_or_default().replace('\'', "\\'")
    );
    let _ = js_sys::eval(&sync_code);

    // Store updated DOM
    LAST_DOCUMENT.with(|d| *d.borrow_mut() = Some(document.clone()));

    // Re-render
    let html = LAST_HTML.with(|h| h.borrow().clone());
    if !html.is_empty() {
        return render_to_canvas(canvas_id, &html, false);
    }
    JsValue::NULL
}

/// Get the ID of the currently focused input (if any). Called from JS for cursor display.
#[wasm_bindgen]
pub fn get_focused_input() -> JsValue {
    FOCUSED_INPUT.with(|f| {
        match &*f.borrow() {
            Some(id) => JsValue::from_str(id),
            None => JsValue::NULL,
        }
    })
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

/// Force-create mock elements for input fields that have values,
/// ensuring they exist in window.__mb_elements before onclick runs.
fn force_create_mock_inputs(doc: &dom::Document) {
    fn collect(node: &dom::Node, inputs: &mut Vec<(String, String)>) {
        if let dom::NodeType::Element(ref data) = node.node_type {
            if matches!(data.tag_name.as_str(), "input" | "textarea") {
                if let Some(id) = data.id() {
                    let value = data.attributes.get("value").cloned().unwrap_or_default();
                    inputs.push((id.to_string(), value));
                }
            }
        }
        for child in &node.children {
            collect(child, inputs);
        }
    }
    let mut inputs = Vec::new();
    collect(&doc.root, &mut inputs);

    for (id, value) in inputs {
        let escaped_value = value.replace('\\', "\\\\").replace('\'', "\\'");
        let code = format!(
            r#"(function() {{
                if (!window.__mb_elements) window.__mb_elements = {{}};
                if (!window.__mb_elements['{id}']) {{
                    window.__mb_elements['{id}'] = new MockElement('{id}');
                }}
                window.__mb_elements['{id}']._attrs['value'] = '{val}';
                window.__mb_elements['{id}']._text = '{val}';
            }})()"#,
            id = id,
            val = escaped_value
        );
        let _ = js_sys::eval(&code);
    }
}

/// Sync all input element values from DOM attributes to JS mock elements.
/// This ensures that when onclick handlers read input.value, they get the typed text.
fn sync_input_values_to_js(doc: &dom::Document) {
    fn collect_inputs(node: &dom::Node, inputs: &mut Vec<(String, String)>) {
        if let dom::NodeType::Element(ref data) = node.node_type {
            if matches!(data.tag_name.as_str(), "input" | "textarea") {
                if let Some(id) = data.id() {
                    let value = data.attributes.get("value").cloned().unwrap_or_default();
                    inputs.push((id.to_string(), value));
                }
            }
        }
        for child in &node.children {
            collect_inputs(child, inputs);
        }
    }

    let mut inputs = Vec::new();
    collect_inputs(&doc.root, &mut inputs);

    for (id, value) in inputs {
        let escaped_value = value.replace('\\', "\\\\").replace('\'', "\\'");
        let code = format!(
            "if (window.__mb_elements && window.__mb_elements['{}']) {{ window.__mb_elements['{}']._attrs['value'] = '{}'; }}",
            id, id, escaped_value
        );
        let _ = js_sys::eval(&code);
    }
}

#[wasm_bindgen]
pub async fn render_url(canvas_id: &str, url: &str) -> Result<JsValue, JsValue> {
    let window = web_sys::window().unwrap();

    let mut opts = RequestInit::new();
    opts.method("GET");
    opts.mode(RequestMode::Cors);

    let request = Request::new_with_str_and_init(url, &opts)
        .map_err(|e| JsValue::from_str(&format!("Request error: {:?}", e)))?;

    let resp_val = JsFuture::from(window.fetch_with_request(&request)).await?;
    let resp: Response = resp_val.dyn_into()?;
    let text_val = JsFuture::from(resp.text()?).await?;
    let html = text_val.as_string().unwrap_or_default();

    Ok(render_to_canvas(canvas_id, &html, false))
}

fn render_to_canvas(canvas_id: &str, html: &str, force_reexecute: bool) -> JsValue {
    // Clear pending image URLs for this render pass
    PENDING_IMAGE_URLS.with(|urls| urls.borrow_mut().clear());

    let perf = web_sys::window().unwrap()
        .performance().unwrap();
    let t0 = perf.now();

    // Check if this is the same page as last render.
    // If yes, use the stored DOM (preserves script modifications).
    // If no, parse fresh and execute scripts.
    let is_same_page = LAST_HTML.with(|h| h.borrow().as_str() == html);

    let document = if is_same_page && !force_reexecute {
        // Re-render of the same page (resize etc.): use stored DOM, run timer callbacks
        let mut doc = LAST_DOCUMENT.with(|d| d.borrow().clone())
            .unwrap_or_else(|| html_parser::parse_html(html));
        // Execute registered interval callbacks (for clocks, etc.)
        let tick_count = js_engine::tick_timers(&mut doc);
        if tick_count > 0 {
            LAST_DOCUMENT.with(|d| *d.borrow_mut() = Some(doc.clone()));
        }
        doc
    } else if is_same_page && force_reexecute {
        // Live re-render (timer tick): re-parse HTML and re-execute scripts.
        // JS state (window.* vars) persists via (0,eval) global scope.
        let mut doc = html_parser::parse_html(html);
        let script_count = js_engine::execute_scripts(&mut doc);
        if script_count > 0 {
            log::info!("Live re-render: {} script(s)", script_count);
        }
        // Update stored DOM with fresh script results
        LAST_DOCUMENT.with(|d| *d.borrow_mut() = Some(doc.clone()));
        doc
    } else {
        // New page: parse HTML, reset JS state, execute scripts
        LAST_HTML.with(|h| *h.borrow_mut() = html.to_string());
        js_engine::reset_js_state();

        let mut doc = html_parser::parse_html(html);

        let script_count = js_engine::execute_scripts(&mut doc);
        if script_count > 0 {
            log::info!("Executed {} script(s), DOM updated", script_count);
        }

        // Store the post-script DOM for future re-renders and clicks
        LAST_DOCUMENT.with(|d| *d.borrow_mut() = Some(doc.clone()));

        doc
    };

    // Resolve styles → StyledTree
    let styled = css::resolve_styles(&document);

    // Get canvas dimensions
    let window = web_sys::window().unwrap();
    let doc = window.document().unwrap();
    let canvas = doc.get_element_by_id(canvas_id)
        .unwrap()
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .unwrap();
    let width = canvas.width();
    let height = canvas.height();

    // Layout — use full content height, not viewport height
    let layout_input = convert_styled(&styled.root);
    let layout_root = layout::layout_tree(&layout_input, width as f32, 100000.0);

    // Calculate total content dimensions
    let total_height = layout_root.margin_box_height().max(height as f32);
    let total_width = layout_root.margin_box_width().max(width as f32);
    CONTENT_HEIGHT.with(|h| *h.borrow_mut() = total_height);
    CONTENT_WIDTH.with(|w| *w.borrow_mut() = total_width);

    // Reset scroll on new page
    if !is_same_page {
        SCROLL_Y.with(|s| *s.borrow_mut() = 0.0);
        SCROLL_X.with(|s| *s.borrow_mut() = 0.0);
    }

    // Collect hit regions
    let mut regions = Vec::new();
    collect_hit_regions(&styled.root, &layout_root, &mut regions);
    let onclick_count = regions.iter().filter(|r| !r.onclick.is_empty()).count();
    log::info!("Hit regions: {} total, {} with onclick", regions.len(), onclick_count);
    HIT_REGIONS.with(|hr| *hr.borrow_mut() = regions);

    // Paint full page
    let render_width = (total_width as u32).max(width).min(4000);
    let render_height = (total_height as u32).min(8000);
    let pixels = paint::paint(&layout_root, render_width, render_height);

    // Store full page pixels for scroll reuse
    LAST_PIXELS.with(|p| *p.borrow_mut() = Some(pixels.clone()));
    LAST_PAGE_SIZE.with(|s| *s.borrow_mut() = (render_width, render_height));

    // Blit the visible viewport portion
    let scroll_x = SCROLL_X.with(|s| *s.borrow());
    let scroll_y = SCROLL_Y.with(|s| *s.borrow());
    blit_viewport_2d(&canvas, &pixels, render_width, render_height, width, height, scroll_x, scroll_y);

    let elapsed = perf.now() - t0;

    // Return stats
    let stats = js_sys::Object::new();
    js_sys::Reflect::set(&stats, &"renderTime".into(), &elapsed.into()).unwrap();
    js_sys::Reflect::set(&stats, &"nodeCount".into(), &(count_nodes(&document.root) as u32).into()).unwrap();
    js_sys::Reflect::set(&stats, &"boxCount".into(), &(layout::count_boxes(&layout_root) as u32).into()).unwrap();
    js_sys::Reflect::set(&stats, &"contentHeight".into(), &(total_height as u32).into()).unwrap();
    stats.into()
}

/// Copy a viewport-sized window from the full-page pixel buffer to the canvas (Y scroll only).
fn blit_viewport(
    canvas: &web_sys::HtmlCanvasElement,
    full_pixels: &[u8],
    page_width: u32,
    page_height: u32,
    viewport_height: u32,
    scroll_y: f32,
) {
    blit_viewport_2d(canvas, full_pixels, page_width, page_height, page_width, viewport_height, 0.0, scroll_y);
}

/// Copy a viewport-sized window from the full-page pixel buffer to the canvas (X+Y scroll).
fn blit_viewport_2d(
    canvas: &web_sys::HtmlCanvasElement,
    full_pixels: &[u8],
    page_width: u32,
    page_height: u32,
    viewport_width: u32,
    viewport_height: u32,
    scroll_x: f32,
    scroll_y: f32,
) {
    let ctx = canvas.get_context("2d").unwrap().unwrap()
        .dyn_into::<web_sys::CanvasRenderingContext2d>().unwrap();

    let src_x = (scroll_x as u32).min(page_width.saturating_sub(viewport_width));
    let src_y = (scroll_y as u32).min(page_height.saturating_sub(viewport_height));
    let copy_w = viewport_width.min(page_width - src_x);
    let copy_h = viewport_height.min(page_height - src_y);

    let mut viewport_pixels = vec![0u8; (viewport_width * viewport_height * 4) as usize];

    for row in 0..copy_h {
        let src_row = src_y + row;
        let src_offset = ((src_row * page_width + src_x) * 4) as usize;
        let dst_offset = (row * viewport_width * 4) as usize;
        let row_bytes = (copy_w * 4) as usize;

        if src_offset + row_bytes <= full_pixels.len() && dst_offset + row_bytes <= viewport_pixels.len() {
            viewport_pixels[dst_offset..dst_offset + row_bytes]
                .copy_from_slice(&full_pixels[src_offset..src_offset + row_bytes]);
        }
    }

    let clamped = wasm_bindgen::Clamped(viewport_pixels.as_slice());
    let image_data = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
        clamped, viewport_width, viewport_height
    ).unwrap();
    ctx.put_image_data(&image_data, 0.0, 0.0).unwrap();
}

/// Scroll the viewport by delta_x and delta_y.
/// Re-blits the cached pixel buffer without re-rendering.
#[wasm_bindgen]
pub fn scroll(canvas_id: &str, delta_x: f32, delta_y: f32) -> JsValue {
    let content_h = CONTENT_HEIGHT.with(|h| *h.borrow());
    let content_w = CONTENT_WIDTH.with(|w| *w.borrow());

    let window = web_sys::window().unwrap();
    let doc = window.document().unwrap();
    let canvas = doc.get_element_by_id(canvas_id)
        .unwrap()
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .unwrap();
    let viewport_h = canvas.height() as f32;
    let viewport_w = canvas.width() as f32;

    // Update scroll positions
    let max_scroll_y = (content_h - viewport_h).max(0.0);
    let max_scroll_x = (content_w - viewport_w).max(0.0);

    SCROLL_Y.with(|s| {
        let mut sy = s.borrow_mut();
        *sy = (*sy + delta_y).clamp(0.0, max_scroll_y);
    });
    SCROLL_X.with(|s| {
        let mut sx = s.borrow_mut();
        *sx = (*sx + delta_x).clamp(0.0, max_scroll_x);
    });

    let scroll_y = SCROLL_Y.with(|s| *s.borrow());
    let scroll_x = SCROLL_X.with(|s| *s.borrow());

    // Blit from cached pixels
    let blitted = LAST_PIXELS.with(|p| {
        let pixels = p.borrow();
        if let Some(ref px) = *pixels {
            let (pw, ph) = LAST_PAGE_SIZE.with(|s| *s.borrow());
            blit_viewport_2d(&canvas, px, pw, ph, canvas.width(), canvas.height(), scroll_x, scroll_y);
            true
        } else {
            false
        }
    });

    if !blitted {
        return JsValue::NULL;
    }

    let stats = js_sys::Object::new();
    js_sys::Reflect::set(&stats, &"scrollX".into(), &scroll_x.into()).unwrap();
    js_sys::Reflect::set(&stats, &"scrollY".into(), &scroll_y.into()).unwrap();
    stats.into()
}

// ---------------------------------------------------------------------------
// Hit-testing: click_at
// ---------------------------------------------------------------------------

/// Handle a click at (x, y) on the canvas. If a hit region with an onclick
/// handler is found, execute the onclick, re-render, and return stats.
/// The DOM from the last render is reused and mutated (state persists).
#[wasm_bindgen]
pub fn click_at(canvas_id: &str, x: f32, y: f32) -> JsValue {
    // Translate viewport coordinates to page coordinates (add scroll offset)
    let scroll_x = SCROLL_X.with(|s| *s.borrow());
    let scroll_y = SCROLL_Y.with(|s| *s.borrow());
    let page_x = x + scroll_x;
    let page_y = y + scroll_y;

    // Find the hit region under (x, page_y). Search in reverse order so that
    // later (more deeply nested / on-top) elements win.
    let hit = HIT_REGIONS.with(|hr| {
        let regions = hr.borrow();
        for region in regions.iter().rev() {
            if page_x >= region.x
                && page_x <= region.x + region.w
                && page_y >= region.y
                && page_y <= region.y + region.h
                && (!region.onclick.is_empty() || region.is_input)
            {
                return Some(region.clone());
            }
        }
        None
    });

    let hit = match hit {
        Some(h) => h,
        None => {
            // Click outside any region → clear focus
            FOCUSED_INPUT.with(|f| *f.borrow_mut() = None);
            return JsValue::NULL;
        }
    };

    // Handle focus: if clicked on an input, focus it
    if hit.is_input && !hit.id.is_empty() {
        log::info!("click_at({}, {}): focusing input #{}", x, y, hit.id);
        FOCUSED_INPUT.with(|f| *f.borrow_mut() = Some(hit.id.clone()));
        LAST_CANVAS_ID.with(|c| *c.borrow_mut() = canvas_id.to_string());
        // Re-render to show cursor
        let html = LAST_HTML.with(|h| h.borrow().clone());
        if !html.is_empty() {
            return render_to_canvas(canvas_id, &html, false);
        }
        return JsValue::NULL;
    }

    // Clear focus if clicking non-input
    FOCUSED_INPUT.with(|f| *f.borrow_mut() = None);

    let onclick_code = hit.onclick;
    if onclick_code.is_empty() {
        return JsValue::NULL;
    }

    log::info!("click_at({}, {}): onclick = {:?}", x, y, onclick_code);

    // Use the stored DOM (with all previous script/click modifications intact)
    let mut document = match LAST_DOCUMENT.with(|d| d.borrow().clone()) {
        Some(doc) => doc,
        None => return JsValue::NULL,
    };

    let perf = web_sys::window().unwrap().performance().unwrap();
    let t0 = perf.now();

    // Sync all input values from DOM to mock elements before running onclick.
    // Ensure mock elements exist even if they weren't accessed during initial script.
    sync_input_values_to_js(&document);
    // Also force-create mock elements for any inputs with values
    force_create_mock_inputs(&document);

    // Execute only the onclick handler (scripts already ran, state persists in window.*)
    js_engine::execute_onclick(&mut document, &onclick_code);

    // Store the updated DOM
    LAST_DOCUMENT.with(|d| *d.borrow_mut() = Some(document.clone()));

    // Resolve styles
    let styled = css::resolve_styles(&document);

    // Get canvas
    let window = web_sys::window().unwrap();
    let doc = window.document().unwrap();
    let canvas = doc.get_element_by_id(canvas_id)
        .unwrap()
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .unwrap();
    let width = canvas.width();
    let height = canvas.height();

    // Layout — full content height (same as render_to_canvas)
    let layout_input = convert_styled(&styled.root);
    let layout_root = layout::layout_tree(&layout_input, width as f32, 100000.0);

    let total_height = layout_root.margin_box_height().max(height as f32);
    CONTENT_HEIGHT.with(|h| *h.borrow_mut() = total_height);

    // Collect new hit regions
    let mut regions = Vec::new();
    collect_hit_regions(&styled.root, &layout_root, &mut regions);
    HIT_REGIONS.with(|hr| *hr.borrow_mut() = regions);

    // Paint full page and cache
    let render_height = (total_height as u32).min(8000);
    let pixels = paint::paint(&layout_root, width, render_height);

    LAST_PIXELS.with(|p| *p.borrow_mut() = Some(pixels.clone()));
    LAST_PAGE_SIZE.with(|s| *s.borrow_mut() = (width, render_height));

    // Blit viewport with scroll
    let scroll_y = SCROLL_Y.with(|s| *s.borrow());
    blit_viewport(&canvas, &pixels, width, render_height, height, scroll_y);

    let elapsed = perf.now() - t0;

    let stats = js_sys::Object::new();
    js_sys::Reflect::set(&stats, &"renderTime".into(), &elapsed.into()).unwrap();
    js_sys::Reflect::set(&stats, &"nodeCount".into(), &(count_nodes(&document.root) as u32).into()).unwrap();
    js_sys::Reflect::set(&stats, &"boxCount".into(), &(layout::count_boxes(&layout_root) as u32).into()).unwrap();
    stats.into()
}

// ---------------------------------------------------------------------------
// Hit region collection
// ---------------------------------------------------------------------------

/// Walk the css::StyledNode tree and layout::LayoutBox tree in parallel,
/// collecting elements that have `onclick` attributes.
fn collect_hit_regions(
    styled: &css::StyledNode,
    layout_box: &layout::LayoutBox,
    out: &mut Vec<HitRegion>,
) {
    if let dom::NodeType::Element(ref data) = styled.node_type {
        let onclick = data.attributes.get("onclick").cloned().unwrap_or_default();
        let id = data.id().unwrap_or("").to_string();
        let is_input = matches!(data.tag_name.as_str(), "input" | "textarea");

        if !onclick.is_empty() || is_input {
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
            });
        }
    }

    // Recurse into children that are visible
    let visible_styled: Vec<&css::StyledNode> = styled.children.iter()
        .filter(|c| {
            c.get("display").map(|v| {
                if let css::CSSValue::Keyword(s) = v { s != "none" } else { true }
            }).unwrap_or(true)
        })
        .collect();

    for (i, child_styled) in visible_styled.iter().enumerate() {
        if let Some(child_layout) = layout_box.children.get(i) {
            collect_hit_regions(child_styled, child_layout, out);
        }
    }
}

fn count_nodes(node: &dom::Node) -> usize {
    1 + node.children.iter().map(|c| count_nodes(c)).sum::<usize>()
}

/// Decode an image from a data: URI. Returns (rgba_pixels, width, height).
/// Returns empty vec if not a data URI or decoding fails.
fn decode_image_src(src: &str) -> (Vec<u8>, u32, u32) {
    // Check image cache first (for URL-fetched images)
    let cached = IMAGE_CACHE.with(|cache| cache.borrow().get(src).cloned());
    if let Some((data, w, h)) = cached {
        return (data, w, h);
    }

    // For non-data URIs, record as pending for JS to fetch
    if !src.is_empty() && !src.starts_with("data:image/") {
        PENDING_IMAGE_URLS.with(|urls| {
            let mut urls = urls.borrow_mut();
            if !urls.contains(&src.to_string()) {
                urls.push(src.to_string());
            }
        });
        return (Vec::new(), 0, 0);
    }

    if !src.starts_with("data:image/") {
        return (Vec::new(), 0, 0);
    }
    // Parse data:[<mediatype>][;base64],<data>
    let after_comma = match src.find(',') {
        Some(pos) => &src[pos + 1..],
        None => return (Vec::new(), 0, 0),
    };
    let is_base64 = src[..src.find(',').unwrap()].contains(";base64");
    if !is_base64 {
        return (Vec::new(), 0, 0);
    }
    // Decode base64
    let decoded = match base64_decode(after_comma) {
        Some(d) => d,
        None => return (Vec::new(), 0, 0),
    };
    // Decode image using the image crate
    match image::load_from_memory(&decoded) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let w = rgba.width();
            let h = rgba.height();
            (rgba.into_raw(), w, h)
        }
        Err(_) => (Vec::new(), 0, 0),
    }
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
fn convert_styled(sn: &css::StyledNode) -> layout::StyledNode {
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

    let content = match &sn.node_type {
        dom::NodeType::Text(s) => layout::NodeContent::Text(s.clone()),
        dom::NodeType::Element(data) => {
            match data.tag_name.as_str() {
                "input" => {
                    let input_id = data.id().unwrap_or("");
                    let value = data.attributes.get("value").cloned().unwrap_or_default();
                    let is_focused = FOCUSED_INPUT.with(|f| {
                        f.borrow().as_deref() == Some(input_id) && !input_id.is_empty()
                    });

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
                            layout::NodeContent::Text(placeholder)
                        }
                    }
                }
                "img" => {
                    let alt = data.attributes.get("alt").cloned().unwrap_or_default();
                    let src = data.attributes.get("src").cloned().unwrap_or_default();
                    let (img_data, iw, ih) = decode_image_src(&src);
                    layout::NodeContent::Image {
                        alt,
                        data: img_data,
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
            right: get_length_or_shorthand("margin-right", "margin", 0.0),
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
        text_color: get_color("color", layout::Color { r: 0, g: 0, b: 0, a: 255 }),
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
        grid_template_columns: parse_grid_template_columns(sn),
        grid_gap: get_length("grid-gap", get_length("gap", 0.0)),
        content,
        children: sn.children.iter().map(|c| convert_styled(c)).collect(),
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
