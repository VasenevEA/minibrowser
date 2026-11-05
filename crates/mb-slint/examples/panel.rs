//! The demo app on a panel, full screen, no window system:
//!   panel page.html [blitz] [--display kms|fb] [--input /dev/input/event0]
//! Build: cargo zigbuild -p mb-slint --example panel --no-default-features
//!        --features panel,blitz,mb-qjs/bindgen --target armv7-unknown-linux-musleabihf
use std::path::PathBuf;

slint::slint! {
    export { App } from "../ui/demo-app.slint";
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = PathBuf::from(&args[0]);
    let blitz = args.iter().any(|a| a == "blitz");
    let (w, h) = mb_slint::panel::init(&args).expect("panel");
    let app = App::new().unwrap();
    let weak = app.as_weak();
    // The header is 60 px: the page gets the rest.
    let view = mb_slint::HtmlView::open(&path, (w, h - 60), blitz, move |img| {
        if let Some(app) = weak.upgrade() {
            app.set_page_frame(img);
        }
    })
    .expect("open page");
    let v = std::rc::Rc::new(std::cell::RefCell::new(view));
    let reload = {
        let (v, weak, path) = (v.clone(), app.as_weak(), path.clone());
        move || {
            let w2 = weak.clone();
            if let Ok(nv) = mb_slint::HtmlView::open(&path, (w, h - 60), blitz, move |img| {
                if let Some(app) = w2.upgrade() {
                    app.set_page_frame(img);
                }
            }) {
                *v.borrow_mut() = nv;
            }
        }
    };
    let v2 = v.clone();
    app.on_page_tapped(move |x, y| v2.borrow().tap(x, y));
    app.on_reload(reload);
    app.set_status(format!("{} · {}", path.file_name().unwrap().to_string_lossy(), if blitz { "blitz" } else { "mb-core" }).into());
    app.run().unwrap();
}
