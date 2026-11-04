//! MiniBrowser engine: HTML → DOM → styles → layout → RGBA pixels.
//!
//! No platform code lives here. Frontends (`minibrowser` for wasm, `mb-fb` for a
//! Linux framebuffer, `mb-headless` for PNG files) drive a [`page::Page`] and
//! supply a [`js_engine::JsHost`].

pub mod css;
pub mod dom;
pub mod font;
pub mod html_parser;
pub mod js_engine;
pub mod layout;
pub mod page;
pub mod paint;

pub use js_engine::JsHost;
pub use page::{Page, Stats};

/// A host for pages without scripts: everything runs, nothing happens.
pub struct NoJs;

impl JsHost for NoJs {
    fn reset(&mut self) {}
    fn run(&mut self, _code: &str, _now_ms: f64) -> Result<String, String> { Ok("[]".into()) }
    fn tick(&mut self, _now_ms: f64) -> Result<String, String> { Ok("[]".into()) }
    fn next_timer(&self, _now_ms: f64) -> Option<f64> { None }
    fn set_input_value(&mut self, _id: &str, _value: &str) {}
}
