use crate::{
    fsinfo,
    model::{
        DirRecord, DirState, DuplicateGroup, ErrorClass, FileEntry, FileId, LogicalSize,
        PhysicalSize, ScanResult, StorageKey,
    },
};
use rayon::prelude::*;
use std::{
    collections::{HashMap, hash_map::Entry},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::Sender,
    },
    time::Instant,
};

pub enum ScanEvent {
    Progress {
        files: usize,
        /// Known physical bytes seen so far (hardlinks not yet de-duplicated).
        bytes: PhysicalSize,
        current: PathBuf,
        phase: &'static str,
    },
    Complete(Result<ScanResult, String>),
}

pub fn start_scan(
    root: PathBuf,
    include_hidden: bool,
    find_duplicates: bool,
    sender: Sender<ScanEvent>,
) {
    std::thread::spawn(move || {
        let result = scan(&root, include_hidden, find_duplicates, &sender);
        let _ = sender.send(ScanEvent::Complete(result));
    });
}

fn scan(
    root: &Path,
    include_hidden: bool,
    find_duplicates: bool,
    sender: &Sender<ScanEvent>,
) -> Result<ScanResult, String> {
    if !root.is_dir() {
        return Err("Choose a folder that exists and can be read.".into());
    }

    let started = Instant::now();
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut seen_physical = PhysicalSize(0);
    let mut file_error_count = 0_u64;

    // follow_links(false): symlinks and Windows junctions are reported as symlinks and are
    // never descended into or sized, so loops and links out of the root cannot inflate totals.
    for item in jwalk::WalkDir::new(root)
        .follow_links(false)
        .skip_hidden(false)
    {
        let entry = match item {
            Ok(entry) => entry,
            Err(_) => {
                file_error_count += 1;
                continue;
            }
        };
        let file_type = entry.file_type();
        let path = entry.path();
        if !include_hidden && is_hidden(&path, root) {
            continue;
        }
        if file_type.is_dir() {
            let state = match &entry.read_children_error {
                Some(error) => DirState::Unreadable(ErrorClass::from_io(error.io_error())),
                None => DirState::Read,
            };
            dirs.push(DirRecord { path, state });
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let info = match fsinfo::query(&path) {
            Ok(info) => info,
            Err(_) => {
                file_error_count += 1;
                continue;
            }
        };
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let extension = path
            .extension()
            .map(|value| value.to_string_lossy().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "no extension".to_owned());

        let physical = info.physical.map(PhysicalSize);
        seen_physical += physical.unwrap_or_default();
        files.push(FileEntry {
            path: path.clone(),
            name,
            extension,
            logical: LogicalSize(info.logical),
            physical,
            modified_unix: info.modified_unix,
            links: info.links,
            id: info.id,
            extra_link: false,
        });

        if files.len() % 1_000 == 0 {
            let _ = sender.send(ScanEvent::Progress {
                files: files.len(),
                bytes: seen_physical,
                current: path,
                phase: "Scanning metadata",
            });
        }
    }

    let hardlink_extra_count = mark_extra_links(&mut files);
    let total_logical: LogicalSize = files.iter().map(FileEntry::counted_logical).sum();
    let total_physical: PhysicalSize = files.iter().filter_map(FileEntry::counted_physical).sum();
    let physical_unknown_count = files
        .iter()
        .filter(|file| file.counted_physical().is_none())
        .count() as u64;

    // Default display is physical (D2); unknown physical sorts last.
    files.sort_unstable_by_key(|file| {
        std::cmp::Reverse((file.counted_physical().map(|size| size.0), file.logical))
    });
    let (duplicate_groups, duplicate_read_errors) = if find_duplicates {
        let _ = sender.send(ScanEvent::Progress {
            files: files.len(),
            bytes: total_physical,
            current: root.to_path_buf(),
            phase: "Checking duplicate candidates",
        });
        let found = find_duplicate_groups(&files);
        (found.groups, found.read_errors)
    } else {
        (Vec::new(), 0)
    };

    Ok(ScanResult {
        files,
        dirs,
        duplicate_groups,
        total_logical,
        total_physical,
        physical_unknown_count,
        hardlink_extra_count,
        file_error_count,
        duplicate_read_errors,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

/// For every storage with several scanned names, the lexicographically first path is the counted
/// one and the rest become `extra_link`s (listed, zero bytes counted). Returns the extra count.
fn mark_extra_links(files: &mut [FileEntry]) -> u64 {
    let mut linked: Vec<usize> = files
        .iter()
        .enumerate()
        .filter(|(_, file)| file.links > 1 && file.id.is_some())
        .map(|(index, _)| index)
        .collect();
    linked.sort_unstable_by(|a, b| files[*a].path.cmp(&files[*b].path));
    let mut first: HashMap<FileId, usize> = HashMap::new();
    let mut extra = 0;
    for index in linked {
        let id = files[index].id.expect("filtered on is_some");
        if let Entry::Vacant(slot) = first.entry(id) {
            slot.insert(index);
        } else {
            files[index].extra_link = true;
            extra += 1;
        }
    }
    extra
}

fn is_hidden(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .any(|part| part.as_os_str().to_string_lossy().starts_with('.'))
}

pub struct DuplicateScan {
    pub groups: Vec<DuplicateGroup>,
    /// Candidate files dropped because opening/reading them failed (counted, never silent).
    pub read_errors: u64,
}

/// Content duplicates, matched by logical size, then sampled hash, then full BLAKE3.
///
/// Hardlinked names of one storage are hashed once and are never duplicates of each other:
/// a size bucket needs two *distinct storages* to form a group. Savings are physical.
pub fn find_duplicate_groups(files: &[FileEntry]) -> DuplicateScan {
    let mut by_size: HashMap<u64, Vec<usize>> = HashMap::new();
    for (index, file) in files
        .iter()
        .enumerate()
        .filter(|(_, file)| file.logical.0 > 0)
    {
        by_size.entry(file.logical.0).or_default().push(index);
    }

    // Each candidate is (representative index, every scanned name of that storage).
    type Storage = (usize, Vec<usize>);
    let candidates: Vec<(u64, Vec<Storage>)> = by_size
        .into_iter()
        .filter(|(_, indices)| indices.len() > 1)
        .filter_map(|(size, indices)| {
            let mut storages: Vec<Storage> = Vec::new();
            let mut slot: HashMap<StorageKey, usize> = HashMap::new();
            for index in indices {
                match slot.entry(files[index].storage_key(index)) {
                    Entry::Occupied(position) => storages[*position.get()].1.push(index),
                    Entry::Vacant(position) => {
                        position.insert(storages.len());
                        storages.push((index, vec![index]));
                    }
                }
            }
            (storages.len() > 1).then_some((size, storages))
        })
        .collect();

    let read_errors = AtomicU64::new(0);
    let hash_stage =
        |groups: Vec<(u64, Vec<Storage>)>,
         hash: &(dyn Fn(&Path, u64) -> std::io::Result<[u8; 32]> + Sync)| {
            groups
                .into_par_iter()
                .flat_map_iter(|(size, storages)| {
                    let mut hashes: HashMap<[u8; 32], Vec<Storage>> = HashMap::new();
                    for storage in storages {
                        match hash(&files[storage.0].path, size) {
                            Ok(digest) => hashes.entry(digest).or_default().push(storage),
                            Err(_) => {
                                read_errors.fetch_add(storage.1.len() as u64, Ordering::Relaxed);
                            }
                        }
                    }
                    hashes
                        .into_values()
                        .filter(|group| group.len() > 1)
                        .map(move |group| (size, group))
                })
                .collect::<Vec<_>>()
        };

    let partial = hash_stage(candidates, &partial_hash);
    let full = hash_stage(partial, &|path, _| full_hash(path));

    let mut groups: Vec<DuplicateGroup> = full
        .into_iter()
        .map(|(size, storages)| {
            let indices = storages.into_iter().flat_map(|(_, names)| names).collect();
            DuplicateGroup::new(LogicalSize(size), indices, files)
        })
        .collect();

    groups.sort_unstable_by_key(|group| std::cmp::Reverse(group.reclaimable));
    DuplicateScan {
        groups,
        read_errors: read_errors.into_inner(),
    }
}

fn partial_hash(path: &Path, size: u64) -> std::io::Result<[u8; 32]> {
    const SAMPLE: usize = 64 * 1024;
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());
    let mut buffer = vec![0_u8; SAMPLE.min(size as usize)];
    file.read_exact(&mut buffer)?;
    hasher.update(&buffer);
    if size > SAMPLE as u64 * 2 {
        file.seek(SeekFrom::End(-(SAMPLE as i64)))?;
        let mut tail = vec![0_u8; SAMPLE];
        file.read_exact(&mut tail)?;
        hasher.update(&tail);
    }
    Ok(*hasher.finalize().as_bytes())
}

fn full_hash(path: &Path) -> std::io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(&mut file)?;
    Ok(*hasher.finalize().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SizeMode;
    use std::fs;

    fn scan_dir(path: &Path, find_duplicates: bool) -> ScanResult {
        let (sender, _receiver) = std::sync::mpsc::channel();
        scan(path, true, find_duplicates, &sender).unwrap()
    }

    fn physical_of(path: &Path) -> PhysicalSize {
        PhysicalSize(fsinfo::query(path).unwrap().physical.unwrap())
    }

    #[test]
    fn scans_sizes_and_confirms_exact_duplicates() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("original.bin"), b"same bytes").unwrap();
        fs::write(temp.path().join("copy.bin"), b"same bytes").unwrap();
        fs::write(temp.path().join("different.bin"), b"other data").unwrap();
        let (sender, _receiver) = std::sync::mpsc::channel();

        let result = scan(temp.path(), false, true, &sender).unwrap();

        assert_eq!(result.files.len(), 3);
        assert_eq!(result.total_logical, LogicalSize(30));
        assert_eq!(result.duplicate_groups.len(), 1);
        assert_eq!(result.duplicate_groups[0].file_indices.len(), 2);
        // Savings are reported physically: one copy's allocation.
        assert_eq!(
            result.duplicate_bytes(),
            physical_of(&temp.path().join("copy.bin"))
        );
    }

    #[test]
    fn excludes_dotfiles_when_hidden_files_are_disabled() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("visible.txt"), b"visible").unwrap();
        fs::write(temp.path().join(".hidden.txt"), b"hidden").unwrap();
        let (sender, _receiver) = std::sync::mpsc::channel();

        let result = scan(temp.path(), false, false, &sender).unwrap();

        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].name, "visible.txt");
    }

    #[test]
    fn hardlink_set_is_counted_once() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("a.bin");
        fs::write(&a, vec![3_u8; 10_000]).unwrap();
        fs::create_dir(temp.path().join("sub")).unwrap();
        fs::hard_link(&a, temp.path().join("b.bin")).unwrap();
        fs::hard_link(&a, temp.path().join("sub").join("c.bin")).unwrap();
        let unique = temp.path().join("unique.bin");
        fs::write(&unique, vec![4_u8; 3_000]).unwrap();

        let result = scan_dir(temp.path(), false);

        assert_eq!(result.files.len(), 4, "every name is listed");
        assert_eq!(result.hardlink_extra_count, 2);
        assert_eq!(result.files.iter().filter(|f| f.extra_link).count(), 2);
        assert_eq!(result.total_logical, LogicalSize(13_000));
        assert_eq!(
            result.total_physical,
            physical_of(&a) + physical_of(&unique)
        );
        assert_eq!(result.physical_unknown_count, 0);
        let listed: u64 = result
            .files
            .iter()
            .filter_map(|f| f.counted_bytes(SizeMode::Logical))
            .sum();
        assert_eq!(listed, 13_000, "list sums match the total");
    }

    #[test]
    fn sparse_file_has_distinct_logical_and_physical_size() {
        const LEN: u64 = 64 * 1024 * 1024;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sparse.img");
        if let Err(reason) = make_sparse(&path, LEN) {
            println!("SKIPPED sparse_file_has_distinct_logical_and_physical_size: {reason}");
            return;
        }

        let result = scan_dir(temp.path(), false);

        assert_eq!(result.files.len(), 1);
        assert_eq!(result.total_logical, LogicalSize(LEN));
        assert!(
            result.total_physical.0 < 1024 * 1024,
            "physical {:?} should be far below logical {LEN}",
            result.total_physical
        );
        assert_eq!(result.total_physical, physical_of(&path));
    }

    #[cfg(unix)]
    fn make_sparse(path: &Path, len: u64) -> Result<(), String> {
        let file = File::create(path).map_err(|e| e.to_string())?;
        file.set_len(len).map_err(|e| e.to_string())?;
        let physical = fsinfo::query(path).map_err(|e| e.to_string())?.physical;
        if physical.unwrap_or(len) >= len {
            return Err("filesystem did not create a hole (no sparse support)".into());
        }
        Ok(())
    }

    #[cfg(windows)]
    fn make_sparse(path: &Path, len: u64) -> Result<(), String> {
        File::create(path).map_err(|e| e.to_string())?;
        let target = path.to_string_lossy().into_owned();
        let len = len.to_string();
        for args in [
            vec!["sparse", "setflag", &target],
            vec!["file", "seteof", &target, &len],
            vec!["sparse", "setrange", &target, "0", &len],
        ] {
            let output = std::process::Command::new("fsutil")
                .args(&args)
                .output()
                .map_err(|e| format!("fsutil not available ({e})"))?;
            if !output.status.success() {
                return Err(format!(
                    "fsutil {args:?} failed: {}",
                    String::from_utf8_lossy(&output.stdout).trim()
                ));
            }
        }
        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    fn make_sparse(_: &Path, _: u64) -> Result<(), String> {
        Err("no sparse helper on this platform".into())
    }

    /// Creates a symlink to `target` at `link`; `Err` = cannot on this machine (privilege).
    fn symlink(target: &Path, link: &Path, dir: bool) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            let _ = dir;
            std::os::unix::fs::symlink(target, link)
        }
        #[cfg(windows)]
        {
            if dir {
                std::os::windows::fs::symlink_dir(target, link)
            } else {
                std::os::windows::fs::symlink_file(target, link)
            }
        }
    }

    #[test]
    fn symlink_loop_parent_and_broken_links_are_not_followed_or_sized() {
        let outer = tempfile::tempdir().unwrap();
        fs::write(outer.path().join("outside.bin"), vec![1_u8; 50_000]).unwrap();
        let root = outer.path().join("root");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("real.bin"), vec![2_u8; 1_234]).unwrap();
        let links = [
            (root.clone(), root.join("loop_to_self"), true),
            (outer.path().to_path_buf(), root.join("to_parent"), true),
            (root.join("missing.bin"), root.join("broken"), false),
            (root.join("real.bin"), root.join("file_link"), false),
        ];
        for (target, link, dir) in &links {
            if let Err(error) = symlink(target, link, *dir) {
                println!(
                    "SKIPPED symlink_loop_parent_and_broken_links_are_not_followed_or_sized: \
                     cannot create symlinks here ({error}); on Windows this needs Developer Mode \
                     or SeCreateSymbolicLinkPrivilege. Junctions are covered separately."
                );
                return;
            }
        }

        let result = scan_dir(&root, true);

        assert_eq!(result.files.len(), 1, "{:?}", result.files);
        assert_eq!(result.files[0].name, "real.bin");
        assert_eq!(result.total_logical, LogicalSize(1_234));
        assert_eq!(result.dirs.len(), 1, "only the root directory is visited");
        assert_eq!(result.unreadable_dir_count(), 0);
        assert!(result.duplicate_groups.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn junction_loop_and_junction_to_parent_are_not_followed() {
        let outer = tempfile::tempdir().unwrap();
        fs::write(outer.path().join("outside.bin"), vec![1_u8; 50_000]).unwrap();
        let root = outer.path().join("root");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("real.bin"), vec![2_u8; 1_234]).unwrap();
        for (name, target) in [("loop", &root), ("up", &outer.path().to_path_buf())] {
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(root.join(name))
                .arg(target)
                .output();
            match status {
                Ok(output) if output.status.success() => {}
                other => {
                    println!("SKIPPED junction test: mklink /J failed ({other:?})");
                    return;
                }
            }
        }

        let result = scan_dir(&root, false);

        assert_eq!(result.files.len(), 1);
        assert_eq!(result.total_logical, LogicalSize(1_234));
        assert_eq!(result.dirs.len(), 1);
    }

    #[test]
    fn unreadable_directory_is_reported_not_sized_zero() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("ok.bin"), vec![5_u8; 700]).unwrap();
        let locked = temp.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("secret.bin"), vec![6_u8; 9_000]).unwrap();
        if let Err(reason) = deny_listing(&locked) {
            println!("SKIPPED unreadable_directory_is_reported_not_sized_zero: {reason}");
            return;
        }

        let result = scan_dir(temp.path(), false);
        restore_listing(&locked);

        assert_eq!(result.files.len(), 1);
        assert_eq!(result.total_logical, LogicalSize(700));
        assert_eq!(result.unreadable_dir_count(), 1);
        let record = result.dirs.iter().find(|d| d.path == locked).unwrap();
        assert_eq!(
            record.state,
            DirState::Unreadable(ErrorClass::PermissionDenied)
        );
        let note = result.unaccounted_note().unwrap();
        assert!(note.contains("1 directory unreadable"), "{note}");
        assert!(note.contains("unaccounted"), "{note}");
    }

    #[cfg(unix)]
    fn deny_listing(dir: &Path) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o000)).map_err(|e| e.to_string())?;
        if fs::read_dir(dir).is_ok() {
            restore_listing(dir);
            return Err("running with privileges that bypass permissions (root?)".into());
        }
        Ok(())
    }

    #[cfg(unix)]
    fn restore_listing(dir: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o755));
    }

    #[cfg(windows)]
    fn deny_listing(dir: &Path) -> Result<(), String> {
        // Deny "list folder" to the current user via an explicit ACE.
        let user = std::env::var("USERNAME").map_err(|e| e.to_string())?;
        let output = std::process::Command::new("icacls")
            .arg(dir)
            .args(["/deny", &format!("{user}:(RD)")])
            .output()
            .map_err(|e| format!("icacls not available ({e})"))?;
        if !output.status.success() {
            return Err(format!(
                "icacls failed: {}",
                String::from_utf8_lossy(&output.stdout).trim()
            ));
        }
        if fs::read_dir(dir).is_ok() {
            restore_listing(dir);
            return Err("deny ACE did not block listing (backup privilege?)".into());
        }
        Ok(())
    }

    #[cfg(windows)]
    fn restore_listing(dir: &Path) {
        if let Ok(user) = std::env::var("USERNAME") {
            let _ = std::process::Command::new("icacls")
                .arg(dir)
                .args(["/remove:d", &user])
                .output();
        }
    }

    #[test]
    fn duplicate_finder_excludes_hardlinks_and_counts_read_errors() {
        let temp = tempfile::tempdir().unwrap();
        let content = vec![9_u8; 20_000];
        let a = temp.path().join("a.bin");
        fs::write(&a, &content).unwrap();
        fs::hard_link(&a, temp.path().join("a_link.bin")).unwrap();
        fs::write(temp.path().join("copy.bin"), &content).unwrap();
        // A hardlink-only pair (same size, same storage) is not a duplicate at all.
        let solo = temp.path().join("solo.bin");
        fs::write(&solo, vec![8_u8; 30_000]).unwrap();
        fs::hard_link(&solo, temp.path().join("solo_link.bin")).unwrap();
        // Zero-byte files stay excluded.
        fs::write(temp.path().join("empty1"), b"").unwrap();
        fs::write(temp.path().join("empty2"), b"").unwrap();

        let result = scan_dir(temp.path(), true);

        assert_eq!(result.files.len(), 7);
        assert_eq!(result.hardlink_extra_count, 2);
        assert_eq!(result.total_logical, LogicalSize(20_000 + 20_000 + 30_000));
        assert_eq!(
            result.duplicate_groups.len(),
            1,
            "{:?}",
            result.duplicate_groups
        );
        let group = &result.duplicate_groups[0];
        assert_eq!(group.file_indices.len(), 3);
        assert_eq!(group.distinct_storage, 2);
        let shared = (0..3)
            .filter(|&p| group.shares_storage_with_earlier(p, &result.files))
            .count();
        assert_eq!(
            shared, 1,
            "exactly one member is labelled 'already shares storage'"
        );
        assert_eq!(result.duplicate_bytes(), physical_of(&a));
        assert_eq!(result.duplicate_read_errors, 0);

        // A candidate that cannot be read is counted, not silently dropped.
        let mut files = result.files.clone();
        let third = temp.path().join("copy2.bin");
        fs::write(&third, &content).unwrap();
        let mut entry = files.iter().find(|f| f.name == "copy.bin").unwrap().clone();
        entry.path = third.clone();
        entry.name = "copy2.bin".into();
        files.push(entry);
        fs::remove_file(&third).unwrap();
        let found = find_duplicate_groups(&files);
        assert_eq!(found.read_errors, 1);
        assert_eq!(found.groups.len(), 1);
        assert_eq!(found.groups[0].distinct_storage, 2);
    }
}
