//! mb-fb — shows an HTML page full-screen on a Linux framebuffer, with touch.
//!
//!   mb-fb page.html [--fb /dev/fb0] [--input /dev/input/event0] [--probe-touch]
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
    buf: Vec<u8>,
    /// The RGBA last presented, to skip rows that did not change.
    last: Vec<u8>,
}

impl Fb {
    fn open(path: &str) -> Fb {
        let file = OpenOptions::new().read(true).write(true).open(path).expect("open fb");
        let mut var = VarInfo::default();
        unsafe { libc::ioctl(file.as_raw_fd(), 0x4600 as _, &mut var) };
        assert!(var.bpp == 32, "need a 32 bpp framebuffer, got {}", var.bpp);
        let line = var.xres_v as usize * 4;
        Fb { file, line, buf: vec![0; line * var.yres as usize], var, last: Vec::new() }
    }

    fn size(&self) -> (u32, u32) {
        (self.var.xres, self.var.yres)
    }

    /// Copies `w * h` RGBA onto the visible screen in the framebuffer's own pixel
    /// layout, converting and writing only the rows that changed since last time.
    fn present(&mut self, rgba: &[u8], w: u32) {
        let v = &self.var;
        let alpha = if v.t[1] > 0 { 0xffu32 << v.t[0] } else { 0 };
        let (cols, rows) = (w.min(v.xres) as usize, (v.yres as usize).min(rgba.len() / (w as usize * 4)));
        let src_line = w as usize * 4;
        let full = self.last.len() != rgba.len();
        let (mut first, mut end) = (usize::MAX, 0);
        let mut dirty = vec![false; rows];
        for y in 0..rows {
            let src = &rgba[y * src_line..y * src_line + cols * 4];
            if !full && src == &self.last[y * src_line..y * src_line + cols * 4] {
                continue;
            }
            dirty[y] = true;
            first = first.min(y);
            end = y + 1;
            let dst = &mut self.buf[y * self.line..y * self.line + cols * 4];
            for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                let p = (s[0] as u32) << v.r[0] | (s[1] as u32) << v.g[0] | (s[2] as u32) << v.b[0] | alpha;
                d.copy_from_slice(&p.to_le_bytes());
            }
        }
        self.last.clear();
        self.last.extend_from_slice(rgba);
        // Write each run of changed rows; a clock at the top and a counter at the
        // bottom should not rewrite everything in between.
        let yoff = v.yoff as usize;
        let mut y = first;
        while y < end {
            if !dirty[y] {
                y += 1;
                continue;
            }
            let start = y;
            while y < end && dirty[y] {
                y += 1;
            }
            let off = ((yoff + start) * self.line) as u64;
            let bytes = &self.buf[start * self.line..y * self.line];
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

/// Sends taps in screen coordinates: one per touch, on release, at most every 300 ms.
fn touch_thread(path: String, (w, h): (u32, u32), tx: mpsc::Sender<(f32, f32)>, probe: bool) {
    let mut f = match File::open(&path) {
        Ok(f) => f,
        Err(e) => return eprintln!("touch: {path}: {e}"),
    };
    let fd = f.as_raw_fd();
    let (ax, ay) = (abs_info(fd, 0), abs_info(fd, 1));
    let flags = std::env::var("MB_TOUCH").unwrap_or_default();
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
        if !up || last.elapsed() < Duration::from_millis(300) {
            continue;
        }
        down = false;
        last = Instant::now();
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

fn mtime(path: &str) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1).filter(|a| !a.starts_with("--")).cloned() else {
        eprintln!("usage: mb-fb page.html [--fb /dev/fb0] [--input /dev/input/event0] [--probe-touch]");
        std::process::exit(2);
    };
    let probe = args.iter().any(|a| a == "--probe-touch");
    // MB_TRACE=1 logs every frame's timings.
    let trace = std::env::var_os("MB_TRACE").is_some();

    let mut fb = Fb::open(&arg(&args, "--fb", "/dev/fb0"));
    let (w, h) = fb.size();
    let _ = fs::write("/sys/class/graphics/fb0/blank", "0");
    let _ = fs::write("/sys/class/graphics/fbcon/cursor_blink", "0");
    let _ = fs::write("/dev/tty1", "\x1b[?25l\x1b[2J");

    let (tx, rx) = mpsc::channel();
    let input = arg(&args, "--input", "/dev/input/event0");
    std::thread::spawn(move || touch_thread(input, (w, h), tx, probe));

    let clock = Instant::now();
    let now_ms = move || clock.elapsed().as_secs_f64() * 1000.0;
    let mut page = Page::new(Box::new(QuickJs::new()), w, h);
    page.set_layout_engine(mb_core::LayoutEngine::from_env());
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
                    let s = page.stats();
                    eprintln!("load {path}: {:.0} ms, {} nodes, {} boxes", ms(t), s.node_count, s.box_count);
                    let t = Instant::now();
                    fb.present(&page.viewport(), w);
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
            match page.viewport_slice() {
                Some(px) => fb.present(px, w),
                None => fb.present(&page.viewport(), w),
            }
            if trace {
                eprintln!("frame: timers+layout+paint {t_tick:.0} ms, present {:.0} ms", ms(t));
            }
        }
    }
}
