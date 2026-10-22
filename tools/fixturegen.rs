//! Edge-case fixture generator (W-08, brief 6.2). std-only. Every case is generated into a
//! caller-supplied directory (a temp dir in tests) and returns a `Manifest` of what a CORRECT
//! scanner must report. Expected values come from this generator, never from the scanner.
//! Shared by `tools/fixture-gen.rs` (CLI) and `tests/fixtures.rs` via `#[path]`.
//! Output is never meant to be committed.
#![allow(dead_code)]
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

/// What a correct scan of `scan_root` must report.
#[derive(Debug, Clone)]
pub struct Manifest {
    /// Directory to scan (verbatim `\\?\` form for the long-path cases on Windows).
    pub scan_root: PathBuf,
    /// Entries in the reported file list (every hardlink name counts as an entry).
    pub files: u64,
    /// Reported total logical bytes (hardlinked storage counted once).
    pub logical: u64,
    /// Names listed as extra hardlinks.
    pub hardlink_extra: u64,
    /// Directories the scan must flag unreadable.
    pub unreadable_dirs: u64,
    /// Upper bound for reported total physical bytes (sparse cases).
    pub max_physical: Option<u64>,
    pub note: String,
}

impl Manifest {
    fn new(root: &Path) -> Self {
        Self {
            scan_root: root.to_path_buf(),
            files: 0,
            logical: 0,
            hardlink_extra: 0,
            unreadable_dirs: 0,
            max_physical: None,
            note: String::new(),
        }
    }
    /// Account for one regular file the scanner must count.
    fn add(&mut self, size: u64) {
        self.files += 1;
        self.logical += size;
    }
}

pub enum Outcome {
    Ready(Manifest),
    /// The case cannot be built here (missing privilege, wrong OS, unsupported filesystem).
    Skipped(String),
}

/// Cases implemented as generators.
pub const CASES: &[&str] = &[
    "deep_nesting",
    "long_path_verbatim",
    "long_path_plain",
    "unicode_nfc_nfd",
    "weird_names",
    "newline_name",
    "case_collision",
    "symlinks",
    "junction_loop",
    "hardlink_set",
    "sparse_1tb",
    "zero_byte_mass",
    "big_file_4gb",
    "permission_denied",
    "ui_separator",
    "preset_1m",
    "preset_10m",
];

pub fn generate(case: &str, out: &Path) -> io::Result<Outcome> {
    fs::create_dir_all(out)?;
    match case {
        "deep_nesting" => deep_nesting(out),
        "long_path_verbatim" => long_path(out, true),
        "long_path_plain" => long_path(out, false),
        "unicode_nfc_nfd" => unicode_nfc_nfd(out),
        "weird_names" => weird_names(out),
        "newline_name" => newline_name(out),
        "case_collision" => case_collision(out),
        "symlinks" => symlinks(out),
        "junction_loop" => junction_loop(out),
        "hardlink_set" => hardlink_set(out),
        "sparse_1tb" => sparse(out, 1 << 40),
        "big_file_4gb" => sparse(out, (4 << 30) + 1),
        "zero_byte_mass" => zero_byte_mass(out),
        "permission_denied" => permission_denied(out),
        "ui_separator" => ui_separator(out),
        "preset_1m" => preset(out, 1_000_000),
        "preset_10m" => preset(out, 10_000_000),
        other => Err(io::Error::other(format!("unknown case {other}"))),
    }
}

/// Undo anything that would stop the directory being deleted (ACL denies, chmod 000).
pub fn cleanup(out: &Path) {
    let denied = out.join("denied");
    if denied.exists() {
        #[cfg(windows)]
        {
            let _ = Command::new("icacls")
                .arg(&denied)
                .args(["/remove:d", "*S-1-1-0"])
                .output();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&denied, fs::Permissions::from_mode(0o755));
        }
    }
}

fn put(path: &Path, size: u64) -> io::Result<()> {
    if size <= 1 << 20 {
        fs::write(path, vec![0x5a_u8; size as usize])
    } else {
        fs::File::create(path)?.set_len(size)
    }
}

/// Windows: the verbatim form lifts MAX_PATH. Elsewhere the path is returned unchanged.
fn extended(path: &Path) -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        fs::canonicalize(path) // always yields \\?\C:\...
    }
    #[cfg(not(windows))]
    {
        Ok(path.to_path_buf())
    }
}

fn is_name_too_long(error: &io::Error) -> bool {
    // ENAMETOOLONG: 36 on Linux, 63 on macOS.
    matches!(error.raw_os_error(), Some(36 | 63))
}

fn deep_nesting(out: &Path) -> io::Result<Outcome> {
    const DEPTH: usize = 1100;
    let base = extended(out)?;
    let mut manifest = Manifest::new(&base);
    let mut path = base;
    for level in 0..=DEPTH {
        if level > 0 {
            path.push("d");
            if let Err(error) = fs::create_dir(&path) {
                return if is_name_too_long(&error) {
                    Ok(Outcome::Skipped(format!("OS path limit at level {level}")))
                } else {
                    Err(error)
                };
            }
        }
        if level % 100 == 0 {
            let size = level as u64 + 1;
            put(&path.join("f.bin"), size)?;
            manifest.add(size);
        }
    }
    manifest.note = format!("{DEPTH} nested directories, files every 100 levels");
    Ok(Outcome::Ready(manifest))
}

fn long_path(out: &Path, verbatim: bool) -> io::Result<Outcome> {
    let base = extended(out)?;
    let mut dir = base.clone();
    for part in ["a", "b", "c"] {
        dir.push(part.repeat(100));
    }
    fs::create_dir_all(&dir)?;
    let mut manifest = Manifest::new(if verbatim { &base } else { out });
    for (name, size) in [("one.bin", 11_u64), ("two.bin", 22)] {
        put(&dir.join(name), size)?;
        manifest.add(size);
    }
    put(&base.join("top.bin"), 5)?;
    manifest.add(5);
    manifest.note = format!(
        "deepest file path is {} chars",
        dir.join("one.bin").as_os_str().len()
    );
    Ok(Outcome::Ready(manifest))
}

/// Create `names[i]` with `sizes[i]` bytes. If the filesystem folds a later name onto an earlier
/// one (case-insensitive or normalisation-insensitive) the fold is detected by probing, and the
/// manifest counts one file for that group, with the first size.
fn colliding(out: &Path, names: &[&str], sizes: &[u64]) -> io::Result<Outcome> {
    let mut manifest = Manifest::new(out);
    put(&out.join(names[0]), sizes[0])?;
    manifest.add(sizes[0]);
    for (name, size) in names.iter().zip(sizes).skip(1) {
        let path = out.join(name);
        if fs::symlink_metadata(&path).is_ok() {
            manifest
                .note
                .push_str(&format!("{name:?} folds onto an earlier name; "));
        } else {
            put(&path, *size)?;
            manifest.add(*size);
        }
    }
    Ok(Outcome::Ready(manifest))
}

fn unicode_nfc_nfd(out: &Path) -> io::Result<Outcome> {
    // U+00E9 (NFC) vs e + U+0301 (NFD); APFS folds them, NTFS and ext4 keep both.
    colliding(out, &["caf\u{e9}.txt", "cafe\u{301}.txt"], &[10, 20])
}

fn case_collision(out: &Path) -> io::Result<Outcome> {
    // The directory itself is probed: NTFS/APFS default folds, ext4 does not.
    colliding(out, &["File.txt", "file.txt", "FILE.TXT"], &[10, 20, 30])
}

fn weird_names(out: &Path) -> io::Result<Outcome> {
    let mut manifest = Manifest::new(out);
    let names = [
        "\u{1F600}smile.txt",
        "rtl\u{200F}mark.txt",
        "\u{202E}gpj.txt", // right-to-left override
        "zero\u{200B}width\u{200D}joiner.bin",
        "\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}.png", // ZWJ emoji sequence
    ];
    for (i, name) in names.iter().enumerate() {
        let size = 10 * (i as u64 + 1);
        put(&out.join(name), size)?;
        manifest.add(size);
    }
    Ok(Outcome::Ready(manifest))
}

fn newline_name(out: &Path) -> io::Result<Outcome> {
    #[cfg(unix)]
    {
        let mut manifest = Manifest::new(out);
        put(&out.join("line1\nline2.txt"), 13)?;
        manifest.add(13);
        put(&out.join("plain.txt"), 7)?;
        manifest.add(7);
        Ok(Outcome::Ready(manifest))
    }
    #[cfg(not(unix))]
    {
        let _ = out;
        Ok(Outcome::Skipped(
            "newline in a filename is not creatable on Windows".into(),
        ))
    }
}

#[cfg(any(unix, windows))]
fn make_dir_link(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link)
    }
}

#[cfg(any(unix, windows))]
fn make_file_link(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link)
    }
}

fn symlinks(out: &Path) -> io::Result<Outcome> {
    let real = out.join("real");
    fs::create_dir_all(real.join("sub"))?;
    let mut manifest = Manifest::new(out);
    put(&real.join("a.bin"), 100)?;
    manifest.add(100);
    put(&real.join("sub").join("b.bin"), 200)?;
    manifest.add(200);
    // Loop (link to its own directory), then to-parent, broken, and a file link.
    if let Err(error) = make_dir_link(&real, &real.join("loop")) {
        return Ok(Outcome::Skipped(format!(
            "cannot create symlinks here (privilege?): {error}"
        )));
    }
    make_dir_link(Path::new(".."), &real.join("sub").join("up"))?;
    make_file_link(Path::new("does-not-exist"), &out.join("broken"))?;
    make_file_link(&real.join("a.bin"), &out.join("filelink"))?;
    manifest.note = "symlinks are never followed or sized".into();
    Ok(Outcome::Ready(manifest))
}

fn junction_loop(out: &Path) -> io::Result<Outcome> {
    #[cfg(windows)]
    {
        let real = out.join("real");
        fs::create_dir_all(real.join("sub"))?;
        let mut manifest = Manifest::new(out);
        put(&real.join("a.bin"), 100)?;
        manifest.add(100);
        put(&real.join("sub").join("b.bin"), 200)?;
        manifest.add(200);
        for link in [real.join("sub").join("jup"), real.join("jself")] {
            let status = Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&real)
                .output()?;
            if !status.status.success() {
                return Ok(Outcome::Skipped("mklink /J failed".into()));
            }
        }
        manifest.note = "junctions back to an ancestor must not be followed".into();
        Ok(Outcome::Ready(manifest))
    }
    #[cfg(not(windows))]
    {
        let _ = out;
        Ok(Outcome::Skipped("junctions are Windows-only".into()))
    }
}

fn hardlink_set(out: &Path) -> io::Result<Outcome> {
    for dir in ["a", "b", "c/deep"] {
        fs::create_dir_all(out.join(dir))?;
    }
    let original = out.join("a").join("orig.bin");
    put(&original, 5000)?;
    for link in [out.join("b").join("l1.bin"), out.join("c/deep/l2.bin")] {
        fs::hard_link(&original, link)?;
    }
    put(&out.join("solo.bin"), 700)?;
    let mut manifest = Manifest::new(out);
    manifest.files = 4;
    manifest.logical = 5700; // the 5000-byte storage counts once
    manifest.hardlink_extra = 2;
    Ok(Outcome::Ready(manifest))
}

fn sparse(out: &Path, apparent: u64) -> io::Result<Outcome> {
    let path = out.join("sparse.bin");
    fs::File::create(&path)?;
    #[cfg(windows)]
    {
        let status = Command::new("fsutil")
            .args(["sparse", "setflag"])
            .arg(&path)
            .output()?;
        if !status.status.success() {
            return Ok(Outcome::Skipped("fsutil sparse setflag failed".into()));
        }
    }
    if let Err(error) = fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .and_then(|file| file.set_len(apparent))
    {
        return Ok(Outcome::Skipped(format!(
            "filesystem cannot hold a {apparent}-byte sparse file: {error}"
        )));
    }
    put(&out.join("small.bin"), 1000)?;
    let mut manifest = Manifest::new(out);
    manifest.add(apparent);
    manifest.add(1000);
    // Allocation is (nearly) zero; allow a generous 4 MiB for cluster/metadata rounding.
    manifest.max_physical = Some(4 << 20);
    Ok(Outcome::Ready(manifest))
}

fn zero_byte_mass(out: &Path) -> io::Result<Outcome> {
    let mut manifest = Manifest::new(out);
    for dir in 0..20 {
        let dir_path = out.join(format!("d{dir:02}"));
        fs::create_dir_all(&dir_path)?;
        for file in 0..1000 {
            fs::File::create(dir_path.join(format!("z{file}.dat")))?;
            manifest.add(0);
        }
    }
    put(&out.join("five.bin"), 5)?;
    manifest.add(5);
    Ok(Outcome::Ready(manifest))
}

fn permission_denied(out: &Path) -> io::Result<Outcome> {
    let denied = out.join("denied");
    fs::create_dir_all(&denied)?;
    put(&denied.join("hidden.bin"), 999)?; // not reachable, must NOT be counted
    put(&out.join("open.bin"), 40)?;
    let mut manifest = Manifest::new(out);
    manifest.add(40);
    manifest.unreadable_dirs = 1;
    #[cfg(windows)]
    {
        let status = Command::new("icacls")
            .arg(&denied)
            .args(["/deny", "*S-1-1-0:(RD)"])
            .output()?;
        if !status.status.success() {
            return Ok(Outcome::Skipped("icacls /deny failed".into()));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o000))?;
    }
    if fs::read_dir(&denied).is_ok() {
        cleanup(out);
        return Ok(Outcome::Skipped(
            "directory is still readable (running as root/admin?)".into(),
        ));
    }
    Ok(Outcome::Ready(manifest))
}

fn ui_separator(out: &Path) -> io::Result<Outcome> {
    // The UI has no fixed separator string yet, so cover the usual candidates in names.
    let mut manifest = Manifest::new(out);
    for (i, dir) in [
        "x \u{203a} y",
        "x \u{bb} y",
        "x - y",
        "x \u{2192} y",
        "x , y",
    ]
    .iter()
    .enumerate()
    {
        fs::create_dir_all(out.join(dir))?;
        let size = 100 * (i as u64 + 1);
        put(&out.join(dir).join("p \u{203a} q.txt"), size)?;
        manifest.add(size);
    }
    Ok(Outcome::Ready(manifest))
}

fn preset(out: &Path, total: u64) -> io::Result<Outcome> {
    let mut manifest = Manifest::new(out);
    for index in 0..total {
        let dir = out.join(format!("d{:05}", index / 1000));
        if index % 1000 == 0 {
            fs::create_dir_all(&dir)?;
        }
        let path = dir.join(format!("f{index}"));
        let size = index % 4;
        if size == 0 {
            fs::File::create(&path)?;
        } else {
            put(&path, size)?;
        }
        manifest.add(size);
    }
    Ok(Outcome::Ready(manifest))
}
