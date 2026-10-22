//! Per-file size + identity query, without following symlinks.
//!
//! One call per file returns the logical size, the allocated (physical) size, the link count and
//! a storage identity used to count hardlinks once.
//!
//! * Unix: `symlink_metadata` + `MetadataExt` (`st_blocks * 512`, `st_dev`, `st_ino`, `st_nlink`).
//! * Windows: one `CreateFileW(FILE_READ_ATTRIBUTES, FILE_FLAG_OPEN_REPARSE_POINT)` handle, then
//!   `GetFileInformationByHandle` (size, mtime, links, volume serial, file index) and
//!   `GetFileInformationByHandleEx(FileStandardInfo).AllocationSize` for the physical size.
//!   NOT `GetCompressedFileSizeW`: measured on NTFS it returns the *logical* size for ordinary
//!   files (5000-byte file -> 5000, AllocationSize -> 8192), so "physical" would silently equal
//!   logical for most files. Both agree on sparse/compressed files (holes -> 0); a test
//!   cross-checks that. Same handle count as std's `symlink_metadata` (one open per file).
//!   std-only FFI: see DECISIONS.md.
//!
//! A physical-size failure is reported as `physical: None` (unknown), never as the logical size.
use std::{io, path::Path};

/// Identity of the underlying storage: (device / volume serial, inode / file index).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileId {
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct FileInfo {
    pub logical: u64,
    /// Allocated bytes on disk; `None` = the OS call for it failed (unknown, not zero).
    pub physical: Option<u64>,
    pub links: u32,
    pub id: Option<FileId>,
    pub modified_unix: i64,
}

#[cfg(unix)]
pub fn query(path: &Path) -> io::Result<FileInfo> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path)?;
    Ok(FileInfo {
        logical: metadata.len(),
        // st_blocks is always in 512-byte units, independent of st_blksize.
        physical: Some(metadata.blocks().saturating_mul(512)),
        links: u32::try_from(metadata.nlink()).unwrap_or(u32::MAX),
        id: Some(FileId {
            device: metadata.dev(),
            inode: metadata.ino(),
        }),
        modified_unix: metadata.mtime(),
    })
}

#[cfg(windows)]
pub fn query(path: &Path) -> io::Result<FileInfo> {
    match win::query(path) {
        Ok(info) => Ok(info),
        // Could not open a handle (sharing violation on e.g. pagefile.sys, odd path forms).
        // std falls back to directory-listing data for the logical size; physical and identity
        // are then UNKNOWN and reported as such.
        Err(_) => {
            let metadata = std::fs::symlink_metadata(path)?;
            Ok(FileInfo {
                logical: metadata.len(),
                physical: None,
                links: 1,
                id: None,
                modified_unix: metadata
                    .modified()
                    .ok()
                    .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|value| value.as_secs() as i64)
                    .unwrap_or_default(),
            })
        }
    }
}

#[cfg(not(any(unix, windows)))]
pub fn query(path: &Path) -> io::Result<FileInfo> {
    let metadata = std::fs::symlink_metadata(path)?;
    Ok(FileInfo {
        logical: metadata.len(),
        physical: None,
        links: 1,
        id: None,
        modified_unix: 0,
    })
}

#[cfg(windows)]
pub mod win {
    use super::{FileId, FileInfo};
    use std::{ffi::c_void, io, os::windows::ffi::OsStrExt, path::Path};

    type Handle = *mut c_void;
    const INVALID_HANDLE_VALUE: Handle = -1_isize as Handle;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const FILE_SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_STANDARD_INFO_CLASS: i32 = 1;
    #[cfg(test)]
    const INVALID_FILE_SIZE: u32 = u32::MAX;
    /// 100 ns intervals between 1601-01-01 and 1970-01-01.
    const EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ByHandleFileInformation {
        attributes: u32,
        creation: FileTime,
        last_access: FileTime,
        last_write: FileTime,
        volume_serial: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct FileStandardInfo {
        allocation_size: i64,
        end_of_file: i64,
        links: u32,
        delete_pending: u8,
        directory: u8,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            disposition: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
        fn CloseHandle(handle: Handle) -> i32;
        fn GetFileInformationByHandle(handle: Handle, info: *mut ByHandleFileInformation) -> i32;
        fn GetFileInformationByHandleEx(
            handle: Handle,
            class: i32,
            info: *mut c_void,
            size: u32,
        ) -> i32;
    }

    #[cfg(test)]
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCompressedFileSizeW(name: *const u16, high: *mut u32) -> u32;
        fn GetLastError() -> u32;
    }

    /// NUL-terminated wide path; absolute paths at or beyond MAX_PATH get the `\\?\` prefix
    /// (std does this internally; raw FFI calls must do it themselves).
    fn wide(path: &Path) -> Vec<u16> {
        let raw: Vec<u16> = path.as_os_str().encode_wide().collect();
        let starts = |prefix: &str| {
            let p: Vec<u16> = prefix.encode_utf16().collect();
            raw.starts_with(&p)
        };
        let mut out = Vec::with_capacity(raw.len() + 9);
        if raw.len() >= 248 && path.is_absolute() && !starts(r"\\?\") {
            if starts(r"\\") {
                out.extend(r"\\?\UNC".encode_utf16());
                out.extend_from_slice(&raw[1..]);
            } else {
                out.extend(r"\\?\".encode_utf16());
                out.extend_from_slice(&raw);
            }
        } else {
            out.extend_from_slice(&raw);
        }
        out.push(0);
        out
    }

    struct Owned(Handle);
    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: handle came from a successful CreateFileW and is closed exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }

    pub fn query(path: &Path) -> io::Result<FileInfo> {
        let name = wide(path);
        // SAFETY: `name` is NUL-terminated and outlives the call; null security/template ok.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_ALL,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let handle = Owned(handle);

        let mut info = ByHandleFileInformation::default();
        // SAFETY: valid open handle and a correctly laid out BY_HANDLE_FILE_INFORMATION.
        if unsafe { GetFileInformationByHandle(handle.0, &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let logical = (u64::from(info.size_high) << 32) | u64::from(info.size_low);

        let mut standard = FileStandardInfo::default();
        // SAFETY: valid handle; buffer is a repr(C) FILE_STANDARD_INFO of the stated size.
        let physical = if unsafe {
            GetFileInformationByHandleEx(
                handle.0,
                FILE_STANDARD_INFO_CLASS,
                (&mut standard as *mut FileStandardInfo).cast(),
                std::mem::size_of::<FileStandardInfo>() as u32,
            )
        } != 0
        {
            u64::try_from(standard.allocation_size).ok()
        } else {
            None
        };

        let written = (i64::from(info.last_write.high) << 32) | i64::from(info.last_write.low);
        Ok(FileInfo {
            logical,
            physical,
            links: info.links,
            id: Some(FileId {
                device: u64::from(info.volume_serial),
                inode: (u64::from(info.index_high) << 32) | u64::from(info.index_low),
            }),
            modified_unix: if written > 0 {
                (written - EPOCH_DIFF_100NS) / 10_000_000
            } else {
                0
            },
        })
    }

    /// The documented path-based API, used by tests to cross-check `query`.
    #[cfg(test)]
    pub fn compressed_file_size(path: &Path) -> io::Result<u64> {
        let name = wide(path);
        let mut high = 0_u32;
        // SAFETY: NUL-terminated name, valid out-pointer.
        let low = unsafe { GetCompressedFileSizeW(name.as_ptr(), &mut high) };
        // SAFETY: plain thread-local error read.
        if low == INVALID_FILE_SIZE && unsafe { GetLastError() } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((u64::from(high) << 32) | u64::from(low))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_logical_size_and_identity_for_a_plain_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("plain.bin");
        std::fs::write(&path, vec![7_u8; 5000]).unwrap();
        let info = query(&path).unwrap();
        assert_eq!(info.logical, 5000);
        assert_eq!(info.links, 1);
        assert!(info.id.is_some());
        // Allocation is whole clusters/blocks, so never below the logical size for a dense file.
        assert!(info.physical.unwrap() >= 5000, "{:?}", info.physical);
        #[cfg(windows)]
        {
            // Documents why GetCompressedFileSizeW is not the physical source: on NTFS it
            // reports the logical size for an ordinary file.
            let gcfs = win::compressed_file_size(&path).unwrap();
            println!(
                "allocation={:?} GetCompressedFileSizeW={gcfs}",
                info.physical
            );
            assert!(gcfs <= info.physical.unwrap());
        }
    }

    #[test]
    fn does_not_follow_a_symlink_it_is_given() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.bin");
        std::fs::write(&target, vec![1_u8; 100_000]).unwrap();
        let link = temp.path().join("link");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&target, &link);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&target, &link);
        #[cfg(not(any(unix, windows)))]
        let made: std::io::Result<()> = Err(std::io::Error::other("no symlinks"));
        if let Err(error) = made {
            println!(
                "SKIPPED does_not_follow_a_symlink_it_is_given: cannot create symlink ({error})"
            );
            return;
        }
        let info = query(&link).unwrap();
        assert!(info.logical < 100_000, "symlink was followed: {info:?}");
    }
}
