use crate::model::{DuplicateGroup, FileEntry, ScanResult};
use rayon::prelude::*;
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::mpsc::Sender,
    time::{Instant, UNIX_EPOCH},
};

pub enum ScanEvent {
    Progress {
        files: usize,
        bytes: u64,
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
    let mut total_bytes = 0_u64;
    let mut skipped = 0_u64;

    for item in jwalk::WalkDir::new(root)
        .follow_links(false)
        .skip_hidden(false)
    {
        let entry = match item {
            Ok(entry) => entry,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !include_hidden && is_hidden(&path, root) {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let size = metadata.len();
        let modified_unix = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_secs() as i64)
            .unwrap_or_default();
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let extension = path
            .extension()
            .map(|value| value.to_string_lossy().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "no extension".to_owned());

        total_bytes = total_bytes.saturating_add(size);
        files.push(FileEntry {
            path: path.clone(),
            name,
            extension,
            size,
            modified_unix,
        });

        if files.len() % 1_000 == 0 {
            let _ = sender.send(ScanEvent::Progress {
                files: files.len(),
                bytes: total_bytes,
                current: path,
                phase: "Scanning metadata",
            });
        }
    }

    files.sort_unstable_by_key(|file| std::cmp::Reverse(file.size));
    let duplicate_groups = if find_duplicates {
        let _ = sender.send(ScanEvent::Progress {
            files: files.len(),
            bytes: total_bytes,
            current: root.to_path_buf(),
            phase: "Checking duplicate candidates",
        });
        find_duplicate_groups(&files)
    } else {
        Vec::new()
    };

    Ok(ScanResult {
        files,
        duplicate_groups,
        total_bytes,
        skipped,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn is_hidden(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .any(|part| part.as_os_str().to_string_lossy().starts_with('.'))
}

fn find_duplicate_groups(files: &[FileEntry]) -> Vec<DuplicateGroup> {
    let mut by_size: HashMap<u64, Vec<usize>> = HashMap::new();
    for (index, file) in files.iter().enumerate().filter(|(_, file)| file.size > 0) {
        by_size.entry(file.size).or_default().push(index);
    }

    let candidates: Vec<(u64, Vec<usize>)> = by_size
        .into_iter()
        .filter(|(_, indices)| indices.len() > 1)
        .collect();

    let partial_groups: Vec<(u64, Vec<usize>)> = candidates
        .into_par_iter()
        .flat_map_iter(|(size, indices)| {
            let mut hashes: HashMap<[u8; 32], Vec<usize>> = HashMap::new();
            for index in indices {
                if let Ok(hash) = partial_hash(&files[index].path, size) {
                    hashes.entry(hash).or_default().push(index);
                }
            }
            hashes
                .into_values()
                .filter(|group| group.len() > 1)
                .map(move |group| (size, group))
        })
        .collect();

    let mut duplicates: Vec<DuplicateGroup> = partial_groups
        .into_par_iter()
        .flat_map_iter(|(size, indices)| {
            let mut hashes: HashMap<[u8; 32], Vec<usize>> = HashMap::new();
            for index in indices {
                if let Ok(hash) = full_hash(&files[index].path) {
                    hashes.entry(hash).or_default().push(index);
                }
            }
            hashes
                .into_values()
                .filter(|group| group.len() > 1)
                .map(move |file_indices| DuplicateGroup {
                    size_each: size,
                    file_indices,
                })
        })
        .collect();

    duplicates.sort_unstable_by_key(|group| std::cmp::Reverse(group.reclaimable()));
    duplicates
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
    use std::fs;

    #[test]
    fn scans_sizes_and_confirms_exact_duplicates() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("original.bin"), b"same bytes").unwrap();
        fs::write(temp.path().join("copy.bin"), b"same bytes").unwrap();
        fs::write(temp.path().join("different.bin"), b"other data").unwrap();
        let (sender, _receiver) = std::sync::mpsc::channel();

        let result = scan(temp.path(), false, true, &sender).unwrap();

        assert_eq!(result.files.len(), 3);
        assert_eq!(result.total_bytes, 30);
        assert_eq!(result.duplicate_groups.len(), 1);
        assert_eq!(result.duplicate_groups[0].file_indices.len(), 2);
        assert_eq!(result.duplicate_bytes(), 10);
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
}
