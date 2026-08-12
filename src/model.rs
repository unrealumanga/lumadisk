use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub extension: String,
    pub size: u64,
    pub modified_unix: i64,
}

#[derive(Clone, Debug)]
pub struct DuplicateGroup {
    pub size_each: u64,
    pub file_indices: Vec<usize>,
}

impl DuplicateGroup {
    pub fn reclaimable(&self) -> u64 {
        self.size_each
            .saturating_mul(self.file_indices.len().saturating_sub(1) as u64)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScanResult {
    pub files: Vec<FileEntry>,
    pub duplicate_groups: Vec<DuplicateGroup>,
    pub total_bytes: u64,
    pub skipped: u64,
    pub elapsed_ms: u128,
}

impl ScanResult {
    pub fn duplicate_bytes(&self) -> u64 {
        self.duplicate_groups
            .iter()
            .map(DuplicateGroup::reclaimable)
            .sum()
    }

    /// Removes a file while preserving every duplicate-group index.
    ///
    /// File positions are used by the UI for compactness, so deleting an item from the
    /// middle of `files` must also shift later positions in every duplicate group.
    pub fn remove_file(&mut self, path: &std::path::Path) -> bool {
        let Some(position) = self.files.iter().position(|file| file.path == path) else {
            return false;
        };

        self.total_bytes = self.total_bytes.saturating_sub(self.files[position].size);
        self.files.remove(position);

        for group in &mut self.duplicate_groups {
            group.file_indices.retain(|index| *index != position);
            for index in &mut group.file_indices {
                if *index > position {
                    *index -= 1;
                }
            }
        }
        self.duplicate_groups
            .retain(|group| group.file_indices.len() > 1);
        self.duplicate_groups
            .sort_unstable_by_key(|group| std::cmp::Reverse(group.reclaimable()));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> FileEntry {
        FileEntry {
            path: PathBuf::from(name),
            name: name.to_owned(),
            extension: "bin".to_owned(),
            size,
            modified_unix: 0,
        }
    }

    #[test]
    fn removing_duplicate_keeps_other_groups_and_reindexes_them() {
        let mut result = ScanResult {
            files: vec![file("a", 10), file("b", 10), file("c", 20), file("d", 20)],
            duplicate_groups: vec![
                DuplicateGroup {
                    size_each: 10,
                    file_indices: vec![0, 1],
                },
                DuplicateGroup {
                    size_each: 20,
                    file_indices: vec![2, 3],
                },
            ],
            total_bytes: 60,
            ..Default::default()
        };

        assert!(result.remove_file(PathBuf::from("b").as_path()));
        assert_eq!(result.files.len(), 3);
        assert_eq!(result.total_bytes, 50);
        assert_eq!(result.duplicate_groups.len(), 1);
        assert_eq!(result.duplicate_groups[0].file_indices, vec![1, 2]);
        assert_eq!(result.files[1].name, "c");
        assert_eq!(result.files[2].name, "d");
    }
}
