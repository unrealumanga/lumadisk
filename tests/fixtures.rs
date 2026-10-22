//! W-08: every edge case is a GENERATED tree, scanned with the real scanner and asserted on
//! reported total size and file count. Expected values come from the generator's manifest.
//! Run ignored/heavy cases with `cargo test --test fixtures -- --ignored <name>`.
use lumadisk::scanner::{ScanEvent, start_scan};
use std::path::Path;

#[path = "../tools/fixturegen.rs"]
mod fixturegen;
use fixturegen::{Manifest, Outcome};

struct Cleanup<'a>(&'a Path);
impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        fixturegen::cleanup(self.0);
    }
}

fn scan(root: &Path, duplicates: bool) -> lumadisk::model::ScanResult {
    let (sender, receiver) = std::sync::mpsc::channel();
    start_scan(root.to_path_buf(), true, duplicates, sender);
    loop {
        match receiver
            .recv()
            .expect("scanner thread ended without Complete")
        {
            ScanEvent::Complete(result) => return result.expect("scan failed"),
            ScanEvent::Progress { .. } => {}
        }
    }
}

fn assert_manifest(case: &str, m: &Manifest, r: &lumadisk::model::ScanResult) {
    assert_eq!(r.files.len() as u64, m.files, "{case}: reported file count");
    assert_eq!(
        r.total_logical.0, m.logical,
        "{case}: reported total logical size"
    );
    assert_eq!(
        r.hardlink_extra_count, m.hardlink_extra,
        "{case}: hardlink extras"
    );
    assert_eq!(
        r.unreadable_dir_count() as u64,
        m.unreadable_dirs,
        "{case}: unreadable dirs"
    );
    assert_eq!(r.file_error_count, 0, "{case}: file errors");
    if let Some(max) = m.max_physical {
        assert!(
            r.total_physical.0 <= max,
            "{case}: physical {} > {max}",
            r.total_physical.0
        );
    }
}

fn check(case: &str) {
    check_with(case, false);
}

fn check_with(case: &str, duplicates: bool) {
    let temp = tempfile::tempdir().unwrap();
    let _cleanup = Cleanup(temp.path());
    match fixturegen::generate(case, temp.path()).unwrap() {
        Outcome::Skipped(why) => println!("SKIPPED {case}: {why}"),
        Outcome::Ready(manifest) => {
            println!("{case}: {}", manifest.note);
            let result = scan(&manifest.scan_root, duplicates);
            assert_manifest(case, &manifest, &result);
            if case == "hardlink_set" {
                assert!(
                    result.duplicate_groups.is_empty(),
                    "hardlinks are not duplicates"
                );
            }
        }
    }
}

macro_rules! case {
    ($name:ident) => {
        #[test]
        fn $name() {
            check(stringify!($name));
        }
    };
    ($name:ident, ignore = $why:literal) => {
        #[test]
        #[ignore = $why]
        fn $name() {
            check(stringify!($name));
        }
    };
}

case!(deep_nesting);
case!(long_path_verbatim);
case!(long_path_plain);
case!(unicode_nfc_nfd);
case!(weird_names);
case!(case_collision);
case!(symlinks);
case!(zero_byte_mass);
case!(sparse_1tb);
case!(big_file_4gb);
case!(permission_denied);
case!(ui_separator);
case!(
    preset_1m,
    ignore = "heavy: 1M files; run on demand with --ignored"
);
case!(
    preset_10m,
    ignore = "heavy: 10M files; run on demand with --ignored"
);

#[test]
fn hardlink_set() {
    check_with("hardlink_set", true);
}

#[test]
#[cfg_attr(
    windows,
    ignore = "newline in a filename is Unix-only; UNVERIFIED on Linux/macOS here"
)]
fn newline_name() {
    check("newline_name");
}

#[test]
#[cfg_attr(not(windows), ignore = "NTFS junctions are Windows-only")]
fn junction_loop() {
    check("junction_loop");
}

macro_rules! stub {
    ($name:ident, $why:literal) => {
        #[test]
        #[ignore = $why]
        fn $name() {
            unimplemented!($why);
        }
    };
}

stub!(
    file_deleted_mid_scan,
    "stub: needs a hook/race in the scanner to delete during the walk"
);
stub!(
    file_growing_mid_scan,
    "stub: needs a writer racing the walk; expected = size at stat time"
);
stub!(
    file_renamed_mid_scan,
    "stub: rename race; must not double count or lose the file"
);
stub!(
    network_share,
    "stub: needs an SMB/NFS share; manual verification only"
);
stub!(
    fuse_mount,
    "stub: needs a FUSE mount (Linux); manual verification only"
);
stub!(
    removable_drive,
    "stub: needs a removable volume; manual verification only"
);
stub!(
    proc_sys,
    "stub: /proc and /sys report size 0 / pseudo files; Linux manual verification"
);
stub!(
    apfs_clone,
    "stub: needs APFS (cp -c); physical vs logical on clones, macOS only"
);
stub!(
    btrfs_reflink,
    "stub: needs btrfs/XFS (cp --reflink); Linux only"
);
stub!(
    exfat_volume,
    "stub: needs an exFAT volume (no hardlinks, 2s mtime); manual verification"
);
