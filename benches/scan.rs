//! Measurement harness (no behaviour changes). Usage:
//!   cargo bench --bench scan -- <tree-dir> [--label 100k]
//! Generate a tree first: cargo run --release --example gen_tree -- --preset 100k --out <dir>
//! Run each tree in its own process: peak RSS is a per-process high-water mark.
use eframe::egui::{Rect, pos2};
use lumadisk::{
    model::{ScanResult, SizeMode},
    scanner::{self, ScanEvent},
    treemap,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

#[cfg(windows)]
fn peak_rss_bytes() -> u64 {
    #[repr(C)]
    struct Counters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged: usize,
        quota_paged: usize,
        quota_peak_nonpaged: usize,
        quota_nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
    }
    let mut c = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        page_fault_count: 0,
        peak_working_set: 0,
        working_set: 0,
        quota_peak_paged: 0,
        quota_paged: 0,
        quota_peak_nonpaged: 0,
        quota_nonpaged: 0,
        pagefile: 0,
        peak_pagefile: 0,
    };
    // SAFETY: valid pseudo-handle and correctly sized, repr(C) PROCESS_MEMORY_COUNTERS.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    if ok != 0 {
        c.peak_working_set as u64
    } else {
        0
    }
}

#[cfg(not(windows))]
fn peak_rss_bytes() -> u64 {
    // Linux: VmHWM is the peak resident set in kB.
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
        })
        .map(|kb| kb * 1024)
        .unwrap_or(0)
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / 1048576.0
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i]
}

fn report(name: &str, mut samples: Vec<f64>) {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "  {name:<44} n={:<5} p50={:.3}ms p95={:.3}ms p99={:.3}ms max={:.3}ms",
        samples.len(),
        pct(&samples, 0.50),
        pct(&samples, 0.95),
        pct(&samples, 0.99),
        samples.last().copied().unwrap_or(0.0)
    );
}

/// Mirror of `LumaDiskApp::filtered_indices` (app.rs) with default filters, plus optional query.
fn filtered_indices(result: &ScanResult, query: &str) -> Vec<usize> {
    let query = query.trim().to_ascii_lowercase();
    let min_size = std::hint::black_box(0_u64);
    result
        .files
        .iter()
        .enumerate()
        .filter(|(_, file)| {
            file.counted_bytes(SizeMode::Physical).unwrap_or(0) >= min_size
                && (query.is_empty()
                    || file.name.to_ascii_lowercase().contains(&query)
                    || file
                        .path
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .contains(&query))
        })
        .map(|(index, _)| index)
        .collect()
}

/// Mirror of the CPU part of `paint_heatmap` (app.rs): group the first 1,500 indices by
/// extension, layout groups, then layout each group's files. Returns tile count.
fn heatmap_layout(result: &ScanResult, indices: &[usize], rect: Rect) -> usize {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for index in indices.iter().copied().take(1_500) {
        groups
            .entry(result.files[index].extension.clone())
            .or_default()
            .push(index);
    }
    let mut groups: Vec<(String, Vec<usize>, u64)> = groups
        .into_iter()
        .map(|(extension, files)| {
            let size = files
                .iter()
                .map(|i| {
                    result.files[*i]
                        .counted_bytes(SizeMode::Physical)
                        .unwrap_or(0)
                        .max(1)
                })
                .sum();
            (extension, files, size)
        })
        .collect();
    groups.sort_unstable_by_key(|g| std::cmp::Reverse(g.2));
    let weights: Vec<(usize, u64)> = groups.iter().enumerate().map(|(i, g)| (i, g.2)).collect();
    let mut tiles = 0;
    for (gi, group_rect) in treemap::layout(&weights, rect) {
        let content = treemap::inset(group_rect, 4.0);
        let fw: Vec<(usize, u64)> = groups[gi]
            .1
            .iter()
            .map(|i| {
                (
                    *i,
                    result.files[*i]
                        .counted_bytes(SizeMode::Physical)
                        .unwrap_or(0)
                        .max(1),
                )
            })
            .collect();
        tiles += treemap::layout(&fw, content).len();
    }
    tiles
}

fn main() {
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| a != "--bench")
        .collect();
    let Some(root) = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .map(PathBuf::from)
    else {
        eprintln!("usage: cargo bench --bench scan -- <tree-dir> [--label NAME]");
        std::process::exit(2);
    };
    let label = args
        .iter()
        .position(|a| a == "--label")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| root.display().to_string());
    println!("== LumaDisk scan bench: {label} ({}) ==", root.display());
    println!(
        "profile: {}; threads: {}",
        if cfg!(debug_assertions) {
            "DEBUG (numbers not meaningful)"
        } else {
            "release"
        },
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0)
    );

    // 1. Scan via the real public entry point, driven like the UI thread (poll at ~60 Hz).
    //    Duplicates OFF here so wall time and the frame probe isolate the metadata walk.
    let (sender, receiver) = mpsc::channel();
    let started = Instant::now();
    scanner::start_scan(root.clone(), true, false, sender);
    let mut drain_ms = Vec::new();
    let mut progress_events = 0_usize;
    let mut result = None;
    while result.is_none() {
        std::thread::sleep(Duration::from_micros(16_667));
        let t = Instant::now();
        // identical to App::poll_scan: collect try_iter() into a Vec, then match
        let events: Vec<ScanEvent> = receiver.try_iter().collect();
        for event in events {
            match event {
                ScanEvent::Progress { .. } => progress_events += 1,
                ScanEvent::Complete(r) => result = Some(r.expect("scan failed")),
            }
        }
        drain_ms.push(ms(t.elapsed()));
    }
    let wall = started.elapsed();
    let result = result.unwrap();
    let rss_scan = peak_rss_bytes();
    println!("scan (dups off):");
    println!(
        "  wall time to Complete: {:.3}s  files={} logical={:.1}MiB physical={:.1}MiB file_errors={} unreadable_dirs={} dirs={} hardlink_extra={} physical_unknown={} (scanner-internal elapsed {}ms)",
        wall.as_secs_f64(),
        result.files.len(),
        mib(result.total_logical.0),
        mib(result.total_physical.0),
        result.file_error_count,
        result.unreadable_dir_count(),
        result.dirs.len(),
        result.hardlink_extra_count,
        result.physical_unknown_count,
        result.elapsed_ms
    );
    println!(
        "  peak RSS (peak working set) after scan: {:.1} MiB; progress events={}",
        mib(rss_scan),
        progress_events
    );

    // 2. Dup finder on the tree.
    let t = Instant::now();
    let found = scanner::find_duplicate_groups(&result.files);
    println!("dup finder:");
    println!(
        "  find_duplicate_groups: {:.3}s, {} groups, {} read errors; peak RSS now {:.1} MiB",
        t.elapsed().as_secs_f64(),
        found.groups.len(),
        found.read_errors,
        mib(peak_rss_bytes())
    );

    // 3. Post-scan per-frame work.
    let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1270.0, 700.0));
    let t = Instant::now();
    let indices = filtered_indices(&result, "");
    let filter_ms = ms(t.elapsed());
    let weights: Vec<(usize, u64)> = indices
        .iter()
        .take(1_500)
        .map(|i| {
            (
                *i,
                result.files[*i]
                    .counted_bytes(SizeMode::Physical)
                    .unwrap_or(0)
                    .max(1),
            )
        })
        .collect();
    let t = Instant::now();
    let tiles = treemap::layout(&weights, rect).len();
    println!("per-frame pieces (single cold-ish shot):");
    println!(
        "  treemap::layout over {} items -> {tiles} tiles: {:.3}ms",
        weights.len(),
        ms(t.elapsed())
    );
    println!(
        "  filtered_indices-equivalent O(n) pass (no query), n={}: {filter_ms:.3}ms",
        result.files.len()
    );
    let t = Instant::now();
    let _ = filtered_indices(&result, "e");
    println!(
        "  filtered_indices-equivalent WITH query 'e' (allocating lowercase per file): {:.3}ms",
        ms(t.elapsed())
    );

    // 4. Frame probe: 300 simulated post-scan UI frames of CPU work.
    let mut frames = Vec::new();
    let mut frames_query = Vec::new();
    for _ in 0..300 {
        let t = Instant::now();
        let idx = filtered_indices(&result, "");
        std::hint::black_box(heatmap_layout(&result, &idx, rect));
        frames.push(ms(t.elapsed()));
    }
    for _ in 0..100 {
        let t = Instant::now();
        let idx = filtered_indices(&result, "e");
        std::hint::black_box(heatmap_layout(&result, &idx, rect));
        frames_query.push(ms(t.elapsed()));
    }
    println!("frame probe (headless CPU only):");
    report("scan phase: drain channel per ~60Hz tick", drain_ms);
    report("post-scan frame: filter + group + layout", frames);
    report("post-scan frame, search query 'e'", frames_query);
    println!(
        "  NOTE: measures only the CPU work app.rs does per frame (poll_scan drain, filtered_indices,\n  \
         paint_heatmap grouping + treemap layout) on this thread. It does NOT measure egui tessellation,\n  \
         painting, GPU/glow submission, vsync or input latency, so it is NOT a real frame time. During a\n  \
         scan app.result is None, so the UI draws no heatmap; only the drain cost applies."
    );
    println!("final peak RSS: {:.1} MiB", mib(peak_rss_bytes()));
}
