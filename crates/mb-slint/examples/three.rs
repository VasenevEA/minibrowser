//! Three small browsers in a scrolling Slint list:
//!   three a.html b.html c.html [blitz] [--panel] [--snapshot out.png]
//! `--panel` runs full screen on mb-fb's screen and touch (feature `panel`);
//! `--snapshot out.png` renders headless after two seconds to a PNG,
//! `--drag X,Y,DY` drags (scrolls) halfway through.
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use slint::{Image, Model, ModelRc, SharedString, VecModel};

slint::slint! {
    export { ThreeBrowsers } from "../ui/three-browsers.slint";
}

const PAGE: (u32, u32) = (800, 480);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pages: Vec<PathBuf> = args.iter().filter(|a| a.ends_with(".html")).map(PathBuf::from).collect();
    assert_eq!(pages.len(), 3, "three pages");
    let blitz = args.iter().any(|a| a == "blitz");
    let snapshot = args.iter().position(|a| a == "--snapshot").map(|i| args[i + 1].clone());

    #[cfg(feature = "panel")]
    if args.iter().any(|a| a == "--panel") {
        mb_slint::panel::init(&args).expect("panel");
    }
    let window = snapshot.as_ref().map(|_| headless::init(800, 480));

    let app = ThreeBrowsers::new().unwrap();
    let frames = Rc::new(VecModel::from(vec![Image::default(); 3]));
    let counts = Rc::new(VecModel::from(vec![0; 3]));
    app.set_frames(ModelRc::from(frames.clone()));
    app.set_counts(ModelRc::from(counts.clone()));
    let name = |p: &PathBuf| SharedString::from(p.file_name().unwrap().to_string_lossy().as_ref());
    app.set_titles(ModelRc::from(Rc::new(VecModel::from(pages.iter().map(name).collect::<Vec<_>>()))));
    let engine = if blitz { "Blitz" } else { "mb-core" };
    app.set_notes(ModelRc::from(Rc::new(VecModel::from(
        (0..3).map(|_| SharedString::from(format!("{engine} + QuickJS, 800×480, shown at 2/3"))).collect::<Vec<_>>(),
    ))));

    let open = {
        let (frames, counts) = (frames.clone(), counts.clone());
        move |i: usize, path: &PathBuf| {
            let (frames, counts) = (frames.clone(), counts.clone());
            mb_slint::HtmlView::open(path, PAGE, blitz, move |img| {
                frames.set_row_data(i, img);
                counts.set_row_data(i, counts.row_data(i).unwrap_or(0) + 1);
            })
            .expect("open page")
        }
    };
    let views: Rc<RefCell<Vec<mb_slint::HtmlView>>> =
        Rc::new(RefCell::new(pages.iter().enumerate().map(|(i, p)| open(i, p)).collect()));
    let v = views.clone();
    app.on_tapped(move |i, x, y| v.borrow()[i as usize].tap(x, y));
    let v = views.clone();
    app.on_reload(move |i| v.borrow_mut()[i as usize] = open(i as usize, &pages[i as usize]));

    match (window, snapshot) {
        (Some(w), Some(out)) => {
            app.show().unwrap();
            let drag = args.iter().position(|a| a == "--drag").map(|i| {
                let v: Vec<f32> = args[i + 1].split(',').map(|n| n.parse().unwrap()).collect();
                (v[0], v[1], v[2])
            });
            headless::run(&w, 2.0, drag);
            headless::save(&w, 800, 480, &out);
        }
        _ => app.run().unwrap(),
    }
}

mod headless {
    //! Slint's software renderer without a display, for snapshots.
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
    use slint::platform::{PointerEventButton, WindowAdapter, WindowEvent};
    use slint::{LogicalPosition, PhysicalSize};

    struct Platform(Rc<MinimalSoftwareWindow>);

    impl slint::platform::Platform for Platform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
    }

    pub fn init(w: u32, h: u32) -> Rc<MinimalSoftwareWindow> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(Platform(window.clone()))).unwrap();
        window.set_size(PhysicalSize::new(w, h));
        window
    }

    /// Runs timers for `secs`; halfway, drags from (x, y) by `dy`.
    pub fn run(window: &MinimalSoftwareWindow, secs: f64, drag: Option<(f32, f32, f32)>) {
        let start = Instant::now();
        let mut dragged = false;
        while start.elapsed().as_secs_f64() < secs {
            slint::platform::update_timers_and_animations();
            if let (Some((x, y, dy)), false) = (drag, dragged) {
                if start.elapsed().as_secs_f64() > secs / 2.0 {
                    let button = PointerEventButton::Left;
                    window.dispatch_event(WindowEvent::PointerPressed { position: LogicalPosition::new(x, y), button });
                    for k in 1..=10 {
                        let position = LogicalPosition::new(x, y + dy * k as f32 / 10.0);
                        window.dispatch_event(WindowEvent::PointerMoved { position });
                        std::thread::sleep(Duration::from_millis(16));
                        slint::platform::update_timers_and_animations();
                    }
                    window.dispatch_event(WindowEvent::PointerReleased { position: LogicalPosition::new(x, y + dy), button });
                    dragged = true;
                }
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        slint::platform::update_timers_and_animations();
    }

    pub fn save(window: &MinimalSoftwareWindow, w: u32, h: u32, out: &str) {
        let mut buf = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(&mut buf, w as usize);
        });
        let rgba: Vec<u8> = buf.iter().flat_map(|p| [p.red, p.green, p.blue, 255]).collect();
        let mut e = png::Encoder::new(std::fs::File::create(out).unwrap(), w, h);
        e.set_color(png::ColorType::Rgba);
        e.write_header().unwrap().write_image_data(&rgba).unwrap();
    }
}
