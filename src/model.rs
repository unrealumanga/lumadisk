use std::{
    iter::Sum,
    ops::{Add, AddAssign},
    path::PathBuf,
};

pub use crate::fsinfo::FileId;

/// Bytes a file *contains* (its length, what Explorer/Finder "Size" shows).
///
/// Distinct from [`PhysicalSize`]: the two can be added only to themselves.
///
/// ```
/// use lumadisk::model::{LogicalSize, PhysicalSize};
/// let logical: LogicalSize = [LogicalSize(1), LogicalSize(2)].into_iter().sum();
/// let physical = PhysicalSize(4096) + PhysicalSize(4096);
/// assert_eq!((logical, physical), (LogicalSize(3), PhysicalSize(8192)));
/// ```
///
/// Adding a physical size to a logical one does not compile:
///
/// ```compile_fail,E0308
/// use lumadisk::model::{LogicalSize, PhysicalSize};
/// let _ = LogicalSize(1) + PhysicalSize(1);
/// ```
///
/// ```compile_fail,E0308
/// use lumadisk::model::{LogicalSize, PhysicalSize};
/// let mut total = PhysicalSize(0);
/// total += LogicalSize(1);
/// ```
///
/// ```compile_fail,E0277
/// use lumadisk::model::{LogicalSize, PhysicalSize};
/// let _: LogicalSize = [PhysicalSize(1), PhysicalSize(2)].into_iter().sum();
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogicalSize(pub u64);

/// Bytes allocated on disk ("size on disk"; what deleting the last link can return).
/// Unix `st_blocks * 512`; Windows `FILE_STANDARD_INFO.AllocationSize`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysicalSize(pub u64);

macro_rules! byte_arithmetic {
    ($ty:ident) => {
        impl Add for $ty {
            type Output = $ty;
            fn add(self, other: $ty) -> $ty {
                $ty(self.0.saturating_add(other.0))
            }
        }
        impl AddAssign for $ty {
            fn add_assign(&mut self, other: $ty) {
                self.0 = self.0.saturating_add(other.0);
            }
        }
        impl Sum for $ty {
            fn sum<I: Iterator<Item = $ty>>(iter: I) -> $ty {
                iter.fold($ty(0), Add::add)
            }
        }
        impl<'a> Sum<&'a $ty> for $ty {
            fn sum<I: Iterator<Item = &'a $ty>>(iter: I) -> $ty {
                iter.copied().sum()
            }
        }
        impl $ty {
            pub fn saturating_sub(self, other: $ty) -> $ty {
                $ty(self.0.saturating_sub(other.0))
            }
        }
    };
}
byte_arithmetic!(LogicalSize);
byte_arithmetic!(PhysicalSize);

/// Which size the UI displays. Default per DECISIONS.md D2: physical.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SizeMode {
    #[default]
    Physical,
    Logical,
}

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub extension: String,
    pub logical: LogicalSize,
    /// `None` = the OS call for the allocated size failed: UNKNOWN, never assumed = logical.
    pub physical: Option<PhysicalSize>,
    pub modified_unix: i64,
    /// Hardlink count reported by the OS (1 = no other names).
    pub links: u32,
    pub id: Option<FileId>,
    /// Another path to the same storage is the counted one; this path adds no bytes to totals.
    pub extra_link: bool,
}

impl FileEntry {
    pub fn counted_logical(&self) -> LogicalSize {
        if self.extra_link {
            LogicalSize(0)
        } else {
            self.logical
        }
    }

    pub fn counted_physical(&self) -> Option<PhysicalSize> {
        if self.extra_link {
            Some(PhysicalSize(0))
        } else {
            self.physical
        }
    }

    /// Bytes this path contributes in `mode`, as a raw number for display/weights.
    /// `None` only for an unknown physical size.
    pub fn counted_bytes(&self, mode: SizeMode) -> Option<u64> {
        match mode {
            SizeMode::Logical => Some(self.counted_logical().0),
            SizeMode::Physical => self.counted_physical().map(|size| size.0),
        }
    }

    /// Storage key: paths with equal keys are the same bytes on disk.
    pub fn storage_key(&self, index: usize) -> StorageKey {
        match self.id {
            Some(id) if self.links > 1 => StorageKey::Shared(id),
            _ => StorageKey::Own(index),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StorageKey {
    Shared(FileId),
    Own(usize),
}

/// Why a directory could not be listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorClass {
    PermissionDenied,
    NotFound,
    Other(std::io::ErrorKind),
    Unknown,
}

impl ErrorClass {
    pub fn from_io(error: Option<&std::io::Error>) -> Self {
        match error.map(std::io::Error::kind) {
            Some(std::io::ErrorKind::PermissionDenied) => Self::PermissionDenied,
            Some(std::io::ErrorKind::NotFound) => Self::NotFound,
            Some(kind) => Self::Other(kind),
            None => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirState {
    Read,
    /// Contents unknown: the bytes below it are UNACCOUNTED, not zero.
    Unreadable(ErrorClass),
}

#[derive(Clone, Debug)]
pub struct DirRecord {
    pub path: PathBuf,
    pub state: DirState,
}

#[derive(Clone, Debug)]
pub struct DuplicateGroup {
    /// Groups are matched on content, so every member has this logical size.
    pub size_each: LogicalSize,
    pub file_indices: Vec<usize>,
    /// Number of distinct storages (hardlinks of one inode count once). Always >= 2.
    pub distinct_storage: usize,
    /// Physical bytes freed by keeping one storage and deleting every path of the others.
    /// Conservative: assumes the largest storage is the one kept.
    pub reclaimable: PhysicalSize,
    /// Some member's physical size is unknown, so `reclaimable` is a lower bound.
    pub physical_unknown: bool,
}

impl DuplicateGroup {
    pub fn new(size_each: LogicalSize, file_indices: Vec<usize>, files: &[FileEntry]) -> Self {
        let mut seen = std::collections::HashSet::new();
        let mut sizes = Vec::new();
        let mut physical_unknown = false;
        for &index in &file_indices {
            if seen.insert(files[index].storage_key(index)) {
                match files[index].physical {
                    Some(size) => sizes.push(size),
                    None => {
                        physical_unknown = true;
                        sizes.push(PhysicalSize(0));
                    }
                }
            }
        }
        let largest = sizes.iter().copied().max().unwrap_or_default();
        let total: PhysicalSize = sizes.iter().sum();
        Self {
            size_each,
            distinct_storage: sizes.len(),
            reclaimable: total.saturating_sub(largest),
            physical_unknown,
            file_indices,
        }
    }

    /// True when the member at `position` is a hardlink of an earlier member: deleting it
    /// frees nothing ("already shares storage").
    pub fn shares_storage_with_earlier(&self, position: usize, files: &[FileEntry]) -> bool {
        let key = |p: usize| {
            let index = self.file_indices[p];
            files[index].storage_key(index)
        };
        let own = key(position);
        (0..position).any(|earlier| key(earlier) == own)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScanResult {
    pub files: Vec<FileEntry>,
    /// Every directory visited, with its readable/unreadable state.
    pub dirs: Vec<DirRecord>,
    pub duplicate_groups: Vec<DuplicateGroup>,
    /// Hardlinked storage counted once.
    pub total_logical: LogicalSize,
    /// Known physical bytes only (hardlinks once); see `physical_unknown_count`.
    pub total_physical: PhysicalSize,
    /// Counted files whose physical size could not be read (not included in `total_physical`).
    pub physical_unknown_count: u64,
    /// Paths listed as hardlinks of an already counted file (no bytes counted for them).
    pub hardlink_extra_count: u64,
    /// Walk entries / files whose metadata could not be read (their size is unknown).
    pub file_error_count: u64,
    /// Duplicate candidates dropped because their content could not be read.
    pub duplicate_read_errors: u64,
    pub elapsed_ms: u128,
}

impl ScanResult {
    pub fn unreadable_dirs(&self) -> impl Iterator<Item = &DirRecord> {
        self.dirs
            .iter()
            .filter(|dir| matches!(dir.state, DirState::Unreadable(_)))
    }

    pub fn unreadable_dir_count(&self) -> usize {
        self.unreadable_dirs().count()
    }

    /// Human note when some bytes are UNACCOUNTED (unknown size, not zero), else `None`.
    pub fn unaccounted_note(&self) -> Option<String> {
        let dirs = self.unreadable_dir_count();
        let mut parts = Vec::new();
        if dirs > 0 {
            parts.push(format!(
                "{dirs} director{} unreadable",
                if dirs == 1 { "y" } else { "ies" }
            ));
        }
        if self.file_error_count > 0 {
            parts.push(format!("{} unreadable entries", self.file_error_count));
        }
        if self.physical_unknown_count > 0 {
            parts.push(format!(
                "{} files with unknown size on disk",
                self.physical_unknown_count
            ));
        }
        if parts.is_empty() {
            None
        } else {
            Some(format!("{} - some space unaccounted", parts.join(", ")))
        }
    }

    /// Physical bytes the duplicate finder could free (always physical, per D2).
    pub fn duplicate_bytes(&self) -> PhysicalSize {
        self.duplicate_groups
            .iter()
            .map(|group| group.reclaimable)
            .sum()
    }

    /// Removes a file while preserving every duplicate-group index and the totals.
    ///
    /// File positions are used by the UI for compactness, so deleting an item from the
    /// middle of `files` must also shift later positions in every duplicate group.
    /// Removing one name of a hardlinked file frees nothing while another scanned name
    /// remains: that name is promoted to be the counted one and totals are unchanged.
    pub fn remove_file(&mut self, path: &std::path::Path) -> bool {
        let Some(position) = self.files.iter().position(|file| file.path == path) else {
            return false;
        };

        let removed = &self.files[position];
        if removed.extra_link {
            self.hardlink_extra_count = self.hardlink_extra_count.saturating_sub(1);
        } else {
            let heir = match removed.storage_key(position) {
                StorageKey::Shared(id) => self
                    .files
                    .iter()
                    .position(|file| file.extra_link && file.id == Some(id)),
                StorageKey::Own(_) => None,
            };
            if let Some(heir) = heir {
                self.files[heir].extra_link = false;
                self.hardlink_extra_count = self.hardlink_extra_count.saturating_sub(1);
            } else {
                let removed = &self.files[position];
                self.total_logical = self.total_logical.saturating_sub(removed.logical);
                match removed.physical {
                    Some(size) => self.total_physical = self.total_physical.saturating_sub(size),
                    None => {
                        self.physical_unknown_count = self.physical_unknown_count.saturating_sub(1)
                    }
                }
            }
        }
        self.files.remove(position);

        let files = &self.files;
        for group in &mut self.duplicate_groups {
            group.file_indices.retain(|index| *index != position);
            for index in &mut group.file_indices {
                if *index > position {
                    *index -= 1;
                }
            }
            *group = DuplicateGroup::new(
                group.size_each,
                std::mem::take(&mut group.file_indices),
                files,
            );
        }
        self.duplicate_groups
            .retain(|group| group.distinct_storage > 1);
        self.duplicate_groups
            .sort_unstable_by_key(|group| std::cmp::Reverse(group.reclaimable));
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
            logical: LogicalSize(size),
            physical: Some(PhysicalSize(size)),
            modified_unix: 0,
            links: 1,
            id: None,
            extra_link: false,
        }
    }

    #[test]
    fn removing_duplicate_keeps_other_groups_and_reindexes_them() {
        let files = vec![file("a", 10), file("b", 10), file("c", 20), file("d", 20)];
        let mut result = ScanResult {
            duplicate_groups: vec![
                DuplicateGroup::new(LogicalSize(10), vec![0, 1], &files),
                DuplicateGroup::new(LogicalSize(20), vec![2, 3], &files),
            ],
            files,
            total_logical: LogicalSize(60),
            total_physical: PhysicalSize(60),
            ..Default::default()
        };

        assert!(result.remove_file(PathBuf::from("b").as_path()));
        assert_eq!(result.files.len(), 3);
        assert_eq!(result.total_logical, LogicalSize(50));
        assert_eq!(result.total_physical, PhysicalSize(50));
        assert_eq!(result.duplicate_groups.len(), 1);
        assert_eq!(result.duplicate_groups[0].file_indices, vec![1, 2]);
        assert_eq!(result.files[1].name, "c");
        assert_eq!(result.files[2].name, "d");
    }

    #[test]
    fn removing_one_hardlink_name_frees_nothing_and_promotes_the_other() {
        let id = FileId {
            device: 1,
            inode: 7,
        };
        let mut first = file("a", 4096);
        first.links = 2;
        first.id = Some(id);
        let mut second = first.clone();
        second.path = PathBuf::from("b");
        second.extra_link = true;
        let mut result = ScanResult {
            files: vec![first, second],
            total_logical: LogicalSize(4096),
            total_physical: PhysicalSize(4096),
            hardlink_extra_count: 1,
            ..Default::default()
        };

        assert!(result.remove_file(PathBuf::from("a").as_path()));
        assert_eq!(result.total_physical, PhysicalSize(4096));
        assert_eq!(result.hardlink_extra_count, 0);
        assert!(!result.files[0].extra_link);
        assert!(result.remove_file(PathBuf::from("b").as_path()));
        assert_eq!(result.total_physical, PhysicalSize(0));
        assert_eq!(result.total_logical, LogicalSize(0));
    }

    #[test]
    fn duplicate_group_counts_hardlinked_members_once() {
        let id = FileId {
            device: 1,
            inode: 9,
        };
        let mut a = file("a", 100);
        a.physical = Some(PhysicalSize(4096));
        a.links = 2;
        a.id = Some(id);
        let mut a2 = a.clone();
        a2.extra_link = true;
        let mut c = file("c", 100);
        c.physical = Some(PhysicalSize(4096));
        let files = vec![a, a2, c];
        let group = DuplicateGroup::new(LogicalSize(100), vec![0, 1, 2], &files);
        assert_eq!(group.distinct_storage, 2);
        assert_eq!(group.reclaimable, PhysicalSize(4096));
        assert!(group.shares_storage_with_earlier(1, &files));
        assert!(!group.shares_storage_with_earlier(2, &files));
    }
}
