# State (generated from orchestration/events.jsonl - do not hand-edit)

- §0 W-00 [merged] orch: ARCHITECTURE + baseline docs
- §0 W-01 [merged] DeepSeek-1: bench harness, 10k/100k/1M baseline (fcde2c8)
- §1 W-04 [merged] DeepSeek: unsigned dmg/AppImage/deb + gated signing; CI-UNVERIFIED until a runner executes it
- §1 W-05 [blocked] user: signing needs Apple acct, HSM cert, VMs
- §2 W-02 [merged] Astra-6: Logical/Physical types, hardlinks once, unreadable dirs, dup-finder hardening; 14+4 tests; scan +7% at 100k
- §3 E-D3 [in_flight] DeepSeek: experiment: UI-fold vs aggregator+arc-swap at 10M (.wt/E-D3)
- §3 W-06 [queued] orch: cancel/snapshot/treemap cache: needs D3+D4 debates first
- §4 W-07 [merged] DeepSeek: TCC detect+banner (macOS UNVERIFIED; open_url may not open pane: fallback copy)
- §5 W-03 [merged] DeepSeek: NSFileManager (macOS, untested) + 50-batch trash; Linux shell path is test-only
- §6 W-08 [in_flight] DeepSeek: fixture-gen + fixtures tests

Metrics:
- cargo_test/tests_unit = 15
- scan_100k_after_w02/scan_100k_wall_s = 2.42
- scan_1m/peak_rss_mib = 393
- scan_1m/scan_wall_s = 37.8
- scan_1m_postscan_noquery/frametime_p99_ms = 16.6
- scan_1m_postscan_query/frametime_p99_ms = 336

Recent notes:
