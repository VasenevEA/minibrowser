//! mb-headless — renders a page to a PNG, the way a panel would show it.
//!
//!   mb-headless page.html out.png [--size 800x480] [--full] [--dump] [--click X,Y]... [--wait MS]
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
    let (mut w, mut h, mut full, mut wait, mut dump) = (800u32, 480u32, false, 0f64, false);
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
            "--dump" => dump = true,
            other => panic!("unknown argument {other}"),
        }
        i += 1;
    }

    let html = std::fs::read_to_string(&args[0]).expect("read page");
    let dir = std::path::Path::new(&args[0]).parent().map(|p| p.to_path_buf()).unwrap_or_else(|| ".".into());
    let mut page = Page::new(Box::new(QuickJs::with_files(&dir)), w, h);
    page.set_layout_engine(mb_core::LayoutEngine::from_env());
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

    // MB_CHECK=1: compare the incrementally repainted pixels with a fresh full paint.
    if std::env::var_os("MB_CHECK").is_some() {
        if let (Some(root), Some(px)) = (page.layout_tree(), page.viewport_slice()) {
            let fresh = mb_core::paint::paint(root, w, h);
            let bad: Vec<usize> = (0..px.len() / 4).filter(|&i| (0..3).any(|c| px[i * 4 + c].abs_diff(fresh[i * 4 + c]) > 2)).collect();
            eprintln!("check: {} pixels differ from a full paint", bad.len());
            fn boxes(b: &mb_core::layout::LayoutBox, d: usize) {
                if b.border.top > 0.0 || b.background_color.is_some() {
                    eprintln!("{:d$}box y={} h={} bt={} r={} bg={:?}", "", b.y + b.margin.top, b.height, b.border.top, b.border_radius, b.background_color.map(|c| (c.r, c.g, c.b)), d = d);
                }
                for c in &b.children { boxes(c, d + 1) }
            }
            if std::env::var_os("MB_CHECK_BOXES").is_some() { boxes(root, 0) }
            for &i in bad.iter().take(8) {
                let (x, y) = (i as u32 % w, i as u32 / w);
                eprintln!("  {x},{y}: {:?} vs {:?}", &px[i * 4..i * 4 + 3], &fresh[i * 4..i * 4 + 3]);
            }
        }
    }
    if dump {
        print!("{}", page.dump_layout());
    }
    let (rgba, ow, oh) = if full { page.full_page() } else { (page.viewport(), w, h) };
    image::save_buffer(&args[1], &rgba, ow, oh, image::ColorType::Rgba8).expect("write png");
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
