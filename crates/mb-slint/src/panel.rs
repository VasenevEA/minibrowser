//! Slint on a panel without a window system: a Slint platform whose window
//! is drawn by Slint's software renderer onto mb-fb's screen (DRM/KMS or
//! fbdev), with taps from mb-fb's touch reader (tslib calibration).
//!
//!   let size = mb_slint::panel::init(&args)?;   // before creating components
//!   let app = App::new()?; ... app.run()?;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use mb_fb::Screen;
use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{LogicalPosition, PhysicalSize, PlatformError};

struct Panel {
    window: Rc<MinimalSoftwareWindow>,
    screen: RefCell<Box<dyn Screen>>,
    taps: mpsc::Receiver<(f32, f32)>,
    size: (u32, u32),
    started: Instant,
    trace: bool,
}

/// Opens the screen and the touch panel (mb-fb's `--display`, `--card`,
/// `--fb`, `--input` arguments) and makes them Slint's platform. Returns the
/// screen size; the window gets it too.
pub fn init(args: &[String]) -> Result<(u32, u32), PlatformError> {
    default_font();
    let screen = mb_fb::open_screen(args);
    let size = screen.size();
    let (tx, taps) = mpsc::channel();
    let input = mb_fb::arg(args, "--input", "/dev/input/event0");
    std::thread::spawn(move || mb_fb::touch_thread(input, size, tx, false));
    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    window.set_size(PhysicalSize::new(size.0, size.1));
    let trace = std::env::var_os("MB_TRACE").is_some();
    slint::platform::set_platform(Box::new(Panel { window, screen: RefCell::new(screen), taps, size, started: Instant::now(), trace }))
        .map_err(|e| PlatformError::Other(e.to_string()))?;
    Ok(size)
}

/// Panels have no fontconfig, and Slint then finds no font at all: unless
/// SLINT_DEFAULT_FONT names one, give it the engine's own DejaVu Sans.
fn default_font() {
    static SANS: &[u8] = include_bytes!("../../mb-core/assets/DejaVuSans-hmi.ttf");
    if std::env::var_os("SLINT_DEFAULT_FONT").is_some() {
        return;
    }
    let path = std::env::temp_dir().join("mb-slint-sans.ttf");
    if std::fs::metadata(&path).map(|m| m.len() as usize) .ok() != Some(SANS.len()) && std::fs::write(&path, SANS).is_err() {
        return;
    }
    // Before any thread starts: set_var is only sound then.
    unsafe { std::env::set_var("SLINT_DEFAULT_FONT", &path) };
}

impl Panel {
    fn tap(&self, (x, y): (f32, f32)) {
        let position = LogicalPosition::new(x, y);
        self.window.dispatch_event(WindowEvent::PointerMoved { position });
        self.window.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
        self.window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
        self.window.dispatch_event(WindowEvent::PointerExited);
    }
}

impl Platform for Panel {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.started.elapsed()
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        let (w, h) = self.size;
        let mut buf = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        loop {
            slint::platform::update_timers_and_animations();
            let t = Instant::now();
            self.window.draw_if_needed(|r| {
                let region = r.render(&mut buf, w as usize);
                let (o, s) = (region.bounding_box_origin(), region.bounding_box_size());
                let rect = [o.x, o.y, o.x + s.width as i32, o.y + s.height as i32];
                if rect[2] <= rect[0] || rect[3] <= rect[1] {
                    return;
                }
                for y in rect[1]..rect[3] {
                    let row = (y as u32 * w) as usize;
                    for x in rect[0]..rect[2] {
                        let p = buf[row + x as usize];
                        let i = (row + x as usize) * 4;
                        rgba[i..i + 4].copy_from_slice(&[p.red, p.green, p.blue, 255]);
                    }
                }
                self.screen.borrow_mut().present(&rgba, w, &[rect]);
                if self.trace {
                    eprintln!("slint frame {:?}: {:.1} ms", rect, t.elapsed().as_secs_f64() * 1e3);
                }
            });
            if self.window.has_active_animations() {
                std::thread::sleep(Duration::from_millis(16));
            } else {
                let wait = slint::platform::duration_until_next_timer_update().unwrap_or(Duration::from_secs(1));
                if let Ok(p) = self.taps.recv_timeout(wait.min(Duration::from_secs(1))) {
                    self.tap(p);
                }
            }
            while let Ok(p) = self.taps.try_recv() {
                self.tap(p);
            }
        }
    }
}
