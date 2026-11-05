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

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::io::AsRawFd;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use mb_core::Page;
use mb_qjs::QuickJs;

#[cfg(feature = "kms")]
mod kms;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Where frames go: fbdev or DRM/KMS.
pub trait Screen {
    fn size(&self) -> (u32, u32);
    /// Copies the `rects` (`[x0, y0, x1, y1)` in page pixels) of a `w`-wide RGBA
    /// page onto the screen.
    fn present(&mut self, rgba: &[u8], w: u32, rects: &[[i32; 4]]);
}

fn open_screen(args: &[String]) -> Box<dyn Screen> {
    let want = arg(args, "--display", "auto");
    #[cfg(feature = "kms")]
    if want != "fb" {
        let card = arg(args, "--card", "/dev/dri/card0");
        match kms::Kms::open(&card) {
            Ok(k) => {
                eprintln!("display: kms {card} {:?}", k.size());
                return Box::new(k);
            }
            Err(e) if want == "kms" => panic!("kms {card}: {e}"),
            Err(e) => eprintln!("display: kms {card}: {e}, using fbdev"),
        }
    }
    let path = arg(args, "--fb", "/dev/fb0");
    let fb = Fb::open(&path);
    eprintln!("display: fbdev {path} {:?}", fb.size());
    Box::new(fb)
}

// ---- framebuffer ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Default)]
struct VarInfo {
    xres: u32,
    yres: u32,
    xres_v: u32,
    yres_v: u32,
    xoff: u32,
    yoff: u32,
    bpp: u32,
    gray: u32,
    r: [u32; 3],
    g: [u32; 3],
    b: [u32; 3],
    t: [u32; 3],
    rest: [u32; 20],
}

struct Fb {
    file: File,
    var: VarInfo,
    line: usize,
    /// The visible screen mapped into memory (null when mmap failed: then a
    /// back buffer is written with seek+write).
    map: *mut u8,
    map_len: usize,
    buf: Vec<u8>,
}

impl Screen for Fb {
    fn size(&self) -> (u32, u32) {
        (self.var.xres, self.var.yres)
    }

    fn present(&mut self, rgba: &[u8], w: u32, rects: &[[i32; 4]]) {
        self.blit(rgba, w, rects)
    }
}

impl Fb {
    fn open(path: &str) -> Fb {
        let file = OpenOptions::new().read(true).write(true).open(path).expect("open fb");
        let mut var = VarInfo::default();
        unsafe { libc::ioctl(file.as_raw_fd(), 0x4600 as _, &mut var) };
        assert!(var.bpp == 32, "need a 32 bpp framebuffer, got {}", var.bpp);
        let line = var.xres_v as usize * 4;
        let map_len = line * (var.yoff + var.yres) as usize;
        let map = unsafe {
            libc::mmap(std::ptr::null_mut(), map_len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, file.as_raw_fd(), 0)
        };
        let map = if map == libc::MAP_FAILED { std::ptr::null_mut() } else { map as *mut u8 };
        let buf = if map.is_null() { vec![0; line * var.yres as usize] } else { Vec::new() };
        Fb { file, line, map, map_len, buf, var }
    }

    /// `present` in the framebuffer's own pixel layout.
    fn blit(&mut self, rgba: &[u8], w: u32, rects: &[[i32; 4]]) {
        let v = &self.var;
        let alpha = if v.t[1] > 0 { 0xffu32 << v.t[0] } else { 0 };
        let (sx, sy) = (w.min(v.xres) as i32, (v.yres as usize).min(rgba.len() / (w as usize * 4)) as i32);
        let src_line = w as usize * 4;
        let screen: &mut [u8] = if self.map.is_null() {
            &mut self.buf
        } else {
            let off = v.yoff as usize * self.line;
            unsafe { std::slice::from_raw_parts_mut(self.map.add(off), self.map_len - off) }
        };
        let (mut first, mut end) = (i32::MAX, 0);
        for r in rects {
            let (x0, y0, x1, y1) = (r[0].max(0), r[1].max(0), r[2].min(sx), r[3].min(sy));
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            first = first.min(y0);
            end = end.max(y1);
            for y in y0 as usize..y1 as usize {
                let src = &rgba[y * src_line + x0 as usize * 4..y * src_line + x1 as usize * 4];
                let dst = &mut screen[y * self.line + x0 as usize * 4..y * self.line + x1 as usize * 4];
                for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    let p = (s[0] as u32) << v.r[0] | (s[1] as u32) << v.g[0] | (s[2] as u32) << v.b[0] | alpha;
                    d.copy_from_slice(&p.to_le_bytes());
                }
            }
        }
        if self.map.is_null() && first < end {
            let off = ((v.yoff as usize + first as usize) * self.line) as u64;
            let bytes = &self.buf[first as usize * self.line..end as usize * self.line];
            let _ = self.file.seek(SeekFrom::Start(off)).and_then(|_| self.file.write_all(bytes));
        }
    }
}

// ---- touch ---------------------------------------------------------------------------------

#[repr(C)]
#[derive(Default)]
struct AbsInfo {
    value: i32,
    min: i32,
    max: i32,
    fuzz: i32,
    flat: i32,
    res: i32,
}

fn abs_info(fd: i32, code: u32) -> AbsInfo {
    let mut a = AbsInfo::default();
    let req = (2u32 << 30) | (24 << 16) | ((b'E' as u32) << 8) | (0x40 + code);
    unsafe { libc::ioctl(fd, req as _, &mut a) };
    a
}

/// tslib's calibration (`a b c d e f s [xres yres]`: x = (c + a*rx + b*ry) / s,
/// y = (f + d*rx + e*ry) / s) as MB_TOUCH_CAL's six factors, scaled when the
/// file was made for another resolution.
fn pointercal(path: &str, (w, h): (u32, u32)) -> Option<Vec<f32>> {
    let v: Vec<f64> = fs::read_to_string(path).ok()?.split_whitespace().filter_map(|n| n.parse().ok()).collect();
    if v.len() < 7 || v[6] == 0.0 {
        return None;
    }
    let s = v[6];
    let (kx, ky) = match v.get(7..9) {
        Some([xr, yr]) if *xr > 0.0 && *yr > 0.0 => (w as f64 / xr, h as f64 / yr),
        _ => (1.0, 1.0),
    };
    Some(
        [v[0] / s * kx, v[1] / s * kx, v[2] / s * kx, v[3] / s * ky, v[4] / s * ky, v[5] / s * ky]
            .iter()
            .map(|x| *x as f32)
            .collect(),
    )
}

/// Sends taps in screen coordinates: one per touch, on release, at most every 120 ms.
fn touch_thread(path: String, (w, h): (u32, u32), tx: mpsc::Sender<(f32, f32)>, probe: bool) {
    let mut f = match File::open(&path) {
        Ok(f) => f,
        Err(e) => return eprintln!("touch: {path}: {e}"),
    };
    let fd = f.as_raw_fd();
    let (ax, ay) = (abs_info(fd, 0), abs_info(fd, 1));
    let flags = std::env::var("MB_TOUCH").unwrap_or_default();
    let mut cal: Vec<f32> = std::env::var("MB_TOUCH_CAL").unwrap_or_default().split(',').filter_map(|v| v.trim().parse().ok()).collect();
    if cal.is_empty() {
        cal = pointercal("/etc/pointercal", (w, h)).unwrap_or_default();
        if !cal.is_empty() {
            eprintln!("touch: calibration from /etc/pointercal");
        }
    }
    eprintln!("touch: x {}..{} y {}..{} flags '{flags}'", ax.min, ax.max, ay.min, ay.max);
    // The kernel drops ABS events that repeat the last value, so start from it.
    let (mut x, mut y, mut down) = (ax.value, ay.value, false);
    let mut last = Instant::now() - Duration::from_secs(1);
    let mut ev = [0u8; 16];
    while f.read_exact(&mut ev).is_ok() {
        let ty = u16::from_le_bytes([ev[8], ev[9]]);
        let code = u16::from_le_bytes([ev[10], ev[11]]);
        let val = i32::from_le_bytes([ev[12], ev[13], ev[14], ev[15]]);
        if probe {
            eprintln!("ev type {ty} code {code} value {val}");
        }
        let up = match (ty, code) {
            (3, 0) | (3, 0x35) => { x = val; down = true; false }
            (3, 1) | (3, 0x36) => { y = val; down = true; false }
            (1, 330) => { if val == 1 { down = true; false } else { down } }
            (3, 24) => val == 0 && down, // pressure gone
            _ => false,
        };
        if !up || last.elapsed() < Duration::from_millis(120) {
            continue;
        }
        down = false;
        last = Instant::now();
        // MB_TOUCH_CAL="a,b,c,d,e,f": screen x = a*rx + b*ry + c, y = d*rx + e*ry + f.
        if let [a, b, c, d, e, f] = cal[..] {
            let (rx, ry) = (x as f32, y as f32);
            let p = ((a * rx + b * ry + c).clamp(0.0, w as f32 - 1.0), (d * rx + e * ry + f).clamp(0.0, h as f32 - 1.0));
            if probe {
                eprintln!("tap raw {x},{y} -> {:.0},{:.0}", p.0, p.1);
            }
            if tx.send(p).is_err() {
                return;
            }
            continue;
        }
        let span = |a: &AbsInfo| (a.max - a.min).max(1) as f32;
        let mut fx = (x - ax.min) as f32 / span(&ax);
        let mut fy = (y - ay.min) as f32 / span(&ay);
        if flags.contains("swap") { std::mem::swap(&mut fx, &mut fy) }
        if flags.contains("invx") { fx = 1.0 - fx }
        if flags.contains("invy") { fy = 1.0 - fy }
        let p = (fx.clamp(0.0, 1.0) * w as f32, fy.clamp(0.0, 1.0) * h as f32);
        if probe {
            eprintln!("tap raw {x},{y} -> {:.0},{:.0}", p.0, p.1);
        }
        if tx.send(p).is_err() {
            return;
        }
    }
}

// ---- main ----------------------------------------------------------------------------------

fn arg(args: &[String], name: &str, default: &str) -> String {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned().unwrap_or(default.into())
}

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
