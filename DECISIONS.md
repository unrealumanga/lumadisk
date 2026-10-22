# Decisions

## P0.2 measurement harness

- No new dependencies. Peak RSS on Windows uses std-only FFI to `K32GetProcessMemoryInfo`
  (kernel32, `PeakWorkingSetSize`); on Linux it reads `VmHWM` from `/proc/self/status`.
  `windows-sys` was rejected because one function and one struct do not justify a crate.
- `src/lib.rs` re-exports `model`, `scanner`, `treemap` so `benches/scan.rs` calls the real code.
  `main.rs` still declares its own modules, so the app build is unchanged. Side effect: unit tests
  in those modules now run twice (lib target and bin target).
- `scanner::find_duplicate_groups` was made `pub` (visibility only, no behaviour change) so the
  dup-finder can be timed in isolation.
- The tree generator lives in `tools/treegen.rs`, used by `examples/gen_tree.rs`. Trees are
  written to a directory the caller chooses (use a temp dir); nothing is committed.
