//! mb-headless — renders a page to a PNG, the way a panel would show it.
//!
//!   mb-headless page.html out.png [--size 800x480] [--full] [--click X,Y]... [--wait MS]
//!
//! Clicks run in order after the page loads; `--wait` advances the page clock
//! (timers fire) before the screenshot. `--full` saves the whole page, not the viewport.

use std::time::Instant;

use mb_core::Page;
use mb_qjs::QuickJs;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: mb-headless page.html out.png [--size WxH] [--full] [--click X,Y]... [--wait MS]");
        std::process::exit(2);
    }
    let (mut w, mut h, mut full, mut wait) = (800u32, 480u32, false, 0f64);
    let mut clicks = Vec::new();
    let mut i = 2;
    while i < args.len() {
        let next = || args.get(i + 1).map(String::as_str).unwrap_or("");
        match args[i].as_str() {
            "--size" => {
                let (a, b) = next().split_once('x').expect("--size WxH");
                (w, h) = (a.parse().expect("width"), b.parse().expect("height"));
                i += 1;
            }
            "--click" => {
                let (a, b) = next().split_once(',').expect("--click X,Y");
                clicks.push((a.parse::<f32>().expect("x"), b.parse::<f32>().expect("y")));
                i += 1;
            }
            "--wait" => {
                wait = next().parse().expect("--wait MS");
                i += 1;
            }
            "--full" => full = true,
            other => panic!("unknown argument {other}"),
        }
        i += 1;
    }

    let html = std::fs::read_to_string(&args[0]).expect("read page");
    let mut page = Page::new(Box::new(QuickJs::new()), w, h);
    let t = Instant::now();
    page.load(&html);
    let s = page.stats();
    eprintln!("load {:.1} ms: {} nodes, {} boxes, content {}x{}", ms(t), s.node_count, s.box_count, s.content_width, s.content_height);

    for (x, y) in clicks {
        let t = Instant::now();
        let changed = page.click(x, y);
        eprintln!("click {x},{y}: {} in {:.1} ms", if changed { "repainted" } else { "nothing" }, ms(t));
    }
    // Timers count from page start; step through them so intervals fire as often as they would live.
    let mut now = 0.0;
    while wait > 0.0 && now < wait {
        let step = page.next_timer(now).unwrap_or(wait).max(1.0).min(wait - now);
        now += step;
        page.tick(now);
    }

    let (rgba, ow, oh) = if full { page.full_page() } else { (page.viewport(), w, h) };
    image::save_buffer(&args[1], &rgba, ow, oh, image::ColorType::Rgba8).expect("write png");
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
