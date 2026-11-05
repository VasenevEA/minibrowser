//! mb-fb — shows an HTML page full-screen on a Linux framebuffer, with touch.
//!
//!   mb-fb page.html [--display auto|kms|fb] [--fb /dev/fb0] [--card /dev/dri/card0]
//!                   [--input /dev/input/event0] [--probe-touch]
//!
//! Output goes through DRM/KMS when the board has it (double buffered, flips on
//! vsync), else to the fbdev framebuffer.
//!
//! Scripts run on QuickJS; taps become clicks, timers fire on time, and the
//! framebuffer is only written when the page changed. The page file is watched
//! and reloaded when it changes, so a new version can simply be copied over it.
//! MB_TOUCH takes "swap", "invx", "invy" for a panel mounted the other way;
//! MB_TRACE=1 logs each frame's timings; MB_LAYOUT=builtin picks the old layout.

use std::fs;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use mb_core::Page;
use mb_qjs::QuickJs;
use mb_fb::{arg, open_screen, touch_thread};

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

// ---- main ----------------------------------------------------------------------------------

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// The engine a page runs on: mb-core's own, or (feature `blitz`, flag
/// `--engine blitz`) Blitz's DOM, Stylo and Taffy painted by tiny-skia.
enum Engine {
    Core(Page),
    #[cfg(feature = "blitz")]
    Blitz(mb_blitz::BlitzPage),
}

macro_rules! each {
    ($e:expr, $p:ident => $body:expr) => {
        match $e {
            Engine::Core($p) => $body,
            #[cfg(feature = "blitz")]
            Engine::Blitz($p) => $body,
        }
    };
}

impl Engine {
    fn new(args: &[String], js: Box<dyn mb_core::JsHost>, w: u32, h: u32) -> Self {
        let name = std::env::var("MB_ENGINE").ok().unwrap_or_else(|| arg(args, "--engine", "core"));
        match name.as_str() {
            #[cfg(feature = "blitz")]
            "blitz" => Engine::Blitz(mb_blitz::BlitzPage::new(js, w, h)),
            "core" => {
                let mut page = Page::new(js, w, h);
                page.set_layout_engine(mb_core::LayoutEngine::from_env());
                Engine::Core(page)
            }
            other => {
                eprintln!("engine {other}: not built in, using core");
                Engine::Core(Page::new(js, w, h))
            }
        }
    }
    fn load(&mut self, html: &str) { each!(self, p => p.load(html)) }
    fn tick(&mut self, now: f64) -> bool { each!(self, p => p.tick(now)) }
    fn click(&mut self, x: f32, y: f32) -> bool { each!(self, p => p.click(x, y)) }
    fn next_timer(&self, now: f64) -> Option<f64> { each!(self, p => p.next_timer(now)) }
    fn take_damage(&mut self) -> Vec<[i32; 4]> { each!(self, p => p.take_damage()) }
    fn viewport(&self) -> Vec<u8> { each!(self, p => p.viewport()) }
    /// The pixels when they are the screen as is (not scrolled).
    fn screen(&self) -> Option<&[u8]> {
        match self {
            Engine::Core(p) => p.viewport_slice().filter(|_| p.scroll_pos() == (0.0, 0.0)),
            #[cfg(feature = "blitz")]
            Engine::Blitz(p) => p.viewport_slice(),
        }
    }
    fn describe(&self) -> String {
        match self {
            Engine::Core(p) => {
                let s = p.stats();
                format!("{} nodes, {} boxes", s.node_count, s.box_count)
            }
            #[cfg(feature = "blitz")]
            Engine::Blitz(p) => format!("{} nodes (blitz)", p.node_count()),
        }
    }
}

fn mtime(path: &str) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1).filter(|a| !a.starts_with("--")).cloned() else {
        eprintln!("usage: mb-fb page.html [--engine core|blitz] [--fb /dev/fb0] [--input /dev/input/event0] [--probe-touch]");
        std::process::exit(2);
    };
    let probe = args.iter().any(|a| a == "--probe-touch");
    // MB_TRACE=1 logs every frame's timings.
    let trace = std::env::var_os("MB_TRACE").is_some();

    let mut fb = open_screen(&args);
    let (w, h) = fb.size();
    let _ = fs::write("/sys/class/graphics/fb0/blank", "0");
    let _ = fs::write("/sys/class/graphics/fbcon/cursor_blink", "0");
    let _ = fs::write("/dev/tty1", "\x1b[?25l\x1b[2J");

    let (tx, rx) = mpsc::channel();
    let input = arg(&args, "--input", "/dev/input/event0");
    std::thread::spawn(move || touch_thread(input, (w, h), tx, probe));

    let clock = Instant::now();
    let now_ms = move || clock.elapsed().as_secs_f64() * 1000.0;
    // fetch() reads files next to the page and in /tmp.
    let dir = std::path::Path::new(&path).parent().map(|p| p.to_path_buf()).unwrap_or_else(|| ".".into());
    let mut page = Engine::new(&args, Box::new(QuickJs::with_files(&dir)), w, h);
    let mut loaded = None;
    loop {
        // (Re)load when the file appears or changes.
        let m = mtime(&path);
        if m.is_some() && m != loaded {
            match fs::read_to_string(&path) {
                Ok(html) => {
                    let t = Instant::now();
                    page.tick(now_ms());
                    page.load(&html);
                    eprintln!("load {path}: {:.0} ms, {}", ms(t), page.describe());
                    let t = Instant::now();
                    page.take_damage();
                    fb.present(&page.viewport(), w, &[[0, 0, w as i32, h as i32]]);
                    eprintln!("present {:.0} ms", ms(t));
                }
                Err(e) => eprintln!("{path}: {e}"),
            }
            loaded = m;
        }

        // Sleep until the next timer, a tap, or the next file check (1 s).
        let wait = page.next_timer(now_ms()).unwrap_or(1000.0).clamp(0.0, 1000.0);
        let mut dirty = false;
        if let Ok((x, y)) = rx.recv_timeout(Duration::from_micros((wait * 1000.0) as u64)) {
            let t = Instant::now();
            dirty |= page.click(x, y);
            eprintln!("tap {x:.0},{y:.0}: {} {:.0} ms", if dirty { "repaint" } else { "-" }, ms(t));
            while let Ok((x, y)) = rx.try_recv() {
                dirty |= page.click(x, y);
            }
        }
        let t = Instant::now();
        let ticked = page.tick(now_ms());
        let t_tick = ms(t);
        dirty |= ticked;
        if dirty {
            let t = Instant::now();
            // Unscrolled, only the repainted rectangles changed; otherwise all of it.
            let full = [[0, 0, w as i32, h as i32]];
            let damage = page.take_damage();
            match page.screen() {
                Some(px) => fb.present(px, w, &damage),
                None => fb.present(&page.viewport(), w, &full),
            }
            if trace {
                eprintln!("frame: timers+layout+paint {t_tick:.0} ms, present {:.0} ms", ms(t));
            }
        }
    }
}
