//! A page on Blitz to a PNG, with frame times:
//!   cargo run -p mb-blitz --example shot -- page.html out.png [--wait MS] [--click X,Y]
//! MB_CHECK=1 compares the incrementally painted pixels with a full repaint.
use mb_blitz::BlitzPage;
use mb_qjs::QuickJs;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut wait, mut clicks) = (0f64, Vec::new());
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--wait" => { i += 1; wait = args[i].parse().unwrap() }
            "--click" => {
                i += 1;
                let (x, y) = args[i].split_once(',').unwrap();
                clicks.push((x.parse::<f32>().unwrap(), y.parse::<f32>().unwrap()));
            }
            a => panic!("unknown argument {a}"),
        }
        i += 1;
    }
    let (w, h) = (800, 480);
    let html = std::fs::read_to_string(&args[0]).unwrap();
    let dir = std::path::Path::new(&args[0]).parent().map(|p| p.to_path_buf()).unwrap_or_else(|| ".".into());
    let mut page = BlitzPage::new(Box::new(QuickJs::with_files(&dir)), w, h).with_base(&dir);
    let t = Instant::now();
    page.load(&html);
    eprintln!("load {:.1} ms, {} nodes", t.elapsed().as_secs_f64() * 1e3, page.node_count());
    page.take_damage();
    eprintln!("scene: {}", page.scene_summary());
    for (x, y) in clicks {
        let t = Instant::now();
        let r = page.click(x, y);
        eprintln!("click {x},{y}: {r} in {:.1} ms {:?}", t.elapsed().as_secs_f64() * 1e3, page.take_damage());
    }
    let (mut now, mut frames, mut total, mut worst, mut area) = (0.0, 0, 0.0, 0f64, 0i64);
    while wait > 0.0 && now < wait {
        let step = page.next_timer(now).unwrap_or(wait).max(1.0).min(wait - now);
        now += step;
        let t = Instant::now();
        if page.tick(now) {
            let ms = t.elapsed().as_secs_f64() * 1e3;
            frames += 1;
            total += ms;
            worst = worst.max(ms);
            area += page.take_damage().iter().map(|r| ((r[2] - r[0]) * (r[3] - r[1])) as i64).sum::<i64>();
        }
    }
    if frames > 0 {
        eprintln!("{frames} frames: avg {:.2} ms, worst {:.2} ms, avg damage {} px ({:.0}% of screen)",
            total / frames as f64, worst, area / frames, 100.0 * area as f64 / frames as f64 / (w * h) as f64);
    }
    let px = page.viewport();
    if std::env::var_os("MB_CHECK").is_some() {
        // A full repaint of the same DOM: force it by clearing the last scene.
        let full = page.repaint_full();
        let bad: Vec<usize> = (0..px.len() / 4).filter(|&i| (0..3).any(|c| px[i * 4 + c].abs_diff(full[i * 4 + c]) > 2)).collect();
        let worst = bad.iter().map(|&i| (0..3).map(|c| px[i * 4 + c].abs_diff(full[i * 4 + c])).max().unwrap()).max().unwrap_or(0);
        eprintln!("check: {} pixels differ from a full repaint (by up to {worst})", bad.len());
        if let (Some(a), Some(b)) = (bad.iter().min_by_key(|&&i| i % w as usize), bad.iter().max_by_key(|&&i| i % w as usize)) {
            eprintln!("  x {}..{}, y {}..{}", a % w as usize, b % w as usize, bad[0] / w as usize, bad[bad.len() - 1] / w as usize);
        }
    }
    let mut e = png::Encoder::new(std::fs::File::create(&args[1]).unwrap(), w, h);
    e.set_color(png::ColorType::Rgba);
    e.write_header().unwrap().write_image_data(&px).unwrap();
}
