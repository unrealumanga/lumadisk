<!-- Pre-existing tests: 4 unique (8 executions after src/lib.rs duplicate-compiles modules). Orchestrator note. -->
# Baseline

Tests: `cargo test --locked` = 4 unique unit tests (8 executions, because `src/lib.rs` now compiles
the same modules into a lib target as well as the bin target).

## P0.2 BEFORE numbers (no scan behaviour changed)

Machine: Intel Core i9-12900KS (24 logical threads), 127.7 GB RAM, WDS100T1XHE NVMe/SSD, Windows 11 Pro
10.0.26200, rustc 1.94.0, release (`cargo bench` profile, thin LTO). Single run per size, so treat as
indicative, not statistical.

Cache state: every tree was generated immediately before its run, on a machine with ample RAM, so
all runs are WARM-cache (metadata and file contents in the OS cache). A cold run (after reboot or
cache flush) was NOT measured and will be slower, especially the dup finder (reads file contents).

Reproduce:

    cargo run --release --example gen_tree -- --preset 100k --out <TEMPDIR>
    cargo bench --bench scan -- <TEMPDIR> --label 100k

Trees: depth 5, ~5% exact-duplicate files, sizes 90% 100B-4KB / 9% 4-64KB / 0.99% 64KB-1MB / 0.01% 1-8MB.
10k = 100.4 MiB, 100k = 1047 MiB, 1M = 10308 MiB (generation of 1M took 262 s).

| Metric | 10k | 100k | 1M |
|---|---|---|---|
| Scan wall time (dups off, include_hidden) | 0.272 s | 2.492 s | 37.843 s |
| Peak RSS after scan (peak working set) | 20.3 MiB | 57.8 MiB | 393.1 MiB |
| Dup finder `find_duplicate_groups` | 0.086 s (488 groups) | 1.031 s (2904) | 15.103 s (4096) |
| Peak RSS after dup finder | 32.5 MiB | 75.2 MiB | 393.1 MiB (final 408.4) |
| `treemap::layout`, 1,500 items | 0.051 ms | 0.042 ms | 0.101 ms |
| filtered_indices-equivalent, no query | 0.196 ms | 0.914 ms | 11.723 ms |
| filtered_indices-equivalent, query "e" | 1.503 ms | 19.878 ms | 252.958 ms |

Frame probe (headless CPU cost per simulated UI frame; 300 frames, 100 for the query case):

| Probe | 10k p50/p95/p99 | 100k p50/p95/p99 | 1M p50/p95/p99 |
|---|---|---|---|
| Scan phase: channel drain per ~60 Hz tick | 0.003/0.005/0.009 ms | 0.003/0.008/0.010 ms | 0.002/0.006/0.008 ms |
| Post-scan frame: filter + group + layout | 0.123/0.135/0.181 ms | 0.613/0.806/0.975 ms | 9.056/14.107/16.626 ms |
| Post-scan frame with search query "e" | 1.266/1.413/1.630 ms | 20.401/25.447/27.378 ms | 250.356/310.306/336.118 ms |

What the frame probe does: replicates the CPU work `app.rs` does on the UI thread, namely
`poll_scan` draining the channel, `filtered_indices`, and the grouping + `treemap::layout` part of
`paint_heatmap` (1,500-tile cap). What it does NOT measure: egui tessellation, painting, hover and
interaction handling, GPU/glow submission, vsync or input latency. It is not a real frame time.
During a scan `app.result` is `None`, so no heatmap is drawn; only the drain cost applies.

Notes:
- The scan and the dup finder were run separately (scan with dups off) so each is isolated; the app
  runs both when "find duplicates" is on (the default).
- Peak RSS: Windows via `K32GetProcessMemoryInfo` PeakWorkingSetSize; the generator runs in a separate
  process so it does not inflate the number.
- Observed: the per-frame cost at 1M is dominated by the full O(n) `filtered_indices` pass that runs
  every frame (the 1,500-tile layout itself is ~0.1 ms), and a search query makes it roughly 250 ms.

## W-02 AFTER numbers (100k tree, same machine, 2026-10-22)

A/B on the SAME freshly generated 100k tree: base = `git archive 718589e` built separately, W-02 =
this branch, alternated 3 times each. Run 1 of each pair is warm-up noise (AV / cache after
generation: base 3.86 s, W-02 3.91-4.23 s) and is excluded. Single machine, warm cache.

| Metric | base (718589e) | W-02 | Delta |
|---|---|---|---|
| Scan wall time (dups off) | 2.265 / 2.267 s | 2.418 / 2.428 s | about +7% (+0.16 s) |
| Peak RSS after scan | 58.1 / 60.3 MiB | 64.6 / 68.1 MiB | +4 to +8 MiB (bigger FileEntry + 3,906 DirRecords) |
| Dup finder | 0.949 - 1.105 s | 0.976 - 1.217 s | within noise |
| Post-scan frame p50 (filter + group + layout) | 0.576 - 0.595 ms | 0.531 - 0.535 ms | within noise |
| Post-scan frame p50, query "e" | 15.3 - 18.1 ms | 16.0 - 16.7 ms | within noise |

Totals reported by W-02 on this tree: logical 1047.0 MiB, physical 1189.0 MiB (+13.6%: 90% of files
are 100 B - 4 KB, each rounded up to whole 4 KiB clusters), 0 unreadable, 0 unknown.
The scan cost comes from one extra `GetFileInformationByHandleEx` call per already-open handle
plus wider entries; the handle count per file is unchanged (std's `symlink_metadata` also opens one).
