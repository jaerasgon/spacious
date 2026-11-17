//! Spacious: see what's taking up your disk.
//!
//! `spacious [folder]` opens the window (and scans the folder if given).
//! `spacious --bench <folder>` scans without a window and prints the totals.

mod app;
mod icon;
mod live;
mod patch;
mod scan;
mod stream;
mod theme;
mod treemap;
mod util;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--iconset") {
        let dir = args.get(1).map(String::as_str).unwrap_or("Spacious.iconset");
        if let Err(e) = icon::write_iconset(std::path::Path::new(dir)) {
            eprintln!("Can't write {dir}: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("--bench-stream") {
        bench_stream(args.get(1).map(String::as_str).unwrap_or("/"));
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("--bench") {
        bench(args.get(1).map(String::as_str).unwrap_or("."));
        return Ok(());
    }
    // Finder passes -psn_… when launched from an .app on old systems; ignore flags.
    let initial = args.into_iter().find(|a| !a.starts_with('-'));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Spacious")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([800.0, 560.0])
            .with_icon(std::sync::Arc::new(egui::IconData { rgba: icon::rgba(256), width: 256, height: 256 })),
        ..Default::default()
    };
    eframe::run_native("Spacious", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, initial)))))
}

fn bench(path: &str) {
    use std::sync::atomic::Ordering;
    let path = util::expand(path);
    let p = scan::Progress::default();
    let mem0 = rss();
    let Some(t) = scan::scan_blocking(&path, &p) else {
        eprintln!("Can't scan {path}");
        std::process::exit(1);
    };
    let r = t.node(0);
    println!("path        {path}");
    println!("size        {} ({} bytes)", util::fmt_size(r.size), r.size);
    println!("files       {}", util::fmt_count(r.files));
    println!("folders     {}", util::fmt_count(r.dirs as u64));
    println!("nodes       {}", util::fmt_count(t.nodes.len() as u64));
    println!("unreadable  {}", p.unreadable.load(Ordering::Relaxed));
    println!("time        {}", util::fmt_duration(t.elapsed));
    println!("memory      {}", util::fmt_size(rss().saturating_sub(mem0)));
    let t0 = std::time::Instant::now();
    let mut cells = Vec::new();
    treemap::layout(&t, 0, egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1400.0, 800.0)), &Default::default(), &mut cells);
    println!("layout      {} cells in {:.1} ms", cells.len(), t0.elapsed().as_secs_f64() * 1000.0);
    for c in t.children(0).take(8) {
        let n = t.node(c);
        println!("  {:>10}  {}{}", util::fmt_size(n.size), n.name, if n.is_dir { "/" } else { "" });
    }
}

/// Streaming scan as the window runs it: time spent applying folders on the UI thread.
fn bench_stream(path: &str) {
    let path = util::expand(path);
    let job = stream::start(path.clone(), || {});
    let mut tree = stream::new_tree(&path);
    let mut b = stream::Builder::new();
    let mut apply = std::time::Duration::ZERO;
    let mut first_seen = None;
    loop {
        let done = job.done();
        let t0 = std::time::Instant::now();
        let mut n = 0;
        while let Some(m) = job.try_recv() {
            b.apply(&mut tree, m);
            n += 1;
        }
        apply += t0.elapsed();
        if n > 0 && first_seen.is_none() {
            first_seen = Some(job.started.elapsed());
        }
        if done && n == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    let scan = job.started.elapsed();
    let t0 = std::time::Instant::now();
    stream::finish(&mut tree, scan);
    let r = tree.node(0);
    println!("path        {path}");
    println!("size        {}", util::fmt_size(r.size));
    println!("files       {}", util::fmt_count(r.files));
    println!("folders     {}", util::fmt_count(r.dirs as u64));
    println!("unreadable  {}", tree.unreadable);
    println!("first map   {:.0} ms", first_seen.unwrap_or_default().as_secs_f64() * 1000.0);
    println!("scan        {}", util::fmt_duration(scan));
    println!("ui apply    {:.0} ms in total", apply.as_secs_f64() * 1000.0);
    println!("finish      {:.0} ms (sort and compact)", t0.elapsed().as_secs_f64() * 1000.0);
    for c in tree.children(0).take(6) {
        let n = tree.node(c);
        println!("  {:>10}  {}", util::fmt_size(n.size), n.name);
    }
}

/// Resident memory of this process.
fn rss() -> u64 {
    // SAFETY: getrusage writes into the struct we own.
    unsafe {
        let mut u: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut u);
        // ru_maxrss is in bytes on macOS.
        u.ru_maxrss as u64
    }
}
