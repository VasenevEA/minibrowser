//! A Slint window with an HTML page inside:
//!   cargo run -p mb-slint --example dashboard [--features blitz] -- page.html [blitz]
//! The header and the buttons are Slint; the page below is HTML/CSS/JS.
use std::path::PathBuf;

slint::slint! {
    export { App } from "../ui/demo-app.slint";
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = PathBuf::from(args.first().map(String::as_str).unwrap_or("www/dash/snapshot.html"));
    let blitz = args.iter().any(|a| a == "blitz");
    let app = App::new().unwrap();

    let open = {
        let weak = app.as_weak();
        let path = path.clone();
        move || {
            let w = weak.clone();
            mb_slint::HtmlView::open(&path, (800, 480), blitz, move |img| {
                if let Some(app) = w.upgrade() {
                    app.set_page_frame(img);
                }
            })
            .expect("open page")
        }
    };
    let view = std::rc::Rc::new(std::cell::RefCell::new(open()));
    app.set_status(format!("{} · {}", path.display(), if blitz { "blitz" } else { "mb-core" }).into());

    let v = view.clone();
    app.on_page_tapped(move |x, y| v.borrow().tap(x, y));
    let v = view.clone();
    app.on_reload(move || *v.borrow_mut() = open());
    app.run().unwrap();
}
