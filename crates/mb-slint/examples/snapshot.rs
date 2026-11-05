//! The demo window rendered without a display (Slint's software renderer on
//! a minimal platform, as on a panel), with a tap, to a PNG:
//!   cargo run -p mb-slint --example snapshot [--features blitz] -- page.html out.png [blitz] [--tap X,Y]
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{PointerEventButton, WindowAdapter, WindowEvent};
use slint::{LogicalPosition, PhysicalSize};

slint::slint! {
    export { App } from "../ui/demo-app.slint";
}

struct Platform(Rc<MinimalSoftwareWindow>);

impl slint::platform::Platform for Platform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = PathBuf::from(&args[0]);
    let out = &args[1];
    let blitz = args.iter().any(|a| a == "blitz");
    let tap = args.iter().position(|a| a == "--tap").map(|i| {
        let (x, y) = args[i + 1].split_once(',').unwrap();
        (x.parse::<f32>().unwrap(), y.parse::<f32>().unwrap())
    });
    let (w, h) = (800u32, 540u32);

    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Platform(window.clone()))).unwrap();
    window.set_size(PhysicalSize::new(w, h));

    let app = App::new().unwrap();
    let weak = app.as_weak();
    let view = mb_slint::HtmlView::open(&path, (800, 480), blitz, move |img| {
        if let Some(app) = weak.upgrade() {
            app.set_page_frame(img);
        }
    })
    .unwrap();
    let v = view.clone();
    app.on_page_tapped(move |x, y| v.tap(x, y));
    app.set_status(format!("{} · {}", path.display(), if blitz { "blitz" } else { "mb-core" }).into());
    app.show().unwrap();

    // Run the event loop by hand for two seconds; tap after one.
    let start = Instant::now();
    let mut tapped = false;
    while start.elapsed() < Duration::from_secs(2) {
        slint::platform::update_timers_and_animations();
        if let (Some((x, y)), false) = (tap, tapped) {
            if start.elapsed() > Duration::from_secs(1) {
                let position = LogicalPosition::new(x, y);
                window.dispatch_event(WindowEvent::PointerMoved { position });
                window.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
                window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
                tapped = true;
            }
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    slint::platform::update_timers_and_animations();
    let mut buf = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut buf, w as usize);
    });
    let rgba: Vec<u8> = buf.iter().flat_map(|p| [p.red, p.green, p.blue, 255]).collect();
    let mut e = png::Encoder::new(std::fs::File::create(out).unwrap(), w, h);
    e.set_color(png::ColorType::Rgba);
    e.write_header().unwrap().write_image_data(&rgba).unwrap();
    drop(view);
}
