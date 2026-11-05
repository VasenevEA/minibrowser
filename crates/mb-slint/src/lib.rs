//! MiniBrowser inside Slint: a live HTML/CSS/JS page as a Slint element.
//!
//! In `.slint`, import `HtmlView` from this crate's `ui/html-view.slint` and
//! place it like any element. On the Rust side, [`HtmlView::attach`] takes
//! the page, the element's size and two hooks: one to set the element's
//! `frame`, and the page then runs on a Slint timer (scripts, animation) and
//! takes taps through [`HtmlView::tap`] (wire the element's `tapped` to it).

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// Path of the Slint file with the `HtmlView` element (for `slint-build`
/// library paths or `import`).
pub const SLINT_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/ui/html-view.slint");

/// A page engine: mb-core's, or Blitz's with the `blitz` feature.
pub trait Engine {
    fn load(&mut self, html: &str);
    fn tick(&mut self, now_ms: f64) -> bool;
    fn click(&mut self, x: f32, y: f32) -> bool;
    fn next_timer(&self, now_ms: f64) -> Option<f64>;
    /// RGBA pixels of the viewport.
    fn pixels(&self) -> Vec<u8>;
}

impl Engine for mb_core::Page {
    fn load(&mut self, html: &str) { mb_core::Page::load(self, html) }
    fn tick(&mut self, now_ms: f64) -> bool { mb_core::Page::tick(self, now_ms) }
    fn click(&mut self, x: f32, y: f32) -> bool { mb_core::Page::click(self, x, y) }
    fn next_timer(&self, now_ms: f64) -> Option<f64> { mb_core::Page::next_timer(self, now_ms) }
    fn pixels(&self) -> Vec<u8> { self.viewport() }
}

#[cfg(feature = "blitz")]
impl Engine for mb_blitz::BlitzPage {
    fn load(&mut self, html: &str) { mb_blitz::BlitzPage::load(self, html) }
    fn tick(&mut self, now_ms: f64) -> bool { mb_blitz::BlitzPage::tick(self, now_ms) }
    fn click(&mut self, x: f32, y: f32) -> bool { mb_blitz::BlitzPage::click(self, x, y) }
    fn next_timer(&self, now_ms: f64) -> Option<f64> { mb_blitz::BlitzPage::next_timer(self, now_ms) }
    fn pixels(&self) -> Vec<u8> { self.viewport() }
}

/// A page engine for `size`, its scripts reading files in `dir`. `blitz`
/// picks Blitz when it is built in.
pub fn engine(dir: &Path, size: (u32, u32), blitz: bool) -> Box<dyn Engine> {
    let js = Box::new(mb_qjs::QuickJs::with_files(dir));
    #[cfg(feature = "blitz")]
    if blitz {
        return Box::new(mb_blitz::BlitzPage::new(js, size.0, size.1));
    }
    let _ = blitz;
    let mut page = mb_core::Page::new(js, size.0, size.1);
    page.set_layout_engine(mb_core::LayoutEngine::from_env());
    Box::new(page)
}

struct State {
    engine: Box<dyn Engine>,
    size: (u32, u32),
    started: Instant,
    show: Box<dyn Fn(Image)>,
}

impl State {
    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1e3
    }

    fn present(&self) {
        let (w, h) = self.size;
        let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&self.engine.pixels(), w, h);
        (self.show)(Image::from_rgba8(buf));
    }
}

/// The Rust side of one `HtmlView` element. Keep it alive as long as the
/// element shows the page.
#[derive(Clone)]
pub struct HtmlView {
    state: Rc<RefCell<State>>,
    timer: Rc<slint::Timer>,
}

impl HtmlView {
    /// Loads `html` into `engine` (made for `size`, the element's size in
    /// pixels) and shows it through `show` (set the element's `frame`).
    pub fn attach(mut engine: Box<dyn Engine>, size: (u32, u32), html: &str, show: impl Fn(Image) + 'static) -> Self {
        engine.load(html);
        let state = Rc::new(RefCell::new(State { engine, size, started: Instant::now(), show: Box::new(show) }));
        state.borrow().present();
        let view = HtmlView { state, timer: Rc::new(slint::Timer::default()) };
        view.schedule();
        view
    }

    /// Opens a page file: its scripts read files next to it.
    pub fn open(path: &Path, size: (u32, u32), blitz: bool, show: impl Fn(Image) + 'static) -> std::io::Result<Self> {
        let html = std::fs::read_to_string(path)?;
        let dir = path.parent().unwrap_or(Path::new("."));
        Ok(Self::attach(engine(dir, size, blitz), size, &html, show))
    }

    /// A tap at element coordinates (logical pixels, as `tapped` gives them).
    pub fn tap(&self, x: f32, y: f32) {
        let mut s = self.state.borrow_mut();
        if s.engine.click(x, y) {
            s.present();
        }
        drop(s);
        self.schedule();
    }

    /// Runs the page's due timers and repaints, then sleeps until the next
    /// timer (at most a second, at least a frame).
    fn schedule(&self) {
        let now = self.state.borrow().now();
        let wait = self.state.borrow().engine.next_timer(now).unwrap_or(1000.0).clamp(16.0, 1000.0);
        let this = self.clone();
        self.timer.start(slint::TimerMode::SingleShot, Duration::from_millis(wait as u64), move || {
            {
                let mut s = this.state.borrow_mut();
                let now = s.now();
                if s.engine.tick(now) {
                    s.present();
                }
            }
            this.schedule();
        });
    }
}
