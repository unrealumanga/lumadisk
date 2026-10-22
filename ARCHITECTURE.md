# LumaDisk architecture (as-is, v0.1.1, commit 8c9bae9)

~1,940 lines Rust: `main.rs` (window), `app.rs` (1454, all UI + state), `scanner.rs` (walk + dup finder), `model.rs` (FileEntry/ScanResult), `treemap.rs` (binary-split layout).

## Threads
- **Scanning does NOT run on the UI thread.** `scanner::start_scan` spawns one `std::thread`; `jwalk` walks with its own rayon pool. Dup finder uses rayon `par_iter`.
- Results: unbounded `std::sync::mpsc` channel. Progress every 1,000 files; one `Complete(ScanResult)` at the end. UI drains with `try_iter` per frame (`poll_scan`) and repaints every 80 ms while scanning.
- No streaming results (UI sees nothing but a counter until done), **no cancel**, no snapshot swap.
- Trash runs on its own spawned thread (`trash::delete_all`), polled the same way.
- Not-on-UI-thread, but UI-thread hazards exist: `move_selected` does `rfd` dialog + `fs::rename/copy` synchronously on the UI thread (copy fallback of a large file = freeze); `reveal` calls showfile on the UI thread.

## Data model
- (pre-W-02) Flat `Vec<FileEntry{path,name,extension,size,modified_unix}>`. **No directory tree.** Directories were never recorded. See "Data model after W-02".
- `size = metadata.len()` = logical size only. No physical size, no inode/dev, no hardlink/clone/sparse handling.
- `skipped: u64` counts walk/metadata errors; shown count only, no bytes, no path class.
- Dup groups hold `usize` indices into `files`; `remove_file` is O(n) + reindexes all groups.

## Data model after W-02 (issue #4)
- `FileEntry` carries `logical: LogicalSize`, `physical: Option<PhysicalSize>` (None = unknown),
  `links`, `id: Option<FileId>` (dev, ino) and `extra_link`. The two size newtypes only add to
  themselves (compile_fail doctests). Totals live in `ScanResult` as `total_logical` /
  `total_physical` plus `physical_unknown_count`, `hardlink_extra_count`, `file_error_count`,
  `duplicate_read_errors`, and `unaccounted_note()`.
- **Directory records, still no tree (choice):** `ScanResult.dirs: Vec<DirRecord{path, state}>`
  records every visited directory as `Read` or `Unreadable(ErrorClass)` (from jwalk's
  `read_children_error`). The flat file list stays because the whole UI indexes into it and D3/D4
  (snapshot handoff, directory-aggregated treemap) will redesign the model anyway; building a tree
  now would be rewritten. The records are enough to say "N directories unreadable - some space
  unaccounted" and to list them on hover; they are the seed for a later tree (parent links + rolled
  up sizes). Cost at 100k files / 3,906 dirs: a few MiB.
- Physical sizing: see `src/fsinfo.rs` and DECISIONS.md W-02 (Windows uses AllocationSize).

## Scan pipeline
jwalk (follow_links=false) → keep `is_file()` only → metadata → push. Sort by size desc. Dup: size groups (size>0) → `partial_hash` (blake3 of size + first ≤64K + last 64K if size>128K) → `full_hash` (blake3 stream). Partial stage is a pure function of (size, content) so identical files always collide: no false negative by construction. **Caveat (fixed in W-02):** an `Err` from open/read used to silently drop the file; now it is counted in `duplicate_read_errors` and shown. Hardlinked names are hashed once and never form a group on their own.
No hidden/pseudo-fs exclusion beyond dotfile toggle. No `(dev,ino)` visited tracking (symlinks not followed, but bind mounts/junctions/firmlinks untested).

## Treemap
`paint_heatmap` runs **per frame**: groups first 1,500 filtered indices by extension, calls `treemap::layout` (binary split, not squarified) for groups and again per group. Hard cap = 1,500 tiles. No layout caching. `filtered_indices()` also recomputed per frame (O(n) over all files).

## Deletion / reveal (README-claim audit)
- Trash: `trash` crate 5.2.6. Windows = native; Linux = in-process freedesktop impl (crate also has a `Command::new(trash)` path at freedesktop.rs:1410, reachable condition TBD).
- **macOS: crate default `DeleteMethod::Finder` = spawns `osascript`.** This contradicts the README "no command shell" claim, is an AV/behavioural signal, and triggers an Automation (Apple Events) TCC prompt. Fix: `set_delete_method(NsFileManager)`.
- `showfile` on Linux uses D-Bus (gio/zbus feature), not a shell. Verify per feature set.
- "Nothing is permanently deleted": `move_selected` falls back to copy + `remove_file` (permanent delete of source after copy; no hash verify). Soft contradiction — track in §2.

## CI / dist
`build.yml`: fmt, test, clippy, release build on 4 runners; packages zip / tar.gz / `.app.zip` + sha256. No signing, no tests beyond 4, no provenance.

## Open decisions (see DECISIONS.md)
Tile cap, tree vs flat model, cancel token design.
